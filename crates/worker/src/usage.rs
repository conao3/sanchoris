use chrono::{DateTime, Utc};
use sanchoris_core::db::{TenantTx, plan};
use sanchoris_core::items::{Credential, Tenant, UsageAlert, UsageSnapshot};
use sanchoris_core::keys;
use sanchoris_core::store::{Index, SortKey};
use sanchoris_core::tasks::load_task;
use serde_json::Value;
use uuid::Uuid;

use crate::Worker;
use crate::error::{Result, WorkerError};

pub const ALERT_THRESHOLD_PERCENT: i64 = 80;

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

pub async fn credential_of(tx: &mut TenantTx, task_id: Uuid) -> Result<Credential> {
    let task = load_task(tx, task_id).await?;
    let worker = &task.definition["Session"]["Definition"]["Worker"];
    let name = worker["Credential"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let agent = worker["Agent"].as_str().unwrap_or_default().to_string();
    let mut credentials = tx
        .query::<Credential>(&keys::tenant_pk(tx.tenant_id()), "CRED#")
        .await?;
    credentials.sort_by_key(|a| (a.created_at, a.id));
    let found = if name == sanchoris_config::REQUESTER {
        credentials.into_iter().find(|c| {
            c.scope == "personal"
                && c.member_id.is_some()
                && c.member_id == task.requested_by_member_id
                && c.provider == agent
        })
    } else {
        credentials.into_iter().find(|c| c.name == name)
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
    let tenant_id = tx.tenant_id();
    let mut credential = credential_of(tx, task_id).await?;
    let (five_hour, five_hour_resets_at) = window(payload, "five_hour")?;
    let (seven_day, seven_day_resets_at) = window(payload, "seven_day")?;
    let recent = tx
        .query::<UsageSnapshot>(
            &keys::tenant_pk(tenant_id),
            &keys::usage_snapshot_prefix(credential.id),
        )
        .await?
        .iter()
        .any(|s| s.captured_at > now - worker.settings.usage_snapshot_interval);
    if !recent {
        tx.insert(&UsageSnapshot {
            tenant_id,
            credential_id: credential.id,
            captured_at: now,
            five_hour_utilization: five_hour,
            five_hour_resets_at,
            seven_day_utilization: seven_day,
            seven_day_resets_at,
        })?;
    }
    credential.last_used_at = Some(now);
    credential.updated_at = now;
    tx.put(&credential)?;
    Ok(())
}

pub async fn sweep_alerts(worker: &Worker) -> Result<usize> {
    let tenants = worker
        .store
        .query_index::<Tenant>(Index::Gsi2, keys::TENANTS_INDEX, SortKey::All)
        .await?;
    let mut created = 0;
    for tenant in tenants.into_iter().filter(|t| t.status == "active") {
        let plan = plan(&worker.store, &tenant.plan_id).await?;
        let metrics = [
            ("tasks", tenant.tasks_this_period, plan.task_limit),
            (
                "workspace_seconds",
                tenant.workspace_seconds_this_period,
                plan.workspace_seconds_limit,
            ),
        ];
        let due = metrics
            .iter()
            .filter(|(_, used, limit)| used * 100 >= limit * ALERT_THRESHOLD_PERCENT)
            .collect::<Vec<_>>();
        if due.is_empty() {
            continue;
        }
        let mut tx = worker.tx(tenant.id);
        for (metric, used, limit) in due {
            let alert = UsageAlert {
                tenant_id: tenant.id,
                period_start: tenant.period_start,
                metric: metric.to_string(),
                threshold_percent: ALERT_THRESHOLD_PERCENT,
                used: *used,
                limit_value: *limit,
                created_at: worker.now(),
            };
            let key = keys::usage_alert_key(
                tenant.id,
                tenant.period_start,
                metric,
                ALERT_THRESHOLD_PERCENT,
            );
            if tx.get::<UsageAlert>(&key).await?.is_none() {
                tx.insert(&alert)?;
                created += 1;
            }
        }
        tx.commit().await?;
    }
    Ok(created)
}
