use std::collections::BTreeSet;

use indexmap::IndexMap;
use sanchoris_config::{ConfigFiles, ConfigRepository, parse_repository, root_prefix, validate};
use sanchoris_core::audit::{AuditEntry, record_audit};
use sanchoris_core::db::TenantTx;
use sanchoris_core::item::Entity;
use sanchoris_core::items::{ChangeRequest, Config, Task};
use sanchoris_core::keys;
use sanchoris_core::model::{ActorKind, AuditOrigin};
use sanchoris_core::tasks::branch_task;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::config_sync::validation_context;
use crate::error::{Result, WorkerError};
use crate::ports::{PullRequest, installation_with};
use crate::{Worker, installations};

struct Changes {
    files: Vec<String>,
    definitions: Option<Vec<Value>>,
    validation_status: &'static str,
    validation_messages: Value,
}

/// Records the pull request `number` on the config's repository as a change request while it is open on the config's branch and changes the config's files, and keeps the title, state, changes, and validation of a recorded one current.
pub async fn sync(worker: &Worker, tenant_id: Uuid, config_id: Uuid, number: i64) -> Result<usize> {
    let now = worker.now();
    let mut tx = worker.tx(tenant_id);
    let config = tx
        .require::<Config>(&keys::config_key(tenant_id, config_id))
        .await?;
    let repository_id = config
        .repository_id
        .ok_or_else(|| WorkerError::Invalid(format!("config {config_id} has no repository id")))?;
    let installations = installations(&mut tx).await?;
    let installation = installation_with(&installations, repository_id)
        .map(|(installation, _)| installation.id)
        .ok_or_else(|| {
            WorkerError::Invalid(format!(
                "{} is not a repository of the tenant's GitHub App installation",
                config.repository
            ))
        })?;
    let pull = worker
        .deps
        .github
        .get_pull_request(installation, &config.repository, number)
        .await?;
    let state = if pull.merged {
        "merged"
    } else if pull.state == "open" {
        "open"
    } else {
        "closed"
    };
    let existing = tx
        .get::<ChangeRequest>(&keys::change_request_key(tenant_id, config_id, number))
        .await?;
    let changes = if state == "open" && pull.base_ref == config.branch {
        let changes = changes(worker, &mut tx, &config, installation, repository_id, &pull).await?;
        if changes.files.is_empty() {
            if let Some(existing) = existing {
                tx.delete(&existing.key())?;
                tx.commit().await?;
                return Ok(1);
            }
            return Ok(0);
        }
        Some(changes)
    } else {
        None
    };
    let Some(mut row) = existing.clone().or_else(|| {
        changes.as_ref().map(|_| ChangeRequest {
            id: Uuid::new_v4(),
            tenant_id,
            config_id,
            pr_number: number,
            title: pull.title.clone(),
            state: state.into(),
            changed_files: json!([]),
            changed_definitions: json!([]),
            validation_status: None,
            validation_messages: json!([]),
            requested_by_member_id: None,
            requested_by_task_id: None,
            created_at: now,
            updated_at: now,
        })
    }) else {
        return Ok(0);
    };
    row.title = pull.title.clone();
    row.state = state.into();
    if let Some(changes) = changes {
        row.changed_files = json!(changes.files);
        if let Some(definitions) = changes.definitions {
            row.changed_definitions = json!(definitions);
        }
        row.validation_status = Some(changes.validation_status.into());
        row.validation_messages = changes.validation_messages;
    }
    if row.requested_by_task_id.is_none() {
        row.requested_by_task_id = opened_from_task(&mut tx, &pull.head_ref).await?;
    }
    if existing.as_ref() == Some(&row) {
        return Ok(0);
    }
    row.updated_at = now;
    tx.put(&row)?;
    let action = match (existing.as_ref().map(|e| e.state.as_str()), state) {
        (None, _) => Some("change_request.open"),
        (Some(before), "merged") if before != "merged" => Some("change_request.merge"),
        (Some(before), "closed") if before != "closed" => Some("change_request.close"),
        (Some(before), "open") if before != "open" => Some("change_request.reopen"),
        _ => None,
    };
    if let Some(action) = action {
        let target_ref = row.id.to_string();
        let mut metadata = Map::new();
        metadata.insert("number".into(), json!(number));
        record_audit(
            &mut tx,
            &AuditEntry {
                at: now,
                actor_kind: ActorKind::System,
                actor_ref: "github",
                action,
                target_kind: "change_request",
                target_ref: &target_ref,
                summary: &format!(
                    "Pull request #{number} of {} is {state}: {}",
                    config.repository, row.title
                ),
                origin: AuditOrigin::Github,
                source_ip: None,
                metadata,
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(1)
}

async fn changes(
    worker: &Worker,
    tx: &mut TenantTx,
    config: &Config,
    installation: i64,
    repository_id: i64,
    pull: &PullRequest,
) -> Result<Changes> {
    let source = &worker.deps.config_source;
    let head = source
        .fetch(
            installation,
            repository_id,
            &config.root_path,
            &pull.head_sha,
        )
        .await?;
    let base = source
        .fetch(
            installation,
            repository_id,
            &config.root_path,
            &pull.base_sha,
        )
        .await?;
    let prefix = root_prefix(&config.root_path);
    let files = changed_files(&base, &head)
        .into_iter()
        .map(|path| format!("{prefix}{path}"))
        .collect();
    let context = validation_context(tx).await?;
    let parsed = parse_repository(&head);
    let errors = match &parsed {
        Ok(repository) => validate(repository, &context).err(),
        Err(errors) => Some(errors.clone()),
    };
    let definitions = match (parse_repository(&base), parsed) {
        (Ok(base), Ok(head)) => Some(changed_definitions(&base, &head)),
        _ => None,
    };
    Ok(Changes {
        files,
        definitions,
        validation_status: if errors.is_some() { "invalid" } else { "valid" },
        validation_messages: serde_json::to_value(errors.map(|e| e.0).unwrap_or_default())?,
    })
}

fn changed_files(base: &ConfigFiles, head: &ConfigFiles) -> Vec<String> {
    base.keys()
        .chain(head.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|path| base.get(*path) != head.get(*path))
        .cloned()
        .collect()
}

fn changed_definitions(base: &ConfigRepository, head: &ConfigRepository) -> Vec<Value> {
    fn changed<T: PartialEq>(
        kind: &str,
        base: &IndexMap<String, T>,
        head: &IndexMap<String, T>,
        found: &mut Vec<Value>,
    ) {
        let names = base.keys().chain(head.keys()).collect::<BTreeSet<_>>();
        for name in names {
            if base.get(name) != head.get(name) {
                found.push(json!({ "kind": kind, "name": name }));
            }
        }
    }
    let mut found = Vec::new();
    changed(
        "trigger",
        &base.config.triggers,
        &head.config.triggers,
        &mut found,
    );
    changed(
        "session",
        &base.config.sessions,
        &head.config.sessions,
        &mut found,
    );
    changed("skill", &base.skills, &head.skills, &mut found);
    changed(
        "project",
        &base.config.projects,
        &head.config.projects,
        &mut found,
    );
    found
}

async fn opened_from_task(tx: &mut TenantTx, head_ref: &str) -> Result<Option<Uuid>> {
    let Some(task_id) = branch_task(head_ref) else {
        return Ok(None);
    };
    Ok(tx
        .get::<Task>(&keys::task_key(tx.tenant_id(), task_id))
        .await?
        .map(|task| task.id))
}
