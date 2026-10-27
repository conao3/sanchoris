use std::collections::{BTreeMap, BTreeSet};

use sanchoris_config::{
    Checkout, Compute, Config, ConfigRepository, PlanLimits, Trigger, TriggerSource, TriggerState,
    ValidationContext, parse_repository, validate,
};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::error::Result;
use crate::{Worker, adopt, installation_for, installations};
use sanchoris_core::db::TenantTx;

pub const DUE_TRIGGER_SCHEDULE: &str = "trigger_schedule";

pub async fn tick(worker: &Worker) -> Result<usize> {
    let mut processed = 0;
    while processed < worker.settings.batch {
        if !sync_one(worker).await? {
            break;
        }
        processed += 1;
    }
    Ok(usize::try_from(processed).expect("batch fits in usize"))
}

async fn sync_one(worker: &Worker) -> Result<bool> {
    let mut tx = worker.begin().await?;
    let Some(row) = sqlx::query(
        "SELECT id, tenant_id, repository, root_path, branch FROM sanchoris.configs \
         WHERE synced_at IS NULL OR synced_at <= $1 \
         ORDER BY synced_at NULLS FIRST, id LIMIT 1 FOR UPDATE SKIP LOCKED",
    )
    .bind(worker.now() - worker.settings.config_poll_interval)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(false);
    };
    sync_locked(worker, tx, &row).await?;
    Ok(true)
}

pub async fn sync_config(worker: &Worker, tenant_id: Uuid, config_id: Uuid) -> Result<()> {
    let mut tx = worker.begin().await?;
    let row = sqlx::query(
        "SELECT id, tenant_id, repository, root_path, branch FROM sanchoris.configs \
         WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
    )
    .bind(config_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        crate::error::WorkerError::Invalid(format!(
            "config {config_id} of tenant {tenant_id} does not exist"
        ))
    })?;
    sync_locked(worker, tx, &row).await
}

async fn sync_locked(
    worker: &Worker,
    tx: sqlx::Transaction<'static, sqlx::Postgres>,
    row: &sqlx::postgres::PgRow,
) -> Result<()> {
    let now = worker.now();
    let config_id: Uuid = row.try_get("id")?;
    let repository: String = row.try_get("repository")?;
    let root_path: String = row.try_get("root_path")?;
    let branch: String = row.try_get("branch")?;
    let mut tx = adopt(tx, row.try_get("tenant_id")?).await?;
    let installation = installation_for(&installations(&mut tx).await?, &repository)?;
    let sha = worker
        .deps
        .config_source
        .head_commit(installation, &repository, &branch)
        .await?;
    let known = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM sanchoris.config_revisions WHERE config_id = $1 AND commit_sha = $2",
    )
    .bind(config_id)
    .bind(&sha)
    .fetch_optional(tx.conn())
    .await?;
    if known.is_none() {
        let files = worker
            .deps
            .config_source
            .fetch(installation, &repository, &root_path, &sha)
            .await?;
        let context = validation_context(&mut tx).await?;
        store_revision(&mut tx, config_id, &sha, &files, &context).await?;
    }
    sqlx::query("UPDATE sanchoris.configs SET synced_at = $2, updated_at = now() WHERE id = $1")
        .bind(config_id)
        .bind(now)
        .execute(tx.conn())
        .await?;
    worker.faults.check("config_sync.before_commit")?;
    tx.commit().await?;
    Ok(())
}

pub async fn validation_context(tx: &mut TenantTx) -> Result<ValidationContext> {
    let tenant_id = tx.tenant_id();
    let credentials = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sanchoris.credentials WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_all(tx.conn())
    .await?
    .into_iter()
    .collect::<BTreeSet<_>>();
    let plan = sqlx::query(
        "SELECT p.max_task_turns, p.max_task_cost_usd::float8 AS max_task_cost_usd, p.max_task_timeout_seconds, \
                p.max_idle_seconds, p.max_children_depth, p.max_children_count, p.computes \
         FROM sanchoris.tenants t JOIN sanchoris.plans p ON p.id = t.plan_id WHERE t.id = $1",
    )
    .bind(tenant_id)
    .fetch_one(tx.conn())
    .await?;
    let unsigned = |value: i64| u64::try_from(value).expect("plan limits are non-negative");
    let computes = plan
        .try_get::<Vec<String>, _>("computes")?
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
            max_turns: unsigned(i64::from(plan.try_get::<i32, _>("max_task_turns")?)),
            max_cost_usd: plan.try_get("max_task_cost_usd")?,
            max_timeout_seconds: unsigned(plan.try_get("max_task_timeout_seconds")?),
            max_idle_seconds: unsigned(plan.try_get("max_idle_seconds")?),
            max_children_depth: unsigned(i64::from(plan.try_get::<i32, _>("max_children_depth")?)),
            max_children_count: unsigned(i64::from(plan.try_get::<i32, _>("max_children_count")?)),
            computes,
        },
    })
}

async fn store_revision(
    tx: &mut TenantTx,
    config_id: Uuid,
    sha: &str,
    files: &sanchoris_config::ConfigFiles,
    context: &ValidationContext,
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
    let revision_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO sanchoris.config_revisions (tenant_id, config_id, commit_sha, parsed, validation_status, validation_messages) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(tenant_id)
    .bind(config_id)
    .bind(sha)
    .bind(parsed_json)
    .bind(status)
    .bind(messages)
    .fetch_one(tx.conn())
    .await?;
    if let (Some(repository), "valid") = (repository, status) {
        activate(tx, config_id, revision_id, &repository).await?;
    }
    Ok(())
}

pub fn resolve_repository_key(config: &Config, trigger: &Trigger, key: &str) -> Option<String> {
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

pub fn rewrite_event_pattern(config: &Config, trigger: &Trigger, pattern: &Value) -> Value {
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

async fn activate(
    tx: &mut TenantTx,
    config_id: Uuid,
    revision_id: Uuid,
    repository: &ConfigRepository,
) -> Result<()> {
    let tenant_id = tx.tenant_id();
    let config = &repository.config;

    let names = config.projects.keys().cloned().collect::<Vec<_>>();
    sqlx::query("DELETE FROM sanchoris.projects WHERE config_id = $1 AND NOT (name = ANY($2))")
        .bind(config_id)
        .bind(&names)
        .execute(tx.conn())
        .await?;
    for (name, project) in &config.projects {
        let project_id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO sanchoris.projects (tenant_id, config_id, config_revision_id, name, system_prompt, source_path) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (config_id, name) DO UPDATE SET config_revision_id = EXCLUDED.config_revision_id, \
             system_prompt = EXCLUDED.system_prompt, source_path = EXCLUDED.source_path \
             RETURNING id",
        )
        .bind(tenant_id)
        .bind(config_id)
        .bind(revision_id)
        .bind(name)
        .bind(&project.system_prompt)
        .bind(sanchoris_config::CONFIG_FILE)
        .fetch_one(tx.conn())
        .await?;
        let repo_names = project.repos.keys().cloned().collect::<Vec<_>>();
        sqlx::query(
            "DELETE FROM sanchoris.repositories WHERE project_id = $1 AND NOT (name = ANY($2))",
        )
        .bind(project_id)
        .bind(&repo_names)
        .execute(tx.conn())
        .await?;
        for (repo_name, repo) in &project.repos {
            sqlx::query(
                "INSERT INTO sanchoris.repositories (tenant_id, project_id, name, url, warm_hook, start_hook) \
                 VALUES ($1, $2, $3, $4, $5, $6) \
                 ON CONFLICT (project_id, name) DO UPDATE SET url = EXCLUDED.url, \
                 warm_hook = EXCLUDED.warm_hook, start_hook = EXCLUDED.start_hook",
            )
            .bind(tenant_id)
            .bind(project_id)
            .bind(repo_name)
            .bind(&repo.repo)
            .bind(repo.warm_hook.as_ref().map(serde_json::to_value).transpose()?)
            .bind(repo.start_hook.as_ref().map(serde_json::to_value).transpose()?)
            .execute(tx.conn())
            .await?;
        }
    }

    let names = config.sessions.keys().cloned().collect::<Vec<_>>();
    sqlx::query("DELETE FROM sanchoris.sessions WHERE config_id = $1 AND NOT (name = ANY($2))")
        .bind(config_id)
        .bind(&names)
        .execute(tx.conn())
        .await?;
    for (name, session) in &config.sessions {
        sqlx::query(
            "INSERT INTO sanchoris.sessions (tenant_id, config_id, config_revision_id, name, definition) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (config_id, name) DO UPDATE SET config_revision_id = EXCLUDED.config_revision_id, \
             definition = EXCLUDED.definition",
        )
        .bind(tenant_id)
        .bind(config_id)
        .bind(revision_id)
        .bind(name)
        .bind(serde_json::to_value(session)?)
        .execute(tx.conn())
        .await?;
    }

    let names = repository.skills.keys().cloned().collect::<Vec<_>>();
    sqlx::query("DELETE FROM sanchoris.skills WHERE config_id = $1 AND NOT (name = ANY($2))")
        .bind(config_id)
        .bind(&names)
        .execute(tx.conn())
        .await?;
    for (name, skill) in &repository.skills {
        let definition = skill
            .definition
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?;
        let layout = skill
            .definition
            .as_ref()
            .and_then(|d| d.layout.as_ref())
            .map(serde_json::to_value)
            .transpose()?;
        sqlx::query(
            "INSERT INTO sanchoris.skills (tenant_id, config_id, config_revision_id, name, kind, description, skill_md, skill_yaml, definition, layout) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             ON CONFLICT (config_id, name) DO UPDATE SET config_revision_id = EXCLUDED.config_revision_id, \
             kind = EXCLUDED.kind, description = EXCLUDED.description, skill_md = EXCLUDED.skill_md, \
             skill_yaml = EXCLUDED.skill_yaml, definition = EXCLUDED.definition, layout = EXCLUDED.layout",
        )
        .bind(tenant_id)
        .bind(config_id)
        .bind(revision_id)
        .bind(name)
        .bind(if skill.is_reliable() { "reliable" } else { "regular" })
        .bind(&skill.description)
        .bind(&skill.skill_md)
        .bind(&skill.skill_yaml)
        .bind(definition)
        .bind(layout)
        .execute(tx.conn())
        .await?;
    }

    let previous = sqlx::query(
        "SELECT id, name, state, schedule_expression, removed_at IS NOT NULL AS removed \
         FROM sanchoris.triggers WHERE config_id = $1",
    )
    .bind(config_id)
    .fetch_all(tx.conn())
    .await?
    .iter()
    .map(|row| {
        Ok((
            row.try_get::<String, _>("name")?,
            (
                row.try_get::<Uuid, _>("id")?,
                row.try_get::<String, _>("state")?,
                row.try_get::<Option<String>, _>("schedule_expression")?,
                row.try_get::<bool, _>("removed")?,
            ),
        ))
    })
    .collect::<std::result::Result<BTreeMap<_, _>, sqlx::Error>>()?;
    let mut stale_schedules = Vec::new();
    for (name, (id, state, schedule, removed)) in &previous {
        let current = config.triggers.get(name);
        let same = current.is_some_and(|t| {
            let expression = match &t.source {
                TriggerSource::ScheduleExpression(s) => Some(s),
                TriggerSource::EventPattern(_) => None,
            };
            !removed
                && expression == schedule.as_ref()
                && state
                    == if t.state == TriggerState::Enabled {
                        "ENABLED"
                    } else {
                        "DISABLED"
                    }
        });
        if !same && schedule.is_some() {
            stale_schedules.push(*id);
        }
    }
    sqlx::query("DELETE FROM sanchoris.due_jobs WHERE kind = $1 AND ref_id = ANY($2)")
        .bind(DUE_TRIGGER_SCHEDULE)
        .bind(&stale_schedules)
        .execute(tx.conn())
        .await?;
    let names = config.triggers.keys().cloned().collect::<Vec<_>>();
    sqlx::query(
        "UPDATE sanchoris.triggers SET removed_at = now() \
         WHERE config_id = $1 AND NOT (name = ANY($2)) AND removed_at IS NULL",
    )
    .bind(config_id)
    .bind(&names)
    .execute(tx.conn())
    .await?;
    for (name, trigger) in &config.triggers {
        let (event_pattern, schedule_expression) = match &trigger.source {
            TriggerSource::EventPattern(pattern) => {
                (Some(rewrite_event_pattern(config, trigger, pattern)), None)
            }
            TriggerSource::ScheduleExpression(expression) => (None, Some(expression.clone())),
        };
        sqlx::query(
            "INSERT INTO sanchoris.triggers (tenant_id, config_id, config_revision_id, name, state, event_pattern, schedule_expression, input_transformer, start) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (config_id, name) DO UPDATE SET config_revision_id = EXCLUDED.config_revision_id, \
             state = EXCLUDED.state, event_pattern = EXCLUDED.event_pattern, \
             schedule_expression = EXCLUDED.schedule_expression, input_transformer = EXCLUDED.input_transformer, \
             start = EXCLUDED.start, removed_at = NULL",
        )
        .bind(tenant_id)
        .bind(config_id)
        .bind(revision_id)
        .bind(name)
        .bind(if trigger.state == TriggerState::Enabled { "ENABLED" } else { "DISABLED" })
        .bind(event_pattern)
        .bind(schedule_expression)
        .bind(trigger.input_transformer.as_ref().map(serde_json::to_value).transpose()?)
        .bind(serde_json::to_value(&trigger.start)?)
        .execute(tx.conn())
        .await?;
    }

    sqlx::query(
        "UPDATE sanchoris.configs SET active_revision_id = $2, updated_at = now() WHERE id = $1",
    )
    .bind(config_id)
    .bind(revision_id)
    .execute(tx.conn())
    .await?;
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
