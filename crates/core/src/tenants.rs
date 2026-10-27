use std::net::IpAddr;

use chrono::{DateTime, Utc};
use serde_json::{Map, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::audit::{AuditEntry, record_audit};
use crate::billing::PAYMENT_PENDING;
use crate::db::{DbError, TenantTx};
use crate::model::{ActorKind, AuditOrigin};

const SLUG_LIMIT: usize = 40;
const SLUG_ATTEMPTS: u32 = 100;

#[derive(Debug, Clone, PartialEq)]
pub struct NewTenant<'a> {
    pub id: Uuid,
    pub name: &'a str,
    pub plan_id: &'a str,
    pub user_id: Uuid,
    pub stripe_customer_id: Option<&'a str>,
    pub payment_pending: bool,
    pub origin: AuditOrigin,
    pub source_ip: Option<IpAddr>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedTenant {
    pub tenant_id: Uuid,
    pub slug: String,
    pub member_id: Uuid,
}

pub fn slug_base(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= SLUG_LIMIT {
            break;
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "tenant".to_string()
    } else {
        slug
    }
}

pub async fn create_tenant(
    pool: &PgPool,
    tenant: &NewTenant<'_>,
) -> Result<CreatedTenant, DbError> {
    let tenant_id = tenant.id;
    let (status, reason) = if tenant.payment_pending {
        ("suspended", Some(PAYMENT_PENDING))
    } else {
        ("active", None)
    };
    let mut tx = TenantTx::begin(pool, tenant_id).await?;
    let base = slug_base(tenant.name);
    let mut slug = None;
    for attempt in 1..=SLUG_ATTEMPTS {
        let candidate = if attempt == 1 {
            base.clone()
        } else {
            format!("{base}-{attempt}")
        };
        let inserted = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO sanchoris.tenants \
             (id, slug, name, plan_id, period_start, status, suspended_reason, stripe_customer_id, created_by_user_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $5, $5) \
             ON CONFLICT (slug) DO NOTHING RETURNING id",
        )
        .bind(tenant_id)
        .bind(&candidate)
        .bind(tenant.name)
        .bind(tenant.plan_id)
        .bind(tenant.now)
        .bind(status)
        .bind(reason)
        .bind(tenant.stripe_customer_id)
        .bind(tenant.user_id)
        .fetch_optional(tx.conn())
        .await?;
        if inserted.is_some() {
            slug = Some(candidate);
            break;
        }
    }
    let slug = slug.ok_or(DbError::SlugExhausted(base))?;
    let member_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO sanchoris.members (tenant_id, user_id, role, joined_at, last_active_at) \
         VALUES ($1, $2, 'owner', $3, $3) RETURNING id",
    )
    .bind(tenant_id)
    .bind(tenant.user_id)
    .bind(tenant.now)
    .fetch_one(tx.conn())
    .await?;
    let mut metadata = Map::new();
    metadata.insert("plan".into(), json!(tenant.plan_id));
    if tenant.payment_pending {
        metadata.insert("status".into(), json!(PAYMENT_PENDING));
    }
    record_audit(
        &mut tx,
        &AuditEntry {
            at: tenant.now,
            actor_kind: ActorKind::Member,
            actor_ref: &member_id.to_string(),
            action: "tenant.create",
            target_kind: "tenant",
            target_ref: &tenant_id.to_string(),
            summary: &format!("Created tenant {} on plan {}", tenant.name, tenant.plan_id),
            origin: tenant.origin,
            source_ip: tenant.source_ip,
            metadata,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(CreatedTenant {
        tenant_id,
        slug,
        member_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_keep_ascii_words() {
        assert_eq!(slug_base("Acme Corp."), "acme-corp");
        assert_eq!(slug_base("  --Hello__World--  "), "hello-world");
        assert_eq!(slug_base("日本語"), "tenant");
        assert_eq!(slug_base(&"a".repeat(60)).len(), SLUG_LIMIT);
    }
}
