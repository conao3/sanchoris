use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use uuid::Uuid;

use crate::channel::{OutboxType, enqueue_outbox, enqueue_user_message};
use crate::db::{DbError, REF_RUN, TenantTx, locate};
use crate::item::Entity;
use crate::items::{HeldMessage, Run, Task};
use crate::jobs::Job;
use crate::keys;
use crate::lifecycle::TaskDecision;
use crate::model::{TaskStatus, WaitReason};
use crate::skillrun::abort_skill_runs;
use crate::store::Update;

pub const ERROR_APPROVAL_REQUESTED: &str = "Approval.Requested";

pub fn task_branch(task_id: Uuid) -> String {
    format!("sanchoris/{task_id}")
}

pub async fn load_task(tx: &mut TenantTx, task_id: Uuid) -> Result<Task, DbError> {
    let key = keys::task_key(tx.tenant_id(), task_id);
    tx.get::<Task>(&key)
        .await?
        .ok_or_else(|| DbError::Missing(format!("task {task_id}")))
}

pub fn idle_seconds(task: &Task) -> Option<Duration> {
    task.definition["Session"]["Definition"]["Workspace"]["IdleSeconds"]
        .as_f64()
        .map(|seconds| Duration::milliseconds((seconds * 1000.0).round() as i64))
}

pub async fn store_task(tx: &mut TenantTx, task: &Task, now: DateTime<Utc>) -> Result<(), DbError> {
    let tenant_id = tx.tenant_id();
    let previous = tx.get::<Task>(&task.key()).await?;
    let before = previous.as_ref().map(|t| t.status);
    let running = |status: Option<TaskStatus>| i64::from(status == Some(TaskStatus::Running));
    let delta = running(Some(task.status)) - running(before);
    if delta != 0 {
        tx.update(
            &keys::tenant_key(tenant_id),
            Update::new().add("running_tasks", delta),
        )?;
    }
    if before == Some(TaskStatus::Running) && task.status != TaskStatus::Running {
        tx.emit(Job::Dispatch { tenant_id });
    }
    let idle_changed = previous
        .as_ref()
        .is_none_or(|p| p.status != task.status || p.updated_at != task.updated_at);
    if matches!(task.status, TaskStatus::Waiting | TaskStatus::NeedsYou)
        && idle_changed
        && let Some(idle) = idle_seconds(task)
        && let Some(run_id) = latest_run(tx, task.id).await?
    {
        tx.emit_at(
            Job::Reconcile {
                tenant_id,
                task_id: task.id,
                run_id,
            },
            task.updated_at.max(now) + idle,
        );
    }
    tx.put(task)
}

pub async fn runs_of(tx: &mut TenantTx, task_id: Uuid) -> Result<Vec<Run>, DbError> {
    let mut runs = tx
        .query::<Run>(&keys::task_pk(tx.tenant_id(), task_id), "RUN#")
        .await?;
    runs.sort_by_key(|a| (a.started_at, a.id));
    Ok(runs)
}

pub async fn latest_run(tx: &mut TenantTx, task_id: Uuid) -> Result<Option<Uuid>, DbError> {
    Ok(runs_of(tx, task_id).await?.last().map(|run| run.id))
}

pub async fn load_run(tx: &mut TenantTx, run_id: Uuid) -> Result<Option<Run>, DbError> {
    let Some(location) = locate(tx, run_id, REF_RUN).await? else {
        return Ok(None);
    };
    tx.get::<Run>(&location.target).await
}

pub async fn wake_task(
    tx: &mut TenantTx,
    task_id: Uuid,
    now: DateTime<Utc>,
) -> Result<(), DbError> {
    let mut task = load_task(tx, task_id).await?;
    let wakes = task.status == TaskStatus::Running
        || (task.status == TaskStatus::Waiting
            && matches!(
                task.wait_reason,
                Some(WaitReason::Skill) | Some(WaitReason::Chat)
            ));
    if !wakes {
        return Ok(());
    }
    task.status = TaskStatus::Running;
    task.wait_reason = None;
    task.error = None;
    task.cause = None;
    task.updated_at = now;
    store_task(tx, &task, now).await
}

pub fn holds_messages(task: &Task) -> bool {
    task.status == TaskStatus::NeedsYou && task.error.as_deref() != Some(ERROR_APPROVAL_REQUESTED)
}

pub fn hold_message(
    tx: &mut TenantTx,
    task_id: Uuid,
    text: &str,
    dedupe_key: &str,
    now: DateTime<Utc>,
) -> Result<(), DbError> {
    tx.insert(&HeldMessage {
        tenant_id: tx.tenant_id(),
        task_id,
        created_at: now,
        dedupe_key: dedupe_key.to_string(),
        text: text.to_string(),
    })
}

async fn deliver_held_messages(
    tx: &mut TenantTx,
    task_id: Uuid,
    run_id: Uuid,
) -> Result<(), DbError> {
    let held = tx
        .query::<HeldMessage>(&keys::task_pk(tx.tenant_id(), task_id), "HELD#")
        .await?;
    for message in held {
        enqueue_user_message(tx, run_id, &message.text, &message.dedupe_key).await?;
        tx.delete(&message.key())?;
    }
    Ok(())
}

pub async fn send_to_task(
    tx: &mut TenantTx,
    task_id: Uuid,
    run_id: Uuid,
    text: &str,
    dedupe_key: &str,
    now: DateTime<Utc>,
) -> Result<bool, DbError> {
    let inserted = enqueue_user_message(tx, run_id, text, dedupe_key).await?;
    wake_task(tx, task_id, now).await?;
    Ok(inserted)
}

pub async fn apply_decision(
    tx: &mut TenantTx,
    task_id: Uuid,
    run_id: Uuid,
    decision: &TaskDecision,
    dedupe_key: &str,
    now: DateTime<Utc>,
) -> Result<(), DbError> {
    let tenant_id = tx.tenant_id();
    let mut task = load_task(tx, task_id).await?;
    task.updated_at = now;
    match decision {
        TaskDecision::Succeed => {
            task.status = TaskStatus::Succeeded;
            task.wait_reason = None;
            task.resume_at = None;
            task.error = None;
            task.cause = None;
            task.finished_at = Some(now);
            store_task(tx, &task, now).await?;
            abort_skill_runs(tx, task_id, "The task succeeded", now).await?;
            finish_runs(tx, task_id, "completed", now).await?;
        }
        TaskDecision::Wait { reason, resume_at } => {
            task.status = TaskStatus::Waiting;
            task.wait_reason = Some(*reason);
            task.resume_at = *resume_at;
            task.error = None;
            task.cause = None;
            store_task(tx, &task, now).await?;
            if let (WaitReason::UsageLimit, Some(at)) = (reason, resume_at) {
                tx.emit_at(
                    Job::TaskResume {
                        tenant_id,
                        task_id,
                        resume_at: *at,
                    },
                    *at,
                );
            }
        }
        TaskDecision::Nudge { nudges, message } => {
            task.status = TaskStatus::Running;
            task.wait_reason = None;
            task.nudges = i64::from(*nudges);
            store_task(tx, &task, now).await?;
            enqueue_user_message(tx, run_id, message, dedupe_key).await?;
        }
        TaskDecision::NeedsYou { error, cause } => {
            task.status = TaskStatus::NeedsYou;
            task.wait_reason = None;
            task.resume_at = None;
            task.error = Some(error.clone());
            task.cause = Some(cause.clone());
            store_task(tx, &task, now).await?;
        }
        TaskDecision::Resume {
            message,
            reset_nudges,
        } => {
            task.status = TaskStatus::Running;
            task.wait_reason = None;
            task.resume_at = None;
            task.error = None;
            task.cause = None;
            if *reset_nudges {
                task.nudges = 0;
            }
            store_task(tx, &task, now).await?;
            deliver_held_messages(tx, task_id, run_id).await?;
            enqueue_user_message(tx, run_id, message, dedupe_key).await?;
        }
    }
    Ok(())
}

pub async fn abort_task(
    tx: &mut TenantTx,
    task_id: Uuid,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<bool, DbError> {
    let mut task = load_task(tx, task_id).await?;
    if task.status.is_terminal() {
        return Ok(false);
    }
    task.status = TaskStatus::Aborted;
    task.wait_reason = None;
    task.resume_at = None;
    task.snoozed_until = None;
    task.error = None;
    task.cause = None;
    task.finished_at = Some(now);
    task.updated_at = now;
    store_task(tx, &task, now).await?;
    abort_skill_runs(tx, task_id, "The task was stopped", now).await?;
    for run in runs_of(tx, task_id).await? {
        if !run.is_finished() {
            stop_run(tx, run.id, reason, now).await?;
        }
    }
    Ok(true)
}

pub async fn stop_run(
    tx: &mut TenantTx,
    run_id: Uuid,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<bool, DbError> {
    let Some(mut run) = load_run(tx, run_id).await? else {
        return Ok(false);
    };
    if run.is_finished() {
        return Ok(false);
    }
    run.status = "completed".into();
    run.finished_at = Some(now);
    run.updated_at = now;
    tx.put(&run)?;
    enqueue_outbox(
        tx,
        run_id,
        OutboxType::Shutdown,
        &json!({ "reason": reason }),
        "shutdown:stopped",
    )
    .await?;
    tx.emit(Job::Reconcile {
        tenant_id: tx.tenant_id(),
        task_id: run.task_id,
        run_id,
    });
    Ok(true)
}

pub async fn finish_runs(
    tx: &mut TenantTx,
    task_id: Uuid,
    status: &str,
    now: DateTime<Utc>,
) -> Result<(), DbError> {
    for mut run in runs_of(tx, task_id).await? {
        if run.is_finished() {
            continue;
        }
        run.status = status.to_string();
        run.finished_at = Some(now);
        run.updated_at = now;
        tx.put(&run)?;
        enqueue_outbox(
            tx,
            run.id,
            OutboxType::Shutdown,
            &json!({ "reason": format!("the task is {status}") }),
            "shutdown:finished",
        )
        .await?;
        tx.emit(Job::Reconcile {
            tenant_id: tx.tenant_id(),
            task_id,
            run_id: run.id,
        });
    }
    Ok(())
}
