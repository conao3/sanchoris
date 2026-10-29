use std::net::IpAddr;

use chrono::{DateTime, Utc};
use serde_json::{Map, json};
use thiserror::Error;
use uuid::Uuid;

use crate::audit::{AuditEntry, record_audit};
use crate::billing::{FREE_PLAN, PAYMENT_PENDING};
use crate::db::{DbError, Store, TenantTx};
use crate::item::{Entity, ItemReader, ItemWriter, Key};
use crate::items::{Guard, Member, Membership, Tenant, User, uniq};
use crate::keys;
use crate::model::{ActorKind, AuditOrigin};

const FREE_TENANT_GITHUB: &str = "free_tenant_github";
const SLUG_LIMIT: usize = 40;
const SLUG_ATTEMPTS: u32 = 100;
const DISCARD_ATTEMPTS: u32 = 8;
pub const PENDING_CHECKOUT_MINUTES: i64 = 60;
pub const RESERVED_SLUGS: [&str; 14] = [
    "settings", "operator", "api", "auth", "webhooks", "health", "runs", "records", "login",
    "tenants", "chat", "tasks", "triggers", "skills",
];

#[derive(Debug, Error)]
pub enum TenantError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("a user and a linked GitHub account can each have one tenant on the free plan")]
    FreeTenantTaken,
    #[error("tenant {0} is still waiting for its checkout")]
    CheckoutPending(Uuid),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PendingSession<'a> {
    pub session_id: &'a str,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewTenant<'a> {
    pub id: Uuid,
    pub name: &'a str,
    pub plan_id: &'a str,
    pub user_id: Uuid,
    pub checkout: Option<PendingSession<'a>>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct PendingCheckout {
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub session_id: String,
    pub expires_at: DateTime<Utc>,
}

impl Entity for PendingCheckout {
    const KIND: &'static str = "pending_checkout";

    fn key(&self) -> Key {
        pending_checkout_key(self.user_id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("user_id", self.user_id)
            .uuid("tenant_id", self.tenant_id)
            .s("session_id", &self.session_id)
            .time("expires_at", self.expires_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            user_id: r.uuid("user_id")?,
            tenant_id: r.uuid("tenant_id")?,
            session_id: r.s("session_id")?,
            expires_at: r.time("expires_at")?,
        })
    }
}

pub fn pending_checkout_key(user_id: Uuid) -> Key {
    keys::uniq_key(uniq::PENDING_CHECKOUT, &user_id.to_string())
}

pub fn free_tenant_key(user_id: Uuid) -> Key {
    keys::uniq_key(uniq::FREE_TENANT, &user_id.to_string())
}

pub fn is_payment_pending(tenant: &Tenant) -> bool {
    tenant.status == "suspended" && tenant.suspended_reason.as_deref() == Some(PAYMENT_PENDING)
}

pub fn free_github_tenant_key(github_user_id: i64) -> Key {
    keys::uniq_key(FREE_TENANT_GITHUB, &github_user_id.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeGitHubHold {
    pub tenant_id: Uuid,
    pub github_user_id: i64,
}

impl Entity for FreeGitHubHold {
    const KIND: &'static str = "free_github_hold";

    fn key(&self) -> Key {
        free_github_hold_key(self.tenant_id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .n("github_user_id", self.github_user_id);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            github_user_id: r.n("github_user_id")?,
        })
    }
}

pub fn free_github_hold_key(tenant_id: Uuid) -> Key {
    Key::main(keys::tenant_pk(tenant_id), "#FREE_GITHUB")
}

fn taken_by_other(guard: Option<&Guard>, tenant: &Tenant) -> bool {
    guard.is_some_and(|guard| guard.target != tenant.id)
}

pub async fn hold_free_tenant(tx: &mut TenantTx, tenant: &Tenant) -> Result<bool, DbError> {
    let user_key = free_tenant_key(tenant.created_by_user_id);
    let user_guard = tx.get::<Guard>(&user_key).await?;
    if taken_by_other(user_guard.as_ref(), tenant) {
        return Ok(false);
    }
    let held = tx
        .get::<FreeGitHubHold>(&free_github_hold_key(tenant.id))
        .await?;
    let linked = tx
        .get::<User>(&keys::user_key(tenant.created_by_user_id))
        .await?
        .and_then(|owner| owner.github_user_id);
    let github = match (held, linked) {
        (None, Some(github_user_id)) => {
            let key = free_github_tenant_key(github_user_id);
            let guard = tx.get::<Guard>(&key).await?;
            if taken_by_other(guard.as_ref(), tenant) {
                return Ok(false);
            }
            Some((key, guard.is_none(), github_user_id))
        }
        _ => None,
    };
    if user_guard.is_none() {
        tx.insert(&Guard::of_tenant(user_key, tenant.id, tenant.id))?;
    }
    if let Some((key, missing, github_user_id)) = github {
        if missing {
            tx.insert(&Guard::of_tenant(key, tenant.id, tenant.id))?;
        }
        tx.insert(&FreeGitHubHold {
            tenant_id: tenant.id,
            github_user_id,
        })?;
    }
    Ok(true)
}

pub async fn release_free_tenant(tx: &mut TenantTx, tenant: &Tenant) -> Result<(), DbError> {
    let user_key = free_tenant_key(tenant.created_by_user_id);
    if tx
        .get::<Guard>(&user_key)
        .await?
        .is_some_and(|guard| guard.target == tenant.id)
    {
        tx.delete(&user_key)?;
    }
    let hold_key = free_github_hold_key(tenant.id);
    if let Some(hold) = tx.get::<FreeGitHubHold>(&hold_key).await? {
        let key = free_github_tenant_key(hold.github_user_id);
        if tx
            .get::<Guard>(&key)
            .await?
            .is_some_and(|guard| guard.target == tenant.id)
        {
            tx.delete(&key)?;
        }
        tx.delete(&hold_key)?;
    }
    Ok(())
}

pub async fn pending_checkout(
    store: &Store,
    user_id: Uuid,
) -> Result<Option<PendingCheckout>, DbError> {
    store.get(&pending_checkout_key(user_id)).await
}

pub async fn drop_pending_checkout(
    store: &Store,
    pending: &PendingCheckout,
) -> Result<(), DbError> {
    let mut tx = store.global_tx();
    let key = pending.key();
    if tx
        .get::<PendingCheckout>(&key)
        .await?
        .is_some_and(|current| current.tenant_id == pending.tenant_id)
    {
        tx.delete(&key)?;
    }
    tx.commit().await
}

pub async fn release_pending_checkout(
    tx: &mut TenantTx,
    tenant: &Tenant,
) -> Result<Option<PendingCheckout>, DbError> {
    let key = pending_checkout_key(tenant.created_by_user_id);
    let pending = tx
        .get::<PendingCheckout>(&key)
        .await?
        .filter(|pending| pending.tenant_id == tenant.id);
    if pending.is_some() {
        tx.delete(&key)?;
    }
    Ok(pending)
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
        if RESERVED_SLUGS.contains(&candidate.as_str()) {
            continue;
        }
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
) -> Result<CreatedTenant, TenantError> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match try_create_tenant(store, tenant).await {
            Err(TenantError::Db(DbError::Conflict)) if attempt < SLUG_ATTEMPTS => {}
            other => return other,
        }
    }
}

async fn try_create_tenant(
    store: &Store,
    tenant: &NewTenant<'_>,
) -> Result<CreatedTenant, TenantError> {
    let tenant_id = tenant.id;
    let (status, reason) = match tenant.checkout {
        Some(_) => ("suspended", Some(PAYMENT_PENDING)),
        None => ("active", None),
    };
    let slug = free_slug(store, &slug_base(tenant.name)).await?;
    let member_id = Uuid::new_v4();
    let mut tx = store.tx(tenant_id);
    let row = Tenant {
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
        stripe_customer_id: None,
        created_by_user_id: tenant.user_id,
        created_at: tenant.now,
        updated_at: tenant.now,
    };
    tx.insert(&row)?;
    if tenant.plan_id == FREE_PLAN && !hold_free_tenant(&mut tx, &row).await? {
        return Err(TenantError::FreeTenantTaken);
    }
    if let Some(checkout) = tenant.checkout {
        let key = pending_checkout_key(tenant.user_id);
        if let Some(existing) = tx.get::<PendingCheckout>(&key).await? {
            return Err(TenantError::CheckoutPending(existing.tenant_id));
        }
        tx.insert(&PendingCheckout {
            user_id: tenant.user_id,
            tenant_id,
            session_id: checkout.session_id.to_string(),
            expires_at: checkout.expires_at,
        })?;
    }
    tx.insert(&Guard::of_tenant(
        keys::uniq_key(uniq::SLUG, &slug),
        tenant_id,
        tenant_id,
    ))?;
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
    if tenant.checkout.is_some() {
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

pub async fn discard_pending_tenant(
    store: &Store,
    tenant_id: Uuid,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<bool, DbError> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match try_discard(store, tenant_id, reason, now).await {
            Err(DbError::Conflict) if attempt < DISCARD_ATTEMPTS => {}
            other => return other,
        }
    }
}

async fn try_discard(
    store: &Store,
    tenant_id: Uuid,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<bool, DbError> {
    let mut tx = store.tx(tenant_id);
    let Some(tenant) = tx.get::<Tenant>(&keys::tenant_key(tenant_id)).await? else {
        return Ok(false);
    };
    if !is_payment_pending(&tenant) {
        return Ok(false);
    }
    let mut metadata = Map::new();
    metadata.insert("reason".into(), json!(reason));
    record_audit(
        &mut tx,
        &AuditEntry {
            at: now,
            actor_kind: ActorKind::System,
            actor_ref: "billing",
            action: "tenant.discard",
            target_kind: "tenant",
            target_ref: &tenant_id.to_string(),
            summary: &format!("Discarded the unpaid tenant {}: {reason}", tenant.name),
            origin: AuditOrigin::System,
            source_ip: None,
            metadata,
        },
    )
    .await?;
    release_pending_checkout(&mut tx, &tenant).await?;
    release_free_tenant(&mut tx, &tenant).await?;
    let slug_key = keys::uniq_key(uniq::SLUG, &tenant.slug);
    tx.get::<Guard>(&slug_key).await?;
    tx.delete(&slug_key)?;
    if let Some(customer) = &tenant.stripe_customer_id {
        let customer_key = keys::uniq_key(uniq::STRIPE_CUSTOMER, customer);
        tx.get::<Guard>(&customer_key).await?;
        tx.delete(&customer_key)?;
    }
    for member in tx
        .query::<Member>(&keys::tenant_pk(tenant_id), "MEMBER#")
        .await?
    {
        tx.delete(&member.key())?;
        tx.delete(&keys::membership_key(member.user_id, tenant_id))?;
    }
    tx.delete(&keys::tenant_key(tenant_id))?;
    tx.commit().await?;
    Ok(true)
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
