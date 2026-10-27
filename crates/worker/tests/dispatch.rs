mod common;

use common::{REPO, World};
use sanchoris_worker::Loop;
use serde_json::{Value, json};
use sqlx::Row;

const START_TEXT: &str = "[sanchoris] Trigger labeled-issue started this task: Fix flaky checkout test (acme/shop-web#35).\n\
Instruction: Resolve issue 35 in acme/shop-web: Fix flaky checkout test. Write tests for the change.\n\
Skills: open-pr-green-ci (required). Run it with `sanchoris skill run open-pr-green-ci`.";

#[tokio::test]
async fn config_sync_resolves_project_repository_keys() {
    let world = World::new("pro").await;
    let pattern: Value = sqlx::query_scalar(
        "SELECT event_pattern FROM sanchoris.triggers WHERE name = 'labeled-issue'",
    )
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(
        pattern["detail"]["repository"],
        json!(["acme/shop-web", "acme/shop-api"])
    );
    let row = sqlx::query(
        "SELECT c.active_revision_id = r.id AS active, r.validation_status, r.commit_sha \
         FROM sanchoris.configs c JOIN sanchoris.config_revisions r ON r.config_id = c.id \
         WHERE r.commit_sha = 'config-1'",
    )
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert!(row.get::<bool, _>("active"));
    assert_eq!(row.get::<String, _>("validation_status"), "valid");
    let skills: Vec<(String, String)> =
        sqlx::query_as("SELECT name, kind FROM sanchoris.skills ORDER BY name")
            .fetch_all(&world.pool)
            .await
            .unwrap();
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
    sqlx::query("UPDATE sanchoris.configs SET synced_at = NULL")
        .execute(&world.admin)
        .await
        .unwrap();
    assert_eq!(world.worker.tick(Loop::ConfigSyncer).await.unwrap(), 1);
    let row = sqlx::query(
        "SELECT r.validation_status, r.validation_messages, c.active_revision_id = r.id AS active \
         FROM sanchoris.config_revisions r JOIN sanchoris.configs c ON c.id = r.config_id \
         WHERE r.commit_sha = 'config-2'",
    )
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("validation_status"), "invalid");
    assert!(!row.get::<bool, _>("active"));
    let messages: Value = row.get("validation_messages");
    assert!(
        messages
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["Location"] == "Triggers.labeled-issue.Start.Session"),
        "{messages}"
    );
    let session: String = sqlx::query_scalar(
        "SELECT start->>'Session' FROM sanchoris.triggers WHERE name = 'labeled-issue'",
    )
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(session, "shop");
}

#[tokio::test]
async fn labeled_issue_starts_a_running_task_with_the_documented_message() {
    let world = World::new("pro").await;
    world
        .label_issue("delivery-35", 35, "Fix flaky checkout test", "mika-dev")
        .await;
    assert_eq!(world.worker.tick(Loop::TriggerDispatcher).await.unwrap(), 1);
    let task = world.task("acme/shop-web#35").await;
    assert_eq!(task.status, "RUNNING");
    assert_eq!(task.title, "Fix flaky checkout test (acme/shop-web#35)");
    assert_eq!(task.requested_by_member_id, Some(world.seeded.member_id));
    let run_id = task.run_id.expect("a run is created");
    assert_eq!(world.worker.tick(Loop::Workspaces).await.unwrap(), 1);
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
    assert_eq!(
        world.trigger_event("delivery-35").await,
        (Some("started".into()), None)
    );

    world
        .label_issue("delivery-35", 35, "Fix flaky checkout test", "mika-dev")
        .await;
    world.worker.settle().await.unwrap();
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM sanchoris.inbound_events), (SELECT count(*) FROM sanchoris.trigger_events)",
    )
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(counts, (1, 1));
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
    let tasks: i64 = sqlx::query_scalar("SELECT count(*) FROM sanchoris.tasks")
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert_eq!(tasks, 1);
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
    let tasks: i64 = sqlx::query_scalar("SELECT count(*) FROM sanchoris.tasks")
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert_eq!(tasks, 0);
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
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM sanchoris.trigger_events")
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert_eq!(events, 0);
    let processed: bool = sqlx::query_scalar(
        "SELECT processed_at IS NOT NULL FROM sanchoris.inbound_events WHERE key = 'delivery-elsewhere'",
    )
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert!(processed);
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
    let grandchild: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sanchoris.tasks WHERE name = 'acme/shop-web#37'")
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(grandchild, 0);
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
        let pool = world.pool.clone();
        let tenant = world.tenant();
        let event = message(key, text);
        async move {
            let mut tx = sanchoris_core::db::TenantTx::begin(&pool, tenant)
                .await
                .unwrap();
            sanchoris_core::channel::record_inbound_event(&mut tx, "web", key, &event)
                .await
                .unwrap();
            tx.commit().await.unwrap();
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
    sqlx::query("UPDATE sanchoris.tasks SET status = 'SUCCEEDED', finished_at = now() WHERE name = 'thread-1/1'")
        .execute(&world.admin)
        .await
        .unwrap();
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
    sqlx::query("UPDATE sanchoris.configs SET synced_at = NULL")
        .execute(&world.admin)
        .await
        .unwrap();
    world.worker.settle().await.unwrap();
    let tasks: i64 = sqlx::query_scalar("SELECT count(*) FROM sanchoris.tasks")
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert_eq!(tasks, 0);
    world.clock.advance(chrono::Duration::hours(1));
    world.worker.settle().await.unwrap();
    let task = world.task("nightly@2026-10-27T10:00:00Z").await;
    assert_eq!(task.status, "RUNNING");
    world.clock.advance(chrono::Duration::hours(1));
    world.worker.settle().await.unwrap();
    world.task("nightly@2026-10-27T11:00:00Z").await;
}
