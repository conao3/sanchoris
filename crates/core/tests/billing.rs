use chrono::{DateTime, Duration, TimeZone, Utc};
use sanchoris_core::billing::{
    self, ADMIN_PLAN, BillingError, Checkout, FREE_PLAN, Invoice, Outcome, PAYMENT_FAILED,
    PAYMENT_PENDING, Payer, StripeEvent, complete_checkout, end_checkout, end_subscription,
    fail_payment, start_period,
};
use sanchoris_core::items::{AuditEvent, Guard, Member, Membership, Tenant, uniq};
use sanchoris_core::keys;
use sanchoris_core::model::AuditOrigin;
use sanchoris_core::store::{Relay, Store};
use sanchoris_core::tenants::{
    NewTenant, PendingSession, TenantError, create_tenant, free_tenant_key, pending_checkout,
};
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
    events.sort_by_key(|a| (a.at, a.id));
    events.into_iter().map(|e| e.action).collect()
}

fn at(day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, day, 0, 0, 0).unwrap()
}

fn event<'a>(id: &'a str, kind: &'a str, created: DateTime<Utc>) -> StripeEvent<'a> {
    StripeEvent { id, kind, created }
}

fn customer(customer_id: &str) -> Payer<'_> {
    Payer {
        customer_id,
        tenant_id: None,
    }
}

fn paid<'a>(tenant_id: Uuid, customer_id: &'a str, plan_id: &'a str) -> Checkout<'a> {
    Checkout {
        tenant_id,
        customer_id,
        plan_id,
        paid: true,
    }
}

fn cycle(start: DateTime<Utc>) -> Invoice<'static> {
    Invoice {
        billing_reason: "subscription_cycle",
        period_start: Some(start),
    }
}

async fn pending_tenant(dynamo: &TestDynamo, plan_id: &str) -> Uuid {
    let seeded = dynamo.seed_tenant(plan_id).await;
    dynamo
        .modify::<Tenant>(&keys::tenant_key(seeded.tenant_id), |t| {
            t.status = "suspended".into();
            t.suspended_reason = Some(PAYMENT_PENDING.into());
            t.period_start = at(1) - Duration::days(30);
        })
        .await;
    seeded.tenant_id
}

async fn used(dynamo: &TestDynamo, tenant_id: Uuid, count: i64) {
    dynamo
        .modify::<Tenant>(&keys::tenant_key(tenant_id), |t| {
            t.tasks_this_period = count
        })
        .await;
}

async fn prices(dynamo: &TestDynamo) {
    dynamo.set_stripe_price("starter", "price_starter").await;
    dynamo.set_stripe_price("pro", "price_pro").await;
}

#[tokio::test]
async fn stripe_events_drive_the_tenant_status_plan_and_period() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    prices(&dynamo).await;
    let tenant_id = pending_tenant(&dynamo, "free").await;
    dynamo
        .modify::<Tenant>(&keys::tenant_key(tenant_id), |t| {
            t.tasks_this_period = 4;
            t.workspace_seconds_this_period = 600;
        })
        .await;

    let activated = at(1);
    complete_checkout(
        &store,
        event("evt_1", "checkout.session.completed", activated),
        paid(tenant_id, "cus_1", "starter"),
    )
    .await
    .unwrap();
    let t = tenant(&store, tenant_id).await;
    assert_eq!(
        (
            t.status.as_str(),
            t.stripe_customer_id.as_deref(),
            t.plan_id.as_str(),
            t.tasks_this_period,
            t.workspace_seconds_this_period,
            t.period_start
        ),
        ("active", Some("cus_1"), "starter", 0, 0, activated)
    );
    assert!(matches!(
        complete_checkout(
            &store,
            event("evt_2", "checkout.session.completed", at(1)),
            paid(tenant_id, "cus_1", "free")
        )
        .await,
        Err(BillingError::UnknownPlan(_))
    ));
    assert_eq!(
        complete_checkout(
            &store,
            event("evt_1", "checkout.session.completed", activated),
            paid(tenant_id, "cus_1", "starter"),
        )
        .await
        .unwrap(),
        Outcome::Duplicate
    );

    let updated = event("evt_3", "customer.subscription.updated", at(2));
    billing::change_plan(&store, updated, customer("cus_1"), "active", "price_pro")
        .await
        .unwrap();
    assert_eq!(tenant(&store, tenant_id).await.plan_id, "pro");
    assert!(matches!(
        billing::change_plan(
            &store,
            event("evt_4", "customer.subscription.updated", at(2)),
            customer("cus_1"),
            "active",
            "price_unknown"
        )
        .await,
        Err(BillingError::UnknownPrice(_))
    ));
    assert!(matches!(
        billing::change_plan(
            &store,
            event("evt_5", "customer.subscription.updated", at(2)),
            customer("cus_other"),
            "active",
            "price_pro"
        )
        .await,
        Err(BillingError::UnknownCustomer(_))
    ));

    fail_payment(
        &store,
        event("evt_6", "invoice.payment_failed", at(3)),
        customer("cus_1"),
    )
    .await
    .unwrap();
    let t = tenant(&store, tenant_id).await;
    assert_eq!(
        (t.status.as_str(), t.suspended_reason.as_deref()),
        ("suspended", Some(PAYMENT_FAILED))
    );

    let period = at(5);
    start_period(
        &store,
        event("evt_7", "invoice.paid", at(5)),
        customer("cus_1"),
        cycle(period),
    )
    .await
    .unwrap();
    let t = tenant(&store, tenant_id).await;
    assert_eq!(
        (
            t.status.as_str(),
            t.tasks_this_period,
            t.workspace_seconds_this_period,
            t.period_start
        ),
        ("active", 0, 0, period)
    );

    dynamo
        .modify::<Tenant>(&keys::tenant_key(tenant_id), |t| t.tasks_this_period = 7)
        .await;
    end_subscription(
        &store,
        event("evt_8", "customer.subscription.deleted", at(9)),
        customer("cus_1"),
    )
    .await
    .unwrap();
    let t = tenant(&store, tenant_id).await;
    assert_eq!(
        (t.plan_id.as_str(), t.tasks_this_period, t.period_start),
        (FREE_PLAN, 0, at(9))
    );

    assert_eq!(
        audit_actions(&dynamo, tenant_id).await,
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
async fn an_unpaid_checkout_keeps_the_tenant_pending_until_the_payment_succeeds() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    prices(&dynamo).await;
    let tenant_id = pending_tenant(&dynamo, "pro").await;
    complete_checkout(
        &store,
        event("evt_1", "checkout.session.completed", at(1)),
        Checkout {
            paid: false,
            ..paid(tenant_id, "cus_1", "pro")
        },
    )
    .await
    .unwrap();
    let t = tenant(&store, tenant_id).await;
    assert_eq!(
        (
            t.status.as_str(),
            t.suspended_reason.as_deref(),
            t.stripe_customer_id.as_deref()
        ),
        ("suspended", Some(PAYMENT_PENDING), Some("cus_1"))
    );
    complete_checkout(
        &store,
        event("evt_2", "checkout.session.async_payment_succeeded", at(2)),
        paid(tenant_id, "cus_1", "pro"),
    )
    .await
    .unwrap();
    assert_eq!(tenant(&store, tenant_id).await.status, "active");
}

#[tokio::test]
async fn invoice_paid_resets_the_period_once_and_only_for_a_new_period() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    prices(&dynamo).await;
    let tenant_id = pending_tenant(&dynamo, "pro").await;
    complete_checkout(
        &store,
        event("evt_c", "checkout.session.completed", at(1)),
        paid(tenant_id, "cus_1", "pro"),
    )
    .await
    .unwrap();
    used(&dynamo, tenant_id, 3).await;
    let renewal = event("evt_renew", "invoice.paid", at(28));
    assert_eq!(
        start_period(&store, renewal, customer("cus_1"), cycle(at(28)))
            .await
            .unwrap(),
        Outcome::Processed
    );
    assert_eq!(tenant(&store, tenant_id).await.tasks_this_period, 0);

    used(&dynamo, tenant_id, 2).await;
    assert_eq!(
        start_period(&store, renewal, customer("cus_1"), cycle(at(28)))
            .await
            .unwrap(),
        Outcome::Duplicate
    );
    assert_eq!(tenant(&store, tenant_id).await.tasks_this_period, 2);

    start_period(
        &store,
        event("evt_proration", "invoice.paid", at(29)),
        customer("cus_1"),
        Invoice {
            billing_reason: "subscription_update",
            period_start: Some(at(29)),
        },
    )
    .await
    .unwrap();
    start_period(
        &store,
        event("evt_late", "invoice.paid", at(29)),
        customer("cus_1"),
        cycle(at(1)),
    )
    .await
    .unwrap();
    let t = tenant(&store, tenant_id).await;
    assert_eq!((t.tasks_this_period, t.period_start), (2, at(28)));
}

#[tokio::test]
async fn subscription_events_before_the_checkout_find_the_tenant_by_metadata() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    prices(&dynamo).await;
    let tenant_id = pending_tenant(&dynamo, "pro").await;
    let hinted = Payer {
        customer_id: "cus_early",
        tenant_id: Some(tenant_id),
    };
    assert_eq!(
        billing::change_plan(
            &store,
            event("evt_1", "customer.subscription.updated", at(1)),
            hinted,
            "active",
            "price_pro"
        )
        .await
        .unwrap(),
        Outcome::Processed
    );
    start_period(
        &store,
        event("evt_2", "invoice.paid", at(1)),
        hinted,
        Invoice {
            billing_reason: "subscription_create",
            period_start: Some(at(1)),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        billing::change_plan(
            &store,
            event("evt_3", "customer.subscription.updated", at(1)),
            customer("cus_early"),
            "active",
            "price_pro"
        )
        .await,
        Err(BillingError::UnknownCustomer(_))
    ));
    assert_eq!(
        end_subscription(
            &store,
            event("evt_4", "customer.subscription.deleted", at(1)),
            hinted
        )
        .await
        .unwrap(),
        Outcome::Ignored("the tenant is waiting for its checkout".into())
    );
    complete_checkout(
        &store,
        event("evt_5", "checkout.session.completed", at(1)),
        paid(tenant_id, "cus_early", "pro"),
    )
    .await
    .unwrap();
    let t = tenant(&store, tenant_id).await;
    assert_eq!(
        (t.status.as_str(), t.plan_id.as_str(), t.period_start),
        ("active", "pro", at(1))
    );
    assert!(matches!(
        billing::change_plan(
            &store,
            event("evt_6", "customer.subscription.updated", at(2)),
            Payer {
                customer_id: "cus_early",
                tenant_id: Some(Uuid::new_v4()),
            },
            "active",
            "price_starter"
        )
        .await,
        Ok(Outcome::Processed)
    ));
    assert_eq!(tenant(&store, tenant_id).await.plan_id, "starter");
    assert!(matches!(
        billing::change_plan(
            &store,
            event("evt_7", "customer.subscription.updated", at(2)),
            Payer {
                customer_id: "cus_gone",
                tenant_id: Some(Uuid::new_v4()),
            },
            "active",
            "price_pro"
        )
        .await,
        Err(BillingError::TenantGone(_))
    ));
}

#[tokio::test]
async fn subscription_updates_outside_active_trialing_and_past_due_are_ignored() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    prices(&dynamo).await;
    let tenant_id = pending_tenant(&dynamo, "starter").await;
    complete_checkout(
        &store,
        event("evt_1", "checkout.session.completed", at(1)),
        paid(tenant_id, "cus_1", "starter"),
    )
    .await
    .unwrap();
    for status in [
        "incomplete",
        "incomplete_expired",
        "unpaid",
        "canceled",
        "paused",
    ] {
        assert_eq!(
            billing::change_plan(
                &store,
                event(status, "customer.subscription.updated", at(2)),
                customer("cus_1"),
                status,
                "price_pro"
            )
            .await
            .unwrap(),
            Outcome::Ignored(format!("the subscription is {status}"))
        );
    }
    assert_eq!(tenant(&store, tenant_id).await.plan_id, "starter");
    billing::change_plan(
        &store,
        event("evt_past_due", "customer.subscription.updated", at(2)),
        customer("cus_1"),
        "past_due",
        "price_pro",
    )
    .await
    .unwrap();
    assert_eq!(tenant(&store, tenant_id).await.plan_id, "pro");
}

#[tokio::test]
async fn a_free_tenant_upgraded_by_checkout_starts_a_fresh_period() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    prices(&dynamo).await;
    let seeded = dynamo.seed_tenant("free").await;
    dynamo
        .modify::<Tenant>(&keys::tenant_key(seeded.tenant_id), |t| {
            t.tasks_this_period = 5;
            t.workspace_seconds_this_period = 3_600;
        })
        .await;
    complete_checkout(
        &store,
        event("evt_1", "checkout.session.completed", at(3)),
        paid(seeded.tenant_id, "cus_1", "pro"),
    )
    .await
    .unwrap();
    let t = tenant(&store, seeded.tenant_id).await;
    assert_eq!(
        (
            t.plan_id.as_str(),
            t.tasks_this_period,
            t.workspace_seconds_this_period,
            t.period_start
        ),
        ("pro", 0, 0, at(3))
    );
}

#[tokio::test]
async fn checkout_does_not_reactivate_a_tenant_suspended_for_another_reason() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    prices(&dynamo).await;
    let seeded = dynamo.seed_tenant("free").await;
    dynamo
        .modify::<Tenant>(&keys::tenant_key(seeded.tenant_id), |t| {
            t.status = "suspended".into();
            t.suspended_reason = Some("mining detected".into());
        })
        .await;
    assert!(matches!(
        complete_checkout(
            &store,
            event("evt_1", "checkout.session.completed", at(1)),
            paid(seeded.tenant_id, "cus_1", "starter")
        )
        .await,
        Err(BillingError::TenantNotPayable(_))
    ));
    assert!(matches!(
        complete_checkout(
            &store,
            event("evt_2", "checkout.session.completed", at(1)),
            paid(Uuid::new_v4(), "cus_1", "starter")
        )
        .await,
        Err(BillingError::TenantNotPayable(_))
    ));
    assert_eq!(tenant(&store, seeded.tenant_id).await.status, "suspended");
}

fn new_tenant<'a>(
    user_id: Uuid,
    name: &'a str,
    plan_id: &'a str,
    checkout: Option<PendingSession<'a>>,
) -> NewTenant<'a> {
    NewTenant {
        id: Uuid::new_v4(),
        name,
        plan_id,
        user_id,
        checkout,
        origin: AuditOrigin::Web,
        source_ip: None,
        now: at(1),
    }
}

#[tokio::test]
async fn a_user_holds_one_free_tenant_and_one_pending_checkout() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    prices(&dynamo).await;
    let user = Uuid::new_v4();
    let free = create_tenant(&store, &new_tenant(user, "Settings", FREE_PLAN, None))
        .await
        .unwrap();
    assert_eq!(free.slug, "settings-2");
    assert!(matches!(
        create_tenant(&store, &new_tenant(user, "Second", FREE_PLAN, None)).await,
        Err(TenantError::FreeTenantTaken)
    ));
    let guard = store
        .get::<Guard>(&free_tenant_key(user))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(guard.target, free.tenant_id);

    let session = PendingSession {
        session_id: "cs_1",
        expires_at: at(2),
    };
    let pending = create_tenant(&store, &new_tenant(user, "Paid", "pro", Some(session)))
        .await
        .unwrap();
    assert_eq!(
        pending_checkout(&store, user)
            .await
            .unwrap()
            .unwrap()
            .tenant_id,
        pending.tenant_id
    );
    assert!(matches!(
        create_tenant(
            &store,
            &new_tenant(
                user,
                "Paid again",
                "pro",
                Some(PendingSession {
                    session_id: "cs_2",
                    ..session
                })
            )
        )
        .await,
        Err(TenantError::CheckoutPending(id)) if id == pending.tenant_id
    ));

    complete_checkout(
        &store,
        event("evt_1", "checkout.session.completed", at(1)),
        paid(pending.tenant_id, "cus_1", "pro"),
    )
    .await
    .unwrap();
    assert!(pending_checkout(&store, user).await.unwrap().is_none());

    complete_checkout(
        &store,
        event("evt_2", "checkout.session.completed", at(1)),
        paid(free.tenant_id, "cus_2", "starter"),
    )
    .await
    .unwrap();
    assert!(
        store
            .get::<Guard>(&free_tenant_key(user))
            .await
            .unwrap()
            .is_none()
    );
    create_tenant(&store, &new_tenant(user, "Another free", FREE_PLAN, None))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_expired_or_failed_checkout_discards_the_pending_tenant() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    prices(&dynamo).await;
    let user = Uuid::new_v4();
    let session = PendingSession {
        session_id: "cs_1",
        expires_at: at(2),
    };
    let pending = create_tenant(&store, &new_tenant(user, "Abandoned", "pro", Some(session)))
        .await
        .unwrap();
    let expired = event("evt_1", "checkout.session.expired", at(2));
    assert_eq!(
        end_checkout(&store, expired, pending.tenant_id)
            .await
            .unwrap(),
        Outcome::Processed
    );
    assert!(
        store
            .get::<Tenant>(&keys::tenant_key(pending.tenant_id))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .get::<Guard>(&keys::uniq_key(uniq::SLUG, &pending.slug))
            .await
            .unwrap()
            .is_none()
    );
    assert!(dynamo.all::<Member>().await.is_empty());
    assert!(
        dynamo
            .all::<Membership>()
            .await
            .iter()
            .all(|m| m.tenant_id != pending.tenant_id)
    );
    assert!(pending_checkout(&store, user).await.unwrap().is_none());
    let mut actions = audit_actions(&dynamo, pending.tenant_id).await;
    actions.sort();
    assert_eq!(actions, ["tenant.create", "tenant.discard"]);

    let active = dynamo.seed_tenant("pro").await;
    assert!(matches!(
        end_checkout(
            &store,
            event("evt_2", "checkout.session.async_payment_failed", at(2)),
            active.tenant_id
        )
        .await
        .unwrap(),
        Outcome::Ignored(_)
    ));
    assert_eq!(tenant(&store, active.tenant_id).await.status, "active");
}

#[tokio::test]
async fn stripe_events_never_change_an_admin_tenant() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    prices(&dynamo).await;
    let tenant_id = dynamo.seed_tenant(ADMIN_PLAN).await.tenant_id;
    used(&dynamo, tenant_id, 7).await;
    let before = tenant(&store, tenant_id).await;
    let by_metadata = Payer {
        customer_id: "cus_admin",
        tenant_id: Some(tenant_id),
    };
    let ignored = Outcome::Ignored(format!("tenant {tenant_id} is on the admin plan"));

    assert_eq!(
        complete_checkout(
            &store,
            event("evt_a1", "checkout.session.completed", at(1)),
            paid(tenant_id, "cus_admin", "starter"),
        )
        .await
        .unwrap(),
        ignored
    );
    assert_eq!(
        billing::change_plan(
            &store,
            event("evt_a2", "customer.subscription.updated", at(2)),
            by_metadata,
            "active",
            "price_pro",
        )
        .await
        .unwrap(),
        ignored
    );
    assert_eq!(
        end_subscription(
            &store,
            event("evt_a3", "customer.subscription.deleted", at(3)),
            by_metadata,
        )
        .await
        .unwrap(),
        ignored
    );
    assert_eq!(
        fail_payment(
            &store,
            event("evt_a4", "invoice.payment_failed", at(4)),
            by_metadata,
        )
        .await
        .unwrap(),
        ignored
    );
    assert_eq!(
        start_period(
            &store,
            event("evt_a5", "invoice.paid", at(5)),
            by_metadata,
            cycle(at(5)),
        )
        .await
        .unwrap(),
        ignored
    );

    assert_eq!(tenant(&store, tenant_id).await, before);
    assert_eq!(
        audit_actions(&dynamo, tenant_id).await,
        Vec::<String>::new()
    );
    let recorded = billing::recorded(&store, "evt_a2").await.unwrap().unwrap();
    assert_eq!(
        (recorded.outcome.as_str(), recorded.tenant_id),
        ("ignored", Some(tenant_id))
    );
}

#[test]
fn privileged_emails_end_with_the_sancode_domain() {
    assert!(billing::is_privileged_email("agent001@sancode.dev"));
    assert!(billing::is_privileged_email("Agent001@SanCode.Dev"));
    assert!(!billing::is_privileged_email("conao3@gmail.com"));
    assert!(!billing::is_privileged_email("someone@notsancode.dev"));
    assert!(!billing::is_privileged_email(
        "someone@sancode.dev.example.com"
    ));
    assert!(!billing::is_privileged_email("@sancode.dev"));
    assert!(!billing::is_privileged_email("sancode.dev"));
}
