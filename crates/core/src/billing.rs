use aws_sdk_dynamodb::types::AttributeValue;
use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::audit::{AuditEntry, record_audit};
use crate::db::{DbError, Store, TenantTx, plans, tenant};
use crate::items::{Guard, Tenant, uniq};
use crate::jobs::Job;
use crate::keys;
use crate::model::{ActorKind, AuditOrigin};
use crate::store::Update;

pub const PAYMENT_PENDING: &str = "payment_pending";
pub const PAYMENT_FAILED: &str = "payment_failed";
pub const FREE_PLAN: &str = "free";
const ATTEMPTS: u32 = 8;

#[derive(Debug, Error)]
pub enum BillingError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("no tenant {0} is waiting for a payment")]
    TenantNotPayable(Uuid),
    #[error("no tenant has Stripe customer {0}")]
    UnknownCustomer(String),
    #[error("no plan has Stripe price {0}")]
    UnknownPrice(String),
    #[error("no paid plan has id {0}")]
    UnknownPlan(String),
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

async fn tenant_of_customer(store: &Store, customer_id: &str) -> Result<Uuid, BillingError> {
    let guard = store
        .get::<Guard>(&keys::uniq_key(uniq::STRIPE_CUSTOMER, customer_id))
        .await?
        .ok_or_else(|| BillingError::UnknownCustomer(customer_id.to_string()))?;
    Ok(guard.target)
}

async fn retrying<T, F, Fut>(mut operation: F) -> Result<T, BillingError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, BillingError>>,
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

fn guard_status(tenant: &Tenant) -> Update {
    Update::new().require("status", s(&tenant.status))
}

pub async fn complete_checkout(
    store: &Store,
    tenant_id: Uuid,
    customer_id: &str,
    plan_id: &str,
) -> Result<(), BillingError> {
    retrying(|| async {
        let mut tx = store.tx(tenant_id);
        let current = match tenant(&mut tx).await {
            Ok(current) => current,
            Err(DbError::TenantNotFound(_)) => {
                return Err(BillingError::TenantNotPayable(tenant_id));
            }
            Err(other) => return Err(other.into()),
        };
        if current
            .stripe_customer_id
            .as_deref()
            .is_some_and(|c| c != customer_id)
        {
            return Err(BillingError::TenantNotPayable(tenant_id));
        }
        if current.status == "active"
            && current.stripe_customer_id.is_some()
            && current.plan_id == plan_id
        {
            return Ok(());
        }
        if current.status != "active"
            && current.suspended_reason.as_deref() != Some(PAYMENT_PENDING)
        {
            return Err(BillingError::TenantNotPayable(tenant_id));
        }
        let known = plans(store)
            .await?
            .iter()
            .any(|p| p.id == plan_id && p.stripe_price_id.is_some());
        if !known {
            return Err(BillingError::UnknownPlan(plan_id.to_string()));
        }
        tx.update(
            &keys::tenant_key(tenant_id),
            guard_status(&current)
                .set_s("stripe_customer_id", customer_id)
                .set_s("plan_id", plan_id)
                .set_s("status", "active")
                .remove("suspended_reason")
                .set_s("updated_at", crate::item::format_time(Utc::now())),
        )?;
        if current.stripe_customer_id.is_none() {
            tx.insert(&Guard::of_tenant(
                keys::uniq_key(uniq::STRIPE_CUSTOMER, customer_id),
                tenant_id,
                tenant_id,
            ))?;
        }
        audit(
            &mut tx,
            "tenant.activate",
            &format!("Checkout completed; the tenant is active on plan {plan_id}"),
            json!({ "stripeCustomerId": customer_id, "from": current.plan_id, "to": plan_id }),
        )
        .await?;
        tx.emit(Job::Dispatch { tenant_id });
        tx.commit().await?;
        Ok(())
    })
    .await
}

pub async fn change_plan(
    store: &Store,
    customer_id: &str,
    price_id: &str,
) -> Result<(), BillingError> {
    let tenant_id = tenant_of_customer(store, customer_id).await?;
    let plan_id = plans(store)
        .await?
        .into_iter()
        .find(|p| p.stripe_price_id.as_deref() == Some(price_id))
        .map(|p| p.id)
        .ok_or_else(|| BillingError::UnknownPrice(price_id.to_string()))?;
    set_plan(store, tenant_id, &plan_id).await
}

pub async fn end_subscription(store: &Store, customer_id: &str) -> Result<(), BillingError> {
    let tenant_id = tenant_of_customer(store, customer_id).await?;
    set_plan(store, tenant_id, FREE_PLAN).await
}

async fn set_plan(store: &Store, tenant_id: Uuid, to: &str) -> Result<(), BillingError> {
    retrying(|| async {
        let mut tx = store.tx(tenant_id);
        let current = tenant(&mut tx).await?;
        if current.plan_id == to {
            return Ok(());
        }
        tx.update(
            &keys::tenant_key(tenant_id),
            Update::new()
                .require("plan_id", s(&current.plan_id))
                .set_s("plan_id", to)
                .set_s("updated_at", crate::item::format_time(Utc::now())),
        )?;
        audit(
            &mut tx,
            "tenant.plan_change",
            &format!("Plan changed from {} to {to}", current.plan_id),
            json!({ "from": current.plan_id, "to": to }),
        )
        .await?;
        tx.emit(Job::Dispatch { tenant_id });
        tx.commit().await?;
        Ok(())
    })
    .await
}

pub async fn start_period(
    store: &Store,
    customer_id: &str,
    period_start: DateTime<Utc>,
) -> Result<(), BillingError> {
    let tenant_id = tenant_of_customer(store, customer_id).await?;
    retrying(|| async {
        let mut tx = store.tx(tenant_id);
        let current = tenant(&mut tx).await?;
        let resumes = current.status == "suspended"
            && current.suspended_reason.as_deref() == Some(PAYMENT_FAILED);
        let mut update = guard_status(&current)
            .set_s("period_start", crate::item::format_time(period_start))
            .set_n("tasks_this_period", 0)
            .set_n("workspace_seconds_this_period", 0)
            .set_s("updated_at", crate::item::format_time(Utc::now()));
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
        tx.commit().await?;
        Ok(())
    })
    .await
}

pub async fn fail_payment(store: &Store, customer_id: &str) -> Result<(), BillingError> {
    let tenant_id = tenant_of_customer(store, customer_id).await?;
    retrying(|| async {
        let mut tx = store.tx(tenant_id);
        let current = tenant(&mut tx).await?;
        if current.status != "active" {
            return Ok(());
        }
        tx.update(
            &keys::tenant_key(tenant_id),
            guard_status(&current)
                .set_s("status", "suspended")
                .set_s("suspended_reason", PAYMENT_FAILED)
                .set_s("updated_at", crate::item::format_time(Utc::now())),
        )?;
        audit(
            &mut tx,
            "tenant.suspend",
            "The invoice payment failed; new tasks wait in PENDING",
            json!({ "reason": PAYMENT_FAILED }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    })
    .await
}
