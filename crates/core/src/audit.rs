use std::net::IpAddr;

use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::db::{DbError, TenantTx, tenant_plan};
use crate::items::AuditEvent;
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
    let tenant_id = tx.tenant_id();
    let (_, plan) = tenant_plan(tx).await?;
    let mut metadata = entry.metadata.clone();
    metadata.insert("origin".into(), json!(entry.origin.as_str()));
    let id = Uuid::new_v4();
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
    })?;
    Ok(id)
}

pub async fn audit_events(tx: &mut TenantTx) -> Result<Vec<AuditEvent>, DbError> {
    let pk = crate::keys::tenant_pk(tx.tenant_id());
    let items = tx.query_items(crate::item::Table::Audit, &pk, "").await?;
    items
        .iter()
        .map(<AuditEvent as crate::item::Entity>::from_item)
        .collect()
}
