mod common;

use chrono::Duration;
use common::{REPO, World, skill_input, turn_end_payload};
use sanchoris_core::channel::{RunEventType, SEQ_STRIDE};
use sanchoris_core::items::{HeldMessage, Run, UsageRecord};
use sanchoris_core::keys;
use serde_json::{Value, json};

#[tokio::test]
async fn cumulative_turn_costs_add_once_per_agent_instance() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    let turn_end = |turn: i64, total: f64| {
        let mut payload = turn_end_payload(turn, Value::Null);
        payload["totalCostUsd"] = json!(total);
        payload
    };
    for (turn, total) in [(1, 0.1), (2, 0.2), (3, 0.3)] {
        world
            .sequenced_event(run_id, turn, RunEventType::TurnEnd, turn_end(turn, total))
            .await;
        world.worker.settle().await.unwrap();
    }
    let run = |world: &World| {
        let tenant = world.tenant();
        let store = world.store.clone();
        async move {
            store
                .query::<Run>(&sanchoris_core::keys::task_pk(tenant, task.id), "RUN#")
                .await
                .unwrap()
                .pop()
                .unwrap()
        }
    };
    assert!((run(&world).await.cost_usd - 0.3).abs() < 1e-9);
    world
        .sequenced_event(
            run_id,
            SEQ_STRIDE + 1,
            RunEventType::TurnEnd,
            turn_end(4, 0.05),
        )
        .await;
    world.worker.settle().await.unwrap();
    let after_restore = run(&world).await;
    assert!((after_restore.cost_usd - 0.35).abs() < 1e-9);
    assert_eq!(after_restore.turns, 4);
    let mut recorded = world
        .dynamo
        .all::<UsageRecord>()
        .await
        .into_iter()
        .filter(|u| u.run_id == run_id)
        .map(|u| u.cost_usd)
        .collect::<Vec<_>>();
    recorded.sort_by(f64::total_cmp);
    let total: f64 = recorded.iter().sum();
    assert_eq!(recorded.len(), 4);
    assert!((total - 0.35).abs() < 1e-9);
}

#[tokio::test]
async fn skill_notifications_wait_while_the_task_needs_you() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world.request_skill(run_id, skill_input(35)).await;
    world.drive().await;
    assert_eq!(
        common::skill_run(&world.store, world.tenant(), task.id, 1)
            .await
            .0,
        "WAITING"
    );
    world
        .dynamo
        .modify::<Run>(&keys::run_key(world.tenant(), task.id, run_id), |run| {
            run.turns = 60
        })
        .await;
    world.worker.settle().await.unwrap();
    let stuck = world.task(&format!("{REPO}#35")).await;
    assert_eq!(stuck.status, "NEEDS_YOU");
    assert_eq!(stuck.error.as_deref(), Some("Task.TurnLimitExceeded"));
    world.clock.advance(Duration::seconds(901));
    world.worker.settle().await.unwrap();
    assert_eq!(*world.runner.stopped.lock().unwrap(), vec![run_id]);

    world.github.state.lock().unwrap().check_results.clear();
    world.github.script_checks(&["success"]);
    world.clock.advance(Duration::minutes(5));
    world.worker.settle().await.unwrap();
    assert_eq!(
        common::skill_run(&world.store, world.tenant(), task.id, 1)
            .await
            .0,
        "SUCCEEDED"
    );
    assert!(world.messages(run_id).await.is_empty());
    assert_eq!(world.runner.started.lock().unwrap().len(), 1);
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "NEEDS_YOU");

    world
        .run_event(
            run_id,
            RunEventType::HumanReply,
            json!({ "member": "Mika", "text": "Go on." }),
        )
        .await;
    world.worker.settle().await.unwrap();
    let messages = world.messages(run_id).await;
    assert_eq!(messages.len(), 2);
    assert!(
        messages[0].starts_with("[sanchoris] Skill open-pr-green-ci (run 1) succeeded."),
        "{}",
        messages[0]
    );
    assert_eq!(messages[1], "[sanchoris] Message from Mika:\nGo on.");
    assert!(world.dynamo.all::<HeldMessage>().await.is_empty());
    assert_eq!(world.runner.started.lock().unwrap().len(), 2);
}
