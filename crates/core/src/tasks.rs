use chrono::{DateTime, Utc};
use serde_json::json;
use uuid::Uuid;

use crate::channel::{OutboxType, enqueue_outbox, enqueue_user_message};
use crate::db::{DbError, TenantTx, schedule_due_job};
use crate::lifecycle::TaskDecision;
use crate::model::WaitReason;

pub const DUE_TASK_RESUME: &str = "task_resume";
pub const ERROR_APPROVAL_REQUESTED: &str = "Approval.Requested";

pub fn task_branch(task_id: Uuid) -> String {
    format!("sanchoris/{task_id}")
}

pub async fn latest_run(tx: &mut TenantTx, task_id: Uuid) -> Result<Option<Uuid>, DbError> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM sanchoris.runs WHERE task_id = $1 ORDER BY started_at DESC, id DESC LIMIT 1",
    )
    .bind(task_id)
    .fetch_optional(tx.conn())
    .await?)
}

pub async fn wake_task(
    tx: &mut TenantTx,
    task_id: Uuid,
    now: DateTime<Utc>,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE sanchoris.tasks SET status = 'RUNNING', wait_reason = NULL, error = NULL, cause = NULL, \
         updated_at = $2 \
         WHERE id = $1 AND ((status = 'WAITING' AND wait_reason IN ('skill', 'chat')) OR status = 'RUNNING')",
    )
    .bind(task_id)
    .bind(now)
    .execute(tx.conn())
    .await?;
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
    match decision {
        TaskDecision::Succeed => {
            sqlx::query(
                "UPDATE sanchoris.tasks SET status = 'SUCCEEDED', wait_reason = NULL, resume_at = NULL, \
                 error = NULL, cause = NULL, finished_at = $2, updated_at = $2 WHERE id = $1",
            )
            .bind(task_id)
            .bind(now)
            .execute(tx.conn())
            .await?;
            finish_runs(tx, task_id, "completed", now).await?;
        }
        TaskDecision::Wait { reason, resume_at } => {
            sqlx::query(
                "UPDATE sanchoris.tasks SET status = 'WAITING', wait_reason = $2, resume_at = $3, \
                 error = NULL, cause = NULL, updated_at = $4 WHERE id = $1",
            )
            .bind(task_id)
            .bind(reason.as_str())
            .bind(resume_at)
            .bind(now)
            .execute(tx.conn())
            .await?;
            if let (WaitReason::UsageLimit, Some(at)) = (reason, resume_at) {
                schedule_due_job(tx, DUE_TASK_RESUME, task_id, *at).await?;
            }
        }
        TaskDecision::Nudge { nudges, message } => {
            sqlx::query(
                "UPDATE sanchoris.tasks SET status = 'RUNNING', wait_reason = NULL, nudges = $2, \
                 updated_at = $3 WHERE id = $1",
            )
            .bind(task_id)
            .bind(i32::try_from(*nudges).expect("nudges fit in i32"))
            .bind(now)
            .execute(tx.conn())
            .await?;
            enqueue_user_message(tx, run_id, message, dedupe_key).await?;
        }
        TaskDecision::NeedsYou { error, cause } => {
            sqlx::query(
                "UPDATE sanchoris.tasks SET status = 'NEEDS_YOU', wait_reason = NULL, resume_at = NULL, \
                 error = $2, cause = $3, updated_at = $4 WHERE id = $1",
            )
            .bind(task_id)
            .bind(error)
            .bind(cause)
            .bind(now)
            .execute(tx.conn())
            .await?;
        }
        TaskDecision::Resume {
            message,
            reset_nudges,
        } => {
            sqlx::query(
                "UPDATE sanchoris.tasks SET status = 'RUNNING', wait_reason = NULL, resume_at = NULL, \
                 error = NULL, cause = NULL, nudges = CASE WHEN $2 THEN 0 ELSE nudges END, \
                 updated_at = $3 WHERE id = $1",
            )
            .bind(task_id)
            .bind(reset_nudges)
            .bind(now)
            .execute(tx.conn())
            .await?;
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
    let aborted = sqlx::query_scalar::<_, Uuid>(
        "UPDATE sanchoris.tasks SET status = 'ABORTED', wait_reason = NULL, resume_at = NULL, \
         snoozed_until = NULL, error = NULL, cause = NULL, finished_at = $2, updated_at = $2 \
         WHERE id = $1 AND status NOT IN ('SUCCEEDED', 'FAILED', 'ABORTED') RETURNING id",
    )
    .bind(task_id)
    .bind(now)
    .fetch_optional(tx.conn())
    .await?;
    if aborted.is_none() {
        return Ok(false);
    }
    sqlx::query("DELETE FROM sanchoris.due_jobs WHERE kind = $1 AND ref_id = $2")
        .bind(DUE_TASK_RESUME)
        .bind(task_id)
        .execute(tx.conn())
        .await?;
    let runs = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM sanchoris.runs WHERE task_id = $1 AND status NOT IN ('completed', 'failed') \
         ORDER BY started_at, id",
    )
    .bind(task_id)
    .fetch_all(tx.conn())
    .await?;
    for run_id in runs {
        stop_run(tx, run_id, reason, now).await?;
    }
    Ok(true)
}

pub async fn stop_run(
    tx: &mut TenantTx,
    run_id: Uuid,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<bool, DbError> {
    let stopped = sqlx::query_scalar::<_, Uuid>(
        "UPDATE sanchoris.runs SET status = 'completed', finished_at = $2, updated_at = $2 \
         WHERE id = $1 AND status NOT IN ('completed', 'failed') RETURNING id",
    )
    .bind(run_id)
    .bind(now)
    .fetch_optional(tx.conn())
    .await?;
    if stopped.is_none() {
        return Ok(false);
    }
    enqueue_outbox(
        tx,
        run_id,
        OutboxType::Shutdown,
        &json!({ "reason": reason }),
        "shutdown:stopped",
    )
    .await?;
    Ok(true)
}

pub async fn finish_runs(
    tx: &mut TenantTx,
    task_id: Uuid,
    status: &str,
    now: DateTime<Utc>,
) -> Result<(), DbError> {
    let runs = sqlx::query_scalar::<_, Uuid>(
        "UPDATE sanchoris.runs SET status = $2, finished_at = $3, updated_at = now() \
         WHERE task_id = $1 AND status NOT IN ('completed', 'failed') RETURNING id",
    )
    .bind(task_id)
    .bind(status)
    .bind(now)
    .fetch_all(tx.conn())
    .await?;
    for run_id in runs {
        enqueue_outbox(
            tx,
            run_id,
            OutboxType::Shutdown,
            &json!({ "reason": format!("the task is {status}") }),
            "shutdown:finished",
        )
        .await?;
    }
    Ok(())
}
