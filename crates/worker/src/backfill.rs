use sanchoris_core::db::{REF_TRIGGER_EVENT, locate};
use sanchoris_core::item::{Entity, Item, ItemReader, KIND, Key, Table};
use sanchoris_core::items::{
    CheckWaitMarker, Config, GITHUB_TASK_PREFIX, Output, Task, TaskName, Tenant, TriggerEvent,
    github_task_name,
};
use sanchoris_core::keys;
use sanchoris_core::store::{Index, SortKey};
use serde_json::json;
use uuid::Uuid;

use crate::config_sync::reactivate;
use crate::error::Result;
use crate::ports::{Installation, repository_named};
use crate::{Worker, installations};

const LEGACY_DEFINITION_PREFIXES: [&str; 3] = ["PROJECT#", "SESSION#", "SKILL#"];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BackfillReport {
    pub lines: Vec<String>,
}

pub async fn stable_keys(worker: &Worker) -> Result<BackfillReport> {
    let mut report = BackfillReport::default();
    let tenants = worker
        .store
        .query_index::<Tenant>(Index::Gsi2, keys::TENANTS_INDEX, SortKey::All)
        .await?;
    for tenant in tenants {
        let mut tx = worker.tx(tenant.id);
        let installations = installations(&mut tx).await?;
        definitions(worker, tenant.id, &installations, &mut report).await?;
        task_names(worker, tenant.id, &mut report).await?;
        check_wait_markers(worker, tenant.id, &installations, &mut report).await?;
        outputs(worker, tenant.id, &installations, &mut report).await?;
    }
    Ok(report)
}

fn kind_of(item: &Item) -> Option<&str> {
    item.get(KIND)
        .and_then(|value| value.as_s().ok())
        .map(String::as_str)
}

async fn definitions(
    worker: &Worker,
    tenant_id: Uuid,
    installations: &[Installation],
    report: &mut BackfillReport,
) -> Result<()> {
    let configs = worker
        .store
        .query::<Config>(&keys::tenant_pk(tenant_id), "CONFIG#")
        .await?;
    for config in configs {
        let pk = keys::config_pk(tenant_id, config.id);
        let mut legacy = Vec::new();
        for prefix in LEGACY_DEFINITION_PREFIXES {
            for item in worker
                .store
                .query_items(Table::Main, None, &pk, &SortKey::prefix(prefix), true, None)
                .await?
            {
                legacy.push(Key::of(Table::Main, &item)?);
            }
        }
        if legacy.is_empty() {
            continue;
        }
        if config.is_active() && config.active_revision_id.is_some() {
            reactivate(worker, tenant_id, config.id, installations).await?;
        }
        for chunk in legacy.chunks(90) {
            let mut tx = worker.tx(tenant_id);
            for key in chunk {
                tx.get_item(key).await?;
                tx.delete(key)?;
            }
            tx.commit().await?;
        }
        report.lines.push(format!(
            "tenant {tenant_id} config {} ({}): moved the definitions under the active revision and deleted {} items of the former layout",
            config.id,
            config.repository,
            legacy.len()
        ));
    }
    Ok(())
}

async fn task_names(worker: &Worker, tenant_id: Uuid, report: &mut BackfillReport) -> Result<()> {
    let tasks = worker
        .store
        .query_index::<Task>(Index::Gsi1, &keys::tasks_index(tenant_id), SortKey::All)
        .await?;
    for listed in tasks {
        if listed.name.starts_with(GITHUB_TASK_PREFIX) {
            continue;
        }
        let Some(repository) = listed
            .channel
            .as_ref()
            .filter(|channel| channel["source"] == "github")
            .and_then(|channel| channel["repository"].as_str())
            .map(str::to_string)
        else {
            continue;
        };
        let Some(suffix) = listed.name.strip_prefix(&repository).map(str::to_string) else {
            report.lines.push(format!(
                "tenant {tenant_id} task {}: the name {} does not start with {repository}, left as it is",
                listed.id, listed.name
            ));
            continue;
        };
        let mut tx = worker.tx(tenant_id);
        let mut task = tx.require::<Task>(&listed.key()).await?;
        let Some(trigger_event_id) = task.trigger_event_id else {
            continue;
        };
        let Some(location) = locate(&mut tx, trigger_event_id, REF_TRIGGER_EVENT).await? else {
            continue;
        };
        let event = tx.require::<TriggerEvent>(&location.target).await?;
        let Some(repository_id) = event.event["detail"]["repository_id"].as_i64() else {
            report.lines.push(format!(
                "tenant {tenant_id} task {}: the trigger event has no repository id, left as it is",
                task.id
            ));
            continue;
        };
        let old_name = std::mem::replace(&mut task.name, github_task_name(repository_id, &suffix));
        if let Some(channel) = task.channel.as_mut() {
            channel["repository_id"] = json!(repository_id);
        }
        tx.put(&task)?;
        let old_key = keys::task_name_key(tenant_id, task.trigger_id, &old_name);
        let created_at = match tx.get::<TaskName>(&old_key).await? {
            Some(old) => {
                tx.delete(&old_key)?;
                old.created_at
            }
            None => task.created_at,
        };
        let renamed = TaskName {
            tenant_id,
            trigger_id: task.trigger_id,
            name: task.name.clone(),
            task_id: task.id,
            created_at,
        };
        if tx.get::<TaskName>(&renamed.key()).await?.is_none() {
            tx.insert(&renamed)?;
        }
        tx.commit().await?;
        report.lines.push(format!(
            "tenant {tenant_id} task {}: {old_name} -> {}",
            task.id, task.name
        ));
    }
    Ok(())
}

async fn check_wait_markers(
    worker: &Worker,
    tenant_id: Uuid,
    installations: &[Installation],
    report: &mut BackfillReport,
) -> Result<()> {
    let items = worker
        .store
        .query_items(
            Table::Main,
            None,
            &keys::tenant_pk(tenant_id),
            &SortKey::prefix("CHECKWAIT#"),
            true,
            None,
        )
        .await?;
    for item in items.iter().filter(|item| {
        kind_of(item) == Some("check_wait_marker") && !item.contains_key("repository_ids")
    }) {
        let r = ItemReader::new(item);
        let repos = r.strs("repos")?;
        let ids = repos
            .iter()
            .map(|repo| repository_named(installations, repo).map(|found| found.id))
            .collect::<Option<Vec<_>>>();
        let key = Key::of(Table::Main, item)?;
        let mut tx = worker.tx(tenant_id);
        tx.get_item(&key).await?;
        match ids {
            Some(repository_ids) => {
                let marker = CheckWaitMarker {
                    tenant_id,
                    wait_id: r.uuid("wait_id")?,
                    task_id: r.uuid("task_id")?,
                    repos: repos.clone(),
                    repository_ids,
                };
                tx.put(&marker)?;
                report.lines.push(format!(
                    "tenant {tenant_id} check wait {}: {} -> {:?}",
                    marker.wait_id,
                    repos.join(", "),
                    marker.repository_ids
                ));
            }
            None => {
                tx.delete(&key)?;
                report.lines.push(format!(
                    "tenant {tenant_id} check wait {}: {} is not in the installation lists; deleted the webhook marker, polling continues",
                    r.uuid("wait_id")?,
                    repos.join(", ")
                ));
            }
        }
        tx.commit().await?;
    }
    Ok(())
}

async fn outputs(
    worker: &Worker,
    tenant_id: Uuid,
    installations: &[Installation],
    report: &mut BackfillReport,
) -> Result<()> {
    let items = worker
        .store
        .query_items(
            Table::Main,
            Some(Index::Gsi1),
            &keys::outputs_index(tenant_id),
            &SortKey::All,
            true,
            None,
        )
        .await?;
    for item in items
        .iter()
        .filter(|item| kind_of(item) == Some("output") && !item.contains_key("repository_id"))
    {
        let r = ItemReader::new(item);
        let repository = r.s("repository")?;
        let url = r.s("url")?;
        let task = worker
            .store
            .get::<Task>(&keys::task_key(tenant_id, r.uuid("task_id")?))
            .await?;
        let of_task = task
            .and_then(|task| task.channel)
            .filter(|channel| channel["repository"] == repository.as_str())
            .and_then(|channel| channel["repository_id"].as_i64());
        let resolved = repository_named(installations, &repository)
            .or_else(|| {
                let path = url.strip_prefix("https://github.com/")?;
                let mut parts = path.split('/');
                let name = format!("{}/{}", parts.next()?, parts.next()?);
                repository_named(installations, &name)
            })
            .map(|found| found.id)
            .or(of_task);
        let old_key = Key::of(Table::Main, item)?;
        let mut tx = worker.tx(tenant_id);
        tx.get_item(&old_key).await?;
        tx.delete(&old_key)?;
        match resolved {
            Some(repository_id) => {
                let output = Output {
                    id: r.uuid("id")?,
                    tenant_id,
                    task_id: r.uuid("task_id")?,
                    skill_run_id: r.uuid("skill_run_id")?,
                    skill_run_number: r.i32("skill_run_number")?,
                    kind: r.s("output_kind")?,
                    repository: repository.clone(),
                    repository_id,
                    number: r.n("number")?,
                    url: url.clone(),
                    state: r.opt_s("state")?,
                    created_at: r.time("created_at")?,
                    updated_at: r.time("updated_at")?,
                };
                if tx.get::<Output>(&output.key()).await?.is_none() {
                    tx.insert(&output)?;
                }
                report.lines.push(format!(
                    "tenant {tenant_id} output {url}: {repository} -> {repository_id}"
                ));
            }
            None => report.lines.push(format!(
                "tenant {tenant_id} output {url}: {repository} is not in the installation lists; deleted"
            )),
        }
        tx.commit().await?;
    }
    Ok(())
}
