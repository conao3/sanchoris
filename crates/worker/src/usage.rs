use chrono::{DateTime, Utc};
use sanchoris_core::db::TenantTx;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::error::{Result, WorkerError};
use crate::{Worker, adopt};

pub const ALERT_THRESHOLD_PERCENT: i32 = 80;

fn window(payload: &Value, name: &str) -> Result<(f64, DateTime<Utc>)> {
    let window = &payload["windows"][name];
    let utilization = window["utilization"]
        .as_f64()
        .ok_or_else(|| WorkerError::Invalid(format!("rate_limit has no {name} utilization")))?;
    let resets_at = window["resetsAt"]
        .as_i64()
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
        .ok_or_else(|| WorkerError::Invalid(format!("rate_limit has no {name} resetsAt")))?;
    Ok(((utilization * 100.0).clamp(0.0, 999.99), resets_at))
}

async fn credential_of(tx: &mut TenantTx, task_id: Uuid) -> Result<Uuid> {
    let row = sqlx::query(
        "SELECT definition #>> '{Session,Definition,Worker,Credential}' AS credential, \
                definition #>> '{Session,Definition,Worker,Agent}' AS agent, requested_by_member_id \
         FROM sanchoris.tasks WHERE id = $1",
    )
    .bind(task_id)
    .fetch_one(tx.conn())
    .await?;
    let name: String = row.try_get("credential")?;
    let agent: String = row.try_get("agent")?;
    let member: Option<Uuid> = row.try_get("requested_by_member_id")?;
    let tenant_id = tx.tenant_id();
    let found = if name == sanchoris_config::REQUESTER {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM sanchoris.credentials WHERE tenant_id = $1 AND scope = 'personal' \
             AND member_id = $2 AND provider = $3 ORDER BY created_at LIMIT 1",
        )
        .bind(tenant_id)
        .bind(member)
        .bind(&agent)
        .fetch_optional(tx.conn())
        .await?
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM sanchoris.credentials WHERE tenant_id = $1 AND name = $2",
        )
        .bind(tenant_id)
        .bind(&name)
        .fetch_optional(tx.conn())
        .await?
    };
    found.ok_or_else(|| WorkerError::Invalid(format!("credential {name} does not exist")))
}

pub async fn record_rate_limit(
    worker: &Worker,
    tx: &mut TenantTx,
    task_id: Uuid,
    _run_id: Uuid,
    payload: &Value,
) -> Result<()> {
    let now = worker.now();
    let credential_id = credential_of(tx, task_id).await?;
    let (five_hour, five_hour_resets_at) = window(payload, "five_hour")?;
    let (seven_day, seven_day_resets_at) = window(payload, "seven_day")?;
    sqlx::query(
        "INSERT INTO sanchoris.usage_snapshots (tenant_id, credential_id, captured_at, five_hour_utilization, \
           five_hour_resets_at, seven_day_utilization, seven_day_resets_at) \
         SELECT $1, $2, $3, $4, $5, $6, $7 \
         WHERE NOT EXISTS (SELECT 1 FROM sanchoris.usage_snapshots \
           WHERE credential_id = $2 AND captured_at > $8)",
    )
    .bind(tx.tenant_id())
    .bind(credential_id)
    .bind(now)
    .bind(five_hour)
    .bind(five_hour_resets_at)
    .bind(seven_day)
    .bind(seven_day_resets_at)
    .bind(now - worker.settings.usage_snapshot_interval)
    .execute(tx.conn())
    .await?;
    sqlx::query(
        "UPDATE sanchoris.credentials SET last_used_at = $2, updated_at = now() WHERE id = $1",
    )
    .bind(credential_id)
    .bind(now)
    .execute(tx.conn())
    .await?;
    Ok(())
}

pub async fn tick_limits_enforcer(worker: &Worker) -> Result<usize> {
    let tenants = sqlx::query(
        "SELECT t.id, t.period_start, t.tasks_this_period, t.workspace_seconds_this_period, \
                p.task_limit, p.workspace_seconds_limit \
         FROM sanchoris.tenants t JOIN sanchoris.plans p ON p.id = t.plan_id \
         WHERE t.status = 'active'",
    )
    .fetch_all(&worker.pool)
    .await?;
    let mut created = 0;
    for row in tenants {
        let tenant_id: Uuid = row.try_get("id")?;
        let period_start: DateTime<Utc> = row.try_get("period_start")?;
        let metrics = [
            (
                "tasks",
                i64::from(row.try_get::<i32, _>("tasks_this_period")?),
                i64::from(row.try_get::<i32, _>("task_limit")?),
            ),
            (
                "workspace_seconds",
                row.try_get::<i64, _>("workspace_seconds_this_period")?,
                row.try_get::<i64, _>("workspace_seconds_limit")?,
            ),
        ];
        let due = metrics
            .iter()
            .filter(|(_, used, limit)| used * 100 >= limit * i64::from(ALERT_THRESHOLD_PERCENT))
            .collect::<Vec<_>>();
        if due.is_empty() {
            continue;
        }
        let tx = worker.begin().await?;
        let mut tx = adopt(tx, tenant_id).await?;
        for (metric, used, limit) in due {
            let inserted = sqlx::query(
                "INSERT INTO sanchoris.usage_alerts (tenant_id, period_start, metric, threshold_percent, used, limit_value) \
                 VALUES ($1, $2, $3, $4, $5, $6) \
                 ON CONFLICT (tenant_id, period_start, metric, threshold_percent) DO NOTHING",
            )
            .bind(tenant_id)
            .bind(period_start)
            .bind(metric)
            .bind(ALERT_THRESHOLD_PERCENT)
            .bind(used)
            .bind(limit)
            .execute(tx.conn())
            .await?;
            created += usize::try_from(inserted.rows_affected()).expect("small count");
        }
        tx.commit().await?;
    }
    Ok(created)
}
