mod common;

use common::World;
use sanchoris_core::items::{SkillRunItem, StateEntryItem, Task};
use sanchoris_core::keys;
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
