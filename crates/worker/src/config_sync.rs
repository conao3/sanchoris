use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use sanchoris_config::{
    Checkout, Compute, Config as RepoConfig, ConfigRepository, PlanLimits, Trigger as RepoTrigger,
    TriggerSource, TriggerState, ValidationContext, parse_repository, validate,
};
use sanchoris_core::audit::{AuditEntry, record_audit};
use sanchoris_core::db::{
    REF_CONFIG_REVISION, REF_TRIGGER, Store, TenantTx, insert_ref, locate, tenant_plan,
};
use sanchoris_core::item::{Entity, Key, format_time};
use sanchoris_core::items::{
    CONFIG_DISABLED, Config, ConfigRevision, Credential, GitHubRepository, Guard, NextTrigger,
    Project, Repository, SessionDef, SkillDef, Task, Trigger, TriggerSpec,
};
use sanchoris_core::jobs::Job;
use sanchoris_core::keys;
use sanchoris_core::model::{ActorKind, AuditOrigin};
use sanchoris_core::store::{Index, MAX_TRANSACTION_ITEMS, SortKey};
use sanchoris_core::tasks::abort_task;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::dispatcher::schedule_trigger;
use crate::error::{Result, WorkerError};
use crate::ports::{ConfigSource, Installation, installation_with};
use crate::{Worker, installation_for, installations};

const CONFLICT_ATTEMPTS: u32 = 8;

pub async fn sweep(worker: &Worker) -> Result<usize> {
    let cutoff = worker.now() - worker.settings.config_poll_interval;
    let due = worker
        .store
        .query_index::<Config>(
            Index::Gsi2,
            keys::SWEEP_CONFIGS,
            SortKey::AtMost(format!("{}#~", format_time(cutoff))),
        )
        .await?;
    for config in &due {
        let mut tx = worker.tx(config.tenant_id);
        tx.emit(Job::ConfigSync {
            tenant_id: config.tenant_id,
            config_id: config.id,
        });
        tx.commit().await?;
    }
    Ok(due.len())
}

pub async fn sync_config(worker: &Worker, tenant_id: Uuid, config_id: Uuid) -> Result<()> {
    let config = worker
        .store
        .get::<Config>(&keys::config_key(tenant_id, config_id))
        .await?
        .ok_or_else(|| {
            WorkerError::Invalid(format!(
                "config {config_id} of tenant {tenant_id} does not exist"
            ))
        })?;
    if !config.is_active() {
        return Ok(());
    }
    match sync_active(worker, tenant_id, config_id).await {
        Err(error @ (WorkerError::GitHub(_) | WorkerError::Invalid(_))) => {
            record_sync_error(worker, tenant_id, config_id, &error.to_string()).await
        }
        other => other,
    }
}

async fn record_sync_error(
    worker: &Worker,
    tenant_id: Uuid,
    config_id: Uuid,
    message: &str,
) -> Result<()> {
    let mut tx = worker.tx(tenant_id);
    let Some(mut config) = tx
        .get::<Config>(&keys::config_key(tenant_id, config_id))
        .await?
    else {
        return Ok(());
    };
    config.sync_error = Some(message.to_string());
    config.updated_at = worker.now();
    tx.put(&config)?;
    tx.commit().await?;
    Ok(())
}

pub async fn resolve_repository(
    source: &dyn ConfigSource,
    installations: &[Installation],
    name: &str,
) -> Result<GitHubRepository> {
    if let Some(found) = installations.iter().find_map(|installation| {
        installation
            .repositories
            .iter()
            .find(|r| r.full_name.eq_ignore_ascii_case(name))
    }) {
        return Ok(found.clone());
    }
    let missing = || {
        WorkerError::Invalid(format!(
            "{name} is not a repository of the tenant's GitHub App installation"
        ))
    };
    let installation = installation_for(installations, name).map_err(|_| missing())?;
    source
        .repository(installation, name)
        .await?
        .filter(|found| {
            installations
                .iter()
                .any(|i| i.id == installation && i.repository(found.id).is_some())
        })
        .ok_or_else(missing)
}

pub fn named_repositories(config: &RepoConfig) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for project in config.projects.values() {
        names.extend(
            project
                .repos
                .values()
                .filter_map(|repo| repo.full_name())
                .map(str::to_string),
        );
    }
    for session in config.sessions.values() {
        if let Checkout::Repos(repos) = &session.checkout {
            names.extend(repos.iter().cloned());
        }
        names.extend(
            session
                .github
                .repos
                .iter()
                .filter(|repo| !repo.contains('*'))
                .cloned(),
        );
    }
    for trigger in config.triggers.values() {
        if let TriggerSource::EventPattern(pattern) = &trigger.source
            && let Some(Value::Array(items)) =
                rewrite_event_pattern(config, trigger, pattern).pointer("/detail/repository")
        {
            names.extend(
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|name| name.contains('/') && !name.contains('*'))
                    .map(str::to_string),
            );
        }
    }
    names
}

async fn resolve_repositories(
    worker: &Worker,
    installations: &[Installation],
    config: &RepoConfig,
) -> Result<BTreeMap<String, GitHubRepository>> {
    let mut resolved = BTreeMap::new();
    for name in named_repositories(config) {
        let repository =
            resolve_repository(worker.deps.config_source.as_ref(), installations, &name).await?;
        resolved.insert(name, repository);
    }
    Ok(resolved)
}

async fn sync_active(worker: &Worker, tenant_id: Uuid, config_id: Uuid) -> Result<()> {
    let now = worker.now();
    let mut tx = worker.tx(tenant_id);
    let mut config = tx
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
    let source = &worker.deps.config_source;
    config.repository = source
        .repository_by_id(installation, repository_id)
        .await?
        .full_name;
    let sha = source
        .head_commit(installation, repository_id, &config.branch)
        .await?;
    let known = tx
        .get::<Guard>(&keys::revision_sha_key(tenant_id, config_id, &sha))
        .await?;
    let revision = match known {
        Some(guard) => {
            let location = locate(&mut tx, guard.target, REF_CONFIG_REVISION)
                .await?
                .ok_or_else(|| {
                    WorkerError::Invalid(format!("config revision {} has no ref", guard.target))
                })?;
            tx.require::<ConfigRevision>(&location.target).await?
        }
        None => {
            let files = source
                .fetch(installation, repository_id, &config.root_path, &sha)
                .await?;
            let context = validation_context(&mut tx).await?;
            store_revision(&mut tx, &mut config, &sha, &files, &context, now)?
        }
    };
    config.synced_at = Some(now);
    config.sync_error = None;
    config.updated_at = now;
    tx.put(&config)?;
    worker.faults.check("config_sync.before_commit")?;
    tx.commit().await?;
    if revision.validation_status != "valid" {
        return Ok(());
    }
    if config.active_revision_id == Some(revision.id) {
        return settle(worker, tenant_id, config_id, now).await;
    }
    let parsed = revision.parsed.clone().ok_or_else(|| {
        WorkerError::Invalid(format!(
            "valid revision {} has no parsed config",
            revision.id
        ))
    })?;
    let repository: ConfigRepository = serde_json::from_value(parsed)?;
    let repositories = resolve_repositories(worker, &installations, &repository.config).await?;
    let author = source
        .commit_author(installation, repository_id, &sha)
        .await?;
    activate(
        worker,
        tenant_id,
        config_id,
        revision.id,
        &repository,
        &repositories,
    )
    .await?;
    let mut tx = worker.tx(tenant_id);
    let mut config = tx
        .require::<Config>(&keys::config_key(tenant_id, config_id))
        .await?;
    config.active_revision_id = Some(revision.id);
    config.repositories = repositories;
    config.updated_at = now;
    tx.put(&config)?;
    let config_ref = config_id.to_string();
    let mut metadata = Map::new();
    metadata.insert("revision".into(), json!(revision.number));
    metadata.insert("commit_sha".into(), json!(sha));
    metadata.insert("commit_author".into(), json!(author));
    record_audit(
        &mut tx,
        &AuditEntry {
            at: now,
            actor_kind: ActorKind::System,
            actor_ref: "config-sync",
            action: "config.activate",
            target_kind: "config",
            target_ref: &config_ref,
            summary: &format!(
                "Activated revision {} of {} at {sha} by {author}",
                revision.number, config.repository
            ),
            origin: AuditOrigin::Github,
            source_ip: None,
            metadata,
        },
    )
    .await?;
    tx.commit().await?;
    settle(worker, tenant_id, config_id, now).await
}

pub async fn reactivate(
    worker: &Worker,
    tenant_id: Uuid,
    config_id: Uuid,
    installations: &[Installation],
) -> Result<()> {
    let mut tx = worker.tx(tenant_id);
    let config = tx
        .require::<Config>(&keys::config_key(tenant_id, config_id))
        .await?;
    let Some(revision_id) = config.active_revision_id else {
        return Ok(());
    };
    let location = locate(&mut tx, revision_id, REF_CONFIG_REVISION)
        .await?
        .ok_or_else(|| WorkerError::Invalid(format!("config revision {revision_id} has no ref")))?;
    let revision = tx.require::<ConfigRevision>(&location.target).await?;
    let parsed = revision.parsed.clone().ok_or_else(|| {
        WorkerError::Invalid(format!(
            "active revision {revision_id} has no parsed config"
        ))
    })?;
    let repository: ConfigRepository = serde_json::from_value(parsed)?;
    let repositories = resolve_repositories(worker, installations, &repository.config).await?;
    activate(
        worker,
        tenant_id,
        config_id,
        revision_id,
        &repository,
        &repositories,
    )
    .await?;
    let mut tx = worker.tx(tenant_id);
    let mut config = tx
        .require::<Config>(&keys::config_key(tenant_id, config_id))
        .await?;
    config.repositories = repositories;
    config.updated_at = worker.now();
    tx.put(&config)?;
    tx.commit().await?;
    settle(worker, tenant_id, config_id, worker.now()).await
}

async fn retrying<T, F, Fut>(mut operation: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut attempt = 0;
    loop {
        attempt += 1;
        match operation().await {
            Err(error) if error.is_conflict() && attempt < CONFLICT_ATTEMPTS => {}
            other => return other,
        }
    }
}

pub async fn disable_config(
    store: &Store,
    tenant_id: Uuid,
    config_id: Uuid,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    retrying(|| async {
        let mut tx = store.tx(tenant_id);
        let Some(mut config) = tx
            .get::<Config>(&keys::config_key(tenant_id, config_id))
            .await?
        else {
            return Ok(());
        };
        if !config.is_active() {
            return Ok(());
        }
        config.status = CONFIG_DISABLED.into();
        config.sync_error = Some(reason.to_string());
        config.repositories = BTreeMap::new();
        config.updated_at = now;
        tx.put(&config)?;
        if let Some(repository_id) = config.repository_id {
            let guard = keys::config_repository_key(tenant_id, repository_id, &config.root_path);
            if tx.get::<Guard>(&guard).await?.is_some() {
                tx.delete(&guard)?;
            }
        }
        let config_ref = config_id.to_string();
        record_audit(
            &mut tx,
            &AuditEntry {
                at: now,
                actor_kind: ActorKind::System,
                actor_ref: "github",
                action: "config.disable",
                target_kind: "config",
                target_ref: &config_ref,
                summary: &format!("Disabled the config {}: {reason}", config.repository),
                origin: AuditOrigin::Github,
                source_ip: None,
                metadata: Map::new(),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
    })
    .await?;
    retire_definitions(store, tenant_id, config_id, reason, now).await
}

async fn retire_definitions(
    store: &Store,
    tenant_id: Uuid,
    config_id: Uuid,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    let pk = keys::config_pk(tenant_id, config_id);
    let triggers = store.query::<Trigger>(&pk, "TRIGGER#").await?;
    for listed in triggers.iter().filter(|t| t.removed_at.is_none()) {
        retrying(|| async {
            let mut tx = store.tx(tenant_id);
            let Some(mut trigger) = tx.get::<Trigger>(&listed.key()).await? else {
                return Ok(());
            };
            if trigger.removed_at.is_some() {
                return Ok(());
            }
            trigger.removed_at = Some(now);
            schedule_trigger(&mut tx, &mut trigger, now)?;
            tx.put(&trigger)?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
    }
    let definitions = all_definitions(store, &pk).await?;
    for chunk in definitions.chunks(BATCH_WRITES) {
        let mut tx = store.tx(tenant_id);
        for key in chunk {
            tx.delete(key)?;
        }
        tx.commit().await?;
    }
    let trigger_ids = triggers.iter().map(|t| t.id).collect::<BTreeSet<_>>();
    let unfinished = store
        .query_index::<Task>(Index::Gsi1, &keys::tasks_index(tenant_id), SortKey::All)
        .await?
        .into_iter()
        .filter(|task| trigger_ids.contains(&task.trigger_id) && !task.status.is_terminal());
    for task in unfinished {
        retrying(|| async {
            let mut tx = store.tx(tenant_id);
            if abort_task(&mut tx, task.id, reason, now).await? {
                tx.commit().await?;
            }
            Ok(())
        })
        .await?;
    }
    Ok(())
}

pub async fn rename_repository(
    store: &Store,
    tenant_id: Uuid,
    repository: &GitHubRepository,
    now: DateTime<Utc>,
) -> Result<()> {
    let configs = store
        .query::<Config>(&keys::tenant_pk(tenant_id), "CONFIG#")
        .await?;
    for listed in &configs {
        retrying(|| async {
            let mut tx = store.tx(tenant_id);
            let Some(mut config) = tx.get::<Config>(&listed.key()).await? else {
                return Ok(());
            };
            let mut changed = false;
            if config.repository_id == Some(repository.id)
                && config.repository != repository.full_name
            {
                let previous =
                    std::mem::replace(&mut config.repository, repository.full_name.clone());
                let config_ref = config.id.to_string();
                record_audit(
                    &mut tx,
                    &AuditEntry {
                        at: now,
                        actor_kind: ActorKind::System,
                        actor_ref: "github",
                        action: "config.rename",
                        target_kind: "config",
                        target_ref: &config_ref,
                        summary: &format!(
                            "The config repository {previous} is now {}",
                            repository.full_name
                        ),
                        origin: AuditOrigin::Github,
                        source_ip: None,
                        metadata: Map::new(),
                    },
                )
                .await?;
                changed = true;
            }
            for cached in config.repositories.values_mut() {
                if cached.id == repository.id && cached.full_name != repository.full_name {
                    cached.full_name = repository.full_name.clone();
                    changed = true;
                }
            }
            if changed {
                config.updated_at = now;
                tx.put(&config)?;
                tx.commit().await?;
            }
            Ok(())
        })
        .await?;
        let pk = keys::config_pk(tenant_id, listed.id);
        for row in store
            .query::<Repository>(&pk, keys::DEFINITIONS_PREFIX)
            .await?
        {
            if row.github_repository_id != repository.id || row.full_name == repository.full_name {
                continue;
            }
            retrying(|| async {
                let mut tx = store.tx(tenant_id);
                let Some(mut row) = tx.get::<Repository>(&row.key()).await? else {
                    return Ok(());
                };
                row.full_name = repository.full_name.clone();
                tx.put(&row)?;
                tx.commit().await?;
                Ok(())
            })
            .await?;
        }
    }
    Ok(())
}

pub async fn validation_context(tx: &mut TenantTx) -> Result<ValidationContext> {
    let credentials = tx
        .query::<Credential>(&keys::tenant_pk(tx.tenant_id()), "CRED#")
        .await?
        .into_iter()
        .map(|c| c.name)
        .collect::<BTreeSet<_>>();
    let (_, plan) = tenant_plan(tx).await?;
    let unsigned = |value: i64| u64::try_from(value).expect("plan limits are non-negative");
    let computes = plan
        .computes
        .iter()
        .map(|c| match c.as_str() {
            "standard" => Compute::Standard,
            "docker" => Compute::Docker,
            other => panic!("plans.computes has unknown compute {other}"),
        })
        .collect();
    Ok(ValidationContext {
        credentials,
        plan: PlanLimits {
            max_turns: unsigned(plan.max_task_turns),
            max_cost_usd: plan.max_task_cost_usd,
            max_timeout_seconds: unsigned(plan.max_task_timeout_seconds),
            max_idle_seconds: unsigned(plan.max_idle_seconds),
            max_children_depth: unsigned(plan.max_children_depth),
            max_children_count: unsigned(plan.max_children_count),
            computes,
        },
    })
}

fn store_revision(
    tx: &mut TenantTx,
    config: &mut Config,
    sha: &str,
    files: &sanchoris_config::ConfigFiles,
    context: &ValidationContext,
    now: DateTime<Utc>,
) -> Result<ConfigRevision> {
    let tenant_id = tx.tenant_id();
    let (repository, errors) = match parse_repository(files) {
        Ok(repository) => {
            let errors = validate(&repository, context).err();
            (Some(repository), errors)
        }
        Err(errors) => (None, Some(errors)),
    };
    let status = if errors.is_some() { "invalid" } else { "valid" };
    let parsed_json = repository.as_ref().map(serde_json::to_value).transpose()?;
    let messages = serde_json::to_value(errors.map(|e| e.0).unwrap_or_default())?;
    config.revisions += 1;
    let revision = ConfigRevision {
        id: Uuid::new_v4(),
        tenant_id,
        config_id: config.id,
        number: config.revisions,
        commit_sha: sha.to_string(),
        parsed: parsed_json,
        validation_status: status.to_string(),
        validation_messages: messages,
        created_at: now,
    };
    tx.insert(&revision)?;
    tx.insert(&Guard::new(
        keys::revision_sha_key(tenant_id, config.id, sha),
        revision.id,
    ))?;
    insert_ref(
        tx,
        revision.id,
        REF_CONFIG_REVISION,
        revision.key(),
        Some(config.id),
    )?;
    Ok(revision)
}

pub fn resolve_repository_key(
    config: &RepoConfig,
    trigger: &RepoTrigger,
    key: &str,
) -> Option<String> {
    if key.contains('/') {
        return Some(key.to_string());
    }
    let session = config.sessions.get(&trigger.start.session)?;
    let Checkout::Project(project) = &session.checkout else {
        return None;
    };
    config
        .projects
        .get(project)?
        .repos
        .get(key)?
        .full_name()
        .map(str::to_string)
}

pub fn rewrite_event_pattern(config: &RepoConfig, trigger: &RepoTrigger, pattern: &Value) -> Value {
    let mut pattern = pattern.clone();
    if let Some(Value::Array(items)) = pattern.pointer_mut("/detail/repository") {
        for item in items.iter_mut() {
            if let Some(resolved) = item
                .as_str()
                .and_then(|key| resolve_repository_key(config, trigger, key))
            {
                *item = Value::String(resolved);
            }
        }
    }
    pattern
}

const BATCH_WRITES: usize = MAX_TRANSACTION_ITEMS - 10;
const TRIGGER_WRITES: usize = 4;

struct Batches<'a> {
    worker: &'a Worker,
    tenant_id: Uuid,
    tx: TenantTx,
    writes: usize,
}

impl<'a> Batches<'a> {
    fn new(worker: &'a Worker, tenant_id: Uuid) -> Self {
        Self {
            worker,
            tenant_id,
            tx: worker.tx(tenant_id),
            writes: 0,
        }
    }

    async fn room(&mut self, writes: usize) -> Result<&mut TenantTx> {
        if self.writes + writes > BATCH_WRITES {
            let full = std::mem::replace(&mut self.tx, self.worker.tx(self.tenant_id));
            full.commit().await?;
            self.worker.faults.check("config_sync.between_batches")?;
            self.writes = 0;
        }
        self.writes += writes;
        Ok(&mut self.tx)
    }

    async fn finish(self) -> Result<()> {
        self.tx.commit().await?;
        Ok(())
    }
}

async fn upsert<E: Entity>(batches: &mut Batches<'_>, entity: &E) -> Result<()> {
    let tx = batches.room(1).await?;
    tx.get_item(&entity.key()).await?;
    tx.put(entity)?;
    Ok(())
}

async fn delete(batches: &mut Batches<'_>, key: &Key) -> Result<()> {
    batches.room(1).await?.delete(key)?;
    Ok(())
}

async fn activate(
    worker: &Worker,
    tenant_id: Uuid,
    config_id: Uuid,
    revision_id: Uuid,
    repository: &ConfigRepository,
    repositories: &BTreeMap<String, GitHubRepository>,
) -> Result<()> {
    let pk = keys::config_pk(tenant_id, config_id);
    let config = &repository.config;
    let previous_revision = worker
        .store
        .get::<Config>(&keys::config_key(tenant_id, config_id))
        .await?
        .and_then(|c| c.active_revision_id);
    let mut batches = Batches::new(worker, tenant_id);

    for (name, project) in &config.projects {
        upsert(
            &mut batches,
            &Project {
                id: keys::config_entity_id(config_id, "project", name),
                tenant_id,
                config_id,
                config_revision_id: revision_id,
                name: name.clone(),
                system_prompt: project.system_prompt.clone(),
                source_path: sanchoris_config::CONFIG_FILE.to_string(),
            },
        )
        .await?;
        for (repo_name, repo) in &project.repos {
            let resolved = repo
                .full_name()
                .and_then(|name| repositories.get(name))
                .ok_or_else(|| {
                    WorkerError::Invalid(format!(
                        "project {name} repository {repo_name} ({}) is not resolved",
                        repo.repo
                    ))
                })?;
            let previous = match previous_revision {
                Some(previous) => {
                    worker
                        .store
                        .get::<Repository>(&keys::repository_key(
                            tenant_id, config_id, previous, name, repo_name,
                        ))
                        .await?
                }
                None => None,
            };
            upsert(
                &mut batches,
                &Repository {
                    id: keys::config_entity_id(
                        config_id,
                        "repository",
                        &format!("{name}/{repo_name}"),
                    ),
                    tenant_id,
                    config_id,
                    config_revision_id: revision_id,
                    project: name.clone(),
                    name: repo_name.clone(),
                    url: repo.repo.clone(),
                    github_repository_id: resolved.id,
                    full_name: resolved.full_name.clone(),
                    warm_hook: repo
                        .warm_hook
                        .as_ref()
                        .map(serde_json::to_value)
                        .transpose()?,
                    start_hook: repo
                        .start_hook
                        .as_ref()
                        .map(serde_json::to_value)
                        .transpose()?,
                    prebuild_status: previous.as_ref().and_then(|r| r.prebuild_status.clone()),
                    prebuild_at: previous.as_ref().and_then(|r| r.prebuild_at),
                },
            )
            .await?;
        }
    }

    for (name, session) in &config.sessions {
        upsert(
            &mut batches,
            &SessionDef {
                id: keys::config_entity_id(config_id, "session", name),
                tenant_id,
                config_id,
                config_revision_id: revision_id,
                name: name.clone(),
                definition: serde_json::to_value(session)?,
            },
        )
        .await?;
    }

    for (name, skill) in &repository.skills {
        upsert(
            &mut batches,
            &SkillDef {
                id: keys::config_entity_id(config_id, "skill", name),
                tenant_id,
                config_id,
                config_revision_id: revision_id,
                name: name.clone(),
                kind: if skill.is_reliable() {
                    "reliable"
                } else {
                    "regular"
                }
                .to_string(),
                description: skill.description.clone(),
                skill_md: skill.skill_md.clone(),
                skill_yaml: skill.skill_yaml.clone(),
                definition: skill
                    .definition
                    .as_ref()
                    .map(serde_json::to_value)
                    .transpose()?,
                layout: skill
                    .definition
                    .as_ref()
                    .and_then(|d| d.layout.as_ref())
                    .map(serde_json::to_value)
                    .transpose()?,
            },
        )
        .await?;
    }

    for (name, trigger) in &config.triggers {
        let tx = batches.room(TRIGGER_WRITES).await?;
        let (event_pattern, schedule_expression) = match &trigger.source {
            TriggerSource::EventPattern(pattern) => {
                (Some(rewrite_event_pattern(config, trigger, pattern)), None)
            }
            TriggerSource::ScheduleExpression(expression) => (None, Some(expression.clone())),
        };
        let spec = TriggerSpec {
            state: if trigger.state == TriggerState::Enabled {
                "ENABLED"
            } else {
                "DISABLED"
            }
            .to_string(),
            event_pattern,
            schedule_expression,
            input_transformer: trigger
                .input_transformer
                .as_ref()
                .map(serde_json::to_value)
                .transpose()?,
            start: serde_json::to_value(&trigger.start)?,
        };
        let key = keys::trigger_key(tenant_id, config_id, name);
        match tx.get::<Trigger>(&key).await? {
            Some(mut existing) => {
                existing.next = Some(NextTrigger {
                    config_revision_id: revision_id,
                    definition: Some(spec),
                });
                tx.put(&existing)?;
            }
            None => {
                let id = keys::trigger_id(config_id, name);
                tx.insert(&Trigger {
                    id,
                    tenant_id,
                    config_id,
                    config_revision_id: revision_id,
                    name: name.clone(),
                    state: spec.state,
                    event_pattern: spec.event_pattern,
                    schedule_expression: spec.schedule_expression,
                    input_transformer: spec.input_transformer,
                    start: spec.start,
                    removed_at: None,
                    next_run_at: None,
                    schedule_message_id: None,
                    next: None,
                })?;
                insert_ref(tx, id, REF_TRIGGER, key, Some(config_id))?;
            }
        }
    }

    for listed in worker.store.query::<Trigger>(&pk, "TRIGGER#").await? {
        if config.triggers.contains_key(&listed.name) {
            continue;
        }
        let tx = batches.room(1).await?;
        let Some(mut previous) = tx.get::<Trigger>(&listed.key()).await? else {
            continue;
        };
        previous.next = Some(NextTrigger {
            config_revision_id: revision_id,
            definition: None,
        });
        tx.put(&previous)?;
    }
    batches.finish().await
}

pub async fn settle(
    worker: &Worker,
    tenant_id: Uuid,
    config_id: Uuid,
    now: DateTime<Utc>,
) -> Result<()> {
    let Some(config) = worker
        .store
        .get::<Config>(&keys::config_key(tenant_id, config_id))
        .await?
    else {
        return Ok(());
    };
    let Some(active) = config.active_revision_id.filter(|_| config.is_active()) else {
        return Ok(());
    };
    let pk = keys::config_pk(tenant_id, config_id);
    let store = &worker.store;
    let mut batches = Batches::new(worker, tenant_id);
    for listed in store.query::<Trigger>(&pk, "TRIGGER#").await? {
        let settled = match listed.in_revision(Some(active)) {
            Some(view) => view,
            None if listed.removed_at.is_some() && listed.next.is_none() => continue,
            None => Trigger {
                removed_at: listed.removed_at.or(Some(now)),
                next: None,
                ..listed.clone()
            },
        };
        let reschedule = listed.is_active() != settled.is_active()
            || listed.schedule_expression != settled.schedule_expression
            || (settled.is_active()
                && settled.schedule_expression.is_some()
                && settled.schedule_message_id.is_none());
        if !reschedule && settled == listed {
            continue;
        }
        let tx = batches.room(TRIGGER_WRITES).await?;
        let Some(current) = tx.get::<Trigger>(&listed.key()).await? else {
            continue;
        };
        if current != listed {
            continue;
        }
        let mut settled = settled;
        if reschedule {
            schedule_trigger(tx, &mut settled, now)?;
        }
        tx.put(&settled)?;
    }
    for key in stale_definitions(store, &pk, active).await? {
        delete(&mut batches, &key).await?;
    }
    batches.finish().await
}

async fn definition_keys<E: Entity>(
    store: &Store,
    pk: &str,
    keep: impl Fn(&E) -> bool,
) -> Result<Vec<Key>> {
    Ok(store
        .query::<E>(pk, keys::DEFINITIONS_PREFIX)
        .await?
        .iter()
        .filter(|item| !keep(item))
        .map(Entity::key)
        .collect())
}

async fn stale_definitions(store: &Store, pk: &str, active: Uuid) -> Result<Vec<Key>> {
    let mut stale =
        definition_keys::<Repository>(store, pk, |r| r.config_revision_id == active).await?;
    stale.extend(definition_keys::<Project>(store, pk, |p| p.config_revision_id == active).await?);
    stale.extend(
        definition_keys::<SessionDef>(store, pk, |s| s.config_revision_id == active).await?,
    );
    stale.extend(definition_keys::<SkillDef>(store, pk, |s| s.config_revision_id == active).await?);
    Ok(stale)
}

async fn all_definitions(store: &Store, pk: &str) -> Result<Vec<Key>> {
    let mut all = definition_keys::<Repository>(store, pk, |_| false).await?;
    all.extend(definition_keys::<Project>(store, pk, |_| false).await?);
    all.extend(definition_keys::<SessionDef>(store, pk, |_| false).await?);
    all.extend(definition_keys::<SkillDef>(store, pk, |_| false).await?);
    Ok(all)
}

pub fn trigger_definition(
    state: &str,
    event_pattern: Option<&Value>,
    schedule_expression: Option<&str>,
    input_transformer: Option<&Value>,
    start: &Value,
) -> Value {
    let mut definition = json!({ "State": state, "Start": start });
    if let Some(pattern) = event_pattern {
        definition["EventPattern"] = pattern.clone();
    }
    if let Some(expression) = schedule_expression {
        definition["ScheduleExpression"] = json!(expression);
    }
    if let Some(transformer) = input_transformer {
        definition["InputTransformer"] = transformer.clone();
    }
    definition
}
