use chrono::{DateTime, Duration, DurationRound, SecondsFormat, Utc};
use serde_json::{Map, json};
use uuid::Uuid;

use crate::audit::{AuditEntry, record_audit};
use crate::db::{self, DbError, Store};
use crate::item::{Entity, ItemWriter};
use crate::items::{ReceiveRate, Tenant};
use crate::keys;
use crate::model::{ActorKind, AuditOrigin};
use crate::store::retry;

pub const RECEIVE_WINDOW_SECONDS: i64 = 60;
pub const RECEIVE_RATE_RETENTION_HOURS: i64 = 24;
pub const SOURCE_GITHUB: &str = "github";
pub const SOURCE_CHAT: &str = "web";
pub const SOURCE_REPLY: &str = "reply";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveLimited {
    pub limit: i64,
    pub window_start: DateTime<Utc>,
    pub retry_after_seconds: i64,
}

impl ReceiveLimited {
    pub fn reason(&self) -> String {
        format!(
            "receive limit: the tenant already received {} events in the minute from {}; retry after {} s",
            self.limit,
            self.window_start.to_rfc3339_opts(SecondsFormat::Secs, true),
            self.retry_after_seconds
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    Admitted,
    Dropped(ReceiveLimited),
}

pub fn window_start(now: DateTime<Utc>) -> DateTime<Utc> {
    now.duration_trunc(Duration::seconds(RECEIVE_WINDOW_SECONDS))
        .expect("a minute divides the time")
}

fn rate_fields(tenant_id: Uuid, window_start: DateTime<Utc>, other_counter: &str) -> ItemWriter {
    let mut fields = ItemWriter::new(ReceiveRate::KIND);
    fields
        .uuid("tenant_id", tenant_id)
        .time("window_start", window_start)
        .n(other_counter, 0)
        .ttl(window_start + Duration::hours(RECEIVE_RATE_RETENTION_HOURS));
    fields
}

pub async fn admit_inbound(
    store: &Store,
    tenant_id: Uuid,
    source: &str,
    key: &str,
    now: DateTime<Utc>,
) -> Result<Admission, DbError> {
    let tenant = store
        .get::<Tenant>(&keys::tenant_key(tenant_id))
        .await?
        .ok_or(DbError::TenantNotFound(tenant_id))?;
    let limit = db::plan(store, &tenant.plan_id)
        .await?
        .inbound_events_per_minute;
    let start = window_start(now);
    let rate_key = keys::receive_rate_key(tenant_id, start);
    let received = store
        .increment(
            &rate_key,
            &rate_fields(tenant_id, start, "dropped").finish(),
            "received",
            Some(limit),
        )
        .await?;
    if received.is_some() {
        return Ok(Admission::Admitted);
    }
    let dropped = store
        .increment(
            &rate_key,
            &rate_fields(tenant_id, start, "received").finish(),
            "dropped",
            None,
        )
        .await?
        .expect("an unconditional increment returns the count");
    let window_end = start + Duration::seconds(RECEIVE_WINDOW_SECONDS);
    let limited = ReceiveLimited {
        limit,
        window_start: start,
        retry_after_seconds: (window_end - now).num_seconds().max(1),
    };
    if dropped == 1 {
        audit_first_drop(store, tenant_id, source, key, &limited, now).await?;
    }
    Ok(Admission::Dropped(limited))
}

async fn audit_first_drop(
    store: &Store,
    tenant_id: Uuid,
    source: &str,
    key: &str,
    limited: &ReceiveLimited,
    now: DateTime<Utc>,
) -> Result<(), DbError> {
    let reason = limited.reason();
    let target = tenant_id.to_string();
    let origin = if source == SOURCE_GITHUB {
        AuditOrigin::Github
    } else {
        AuditOrigin::Web
    };
    let mut metadata = Map::new();
    metadata.insert("source".into(), json!(source));
    metadata.insert("key".into(), json!(key));
    metadata.insert("limit".into(), json!(limited.limit));
    metadata.insert(
        "window_start".into(),
        json!(
            limited
                .window_start
                .to_rfc3339_opts(SecondsFormat::Secs, true)
        ),
    );
    retry(8, || async {
        let mut tx = store.tx(tenant_id);
        record_audit(
            &mut tx,
            &AuditEntry {
                at: now,
                actor_kind: ActorKind::System,
                actor_ref: source,
                action: "tenant.receive_limit",
                target_kind: "tenant",
                target_ref: &target,
                summary: &format!("Dropped inbound events over the receive limit: {reason}"),
                origin,
                source_ip: None,
                metadata: metadata.clone(),
            },
        )
        .await?;
        tx.commit().await
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn windows_start_on_the_minute() {
        let now = Utc.with_ymd_and_hms(2026, 10, 27, 9, 30, 42).unwrap();
        assert_eq!(
            window_start(now),
            Utc.with_ymd_and_hms(2026, 10, 27, 9, 30, 0).unwrap()
        );
        let limited = ReceiveLimited {
            limit: 120,
            window_start: window_start(now),
            retry_after_seconds: 18,
        };
        assert_eq!(
            limited.reason(),
            "receive limit: the tenant already received 120 events in the minute from 2026-10-27T09:30:00Z; retry after 18 s"
        );
    }
}
