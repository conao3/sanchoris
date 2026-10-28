use std::collections::BTreeMap;

use sanchoris_core::audit::{AuditEntry, record_audit};
use sanchoris_core::item::{Item, ItemReader, KIND, Key, Table};
use sanchoris_core::items::{CONFIG_ACTIVE, Config, Guard, Integration, Tenant};
use sanchoris_core::keys;
use sanchoris_core::model::{ActorKind, AuditOrigin};
use sanchoris_core::store::{Index, SortKey};
use serde_json::{Map, json};
use uuid::Uuid;

use crate::config_sync::{disable_config, reactivate, resolve_repository};
use crate::error::{Result, WorkerError};
use crate::{Worker, installations};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BackfillReport {
    pub lines: Vec<String>,
}

pub async fn repository_ids(worker: &Worker) -> Result<BackfillReport> {
    let mut report = BackfillReport::default();
    let tenants = worker
        .store
        .query_index::<Tenant>(Index::Gsi2, keys::TENANTS_INDEX, SortKey::All)
        .await?;
    for tenant in tenants {
        backfill_tenant(worker, tenant.id, &mut report).await?;
    }
    Ok(report)
}

async fn backfill_tenant(
    worker: &Worker,
    tenant_id: Uuid,
    report: &mut BackfillReport,
) -> Result<()> {
    let mut tx = worker.tx(tenant_id);
    for mut integration in tx
        .query::<Integration>(&keys::tenant_pk(tenant_id), "INTEG#")
        .await?
    {
        let Some(installation) = integration
            .github_installation_id
            .filter(|_| integration.kind == "github_installation")
        else {
            continue;
        };
        let repositories = if integration.status == "connected" {
            worker
                .deps
                .config_source
                .installation_repositories(installation)
                .await?
        } else {
            Vec::new()
        };
        report.lines.push(format!(
            "tenant {tenant_id} installation {installation} ({}): {} repositories: {}",
            integration.status,
            repositories.len(),
            repositories
                .iter()
                .map(|r| format!("{} ({})", r.full_name, r.id))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        integration.details["repositories"] = json!(repositories);
        integration.updated_at = worker.now();
        tx.put(&integration)?;
    }
    tx.commit().await?;

    let mut tx = worker.tx(tenant_id);
    let installations = installations(&mut tx).await?;
    let legacy = tx
        .query_items(Table::Main, &keys::tenant_pk(tenant_id), "CONFIG#")
        .await?
        .into_iter()
        .filter(|item| kind_of(item) == Some("config") && !item.contains_key("status"))
        .collect::<Vec<_>>();
    for item in legacy {
        backfill_config(worker, tenant_id, &installations, &item, report).await?;
    }
    Ok(())
}

fn kind_of(item: &Item) -> Option<&str> {
    item.get(KIND)
        .and_then(|value| value.as_s().ok())
        .map(String::as_str)
}

async fn backfill_config(
    worker: &Worker,
    tenant_id: Uuid,
    installations: &[crate::ports::Installation],
    item: &Item,
    report: &mut BackfillReport,
) -> Result<()> {
    let r = ItemReader::new(item);
    let config_id = r.uuid("id")?;
    let name = r.s("repository")?;
    let root_path = r.s("root_path")?;
    let resolved =
        match resolve_repository(worker.deps.config_source.as_ref(), installations, &name).await {
            Ok(found) => Some(found),
            Err(WorkerError::Invalid(_)) => None,
            Err(error) => return Err(error),
        };
    let now = worker.now();
    let mut tx = worker.tx(tenant_id);
    tx.get_item(&keys::config_key(tenant_id, config_id)).await?;
    let legacy_guard = Key::main(
        keys::tenant_pk(tenant_id),
        format!("CONFIGREPO#{name}#{root_path}"),
    );
    if tx.get_item(&legacy_guard).await?.is_some() {
        tx.delete(&legacy_guard)?;
    }
    for repository in tx
        .query_items(
            Table::Main,
            &keys::config_pk(tenant_id, config_id),
            "PROJECT#",
        )
        .await?
        .into_iter()
        .filter(|item| {
            kind_of(item) == Some("repository") && !item.contains_key("github_repository_id")
        })
    {
        tx.delete(&Key::of(Table::Main, &repository)?)?;
    }
    let config = Config {
        id: config_id,
        tenant_id,
        repository_id: resolved.as_ref().map(|found| found.id),
        repository: resolved
            .as_ref()
            .map_or_else(|| name.clone(), |found| found.full_name.clone()),
        root_path: root_path.clone(),
        branch: r.s("branch")?,
        status: CONFIG_ACTIVE.into(),
        active_revision_id: r.opt_uuid("active_revision_id")?,
        revisions: r.n("revisions")?,
        repositories: BTreeMap::new(),
        synced_at: r.opt_time("synced_at")?,
        sync_error: None,
        created_at: r.time("created_at")?,
        updated_at: now,
    };
    tx.put(&config)?;
    if let Some(found) = &resolved {
        tx.insert(&Guard::new(
            keys::config_repository_key(tenant_id, found.id, &root_path),
            config_id,
        ))?;
        let config_ref = config_id.to_string();
        let mut metadata = Map::new();
        metadata.insert("repository_id".into(), json!(found.id));
        metadata.insert("stored_name".into(), json!(name));
        record_audit(
            &mut tx,
            &AuditEntry {
                at: now,
                actor_kind: ActorKind::System,
                actor_ref: "backfill",
                action: "config.repository_id",
                target_kind: "config",
                target_ref: &config_ref,
                summary: &format!(
                    "The config {name} follows the repository {} ({})",
                    found.full_name, found.id
                ),
                origin: AuditOrigin::System,
                source_ip: None,
                metadata,
            },
        )
        .await?;
    }
    tx.commit().await?;
    match resolved {
        Some(found) => {
            let outcome = match reactivate(worker, tenant_id, config_id, installations).await {
                Ok(()) => "active".to_string(),
                Err(WorkerError::Invalid(message)) => {
                    let mut tx = worker.tx(tenant_id);
                    let mut config = tx
                        .require::<Config>(&keys::config_key(tenant_id, config_id))
                        .await?;
                    config.sync_error = Some(message.clone());
                    tx.put(&config)?;
                    tx.commit().await?;
                    format!("active with a sync error: {message}")
                }
                Err(error) => return Err(error),
            };
            report.lines.push(format!(
                "tenant {tenant_id} config {config_id}: {name} -> {} ({}), {outcome}",
                found.full_name, found.id
            ));
        }
        None => {
            let reason =
                format!("{name} is not a repository of the tenant's GitHub App installation");
            disable_config(&worker.store, tenant_id, config_id, &reason, now).await?;
            report.lines.push(format!(
                "tenant {tenant_id} config {config_id}: {name} disabled: {reason}"
            ));
        }
    }
    Ok(())
}
