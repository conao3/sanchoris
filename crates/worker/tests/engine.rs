mod common;

use common::World;
use sanchoris_core::item::Entity;
use sanchoris_core::items::{SkillRunItem, StateEntryItem, Task, TimerItem};
use sanchoris_core::jobs::schedule_name;
use sanchoris_core::keys;
use sanchoris_worker::engine_worker::fire_timer;
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
