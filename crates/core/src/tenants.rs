use std::net::IpAddr;

use chrono::{DateTime, Utc};
use serde_json::{Map, json};
use uuid::Uuid;

use crate::audit::{AuditEntry, record_audit};
use crate::billing::PAYMENT_PENDING;
use crate::db::{DbError, Store};
use crate::items::{Guard, Member, Membership, Tenant, uniq};
use crate::keys;
use crate::model::{ActorKind, AuditOrigin};
use crate::store::retry;

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

async fn free_slug(store: &Store, base: &str) -> Result<String, DbError> {
    for attempt in 1..=SLUG_ATTEMPTS {
        let candidate = if attempt == 1 {
            base.to_string()
        } else {
            format!("{base}-{attempt}")
        };
        let key = keys::uniq_key(uniq::SLUG, &candidate);
        if store.get::<Guard>(&key).await?.is_none() {
            return Ok(candidate);
        }
    }
    Err(DbError::SlugExhausted(base.to_string()))
}

pub async fn create_tenant(
    store: &Store,
    tenant: &NewTenant<'_>,
) -> Result<CreatedTenant, DbError> {
    let tenant_id = tenant.id;
    let (status, reason) = if tenant.payment_pending {
        ("suspended", Some(PAYMENT_PENDING))
    } else {
        ("active", None)
    };
    let base = slug_base(tenant.name);
    retry(SLUG_ATTEMPTS, || async {
        let slug = free_slug(store, &base).await?;
        let member_id = Uuid::new_v4();
        let mut tx = store.tx(tenant_id);
        tx.insert(&Tenant {
            id: tenant_id,
            slug: slug.clone(),
            name: tenant.name.to_string(),
            plan_id: tenant.plan_id.to_string(),
            period_start: tenant.now,
            tasks_this_period: 0,
            workspace_seconds_this_period: 0,
            running_tasks: 0,
            status: status.to_string(),
            suspended_reason: reason.map(str::to_string),
            stripe_customer_id: tenant.stripe_customer_id.map(str::to_string),
            created_by_user_id: tenant.user_id,
            created_at: tenant.now,
            updated_at: tenant.now,
        })?;
        tx.insert(&Guard::of_tenant(
            keys::uniq_key(uniq::SLUG, &slug),
            tenant_id,
            tenant_id,
        ))?;
        if let Some(customer) = tenant.stripe_customer_id {
            tx.insert(&Guard::of_tenant(
                keys::uniq_key(uniq::STRIPE_CUSTOMER, customer),
                tenant_id,
                tenant_id,
            ))?;
        }
        tx.insert(&Member {
            id: member_id,
            tenant_id,
            user_id: tenant.user_id,
            role: "owner".into(),
            joined_at: tenant.now,
            last_active_at: Some(tenant.now),
        })?;
        tx.insert(&Membership {
            user_id: tenant.user_id,
            tenant_id,
            member_id,
        })?;
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
    })
    .await
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
