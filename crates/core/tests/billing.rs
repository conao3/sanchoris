use chrono::{TimeZone, Utc};
use sanchoris_core::billing::{
    self, BillingError, PAYMENT_FAILED, PAYMENT_PENDING, complete_checkout, end_subscription,
    fail_payment, start_period,
};
use sanchoris_core::testing::TestPostgres;
use sqlx::{PgPool, Row};
use uuid::Uuid;

struct Tenant {
    plan_id: String,
    status: String,
    suspended_reason: Option<String>,
    tasks: i32,
    seconds: i64,
    customer: Option<String>,
}

async fn tenant(pool: &PgPool, id: Uuid) -> Tenant {
    let row = sqlx::query(
        "SELECT plan_id, status, suspended_reason, tasks_this_period, workspace_seconds_this_period, stripe_customer_id \
         FROM sanchoris.tenants WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap();
    Tenant {
        plan_id: row.get("plan_id"),
        status: row.get("status"),
        suspended_reason: row.get("suspended_reason"),
        tasks: row.get("tasks_this_period"),
        seconds: row.get("workspace_seconds_this_period"),
        customer: row.get("stripe_customer_id"),
    }
}

async fn audit_actions(pool: &PgPool, id: Uuid) -> Vec<String> {
    sqlx::query_scalar("SELECT action FROM sanchoris.audit_events WHERE tenant_id = $1 ORDER BY at, id")
        .bind(id)
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn stripe_events_drive_the_tenant_status_plan_and_period() {
    let pg = TestPostgres::start().await;
    let admin = pg.admin_pool().await;
    let pool = pg.worker_pool().await;
    let seeded = pg.seed_tenant(&admin, "free").await;
    sqlx::query("UPDATE sanchoris.plans SET stripe_price_id = 'price_pro' WHERE id = 'pro'")
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE sanchoris.tenants SET status = 'suspended', suspended_reason = $2, tasks_this_period = 4, \
         workspace_seconds_this_period = 600 WHERE id = $1",
    )
    .bind(seeded.tenant_id)
    .bind(PAYMENT_PENDING)
    .execute(&admin)
    .await
    .unwrap();
    let mut conn = pool.acquire().await.unwrap();

    complete_checkout(&mut conn, seeded.tenant_id, "cus_1").await.unwrap();
    let t = tenant(&pool, seeded.tenant_id).await;
    assert_eq!((t.status.as_str(), t.customer.as_deref()), ("active", Some("cus_1")));
    complete_checkout(&mut conn, seeded.tenant_id, "cus_1").await.unwrap();

    billing::change_plan(&mut conn, "cus_1", "price_pro").await.unwrap();
    assert_eq!(tenant(&pool, seeded.tenant_id).await.plan_id, "pro");
    assert!(matches!(
        billing::change_plan(&mut conn, "cus_1", "price_unknown").await,
        Err(BillingError::UnknownPrice(_))
    ));
    assert!(matches!(
        billing::change_plan(&mut conn, "cus_other", "price_pro").await,
        Err(BillingError::UnknownCustomer(_))
    ));

    fail_payment(&mut conn, "cus_1").await.unwrap();
    let t = tenant(&pool, seeded.tenant_id).await;
    assert_eq!(
        (t.status.as_str(), t.suspended_reason.as_deref()),
        ("suspended", Some(PAYMENT_FAILED))
    );

    let period = Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap();
    start_period(&mut conn, "cus_1", period).await.unwrap();
    let t = tenant(&pool, seeded.tenant_id).await;
    assert_eq!((t.status.as_str(), t.tasks, t.seconds), ("active", 0, 0));

    end_subscription(&mut conn, "cus_1").await.unwrap();
    assert_eq!(tenant(&pool, seeded.tenant_id).await.plan_id, "free");

    assert_eq!(
        audit_actions(&pool, seeded.tenant_id).await,
        [
            "tenant.activate",
            "tenant.plan_change",
            "tenant.suspend",
            "tenant.resume",
            "tenant.plan_change"
        ]
    );
}

#[tokio::test]
async fn checkout_does_not_reactivate_a_tenant_suspended_for_another_reason() {
    let pg = TestPostgres::start().await;
    let admin = pg.admin_pool().await;
    let pool = pg.worker_pool().await;
    let seeded = pg.seed_tenant(&admin, "free").await;
    sqlx::query(
        "UPDATE sanchoris.tenants SET status = 'suspended', suspended_reason = 'mining detected' WHERE id = $1",
    )
    .bind(seeded.tenant_id)
    .execute(&admin)
    .await
    .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    assert!(matches!(
        complete_checkout(&mut conn, seeded.tenant_id, "cus_1").await,
        Err(BillingError::TenantNotPayable(_))
    ));
    assert_eq!(tenant(&pool, seeded.tenant_id).await.status, "suspended");
}
