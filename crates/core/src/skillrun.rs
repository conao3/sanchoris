use chrono::{DateTime, Utc};
use sanchoris_config::Skill;
use sanchoris_config::skill::SkillDefinition;
use sanchoris_engine::{
    Disposition, Effect, EngineError, Event, NotificationKind, SkillRun, StartRequest, StateEntry,
    Step, Timer,
};
use serde_json::{Value, json};
use sqlx::Row;
use thiserror::Error;
use uuid::Uuid;

use crate::db::{DbError, TenantTx};
use crate::lifecycle::decide_skill_run_finished;
use crate::model::{SkillRunStatus, TaskDefinition, TaskStatus};
use crate::tasks::{ERROR_APPROVAL_REQUESTED, apply_decision, send_to_task};
use crate::tokens::{TaskTokenSigner, TokenError};

pub const DUE_ENGINE_TIMER: &str = "engine_timer";

#[derive(Debug, Error)]
pub enum SkillRunError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("run {0} does not exist")]
    RunNotFound(Uuid),
    #[error("skill run {0} does not exist")]
    NotFound(String),
    #[error("skill {0:?} is not in this task's Start.Skills")]
    NotOffered(String),
    #[error("skill {0:?} has no skill.yaml and cannot be run by sanchoris")]
    NotReliable(String),
    #[error("the task is {0} and accepts no skill runs")]
    TaskFinished(TaskStatus),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("skill run {0} is not waiting for the agent")]
    NotWaitingForAgent(i32),
    #[error("the approval is not waiting")]
    ApprovalNotWaiting,
    #[error(transparent)]
    Token(#[from] TokenError),
    #[error(transparent)]
    Engine(EngineError),
}

impl From<sqlx::Error> for SkillRunError {
    fn from(error: sqlx::Error) -> Self {
        Self::Db(DbError::Sqlx(error))
    }
}

impl SkillRunError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::RunNotFound(_) | Self::NotFound(_) => "not_found",
            Self::NotOffered(_) => "skill_not_offered",
            Self::NotReliable(_) => "skill_not_reliable",
            Self::TaskFinished(_) => "task_finished",
            Self::InvalidInput(_) => "invalid_input",
            Self::NotWaitingForAgent(_) => "not_waiting",
            Self::ApprovalNotWaiting => "approval_not_waiting",
            Self::Token(_) => "invalid_token",
            Self::Db(_) | Self::Json(_) | Self::Engine(_) => "internal",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillRunMeta {
    pub id: Uuid,
    pub task_id: Uuid,
    pub run_id: Option<Uuid>,
    pub number: i32,
    pub skill_name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LockedSkillRun {
    pub meta: SkillRunMeta,
    pub definition: SkillDefinition,
    pub run: SkillRun,
}

pub fn engine_status(status: sanchoris_engine::SkillRunStatus) -> SkillRunStatus {
    match status {
        sanchoris_engine::SkillRunStatus::Running => SkillRunStatus::Running,
        sanchoris_engine::SkillRunStatus::WaitingAgent => SkillRunStatus::WaitingAgent,
        sanchoris_engine::SkillRunStatus::Waiting => SkillRunStatus::Waiting,
        sanchoris_engine::SkillRunStatus::Succeeded => SkillRunStatus::Succeeded,
        sanchoris_engine::SkillRunStatus::Failed => SkillRunStatus::Failed,
    }
}

pub fn parse_task_token(token: &str) -> Option<i32> {
    let rest = token.strip_prefix("sanchoris-task:")?;
    let number = rest.split(':').next()?;
    number.parse().ok()
}

pub async fn lock_skill_run(
    tx: &mut TenantTx,
    skill_run_id: Uuid,
) -> Result<LockedSkillRun, SkillRunError> {
    let row = sqlx::query(
        "SELECT id, task_id, run_id, number, skill_name, definition, state \
         FROM sanchoris.skill_runs WHERE id = $1 FOR UPDATE",
    )
    .bind(skill_run_id)
    .fetch_optional(tx.conn())
    .await?
    .ok_or_else(|| SkillRunError::NotFound(skill_run_id.to_string()))?;
    Ok(LockedSkillRun {
        meta: SkillRunMeta {
            id: row.try_get("id")?,
            task_id: row.try_get("task_id")?,
            run_id: row.try_get("run_id")?,
            number: row.try_get("number")?,
            skill_name: row.try_get("skill_name")?,
        },
        definition: serde_json::from_value(row.try_get("definition")?)?,
        run: serde_json::from_value(row.try_get("state")?)?,
    })
}

pub async fn apply_event(
    tx: &mut TenantTx,
    skill_run_id: Uuid,
    event: Event,
    now: DateTime<Utc>,
) -> Result<Disposition, SkillRunError> {
    let locked = lock_skill_run(tx, skill_run_id).await?;
    let step = sanchoris_engine::advance(&locked.definition, &locked.run, event, now)
        .map_err(SkillRunError::Engine)?;
    if step.disposition == Disposition::Applied {
        persist_step(tx, &locked.meta, &step, now).await?;
    }
    Ok(step.disposition)
}

pub fn timer_job_id(skill_run_id: Uuid, timer: &Timer) -> Uuid {
    let name = serde_json::to_vec(timer).expect("timers serialize");
    Uuid::new_v5(&skill_run_id, &name)
}

pub async fn persist_step(
    tx: &mut TenantTx,
    meta: &SkillRunMeta,
    step: &Step,
    now: DateTime<Utc>,
) -> Result<(), SkillRunError> {
    let tenant_id = tx.tenant_id();
    let run = &step.run;
    sqlx::query(
        "UPDATE sanchoris.skill_runs SET status = $2, current_state = $3, state = $4, output = $5, \
         error = $6, cause = $7, finished_at = $8, updated_at = now() WHERE id = $1",
    )
    .bind(meta.id)
    .bind(run.status.as_str())
    .bind(run.current.as_ref().map(|c| c.state_name.as_str()))
    .bind(serde_json::to_value(run)?)
    .bind(&run.output)
    .bind(&run.error)
    .bind(&run.cause)
    .bind(run.finished_at)
    .execute(tx.conn())
    .await?;
    for entry in &step.entries {
        upsert_entry(tx, meta.id, entry).await?;
    }
    for effect in &step.effects {
        match effect {
            Effect::InvokeResource(invocation) => {
                sqlx::query(
                    "INSERT INTO sanchoris.resource_invocations \
                     (tenant_id, skill_run_id, idempotency_key, resource, invocation, status) \
                     VALUES ($1, $2, $3, $4, $5, 'pending') \
                     ON CONFLICT (skill_run_id, idempotency_key) DO NOTHING",
                )
                .bind(tenant_id)
                .bind(meta.id)
                .bind(invocation.idempotency_key.to_string())
                .bind(invocation.resource.name())
                .bind(serde_json::to_value(invocation)?)
                .execute(tx.conn())
                .await?;
            }
            Effect::ScheduleTimer { timer, due_at } => {
                sqlx::query(
                    "INSERT INTO sanchoris.due_jobs (tenant_id, kind, ref_id, due_at, payload) \
                     VALUES ($1, $2, $3, $4, $5) \
                     ON CONFLICT (kind, ref_id) DO UPDATE SET due_at = EXCLUDED.due_at, attempts = 0, payload = EXCLUDED.payload",
                )
                .bind(tenant_id)
                .bind(DUE_ENGINE_TIMER)
                .bind(timer_job_id(meta.id, timer))
                .bind(due_at)
                .bind(json!({ "skill_run_id": meta.id, "timer": timer }))
                .execute(tx.conn())
                .await?;
            }
            Effect::CancelTimers { sequence } => {
                sqlx::query(
                    "DELETE FROM sanchoris.due_jobs WHERE kind = $1 \
                     AND payload->>'skill_run_id' = $2 AND (payload->'timer'->>'sequence')::bigint = $3",
                )
                .bind(DUE_ENGINE_TIMER)
                .bind(meta.id.to_string())
                .bind(i64::try_from(*sequence).expect("sequence fits in i64"))
                .execute(tx.conn())
                .await?;
            }
            Effect::CancelAllTimers => {
                sqlx::query(
                    "DELETE FROM sanchoris.due_jobs WHERE kind = $1 AND payload->>'skill_run_id' = $2",
                )
                .bind(DUE_ENGINE_TIMER)
                .bind(meta.id.to_string())
                .execute(tx.conn())
                .await?;
            }
            Effect::Notify(notification) => {
                if let Some(run_id) = meta.run_id {
                    let kind = match notification.kind {
                        NotificationKind::Succeeded { .. } => "succeeded",
                        NotificationKind::Failed { .. } => "failed",
                        NotificationKind::NeedsAgent { .. } => "needs_agent",
                    };
                    let dedupe = format!("skill:{}:{}:{}", meta.number, run.next_sequence, kind);
                    send_to_task(tx, meta.task_id, run_id, &notification.text, &dedupe).await?;
                }
            }
        }
    }
    if run.status.is_finished() {
        on_finished(tx, meta, run, now).await?;
    }
    Ok(())
}

async fn upsert_entry(
    tx: &mut TenantTx,
    skill_run_id: Uuid,
    entry: &StateEntry,
) -> Result<(), SkillRunError> {
    let tenant_id = tx.tenant_id();
    sqlx::query(
        "INSERT INTO sanchoris.state_entries \
         (tenant_id, skill_run_id, sequence, state_name, state_type, status, entered_at, exited_at, input, output, error, cause, attempts, next_state) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
         ON CONFLICT (skill_run_id, sequence) DO UPDATE SET status = EXCLUDED.status, exited_at = EXCLUDED.exited_at, \
         output = EXCLUDED.output, error = EXCLUDED.error, cause = EXCLUDED.cause, attempts = EXCLUDED.attempts, \
         next_state = EXCLUDED.next_state",
    )
    .bind(tenant_id)
    .bind(skill_run_id)
    .bind(i32::try_from(entry.sequence).expect("sequence fits in i32"))
    .bind(&entry.state_name)
    .bind(&entry.state_type)
    .bind(entry.status.as_str())
    .bind(entry.entered_at)
    .bind(entry.exited_at)
    .bind(&entry.input)
    .bind(&entry.output)
    .bind(&entry.error)
    .bind(&entry.cause)
    .bind(i32::try_from(entry.attempts.max(1)).expect("attempts fit in i32"))
    .bind(&entry.next_state)
    .execute(tx.conn())
    .await?;
    Ok(())
}

async fn on_finished(
    tx: &mut TenantTx,
    meta: &SkillRunMeta,
    run: &SkillRun,
    now: DateTime<Utc>,
) -> Result<(), SkillRunError> {
    let row =
        sqlx::query("SELECT status, definition FROM sanchoris.tasks WHERE id = $1 FOR UPDATE")
            .bind(meta.task_id)
            .fetch_one(tx.conn())
            .await?;
    let status: TaskStatus = row
        .try_get::<String, _>("status")?
        .parse()
        .expect("task status is valid");
    if status.is_terminal() {
        return Ok(());
    }
    let definition: TaskDefinition = serde_json::from_value(row.try_get("definition")?)?;
    let required = required_skills(&definition);
    let required = required.iter().map(String::as_str).collect::<Vec<_>>();
    let decision = decide_skill_run_finished(
        &required,
        &meta.skill_name,
        engine_status(run.status),
        run.error.as_deref().unwrap_or(""),
        run.cause.as_deref().unwrap_or(""),
    );
    if let (Some(decision), Some(run_id)) = (decision, meta.run_id) {
        apply_decision(
            tx,
            meta.task_id,
            run_id,
            &decision,
            &format!("skill:{}:finished", meta.number),
            now,
        )
        .await?;
    }
    Ok(())
}

pub fn required_skills(definition: &TaskDefinition) -> Vec<String> {
    definition.trigger.definition["Start"]["RequiredSkills"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq)]
pub struct AcceptedSkillRun {
    pub id: Uuid,
    pub number: i32,
    pub status: SkillRunStatus,
}

impl AcceptedSkillRun {
    pub fn response(&self) -> Value {
        json!({ "RunId": self.number, "Status": self.status.as_str() })
    }
}

async fn task_of_run(
    tx: &mut TenantTx,
    run_id: Uuid,
) -> Result<(Uuid, TaskStatus, TaskDefinition), SkillRunError> {
    let row = sqlx::query(
        "SELECT t.id, t.status, t.definition FROM sanchoris.runs r \
         JOIN sanchoris.tasks t ON t.id = r.task_id WHERE r.id = $1 FOR UPDATE OF t",
    )
    .bind(run_id)
    .fetch_optional(tx.conn())
    .await?
    .ok_or(SkillRunError::RunNotFound(run_id))?;
    let status: TaskStatus = row
        .try_get::<String, _>("status")?
        .parse()
        .expect("task status is valid");
    Ok((
        row.try_get("id")?,
        status,
        serde_json::from_value(row.try_get("definition")?)?,
    ))
}

pub async fn accept_skill_request(
    tx: &mut TenantTx,
    run_id: Uuid,
    skill_name: &str,
    input: Value,
    now: DateTime<Utc>,
) -> Result<AcceptedSkillRun, SkillRunError> {
    let (task_id, status, definition) = task_of_run(tx, run_id).await?;
    if status.is_terminal() {
        return Err(SkillRunError::TaskFinished(status));
    }
    let offered = definition
        .skills
        .iter()
        .find(|skill| skill.name == skill_name)
        .ok_or_else(|| SkillRunError::NotOffered(skill_name.to_string()))?;
    let skill: Skill = serde_json::from_value(offered.definition.clone())?;
    let skill_definition = skill
        .definition
        .ok_or_else(|| SkillRunError::NotReliable(skill_name.to_string()))?;
    let number = sqlx::query_scalar::<_, i32>(
        "SELECT coalesce(max(number), 0) + 1 FROM sanchoris.skill_runs WHERE task_id = $1",
    )
    .bind(task_id)
    .fetch_one(tx.conn())
    .await?;
    let step = sanchoris_engine::start(
        &skill_definition,
        StartRequest {
            skill_run_id: i64::from(number),
            skill_name: skill_name.to_string(),
            input: input.clone(),
        },
        now,
    )
    .map_err(|error| match error {
        EngineError::InvalidInput(message) => SkillRunError::InvalidInput(message),
        other => SkillRunError::Engine(other),
    })?;
    let tenant_id = tx.tenant_id();
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO sanchoris.skill_runs \
         (tenant_id, task_id, run_id, number, skill_name, definition, input, status, state, started_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'RUNNING', $8, $9) RETURNING id",
    )
    .bind(tenant_id)
    .bind(task_id)
    .bind(run_id)
    .bind(number)
    .bind(skill_name)
    .bind(serde_json::to_value(&skill_definition)?)
    .bind(&input)
    .bind(serde_json::to_value(&step.run)?)
    .bind(now)
    .fetch_one(tx.conn())
    .await?;
    let meta = SkillRunMeta {
        id,
        task_id,
        run_id: Some(run_id),
        number,
        skill_name: skill_name.to_string(),
    };
    persist_step(tx, &meta, &step, now).await?;
    Ok(AcceptedSkillRun {
        id,
        number,
        status: engine_status(step.run.status),
    })
}

async fn skill_run_by_number(
    tx: &mut TenantTx,
    run_id: Uuid,
    number: i32,
) -> Result<Uuid, SkillRunError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT s.id FROM sanchoris.runs r JOIN sanchoris.skill_runs s ON s.task_id = r.task_id \
         WHERE r.id = $1 AND s.number = $2",
    )
    .bind(run_id)
    .bind(number)
    .fetch_optional(tx.conn())
    .await?
    .ok_or_else(|| SkillRunError::NotFound(number.to_string()))
}

pub async fn continue_skill_run(
    tx: &mut TenantTx,
    run_id: Uuid,
    number: i32,
    output: Value,
    now: DateTime<Utc>,
) -> Result<Value, SkillRunError> {
    let id = skill_run_by_number(tx, run_id, number).await?;
    let locked = lock_skill_run(tx, id).await?;
    let token = match (locked.run.status, locked.run.waiting_task_token()) {
        (sanchoris_engine::SkillRunStatus::WaitingAgent, Some(token)) => token.to_string(),
        _ => return Err(SkillRunError::NotWaitingForAgent(number)),
    };
    let output = if output.is_null() { json!({}) } else { output };
    apply_event(
        tx,
        id,
        Event::TaskSuccess {
            task_token: token,
            output,
        },
        now,
    )
    .await?;
    skill_run_status(tx, run_id, number).await
}

pub async fn skill_run_status(
    tx: &mut TenantTx,
    run_id: Uuid,
    number: i32,
) -> Result<Value, SkillRunError> {
    let id = skill_run_by_number(tx, run_id, number).await?;
    let row =
        sqlx::query("SELECT status, output, error, cause FROM sanchoris.skill_runs WHERE id = $1")
            .bind(id)
            .fetch_one(tx.conn())
            .await?;
    Ok(json!({
        "RunId": number,
        "Status": row.try_get::<String, _>("status")?,
        "Output": row.try_get::<Option<Value>, _>("output")?,
        "Error": row.try_get::<Option<String>, _>("error")?,
        "Cause": row.try_get::<Option<String>, _>("cause")?,
    }))
}

pub async fn resolve_approval(
    tx: &mut TenantTx,
    signer: &TaskTokenSigner,
    signed_token: &str,
    approved: bool,
    member: &str,
    now: DateTime<Utc>,
) -> Result<Disposition, SkillRunError> {
    let verified = signer.verify(signed_token)?;
    let row = sqlx::query(
        "UPDATE sanchoris.approval_waits SET status = $3, resolved_at = $4 \
         WHERE skill_run_id = $1 AND task_token = $2 AND status = 'waiting' RETURNING task_id",
    )
    .bind(verified.skill_run_id)
    .bind(&verified.task_token)
    .bind(if approved { "approved" } else { "rejected" })
    .bind(now)
    .fetch_optional(tx.conn())
    .await?
    .ok_or(SkillRunError::ApprovalNotWaiting)?;
    let task_id: Uuid = row.try_get("task_id")?;
    sqlx::query(
        "UPDATE sanchoris.tasks SET status = 'WAITING', wait_reason = 'skill', error = NULL, cause = NULL, \
         updated_at = $3 WHERE id = $1 AND status = 'NEEDS_YOU' AND error = $2",
    )
    .bind(task_id)
    .bind(ERROR_APPROVAL_REQUESTED)
    .bind(now)
    .execute(tx.conn())
    .await?;
    let event = if approved {
        Event::TaskSuccess {
            task_token: verified.task_token,
            output: json!({ "Approved": true, "By": member }),
        }
    } else {
        Event::TaskFailure {
            task_token: verified.task_token,
            error: Some("Approval.Rejected".to_string()),
            cause: Some(format!("{member} rejected the request")),
        }
    };
    apply_event(tx, verified.skill_run_id, event, now).await
}
