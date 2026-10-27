mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::{REPO, World, skill_input};
use sanchoris_core::items::{OutboxItem, Run, SkillRunItem, StateEntryItem, TriggerEvent};
use sanchoris_core::model::{SkillRunStatus, TaskStatus};
use sanchoris_worker::driver::LocalDriver;
use sanchoris_worker::error::WorkerError;

const CRASH_POINTS: [&str; 5] = [
    "dispatcher.before_commit",
    "timer.before_commit",
    "invocation.before_commit",
    "checks.before_commit",
    "run_event.before_commit",
];

async fn crashy_settle(driver: &LocalDriver) -> usize {
    let mut crashes = 0;
    loop {
        let mut progressed = 0;
        for point in CRASH_POINTS {
            driver.worker.faults.arm(point);
        }
        loop {
            progressed += driver.fire_due().await.unwrap();
            match driver.handle_next().await {
                Ok(Some(handled)) => progressed += handled,
                Ok(None) => break,
                Err(WorkerError::Injected(_)) => {
                    crashes += 1;
                    progressed += 1;
                }
                Err(other) => panic!("{other}"),
            }
        }
        driver.worker.faults.clear();
        progressed += driver.round().await.unwrap();
        if progressed == 0 {
            return crashes;
        }
    }
}

#[derive(Debug, PartialEq)]
struct Summary {
    task_status: String,
    skill_status: String,
    path: Vec<String>,
    messages: Vec<String>,
    execs: usize,
    pulls: usize,
    comments: usize,
    comment_updates: usize,
}

async fn scenario(crash: bool) -> (Summary, usize) {
    let world = World::new("pro").await;
    world.github.script_checks(&["failure", "success"]);
    let mut crashes = 0;
    async fn settle_with(world: &World, crash: bool) -> usize {
        if crash {
            crashy_settle(&world.worker).await
        } else {
            world.worker.settle().await.unwrap();
            0
        }
    }
    world
        .label_issue("delivery-35", 35, "Fix flaky checkout test", "mika-dev")
        .await;
    crashes += settle_with(&world, crash).await;
    let task = world.task(&format!("{REPO}#35")).await;
    let run_id = task.run_id.unwrap();
    world.request_skill(run_id, skill_input(35)).await;
    loop {
        crashes += settle_with(&world, crash).await;
        if world.answer_execs(true).await == 0 {
            break;
        }
    }
    world.continue_skill(run_id, 1).await;
    loop {
        crashes += settle_with(&world, crash).await;
        if world.answer_execs(true).await == 0 {
            break;
        }
    }
    world.turn_end(run_id, 1).await;
    crashes += settle_with(&world, crash).await;

    let skill_runs = world.skill_runs().await;
    assert_eq!(skill_runs.len(), 1);
    let mut entries = world.dynamo.all::<StateEntryItem>().await;
    entries.sort_by_key(|e| e.sequence);
    let task_status = world.task(&format!("{REPO}#35")).await.status;
    let messages = world.messages(run_id).await;
    let execs = world.outbox(run_id, "exec").await.len();
    let state = world.github.state.lock().unwrap();
    let summary = Summary {
        task_status,
        skill_status: skill_runs[0].status.as_str().to_string(),
        path: entries.into_iter().map(|e| e.state_name).collect(),
        messages,
        execs,
        pulls: state.created_pulls.len(),
        comments: state.created_comments.len(),
        comment_updates: state.updated_comments.len(),
    };
    drop(state);
    (summary, crashes)
}

#[tokio::test]
async fn crashes_between_claim_and_commit_converge_without_duplicate_side_effects() {
    let (clean, none) = scenario(false).await;
    let (crashed, crashes) = scenario(true).await;
    assert_eq!(none, 0);
    assert!(crashes >= 5, "only {crashes} crashes were injected");
    assert_eq!(clean.task_status, "SUCCEEDED");
    assert_eq!(clean.skill_status, "SUCCEEDED");
    assert_eq!(
        (clean.pulls, clean.comments, clean.comment_updates),
        (1, 1, 0)
    );
    assert_eq!(clean.execs, 2);
    assert_eq!(crashed, clean);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_workers_never_process_the_same_row_twice() {
    let world = World::new("pro").await;
    let first = Arc::new(world.driver_with_own_worker());
    let second = Arc::new(world.driver_with_own_worker());
    for number in 1..=8 {
        world
            .label_issue(&format!("d-{number}"), number, "Work", "mika-dev")
            .await;
    }
    let both = |a: &Arc<LocalDriver>, b: &Arc<LocalDriver>| {
        let (a, b) = (Arc::clone(a), Arc::clone(b));
        async move {
            let left = tokio::spawn(async move { a.settle().await.unwrap() });
            let right = tokio::spawn(async move { b.settle().await.unwrap() });
            left.await.unwrap() + right.await.unwrap()
        }
    };
    both(&first, &second).await;

    let mut results = BTreeMap::new();
    for event in world.dynamo.all::<TriggerEvent>().await {
        *results.entry(event.result.unwrap()).or_insert(0) += 1;
    }
    assert_eq!(results, BTreeMap::from([("started".to_string(), 8)]));
    let mut statuses = BTreeMap::new();
    for task in world.tasks().await {
        *statuses
            .entry(task.status.as_str().to_string())
            .or_insert(0) += 1;
    }
    assert_eq!(
        statuses,
        BTreeMap::from([("PENDING".to_string(), 3), ("RUNNING".to_string(), 5)])
    );
    let started = world.runner.started.lock().unwrap().clone();
    let mut distinct = started.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!((started.len(), distinct.len()), (5, 5));

    let runs = world.dynamo.all::<Run>().await;
    let mut running = world
        .tasks()
        .await
        .into_iter()
        .filter(|t| t.status == TaskStatus::Running)
        .collect::<Vec<_>>();
    running.sort_by(|a, b| a.name.cmp(&b.name));
    world.github.script_checks(&["success"; 5]);
    for task in &running {
        let number: i64 = task.name.rsplit('#').next().unwrap().parse().unwrap();
        let run = runs.iter().find(|r| r.task_id == task.id).unwrap();
        world.request_skill(run.id, skill_input(number)).await;
    }
    loop {
        both(&first, &second).await;
        if world.answer_execs(true).await == 0 {
            break;
        }
    }
    let succeeded = world
        .dynamo
        .all::<SkillRunItem>()
        .await
        .into_iter()
        .filter(|s| s.status == SkillRunStatus::Succeeded)
        .count();
    assert_eq!(succeeded, 5);
    let mut seen = BTreeMap::new();
    for entry in world.dynamo.all::<StateEntryItem>().await {
        *seen
            .entry((entry.skill_run_id, entry.state_name))
            .or_insert(0) += 1;
    }
    assert!(seen.values().all(|count| *count == 1), "{seen:?}");
    let notifications = world
        .dynamo
        .all::<OutboxItem>()
        .await
        .into_iter()
        .filter(|o| o.kind == "user_message")
        .count();
    assert_eq!(notifications, 5);
    let state = world.github.state.lock().unwrap();
    assert_eq!(state.created_pulls.len(), 5);
    assert_eq!(state.created_comments.len(), 5);
    assert!(state.updated_comments.is_empty());
}
