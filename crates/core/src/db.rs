use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use thiserror::Error;
use uuid::Uuid;

use crate::model::{TaskDefinition, TriggerEventResult};

#[derive(Debug, Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("failed to serialize the task definition: {0}")]
    Definition(#[from] serde_json::Error),
    #[error("trigger event {0} was already evaluated or does not exist")]
    TriggerEventNotPending(Uuid),
    #[error("tenant {0} does not exist")]
    TenantNotFound(Uuid),
    #[error("{0} conflicted but the existing row is not visible")]
    ConflictNotVisible(&'static str),
}

pub struct TenantTx {
    tenant_id: Uuid,
    tx: Transaction<'static, Postgres>,
}

impl TenantTx {
    pub async fn begin(pool: &PgPool, tenant_id: Uuid) -> Result<Self, DbError> {
        let mut tx = pool.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
            .bind(tenant_id.to_string())
            .execute(&mut *tx)
            .await?;
        Ok(Self { tenant_id, tx })
    }

    pub async fn adopt(
        mut tx: Transaction<'static, Postgres>,
        tenant_id: Uuid,
    ) -> Result<Self, DbError> {
        sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
            .bind(tenant_id.to_string())
            .execute(&mut *tx)
            .await?;
        Ok(Self { tenant_id, tx })
    }

    pub fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }

    pub fn conn(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    pub async fn commit(self) -> Result<(), DbError> {
        self.tx.commit().await?;
        Ok(())
    }

    pub async fn rollback(self) -> Result<(), DbError> {
        self.tx.rollback().await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    New(Uuid),
    Duplicate(Uuid),
}

impl Recorded {
    pub fn id(self) -> Uuid {
        match self {
            Self::New(id) | Self::Duplicate(id) => id,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewTriggerEvent<'a> {
    pub trigger_id: Uuid,
    pub key: &'a str,
    pub event: &'a Value,
}

pub async fn record_trigger_event(
    tx: &mut TenantTx,
    event: &NewTriggerEvent<'_>,
) -> Result<Recorded, DbError> {
    let tenant_id = tx.tenant_id();
    let inserted = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO sanchoris.trigger_events (tenant_id, trigger_id, key, event) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (trigger_id, key) DO NOTHING \
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(event.trigger_id)
    .bind(event.key)
    .bind(event.event)
    .fetch_optional(tx.conn())
    .await?;
    if let Some(id) = inserted {
        return Ok(Recorded::New(id));
    }
    let existing = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM sanchoris.trigger_events WHERE trigger_id = $1 AND key = $2",
    )
    .bind(event.trigger_id)
    .bind(event.key)
    .fetch_optional(tx.conn())
    .await?
    .ok_or(DbError::ConflictNotVisible("trigger event"))?;
    Ok(Recorded::Duplicate(existing))
}

#[derive(Debug, Clone, PartialEq)]
pub enum TriggerEventOutcome {
    Started,
    Skipped { reason: String },
    Discarded { reason: String },
}

impl TriggerEventOutcome {
    fn parts(&self) -> (TriggerEventResult, Option<&str>) {
        match self {
            Self::Started => (TriggerEventResult::Started, None),
            Self::Skipped { reason } => (TriggerEventResult::Skipped, Some(reason)),
            Self::Discarded { reason } => (TriggerEventResult::Discarded, Some(reason)),
        }
    }
}

pub async fn set_trigger_event_outcome(
    tx: &mut TenantTx,
    trigger_event_id: Uuid,
    outcome: &TriggerEventOutcome,
) -> Result<(), DbError> {
    let (result, reason) = outcome.parts();
    let updated = sqlx::query(
        "UPDATE sanchoris.trigger_events \
         SET result = $2, reason = $3, evaluated_at = now() \
         WHERE id = $1 AND result IS NULL",
    )
    .bind(trigger_event_id)
    .bind(result.as_str())
    .bind(reason)
    .execute(tx.conn())
    .await?;
    if updated.rows_affected() != 1 {
        return Err(DbError::TriggerEventNotPending(trigger_event_id));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewTask<'a> {
    pub trigger_id: Uuid,
    pub trigger_event_id: Option<Uuid>,
    pub config_revision_id: Uuid,
    pub name: &'a str,
    pub title: &'a str,
    pub definition: &'a TaskDefinition,
    pub parent_task_id: Option<Uuid>,
    pub channel: Option<&'a Value>,
    pub requested_by_member_id: Option<Uuid>,
    pub blocked_by: &'a [Uuid],
}

pub async fn create_task(tx: &mut TenantTx, task: &NewTask<'_>) -> Result<Recorded, DbError> {
    let tenant_id = tx.tenant_id();
    let definition = serde_json::to_value(task.definition)?;
    let inserted = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO sanchoris.tasks \
         (tenant_id, trigger_id, trigger_event_id, config_revision_id, name, title, status, definition, parent_task_id, channel, requested_by_member_id) \
         VALUES ($1, $2, $3, $4, $5, $6, 'PENDING', $7, $8, $9, $10) \
         ON CONFLICT (trigger_id, name) DO NOTHING \
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(task.trigger_id)
    .bind(task.trigger_event_id)
    .bind(task.config_revision_id)
    .bind(task.name)
    .bind(task.title)
    .bind(&definition)
    .bind(task.parent_task_id)
    .bind(task.channel)
    .bind(task.requested_by_member_id)
    .fetch_optional(tx.conn())
    .await?;
    let Some(id) = inserted else {
        let existing = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM sanchoris.tasks WHERE trigger_id = $1 AND name = $2",
        )
        .bind(task.trigger_id)
        .bind(task.name)
        .fetch_optional(tx.conn())
        .await?
        .ok_or(DbError::ConflictNotVisible("task"))?;
        return Ok(Recorded::Duplicate(existing));
    };
    if !task.blocked_by.is_empty() {
        sqlx::query(
            "INSERT INTO sanchoris.task_dependencies (tenant_id, task_id, blocked_by_task_id) \
             SELECT $1, $2, unnest($3::uuid[])",
        )
        .bind(tenant_id)
        .bind(id)
        .bind(task.blocked_by)
        .execute(tx.conn())
        .await?;
    }
    Ok(Recorded::New(id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchedTask {
    pub task_id: Uuid,
    pub run_id: Uuid,
}

pub async fn dispatch_pending_tasks(
    tx: &mut TenantTx,
    now: DateTime<Utc>,
) -> Result<Vec<DispatchedTask>, DbError> {
    let tenant_id = tx.tenant_id();
    let tenant = sqlx::query(
        "SELECT t.status, t.tasks_this_period, t.workspace_seconds_this_period, \
                p.max_concurrency, p.task_limit, p.workspace_seconds_limit \
         FROM sanchoris.tenants t JOIN sanchoris.plans p ON p.id = t.plan_id \
         WHERE t.id = $1 \
         FOR NO KEY UPDATE OF t",
    )
    .bind(tenant_id)
    .fetch_optional(tx.conn())
    .await?
    .ok_or(DbError::TenantNotFound(tenant_id))?;
    let status: String = tenant.try_get("status")?;
    let tasks_this_period: i32 = tenant.try_get("tasks_this_period")?;
    let workspace_seconds: i64 = tenant.try_get("workspace_seconds_this_period")?;
    let max_concurrency: i32 = tenant.try_get("max_concurrency")?;
    let task_limit: i32 = tenant.try_get("task_limit")?;
    let workspace_seconds_limit: i64 = tenant.try_get("workspace_seconds_limit")?;
    if status != "active" || workspace_seconds >= workspace_seconds_limit {
        return Ok(Vec::new());
    }
    let running = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM sanchoris.tasks WHERE tenant_id = $1 AND status = 'RUNNING'",
    )
    .bind(tenant_id)
    .fetch_one(tx.conn())
    .await?;
    let slots =
        (i64::from(max_concurrency) - running).min(i64::from(task_limit - tasks_this_period));
    if slots <= 0 {
        return Ok(Vec::new());
    }
    let task_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT t.id FROM sanchoris.tasks t \
         WHERE t.tenant_id = $1 AND t.status = 'PENDING' \
           AND NOT EXISTS ( \
             SELECT 1 FROM sanchoris.task_dependencies d \
             JOIN sanchoris.tasks b ON b.id = d.blocked_by_task_id \
             WHERE d.task_id = t.id AND b.status NOT IN ('SUCCEEDED', 'FAILED', 'ABORTED')) \
         ORDER BY t.created_at, t.id \
         LIMIT $2 \
         FOR UPDATE OF t SKIP LOCKED",
    )
    .bind(tenant_id)
    .bind(slots)
    .fetch_all(tx.conn())
    .await?;
    if task_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query(
        "UPDATE sanchoris.tasks \
         SET status = 'RUNNING', started_at = coalesce(started_at, $2), updated_at = $2 \
         WHERE id = ANY($1)",
    )
    .bind(&task_ids)
    .bind(now)
    .execute(tx.conn())
    .await?;
    sqlx::query(
        "UPDATE sanchoris.tenants \
         SET tasks_this_period = tasks_this_period + $2, updated_at = now() \
         WHERE id = $1",
    )
    .bind(tenant_id)
    .bind(i32::try_from(task_ids.len()).expect("slots fit in i32"))
    .execute(tx.conn())
    .await?;
    let rows = sqlx::query(
        "INSERT INTO sanchoris.runs (tenant_id, task_id, session_name, worker, model, status, started_at) \
         SELECT tenant_id, id, \
                definition #>> '{Session,Name}', \
                definition #>> '{Session,Definition,Worker,Agent}', \
                definition #>> '{Session,Definition,Worker,Model}', \
                'creating', $2 \
         FROM sanchoris.tasks WHERE id = ANY($1) \
         ORDER BY created_at, id \
         RETURNING task_id, id",
    )
    .bind(&task_ids)
    .bind(now)
    .fetch_all(tx.conn())
    .await?;
    rows.iter()
        .map(|row| {
            Ok(DispatchedTask {
                task_id: row.try_get("task_id")?,
                run_id: row.try_get("id")?,
            })
        })
        .collect()
}

pub async fn tenants_with_pending_tasks(worker: &mut PgConnection) -> Result<Vec<Uuid>, DbError> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT DISTINCT tenant_id FROM sanchoris.tasks WHERE status = 'PENDING' ORDER BY tenant_id",
    )
    .fetch_all(worker)
    .await?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueJob {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub kind: String,
    pub ref_id: Uuid,
    pub due_at: DateTime<Utc>,
    pub attempts: i32,
}

pub async fn schedule_due_job(
    tx: &mut TenantTx,
    kind: &str,
    ref_id: Uuid,
    due_at: DateTime<Utc>,
) -> Result<Uuid, DbError> {
    let tenant_id = tx.tenant_id();
    Ok(sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO sanchoris.due_jobs (tenant_id, kind, ref_id, due_at) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (kind, ref_id) DO UPDATE SET due_at = EXCLUDED.due_at, attempts = 0 \
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(kind)
    .bind(ref_id)
    .bind(due_at)
    .fetch_one(tx.conn())
    .await?)
}

pub async fn claim_due_jobs(
    worker_tx: &mut PgConnection,
    kind: &str,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<DueJob>, DbError> {
    let rows = sqlx::query(
        "SELECT id, tenant_id, kind, ref_id, due_at, attempts FROM sanchoris.due_jobs \
         WHERE kind = $1 AND due_at <= $2 \
         ORDER BY due_at, id \
         LIMIT $3 \
         FOR UPDATE SKIP LOCKED",
    )
    .bind(kind)
    .bind(now)
    .bind(limit)
    .fetch_all(worker_tx)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(DueJob {
                id: row.try_get("id")?,
                tenant_id: row.try_get("tenant_id")?,
                kind: row.try_get("kind")?,
                ref_id: row.try_get("ref_id")?,
                due_at: row.try_get("due_at")?,
                attempts: row.try_get("attempts")?,
            })
        })
        .collect()
}

pub async fn complete_due_job(worker_tx: &mut PgConnection, job_id: Uuid) -> Result<(), DbError> {
    sqlx::query("DELETE FROM sanchoris.due_jobs WHERE id = $1")
        .bind(job_id)
        .execute(worker_tx)
        .await?;
    Ok(())
}

pub async fn retry_due_job(
    worker_tx: &mut PgConnection,
    job_id: Uuid,
    due_at: DateTime<Utc>,
) -> Result<(), DbError> {
    sqlx::query("UPDATE sanchoris.due_jobs SET due_at = $2, attempts = attempts + 1 WHERE id = $1")
        .bind(job_id)
        .bind(due_at)
        .execute(worker_tx)
        .await?;
    Ok(())
}
