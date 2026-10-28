use chrono::{DateTime, Utc};
use sanchoris_config::Skill;
use sanchoris_config::skill::SkillDefinition;
use sanchoris_engine::{
    Disposition, Effect, EngineError, Event, NotificationKind, SkillRun, StartRequest, StateEntry,
    Step, Timer,
};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::db::{DbError, REF_INVOCATION, REF_RUN, REF_SKILL_RUN, TenantTx, insert_ref, locate};
use crate::item::Entity;
use crate::items::{
    ApprovalWait, CheckWait, Invocation, SkillRunItem, StateEntryItem, Task, TimerItem,
};
use crate::jobs::Job;
use crate::keys;
use crate::lifecycle::decide_skill_run_finished;
use crate::model::{SkillRunStatus, TaskDefinition, TaskStatus, WaitReason};
use crate::tasks::{ERROR_APPROVAL_REQUESTED, apply_decision, load_task, send_to_task, store_task};
use crate::tokens::{TaskTokenSigner, TokenError};

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

    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Db(DbError::Conflict))
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

pub async fn skill_run_item(
    tx: &mut TenantTx,
    skill_run_id: Uuid,
) -> Result<Option<SkillRunItem>, DbError> {
    let Some(location) = locate(tx, skill_run_id, REF_SKILL_RUN).await? else {
        return Ok(None);
    };
    tx.get::<SkillRunItem>(&location.target).await
}

pub async fn lock_skill_run(
    tx: &mut TenantTx,
    skill_run_id: Uuid,
) -> Result<LockedSkillRun, SkillRunError> {
    let item = skill_run_item(tx, skill_run_id)
        .await?
        .ok_or_else(|| SkillRunError::NotFound(skill_run_id.to_string()))?;
    Ok(LockedSkillRun {
        meta: SkillRunMeta {
            id: item.id,
            task_id: item.task_id,
            run_id: item.run_id,
            number: item.number,
            skill_name: item.skill_name.clone(),
        },
        definition: serde_json::from_value(item.definition)?,
        run: serde_json::from_value(item.state)?,
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

pub async fn heartbeat_agent_skill_runs(
    tx: &mut TenantTx,
    task_id: Uuid,
    run_id: Uuid,
    now: DateTime<Utc>,
) -> Result<usize, SkillRunError> {
    let waiting = tx
        .query::<SkillRunItem>(&keys::task_pk(tx.tenant_id(), task_id), "SR#")
        .await?
        .into_iter()
        .filter(|s| s.run_id == Some(run_id) && s.status == SkillRunStatus::WaitingAgent)
        .collect::<Vec<_>>();
    let mut applied = 0;
    for item in waiting {
        let locked = lock_skill_run(tx, item.id).await?;
        let Some(task_token) = locked.run.waiting_task_token() else {
            continue;
        };
        let event = Event::TaskHeartbeat {
            task_token: task_token.to_string(),
        };
        if apply_event(tx, item.id, event, now).await? == Disposition::Applied {
            applied += 1;
        }
    }
    Ok(applied)
}

pub fn timer_job_id(skill_run_id: Uuid, timer: &Timer) -> Uuid {
    let name = serde_json::to_vec(timer).expect("timers serialize");
    Uuid::new_v5(&skill_run_id, &name)
}

fn timer_sequence(timer: &Timer) -> i64 {
    timer
        .sequence()
        .map(|s| i64::try_from(s).expect("sequence fits in i64"))
        .unwrap_or(-1)
}

async fn timers(tx: &mut TenantTx, meta: &SkillRunMeta) -> Result<Vec<TimerItem>, DbError> {
    tx.query::<TimerItem>(
        &keys::task_pk(tx.tenant_id(), meta.task_id),
        &keys::timer_prefix(meta.number),
    )
    .await
}

fn token_sequence(task_token: &str) -> Option<u64> {
    task_token
        .strip_prefix("sanchoris-task:")?
        .split(':')
        .nth(1)?
        .parse()
        .ok()
}

async fn cancel_waits(
    tx: &mut TenantTx,
    meta: &SkillRunMeta,
    sequence: Option<u64>,
    now: DateTime<Utc>,
) -> Result<(), DbError> {
    let tenant_id = tx.tenant_id();
    let pk = keys::task_pk(tenant_id, meta.task_id);
    let prefix = keys::skill_run_prefix(meta.number);
    let closes = |token: &str| sequence.is_none() || token_sequence(token) == sequence;
    for mut wait in tx
        .query::<CheckWait>(&pk, &format!("{prefix}#CHECK#"))
        .await?
    {
        if wait.status == "waiting" && closes(&wait.task_token) {
            wait.status = "cancelled".into();
            wait.completed_at = Some(now);
            tx.put(&wait)?;
            tx.delete(&keys::check_wait_marker_key(tenant_id, wait.id))?;
        }
    }
    for mut wait in tx
        .query::<ApprovalWait>(&pk, &format!("{prefix}#APPROVAL#"))
        .await?
    {
        if wait.status == "waiting" && closes(&wait.task_token) {
            wait.status = "cancelled".into();
            wait.resolved_at = Some(now);
            tx.put(&wait)?;
        }
    }
    Ok(())
}

pub async fn abort_skill_runs(
    tx: &mut TenantTx,
    task_id: Uuid,
    cause: &str,
    now: DateTime<Utc>,
) -> Result<usize, DbError> {
    let active = tx
        .query::<SkillRunItem>(&keys::task_pk(tx.tenant_id(), task_id), "SR#")
        .await?
        .into_iter()
        .filter(|s| s.status.is_active())
        .collect::<Vec<_>>();
    let mut aborted = 0;
    for item in &active {
        let locked = lock_skill_run(tx, item.id).await.map_err(into_db)?;
        let step = sanchoris_engine::abort(&locked.definition, &locked.run, cause, now)
            .map_err(|error| DbError::Decode(error.to_string()))?;
        if step.disposition == Disposition::Applied {
            Box::pin(persist_step(tx, &locked.meta, &step, now))
                .await
                .map_err(into_db)?;
            aborted += 1;
        }
    }
    Ok(aborted)
}

fn into_db(error: SkillRunError) -> DbError {
    match error {
        SkillRunError::Db(error) => error,
        SkillRunError::Json(error) => DbError::Definition(error),
        other => DbError::Decode(other.to_string()),
    }
}

pub async fn persist_step(
    tx: &mut TenantTx,
    meta: &SkillRunMeta,
    step: &Step,
    now: DateTime<Utc>,
) -> Result<(), SkillRunError> {
    let tenant_id = tx.tenant_id();
    let run = &step.run;
    let key = keys::skill_run_key(tenant_id, meta.task_id, meta.number);
    let mut item = tx
        .get::<SkillRunItem>(&key)
        .await?
        .ok_or_else(|| SkillRunError::NotFound(meta.id.to_string()))?;
    item.status = engine_status(run.status);
    item.current_state = run.current.as_ref().map(|c| c.state_name.clone());
    item.state = serde_json::to_value(run)?;
    item.output = run.output.clone();
    item.error = run.error.clone();
    item.cause = run.cause.clone();
    item.finished_at = run.finished_at;
    item.updated_at = now;
    tx.put(&item)?;
    for entry in &step.entries {
        upsert_entry(tx, meta, entry).await?;
    }
    for effect in &step.effects {
        match effect {
            Effect::InvokeResource(invocation) => {
                let key = keys::invocation_key(
                    tenant_id,
                    meta.task_id,
                    meta.number,
                    &invocation.idempotency_key.to_string(),
                );
                if tx.get::<Invocation>(&key).await?.is_none() {
                    let id = Uuid::new_v4();
                    tx.insert(&Invocation {
                        id,
                        tenant_id,
                        task_id: meta.task_id,
                        skill_run_id: meta.id,
                        skill_run_number: meta.number,
                        idempotency_key: invocation.idempotency_key.to_string(),
                        resource: invocation.resource.name(),
                        invocation: serde_json::to_value(invocation)?,
                        status: "pending".into(),
                        result: None,
                        attempts: 0,
                        created_at: now,
                        completed_at: None,
                    })?;
                    insert_ref(tx, id, REF_INVOCATION, key, Some(meta.task_id))?;
                    tx.emit(Job::Invoke {
                        tenant_id,
                        task_id: meta.task_id,
                        invocation_id: id,
                    });
                }
            }
            Effect::ScheduleTimer { timer, due_at } => {
                let id = timer_job_id(meta.id, timer);
                let key = keys::timer_key(tenant_id, meta.task_id, meta.number, id);
                let existing = tx.get::<TimerItem>(&key).await?;
                if let Some(existing) = &existing {
                    tx.cancel_schedule(existing.message_id);
                }
                let message_id = tx.emit_at(
                    Job::EngineTimer {
                        tenant_id,
                        task_id: meta.task_id,
                        skill_run_id: meta.id,
                        timer_id: id,
                    },
                    *due_at,
                );
                let timer_item = TimerItem {
                    id,
                    tenant_id,
                    task_id: meta.task_id,
                    skill_run_id: meta.id,
                    skill_run_number: meta.number,
                    sequence: timer_sequence(timer),
                    timer: serde_json::to_value(timer)?,
                    due_at: *due_at,
                    message_id,
                };
                if existing.is_some() {
                    tx.put(&timer_item)?;
                } else {
                    tx.insert(&timer_item)?;
                }
            }
            Effect::CancelTimers { sequence } => {
                let timer_sequence = i64::try_from(*sequence).expect("sequence fits in i64");
                for timer in timers(tx, meta).await? {
                    if timer.sequence == timer_sequence {
                        tx.cancel_schedule(timer.message_id);
                        tx.delete(&timer.key())?;
                    }
                }
                cancel_waits(tx, meta, Some(*sequence), now).await?;
            }
            Effect::CancelAllTimers => {
                for timer in timers(tx, meta).await? {
                    tx.cancel_schedule(timer.message_id);
                    tx.delete(&timer.key())?;
                }
                cancel_waits(tx, meta, None, now).await?;
            }
            Effect::Notify(notification) => {
                if let Some(run_id) = meta.run_id {
                    let kind = match notification.kind {
                        NotificationKind::Succeeded { .. } => "succeeded",
                        NotificationKind::Failed { .. } => "failed",
                        NotificationKind::NeedsAgent { .. } => "needs_agent",
                    };
                    let dedupe = format!("skill:{}:{}:{}", meta.number, run.next_sequence, kind);
                    send_to_task(tx, meta.task_id, run_id, &notification.text, &dedupe, now)
                        .await?;
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
    meta: &SkillRunMeta,
    entry: &StateEntry,
) -> Result<(), SkillRunError> {
    let tenant_id = tx.tenant_id();
    let sequence = i64::try_from(entry.sequence).expect("sequence fits in i64");
    let key = keys::state_entry_key(tenant_id, meta.task_id, meta.number, sequence);
    let existing = tx.get::<StateEntryItem>(&key).await?;
    let attempts = i64::from(entry.attempts.max(1));
    let item = match existing {
        Some(mut item) => {
            item.status = entry.status.as_str().to_string();
            item.exited_at = entry.exited_at;
            item.output = entry.output.clone();
            item.error = entry.error.clone();
            item.cause = entry.cause.clone();
            item.attempts = attempts;
            item.next_state = entry.next_state.clone();
            item
        }
        None => StateEntryItem {
            tenant_id,
            task_id: meta.task_id,
            skill_run_id: meta.id,
            skill_run_number: meta.number,
            sequence,
            state_name: entry.state_name.clone(),
            state_type: entry.state_type.clone(),
            status: entry.status.as_str().to_string(),
            entered_at: entry.entered_at,
            exited_at: entry.exited_at,
            input: entry.input.clone(),
            output: entry.output.clone(),
            error: entry.error.clone(),
            cause: entry.cause.clone(),
            attempts,
            next_state: entry.next_state.clone(),
        },
    };
    tx.put(&item)?;
    Ok(())
}

async fn on_finished(
    tx: &mut TenantTx,
    meta: &SkillRunMeta,
    run: &SkillRun,
    now: DateTime<Utc>,
) -> Result<(), SkillRunError> {
    let task = load_task(tx, meta.task_id).await?;
    if task.status.is_terminal() {
        return Ok(());
    }
    let definition: TaskDefinition = serde_json::from_value(task.definition.clone())?;
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

async fn task_of_run(tx: &mut TenantTx, run_id: Uuid) -> Result<Task, SkillRunError> {
    let location = locate(tx, run_id, REF_RUN)
        .await?
        .ok_or(SkillRunError::RunNotFound(run_id))?;
    let task_id = location
        .parent_id
        .ok_or(SkillRunError::RunNotFound(run_id))?;
    Ok(load_task(tx, task_id).await?)
}

pub async fn accept_skill_request(
    tx: &mut TenantTx,
    run_id: Uuid,
    skill_name: &str,
    input: Value,
    now: DateTime<Utc>,
) -> Result<AcceptedSkillRun, SkillRunError> {
    let mut task = task_of_run(tx, run_id).await?;
    if task.status.is_terminal() {
        return Err(SkillRunError::TaskFinished(task.status));
    }
    let definition: TaskDefinition = serde_json::from_value(task.definition.clone())?;
    let offered = definition
        .skills
        .iter()
        .find(|skill| skill.name == skill_name)
        .ok_or_else(|| SkillRunError::NotOffered(skill_name.to_string()))?;
    let skill: Skill = serde_json::from_value(offered.definition.clone())?;
    let skill_definition = skill
        .definition
        .ok_or_else(|| SkillRunError::NotReliable(skill_name.to_string()))?;
    let number = i32::try_from(task.skill_runs + 1).expect("skill run numbers fit in i32");
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
    task.skill_runs += 1;
    tx.put(&task)?;
    let tenant_id = tx.tenant_id();
    let id = Uuid::new_v4();
    let item = SkillRunItem {
        id,
        tenant_id,
        task_id: task.id,
        run_id: Some(run_id),
        number,
        skill_name: skill_name.to_string(),
        definition: serde_json::to_value(&skill_definition)?,
        input,
        output: None,
        status: SkillRunStatus::Running,
        current_state: None,
        state: serde_json::to_value(&step.run)?,
        error: None,
        cause: None,
        started_at: now,
        finished_at: None,
        updated_at: now,
    };
    tx.insert(&item)?;
    insert_ref(tx, id, REF_SKILL_RUN, item.key(), Some(task.id))?;
    let meta = SkillRunMeta {
        id,
        task_id: task.id,
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
) -> Result<SkillRunItem, SkillRunError> {
    let task = task_of_run(tx, run_id).await?;
    let key = keys::skill_run_key(tx.tenant_id(), task.id, number);
    tx.get::<SkillRunItem>(&key)
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
    let id = skill_run_by_number(tx, run_id, number).await?.id;
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
    let item = skill_run_by_number(tx, run_id, number).await?;
    Ok(json!({
        "RunId": number,
        "Status": item.status.as_str(),
        "Output": item.output,
        "Error": item.error,
        "Cause": item.cause,
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
    let skill_run = skill_run_item(tx, verified.skill_run_id)
        .await?
        .ok_or(SkillRunError::ApprovalNotWaiting)?;
    let key = keys::approval_wait_key(
        tx.tenant_id(),
        skill_run.task_id,
        skill_run.number,
        &verified.task_token,
    );
    let mut wait = tx
        .get::<ApprovalWait>(&key)
        .await?
        .filter(|w| w.status == "waiting")
        .ok_or(SkillRunError::ApprovalNotWaiting)?;
    let locked = lock_skill_run(tx, verified.skill_run_id).await?;
    if locked.run.waiting_task_token() != Some(verified.task_token.as_str()) {
        return Err(SkillRunError::ApprovalNotWaiting);
    }
    wait.status = if approved { "approved" } else { "rejected" }.to_string();
    wait.resolved_at = Some(now);
    tx.put(&wait)?;
    let mut task = load_task(tx, wait.task_id).await?;
    if task.status == TaskStatus::NeedsYou
        && task.error.as_deref() == Some(ERROR_APPROVAL_REQUESTED)
    {
        task.status = TaskStatus::Waiting;
        task.wait_reason = Some(WaitReason::Skill);
        task.error = None;
        task.cause = None;
        task.updated_at = now;
        store_task(tx, &task, now).await?;
    }
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
