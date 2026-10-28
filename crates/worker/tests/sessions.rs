mod common;

use common::{World, turn_end_payload};
use sanchoris_core::channel::{RunEventType, SEQ_STRIDE};
use sanchoris_core::items::{Run, UsageRecord};
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
