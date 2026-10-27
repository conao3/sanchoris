use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use sanchoris_config::{
    Checkout, Compute, Config as RepoConfig, ConfigRepository, PlanLimits, Trigger as RepoTrigger,
    TriggerSource, TriggerState, ValidationContext, parse_repository, validate,
};
use sanchoris_core::db::{REF_CONFIG_REVISION, REF_TRIGGER, TenantTx, insert_ref, tenant_plan};
use sanchoris_core::item::{Entity, format_time};
use sanchoris_core::items::{
    Config, ConfigRevision, Credential, Guard, Project, Repository, SessionDef, SkillDef, Trigger,
};
use sanchoris_core::jobs::Job;
use sanchoris_core::keys;
use sanchoris_core::store::{Index, SortKey};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::dispatcher::schedule_trigger;
use crate::error::{Result, WorkerError};
use crate::{Worker, installation_for, installations};

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
    let now = worker.now();
    let mut tx = worker.tx(tenant_id);
    let mut config = tx
        .get::<Config>(&keys::config_key(tenant_id, config_id))
        .await?
        .ok_or_else(|| {
            WorkerError::Invalid(format!(
                "config {config_id} of tenant {tenant_id} does not exist"
            ))
        })?;
    let installation = installation_for(&installations(&mut tx).await?, &config.repository)?;
    let sha = worker
        .deps
        .config_source
        .head_commit(installation, &config.repository, &config.branch)
        .await?;
    let known = tx
        .get::<Guard>(&keys::revision_sha_key(tenant_id, config_id, &sha))
        .await?;
    if known.is_none() {
        let files = worker
            .deps
            .config_source
            .fetch(installation, &config.repository, &config.root_path, &sha)
            .await?;
        let context = validation_context(&mut tx).await?;
        store_revision(&mut tx, &mut config, &sha, &files, &context, now).await?;
    }
    config.synced_at = Some(now);
    config.updated_at = now;
    tx.put(&config)?;
    worker.faults.check("config_sync.before_commit")?;
    tx.commit().await?;
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

async fn store_revision(
    tx: &mut TenantTx,
    config: &mut Config,
    sha: &str,
    files: &sanchoris_config::ConfigFiles,
    context: &ValidationContext,
    now: DateTime<Utc>,
) -> Result<()> {
    let tenant_id = tx.tenant_id();
    let parsed = parse_repository(files);
    let (repository, errors) = match parsed {
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
    if let (Some(repository), "valid") = (repository, status) {
        activate(tx, config.id, revision.id, &repository, now).await?;
        config.active_revision_id = Some(revision.id);
    }
    Ok(())
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

async fn upsert<E: Entity>(tx: &mut TenantTx, entity: &E) -> Result<()> {
    tx.get::<E>(&entity.key()).await?;
    tx.put(entity)?;
    Ok(())
}

async fn activate(
    tx: &mut TenantTx,
    config_id: Uuid,
    revision_id: Uuid,
    repository: &ConfigRepository,
    now: DateTime<Utc>,
) -> Result<()> {
    let tenant_id = tx.tenant_id();
    let pk = keys::config_pk(tenant_id, config_id);
    let config = &repository.config;

    for project in tx.query::<Project>(&pk, "PROJECT#").await? {
        if !config.projects.contains_key(&project.name) {
            tx.delete(&project.key())?;
        }
    }
    for repo in tx.query::<Repository>(&pk, "PROJECT#").await? {
        let keep = config
            .projects
            .get(&repo.project)
            .is_some_and(|p| p.repos.contains_key(&repo.name));
        if !keep {
            tx.delete(&repo.key())?;
        }
    }
    for (name, project) in &config.projects {
        upsert(
            tx,
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
            upsert(
                tx,
                &Repository {
                    id: keys::config_entity_id(
                        config_id,
                        "repository",
                        &format!("{name}/{repo_name}"),
                    ),
                    tenant_id,
                    config_id,
                    project: name.clone(),
                    name: repo_name.clone(),
                    url: repo.repo.clone(),
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
                    prebuild_status: None,
                    prebuild_at: None,
                },
            )
            .await?;
        }
    }

    for session in tx.query::<SessionDef>(&pk, "SESSION#").await? {
        if !config.sessions.contains_key(&session.name) {
            tx.delete(&session.key())?;
        }
    }
    for (name, session) in &config.sessions {
        upsert(
            tx,
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

    for skill in tx.query::<SkillDef>(&pk, "SKILL#").await? {
        if !repository.skills.contains_key(&skill.name) {
            tx.delete(&skill.key())?;
        }
    }
    for (name, skill) in &repository.skills {
        upsert(
            tx,
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

    for mut previous in tx.query::<Trigger>(&pk, "TRIGGER#").await? {
        if !config.triggers.contains_key(&previous.name) && previous.removed_at.is_none() {
            previous.removed_at = Some(now);
            schedule_trigger(tx, &mut previous, now)?;
            tx.put(&previous)?;
        }
    }
    for (name, trigger) in &config.triggers {
        let (event_pattern, schedule_expression) = match &trigger.source {
            TriggerSource::EventPattern(pattern) => {
                (Some(rewrite_event_pattern(config, trigger, pattern)), None)
            }
            TriggerSource::ScheduleExpression(expression) => (None, Some(expression.clone())),
        };
        let key = keys::trigger_key(tenant_id, config_id, name);
        let existing = tx.get::<Trigger>(&key).await?;
        let state = if trigger.state == TriggerState::Enabled {
            "ENABLED"
        } else {
            "DISABLED"
        };
        let id = keys::trigger_id(config_id, name);
        let mut next = Trigger {
            id,
            tenant_id,
            config_id,
            config_revision_id: revision_id,
            name: name.clone(),
            state: state.to_string(),
            event_pattern,
            schedule_expression,
            input_transformer: trigger
                .input_transformer
                .as_ref()
                .map(serde_json::to_value)
                .transpose()?,
            start: serde_json::to_value(&trigger.start)?,
            removed_at: None,
            next_run_at: existing.as_ref().and_then(|t| t.next_run_at),
            schedule_message_id: existing.as_ref().and_then(|t| t.schedule_message_id),
        };
        let reschedule = match &existing {
            None => true,
            Some(previous) => {
                previous.is_active() != next.is_active()
                    || previous.schedule_expression != next.schedule_expression
                    || (next.is_active()
                        && next.schedule_expression.is_some()
                        && next.schedule_message_id.is_none())
            }
        };
        if reschedule {
            schedule_trigger(tx, &mut next, now)?;
        }
        match existing {
            Some(_) => tx.put(&next)?,
            None => {
                tx.insert(&next)?;
                insert_ref(tx, id, REF_TRIGGER, key, Some(config_id))?;
            }
        }
    }
    Ok(())
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
