use crate::common;

use common::World;
use sanchoris_core::items::{AuditEvent, ChangeRequest};
use sanchoris_core::jobs::{Envelope, Job};
use sanchoris_worker::ports::PullRequest;
use serde_json::json;

const CONFIG_REPO: &str = "acme/sanchoris-config";

fn open_pull(world: &World, number: i64, head_ref: &str, yaml: Option<String>) {
    let mut state = world.github.state.lock().unwrap();
    let head_sha = format!("{head_ref}-head");
    if let Some(yaml) = yaml {
        let mut files = state.files.clone();
        files.insert("sanchoris.yaml".into(), yaml);
        state.files_at.insert(head_sha.clone(), files);
    }
    state.pulls.insert(
        (CONFIG_REPO.into(), head_ref.into()),
        PullRequest {
            number,
            url: format!("https://github.com/{CONFIG_REPO}/pull/{number}"),
            title: "Allow example.com".into(),
            base_ref: "main".into(),
            base_sha: "config-1".into(),
            head_ref: head_ref.into(),
            head_sha,
            state: "open".into(),
            merged: false,
        },
    );
}

async fn sync(world: &World, number: i64) {
    world
        .worker
        .worker
        .handle(&Envelope::new(Job::ChangeRequestSync {
            tenant_id: world.tenant(),
            config_id: world.seeded.config_id,
            number,
        }))
        .await
        .unwrap();
}

async fn audits(world: &World) -> Vec<String> {
    let mut actions: Vec<String> = world
        .dynamo
        .all::<AuditEvent>()
        .await
        .into_iter()
        .filter(|event| event.action.starts_with("change_request."))
        .map(|event| event.action)
        .collect();
    actions.sort();
    actions
}

#[tokio::test]
async fn a_task_pull_request_on_the_config_becomes_a_change_request_that_follows_its_state() {
    let world = World::new("max").await;
    let task = world.started_task(35).await;
    let yaml = common::CONFIG_YAML.replacen("Limits: { Turns: 60,", "Limits: { Turns: 61,", 1);
    open_pull(
        &world,
        80,
        &format!("sanchoris/{}", task.id),
        Some(yaml.clone()),
    );
    sync(&world, 80).await;
    let recorded = world.dynamo.all::<ChangeRequest>().await;
    assert_eq!(recorded.len(), 1);
    let change = &recorded[0];
    assert_eq!(change.pr_number, 80);
    assert_eq!(change.title, "Allow example.com");
    assert_eq!(change.state, "open");
    assert_eq!(change.changed_files, json!(["sanchoris.yaml"]));
    assert_eq!(
        change.changed_definitions,
        json!([{ "kind": "trigger", "name": "labeled-issue" }])
    );
    assert_eq!(change.validation_status.as_deref(), Some("valid"));
    assert_eq!(change.requested_by_task_id, Some(task.id));
    assert_eq!(change.requested_by_member_id, None);

    {
        let mut state = world.github.state.lock().unwrap();
        let pull = state
            .pulls
            .values_mut()
            .find(|pull| pull.number == 80)
            .unwrap();
        pull.state = "closed".into();
        pull.merged = true;
    }
    sync(&world, 80).await;
    let merged = world.dynamo.all::<ChangeRequest>().await;
    assert_eq!(merged[0].state, "merged");
    assert_eq!(merged[0].id, change.id);
    assert_eq!(
        audits(&world).await,
        vec!["change_request.merge", "change_request.open"]
    );
}

#[tokio::test]
async fn a_pull_request_that_leaves_the_config_alone_is_not_a_change_request() {
    let world = World::new("max").await;
    open_pull(&world, 81, "docs", None);
    sync(&world, 81).await;
    assert!(world.dynamo.all::<ChangeRequest>().await.is_empty());

    let broken = common::CONFIG_YAML.replacen("Version: 1", "Version: 9", 1);
    open_pull(&world, 82, "broken", Some(broken));
    sync(&world, 82).await;
    let recorded = world.dynamo.all::<ChangeRequest>().await;
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].validation_status.as_deref(), Some("invalid"));
    assert_eq!(recorded[0].requested_by_task_id, None);
}
