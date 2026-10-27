use std::net::IpAddr;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::db::{DbError, TenantTx};
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
    let mut metadata = entry.metadata.clone();
    metadata.insert("origin".into(), json!(entry.origin.as_str()));
    Ok(sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO sanchoris.audit_events \
         (tenant_id, at, actor_kind, actor_ref, action, target_kind, target_ref, summary, source_ip, metadata) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::inet, $10) RETURNING id",
    )
    .bind(tenant_id)
    .bind(entry.at)
    .bind(entry.actor_kind.as_str())
    .bind(entry.actor_ref)
    .bind(entry.action)
    .bind(entry.target_kind)
    .bind(entry.target_ref)
    .bind(entry.summary)
    .bind(entry.source_ip.map(|ip| ip.to_string()))
    .bind(Value::Object(metadata))
    .fetch_one(tx.conn())
    .await?)
}
