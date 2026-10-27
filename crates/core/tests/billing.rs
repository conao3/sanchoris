use chrono::{TimeZone, Utc};
use sanchoris_core::billing::{
    self, BillingError, PAYMENT_FAILED, PAYMENT_PENDING, complete_checkout, end_subscription,
    fail_payment, start_period,
};
use sanchoris_core::items::{AuditEvent, Tenant};
use sanchoris_core::keys;
use sanchoris_core::store::{Relay, Store};
use sanchoris_core::testing::TestDynamo;
use uuid::Uuid;

async fn tenant(store: &Store, id: Uuid) -> Tenant {
    store
        .get::<Tenant>(&keys::tenant_key(id))
        .await
        .unwrap()
        .unwrap()
}

async fn audit_actions(dynamo: &TestDynamo, id: Uuid) -> Vec<String> {
    let mut events = dynamo
        .all::<AuditEvent>()
        .await
        .into_iter()
        .filter(|e| e.tenant_id == id)
        .collect::<Vec<_>>();
    events.sort_by(|a, b| (a.at, a.id).cmp(&(b.at, b.id)));
    events.into_iter().map(|e| e.action).collect()
}

#[tokio::test]
async fn stripe_events_drive_the_tenant_status_plan_and_period() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("free").await;
    dynamo.set_stripe_price("starter", "price_starter").await;
    dynamo.set_stripe_price("pro", "price_pro").await;
    dynamo
        .modify::<Tenant>(&keys::tenant_key(seeded.tenant_id), |t| {
            t.status = "suspended".into();
            t.suspended_reason = Some(PAYMENT_PENDING.into());
            t.tasks_this_period = 4;
            t.workspace_seconds_this_period = 600;
        })
        .await;

    complete_checkout(&store, seeded.tenant_id, "cus_1", "starter")
        .await
        .unwrap();
    let t = tenant(&store, seeded.tenant_id).await;
    assert_eq!(
        (
            t.status.as_str(),
            t.stripe_customer_id.as_deref(),
            t.plan_id.as_str()
        ),
        ("active", Some("cus_1"), "starter")
    );
    assert!(matches!(
        complete_checkout(&store, seeded.tenant_id, "cus_1", "free").await,
        Err(BillingError::UnknownPlan(_))
    ));
    complete_checkout(&store, seeded.tenant_id, "cus_1", "starter")
        .await
        .unwrap();

    billing::change_plan(&store, "cus_1", "price_pro")
        .await
        .unwrap();
    assert_eq!(tenant(&store, seeded.tenant_id).await.plan_id, "pro");
    assert!(matches!(
        billing::change_plan(&store, "cus_1", "price_unknown").await,
        Err(BillingError::UnknownPrice(_))
    ));
    assert!(matches!(
        billing::change_plan(&store, "cus_other", "price_pro").await,
        Err(BillingError::UnknownCustomer(_))
    ));

    fail_payment(&store, "cus_1").await.unwrap();
    let t = tenant(&store, seeded.tenant_id).await;
    assert_eq!(
        (t.status.as_str(), t.suspended_reason.as_deref()),
        ("suspended", Some(PAYMENT_FAILED))
    );

    let period = Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap();
    start_period(&store, "cus_1", period).await.unwrap();
    let t = tenant(&store, seeded.tenant_id).await;
    assert_eq!(
        (
            t.status.as_str(),
            t.tasks_this_period,
            t.workspace_seconds_this_period,
            t.period_start
        ),
        ("active", 0, 0, period)
    );

    end_subscription(&store, "cus_1").await.unwrap();
    assert_eq!(tenant(&store, seeded.tenant_id).await.plan_id, "free");

    assert_eq!(
        audit_actions(&dynamo, seeded.tenant_id).await,
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
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("free").await;
    dynamo
        .modify::<Tenant>(&keys::tenant_key(seeded.tenant_id), |t| {
            t.status = "suspended".into();
            t.suspended_reason = Some("mining detected".into());
        })
        .await;
    assert!(matches!(
        complete_checkout(&store, seeded.tenant_id, "cus_1", "starter").await,
        Err(BillingError::TenantNotPayable(_))
    ));
    assert_eq!(tenant(&store, seeded.tenant_id).await.status, "suspended");
}
