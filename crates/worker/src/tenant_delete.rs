use chrono::Duration;
use sanchoris_core::db::{DbError, Store};
use sanchoris_core::item::Key;
use sanchoris_core::items::{
    Config, Guard, Integration, Invitation, Member, Tenant, Trigger, Workspace, uniq,
};
use sanchoris_core::jobs::{Deletion, ItemCursor, Job};
use sanchoris_core::keys;
use sanchoris_core::store::{Index, SortKey, retry};
use sanchoris_core::tenants::{is_deleting, release_pending_checkout};
use uuid::Uuid;

use crate::Worker;
use crate::error::{Result, WorkerError};

pub const ITEMS_PER_JOB: usize = 100;
pub const SCAN_PAGE: usize = 1000;
pub const SCAN_PAGES_PER_JOB: usize = 10;
pub const BLOBS_PER_JOB: usize = 1000;
pub const RELEASE_RETRY_SECONDS: i64 = 30;
pub const LAUNCH_GRACE_MINUTES: i64 = 15;
const REFERENCES_PER_TX: usize = 40;

pub async fn delete_tenant(worker: &Worker, tenant_id: Uuid, phase: &Deletion) -> Result<usize> {
    let Some(tenant) = worker
        .store
        .get::<Tenant>(&keys::tenant_key(tenant_id))
        .await?
    else {
        return Ok(0);
    };
    if !is_deleting(&tenant) {
        return Err(WorkerError::Invalid(format!(
            "tenant {tenant_id} is {}, not deleting",
            tenant.status
        )));
    }
    match phase {
        Deletion::Release => release(worker, &tenant).await,
        Deletion::Items { after, deleted } => {
            items(worker, tenant_id, after.as_ref(), *deleted).await
        }
        Deletion::Blobs => blobs(worker, tenant_id).await,
        Deletion::Finish => finish(&worker.store, tenant_id).await,
    }
}

async fn next(store: &Store, tenant_id: Uuid, phase: Deletion) -> Result<()> {
    let mut tx = store.tx(tenant_id);
    tx.emit(Job::DeleteTenant { tenant_id, phase });
    tx.commit().await?;
    Ok(())
}

async fn release(worker: &Worker, tenant: &Tenant) -> Result<usize> {
    let store = &worker.store;
    let tenant_id = tenant.id;
    if !stop_workspaces(worker, tenant_id).await? {
        let mut tx = store.tx(tenant_id);
        tx.emit_at(
            Job::DeleteTenant {
                tenant_id,
                phase: Deletion::Release,
            },
            worker.now() + Duration::seconds(RELEASE_RETRY_SECONDS),
        );
        tx.commit().await?;
        return Ok(0);
    }
    cancel_trigger_schedules(store, tenant_id).await?;
    let pk = keys::tenant_pk(tenant_id);
    let mut references = Vec::new();
    for member in store.query::<Member>(&pk, "MEMBER#").await? {
        references.push(keys::membership_key(member.user_id, tenant_id));
    }
    for invitation in store.query::<Invitation>(&pk, "INVITE#").await? {
        references.push(keys::invitation_email_key(
            &invitation.email,
            tenant_id,
            invitation.id,
        ));
    }
    let mut guards = vec![keys::uniq_key(uniq::SLUG, &tenant.slug)];
    if let Some(customer) = &tenant.stripe_customer_id {
        guards.push(keys::uniq_key(uniq::STRIPE_CUSTOMER, customer));
    }
    for integration in store.query::<Integration>(&pk, "INTEG#").await? {
        if let Some(installation) = integration.github_installation_id {
            guards.push(keys::uniq_key(
                uniq::INSTALLATION,
                &installation.to_string(),
            ));
        }
        if let Some(team) = &integration.slack_team_id {
            guards.push(keys::uniq_key(uniq::SLACK_TEAM, team));
        }
    }
    for chunk in references.chunks(REFERENCES_PER_TX) {
        retry(8, || async {
            let mut tx = store.tx(tenant_id);
            for key in chunk {
                if tx.get_item(key).await?.is_some() {
                    tx.delete(key)?;
                }
            }
            tx.commit().await
        })
        .await?;
    }
    retry(8, || async {
        let mut tx = store.tx(tenant_id);
        for key in &guards {
            if tx.get::<Guard>(key).await?.is_some_and(|guard| {
                guard.target == tenant_id || guard.tenant_id == Some(tenant_id)
            }) {
                tx.delete(key)?;
            }
        }
        release_pending_checkout(&mut tx, tenant).await?;
        tx.commit().await
    })
    .await?;
    next(
        store,
        tenant_id,
        Deletion::Items {
            after: None,
            deleted: 0,
        },
    )
    .await?;
    Ok(1)
}

async fn stop_workspaces(worker: &Worker, tenant_id: Uuid) -> Result<bool> {
    let open = worker
        .store
        .query_index::<Workspace>(
            Index::Gsi1,
            &keys::workspaces_index(tenant_id),
            SortKey::All,
        )
        .await?
        .into_iter()
        .filter(|workspace| workspace.stopped_at.is_none());
    let launching_since = worker.now() - Duration::minutes(LAUNCH_GRACE_MINUTES);
    let mut settled = true;
    for workspace in open {
        match &workspace.platform_ref {
            Some(platform_ref) => {
                worker
                    .deps
                    .runner
                    .stop_workspace(workspace.run_id, platform_ref)
                    .await?;
            }
            None if workspace.started_at > launching_since => settled = false,
            None => {}
        }
    }
    Ok(settled)
}

async fn cancel_trigger_schedules(store: &Store, tenant_id: Uuid) -> Result<()> {
    let configs = store
        .query::<Config>(&keys::tenant_pk(tenant_id), "CONFIG#")
        .await?;
    let mut tx = store.tx(tenant_id);
    for config in &configs {
        for trigger in store
            .query::<Trigger>(&keys::config_pk(tenant_id, config.id), "TRIGGER#")
            .await?
        {
            if let Some(message_id) = trigger.schedule_message_id {
                tx.cancel_schedule(message_id);
            }
        }
    }
    tx.commit().await?;
    Ok(())
}

async fn items(
    worker: &Worker,
    tenant_id: Uuid,
    after: Option<&ItemCursor>,
    deleted: u64,
) -> Result<usize> {
    let store = &worker.store;
    let tenant_key = keys::tenant_key(tenant_id);
    let mut cursor = after.map(|cursor| Key::main(cursor.pk.clone(), cursor.sk.clone()));
    let mut found = Vec::new();
    for _ in 0..SCAN_PAGES_PER_JOB {
        let (keys, mut next_key) = store
            .scan_tenant_keys(tenant_id, cursor.as_ref(), SCAN_PAGE)
            .await?;
        for key in keys.into_iter().filter(|key| *key != tenant_key) {
            if found.len() == ITEMS_PER_JOB {
                next_key = found.last().cloned();
                break;
            }
            found.push(key);
        }
        cursor = next_key;
        if cursor.is_none() || found.len() >= ITEMS_PER_JOB {
            break;
        }
    }
    store.delete_keys(&found).await?;
    let deleted = deleted + found.len() as u64;
    let phase = match (cursor, deleted) {
        (Some(key), _) => Deletion::Items {
            after: Some(ItemCursor {
                pk: key.pk,
                sk: key.sk,
            }),
            deleted,
        },
        (None, 0) => Deletion::Blobs,
        (None, _) => Deletion::Items {
            after: None,
            deleted: 0,
        },
    };
    next(store, tenant_id, phase).await?;
    Ok(found.len())
}

async fn blobs(worker: &Worker, tenant_id: Uuid) -> Result<usize> {
    let docs = worker
        .store
        .blobs()
        .delete_prefix(&format!("docs/{tenant_id}/"), BLOBS_PER_JOB)
        .await?;
    let records = worker
        .deps
        .records
        .delete_prefix(&format!("t/{tenant_id}/"), BLOBS_PER_JOB)
        .await?;
    let phase = if docs + records > 0 {
        Deletion::Blobs
    } else {
        Deletion::Finish
    };
    next(&worker.store, tenant_id, phase).await?;
    Ok(docs + records)
}

async fn finish(store: &Store, tenant_id: Uuid) -> Result<usize> {
    retry(8, || async {
        let mut tx = store.tx(tenant_id);
        let key = keys::tenant_key(tenant_id);
        if tx.get_item(&key).await?.is_none() {
            return Ok::<_, DbError>(());
        }
        tx.delete(&key)?;
        tx.commit().await
    })
    .await?;
    Ok(1)
}
