use chrono::{DateTime, Duration, TimeZone, Utc};
use sanchoris_core::audit::audit_events;
use sanchoris_core::db::{self, Store};
use sanchoris_core::items::ReceiveRate;
use sanchoris_core::keys;
use sanchoris_core::receive::{
    Admission, ReceiveLimited, SOURCE_CHAT, SOURCE_GITHUB, admit_inbound, window_start,
};
use sanchoris_core::store::Relay;
use sanchoris_core::testing::TestDynamo;
use uuid::Uuid;

fn at(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 27, 9, 30, second).unwrap()
}

async fn fill(dynamo: &TestDynamo, tenant_id: Uuid, now: DateTime<Utc>, received: i64) {
    let start = window_start(now);
    dynamo
        .put(&ReceiveRate {
            tenant_id,
            window_start: start,
            received,
            dropped: 0,
            expires_at: start + Duration::days(1),
        })
        .await;
}

async fn rate(store: &Store, tenant_id: Uuid, now: DateTime<Utc>) -> (i64, i64) {
    let rate = store
        .get::<ReceiveRate>(&keys::receive_rate_key(tenant_id, window_start(now)))
        .await
        .unwrap()
        .unwrap();
    (rate.received, rate.dropped)
}

async fn limit_audits(store: &Store, tenant_id: Uuid) -> Vec<String> {
    audit_events(&mut store.tx(tenant_id))
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.action == "tenant.receive_limit")
        .map(|event| event.summary)
        .collect()
}

async fn admit(store: &Store, tenant_id: Uuid, key: &str, now: DateTime<Utc>) -> Admission {
    admit_inbound(store, tenant_id, SOURCE_GITHUB, key, now)
        .await
        .unwrap()
}

#[tokio::test]
async fn events_under_the_limit_pass_and_later_ones_are_dropped_with_one_audit() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let tenant = dynamo.seed_tenant("free").await.tenant_id;
    let limit = db::plan(&store, "free")
        .await
        .unwrap()
        .inbound_events_per_minute;
    assert_eq!(limit, 20);

    for n in 0..limit {
        assert_eq!(
            admit(&store, tenant, &format!("d-{n}"), at(5)).await,
            Admission::Admitted
        );
    }
    let dropped = ReceiveLimited {
        limit,
        window_start: at(0),
        retry_after_seconds: 50,
    };
    assert_eq!(
        admit(&store, tenant, "d-over-1", at(10)).await,
        Admission::Dropped(dropped.clone())
    );
    assert_eq!(
        admit(&store, tenant, "d-over-2", at(10)).await,
        Admission::Dropped(dropped.clone())
    );
    assert_eq!(rate(&store, tenant, at(10)).await, (limit, 2));
    assert_eq!(
        limit_audits(&store, tenant).await,
        [format!(
            "Dropped inbound events over the receive limit: {}",
            dropped.reason()
        )]
    );
}

#[tokio::test]
async fn the_window_resets_on_the_next_minute() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let tenant = dynamo.seed_tenant("pro").await.tenant_id;
    fill(&dynamo, tenant, at(0), 120).await;
    assert!(matches!(
        admit(&store, tenant, "late", at(59)).await,
        Admission::Dropped(ReceiveLimited {
            limit: 120,
            retry_after_seconds: 1,
            ..
        })
    ));

    let next = at(0) + Duration::seconds(60);
    assert_eq!(
        admit(&store, tenant, "next", next).await,
        Admission::Admitted
    );
    assert_eq!(rate(&store, tenant, next).await, (1, 0));
    fill(&dynamo, tenant, next, 120).await;
    assert!(matches!(
        admit(&store, tenant, "next-late", next).await,
        Admission::Dropped(_)
    ));
    assert_eq!(limit_audits(&store, tenant).await.len(), 2);
}

#[tokio::test]
async fn a_tenant_over_its_limit_leaves_other_tenants_alone() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let flooding = dynamo.seed_tenant("starter").await.tenant_id;
    let quiet = dynamo.seed_tenant("starter").await.tenant_id;
    fill(&dynamo, flooding, at(0), 60).await;

    assert!(matches!(
        admit(&store, flooding, "flood", at(1)).await,
        Admission::Dropped(ReceiveLimited { limit: 60, .. })
    ));
    assert_eq!(
        admit_inbound(&store, quiet, SOURCE_CHAT, "chat", at(1))
            .await
            .unwrap(),
        Admission::Admitted
    );
    assert_eq!(rate(&store, quiet, at(1)).await, (1, 0));
    assert!(limit_audits(&store, quiet).await.is_empty());
}

#[tokio::test]
async fn the_admin_plan_has_a_finite_limit() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let tenant = dynamo.seed_tenant("admin").await.tenant_id;
    fill(&dynamo, tenant, at(0), 299).await;

    assert_eq!(
        admit(&store, tenant, "last", at(2)).await,
        Admission::Admitted
    );
    assert!(matches!(
        admit(&store, tenant, "over", at(2)).await,
        Admission::Dropped(ReceiveLimited { limit: 300, .. })
    ));
    assert_eq!(rate(&store, tenant, at(2)).await, (300, 1));
}

#[tokio::test]
async fn concurrent_events_never_pass_the_limit() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let tenant = dynamo.seed_tenant("free").await.tenant_id;

    let mut burst = tokio::task::JoinSet::new();
    for n in 0..40 {
        let store = store.clone();
        burst.spawn(async move {
            admit_inbound(&store, tenant, SOURCE_GITHUB, "burst", at(3 + n % 2))
                .await
                .unwrap()
        });
    }
    let admitted = burst
        .join_all()
        .await
        .into_iter()
        .filter(|a| *a == Admission::Admitted)
        .count();
    assert_eq!(admitted, 20);
    assert_eq!(rate(&store, tenant, at(3)).await, (20, 20));
    assert_eq!(limit_audits(&store, tenant).await.len(), 1);
}
