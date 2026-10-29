use std::net::IpAddr;

use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::db::{DbError, TenantTx, tenant_plan};
use crate::items::AuditEvent;
use crate::keys;
use crate::model::{ActorKind, AuditOrigin};

#[derive(Debug, Clone, PartialEq)]
pub struct AuditEntry<'a> {
    pub at: DateTime<Utc>,
    pub actor_kind: ActorKind,
    pub actor_ref: &'a str,
    pub action: &'a str,
    pub target_kind: &'a str,
    pub target_ref: &'a str,
    pub summary: &'a str,
    pub origin: AuditOrigin,
    pub source_ip: Option<IpAddr>,
    pub metadata: Map<String, Value>,
}

pub async fn record_audit(tx: &mut TenantTx, entry: &AuditEntry<'_>) -> Result<Uuid, DbError> {
    let id = Uuid::new_v4();
    insert_audit(tx, id, entry).await?;
    Ok(id)
}

pub async fn record_audit_once(
    tx: &mut TenantTx,
    id: Uuid,
    entry: &AuditEntry<'_>,
) -> Result<bool, DbError> {
    let key = keys::audit_key(tx.tenant_id(), entry.at, id);
    if tx.get_item(&key).await?.is_some() {
        return Ok(false);
    }
    insert_audit(tx, id, entry).await?;
    Ok(true)
}

async fn insert_audit(tx: &mut TenantTx, id: Uuid, entry: &AuditEntry<'_>) -> Result<(), DbError> {
    let tenant_id = tx.tenant_id();
    let (_, plan) = tenant_plan(tx).await?;
    let mut metadata = entry.metadata.clone();
    metadata.insert("origin".into(), json!(entry.origin.as_str()));
    tx.insert(&AuditEvent {
        id,
        tenant_id,
        at: entry.at,
        actor_kind: entry.actor_kind.as_str().to_string(),
        actor_ref: entry.actor_ref.to_string(),
        action: entry.action.to_string(),
        target_kind: entry.target_kind.to_string(),
        target_ref: entry.target_ref.to_string(),
        summary: entry.summary.to_string(),
        source_ip: entry.source_ip.map(|ip| ip.to_string()),
        metadata: Value::Object(metadata),
        expires_at: entry.at + Duration::days(plan.audit_retention_days),
    })
}

pub async fn audit_events(tx: &mut TenantTx) -> Result<Vec<AuditEvent>, DbError> {
    let pk = keys::tenant_pk(tx.tenant_id());
    let items = tx.query_items(crate::item::Table::Audit, &pk, "").await?;
    items
        .iter()
        .map(<AuditEvent as crate::item::Entity>::from_item)
        .collect()
}
