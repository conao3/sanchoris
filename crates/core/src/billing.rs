use aws_sdk_dynamodb::types::AttributeValue;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::audit::{AuditEntry, record_audit};
use crate::db::{DbError, Store, TenantTx, plans, tenant};
use crate::item::{Entity, ItemReader, ItemWriter, Key, format_time};
use crate::items::{Guard, Tenant, uniq};
use crate::jobs::Job;
use crate::keys;
use crate::model::{ActorKind, AuditOrigin};
use crate::store::Update;
use crate::tenants::{
    discard_pending_tenant, hold_free_tenant, is_payment_pending, release_free_tenant,
    release_pending_checkout,
};

pub const PAYMENT_PENDING: &str = "payment_pending";
pub const PAYMENT_FAILED: &str = "payment_failed";
pub const FREE_PLAN: &str = "free";
pub const ADMIN_PLAN: &str = "admin";
pub const PRIVILEGED_EMAIL_DOMAIN: &str = "sancode.dev";
pub const EVENT_RETENTION_DAYS: i64 = 30;
pub const PLAN_STATUSES: [&str; 3] = ["active", "trialing", "past_due"];
pub const PERIOD_REASONS: [&str; 2] = ["subscription_create", "subscription_cycle"];
const ATTEMPTS: u32 = 8;

pub fn is_privileged_email(email: &str) -> bool {
    email.rsplit_once('@').is_some_and(|(local, domain)| {
        !local.is_empty() && domain.eq_ignore_ascii_case(PRIVILEGED_EMAIL_DOMAIN)
    })
}

fn admin_ignored(current: &Tenant) -> Option<Outcome> {
    (current.plan_id == ADMIN_PLAN)
        .then(|| Outcome::Ignored(format!("tenant {} is on the {ADMIN_PLAN} plan", current.id)))
}

#[derive(Debug, Error)]
pub enum BillingError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("tenant {0} is not waiting for this payment")]
    TenantNotPayable(Uuid),
    #[error("tenant {0} does not exist")]
    TenantGone(Uuid),
    #[error("no tenant has Stripe customer {0}")]
    UnknownCustomer(String),
    #[error("no plan has Stripe price {0}")]
    UnknownPrice(String),
    #[error("no paid plan has id {0}")]
    UnknownPlan(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StripeEvent<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub created: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Processed,
    Duplicate,
    Ignored(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Payer<'a> {
    pub customer_id: &'a str,
    pub tenant_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkout<'a> {
    pub tenant_id: Uuid,
    pub customer_id: &'a str,
    pub plan_id: &'a str,
    pub paid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invoice<'a> {
    pub billing_reason: &'a str,
    pub period_start: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StripeEventRecord {
    pub id: String,
    pub kind: String,
    pub tenant_id: Option<Uuid>,
    pub outcome: String,
    pub reason: Option<String>,
    pub received_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl Entity for StripeEventRecord {
    const KIND: &'static str = "stripe_event";

    fn key(&self) -> Key {
        event_key(&self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.s("event_id", &self.id)
            .s("event_type", &self.kind)
            .opt_uuid("tenant_id", self.tenant_id)
            .s("outcome", &self.outcome)
            .opt_s("reason", self.reason.as_deref())
            .time("received_at", self.received_at)
            .ttl(self.expires_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.s("event_id")?,
            kind: r.s("event_type")?,
            tenant_id: r.opt_uuid("tenant_id")?,
            outcome: r.s("outcome")?,
            reason: r.opt_s("reason")?,
            received_at: r.time("received_at")?,
            expires_at: DateTime::from_timestamp(r.n(crate::item::TTL)?, 0)
                .ok_or_else(|| DbError::Decode("stripe event expires_at".into()))?,
        })
    }
}

pub fn event_key(event_id: &str) -> Key {
    keys::uniq_key(uniq::STRIPE_EVENT, event_id)
}

fn record(
    event: StripeEvent<'_>,
    tenant_id: Option<Uuid>,
    reason: Option<&str>,
) -> StripeEventRecord {
    let received_at = Utc::now();
    StripeEventRecord {
        id: event.id.to_string(),
        kind: event.kind.to_string(),
        tenant_id,
        outcome: if reason.is_some() {
            "ignored"
        } else {
            "processed"
        }
        .to_string(),
        reason: reason.map(str::to_string),
        received_at,
        expires_at: received_at + Duration::days(EVENT_RETENTION_DAYS),
    }
}

pub async fn recorded(store: &Store, event_id: &str) -> Result<Option<StripeEventRecord>, DbError> {
    store.get(&event_key(event_id)).await
}

pub async fn acknowledge(
    store: &Store,
    event: StripeEvent<'_>,
    tenant_id: Option<Uuid>,
    reason: &str,
) -> Result<(), DbError> {
    let mut tx = store.global_tx();
    tx.insert(&record(event, tenant_id, Some(reason)))?;
    match tx.commit().await {
        Err(DbError::Conflict) => Ok(()),
        other => other,
    }
}

fn s(value: &str) -> AttributeValue {
    AttributeValue::S(value.to_string())
}

async fn audit(
    tx: &mut TenantTx,
    action: &str,
    summary: &str,
    metadata: Value,
) -> Result<(), DbError> {
    let tenant_id = tx.tenant_id().to_string();
    let Value::Object(metadata) = metadata else {
        unreachable!("billing audit metadata is an object");
    };
    record_audit(
        tx,
        &AuditEntry {
            at: Utc::now(),
            actor_kind: ActorKind::System,
            actor_ref: "stripe",
            action,
            target_kind: "tenant",
            target_ref: &tenant_id,
            summary,
            origin: AuditOrigin::System,
            source_ip: None,
            metadata: Map::from_iter(metadata),
        },
    )
    .await?;
    Ok(())
}

async fn retrying<F, Fut>(mut operation: F) -> Result<Outcome, BillingError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Outcome, BillingError>>,
{
    let mut attempt = 0;
    loop {
        attempt += 1;
        match operation().await {
            Err(BillingError::Db(DbError::Conflict)) if attempt < ATTEMPTS => {}
            other => return other,
        }
    }
}

async fn open(
    store: &Store,
    tenant_id: Uuid,
    event: StripeEvent<'_>,
) -> Result<Option<(TenantTx, Tenant)>, BillingError> {
    let mut tx = store.tx(tenant_id);
    if tx
        .get::<StripeEventRecord>(&event_key(event.id))
        .await?
        .is_some()
    {
        return Ok(None);
    }
    match tenant(&mut tx).await {
        Ok(current) => Ok(Some((tx, current))),
        Err(DbError::TenantNotFound(_)) => Err(BillingError::TenantGone(tenant_id)),
        Err(other) => Err(other.into()),
    }
}

async fn close(
    mut tx: TenantTx,
    event: StripeEvent<'_>,
    outcome: Outcome,
) -> Result<Outcome, BillingError> {
    let tenant_id = tx.tenant_id();
    let reason = match &outcome {
        Outcome::Ignored(reason) => Some(reason.as_str()),
        Outcome::Processed | Outcome::Duplicate => None,
    };
    tx.insert(&record(event, Some(tenant_id), reason))?;
    tx.commit().await?;
    Ok(outcome)
}

async fn tenant_of(store: &Store, payer: Payer<'_>) -> Result<Uuid, BillingError> {
    let guard = store
        .get::<Guard>(&keys::uniq_key(uniq::STRIPE_CUSTOMER, payer.customer_id))
        .await?;
    match (guard, payer.tenant_id) {
        (Some(guard), _) => Ok(guard.target),
        (None, Some(tenant_id)) => Ok(tenant_id),
        (None, None) => Err(BillingError::UnknownCustomer(payer.customer_id.to_string())),
    }
}

fn check_customer(current: &Tenant, customer_id: &str) -> Result<(), BillingError> {
    if current
        .stripe_customer_id
        .as_deref()
        .is_some_and(|c| c != customer_id)
    {
        return Err(BillingError::TenantNotPayable(current.id));
    }
    Ok(())
}

fn guard_status(tenant: &Tenant) -> Update {
    Update::new().require("status", s(&tenant.status))
}

fn reset_period(update: Update, start: DateTime<Utc>) -> Update {
    update
        .set_s("period_start", format_time(start))
        .set_n("tasks_this_period", 0)
        .set_n("workspace_seconds_this_period", 0)
}

pub async fn stage_plan_change(
    tx: &mut TenantTx,
    current: &Tenant,
    to: &str,
    at: DateTime<Utc>,
    require_free_slot: bool,
) -> Result<bool, DbError> {
    let mut update = guard_status(current)
        .require("plan_id", s(&current.plan_id))
        .set_s("plan_id", to)
        .set_s("updated_at", format_time(at));
    if to == FREE_PLAN {
        if !hold_free_tenant(tx, current).await? && require_free_slot {
            return Ok(false);
        }
        update = reset_period(update, at);
        if current.status == "suspended"
            && matches!(
                current.suspended_reason.as_deref(),
                Some(PAYMENT_PENDING | PAYMENT_FAILED)
            )
        {
            update = update.set_s("status", "active").remove("suspended_reason");
        }
        release_pending_checkout(tx, current).await?;
    } else if current.plan_id == FREE_PLAN {
        release_free_tenant(tx, current).await?;
    }
    tx.update(&keys::tenant_key(current.id), update)?;
    Ok(true)
}

pub async fn complete_checkout(
    store: &Store,
    event: StripeEvent<'_>,
    checkout: Checkout<'_>,
) -> Result<Outcome, BillingError> {
    let known = plans(store)
        .await?
        .iter()
        .any(|p| p.id == checkout.plan_id && p.stripe_price_id.is_some());
    if !known {
        return Err(BillingError::UnknownPlan(checkout.plan_id.to_string()));
    }
    let tenant_id = checkout.tenant_id;
    let customer_id = checkout.customer_id;
    let plan_id = checkout.plan_id;
    retrying(|| async {
        let (mut tx, current) = match open(store, tenant_id, event).await {
            Ok(Some(opened)) => opened,
            Ok(None) => return Ok(Outcome::Duplicate),
            Err(BillingError::TenantGone(_)) => {
                return Err(BillingError::TenantNotPayable(tenant_id));
            }
            Err(other) => return Err(other),
        };
        if let Some(outcome) = admin_ignored(&current) {
            return close(tx, event, outcome).await;
        }
        check_customer(&current, customer_id)?;
        if current.status != "active" && !is_payment_pending(&current) {
            return Err(BillingError::TenantNotPayable(tenant_id));
        }
        if current.status == "active"
            && current.stripe_customer_id.is_some()
            && current.plan_id == plan_id
        {
            return close(tx, event, Outcome::Processed).await;
        }
        let mut update = guard_status(&current)
            .set_s("stripe_customer_id", customer_id)
            .set_s("updated_at", format_time(Utc::now()));
        if current.stripe_customer_id.is_none() {
            tx.insert(&Guard::of_tenant(
                keys::uniq_key(uniq::STRIPE_CUSTOMER, customer_id),
                tenant_id,
                tenant_id,
            ))?;
        }
        if !checkout.paid {
            tx.update(&keys::tenant_key(tenant_id), update)?;
            audit(
                &mut tx,
                "tenant.checkout_unpaid",
                "Checkout completed without a payment; the tenant waits for it",
                json!({ "stripeCustomerId": customer_id, "plan": plan_id }),
            )
            .await?;
            return close(tx, event, Outcome::Processed).await;
        }
        update = reset_period(update, event.created)
            .set_s("plan_id", plan_id)
            .set_s("status", "active")
            .remove("suspended_reason");
        tx.update(&keys::tenant_key(tenant_id), update)?;
        release_free_tenant(&mut tx, &current).await?;
        release_pending_checkout(&mut tx, &current).await?;
        audit(
            &mut tx,
            "tenant.activate",
            &format!("Checkout completed; the tenant is active on plan {plan_id}"),
            json!({ "stripeCustomerId": customer_id, "from": current.plan_id, "to": plan_id }),
        )
        .await?;
        tx.emit(Job::Dispatch { tenant_id });
        close(tx, event, Outcome::Processed).await
    })
    .await
}

pub async fn end_checkout(
    store: &Store,
    event: StripeEvent<'_>,
    tenant_id: Uuid,
) -> Result<Outcome, BillingError> {
    let reason = format!("Stripe sent {}", event.kind);
    let outcome = if discard_pending_tenant(store, tenant_id, &reason, Utc::now()).await? {
        Outcome::Processed
    } else {
        Outcome::Ignored(format!("tenant {tenant_id} is not waiting for a checkout"))
    };
    if outcome == Outcome::Processed {
        let mut tx = store.global_tx();
        tx.insert(&record(event, Some(tenant_id), None))?;
        match tx.commit().await {
            Err(DbError::Conflict) | Ok(()) => {}
            Err(other) => return Err(other.into()),
        }
    }
    Ok(outcome)
}

pub async fn change_plan(
    store: &Store,
    event: StripeEvent<'_>,
    payer: Payer<'_>,
    status: &str,
    price_id: &str,
) -> Result<Outcome, BillingError> {
    if !PLAN_STATUSES.contains(&status) {
        return Ok(Outcome::Ignored(format!("the subscription is {status}")));
    }
    let tenant_id = tenant_of(store, payer).await?;
    let plan_id = plans(store)
        .await?
        .into_iter()
        .find(|p| p.stripe_price_id.as_deref() == Some(price_id))
        .map(|p| p.id)
        .ok_or_else(|| BillingError::UnknownPrice(price_id.to_string()))?;
    set_plan(store, event, payer, tenant_id, &plan_id).await
}

pub async fn end_subscription(
    store: &Store,
    event: StripeEvent<'_>,
    payer: Payer<'_>,
) -> Result<Outcome, BillingError> {
    let tenant_id = tenant_of(store, payer).await?;
    set_plan(store, event, payer, tenant_id, FREE_PLAN).await
}

async fn set_plan(
    store: &Store,
    event: StripeEvent<'_>,
    payer: Payer<'_>,
    tenant_id: Uuid,
    to: &str,
) -> Result<Outcome, BillingError> {
    retrying(|| async {
        let Some((mut tx, current)) = open(store, tenant_id, event).await? else {
            return Ok(Outcome::Duplicate);
        };
        if let Some(outcome) = admin_ignored(&current) {
            return close(tx, event, outcome).await;
        }
        check_customer(&current, payer.customer_id)?;
        if current.plan_id == to {
            return close(tx, event, Outcome::Processed).await;
        }
        if to == FREE_PLAN && is_payment_pending(&current) {
            let reason = "the tenant is waiting for its checkout".to_string();
            return close(tx, event, Outcome::Ignored(reason)).await;
        }
        stage_plan_change(&mut tx, &current, to, event.created, false).await?;
        audit(
            &mut tx,
            "tenant.plan_change",
            &format!("Plan changed from {} to {to}", current.plan_id),
            json!({ "from": current.plan_id, "to": to }),
        )
        .await?;
        tx.emit(Job::Dispatch { tenant_id });
        close(tx, event, Outcome::Processed).await
    })
    .await
}

pub async fn start_period(
    store: &Store,
    event: StripeEvent<'_>,
    payer: Payer<'_>,
    invoice: Invoice<'_>,
) -> Result<Outcome, BillingError> {
    let tenant_id = tenant_of(store, payer).await?;
    retrying(|| async {
        let Some((mut tx, current)) = open(store, tenant_id, event).await? else {
            return Ok(Outcome::Duplicate);
        };
        if let Some(outcome) = admin_ignored(&current) {
            return close(tx, event, outcome).await;
        }
        check_customer(&current, payer.customer_id)?;
        let resumes = current.status == "suspended"
            && current.suspended_reason.as_deref() == Some(PAYMENT_FAILED);
        let starts = invoice.period_start.filter(|start| {
            PERIOD_REASONS.contains(&invoice.billing_reason) && *start > current.period_start
        });
        if !resumes && starts.is_none() {
            return close(tx, event, Outcome::Processed).await;
        }
        let mut update = guard_status(&current).set_s("updated_at", format_time(Utc::now()));
        if let Some(start) = starts {
            update = reset_period(update, start);
        }
        if resumes {
            update = update.set_s("status", "active").remove("suspended_reason");
        }
        tx.update(&keys::tenant_key(tenant_id), update)?;
        if resumes {
            audit(
                &mut tx,
                "tenant.resume",
                "The invoice was paid; the tenant is active",
                json!({}),
            )
            .await?;
        }
        tx.emit(Job::Dispatch { tenant_id });
        close(tx, event, Outcome::Processed).await
    })
    .await
}

pub async fn fail_payment(
    store: &Store,
    event: StripeEvent<'_>,
    payer: Payer<'_>,
) -> Result<Outcome, BillingError> {
    let tenant_id = tenant_of(store, payer).await?;
    retrying(|| async {
        let Some((mut tx, current)) = open(store, tenant_id, event).await? else {
            return Ok(Outcome::Duplicate);
        };
        if let Some(outcome) = admin_ignored(&current) {
            return close(tx, event, outcome).await;
        }
        check_customer(&current, payer.customer_id)?;
        if current.status != "active" {
            return close(tx, event, Outcome::Processed).await;
        }
        tx.update(
            &keys::tenant_key(tenant_id),
            guard_status(&current)
                .set_s("status", "suspended")
                .set_s("suspended_reason", PAYMENT_FAILED)
                .set_s("updated_at", format_time(Utc::now())),
        )?;
        audit(
            &mut tx,
            "tenant.suspend",
            "The invoice payment failed; new tasks wait in PENDING",
            json!({ "reason": PAYMENT_FAILED }),
        )
        .await?;
        close(tx, event, Outcome::Processed).await
    })
    .await
}
