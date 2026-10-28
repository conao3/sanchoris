mod common;

use chrono::Duration;
use common::{REPO, World, turn_end_payload};
use sanchoris_core::channel::RunEventType;
use sanchoris_core::items::{Tenant, UsageAlert, UsageSnapshot};
use sanchoris_core::keys;
use serde_json::json;

async fn tenant(world: &World) -> Tenant {
    world
        .store
        .get::<Tenant>(&keys::tenant_key(world.tenant()))
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn missing_required_skills_are_nudged_until_needs_you_and_a_reply_resumes() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    for turn in 1..=3 {
        world.turn_end(run_id, turn).await;
        world.worker.settle().await.unwrap();
    }
    assert_eq!(
        world.messages(run_id).await,
        vec![
            "[sanchoris] The required skill open-pr-green-ci has not succeeded yet. Run it before you finish. (reminder 1 of 2)".to_string(),
            "[sanchoris] The required skill open-pr-green-ci has not succeeded yet. Run it before you finish. (reminder 2 of 2)".to_string(),
        ]
    );
    let stuck = world.task(&format!("{REPO}#35")).await;
    assert_eq!(stuck.status, "NEEDS_YOU");
    assert_eq!(stuck.nudges, 2);
    assert_eq!(stuck.error.as_deref(), Some("Task.NudgesExhausted"));

    world
        .run_event(
            run_id,
            RunEventType::HumanReply,
            json!({ "member": "Mika", "text": "Skip the flaky test and open the PR." }),
        )
        .await;
    world.worker.settle().await.unwrap();
    let resumed = world.task(&format!("{REPO}#35")).await;
    assert_eq!(resumed.status, "RUNNING");
    assert_eq!(resumed.nudges, 0);
    assert_eq!(resumed.error, None);
    assert_eq!(
        world.messages(run_id).await.last().unwrap(),
        "[sanchoris] Message from Mika:\nSkip the flaky test and open the PR."
    );
}

#[tokio::test]
async fn usage_limits_wait_and_resume_automatically() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    let reset_at = world.clock_now() + Duration::hours(2);
    world
        .run_event(
            run_id,
            RunEventType::TurnEnd,
            turn_end_payload(
                1,
                json!({ "code": "Worker.UsageLimit", "message": "limit", "resetAt": reset_at.timestamp() }),
            ),
        )
        .await;
    world.worker.settle().await.unwrap();
    let waiting = world.task(&format!("{REPO}#35")).await;
    assert_eq!(waiting.status, "WAITING");
    assert_eq!(waiting.wait_reason.as_deref(), Some("usage_limit"));
    assert_eq!(waiting.resume_at, Some(reset_at));
    assert_eq!(waiting.nudges, 0);
    assert!(world.messages(run_id).await.is_empty());

    world.clock.advance(Duration::minutes(119));
    world.worker.settle().await.unwrap();
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "WAITING");

    world.clock.advance(Duration::minutes(1));
    world.worker.settle().await.unwrap();
    let resumed = world.task(&format!("{REPO}#35")).await;
    assert_eq!(resumed.status, "RUNNING");
    assert_eq!(resumed.resume_at, None);
    assert_eq!(
        world.messages(run_id).await,
        vec!["[sanchoris] Continue where you left off.".to_string()]
    );
}

#[tokio::test]
async fn manual_usage_limits_need_you() {
    let world = World::new("pro").await;
    let yaml = common::CONFIG_YAML.replacen(
        "UsageLimit: { Resume: auto }",
        "UsageLimit: { Resume: manual }",
        1,
    );
    {
        let mut state = world.github.state.lock().unwrap();
        state.head_sha = "config-2".into();
        state.files.insert("sanchoris.yaml".into(), yaml);
    }
    world.resync_config().await;
    world.worker.settle().await.unwrap();
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world
        .run_event(
            run_id,
            RunEventType::TurnEnd,
            turn_end_payload(
                1,
                json!({ "code": "Worker.UsageLimit", "message": "limit", "resetAt": (world.clock_now() + Duration::hours(1)).timestamp() }),
            ),
        )
        .await;
    world.worker.settle().await.unwrap();
    let task = world.task(&format!("{REPO}#35")).await;
    assert_eq!(task.status, "NEEDS_YOU");
    assert_eq!(task.error.as_deref(), Some("Worker.UsageLimit"));
}

#[tokio::test]
async fn idle_workspaces_stop_and_restart_for_the_next_message() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world
        .run_event(
            run_id,
            RunEventType::SessionRecord,
            json!({ "sessionId": "session-1", "turn": 1, "key": "records/run/1.jsonl" }),
        )
        .await;
    world.request_skill(run_id, common::skill_input(35)).await;
    world.turn_end(run_id, 1).await;
    world.drive().await;
    assert_eq!(
        world
            .task(&format!("{REPO}#35"))
            .await
            .wait_reason
            .as_deref(),
        Some("skill")
    );
    world.clock.advance(Duration::seconds(899));
    world.worker.settle().await.unwrap();
    assert!(world.runner.stopped.lock().unwrap().is_empty());
    world.clock.advance(Duration::seconds(1));
    world.worker.settle().await.unwrap();
    assert_eq!(*world.runner.stopped.lock().unwrap(), vec![run_id]);
    assert_eq!(tenant(&world).await.workspace_seconds_this_period, 900);

    world.github.state.lock().unwrap().check_results.clear();
    world.github.script_checks(&["success"]);
    world.clock.advance(Duration::minutes(5));
    world.drive().await;
    assert_eq!(world.runner.started.lock().unwrap().len(), 2);
    let starts = world.outbox(run_id, "start").await;
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[1]["instruction"], serde_json::Value::Null);
    assert_eq!(
        starts[1]["resume"],
        json!({ "sessionId": "session-1", "recordKey": "records/run/1.jsonl", "lastTurn": 1 })
    );
}

#[tokio::test]
async fn a_workspace_that_stops_on_its_own_needs_you_with_the_reason() {
    let world = World::new("pro").await;
    world.started_task(35).await;
    let platform_ref = world.runner.refs.lock().unwrap()[0].clone();
    world.worker.settle().await.unwrap();
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "RUNNING");
    let sweeps = *world.runner.sweeps.lock().unwrap();
    assert!(sweeps > 0);

    let reason = "Essential container in task exited; container workspace exit code 1";
    world.runner.exits.lock().unwrap().insert(
        platform_ref,
        sanchoris_worker::ports::WorkspaceExit {
            reason: reason.into(),
        },
    );
    world.clock.advance(Duration::seconds(120));
    world.worker.settle().await.unwrap();
    let task = world.task(&format!("{REPO}#35")).await;
    assert_eq!(task.status, "NEEDS_YOU");
    assert_eq!(task.error.as_deref(), Some("Workspace.Stopped"));
    assert_eq!(task.cause.as_deref(), Some(reason));
    assert!(*world.runner.sweeps.lock().unwrap() > sweeps);
    assert!(world.runner.stopped.lock().unwrap().is_empty());
    assert_eq!(tenant(&world).await.workspace_seconds_this_period, 120);
}

#[tokio::test]
async fn a_workspace_that_stops_before_the_session_starts_gets_the_instruction_again() {
    let world = World::new("pro").await;
    world
        .label_issue("delivery-35", 35, "Fix flaky checkout test", "mika-dev")
        .await;
    world.worker.settle().await.unwrap();
    let task = world.task(&format!("{REPO}#35")).await;
    let run_id = task.run_id.unwrap();
    let first = world.outbox(run_id, "start").await;
    assert_eq!(first.len(), 1);
    assert_ne!(first[0]["instruction"], serde_json::Value::Null);

    let platform_ref = world.runner.refs.lock().unwrap()[0].clone();
    world.runner.exits.lock().unwrap().insert(
        platform_ref,
        sanchoris_worker::ports::WorkspaceExit {
            reason: "CannotPullContainerError".into(),
        },
    );
    world.clock.advance(Duration::seconds(120));
    world.worker.settle().await.unwrap();
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "NEEDS_YOU");

    world
        .run_event(
            run_id,
            RunEventType::HumanReply,
            json!({ "member": "Mika", "text": "Try again." }),
        )
        .await;
    world.worker.settle().await.unwrap();
    assert_eq!(world.runner.started.lock().unwrap().len(), 2);
    let starts = world.outbox(run_id, "start").await;
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[1]["instruction"], first[0]["instruction"]);
    assert_eq!(starts[1]["resume"], serde_json::Value::Null);
}

#[tokio::test]
async fn the_sweep_stops_workspaces_that_no_open_workspace_records() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    let recorded = world.runner.running.lock().unwrap()[0].clone();
    let stray = sanchoris_worker::ports::RunningWorkspace {
        platform_ref: "fake-stray".into(),
        workspace_id: uuid::Uuid::new_v4(),
        ..recorded.clone()
    };
    world.runner.running.lock().unwrap().push(stray);
    world.clock.advance(Duration::seconds(60));
    world.worker.settle().await.unwrap();
    assert_eq!(*world.runner.stopped.lock().unwrap(), vec![run_id]);
    assert_eq!(*world.runner.running.lock().unwrap(), vec![recorded]);
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "RUNNING");
}

#[tokio::test]
async fn turn_limits_need_you_and_interrupt_the_session() {
    let world = World::new("pro").await;
    let yaml = common::CONFIG_YAML.replacen("Limits: { Turns: 60,", "Limits: { Turns: 2,", 1);
    {
        let mut state = world.github.state.lock().unwrap();
        state.head_sha = "config-2".into();
        state.files.insert("sanchoris.yaml".into(), yaml);
    }
    world.resync_config().await;
    world.worker.settle().await.unwrap();
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world.turn_end(run_id, 1).await;
    world.turn_end(run_id, 2).await;
    world.worker.settle().await.unwrap();
    let task = world.task(&format!("{REPO}#35")).await;
    assert_eq!(task.status, "NEEDS_YOU");
    assert_eq!(task.error.as_deref(), Some("Task.TurnLimitExceeded"));
}

#[tokio::test]
async fn heartbeats_keep_running_tasks_alive() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world.clock.advance(Duration::seconds(600));
    world
        .run_event(run_id, RunEventType::Heartbeat, json!({ "busy": true }))
        .await;
    world.worker.settle().await.unwrap();
    world.clock.advance(Duration::seconds(600));
    world.worker.settle().await.unwrap();
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "RUNNING");
    world.clock.advance(Duration::seconds(300));
    world.worker.settle().await.unwrap();
    let task = world.task(&format!("{REPO}#35")).await;
    assert_eq!(task.status, "NEEDS_YOU");
    assert_eq!(task.error.as_deref(), Some("Task.HeartbeatTimeout"));
    assert_eq!(world.outbox(run_id, "interrupt").await.len(), 1);
}

#[tokio::test]
async fn rate_limits_record_usage_snapshots_and_alerts_fire_at_eighty_percent() {
    let world = World::new("free").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    let windows = |utilization: f64| {
        json!({
            "status": "allowed", "resetsAt": null, "rateLimitType": null,
            "windows": {
                "five_hour": { "utilization": utilization, "resetsAt": 1793000000, "status": "allowed" },
                "seven_day": { "utilization": 0.1, "resetsAt": 1793500000, "status": "allowed" },
            },
        })
    };
    world
        .run_event(run_id, RunEventType::RateLimit, windows(0.42))
        .await;
    world
        .run_event(run_id, RunEventType::RateLimit, windows(0.43))
        .await;
    world.worker.settle().await.unwrap();
    world.clock.advance(Duration::seconds(61));
    world
        .run_event(run_id, RunEventType::RateLimit, windows(0.5))
        .await;
    world.worker.settle().await.unwrap();
    let mut snapshots = world.dynamo.all::<UsageSnapshot>().await;
    snapshots.sort_by_key(|s| s.captured_at);
    assert_eq!(
        snapshots
            .iter()
            .map(|s| s.five_hour_utilization)
            .collect::<Vec<_>>(),
        vec![42.0, 50.0]
    );

    assert!(world.dynamo.all::<UsageAlert>().await.is_empty());
    world
        .dynamo
        .modify::<Tenant>(&keys::tenant_key(world.tenant()), |t| {
            t.tasks_this_period = 4
        })
        .await;
    world.worker.settle().await.unwrap();
    world.worker.settle().await.unwrap();
    let alerts = world
        .dynamo
        .all::<UsageAlert>()
        .await
        .into_iter()
        .map(|a| (a.metric, a.used, a.limit_value))
        .collect::<Vec<_>>();
    assert_eq!(alerts, vec![("tasks".into(), 4, 5)]);
}

#[tokio::test]
async fn tasks_wait_pending_at_the_plan_concurrency() {
    let world = World::new("free").await;
    world.label_issue("d-1", 1, "One", "mika-dev").await;
    world.label_issue("d-2", 2, "Two", "mika-dev").await;
    world.worker.settle().await.unwrap();
    let first = world.task(&format!("{REPO}#1")).await;
    let second = world.task(&format!("{REPO}#2")).await;
    assert_eq!(
        (first.status.as_str(), second.status.as_str()),
        ("RUNNING", "PENDING")
    );
    world
        .request_skill(first.run_id.unwrap(), common::skill_input(1))
        .await;
    world.github.script_checks(&["success"]);
    world.drive().await;
    world.turn_end(first.run_id.unwrap(), 1).await;
    world.worker.settle().await.unwrap();
    assert_eq!(world.task(&format!("{REPO}#1")).await.status, "SUCCEEDED");
    assert_eq!(world.task(&format!("{REPO}#2")).await.status, "RUNNING");
}
