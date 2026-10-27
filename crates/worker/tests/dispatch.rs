mod common;

use common::{REPO, World};
use sanchoris_core::items::{ConfigRevision, InboundEvent, SkillDef, Task, Trigger, TriggerEvent};
use sanchoris_core::jobs::{Envelope, Job};
use sanchoris_core::keys;
use sanchoris_core::model::TaskStatus;
use sanchoris_core::store::retry;
use serde_json::json;

const START_TEXT: &str = "[sanchoris] Trigger labeled-issue started this task: Fix flaky checkout test (acme/shop-web#35).\n\
Instruction: Resolve issue 35 in acme/shop-web: Fix flaky checkout test. Write tests for the change.\n\
Skills: open-pr-green-ci (required). Run it with `sanchoris skill run open-pr-green-ci`.";

async fn trigger(world: &World, name: &str) -> Trigger {
    world
        .dynamo
        .all::<Trigger>()
        .await
        .into_iter()
        .find(|t| t.name == name)
        .unwrap()
}

async fn revision(world: &World, sha: &str) -> ConfigRevision {
    world
        .revisions()
        .await
        .into_iter()
        .find(|r| r.commit_sha == sha)
        .unwrap()
}

async fn active_revision(world: &World) -> Option<uuid::Uuid> {
    world
        .store
        .get::<sanchoris_core::items::Config>(&keys::config_key(
            world.tenant(),
            world.seeded.config_id,
        ))
        .await
        .unwrap()
        .unwrap()
        .active_revision_id
}

#[tokio::test]
async fn config_sync_resolves_project_repository_keys() {
    let world = World::new("pro").await;
    let pattern = trigger(&world, "labeled-issue")
        .await
        .event_pattern
        .unwrap();
    assert_eq!(
        pattern["detail"]["repository"],
        json!(["acme/shop-web", "acme/shop-api"])
    );
    let revision = revision(&world, "config-1").await;
    assert_eq!(active_revision(&world).await, Some(revision.id));
    assert_eq!(revision.validation_status, "valid");
    let mut skills = world
        .dynamo
        .all::<SkillDef>()
        .await
        .into_iter()
        .map(|s| (s.name, s.kind))
        .collect::<Vec<_>>();
    skills.sort();
    assert_eq!(skills, vec![("open-pr-green-ci".into(), "reliable".into())]);
}

#[tokio::test]
async fn invalid_revisions_do_not_replace_the_active_one() {
    let world = World::new("pro").await;
    {
        let mut state = world.github.state.lock().unwrap();
        state.head_sha = "config-2".into();
        let broken = common::CONFIG_YAML.replace(
            "Session: shop\n      Instruction: 'Resolve",
            "Session: missing\n      Instruction: 'Resolve",
        );
        state.files.insert("sanchoris.yaml".into(), broken);
    }
    world.resync_config().await;
    let handled = world
        .worker
        .worker
        .handle(&Envelope::new(Job::ConfigSync {
            tenant_id: world.tenant(),
            config_id: world.seeded.config_id,
        }))
        .await
        .unwrap();
    assert_eq!(handled, 1);
    let revision = revision(&world, "config-2").await;
    assert_eq!(revision.validation_status, "invalid");
    assert_ne!(active_revision(&world).await, Some(revision.id));
    assert!(
        revision
            .validation_messages
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["Location"] == "Triggers.labeled-issue.Start.Session"),
        "{}",
        revision.validation_messages
    );
    assert_eq!(
        trigger(&world, "labeled-issue").await.start["Session"],
        "shop"
    );
}

#[tokio::test]
async fn labeled_issue_starts_a_running_task_with_the_documented_message() {
    let world = World::new("pro").await;
    world
        .label_issue("delivery-35", 35, "Fix flaky checkout test", "mika-dev")
        .await;
    let inbound = world
        .queue
        .bodies()
        .iter()
        .map(|body| Envelope::parse(body).unwrap().job)
        .collect::<Vec<_>>();
    assert_eq!(
        inbound,
        vec![Job::Inbound {
            tenant_id: world.tenant()
        }]
    );
    let envelope = Envelope::new(Job::Inbound {
        tenant_id: world.tenant(),
    });
    assert_eq!(world.worker.worker.handle(&envelope).await.unwrap(), 1);
    let task = world.task("acme/shop-web#35").await;
    assert_eq!(task.status, "RUNNING");
    assert_eq!(task.title, "Fix flaky checkout test (acme/shop-web#35)");
    assert_eq!(task.requested_by_member_id, Some(world.seeded.member_id));
    let run_id = task.run_id.expect("a run is created");
    assert!(world.runner.started.lock().unwrap().is_empty());
    world.worker.settle().await.unwrap();
    let starts = world.outbox(run_id, "start").await;
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0]["instruction"], START_TEXT);
    assert_eq!(starts[0]["agent"], "claude");
    assert_eq!(starts[0]["model"], "claude-sonnet-5-5");
    assert_eq!(
        starts[0]["systemPrompt"],
        "You are working on the Acme shop. Always write tests for new endpoints.\n"
    );
    assert_eq!(
        starts[0]["skills"],
        json!([{ "name": "open-pr-green-ci", "required": true, "inputs": { "Issue": "number", "Repo": "string", "Repos": "array" } }])
    );
    assert_eq!(starts[0]["heartbeatSeconds"], 900);
    assert_eq!(*world.runner.started.lock().unwrap(), vec![run_id]);
    assert!(world.pusher.pushed.lock().unwrap().contains(&run_id));
    assert_eq!(
        world.trigger_event("delivery-35").await,
        (Some("started".into()), None)
    );
    assert_eq!(world.dynamo.all::<InboundEvent>().await.len(), 1);
    assert_eq!(world.dynamo.all::<TriggerEvent>().await.len(), 1);
    world
        .label_issue(
            "delivery-35-again",
            35,
            "Fix flaky checkout test",
            "mika-dev",
        )
        .await;
    world.worker.settle().await.unwrap();
    assert_eq!(
        world.trigger_event("delivery-35-again").await,
        (
            Some("discarded".into()),
            Some(format!("task {REPO}#35 already exists"))
        )
    );
    assert_eq!(world.tasks().await.len(), 1);
    assert_eq!(world.runner.started.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn actors_outside_the_pattern_are_skipped_with_a_reason() {
    let world = World::new("pro").await;
    world
        .label_issue("delivery-eve", 40, "Mine bitcoin", "eve")
        .await;
    world.worker.settle().await.unwrap();
    assert_eq!(
        world.trigger_event("delivery-eve").await,
        (
            Some("skipped".into()),
            Some("the actor eve (MEMBER) is not allowed by the trigger".into())
        )
    );
    assert!(world.tasks().await.is_empty());
}

#[tokio::test]
async fn unrelated_events_leave_no_trigger_event() {
    let world = World::new("pro").await;
    let mut payload = common::labeled_payload(41, "Other repo", "mika-dev");
    payload["repository"]["full_name"] = json!("acme/elsewhere");
    world
        .deliver("delivery-elsewhere", "issues", &payload)
        .await;
    world.worker.settle().await.unwrap();
    assert!(world.dynamo.all::<TriggerEvent>().await.is_empty());
    let inbound = world.dynamo.all::<InboundEvent>().await;
    assert_eq!(inbound.len(), 1);
    assert!(inbound[0].processed_at.is_some());
}

#[tokio::test]
async fn children_beyond_the_limit_are_skipped() {
    let world = World::new("pro").await;
    world.github.set_parent(36, Some(35));
    world.github.set_parent(37, Some(36));
    for (number, title) in [(35, "Parent"), (36, "Child"), (37, "Grandchild")] {
        world
            .label_issue(&format!("delivery-{number}"), number, title, "mika-dev")
            .await;
        world.worker.settle().await.unwrap();
    }
    let parent = world.task("acme/shop-web#35").await;
    let child = world.task("acme/shop-web#36").await;
    assert_eq!(child.parent_task_id, Some(parent.id));
    assert_eq!(
        world.trigger_event("delivery-37").await,
        (
            Some("skipped".into()),
            Some("children limit: depth 2 exceeds MaxDepth 1".into())
        )
    );
    assert!(
        world
            .tasks()
            .await
            .iter()
            .all(|t| t.name != "acme/shop-web#37")
    );
}

#[tokio::test]
async fn children_count_is_limited() {
    let world = World::new("pro").await;
    world
        .label_issue("delivery-35", 35, "Parent", "mika-dev")
        .await;
    world.worker.settle().await.unwrap();
    for number in 100..106 {
        world.github.set_parent(number, Some(35));
        world
            .label_issue(&format!("delivery-{number}"), number, "Child", "mika-dev")
            .await;
        world.worker.settle().await.unwrap();
    }
    assert_eq!(
        world.trigger_event("delivery-105").await,
        (
            Some("skipped".into()),
            Some("children limit: acme/shop-web#35 already has 5 child tasks (MaxCount 5)".into())
        )
    );
}

#[tokio::test]
async fn chat_messages_go_to_the_active_task_of_their_thread() {
    let world = World::new("pro").await;
    let message = |key: &str, text: &str| json!({ "source": "web", "detail-type": "Message", "detail": { "scope": "all", "thread": "thread-1", "user": "Mika", "text": text, "key": key } });
    let send = |key: &'static str, text: &'static str| {
        let store = world.store.clone();
        let tenant = world.tenant();
        let event = message(key, text);
        async move {
            retry(20, || async {
                let mut tx = store.tx(tenant);
                sanchoris_core::channel::record_inbound_event(&mut tx, "web", key, &event).await?;
                tx.commit().await
            })
            .await
            .unwrap();
        }
    };
    send("m1", "Plan the next release").await;
    world.worker.settle().await.unwrap();
    let first = world.task("thread-1/1").await;
    assert_eq!(first.status, "RUNNING");
    assert_eq!(first.title, "Plan the next release");
    send("m2", "Also check the changelog").await;
    world.worker.settle().await.unwrap();
    assert_eq!(
        world.messages(first.run_id.unwrap()).await,
        vec!["[sanchoris] Message from Mika:\nAlso check the changelog".to_string()]
    );
    assert_eq!(
        world.trigger_event("m2").await,
        (
            Some("discarded".into()),
            Some("delivered to the active task thread-1/1 as a message".into())
        )
    );
    world
        .dynamo
        .modify::<Task>(&keys::task_key(world.tenant(), first.id), |task| {
            task.status = TaskStatus::Succeeded;
            task.finished_at = Some(chrono::Utc::now());
        })
        .await;
    send("m3", "One more thing").await;
    world.worker.settle().await.unwrap();
    let second = world.task("thread-1/2").await;
    assert_eq!(second.status, "RUNNING");
}

#[tokio::test]
async fn schedules_emit_events_at_due_times() {
    let world = World::new("pro").await;
    {
        let mut state = world.github.state.lock().unwrap();
        state.head_sha = "config-2".into();
        let yaml = format!(
            "{}  nightly:\n    State: ENABLED\n    ScheduleExpression: rate(1 hour)\n    Start:\n      Session: all\n      Instruction: 'Triage new issues.'\n      Skills: []\n      RequiredSkills: []\n      MaxNudges: 0\n      Limits: {{ Turns: 10, CostUsd: 5, TimeoutSeconds: 3600, HeartbeatSeconds: 600, Children: {{ MaxDepth: 1, MaxCount: 5 }} }}\n",
            common::CONFIG_YAML
        );
        state.files.insert("sanchoris.yaml".into(), yaml);
    }
    world.resync_config().await;
    world.worker.settle().await.unwrap();
    assert!(world.tasks().await.is_empty());
    let nightly = trigger(&world, "nightly").await;
    assert_eq!(
        nightly.next_run_at,
        Some(common::epoch() + chrono::Duration::hours(1))
    );
    assert_eq!(world.scheduler.pending().len(), 1);
    world.clock.advance(chrono::Duration::hours(1));
    world.worker.settle().await.unwrap();
    let task = world.task("nightly@2026-10-27T10:00:00Z").await;
    assert_eq!(task.status, "RUNNING");
    world.clock.advance(chrono::Duration::hours(1));
    world.worker.settle().await.unwrap();
    world.task("nightly@2026-10-27T11:00:00Z").await;
    assert_eq!(
        trigger(&world, "nightly").await.next_run_at,
        Some(common::epoch() + chrono::Duration::hours(3))
    );
}
