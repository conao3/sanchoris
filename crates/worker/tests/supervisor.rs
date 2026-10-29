use crate::common;

use chrono::Duration;
use common::{REPO, World, turn_end_payload};
use sanchoris_core::channel::RunEventType;
use sanchoris_core::items::{Tenant, UsageAlert, UsageSnapshot, Workspace};
use sanchoris_core::keys;
use sanchoris_core::store::{Index, SortKey};
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
        .sequenced_event(
            run_id,
            1,
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
async fn a_usage_limit_resume_delivered_early_is_scheduled_again() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    let reset_at = world.clock_now() + Duration::hours(2);
    world
        .sequenced_event(
            run_id,
            1,
            RunEventType::TurnEnd,
            turn_end_payload(
                1,
                json!({ "code": "Worker.UsageLimit", "message": "limit", "resetAt": reset_at.timestamp() }),
            ),
        )
        .await;
    world.worker.settle().await.unwrap();
    let early = world
        .scheduler
        .take_due(reset_at)
        .into_iter()
        .filter(|message| {
            matches!(
                sanchoris_core::jobs::Envelope::parse(&message.body)
                    .unwrap()
                    .job,
                sanchoris_core::jobs::Job::TaskResume { .. }
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(early.len(), 1);
    world.clock.advance(Duration::minutes(119));
    world
        .worker
        .worker
        .handle_body(&early[0].body)
        .await
        .unwrap();
    world.worker.settle().await.unwrap();
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "WAITING");

    world.clock.advance(Duration::minutes(1));
    world.worker.settle().await.unwrap();
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "RUNNING");
}

#[tokio::test]
async fn a_usage_limit_without_a_reset_time_needs_you() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world
        .sequenced_event(
            run_id,
            1,
            RunEventType::TurnEnd,
            turn_end_payload(
                1,
                json!({ "code": "Worker.UsageLimit", "message": "limit" }),
            ),
        )
        .await;
    world.worker.settle().await.unwrap();
    let task = world.task(&format!("{REPO}#35")).await;
    assert_eq!(task.status, "NEEDS_YOU");
    assert_eq!(task.error.as_deref(), Some("Worker.UsageLimit"));
    assert_eq!(
        task.cause.as_deref(),
        Some("The worker reached its usage limit and did not say when it resets")
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
        .sequenced_event(
            run_id,
            1,
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
            interrupted_at: None,
            stopped_at: None,
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
async fn a_stopped_workspace_counts_until_the_platform_stopped_it() {
    let world = World::new("pro").await;
    world.started_task(35).await;
    let platform_ref = world.runner.refs.lock().unwrap()[0].clone();
    world.clock.advance(Duration::seconds(60));
    let stopped_at = world.clock_now();
    world.runner.exits.lock().unwrap().insert(
        platform_ref,
        sanchoris_worker::ports::WorkspaceExit {
            reason: "Essential container in task exited".into(),
            interrupted_at: None,
            stopped_at: Some(stopped_at),
        },
    );
    world.clock.advance(Duration::seconds(120));
    world.worker.settle().await.unwrap();
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "NEEDS_YOU");
    assert_eq!(workspaces(&world).await[0].stopped_at, Some(stopped_at));
    assert_eq!(tenant(&world).await.workspace_seconds_this_period, 60);
}

#[tokio::test]
async fn a_workspace_that_never_launched_counts_no_seconds() {
    let world = World::new("pro").await;
    world.started_task(35).await;
    let workspace = workspaces(&world).await.remove(0);
    world
        .dynamo
        .modify::<Workspace>(
            &keys::workspace_key(world.tenant(), workspace.task_id, workspace.id),
            |w| w.platform_ref = None,
        )
        .await;
    world.runner.running.lock().unwrap().clear();
    world.clock.advance(Duration::minutes(15));
    world.worker.settle().await.unwrap();
    assert!(workspaces(&world).await[0].stopped_at.is_some());
    assert_eq!(tenant(&world).await.workspace_seconds_this_period, 0);
}

#[tokio::test]
async fn the_sweep_observes_the_workspace_of_a_task_that_needs_you() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    for turn in 1..=3 {
        world.turn_end(run_id, turn).await;
        world.worker.settle().await.unwrap();
    }
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "NEEDS_YOU");
    let platform_ref = world.runner.refs.lock().unwrap()[0].clone();
    world.runner.exits.lock().unwrap().insert(
        platform_ref,
        sanchoris_worker::ports::WorkspaceExit {
            reason: "Essential container in task exited".into(),
            interrupted_at: None,
            stopped_at: None,
        },
    );
    world.clock.advance(Duration::seconds(120));
    world.worker.settle().await.unwrap();
    let recorded = workspaces(&world).await;
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].stopped_at, Some(world.clock_now()));
    assert_eq!(tenant(&world).await.workspace_seconds_this_period, 120);
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "NEEDS_YOU");
}

async fn workspaces(world: &World) -> Vec<Workspace> {
    let mut workspaces = world
        .store
        .query_index::<Workspace>(
            Index::Gsi1,
            &keys::workspaces_index(world.tenant()),
            SortKey::All,
        )
        .await
        .unwrap();
    workspaces.sort_by_key(|w| w.started_at);
    workspaces
}

#[tokio::test]
async fn the_plan_decides_the_workspace_size() {
    for (plan, cpu) in [("starter", 1024), ("pro", 2048)] {
        let world = World::new(plan).await;
        world.started_task(35).await;
        let specs = world.runner.specs.lock().unwrap().clone();
        assert_eq!(
            specs
                .iter()
                .map(|s| (s.cpu, s.memory_mib))
                .collect::<Vec<_>>(),
            vec![(cpu, 4096)],
            "{plan}"
        );
        let recorded = workspaces(&world).await;
        assert_eq!((recorded[0].cpu, recorded[0].memory_mib), (cpu, 4096));
    }
}

#[tokio::test]
async fn a_spot_interruption_restores_the_session_in_a_new_workspace() {
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
    world.worker.settle().await.unwrap();
    let first = workspaces(&world).await;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].architecture, "arm64");
    assert_eq!(
        first[0].capacity_provider.as_deref(),
        Some(common::FAKE_CAPACITY_PROVIDER)
    );

    world.clock.advance(Duration::seconds(300));
    let interrupted_at = world.clock_now();
    let platform_ref = world.runner.refs.lock().unwrap()[0].clone();
    world.runner.exits.lock().unwrap().insert(
        platform_ref,
        sanchoris_worker::ports::WorkspaceExit {
            reason: "Your Spot Task was interrupted. (SpotInterruption)".into(),
            interrupted_at: Some(interrupted_at),
            stopped_at: None,
        },
    );
    world.clock.advance(Duration::seconds(150));
    world.worker.settle().await.unwrap();

    let task = world.task(&format!("{REPO}#35")).await;
    assert_eq!(task.status, "RUNNING");
    assert_eq!(task.error, None);
    assert_eq!(world.runner.started.lock().unwrap().len(), 2);
    let starts = world.outbox(run_id, "start").await;
    assert_eq!(starts.len(), 2);
    assert_eq!(
        starts[1]["resume"],
        json!({ "sessionId": "session-1", "recordKey": "records/run/1.jsonl", "lastTurn": 0 })
    );
    assert_eq!(
        world.messages(run_id).await.last().map(String::as_str),
        Some(sanchoris_core::messages::restored_message().as_str())
    );
    let all = workspaces(&world).await;
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].stopped_at, Some(interrupted_at));
    assert_eq!(all[1].stopped_at, None);
    assert_eq!(tenant(&world).await.workspace_seconds_this_period, 300);
}

#[tokio::test]
async fn a_workspace_whose_agent_reported_sigterm_restores_and_counts_until_the_signal() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world.clock.advance(Duration::seconds(200));
    world
        .sequenced_event(
            run_id,
            1,
            RunEventType::Stopping,
            json!({ "signal": "SIGTERM" }),
        )
        .await;
    world.worker.settle().await.unwrap();
    let signaled = workspaces(&world).await;
    assert_eq!(signaled[0].signaled_at, Some(world.clock_now()));
    world
        .sequenced_event(
            run_id,
            2,
            RunEventType::TurnEnd,
            json!({
                "turn": 1,
                "result": "interrupted",
                "summary": "",
                "error": null,
                "usage": {},
                "totalCostUsd": 0.01,
                "sessionId": "session-1",
            }),
        )
        .await;
    world.worker.settle().await.unwrap();
    assert!(world.messages(run_id).await.is_empty());
    assert_eq!(world.task(&format!("{REPO}#35")).await.nudges, 0);

    let platform_ref = world.runner.refs.lock().unwrap()[0].clone();
    world.runner.exits.lock().unwrap().insert(
        platform_ref,
        sanchoris_worker::ports::WorkspaceExit {
            reason: "Task stopped by user (UserInitiated); container exit code 0".into(),
            interrupted_at: None,
            stopped_at: None,
        },
    );
    world.clock.advance(Duration::seconds(90));
    world.worker.settle().await.unwrap();
    let task = world.task(&format!("{REPO}#35")).await;
    assert_eq!(task.status, "RUNNING");
    assert_eq!(world.runner.started.lock().unwrap().len(), 2);
    assert_eq!(
        world.messages(run_id).await,
        vec![sanchoris_core::messages::restored_message()]
    );
    assert_eq!(tenant(&world).await.workspace_seconds_this_period, 200);
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
    assert_eq!(first[0]["heartbeatSeconds"], json!(300));
    assert_eq!(sanchoris_worker::supervisor::heartbeat_interval(60), 30);
    assert_eq!(sanchoris_worker::supervisor::heartbeat_interval(3600), 300);

    let platform_ref = world.runner.refs.lock().unwrap()[0].clone();
    world.runner.exits.lock().unwrap().insert(
        platform_ref,
        sanchoris_worker::ports::WorkspaceExit {
            reason: "CannotPullContainerError".into(),
            interrupted_at: None,
            stopped_at: None,
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

#[tokio::test]
async fn a_skill_notification_waits_in_the_outbox_while_the_usage_limit_holds() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world.request_skill(run_id, common::skill_input(35)).await;
    let reset_at = world.clock_now() + Duration::hours(2);
    world
        .sequenced_event(
            run_id,
            1,
            RunEventType::TurnEnd,
            turn_end_payload(
                1,
                json!({ "code": "Worker.UsageLimit", "message": "limit", "resetAt": reset_at.timestamp() }),
            ),
        )
        .await;
    world.drive().await;
    assert_eq!(
        world
            .task(&format!("{REPO}#35"))
            .await
            .wait_reason
            .as_deref(),
        Some("usage_limit")
    );
    world.clock.advance(Duration::seconds(900));
    world.worker.settle().await.unwrap();
    assert_eq!(*world.runner.stopped.lock().unwrap(), vec![run_id]);

    world.github.state.lock().unwrap().check_results.clear();
    world.github.script_checks(&["success"]);
    world.clock.advance(Duration::minutes(5));
    world.drive().await;
    assert_eq!(world.skill_runs().await[0].status.as_str(), "SUCCEEDED");
    assert_eq!(world.runner.started.lock().unwrap().len(), 1);
    assert_eq!(
        world
            .task(&format!("{REPO}#35"))
            .await
            .wait_reason
            .as_deref(),
        Some("usage_limit")
    );

    world.clock.advance(Duration::hours(2));
    world.worker.settle().await.unwrap();
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "RUNNING");
    assert_eq!(world.runner.started.lock().unwrap().len(), 2);
}
