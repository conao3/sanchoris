mod common;

use common::{REPO, World};
use sanchoris_core::items::{
    ConfigRevision, InboundEvent, Pending, SkillDef, Task, Tenant, Trigger, TriggerEvent,
    TriggerRate,
};
use sanchoris_core::jobs::{Envelope, Job};
use sanchoris_core::keys;
use sanchoris_core::model::TaskStatus;
use sanchoris_core::store::retry;
use sanchoris_worker::ports::GitHubError;
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
    assert_eq!(starts[0]["heartbeatSeconds"], 300);
    assert_eq!(*world.runner.started.lock().unwrap(), vec![run_id]);
    assert!(world.pusher.pushed.lock().unwrap().contains(&run_id));
    assert_eq!(
        world.trigger_event("delivery-35").await,
        (Some("started".into()), None)
    );
    assert_eq!(world.dynamo.all::<InboundEvent>().await.len(), 1);
    let events = world.dynamo.all::<TriggerEvent>().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].task_id, Some(task.id));
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
            Some("the actor eve is not allowed by the trigger".into())
        )
    );
    assert!(world.tasks().await.is_empty());
}

#[tokio::test]
async fn unrelated_events_leave_no_trigger_event() {
    let world = World::new("pro").await;
    let mut payload = common::labeled_payload(41, "Other repo", "mika-dev");
    payload["repository"] = json!({ "id": 7999, "full_name": "acme/elsewhere" });
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

async fn legacy_pattern(world: &World, name: &str, pattern: serde_json::Value) {
    world
        .dynamo
        .modify::<Trigger>(
            &keys::trigger_key(world.tenant(), world.seeded.config_id, name),
            |trigger| trigger.event_pattern = Some(pattern),
        )
        .await;
}

async fn pending_markers(world: &World) -> usize {
    world
        .store
        .query::<Pending>(&keys::inbound_pk(world.tenant()), "PEND#")
        .await
        .unwrap()
        .len()
}

async fn record_message(world: &World, source: &str, key: &str, event: serde_json::Value) {
    retry(20, || async {
        let mut tx = world.store.tx(world.tenant());
        sanchoris_core::channel::record_inbound_event(&mut tx, source, key, &event).await?;
        tx.commit().await
    })
    .await
    .unwrap();
}

fn web_message(thread: &str, text: &str) -> serde_json::Value {
    json!({ "source": "web", "detail-type": "Message", "detail": { "scope": "all", "thread": thread, "user": "Mika", "text": text } })
}

#[tokio::test]
async fn comments_need_the_commenter_as_actor() {
    let world = World::new("pro").await;
    legacy_pattern(
        &world,
        "labeled-issue",
        json!({
            "source": ["github"],
            "detail-type": ["Issue Comment Created"],
            "detail": {
                "repository": [REPO],
                "issue": { "author_association": ["MEMBER"] },
            },
        }),
    )
    .await;
    world
        .deliver(
            "delivery-comment-eve",
            "issue_comment",
            &common::comment_payload(42, "eve", "NONE"),
        )
        .await;
    world.worker.settle().await.unwrap();
    assert_eq!(
        world.trigger_event("delivery-comment-eve").await,
        (
            Some("skipped".into()),
            Some("the trigger does not constrain the actor of Issue Comment Created events".into())
        )
    );
    legacy_pattern(
        &world,
        "labeled-issue",
        json!({
            "source": ["github"],
            "detail-type": ["Issue Comment Created"],
            "detail": {
                "repository": [REPO],
                "comment": { "author_association": ["OWNER", "MEMBER"] },
            },
        }),
    )
    .await;
    world
        .deliver(
            "delivery-comment-eve-2",
            "issue_comment",
            &common::comment_payload(43, "eve", "NONE"),
        )
        .await;
    world
        .deliver(
            "delivery-comment-mika",
            "issue_comment",
            &common::comment_payload(44, "mika-dev", "MEMBER"),
        )
        .await;
    world.worker.settle().await.unwrap();
    assert_eq!(
        world.trigger_event("delivery-comment-eve-2").await,
        (
            Some("skipped".into()),
            Some("the actor eve (NONE) is not allowed by the trigger".into())
        )
    );
    assert_eq!(
        world.trigger_event("delivery-comment-mika").await,
        (Some("started".into()), None)
    );
    assert_eq!(
        world
            .tasks()
            .await
            .into_iter()
            .map(|t| t.name)
            .collect::<Vec<_>>(),
        vec![format!("{REPO}#44")]
    );
}

#[tokio::test]
async fn unreadable_issue_relations_skip_the_event_and_later_events_run() {
    let world = World::new("pro").await;
    world.github.state.lock().unwrap().relation_errors.insert(
        (REPO.to_string(), 50),
        GitHubError::Status {
            status: 404,
            body: "{\"message\":\"Not Found\"}".into(),
        },
    );
    world
        .label_issue("delivery-50", 50, "Missing issue", "mika-dev")
        .await;
    world
        .label_issue("delivery-51", 51, "Next issue", "mika-dev")
        .await;
    world.worker.settle().await.unwrap();
    assert_eq!(
        world.trigger_event("delivery-50").await,
        (
            Some("skipped".into()),
            Some(format!(
                "children limit: the parent and blockers of {REPO}#50 could not be read: GitHub returned 404: {{\"message\":\"Not Found\"}}"
            ))
        )
    );
    assert_eq!(world.task(&format!("{REPO}#51")).await.status, "RUNNING");
    assert_eq!(pending_markers(&world).await, 0);
}

#[tokio::test]
async fn a_failing_trigger_is_discarded_without_blocking_the_queue() {
    let world = World::new("pro").await;
    world
        .dynamo
        .modify::<Trigger>(
            &keys::trigger_key(world.tenant(), world.seeded.config_id, "labeled-issue"),
            |trigger| trigger.start = json!({ "Session": 5 }),
        )
        .await;
    world
        .label_issue("delivery-60", 60, "Broken trigger", "mika-dev")
        .await;
    record_message(
        &world,
        "web",
        "after-poison",
        web_message("thread-9", "Still there?"),
    )
    .await;
    world.worker.settle().await.unwrap();
    let (result, reason) = world.trigger_event("delivery-60").await;
    assert_eq!(result.as_deref(), Some("discarded"));
    assert!(
        reason
            .as_deref()
            .unwrap()
            .starts_with("the trigger failed on this event: "),
        "{reason:?}"
    );
    assert_eq!(world.task("thread-9/1").await.status, "RUNNING");
    assert_eq!(pending_markers(&world).await, 0);
    assert!(
        world
            .dynamo
            .all::<InboundEvent>()
            .await
            .iter()
            .all(|e| e.processed_at.is_some())
    );
}

#[tokio::test]
async fn an_unreadable_inbound_event_is_recorded_with_its_error() {
    let world = World::new("pro").await;
    record_message(
        &world,
        "schedule",
        "no-trigger",
        json!({ "source": "schedule", "detail-type": "Scheduled Event", "detail": {} }),
    )
    .await;
    record_message(&world, "web", "after", web_message("thread-8", "Hello")).await;
    world.worker.settle().await.unwrap();
    let inbound = world
        .dynamo
        .all::<InboundEvent>()
        .await
        .into_iter()
        .find(|e| e.key == "no-trigger")
        .unwrap();
    assert!(inbound.processed_at.is_some());
    assert_eq!(
        inbound.error.as_deref(),
        Some("a schedule event needs detail.trigger_id")
    );
    assert_eq!(world.task("thread-8/1").await.status, "RUNNING");
    assert_eq!(pending_markers(&world).await, 0);
}

fn bot_labeled(number: i64) -> serde_json::Value {
    let mut payload = common::labeled_payload(number, "Filed by the agent", common::APP_LOGIN);
    payload["sender"]["type"] = json!("Bot");
    payload
}

#[tokio::test]
async fn bot_issues_without_a_parent_task_are_skipped() {
    let world = World::new("pro").await;
    world
        .deliver("delivery-bot-61", "issues", &bot_labeled(61))
        .await;
    world.worker.settle().await.unwrap();
    assert_eq!(
        world.trigger_event("delivery-bot-61").await,
        (
            Some("skipped".into()),
            Some(format!(
                "children limit: sanchoris[bot] started this event on {REPO}#61, which is not a sub-issue of a task's issue"
            ))
        )
    );
    world
        .label_issue("delivery-35", 35, "Parent", "mika-dev")
        .await;
    world.worker.settle().await.unwrap();
    world.github.set_parent(62, Some(35));
    world
        .deliver("delivery-bot-62", "issues", &bot_labeled(62))
        .await;
    world.worker.settle().await.unwrap();
    assert_eq!(
        world.trigger_event("delivery-bot-62").await,
        (Some("started".into()), None)
    );
}

#[tokio::test]
async fn starts_beyond_the_hourly_limit_are_discarded() {
    let world = World::new("pro").await;
    let trigger = trigger(&world, "labeled-issue").await;
    let limit = sanchoris_worker::Settings::default().trigger_starts_per_hour;
    world
        .dynamo
        .put(&TriggerRate {
            tenant_id: world.tenant(),
            trigger_id: trigger.id,
            window_start: common::epoch(),
            starts: limit - 1,
            discarded: 0,
            expires_at: common::epoch() + chrono::Duration::days(1),
        })
        .await;
    for number in [70, 71, 72] {
        world
            .label_issue(&format!("delivery-{number}"), number, "Burst", "mika-dev")
            .await;
    }
    world.worker.settle().await.unwrap();
    assert_eq!(
        world.trigger_event("delivery-70").await,
        (Some("started".into()), None)
    );
    let reason = format!(
        "receive limit: trigger labeled-issue already started {limit} tasks in the hour from 2026-10-27T09:00:00Z"
    );
    for delivery in ["delivery-71", "delivery-72"] {
        assert_eq!(
            world.trigger_event(delivery).await,
            (Some("discarded".into()), Some(reason.clone()))
        );
    }
    let audits = sanchoris_core::audit::audit_events(&mut world.store.tx(world.tenant()))
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.action == "trigger.receive_limit")
        .count();
    assert_eq!(audits, 1);
    world.clock.advance(chrono::Duration::hours(1));
    world
        .label_issue("delivery-73", 73, "Next hour", "mika-dev")
        .await;
    world.worker.settle().await.unwrap();
    assert_eq!(
        world.trigger_event("delivery-73").await,
        (Some("started".into()), None)
    );
}

#[tokio::test]
async fn chat_messages_to_a_pending_task_join_its_start_instruction() {
    let world = World::new("pro").await;
    let tenant_key = keys::tenant_key(world.tenant());
    world
        .dynamo
        .modify::<Tenant>(&tenant_key, |tenant| tenant.running_tasks = 1000)
        .await;
    for (key, text) in [
        ("p1", "Plan the next release"),
        ("p2", "Also check the changelog"),
    ] {
        record_message(&world, "web", key, web_message("thread-p", text)).await;
        world.worker.settle().await.unwrap();
    }
    assert_eq!(world.task("thread-p/1").await.status, "PENDING");
    assert_eq!(
        world.trigger_event("p2").await,
        (
            Some("discarded".into()),
            Some("added to the start instruction of the pending task thread-p/1".into())
        )
    );
    world
        .dynamo
        .modify::<Tenant>(&tenant_key, |tenant| tenant.running_tasks = 0)
        .await;
    retry(20, || async {
        let mut tx = world.store.tx(world.tenant());
        tx.emit(Job::Dispatch {
            tenant_id: world.tenant(),
        });
        tx.commit().await
    })
    .await
    .unwrap();
    world.worker.settle().await.unwrap();
    let task = world.task("thread-p/1").await;
    assert_eq!(task.status, "RUNNING");
    let starts = world.outbox(task.run_id.unwrap(), "start").await;
    let instruction = starts[0]["instruction"].as_str().unwrap();
    assert!(
        instruction.ends_with("\n\n[sanchoris] Message from Mika:\nAlso check the changelog"),
        "{instruction}"
    );
}

#[tokio::test]
async fn missed_schedule_occurrences_fire_once() {
    let world = World::new("pro").await;
    {
        let mut state = world.github.state.lock().unwrap();
        state.head_sha = "config-2".into();
        let yaml = format!(
            "{}  minutely:\n    State: ENABLED\n    ScheduleExpression: rate(1 minute)\n    Start:\n      Session: all\n      Instruction: 'Triage new issues.'\n      Skills: []\n      RequiredSkills: []\n      MaxNudges: 0\n      Limits: {{ Turns: 10, CostUsd: 5, TimeoutSeconds: 3600, HeartbeatSeconds: 600, Children: {{ MaxDepth: 1, MaxCount: 5 }} }}\n",
            common::CONFIG_YAML
        );
        state.files.insert("sanchoris.yaml".into(), yaml);
    }
    world.resync_config().await;
    world.worker.settle().await.unwrap();
    world.clock.advance(chrono::Duration::hours(2));
    world.worker.settle().await.unwrap();
    let names = world
        .tasks()
        .await
        .into_iter()
        .map(|t| t.name)
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["minutely@2026-10-27T09:01:00Z".to_string()]);
    assert_eq!(
        trigger(&world, "minutely").await.next_run_at,
        Some(common::epoch() + chrono::Duration::hours(2) + chrono::Duration::minutes(1))
    );
}

fn with_extra_sessions(count: usize) -> String {
    let extra = (0..count)
        .map(|i| {
            format!(
                "  extra-{i:03}:\n    Worker: {{ Agent: claude, Model: claude-sonnet-5-5, Credential: acme-claude-team }}\n    Repos: [acme/idea]\n    GitHub: {{ Repos: [acme/idea], Permissions: {{ contents: read }} }}\n    Network: {{ Allow: [github.com] }}\n    UsageLimit: {{ Resume: auto }}\n    Workspace: {{ Compute: standard, Architecture: arm64, IdleSeconds: 900 }}\n"
            )
        })
        .collect::<String>();
    common::CONFIG_YAML.replacen("Sessions:\n", &format!("Sessions:\n{extra}"), 1)
}

fn push_config(world: &World, sha: &str, yaml: String) {
    let mut state = world.github.state.lock().unwrap();
    state.head_sha = sha.into();
    state.files.insert("sanchoris.yaml".into(), yaml);
}

async fn stored_revision(world: &World, sha: &str) -> ConfigRevision {
    world
        .store
        .query::<ConfigRevision>(
            &keys::config_pk(world.tenant(), world.seeded.config_id),
            "REV#",
        )
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.commit_sha == sha)
        .unwrap()
}

async fn sync(world: &World) -> Result<usize, sanchoris_worker::error::WorkerError> {
    world.resync_config().await;
    world
        .worker
        .worker
        .handle(&Envelope::new(Job::ConfigSync {
            tenant_id: world.tenant(),
            config_id: world.seeded.config_id,
        }))
        .await
}

#[tokio::test]
async fn large_configs_activate_in_batches_and_audit_the_commit() {
    let world = World::new("pro").await;
    push_config(&world, "config-big", with_extra_sessions(150));
    world
        .worker
        .worker
        .faults
        .arm("config_sync.between_batches");
    assert!(sync(&world).await.is_err());
    let big = stored_revision(&world, "config-big").await;
    assert_eq!(big.validation_status, "valid");
    assert_ne!(active_revision(&world).await, Some(big.id));
    sync(&world).await.unwrap();
    assert_eq!(active_revision(&world).await, Some(big.id));
    let sessions = world
        .dynamo
        .all::<sanchoris_core::items::SessionDef>()
        .await;
    assert_eq!(sessions.len(), 152);
    assert!(sessions.iter().all(|s| s.config_revision_id == big.id));
    let audits = sanchoris_core::audit::audit_events(&mut world.store.tx(world.tenant()))
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.action == "config.activate")
        .collect::<Vec<_>>();
    let big_audits = audits
        .iter()
        .filter(|a| a.metadata["commit_sha"] == "config-big")
        .collect::<Vec<_>>();
    assert_eq!(big_audits.len(), 1);
    assert_eq!(big_audits[0].metadata["commit_author"], "mika-dev");
    assert_eq!(big_audits[0].metadata["revision"], big.number);
    assert_eq!(big_audits[0].actor_kind, "system");

    push_config(&world, "config-small", common::CONFIG_YAML.to_string());
    sync(&world).await.unwrap();
    let small = stored_revision(&world, "config-small").await;
    assert_eq!(active_revision(&world).await, Some(small.id));
    assert_eq!(
        world
            .dynamo
            .all::<sanchoris_core::items::SessionDef>()
            .await
            .len(),
        2
    );
    world
        .label_issue("delivery-90", 90, "After the big config", "mika-dev")
        .await;
    world.worker.settle().await.unwrap();
    assert_eq!(world.task(&format!("{REPO}#90")).await.status, "RUNNING");
}
