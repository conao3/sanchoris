use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::item::{Entity, Key};
use crate::items::{
    Child, Dependency, PendingTask, Plan, Ref, Run, RunChannel, Task, TaskName, Tenant,
    TriggerEvent,
};
use crate::jobs::Job;
use crate::keys;
use crate::model::{TaskDefinition, TaskStatus, TriggerEventResult};
use crate::store::Update;
use crate::tasks::store_task;

pub use crate::store::{DbError, Store, TenantTx};

pub const REF_TRIGGER_EVENT: &str = "trigger_event";
pub const REF_RUN: &str = "run";
pub const REF_SKILL_RUN: &str = "skill_run";
pub const REF_INVOCATION: &str = "resource_invocation";
pub const REF_CHECK_WAIT: &str = "check_wait";
pub const REF_TRIGGER: &str = "trigger";
pub const REF_CONFIG_REVISION: &str = "config_revision";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    New(Uuid),
    Duplicate(Uuid),
}

impl Recorded {
    pub fn id(self) -> Uuid {
        match self {
            Self::New(id) | Self::Duplicate(id) => id,
        }
    }
}

pub fn insert_ref(
    tx: &mut TenantTx,
    id: Uuid,
    kind: &str,
    target: Key,
    parent_id: Option<Uuid>,
) -> Result<(), DbError> {
    let tenant_id = tx.tenant_id();
    tx.insert(&Ref {
        tenant_id,
        id,
        kind: kind.to_string(),
        target,
        parent_id,
    })
}

pub async fn locate(tx: &mut TenantTx, id: Uuid, kind: &str) -> Result<Option<Ref>, DbError> {
    let key = keys::ref_key(tx.tenant_id(), id);
    Ok(tx.get::<Ref>(&key).await?.filter(|r| r.kind == kind))
}

pub async fn plan(store: &Store, plan_id: &str) -> Result<Plan, DbError> {
    store
        .get::<Plan>(&keys::plan_key(plan_id))
        .await?
        .ok_or_else(|| DbError::Missing(format!("plan {plan_id}")))
}

pub async fn plans(store: &Store) -> Result<Vec<Plan>, DbError> {
    store.query::<Plan>(keys::plans_pk(), "").await
}

pub async fn tenant(tx: &mut TenantTx) -> Result<Tenant, DbError> {
    let tenant_id = tx.tenant_id();
    tx.get::<Tenant>(&keys::tenant_key(tenant_id))
        .await?
        .ok_or(DbError::TenantNotFound(tenant_id))
}

pub async fn tenant_plan(tx: &mut TenantTx) -> Result<(Tenant, Plan), DbError> {
    let tenant = tenant(tx).await?;
    let plan = plan(tx.store(), &tenant.plan_id).await?;
    Ok((tenant, plan))
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewTriggerEvent<'a> {
    pub trigger_id: Uuid,
    pub key: &'a str,
    pub event: &'a Value,
}

pub async fn record_trigger_event(
    tx: &mut TenantTx,
    event: &NewTriggerEvent<'_>,
) -> Result<Recorded, DbError> {
    let tenant_id = tx.tenant_id();
    let key = keys::trigger_event_key(tenant_id, event.trigger_id, event.key);
    if let Some(existing) = tx.get::<TriggerEvent>(&key).await? {
        return Ok(Recorded::Duplicate(existing.id));
    }
    let id = Uuid::new_v4();
    tx.insert(&TriggerEvent {
        id,
        tenant_id,
        trigger_id: event.trigger_id,
        key: event.key.to_string(),
        event: event.event.clone(),
        received_at: Utc::now(),
        result: None,
        reason: None,
        evaluated_at: None,
        task_id: None,
    })?;
    insert_ref(tx, id, REF_TRIGGER_EVENT, key, Some(event.trigger_id))?;
    Ok(Recorded::New(id))
}

#[derive(Debug, Clone, PartialEq)]
pub enum TriggerEventOutcome {
    Started,
    Skipped { reason: String },
    Discarded { reason: String },
}

impl TriggerEventOutcome {
    fn parts(&self) -> (TriggerEventResult, Option<&str>) {
        match self {
            Self::Started => (TriggerEventResult::Started, None),
            Self::Skipped { reason } => (TriggerEventResult::Skipped, Some(reason)),
            Self::Discarded { reason } => (TriggerEventResult::Discarded, Some(reason)),
        }
    }
}

pub async fn set_trigger_event_outcome(
    tx: &mut TenantTx,
    trigger_event_id: Uuid,
    outcome: &TriggerEventOutcome,
) -> Result<(), DbError> {
    let not_pending = DbError::TriggerEventNotPending(trigger_event_id);
    let Some(location) = locate(tx, trigger_event_id, REF_TRIGGER_EVENT).await? else {
        return Err(not_pending);
    };
    let Some(mut event) = tx.get::<TriggerEvent>(&location.target).await? else {
        return Err(not_pending);
    };
    if event.result.is_some() {
        return Err(not_pending);
    }
    let (result, reason) = outcome.parts();
    event.result = Some(result.as_str().to_string());
    event.reason = reason.map(str::to_string);
    event.evaluated_at = Some(Utc::now());
    tx.put(&event)
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewTask<'a> {
    pub trigger_id: Uuid,
    pub trigger_event_id: Option<Uuid>,
    pub config_revision_id: Uuid,
    pub name: &'a str,
    pub title: &'a str,
    pub definition: &'a TaskDefinition,
    pub parent_task_id: Option<Uuid>,
    pub channel: Option<&'a Value>,
    pub requested_by_member_id: Option<Uuid>,
    pub blocked_by: &'a [Uuid],
}

pub async fn create_task(tx: &mut TenantTx, task: &NewTask<'_>) -> Result<Recorded, DbError> {
    let tenant_id = tx.tenant_id();
    let name_key = keys::task_name_key(tenant_id, task.trigger_id, task.name);
    if let Some(existing) = tx.get::<TaskName>(&name_key).await? {
        return Ok(Recorded::Duplicate(existing.task_id));
    }
    let id = Uuid::new_v4();
    let now = Utc::now();
    if let Some(trigger_event_id) = task.trigger_event_id {
        let location = locate(tx, trigger_event_id, REF_TRIGGER_EVENT)
            .await?
            .ok_or_else(|| DbError::Missing(format!("trigger event {trigger_event_id}")))?;
        let mut event = tx.require::<TriggerEvent>(&location.target).await?;
        event.task_id = Some(id);
        tx.put(&event)?;
    }
    tx.insert(&Task {
        id,
        tenant_id,
        trigger_id: task.trigger_id,
        trigger_event_id: task.trigger_event_id,
        config_revision_id: task.config_revision_id,
        name: task.name.to_string(),
        title: task.title.to_string(),
        status: TaskStatus::Pending,
        wait_reason: None,
        resume_at: None,
        snoozed_until: None,
        nudges: 0,
        definition: serde_json::to_value(task.definition)?,
        parent_task_id: task.parent_task_id,
        channel: task.channel.cloned(),
        start_payload: None,
        requested_by_member_id: task.requested_by_member_id,
        error: None,
        cause: None,
        runs: 0,
        skill_runs: 0,
        created_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    })?;
    tx.insert(&TaskName {
        tenant_id,
        trigger_id: task.trigger_id,
        name: task.name.to_string(),
        task_id: id,
        created_at: now,
    })?;
    tx.insert(&PendingTask {
        tenant_id,
        task_id: id,
        created_at: now,
    })?;
    for blocker in task.blocked_by {
        tx.insert(&Dependency {
            tenant_id,
            task_id: id,
            blocked_by_task_id: *blocker,
        })?;
    }
    if let Some(parent) = task.parent_task_id {
        tx.insert(&Child {
            tenant_id,
            parent_task_id: parent,
            task_id: id,
            created_at: now,
        })?;
    }
    tx.emit(Job::Dispatch { tenant_id });
    Ok(Recorded::New(id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchedTask {
    pub task_id: Uuid,
    pub run_id: Uuid,
}

async fn is_blocked(tx: &mut TenantTx, task_id: Uuid) -> Result<bool, DbError> {
    let tenant_id = tx.tenant_id();
    let dependencies = tx
        .query::<Dependency>(&keys::task_pk(tenant_id, task_id), "DEP#")
        .await?;
    for dependency in dependencies {
        let blocker = tx
            .get::<Task>(&keys::task_key(tenant_id, dependency.blocked_by_task_id))
            .await?;
        if blocker.is_some_and(|b| !b.status.is_terminal()) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub async fn dispatch_pending_tasks(
    tx: &mut TenantTx,
    now: DateTime<Utc>,
) -> Result<Vec<DispatchedTask>, DbError> {
    let tenant_id = tx.tenant_id();
    let (tenant, plan) = tenant_plan(tx).await?;
    if tenant.status != "active"
        || tenant.workspace_seconds_this_period >= plan.workspace_seconds_limit
    {
        return Ok(Vec::new());
    }
    let slots = (plan.max_concurrency - tenant.running_tasks)
        .min(plan.task_limit - tenant.tasks_this_period);
    if slots <= 0 {
        return Ok(Vec::new());
    }
    let markers = tx
        .query::<PendingTask>(&keys::tenant_pk(tenant_id), "PENDINGTASK#")
        .await?;
    let mut picked = Vec::new();
    for marker in markers {
        if i64::try_from(picked.len()).expect("small") >= slots {
            break;
        }
        let task = tx
            .get::<Task>(&keys::task_key(tenant_id, marker.task_id))
            .await?;
        let Some(task) = task.filter(|t| t.status == TaskStatus::Pending) else {
            tx.delete(&marker.key())?;
            continue;
        };
        if is_blocked(tx, task.id).await? {
            continue;
        }
        picked.push((marker, task));
    }
    if picked.is_empty() {
        return Ok(Vec::new());
    }
    let mut dispatched = Vec::new();
    for (marker, mut task) in picked {
        tx.delete(&marker.key())?;
        let run_id = Uuid::new_v4();
        task.status = TaskStatus::Running;
        task.started_at = Some(task.started_at.unwrap_or(now));
        task.updated_at = now;
        task.runs += 1;
        let definition = &task.definition;
        let session = &definition["Session"];
        let run = Run {
            id: run_id,
            tenant_id,
            task_id: task.id,
            number: task.runs,
            session_name: session["Name"].as_str().unwrap_or_default().to_string(),
            worker: session["Definition"]["Worker"]["Agent"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            model: session["Definition"]["Worker"]["Model"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            agent_session_id: None,
            status: "creating".into(),
            turns: 0,
            cost_usd: 0.0,
            instance_costs: Default::default(),
            summary: None,
            error: None,
            resume_branch: None,
            record_s3_key: None,
            log_s3_key: None,
            last_heartbeat_at: None,
            started_at: now,
            finished_at: None,
            updated_at: now,
        };
        store_task(tx, &task, now).await?;
        tx.insert(&run)?;
        tx.insert(&RunChannel {
            tenant_id,
            run_id,
            task_id: task.id,
            next_outbox_seq: 1,
            instances: 0,
            connection_id: None,
            connection_instance: None,
            connection_started: false,
        })?;
        insert_ref(tx, run_id, REF_RUN, run.key(), Some(task.id))?;
        tx.emit(Job::Reconcile {
            tenant_id,
            task_id: task.id,
            run_id,
        });
        dispatched.push(DispatchedTask {
            task_id: task.id,
            run_id,
        });
    }
    let count = i64::try_from(dispatched.len()).expect("small");
    tx.update(
        &keys::tenant_key(tenant_id),
        Update::new()
            .add("tasks_this_period", count)
            .cap_after("tasks_this_period", plan.task_limit)
            .cap_after("running_tasks", plan.max_concurrency)
            .require(
                "status",
                aws_sdk_dynamodb::types::AttributeValue::S("active".into()),
            ),
    )?;
    Ok(dispatched)
}
