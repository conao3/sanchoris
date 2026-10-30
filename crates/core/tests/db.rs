use std::collections::HashSet;
use std::sync::Arc;

use aws_sdk_dynamodb::types::AttributeValue;
use chrono::{Duration, TimeZone, Utc};
use sanchoris_core::audit::{AuditEntry, audit_events, record_audit};
use sanchoris_core::clock::FakeClock;
use sanchoris_core::db::{
    self, DbError, NewTask, NewTriggerEvent, Recorded, Store, TriggerEventOutcome,
};
use sanchoris_core::item::{Entity, Key};
use sanchoris_core::items::{AuditEvent, PendingTask, Plan, Run, Task, Tenant, TriggerEvent};
use sanchoris_core::jobs::{Envelope, Job};
use sanchoris_core::keys;
use sanchoris_core::model::{ActorKind, AuditOrigin, NamedDefinition, TaskDefinition, TaskStatus};
use sanchoris_core::store::{Relay, Update, retry};
use sanchoris_core::testing::{SeededTenant, TestDynamo};
use serde_json::{Map, json};
use uuid::Uuid;

fn definition() -> TaskDefinition {
    TaskDefinition {
        trigger: NamedDefinition {
            name: "labeled-issue".into(),
            definition: json!({ "State": "ENABLED" }),
        },
        session: NamedDefinition {
            name: "shop".into(),
            definition: json!({ "Worker": { "Agent": "claude", "Model": "claude-sonnet-5-5", "Credential": "acme-claude-team" } }),
        },
        skills: vec![NamedDefinition {
            name: "open-pr-green-ci".into(),
            definition: json!({ "StartAt": "Init" }),
        }],
    }
}

async fn create_task(
    store: &Store,
    seeded: &SeededTenant,
    name: &str,
    blocked_by: &[Uuid],
) -> Recorded {
    let definition = definition();
    retry(8, || async {
        let mut tx = store.tx(seeded.tenant_id);
        let recorded = db::create_task(
            &mut tx,
            &NewTask {
                trigger_id: seeded.trigger_id,
                trigger_event_id: None,
                config_revision_id: seeded.config_revision_id,
                name,
                title: name,
                definition: &definition,
                parent_task_id: None,
                channel: None,
                requested_by_member_id: None,
                requested_by_github_user_id: None,
                requested_by_github_login: None,
                blocked_by,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(recorded)
    })
    .await
    .unwrap()
}

async fn dispatch(store: &Store, tenant_id: Uuid) -> Vec<db::DispatchedTask> {
    retry(50, || async {
        let mut tx = store.tx(tenant_id);
        let dispatched = db::dispatch_pending_tasks(&mut tx, Utc::now()).await?;
        tx.commit().await?;
        Ok(dispatched)
    })
    .await
    .unwrap()
}

async fn finish_task(dynamo: &TestDynamo, tenant_id: Uuid, task_id: Uuid) {
    let store = dynamo.store(Relay::Stream);
    let mut tx = store.tx(tenant_id);
    let mut task = tx
        .get::<Task>(&keys::task_key(tenant_id, task_id))
        .await
        .unwrap()
        .unwrap();
    task.status = TaskStatus::Succeeded;
    task.finished_at = Some(Utc::now());
    sanchoris_core::tasks::store_task(&mut tx, &task, Utc::now())
        .await
        .unwrap();
    for mut run in sanchoris_core::tasks::runs_of(&mut tx, task_id)
        .await
        .unwrap()
    {
        run.status = "completed".into();
        sanchoris_core::tasks::release_slot(&mut tx, &mut run).unwrap();
        tx.put(&run).unwrap();
    }
    tx.commit().await.unwrap();
}

async fn tenant(store: &Store, tenant_id: Uuid) -> Tenant {
    store
        .get::<Tenant>(&keys::tenant_key(tenant_id))
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn duplicate_trigger_events_are_recorded_once() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("pro").await;
    let event = json!({ "source": "github", "detail-type": "Issue Labeled", "detail": {} });
    let new_event = NewTriggerEvent {
        trigger_id: seeded.trigger_id,
        key: "delivery-1",
        event: &event,
    };

    let mut tx = store.tx(seeded.tenant_id);
    let first = db::record_trigger_event(&mut tx, &new_event).await.unwrap();
    tx.commit().await.unwrap();
    let mut tx = store.tx(seeded.tenant_id);
    let second = db::record_trigger_event(&mut tx, &new_event).await.unwrap();
    tx.commit().await.unwrap();

    assert!(matches!(first, Recorded::New(_)));
    assert_eq!(second, Recorded::Duplicate(first.id()));
    assert_eq!(dynamo.all::<TriggerEvent>().await.len(), 1);

    let mut tx = store.tx(seeded.tenant_id);
    db::set_trigger_event_outcome(
        &mut tx,
        first.id(),
        &TriggerEventOutcome::Skipped {
            reason: "children limit".into(),
        },
    )
    .await
    .unwrap();
    let again =
        db::set_trigger_event_outcome(&mut tx, first.id(), &TriggerEventOutcome::Started).await;
    assert!(matches!(again, Err(DbError::TriggerEventNotPending(id)) if id == first.id()));
    tx.commit().await.unwrap();

    let stored = dynamo.all::<TriggerEvent>().await.remove(0);
    assert_eq!(
        (stored.result.as_deref(), stored.reason.as_deref()),
        (Some("skipped"), Some("children limit"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_duplicate_deliveries_insert_one_event() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("pro").await;
    let event = json!({ "source": "github" });

    let attempts = (0..8).map(|_| {
        let store = store.clone();
        let event = event.clone();
        tokio::spawn(async move {
            retry(8, || async {
                let mut tx = store.tx(seeded.tenant_id);
                let recorded = db::record_trigger_event(
                    &mut tx,
                    &NewTriggerEvent {
                        trigger_id: seeded.trigger_id,
                        key: "delivery-race",
                        event: &event,
                    },
                )
                .await?;
                tx.commit().await?;
                Ok(recorded)
            })
            .await
            .unwrap()
        })
    });
    let mut results = Vec::new();
    for attempt in attempts.collect::<Vec<_>>() {
        results.push(attempt.await.unwrap());
    }
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Recorded::New(_)))
            .count(),
        1
    );
    let ids = results.iter().map(|r| r.id()).collect::<HashSet<_>>();
    assert_eq!(ids.len(), 1);
    assert_eq!(dynamo.all::<TriggerEvent>().await.len(), 1);
}

#[tokio::test]
async fn tasks_are_idempotent_by_trigger_and_name() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("pro").await;

    let first = create_task(&store, &seeded, "acme/shop-web#35", &[]).await;
    let second = create_task(&store, &seeded, "acme/shop-web#35", &[]).await;
    let other = create_task(&store, &seeded, "acme/shop-web#36", &[]).await;

    assert!(matches!(first, Recorded::New(_)));
    assert_eq!(second, Recorded::Duplicate(first.id()));
    assert!(matches!(other, Recorded::New(id) if id != first.id()));

    let task = store
        .get::<Task>(&keys::task_key(seeded.tenant_id, first.id()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.status, TaskStatus::Pending);
    assert_eq!(task.definition["Session"]["Name"], "shop");
    assert_eq!(dynamo.all::<Task>().await.len(), 2);
}

#[tokio::test]
async fn dispatch_respects_the_plan_concurrency() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("starter").await;
    let mut tasks = Vec::new();
    for n in 0..3 {
        tasks.push(
            create_task(&store, &seeded, &format!("task-{n}"), &[])
                .await
                .id(),
        );
    }

    let first = dispatch(&store, seeded.tenant_id).await;
    assert_eq!(
        first.iter().map(|d| d.task_id).collect::<Vec<_>>(),
        tasks[..2]
    );
    assert!(dispatch(&store, seeded.tenant_id).await.is_empty());

    let run = dynamo
        .all::<Run>()
        .await
        .into_iter()
        .find(|r| r.id == first[0].run_id)
        .unwrap();
    assert_eq!(
        (
            run.session_name.as_str(),
            run.worker.as_str(),
            run.model.as_str(),
            run.status.as_str()
        ),
        ("shop", "claude", "claude-sonnet-5-5", "creating")
    );

    finish_task(&dynamo, seeded.tenant_id, tasks[0]).await;
    let next = dispatch(&store, seeded.tenant_id).await;
    assert_eq!(
        next.iter().map(|d| d.task_id).collect::<Vec<_>>(),
        vec![tasks[2]]
    );
    let tenant = tenant(&store, seeded.tenant_id).await;
    assert_eq!((tenant.tasks_this_period, tenant.live_workspaces), (3, 2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn concurrent_dispatchers_never_exceed_the_cap() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("pro").await;
    for n in 0..12 {
        create_task(&store, &seeded, &format!("task-{n}"), &[]).await;
    }

    let handles = (0..6)
        .map(|_| {
            let store = store.clone();
            tokio::spawn(async move { dispatch(&store, seeded.tenant_id).await })
        })
        .collect::<Vec<_>>();
    let mut dispatched = Vec::new();
    for handle in handles {
        dispatched.extend(handle.await.unwrap());
    }

    assert_eq!(dispatched.len(), 5);
    let running = dynamo
        .all::<Task>()
        .await
        .into_iter()
        .filter(|t| t.status == TaskStatus::Running)
        .count();
    assert_eq!(running, 5);
    assert_eq!(dynamo.all::<Run>().await.len(), 5);
    assert_eq!(tenant(&store, seeded.tenant_id).await.live_workspaces, 5);
}

#[tokio::test]
async fn dispatch_waits_for_blocking_tasks() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("max").await;
    let blocker = create_task(&store, &seeded, "blocker", &[]).await.id();
    let blocked = create_task(&store, &seeded, "blocked", &[blocker])
        .await
        .id();

    let first = dispatch(&store, seeded.tenant_id).await;
    assert_eq!(
        first.iter().map(|d| d.task_id).collect::<Vec<_>>(),
        vec![blocker]
    );
    assert!(dispatch(&store, seeded.tenant_id).await.is_empty());

    finish_task(&dynamo, seeded.tenant_id, blocker).await;
    let next = dispatch(&store, seeded.tenant_id).await;
    assert_eq!(
        next.iter().map(|d| d.task_id).collect::<Vec<_>>(),
        vec![blocked]
    );
}

#[tokio::test]
async fn dispatch_stops_at_the_plan_task_limit_and_for_suspended_tenants() {
    let dynamo = TestDynamo::start().await;
    let clock = Arc::new(FakeClock::new(Utc::now()));
    let direct = dynamo.direct_store(clock);
    let store = direct.store.clone();
    let free = dynamo.seed_tenant("free").await;
    dynamo
        .modify::<Tenant>(&keys::tenant_key(free.tenant_id), |t| {
            t.tasks_this_period = 5
        })
        .await;
    create_task(&store, &free, "over-limit", &[]).await;
    assert!(dispatch(&store, free.tenant_id).await.is_empty());

    let suspended = dynamo.seed_tenant("pro").await;
    dynamo
        .modify::<Tenant>(&keys::tenant_key(suspended.tenant_id), |t| {
            t.status = "suspended".into();
            t.suspended_reason = Some("abuse".into());
        })
        .await;
    create_task(&store, &suspended, "suspended", &[]).await;
    assert!(dispatch(&store, suspended.tenant_id).await.is_empty());

    let mut doorbells = direct
        .queue
        .bodies()
        .iter()
        .map(|body| Envelope::parse(body).unwrap().job)
        .collect::<Vec<_>>();
    doorbells.sort_by_key(|job| job.group());
    let mut expected = vec![
        Job::Dispatch {
            tenant_id: free.tenant_id,
        },
        Job::Dispatch {
            tenant_id: suspended.tenant_id,
        },
    ];
    expected.sort_by_key(|job| job.group());
    assert_eq!(doorbells, expected);
}

#[tokio::test]
async fn timers_are_scheduled_after_commit_and_cancelled_by_name() {
    let dynamo = TestDynamo::start().await;
    let now = Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap();
    let clock = Arc::new(FakeClock::new(now));
    let direct = dynamo.direct_store(clock.clone());
    let seeded = dynamo.seed_tenant("pro").await;
    let job = |offset: i64| Job::TaskResume {
        tenant_id: seeded.tenant_id,
        task_id: Uuid::new_v4(),
        resume_at: now + Duration::seconds(offset),
    };

    let mut tx = direct.store.tx(seeded.tenant_id);
    let later = tx.emit_at(job(30), now + Duration::seconds(30));
    tx.emit_at(job(10), now + Duration::seconds(10));
    tx.emit_at(job(-5), now - Duration::seconds(5));
    assert!(direct.scheduler.pending().is_empty());
    tx.commit().await.unwrap();

    assert_eq!(direct.queue.len(), 1);
    assert_eq!(direct.scheduler.pending().len(), 2);

    let mut tx = direct.store.tx(seeded.tenant_id);
    tx.cancel_schedule(later);
    tx.commit().await.unwrap();
    let due = direct.scheduler.take_due(now + Duration::minutes(1));
    assert_eq!(due.len(), 1);
    let fired = Envelope::parse(&due[0].body).unwrap();
    assert!(
        matches!(fired.job, Job::TaskResume { resume_at, .. } if resume_at == now + Duration::seconds(10))
    );

    let messages = dynamo.scan(sanchoris_core::item::Table::Main, "msg").await;
    assert_eq!(messages.len(), 4);
    assert!(messages.iter().all(|m| {
        sanchoris_core::store::relayed_from_item(m)
            .unwrap()
            .is_some()
    }));
}

#[tokio::test]
async fn tenant_transactions_cannot_touch_other_tenants_or_plans() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let a = dynamo.seed_tenant("pro").await;
    let b = dynamo.seed_tenant("pro").await;
    let task_a = create_task(&store, &a, "shared-name", &[]).await.id();
    let task_b = create_task(&store, &b, "shared-name", &[]).await.id();
    assert_ne!(task_a, task_b);

    let mut tx = store.tx(a.tenant_id);
    let foreign = keys::task_key(b.tenant_id, task_b);
    assert!(matches!(
        tx.get::<Task>(&foreign).await,
        Err(DbError::CrossTenant { .. })
    ));
    assert!(matches!(
        tx.update(&foreign, Update::new().set_s("title", "stolen")),
        Err(DbError::CrossTenant { .. })
    ));
    assert!(matches!(
        tx.delete(&keys::tenant_key(b.tenant_id)),
        Err(DbError::CrossTenant { .. })
    ));
    let prefixed = Key::main(format!("T#{}0", a.tenant_id), "#TENANT");
    assert!(matches!(
        tx.get_item(&prefixed).await,
        Err(DbError::CrossTenant { .. })
    ));
    assert!(matches!(
        tx.update(
            &keys::plan_key("pro"),
            Update::new().set_n("max_concurrency", 100)
        ),
        Err(DbError::CrossTenant { .. })
    ));
    let own = tx
        .query::<Task>(&keys::task_pk(a.tenant_id, task_a), "")
        .await
        .unwrap();
    assert_eq!(own.iter().map(|t| t.id).collect::<Vec<_>>(), vec![task_a]);
    let plan = db::plan(&store, "pro").await.unwrap();
    assert_eq!(plan.max_concurrency, 5);
    assert_eq!(Plan::KIND, "plan");
}

#[tokio::test]
async fn audit_events_are_append_only() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("pro").await;
    let at = Utc::now();

    let mut tx = store.tx(seeded.tenant_id);
    let id = record_audit(
        &mut tx,
        &AuditEntry {
            at,
            actor_kind: ActorKind::Member,
            actor_ref: &seeded.member_id.to_string(),
            action: "trigger.enable",
            target_kind: "trigger",
            target_ref: "labeled-issue",
            summary: "Enabled labeled-issue",
            origin: AuditOrigin::Web,
            source_ip: None,
            metadata: Map::new(),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let key = keys::audit_key(seeded.tenant_id, at, id);
    let mut tx = store.tx(seeded.tenant_id);
    assert!(matches!(
        tx.update(&key, Update::new().set_s("summary", "changed")),
        Err(DbError::AppendOnly(_))
    ));
    assert!(matches!(tx.delete(&key), Err(DbError::AppendOnly(_))));
    let events = audit_events(&mut tx).await.unwrap();
    let mut replay = events[0].clone();
    replay.summary = "changed".into();
    assert!(matches!(tx.put(&replay), Err(DbError::AppendOnly(_))));
    tx.insert(&replay).unwrap();
    assert!(matches!(tx.commit().await, Err(DbError::Conflict)));

    let stored = dynamo.all::<AuditEvent>().await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].summary, "Enabled labeled-issue");
    assert_eq!(stored[0].metadata["origin"], "web");
    let plan = db::plan(&store, "pro").await.unwrap();
    assert_eq!(
        stored[0].expires_at.timestamp(),
        (at + Duration::days(plan.audit_retention_days)).timestamp()
    );
}

#[tokio::test]
async fn large_documents_are_stored_in_the_blob_store() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("pro").await;
    let mut definition = definition();
    definition.skills[0].definition = json!({ "SkillMd": "x".repeat(200_000) });
    let mut tx = store.tx(seeded.tenant_id);
    let recorded = db::create_task(
        &mut tx,
        &NewTask {
            trigger_id: seeded.trigger_id,
            trigger_event_id: None,
            config_revision_id: seeded.config_revision_id,
            name: "big",
            title: "big",
            definition: &definition,
            parent_task_id: None,
            channel: None,
            requested_by_member_id: None,
            requested_by_github_user_id: None,
            requested_by_github_login: None,
            blocked_by: &[],
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let keys_stored = dynamo.blobs().keys();
    assert_eq!(keys_stored.len(), 1);
    assert!(keys_stored[0].starts_with(&format!("docs/{}/", seeded.tenant_id)));
    let task = store
        .get::<Task>(&keys::task_key(seeded.tenant_id, recorded.id()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        task.definition["Skills"][0]["Definition"]["SkillMd"]
            .as_str()
            .unwrap()
            .len(),
        200_000
    );
}

#[tokio::test]
async fn reads_inside_a_transaction_see_its_own_writes() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("pro").await;
    let id = create_task(&store, &seeded, "visible", &[]).await.id();
    let mut tx = store.tx(seeded.tenant_id);
    let key = keys::task_key(seeded.tenant_id, id);
    let mut task = tx.get::<Task>(&key).await.unwrap().unwrap();
    task.title = "renamed".into();
    tx.put(&task).unwrap();
    tx.update(
        &keys::tenant_key(seeded.tenant_id),
        Update::new().add("live_workspaces", 2),
    )
    .unwrap();
    assert_eq!(
        tx.get::<Task>(&key).await.unwrap().unwrap().title,
        "renamed"
    );
    assert_eq!(db::tenant(&mut tx).await.unwrap().live_workspaces, 2);
    let names = tx
        .query::<Task>(&keys::task_pk(seeded.tenant_id, id), "#")
        .await
        .unwrap();
    assert_eq!(names[0].title, "renamed");

    let mut racing = store.tx(seeded.tenant_id);
    let mut other = racing.get::<Task>(&key).await.unwrap().unwrap();
    other.title = "first".into();
    racing.put(&other).unwrap();
    racing.commit().await.unwrap();
    assert!(matches!(tx.commit().await, Err(DbError::Conflict)));
    assert_eq!(tenant(&store, seeded.tenant_id).await.live_workspaces, 0);
}

#[tokio::test]
async fn unread_deletes_require_the_item_to_exist() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("pro").await;
    let id = create_task(&store, &seeded, "deleted", &[]).await.id();
    let mut tx = store.tx(seeded.tenant_id);
    tx.delete(&keys::task_key(seeded.tenant_id, Uuid::new_v4()))
        .unwrap();
    assert!(matches!(tx.commit().await, Err(DbError::Conflict)));

    let key = keys::task_key(seeded.tenant_id, id);
    let mut tx = store.tx(seeded.tenant_id);
    tx.delete(&key).unwrap();
    tx.commit().await.unwrap();
    assert!(store.get::<Task>(&key).await.unwrap().is_none());
}

#[tokio::test]
async fn conditional_updates_of_a_staged_put_are_rejected() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("pro").await;
    let key = keys::tenant_key(seeded.tenant_id);
    let mut tx = store.tx(seeded.tenant_id);
    let mut tenant = tx.require::<Tenant>(&key).await.unwrap();
    tenant.name = "Renamed".into();
    tx.put(&tenant).unwrap();
    assert!(matches!(
        tx.update(
            &key,
            Update::new()
                .add("tasks_this_period", 1)
                .cap_after("tasks_this_period", 5)
        ),
        Err(DbError::ConditionOnStagedPut(_))
    ));
    assert!(matches!(
        tx.update(
            &key,
            Update::new().require("status", AttributeValue::S("active".into()))
        ),
        Err(DbError::ConditionOnStagedPut(_))
    ));
    tx.update(&key, Update::new().add("tasks_this_period", 1))
        .unwrap();
    tx.commit().await.unwrap();
    let stored = store.get::<Tenant>(&key).await.unwrap().unwrap();
    assert_eq!(
        (stored.name.as_str(), stored.tasks_this_period),
        ("Renamed", 1)
    );
}

#[tokio::test]
async fn an_admin_tenant_dispatches_past_any_task_or_workspace_count() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("admin").await;
    dynamo
        .modify::<Tenant>(&keys::tenant_key(seeded.tenant_id), |t| {
            t.tasks_this_period = 1_000_000;
            t.workspace_seconds_this_period = 1_000_000_000;
        })
        .await;
    for name in ["a", "b", "c"] {
        create_task(&store, &seeded, name, &[]).await;
    }

    assert_eq!(dispatch(&store, seeded.tenant_id).await.len(), 3);
    let t = tenant(&store, seeded.tenant_id).await;
    assert_eq!((t.tasks_this_period, t.live_workspaces), (1_000_003, 3));

    let plan = db::plan(&store, "admin").await.unwrap();
    assert_eq!(
        (
            plan.task_limit,
            plan.workspace_seconds_limit,
            plan.member_limit,
            plan.max_concurrency,
            plan.workspace_cpu,
            plan.workspace_memory_mib,
            plan.price_cents,
            plan.stripe_price_id,
        ),
        (None, None, None, 10, 2048, 4096, 0, None)
    );
    assert_eq!(plan.computes, ["standard"]);
}

#[tokio::test]
async fn a_tenant_on_an_unknown_plan_dispatches_nothing() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let seeded = dynamo.seed_tenant("pro").await;
    dynamo
        .modify::<Tenant>(&keys::tenant_key(seeded.tenant_id), |t| {
            t.plan_id = "enterprise".into();
        })
        .await;
    create_task(&store, &seeded, "a", &[]).await;

    let mut tx = store.tx(seeded.tenant_id);
    assert!(matches!(
        db::dispatch_pending_tasks(&mut tx, Utc::now()).await,
        Err(DbError::Missing(name)) if name == "plan enterprise"
    ));
    assert_eq!(tenant(&store, seeded.tenant_id).await.live_workspaces, 0);
}

#[tokio::test]
async fn a_plan_item_without_a_limit_attribute_does_not_decode() {
    let dynamo = TestDynamo::start().await;
    let store = dynamo.store(Relay::Stream);
    let mut item = db::plan(&store, "free").await.unwrap().to_item();
    item.remove("task_limit");
    dynamo.put_raw(item).await;

    assert!(matches!(
        db::plan(&store, "free").await,
        Err(DbError::Decode(message)) if message == "plan has no attribute task_limit"
    ));
}

#[tokio::test]
async fn dispatch_deletes_stale_markers_in_bounded_transactions() {
    let dynamo = TestDynamo::start().await;
    let direct = dynamo.direct_store(Arc::new(FakeClock::new(Utc::now())));
    let seeded = dynamo.seed_tenant("pro").await;
    let mut stale = Vec::new();
    for n in 0..130 {
        stale.push(
            create_task(&direct.store, &seeded, &format!("stale-{n}"), &[])
                .await
                .id(),
        );
    }
    for id in &stale {
        dynamo
            .modify::<Task>(&keys::task_key(seeded.tenant_id, *id), |task| {
                task.status = TaskStatus::Aborted
            })
            .await;
    }
    let live = create_task(&direct.store, &seeded, "live", &[]).await.id();
    let dispatch_jobs = || {
        direct
            .queue
            .bodies()
            .iter()
            .filter(|body| matches!(Envelope::parse(body).unwrap().job, Job::Dispatch { .. }))
            .count()
    };
    let markers = || async {
        direct
            .store
            .query::<PendingTask>(&keys::tenant_pk(seeded.tenant_id), "PENDINGTASK#")
            .await
            .unwrap()
            .len()
    };

    let before = dispatch_jobs();
    let first = dispatch(&direct.store, seeded.tenant_id).await;
    assert_eq!(
        first.iter().map(|d| d.task_id).collect::<Vec<_>>(),
        vec![live]
    );
    assert!(markers().await > 0);
    assert_eq!(dispatch_jobs(), before + 1);

    let mut rounds = 0;
    while markers().await > 0 {
        assert!(dispatch(&direct.store, seeded.tenant_id).await.is_empty());
        rounds += 1;
        assert!(rounds < 3);
    }
    assert_eq!(dispatch_jobs(), before + 1 + rounds - 1);
}
