use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::{PgConnection, Row};
use thiserror::Error;
use uuid::Uuid;

use crate::db::DbError;

pub const PAYMENT_PENDING: &str = "payment_pending";
pub const PAYMENT_FAILED: &str = "payment_failed";
pub const FREE_PLAN: &str = "free";

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
}

impl From<sqlx::Error> for BillingError {
    fn from(error: sqlx::Error) -> Self {
        Self::Db(DbError::Sqlx(error))
    }
}

async fn audit(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    action: &str,
    summary: &str,
    metadata: serde_json::Value,
) -> Result<(), BillingError> {
    sqlx::query(
        "INSERT INTO sanchoris.audit_events (tenant_id, actor_kind, actor_ref, action, target_kind, target_ref, summary, metadata) \
         VALUES ($1, 'system', 'stripe', $2, 'tenant', $3, $4, $5)",
    )
    .bind(tenant_id)
    .bind(action)
    .bind(tenant_id.to_string())
    .bind(summary)
    .bind(metadata)
    .execute(conn)
    .await?;
    Ok(())
}

async fn tenant_of_customer(
    conn: &mut PgConnection,
    customer_id: &str,
) -> Result<(Uuid, String, String, Option<String>), BillingError> {
    let row = sqlx::query(
        "SELECT id, plan_id, status, suspended_reason FROM sanchoris.tenants \
         WHERE stripe_customer_id = $1 FOR UPDATE",
    )
    .bind(customer_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| BillingError::UnknownCustomer(customer_id.to_string()))?;
    Ok((
        row.try_get("id")?,
        row.try_get("plan_id")?,
        row.try_get("status")?,
        row.try_get("suspended_reason")?,
    ))
}

pub async fn complete_checkout(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    customer_id: &str,
) -> Result<(), BillingError> {
    let row = sqlx::query(
        "SELECT status, suspended_reason, stripe_customer_id FROM sanchoris.tenants WHERE id = $1 FOR UPDATE",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(BillingError::TenantNotPayable(tenant_id))?;
    let status: String = row.try_get("status")?;
    let reason: Option<String> = row.try_get("suspended_reason")?;
    let customer: Option<String> = row.try_get("stripe_customer_id")?;
    if customer.as_deref().is_some_and(|c| c != customer_id) {
        return Err(BillingError::TenantNotPayable(tenant_id));
    }
    if status == "active" && customer.is_some() {
        return Ok(());
    }
    if status != "active" && reason.as_deref() != Some(PAYMENT_PENDING) {
        return Err(BillingError::TenantNotPayable(tenant_id));
    }
    sqlx::query(
        "UPDATE sanchoris.tenants SET stripe_customer_id = $2, status = 'active', suspended_reason = NULL, \
         updated_at = now() WHERE id = $1",
    )
    .bind(tenant_id)
    .bind(customer_id)
    .execute(&mut *conn)
    .await?;
    audit(
        conn,
        tenant_id,
        "tenant.activate",
        "Checkout completed; the tenant is active",
        json!({ "stripeCustomerId": customer_id }),
    )
    .await
}

pub async fn change_plan(
    conn: &mut PgConnection,
    customer_id: &str,
    price_id: &str,
) -> Result<(), BillingError> {
    let (tenant_id, current, _, _) = tenant_of_customer(conn, customer_id).await?;
    let plan_id = sqlx::query_scalar::<_, String>(
        "SELECT id FROM sanchoris.plans WHERE stripe_price_id = $1",
    )
    .bind(price_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| BillingError::UnknownPrice(price_id.to_string()))?;
    if plan_id == current {
        return Ok(());
    }
    set_plan(conn, tenant_id, &current, &plan_id).await
}

pub async fn end_subscription(
    conn: &mut PgConnection,
    customer_id: &str,
) -> Result<(), BillingError> {
    let (tenant_id, current, _, _) = tenant_of_customer(conn, customer_id).await?;
    if current == FREE_PLAN {
        return Ok(());
    }
    set_plan(conn, tenant_id, &current, FREE_PLAN).await
}

async fn set_plan(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    from: &str,
    to: &str,
) -> Result<(), BillingError> {
    sqlx::query("UPDATE sanchoris.tenants SET plan_id = $2, updated_at = now() WHERE id = $1")
        .bind(tenant_id)
        .bind(to)
        .execute(&mut *conn)
        .await?;
    audit(
        conn,
        tenant_id,
        "tenant.plan_change",
        &format!("Plan changed from {from} to {to}"),
        json!({ "from": from, "to": to }),
    )
    .await
}

pub async fn start_period(
    conn: &mut PgConnection,
    customer_id: &str,
    period_start: DateTime<Utc>,
) -> Result<(), BillingError> {
    let (tenant_id, _, status, reason) = tenant_of_customer(conn, customer_id).await?;
    let resumes = status == "suspended" && reason.as_deref() == Some(PAYMENT_FAILED);
    sqlx::query(
        "UPDATE sanchoris.tenants SET period_start = $2, tasks_this_period = 0, workspace_seconds_this_period = 0, \
         status = CASE WHEN $3 THEN 'active' ELSE status END, \
         suspended_reason = CASE WHEN $3 THEN NULL ELSE suspended_reason END, \
         updated_at = now() WHERE id = $1",
    )
    .bind(tenant_id)
    .bind(period_start)
    .bind(resumes)
    .execute(&mut *conn)
    .await?;
    if resumes {
        audit(
            conn,
            tenant_id,
            "tenant.resume",
            "The invoice was paid; the tenant is active",
            json!({}),
        )
        .await?;
    }
    Ok(())
}

pub async fn fail_payment(conn: &mut PgConnection, customer_id: &str) -> Result<(), BillingError> {
    let (tenant_id, _, status, _) = tenant_of_customer(conn, customer_id).await?;
    if status != "active" {
        return Ok(());
    }
    sqlx::query(
        "UPDATE sanchoris.tenants SET status = 'suspended', suspended_reason = $2, updated_at = now() WHERE id = $1",
    )
    .bind(tenant_id)
    .bind(PAYMENT_FAILED)
    .execute(&mut *conn)
    .await?;
    audit(
        conn,
        tenant_id,
        "tenant.suspend",
        "The invoice payment failed; new tasks wait in PENDING",
        json!({ "reason": PAYMENT_FAILED }),
    )
    .await
}
