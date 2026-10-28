use crate::common;

use common::{REPO, World, skill_input};
use sanchoris_core::channel::RunEventType;
use sanchoris_core::item::Entity;
use sanchoris_core::items::{
    ApprovalWait, CheckWait, CheckWaitMarker, SkillRunItem, StateEntryItem, Task, TimerItem,
};
use sanchoris_core::jobs::schedule_name;
use sanchoris_core::keys;
use sanchoris_core::skillrun::{SkillRunError, resolve_approval};
use sanchoris_core::tasks::abort_task;
use sanchoris_core::tokens::{Signer, TaskTokenSigner};
use sanchoris_worker::engine_worker::{fire_timer, sweep_checks};
use serde_json::json;
use uuid::Uuid;

async fn add_skill(world: &World, task_id: Uuid, name: &str, yaml: &str) {
    let definition = sanchoris_config::skill::parse_skill_yaml(yaml).unwrap();
    let skill = sanchoris_config::Skill {
        name: name.into(),
        description: name.into(),
        body: String::new(),
        skill_md: String::new(),
        skill_yaml: Some(yaml.into()),
        definition: Some(definition),
        origin: sanchoris_config::SkillOrigin::Repository,
    };
    world
        .dynamo
        .modify::<Task>(&keys::task_key(world.tenant(), task_id), |stored| {
            stored.definition["Skills"]
                .as_array_mut()
                .unwrap()
                .push(json!({ "Name": name, "Definition": skill }));
        })
        .await;
}

async fn skill_run(world: &World, task_id: Uuid, number: i32) -> SkillRunItem {
    world
        .store
        .get::<SkillRunItem>(&keys::skill_run_key(world.tenant(), task_id, number))
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn a_long_inline_loop_fails_with_a_runtime_error_and_commits() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    add_skill(
        &world,
        task.id,
        "spin",
        "Inputs: {}\nStartAt: Init\nTimeoutSeconds: 600\nStates:\n  Init:\n    Type: Pass\n    Result: { count: 0 }\n    Next: Check\n  Check:\n    Type: Choice\n    Choices:\n      - Variable: $.count\n        NumericGreaterThanEquals: 120\n        Next: Done\n    Default: Count\n  Count:\n    Type: Pass\n    Parameters: { \"count.$\": \"States.MathAdd($.count, 1)\" }\n    Next: Check\n  Done:\n    Type: Succeed\n",
    )
    .await;
    let accepted = world
        .try_request_skill(run_id, "spin", json!({}))
        .await
        .unwrap();
    world.worker.settle().await.unwrap();
    let item = skill_run(&world, task.id, accepted.number).await;
    assert_eq!(item.status.as_str(), "FAILED");
    assert_eq!(item.error.as_deref(), Some("States.Runtime"));
    let entries = world
        .dynamo
        .all::<StateEntryItem>()
        .await
        .into_iter()
        .filter(|e| e.skill_run_id == accepted.id)
        .count();
    assert_eq!(entries, sanchoris_engine::MAX_TRANSITIONS_PER_STEP);
}

const PAUSE: &str = "Inputs: {}\nStartAt: Pause\nTimeoutSeconds: 600\nStates:\n  Pause:\n    Type: Wait\n    Seconds: 30\n    End: true\n";

#[tokio::test]
async fn an_early_timer_is_kept_and_scheduled_again() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    add_skill(&world, task.id, "pause", PAUSE).await;
    let accepted = world
        .try_request_skill(run_id, "pause", json!({}))
        .await
        .unwrap();
    world.worker.settle().await.unwrap();
    let timer = world
        .dynamo
        .all::<TimerItem>()
        .await
        .into_iter()
        .find(|t| t.timer["kind"] == "wait_elapsed")
        .unwrap();
    world
        .clock
        .set(timer.due_at - chrono::Duration::milliseconds(300));
    let fired = fire_timer(
        &world.worker.worker,
        world.tenant(),
        task.id,
        accepted.id,
        timer.id,
        timer.message_id,
    )
    .await
    .unwrap();
    assert_eq!(fired, 1);
    let kept = world
        .store
        .get::<TimerItem>(&timer.key())
        .await
        .unwrap()
        .expect("the timer is kept");
    assert_ne!(kept.message_id, timer.message_id);
    assert_eq!(kept.due_at, timer.due_at);
    assert!(
        world
            .scheduler
            .pending()
            .contains(&(schedule_name(kept.message_id), timer.due_at))
    );
    assert_eq!(
        skill_run(&world, task.id, accepted.number)
            .await
            .status
            .as_str(),
        "WAITING"
    );
    world.clock.set(timer.due_at);
    world.worker.settle().await.unwrap();
    assert_eq!(
        skill_run(&world, task.id, accepted.number)
            .await
            .status
            .as_str(),
        "SUCCEEDED"
    );
}

#[tokio::test]
async fn busy_heartbeats_of_the_run_keep_a_handoff_waiting() {
    let world = World::new("pro").await;
    world.github.script_checks(&["failure"]);
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world.request_skill(run_id, skill_input(35)).await;
    world.drive().await;
    assert_eq!(
        common::skill_run(&world.store, world.tenant(), task.id, 1)
            .await
            .0,
        "WAITING_AGENT"
    );
    for _ in 0..4 {
        world.clock.advance(chrono::Duration::seconds(300));
        world
            .run_event(
                run_id,
                RunEventType::Heartbeat,
                json!({ "at": world.clock_now(), "busy": true }),
            )
            .await;
        world.worker.settle().await.unwrap();
    }
    assert_eq!(
        common::skill_run(&world.store, world.tenant(), task.id, 1).await,
        ("WAITING_AGENT".into(), None)
    );
    world.clock.advance(chrono::Duration::seconds(300));
    world
        .run_event(
            run_id,
            RunEventType::Heartbeat,
            json!({ "at": world.clock_now(), "busy": false }),
        )
        .await;
    world.clock.advance(chrono::Duration::seconds(601));
    world.worker.settle().await.unwrap();
    assert_eq!(
        common::skill_run(&world.store, world.tenant(), task.id, 1).await,
        ("FAILED".into(), Some("States.HeartbeatTimeout".into()))
    );
}

async fn abort(world: &World, task_id: Uuid) {
    sanchoris_core::store::retry(20, || async {
        let mut tx = world.store.tx(world.tenant());
        abort_task(&mut tx, task_id, "stopped by Mika", world.clock_now()).await?;
        tx.commit().await
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stopping_a_task_aborts_its_skill_runs_and_their_timers() {
    let world = World::new("pro").await;
    world.github.script_checks(&["success"]);
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world.request_skill(run_id, skill_input(35)).await;
    world.worker.settle().await.unwrap();
    assert_eq!(world.answer_execs(false).await, 1);
    world.worker.settle().await.unwrap();
    assert_eq!(
        skill_run(&world, task.id, 1).await.status.as_str(),
        "RUNNING"
    );
    assert!(!world.dynamo.all::<TimerItem>().await.is_empty());

    abort(&world, task.id).await;
    world.clock.advance(chrono::Duration::seconds(60));
    world.drive().await;

    let item = skill_run(&world, task.id, 1).await;
    assert_eq!(item.status.as_str(), "FAILED");
    assert_eq!(item.error.as_deref(), Some(sanchoris_engine::ABORTED_ERROR));
    assert!(world.dynamo.all::<TimerItem>().await.is_empty());
    assert_eq!(world.outbox(run_id, "exec").await.len(), 1);
    let state = world.github.state.lock().unwrap();
    assert!(state.pushes.is_empty());
    assert!(state.created_pulls.is_empty());
    assert!(state.created_comments.is_empty());
    assert!(state.merges.is_empty());
}

#[tokio::test]
async fn a_closed_checks_wait_is_no_longer_polled() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world.request_skill(run_id, skill_input(35)).await;
    world.drive().await;
    assert_eq!(
        skill_run(&world, task.id, 1).await.status.as_str(),
        "WAITING"
    );
    let wait = world.dynamo.all::<CheckWait>().await.pop().unwrap();
    assert_eq!(wait.status, "waiting");

    world.clock.advance(chrono::Duration::seconds(3601));
    world.worker.settle().await.unwrap();
    let item = skill_run(&world, task.id, 1).await;
    assert_eq!(
        (item.status.as_str(), item.error.as_deref()),
        ("FAILED", Some("States.Timeout"))
    );
    let wait = world.dynamo.all::<CheckWait>().await.pop().unwrap();
    assert_eq!(wait.status, "cancelled");
    assert!(world.dynamo.all::<CheckWaitMarker>().await.is_empty());
    world.clock.advance(chrono::Duration::minutes(10));
    assert_eq!(sweep_checks(&world.worker.worker).await.unwrap(), 0);
}

#[tokio::test]
async fn an_old_approval_token_does_not_change_the_task() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    add_skill(
        &world,
        task.id,
        "ship",
        "Inputs: {}\nStartAt: Ask\nTimeoutSeconds: 3600\nStates:\n  Ask:\n    Type: Task\n    Resource: sanchoris:::approval:request.waitForTaskToken\n    Parameters: { Message: Ship it?, TaskToken.$: $$.Task.Token }\n    TimeoutSeconds: 300\n    Retry:\n      - ErrorEquals: [States.Timeout]\n        IntervalSeconds: 1\n        MaxAttempts: 1\n        BackoffRate: 1\n    End: true\n",
    )
    .await;
    let accepted = world
        .try_request_skill(run_id, "ship", json!({}))
        .await
        .unwrap();
    world.worker.settle().await.unwrap();
    let tokens = |waits: Vec<ApprovalWait>| {
        let mut waits = waits;
        waits.sort_by_key(|w| w.created_at);
        waits
            .into_iter()
            .map(|w| (w.task_token, w.status))
            .collect::<Vec<_>>()
    };
    let first = tokens(world.dynamo.all().await).pop().unwrap().0;
    world.clock.advance(chrono::Duration::seconds(301));
    world.worker.settle().await.unwrap();
    world.clock.advance(chrono::Duration::seconds(1));
    world.worker.settle().await.unwrap();
    let waits = tokens(world.dynamo.all().await);
    assert_eq!(waits.len(), 2);
    assert_eq!(waits[0], (first.clone(), "cancelled".to_string()));
    assert_eq!(waits[1].1, "waiting");
    let before = world.task(&format!("{REPO}#35")).await;
    assert_eq!(before.status, "NEEDS_YOU");
    assert_eq!(before.error.as_deref(), Some("Approval.Requested"));

    let signer = TaskTokenSigner::new(Signer::new(&[9u8; 32]).unwrap());
    let mut tx = world.store.tx(world.tenant());
    let stale = resolve_approval(
        &mut tx,
        &signer,
        &signer.sign(accepted.id, &first),
        true,
        "Mika",
        world.clock_now(),
    )
    .await;
    assert!(matches!(stale, Err(SkillRunError::ApprovalNotWaiting)));
    tx.commit().await.unwrap();
    let after = world.task(&format!("{REPO}#35")).await;
    assert_eq!(after, before);
    assert_eq!(
        skill_run(&world, task.id, 1).await.status.as_str(),
        "WAITING"
    );
}
