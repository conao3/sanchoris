mod common;

use std::sync::Arc;

use common::{REPO, World, answer_execs, skill_input, worker_for};
use sanchoris_worker::error::WorkerError;
use sanchoris_worker::{Loop, Worker};
use serde_json::Value;
use sqlx::Row;

const CRASH_POINTS: [(Loop, &str); 5] = [
    (Loop::TriggerDispatcher, "dispatcher.before_commit"),
    (Loop::EngineTimers, "timer.before_commit"),
    (Loop::ResourceInvocations, "invocation.before_commit"),
    (Loop::CheckPoller, "checks.before_commit"),
    (Loop::RunEvents, "run_event.before_commit"),
];

async fn crashy_settle(worker: &Worker) -> usize {
    let mut crashes = 0;
    loop {
        let mut progressed = 0;
        for (which, point) in CRASH_POINTS {
            worker.faults.arm(point);
            match worker.tick(which).await {
                Err(WorkerError::Injected(hit)) => {
                    assert_eq!(hit, point);
                    crashes += 1;
                    progressed += 1;
                }
                Ok(n) => progressed += n,
                Err(other) => panic!("{other}"),
            }
            worker.faults.clear();
            progressed += worker.tick(which).await.unwrap();
        }
        for which in Loop::ALL {
            progressed += worker.tick(which).await.unwrap();
        }
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

    let skill_status: String = sqlx::query_scalar("SELECT status FROM sanchoris.skill_runs")
        .fetch_one(&world.pool)
        .await
        .unwrap();
    let path: Vec<String> =
        sqlx::query_scalar("SELECT state_name FROM sanchoris.state_entries ORDER BY sequence")
            .fetch_all(&world.pool)
            .await
            .unwrap();
    let task_status = world.task(&format!("{REPO}#35")).await.status;
    let messages = world.messages(run_id).await;
    let execs = world.outbox(run_id, "exec").await.len();
    let state = world.github.state.lock().unwrap();
    let summary = Summary {
        task_status,
        skill_status,
        path,
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
    let second = Arc::new(worker_for(
        &world.pool,
        &world.clock,
        &world.github,
        &world.runner,
    ));
    let first = Arc::new(worker_for(
        &world.pool,
        &world.clock,
        &world.github,
        &world.runner,
    ));
    for number in 1..=8 {
        world
            .label_issue(&format!("d-{number}"), number, "Work", "mika-dev")
            .await;
    }
    let both = |a: &Arc<Worker>, b: &Arc<Worker>| {
        let (a, b) = (Arc::clone(a), Arc::clone(b));
        async move {
            let left = tokio::spawn(async move { a.settle().await.unwrap() });
            let right = tokio::spawn(async move { b.settle().await.unwrap() });
            left.await.unwrap() + right.await.unwrap()
        }
    };
    both(&first, &second).await;

    let results: Vec<(String, i64)> =
        sqlx::query_as("SELECT result, count(*) FROM sanchoris.trigger_events GROUP BY result")
            .fetch_all(&world.pool)
            .await
            .unwrap();
    assert_eq!(results, vec![("started".into(), 8)]);
    let statuses: Vec<(String, i64)> = sqlx::query_as(
        "SELECT status, count(*) FROM sanchoris.tasks GROUP BY status ORDER BY status",
    )
    .fetch_all(&world.pool)
    .await
    .unwrap();
    assert_eq!(statuses, vec![("PENDING".into(), 3), ("RUNNING".into(), 5)]);
    let started = world.runner.started.lock().unwrap().clone();
    let mut distinct = started.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!((started.len(), distinct.len()), (5, 5));

    let running = sqlx::query(
        "SELECT t.name, r.id AS run_id FROM sanchoris.tasks t JOIN sanchoris.runs r ON r.task_id = t.id \
         WHERE t.status = 'RUNNING' ORDER BY t.name",
    )
    .fetch_all(&world.pool)
    .await
    .unwrap();
    world.github.script_checks(&["success"; 5]);
    for row in &running {
        let name: String = row.get("name");
        let number: i64 = name.rsplit('#').next().unwrap().parse().unwrap();
        world
            .request_skill(row.get("run_id"), skill_input(number))
            .await;
    }
    loop {
        both(&first, &second).await;
        if answer_execs(&world.pool, &world.github, true).await == 0 {
            break;
        }
    }
    let succeeded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sanchoris.skill_runs WHERE status = 'SUCCEEDED'")
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(succeeded, 5);
    let duplicated_entries: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT skill_run_id, state_name FROM sanchoris.state_entries \
         GROUP BY skill_run_id, state_name HAVING count(*) > 1) d",
    )
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(duplicated_entries, 0);
    let notifications: Vec<Value> =
        sqlx::query_scalar("SELECT payload FROM sanchoris.run_outbox WHERE type = 'user_message'")
            .fetch_all(&world.pool)
            .await
            .unwrap();
    assert_eq!(notifications.len(), 5);
    let state = world.github.state.lock().unwrap();
    assert_eq!(state.created_pulls.len(), 5);
    assert_eq!(state.created_comments.len(), 5);
    assert!(state.updated_comments.is_empty());
}
