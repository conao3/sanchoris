use crate::common;

use chrono::Duration;
use common::{INSTALLATION, World, labeled_payload};
use sanchoris_core::audit::{AuditEntry, record_audit};
use sanchoris_core::channel::{IngestError, ingest_github_delivery, record_inbound_event};
use sanchoris_core::item::{Entity, Key, Table};
use sanchoris_core::items::{
    AuditEvent, Guard, InboundEvent, Invitation, InvitationEmail, RunLogEntry, Tenant, Trigger,
    Workspace, uniq,
};
use sanchoris_core::jobs::{Deletion, Envelope, Job, schedule_name};
use sanchoris_core::keys;
use sanchoris_core::model::{ActorKind, AuditOrigin};
use sanchoris_core::ports::{BlobStore, Queue};
use sanchoris_core::tenants::{TENANT_DELETING, free_github_tenant_key, free_tenant_key};
use sanchoris_worker::tenant_delete::ITEMS_PER_JOB;
use serde_json::{Map, json};
use uuid::Uuid;

const LOG_ENTRIES: i64 = 250;

async fn mark_deleting(world: &World, tenant_id: Uuid) {
    world
        .dynamo
        .modify::<Tenant>(&keys::tenant_key(tenant_id), |tenant| {
            tenant.status = TENANT_DELETING.into();
        })
        .await;
}

async fn tenant_keys(world: &World, tenant_id: Uuid) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut after = None;
    loop {
        let (page, next) = world
            .store
            .scan_tenant_keys(tenant_id, after.as_ref(), 1000)
            .await
            .unwrap();
        keys.extend(page);
        match next {
            Some(key) => after = Some(key),
            None => return keys,
        }
    }
}

async fn take_deletion(world: &World) -> Option<Deletion> {
    let mut found = None;
    for message in world.queue.receive(usize::MAX).await.unwrap() {
        world.queue.delete(&message.receipt).await.unwrap();
        if let Job::DeleteTenant { phase, .. } = Envelope::parse(&message.body).unwrap().job {
            assert!(found.is_none(), "one deletion step at a time");
            found = Some(phase);
        }
    }
    found
}

#[tokio::test]
async fn a_deleting_tenant_takes_no_webhooks_inbound_events_or_trigger_schedules() {
    let world = World::new("pro").await;
    let tenant_id = world.tenant();
    mark_deleting(&world, tenant_id).await;

    let ingested = ingest_github_delivery(
        &world.store,
        "delivery-deleting",
        "issues",
        &labeled_payload(7, "Fix it", "mika-dev"),
    )
    .await;
    assert!(
        matches!(ingested, Err(IngestError::UnknownInstallation(id)) if id == INSTALLATION),
        "{ingested:?}"
    );

    let mut tx = world.store.tx(tenant_id);
    record_inbound_event(&mut tx, "web", "chat-1", &json!({ "detail": {} }))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    for job in [
        Job::Inbound { tenant_id },
        Job::Dispatch { tenant_id },
        Job::TriggerSchedule {
            tenant_id,
            config_id: world.seeded.config_id,
            trigger_id: world.seeded.trigger_id,
            due_at: world.clock_now(),
        },
        Job::ConfigSync {
            tenant_id,
            config_id: world.seeded.config_id,
        },
    ] {
        assert_eq!(
            world
                .worker
                .worker
                .handle(&Envelope::new(job))
                .await
                .unwrap(),
            0
        );
    }
    let inbound = world.dynamo.all::<InboundEvent>().await;
    assert!(inbound.iter().all(|event| event.processed_at.is_none()));
}

#[tokio::test]
async fn the_cleanup_job_removes_the_tenant_in_bounded_steps_and_keeps_the_free_guards() {
    let world = World::new("free").await;
    let tenant_id = world.tenant();
    let user_id = world.seeded.user_id;
    let other = world.dynamo.seed_tenant("pro").await;
    let now = world.clock_now();
    let slug = format!("tenant-{}", user_id.simple());
    world
        .dynamo
        .modify::<Tenant>(&keys::tenant_key(tenant_id), |tenant| {
            tenant.stripe_customer_id = Some("cus_deleted".into());
        })
        .await;
    for guard in [
        Guard::of_tenant(keys::uniq_key(uniq::SLUG, &slug), tenant_id, tenant_id),
        Guard::of_tenant(
            keys::uniq_key(uniq::STRIPE_CUSTOMER, "cus_deleted"),
            tenant_id,
            tenant_id,
        ),
        Guard::of_tenant(free_tenant_key(user_id), tenant_id, tenant_id),
        Guard::of_tenant(free_github_tenant_key(1001), tenant_id, tenant_id),
    ] {
        world.dynamo.put(&guard).await;
    }
    let invitation_id = Uuid::new_v4();
    world
        .dynamo
        .put(&Invitation {
            id: invitation_id,
            tenant_id,
            email: "zed@acme.dev".into(),
            role: "member".into(),
            status: "pending".into(),
            invited_by_member_id: world.seeded.member_id,
            expires_at: now + Duration::days(7),
            accepted_at: None,
            created_at: now,
        })
        .await;
    world
        .dynamo
        .put(&InvitationEmail {
            email: "zed@acme.dev".into(),
            tenant_id,
            invitation_id,
        })
        .await;
    let run_id = Uuid::new_v4();
    for seq in 0..LOG_ENTRIES {
        world
            .dynamo
            .put(&RunLogEntry {
                tenant_id,
                run_id,
                seq,
                turn: 1,
                kind: "agent".into(),
                data: json!({ "text": format!("line {seq}") }),
                at: now,
                expires_at: now + Duration::days(30),
            })
            .await;
    }
    let task_id = Uuid::new_v4();
    world
        .dynamo
        .put(&Workspace {
            id: Uuid::new_v4(),
            tenant_id,
            task_id,
            run_id,
            compute: "standard".into(),
            architecture: "arm64".into(),
            cpu: 1024,
            memory_mib: 4096,
            platform: "local".into(),
            platform_ref: Some("ws-running".into()),
            capacity_provider: None,
            repositories: vec!["acme/shop-web".into()],
            branch: "agent/1".into(),
            base_commit: None,
            started_at: now,
            signaled_at: None,
            stopped_at: None,
        })
        .await;
    let mut tx = world.store.tx(tenant_id);
    let schedule = tx.emit_at(
        Job::TriggerSchedule {
            tenant_id,
            config_id: world.seeded.config_id,
            trigger_id: world.seeded.trigger_id,
            due_at: now + Duration::hours(1),
        },
        now + Duration::hours(1),
    );
    tx.commit().await.unwrap();
    world
        .dynamo
        .modify::<Trigger>(
            &keys::trigger_key(tenant_id, world.seeded.config_id, "labeled-issue"),
            |trigger| trigger.schedule_message_id = Some(schedule),
        )
        .await;
    let docs = world.dynamo.blobs();
    docs.put(&format!("docs/{tenant_id}/definition"), b"{}".to_vec())
        .await
        .unwrap();
    docs.put(
        &format!("docs/{}/definition", other.tenant_id),
        b"{}".to_vec(),
    )
    .await
    .unwrap();
    for name in ["turn-1.jsonl", "turn-2.jsonl"] {
        world
            .records
            .put(
                &format!("t/{tenant_id}/runs/{run_id}/s/{name}"),
                b"{}".to_vec(),
            )
            .await
            .unwrap();
    }
    world
        .records
        .put(
            &format!("t/{}/audit/log.jsonl", other.tenant_id),
            b"{}".to_vec(),
        )
        .await
        .unwrap();
    let mut tx = world.store.tx(tenant_id);
    record_audit(
        &mut tx,
        &AuditEntry {
            at: now,
            actor_kind: ActorKind::System,
            actor_ref: "test",
            action: "tenant.create",
            target_kind: "tenant",
            target_ref: &tenant_id.to_string(),
            summary: "Created",
            origin: AuditOrigin::System,
            source_ip: None,
            metadata: Map::new(),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let audit_before = world
        .dynamo
        .all::<AuditEvent>()
        .await
        .into_iter()
        .filter(|event| event.tenant_id == tenant_id)
        .count();
    assert!(audit_before > 0);
    let other_keys = tenant_keys(&world, other.tenant_id).await;
    assert!(tenant_keys(&world, tenant_id).await.len() > 2 * ITEMS_PER_JOB);
    assert!(world.queue.receive(usize::MAX).await.unwrap().is_empty());

    mark_deleting(&world, tenant_id).await;
    let mut steps = Vec::new();
    let mut phase = Some(Deletion::Release);
    while let Some(current) = phase {
        let handled = world
            .worker
            .worker
            .handle(&Envelope::new(Job::DeleteTenant {
                tenant_id,
                phase: current.clone(),
            }))
            .await
            .unwrap();
        if let Deletion::Items { .. } = current {
            assert!(handled <= ITEMS_PER_JOB);
        }
        steps.push(current);
        phase = take_deletion(&world).await;
    }

    let items = steps
        .iter()
        .filter(|step| matches!(step, Deletion::Items { .. }))
        .count();
    let passes = steps
        .iter()
        .filter(|step| matches!(step, Deletion::Items { after: None, .. }))
        .count();
    assert!(items >= 4, "{steps:?}");
    assert_eq!(passes, 2, "{steps:?}");
    assert_eq!(steps.first(), Some(&Deletion::Release));
    assert_eq!(steps.last(), Some(&Deletion::Finish));
    assert!(steps.contains(&Deletion::Blobs));

    assert!(tenant_keys(&world, tenant_id).await.is_empty());
    assert_eq!(tenant_keys(&world, other.tenant_id).await, other_keys);
    for key in [
        keys::membership_key(user_id, tenant_id),
        keys::invitation_email_key("zed@acme.dev", tenant_id, invitation_id),
        keys::uniq_key(uniq::SLUG, &slug),
        keys::uniq_key(uniq::STRIPE_CUSTOMER, "cus_deleted"),
        keys::uniq_key(uniq::INSTALLATION, &INSTALLATION.to_string()),
    ] {
        assert!(
            world.store.get_item(&key).await.unwrap().is_none(),
            "{key:?} remains"
        );
    }
    for key in [free_tenant_key(user_id), free_github_tenant_key(1001)] {
        let guard = world.store.get::<Guard>(&key).await.unwrap().unwrap();
        assert_eq!(guard.target, tenant_id);
    }
    assert!(
        world
            .scheduler
            .pending()
            .iter()
            .all(|(name, _)| *name != schedule_name(schedule))
    );
    assert_eq!(world.runner.stopped.lock().unwrap().clone(), vec![run_id]);
    assert!(
        docs.keys()
            .iter()
            .all(|key| !key.starts_with(&format!("docs/{tenant_id}/")))
    );
    assert!(
        docs.keys()
            .contains(&format!("docs/{}/definition", other.tenant_id))
    );
    assert_eq!(
        world.records.keys(),
        vec![format!("t/{}/audit/log.jsonl", other.tenant_id)]
    );
    let audit_after = world
        .dynamo
        .all::<AuditEvent>()
        .await
        .into_iter()
        .filter(|event| event.tenant_id == tenant_id)
        .count();
    assert_eq!(audit_after, audit_before);
    assert!(
        world
            .dynamo
            .scan(Table::Main, Tenant::KIND)
            .await
            .iter()
            .all(|item| Key::of(Table::Main, item).unwrap() != keys::tenant_key(tenant_id))
    );
}
