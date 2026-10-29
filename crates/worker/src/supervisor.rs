use chrono::{DateTime, Duration, Utc};
use sanchoris_config::{Session, Start};
use sanchoris_core::channel::{OutboxType, RunEventType, SEQ_STRIDE, enqueue_outbox, outbox};
use sanchoris_core::db::{self, REF_TRIGGER, TenantTx, locate};
use sanchoris_core::items::{
    Config, OutboxItem, Pending, Repository, Run, RunEventItem, SkillRunItem, Task, UsageRecord,
    Workspace,
};
use sanchoris_core::jobs::Job;
use sanchoris_core::lifecycle::{
    ERROR_WORKSPACE_STOPPED, RequiredSkill, TaskDecision, TaskLimits, TaskProgress, TurnEnd,
    check_limits, decide_human_reply, decide_turn_end, decide_usage_limit,
    decide_usage_limit_resume,
};
use sanchoris_core::model::{
    SkillRunStatus, TaskDefinition, TaskStatus, UsageLimitResume, WaitReason,
};
use sanchoris_core::skillrun::{heartbeat_agent_skill_runs, required_skills};
use sanchoris_core::store::{Index, SortKey, Update};
use sanchoris_core::tasks::{
    apply_decision, idle_seconds, latest_run, load_run, load_task, runs_of, send_to_task,
    task_branch,
};
use sanchoris_core::{keys, messages};
use sanchoris_engine::ResourceOutcome;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::engine_worker::{complete_invocation, invocation_by_id};
use crate::error::{Result, WorkerError};
use crate::ports::WorkspaceSpec;
use crate::{Worker, usage};

pub const ERROR_USAGE_LIMIT: &str = "Worker.UsageLimit";
const LAUNCH_TIMEOUT_MINUTES: i64 = 15;
const HANDOFF_HEARTBEAT_SECONDS: u64 = 900;
const MIN_HEARTBEAT_INTERVAL_SECONDS: u64 = 30;

pub fn heartbeat_interval(limit_seconds: u64) -> u64 {
    (limit_seconds.min(HANDOFF_HEARTBEAT_SECONDS) / 3).max(MIN_HEARTBEAT_INTERVAL_SECONDS)
}

fn definition_of(task: &Task) -> Result<TaskDefinition> {
    Ok(serde_json::from_value(task.definition.clone())?)
}

fn start_of(task: &Task) -> Result<Start> {
    Ok(serde_json::from_value(
        task.definition["Trigger"]["Definition"]["Start"].clone(),
    )?)
}

fn session_of(task: &Task) -> Result<Session> {
    Ok(serde_json::from_value(
        task.definition["Session"]["Definition"].clone(),
    )?)
}

fn invalid(message: impl Into<String>) -> WorkerError {
    WorkerError::Invalid(message.into())
}

pub async fn process_run_events(worker: &Worker, tenant_id: Uuid, run_id: Uuid) -> Result<usize> {
    let mut processed = 0;
    loop {
        let mut tx = worker.tx(tenant_id);
        let markers = tx
            .query::<Pending>(&keys::run_pk(tenant_id, run_id), "PEND#")
            .await?;
        let Some(marker) = markers.into_iter().next() else {
            return Ok(processed);
        };
        tx.delete(&marker.key)?;
        let Some(mut event) = tx
            .get::<RunEventItem>(&marker.target)
            .await?
            .filter(|e| e.processed_at.is_none())
        else {
            tx.commit().await?;
            continue;
        };
        let run = load_run(&mut tx, run_id)
            .await?
            .ok_or_else(|| invalid(format!("run {run_id} does not exist")))?;
        let error = match handle(worker, &mut tx, run.task_id, &event).await {
            Ok(()) => None,
            Err(WorkerError::Invalid(message)) => Some(message),
            Err(other) => return Err(other),
        };
        event.processed_at = Some(worker.now());
        event.error = error;
        tx.put(&event)?;
        worker.faults.check("run_event.before_commit")?;
        tx.commit().await?;
        processed += 1;
    }
}

async fn require_run(tx: &mut TenantTx, run_id: Uuid) -> Result<Run> {
    load_run(tx, run_id)
        .await?
        .ok_or_else(|| invalid(format!("run {run_id} does not exist")))
}

async fn handle(
    worker: &Worker,
    tx: &mut TenantTx,
    task_id: Uuid,
    event: &RunEventItem,
) -> Result<()> {
    let payload = &event.payload;
    let kind = RunEventType::parse(&event.kind)
        .ok_or_else(|| invalid(format!("unknown run event type {}", event.kind)))?;
    match kind {
        RunEventType::Ready => {
            let mut run = require_run(tx, event.run_id).await?;
            if matches!(run.status.as_str(), "creating" | "idle") {
                run.status = "running".into();
                run.agent_session_id = payload["sessionId"].as_str().map(str::to_string);
                run.last_heartbeat_at = Some(worker.now());
                run.updated_at = worker.now();
                tx.put(&run)?;
            }
        }
        RunEventType::Heartbeat => {
            let mut run = require_run(tx, event.run_id).await?;
            run.last_heartbeat_at = Some(worker.now());
            run.updated_at = worker.now();
            tx.put(&run)?;
        }
        RunEventType::SessionRecord => {
            let key = payload["key"]
                .as_str()
                .ok_or_else(|| invalid("session_record needs key"))?;
            let mut run = require_run(tx, event.run_id).await?;
            run.record_s3_key = Some(key.to_string());
            if let Some(session) = payload["sessionId"].as_str() {
                run.agent_session_id = Some(session.to_string());
            }
            run.updated_at = worker.now();
            tx.put(&run)?;
        }
        RunEventType::ExecResult => exec_result(worker, tx, payload).await?,
        RunEventType::RateLimit => {
            usage::record_rate_limit(worker, tx, task_id, event.run_id, payload).await?
        }
        RunEventType::TurnEnd => turn_end(worker, tx, task_id, event).await?,
        RunEventType::HumanReply => human_reply(worker, tx, task_id, event).await?,
        RunEventType::Stopping => signaled(worker, tx, task_id, event.run_id).await?,
    }
    let agent_active = match kind {
        RunEventType::HumanReply | RunEventType::Stopping => false,
        RunEventType::Heartbeat => payload["busy"].as_bool() == Some(true),
        _ => true,
    };
    if agent_active {
        heartbeat_agent_skill_runs(tx, task_id, event.run_id, worker.now()).await?;
    }
    Ok(())
}

async fn signaled(worker: &Worker, tx: &mut TenantTx, task_id: Uuid, run_id: Uuid) -> Result<()> {
    let now = worker.now();
    let open = tx
        .query::<Workspace>(&keys::task_pk(tx.tenant_id(), task_id), "WS#")
        .await?
        .into_iter()
        .filter(|w| w.run_id == run_id && w.stopped_at.is_none() && w.signaled_at.is_none());
    for mut workspace in open {
        workspace.signaled_at = Some(now);
        tx.put(&workspace)?;
    }
    Ok(())
}

async fn exec_result(worker: &Worker, tx: &mut TenantTx, payload: &Value) -> Result<()> {
    let request_id: Uuid = payload["requestId"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| invalid("exec_result needs requestId"))?;
    let (key, invocation) = invocation_by_id(tx, request_id)
        .await?
        .filter(|(_, item)| item.status == "awaiting")
        .ok_or_else(|| invalid(format!("no command is awaiting result {request_id}")))?;
    let results = payload["results"].clone();
    let ok = payload["ok"]
        .as_bool()
        .ok_or_else(|| invalid("exec_result needs ok"))?;
    let outcome = if ok {
        let output = if invocation.resource == "git:push" {
            json!({ "Pushed": invocation.invocation["parameters"]["Repos"] })
        } else {
            json!({ "Results": results })
        };
        ResourceOutcome::Succeeded { output }
    } else {
        let cause = results
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .find(|r| r["exitCode"].as_i64() != Some(0))
                    .map(|r| {
                        format!(
                            "{} exited with {}: {}",
                            r["command"].as_str().unwrap_or("command"),
                            r["exitCode"],
                            r["stderr"].as_str().unwrap_or("")
                        )
                    })
            })
            .or_else(|| payload["error"].as_str().map(str::to_string))
            .unwrap_or_else(|| "the command failed".to_string());
        ResourceOutcome::Failed {
            error: None,
            cause: Some(cause),
        }
    };
    complete_invocation(worker, tx, &key, outcome).await
}

async fn latest_skill_status(
    tx: &mut TenantTx,
    task_id: Uuid,
    skill: &str,
) -> Result<Option<SkillRunStatus>> {
    Ok(tx
        .query::<SkillRunItem>(&keys::task_pk(tx.tenant_id(), task_id), "SR#")
        .await?
        .into_iter()
        .filter(|s| s.skill_name == skill)
        .max_by_key(|s| s.number)
        .map(|s| s.status))
}

async fn turn_end(
    worker: &Worker,
    tx: &mut TenantTx,
    task_id: Uuid,
    event: &RunEventItem,
) -> Result<()> {
    let now = worker.now();
    let payload = &event.payload;
    let turn = payload["turn"]
        .as_i64()
        .ok_or_else(|| invalid("turn_end needs turn"))?;
    let total_cost = payload["totalCostUsd"].as_f64().unwrap_or(0.0);
    let instance = event
        .seq
        .ok_or_else(|| invalid("turn_end needs a seq"))?
        .div_euclid(SEQ_STRIDE)
        .to_string();
    let usage = &payload["usage"];
    let tokens = |name: &str| usage[name].as_i64().unwrap_or(0);
    let mut run = require_run(tx, event.run_id).await?;
    let previous = run.instance_costs.get(&instance).copied().unwrap_or(0.0);
    let cost = (total_cost - previous).max(0.0);
    run.instance_costs
        .insert(instance, total_cost.max(previous));
    run.turns += 1;
    run.cost_usd += cost;
    run.summary = payload["summary"].as_str().map(str::to_string);
    if let Some(session) = payload["sessionId"].as_str() {
        run.agent_session_id = Some(session.to_string());
    }
    run.last_heartbeat_at = Some(now);
    run.updated_at = now;
    tx.put(&run)?;
    tx.insert(&UsageRecord {
        id: Uuid::new_v4(),
        tenant_id: tx.tenant_id(),
        task_id,
        run_id: event.run_id,
        input_tokens: tokens("inputTokens"),
        output_tokens: tokens("outputTokens"),
        cache_read_tokens: tokens("cacheReadInputTokens"),
        cache_write_tokens: tokens("cacheCreationInputTokens"),
        cost_usd: cost,
        recorded_at: now,
    })?;
    let task = load_task(tx, task_id).await?;
    if task.status != TaskStatus::Running || payload["result"] == "interrupted" {
        return Ok(());
    }
    let start = start_of(&task)?;
    let dedupe = format!("turn:{turn}");
    let error = &payload["error"];
    if error["code"] == ERROR_USAGE_LIMIT {
        let reset_at = error["resetAt"]
            .as_i64()
            .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
            .ok_or_else(|| invalid("a usage limit turn_end needs error.resetAt"))?;
        let resume = match session_of(&task)?.usage_limit.resume {
            sanchoris_config::ResumeMode::Auto => UsageLimitResume::Auto,
            sanchoris_config::ResumeMode::Manual => UsageLimitResume::Manual,
        };
        let decision = decide_usage_limit(resume, reset_at);
        apply_decision(tx, task.id, event.run_id, &decision, &dedupe, now).await?;
        return Ok(());
    }
    let names = required_skills(&definition_of(&task)?);
    let mut required = Vec::new();
    for name in &names {
        required.push(RequiredSkill {
            name,
            latest: latest_skill_status(tx, task.id, name).await?,
        });
    }
    let mut decision = decide_turn_end(&TurnEnd {
        required: &required,
        nudges: u32::try_from(task.nudges).expect("nudges are non-negative"),
        max_nudges: u32::try_from(start.max_nudges).expect("MaxNudges fits in u32"),
    });
    if decision != TaskDecision::Succeed
        && let Some(limited) = check_limits(
            &limits_of(&start),
            &TaskProgress {
                turns: u32::try_from(run.turns).expect("turns fit"),
                cost_usd: run.cost_usd,
                started_at: task.started_at.unwrap_or(now),
                last_activity_at: now,
            },
            now,
        )
    {
        decision = limited;
    }
    apply_decision(tx, task.id, event.run_id, &decision, &dedupe, now).await?;
    Ok(())
}

fn limits_of(start: &Start) -> TaskLimits {
    TaskLimits {
        turns: u32::try_from(start.limits.turns).expect("Turns fits in u32"),
        cost_usd: start.limits.cost_usd,
        timeout_seconds: i64::try_from(start.limits.timeout_seconds).expect("fits"),
        heartbeat_seconds: i64::try_from(start.limits.heartbeat_seconds).expect("fits"),
    }
}

async fn human_reply(
    worker: &Worker,
    tx: &mut TenantTx,
    task_id: Uuid,
    event: &RunEventItem,
) -> Result<()> {
    let member = event.payload["member"]
        .as_str()
        .ok_or_else(|| invalid("human_reply needs member"))?;
    let text = event.payload["text"]
        .as_str()
        .ok_or_else(|| invalid("human_reply needs text"))?;
    let task = load_task(tx, task_id).await?;
    let decision = decide_human_reply(member, text);
    let dedupe = format!("reply:{}", event.id);
    match task.status {
        TaskStatus::NeedsYou | TaskStatus::Waiting => {
            apply_decision(tx, task.id, event.run_id, &decision, &dedupe, worker.now()).await?;
        }
        TaskStatus::Running | TaskStatus::Pending => {
            let TaskDecision::Resume { message, .. } = &decision else {
                unreachable!("a human reply resumes the task");
            };
            send_to_task(tx, task.id, event.run_id, message, &dedupe, worker.now()).await?;
        }
        finished => return Err(invalid(format!("the task is {finished}"))),
    }
    Ok(())
}

pub async fn resume(
    worker: &Worker,
    tenant_id: Uuid,
    task_id: Uuid,
    resume_at: DateTime<Utc>,
) -> Result<usize> {
    let now = worker.now();
    let mut tx = worker.tx(tenant_id);
    let task = load_task(&mut tx, task_id).await?;
    let waiting = task.status == TaskStatus::Waiting
        && task.wait_reason == Some(WaitReason::UsageLimit)
        && task.resume_at == Some(resume_at);
    if !waiting {
        return Ok(0);
    }
    let Some(decision) = decide_usage_limit_resume(resume_at, now) else {
        return Ok(0);
    };
    let Some(run_id) = latest_run(&mut tx, task.id).await? else {
        return Ok(0);
    };
    apply_decision(
        &mut tx,
        task.id,
        run_id,
        &decision,
        &format!("resume:{}", resume_at.timestamp()),
        now,
    )
    .await?;
    tx.commit().await?;
    Ok(1)
}

pub async fn sweep_workspaces(worker: &Worker) -> Result<usize> {
    let active = worker
        .store
        .query_index::<Task>(Index::Gsi2, keys::SWEEP_ACTIVE_TASKS, SortKey::All)
        .await?;
    let mut changed = 0;
    for task in active {
        changed += observe_workspaces(worker, task.tenant_id, task.id).await?;
    }
    changed += stop_unrecorded_workspaces(worker).await?;
    worker.deps.runner.sweep().await?;
    Ok(changed)
}

async fn stop_unrecorded_workspaces(worker: &Worker) -> Result<usize> {
    let mut stopped = 0;
    for running in worker.deps.runner.running_workspaces().await? {
        let mut tx = worker.tx(running.tenant_id);
        let key = keys::workspace_key(running.tenant_id, running.task_id, running.workspace_id);
        let recorded = tx.get::<Workspace>(&key).await?.is_some_and(|workspace| {
            workspace.run_id == running.run_id
                && workspace.stopped_at.is_none()
                && workspace
                    .platform_ref
                    .as_deref()
                    .is_none_or(|platform_ref| platform_ref == running.platform_ref)
        });
        if recorded {
            continue;
        }
        worker
            .deps
            .runner
            .stop_workspace(running.run_id, &running.platform_ref)
            .await?;
        stopped += 1;
    }
    Ok(stopped)
}

async fn observe_workspaces(worker: &Worker, tenant_id: Uuid, task_id: Uuid) -> Result<usize> {
    let now = worker.now();
    let mut tx = worker.tx(tenant_id);
    let open = tx
        .query::<Workspace>(&keys::task_pk(tenant_id, task_id), "WS#")
        .await?
        .into_iter()
        .filter(|w| w.stopped_at.is_none())
        .collect::<Vec<_>>();
    let mut stopped = Vec::new();
    for mut workspace in open {
        let (reason, interrupted_at) = match workspace.platform_ref.clone() {
            None if workspace.started_at + Duration::minutes(LAUNCH_TIMEOUT_MINUTES) <= now => (
                format!("the workspace did not launch within {LAUNCH_TIMEOUT_MINUTES} minutes"),
                None,
            ),
            None => continue,
            Some(platform_ref) => {
                let Some(exit) = worker.deps.runner.workspace_exit(&platform_ref).await? else {
                    continue;
                };
                (exit.reason, exit.interrupted_at)
            }
        };
        let interrupted_at = match (interrupted_at, workspace.signaled_at) {
            (Some(platform), Some(agent)) => Some(platform.min(agent)),
            (platform, agent) => platform.or(agent),
        };
        close_workspace(&mut tx, &mut workspace, interrupted_at.unwrap_or(now))?;
        stopped.push(Stopped {
            workspace_id: workspace.id,
            run_id: workspace.run_id,
            reason,
            interrupted_at,
        });
    }
    if stopped.is_empty() {
        return Ok(0);
    }
    for stop in &stopped {
        let Some(mut run) = tx
            .get::<Run>(&keys::run_key(tenant_id, task_id, stop.run_id))
            .await?
        else {
            continue;
        };
        if !run.is_finished() {
            run.status = "idle".into();
            run.error = stop.interrupted_at.is_none().then(|| stop.reason.clone());
            run.updated_at = now;
            tx.put(&run)?;
        }
    }
    let task = load_task(&mut tx, task_id).await?;
    if let Some(failed) = stopped
        .iter()
        .rev()
        .find(|stop| stop.interrupted_at.is_none())
    {
        if task.status == TaskStatus::Running {
            apply_decision(
                &mut tx,
                task_id,
                failed.run_id,
                &TaskDecision::NeedsYou {
                    error: ERROR_WORKSPACE_STOPPED.into(),
                    cause: failed.reason.clone(),
                },
                "workspace-stopped",
                now,
            )
            .await?;
        }
    } else if !task.status.is_terminal() {
        for stop in &stopped {
            restore_after_interruption(&mut tx, &task, stop, now).await?;
        }
    }
    tx.commit().await?;
    Ok(stopped.len())
}

struct Stopped {
    workspace_id: Uuid,
    run_id: Uuid,
    reason: String,
    interrupted_at: Option<DateTime<Utc>>,
}

async fn restore_after_interruption(
    tx: &mut TenantTx,
    task: &Task,
    stop: &Stopped,
    now: DateTime<Utc>,
) -> Result<()> {
    let Some(run) = load_run(tx, stop.run_id).await? else {
        return Ok(());
    };
    if run.is_finished() {
        return Ok(());
    }
    let mut redelivered = false;
    for mut item in outbox(tx, run.id).await? {
        let Some(delivered_at) = item.delivered_at else {
            continue;
        };
        if item.kind == "user_message"
            && stop
                .interrupted_at
                .is_some_and(|interrupted_at| delivered_at >= interrupted_at)
        {
            item.delivered_at = None;
            tx.put(&item)?;
            redelivered = true;
            continue;
        }
        if item.kind != "exec" {
            continue;
        }
        let Some(request_id) = item.payload["requestId"]
            .as_str()
            .and_then(|id| id.parse::<Uuid>().ok())
        else {
            continue;
        };
        let awaiting = invocation_by_id(tx, request_id)
            .await?
            .is_some_and(|(_, invocation)| invocation.status == "awaiting");
        if awaiting {
            item.delivered_at = None;
            tx.put(&item)?;
            redelivered = true;
        }
    }
    if task.status == TaskStatus::Running {
        send_to_task(
            tx,
            task.id,
            run.id,
            &messages::restored_message(),
            &format!("restore:{}", stop.workspace_id),
            now,
        )
        .await?;
    } else if redelivered {
        tx.emit(Job::Reconcile {
            tenant_id: tx.tenant_id(),
            task_id: task.id,
            run_id: run.id,
        });
    }
    Ok(())
}

pub async fn sweep_limits(worker: &Worker) -> Result<usize> {
    let active = worker
        .store
        .query_index::<Task>(Index::Gsi2, keys::SWEEP_ACTIVE_TASKS, SortKey::All)
        .await?;
    let mut changed = 0;
    for task in active {
        changed += check_task_limits(worker, task.tenant_id, task.id).await?;
    }
    Ok(changed)
}

async fn check_task_limits(worker: &Worker, tenant_id: Uuid, task_id: Uuid) -> Result<usize> {
    let now = worker.now();
    let mut tx = worker.tx(tenant_id);
    let task = load_task(&mut tx, task_id).await?;
    if !matches!(task.status, TaskStatus::Running | TaskStatus::Waiting) {
        return Ok(0);
    }
    let Some(run) = runs_of(&mut tx, task.id).await?.pop() else {
        return Ok(0);
    };
    let start = start_of(&task)?;
    let last_activity_at = if task.status == TaskStatus::Running {
        run.last_heartbeat_at
            .unwrap_or(run.started_at)
            .max(task.updated_at)
    } else {
        now
    };
    let decision = check_limits(
        &limits_of(&start),
        &TaskProgress {
            turns: u32::try_from(run.turns).expect("turns fit"),
            cost_usd: run.cost_usd,
            started_at: task.started_at.unwrap_or(now),
            last_activity_at,
        },
        now,
    );
    let Some(decision) = decision else {
        return Ok(0);
    };
    apply_decision(&mut tx, task.id, run.id, &decision, "limits", now).await?;
    enqueue_outbox(
        &mut tx,
        run.id,
        OutboxType::Interrupt,
        &json!({}),
        "interrupt:limits",
    )
    .await?;
    tx.commit().await?;
    Ok(1)
}

fn skills_payload(definition: &TaskDefinition, required: &[String]) -> Result<Vec<Value>> {
    definition
        .skills
        .iter()
        .map(|skill| {
            let parsed: sanchoris_config::Skill = serde_json::from_value(skill.definition.clone())?;
            let inputs = parsed
                .definition
                .as_ref()
                .map(|d| serde_json::to_value(&d.inputs))
                .transpose()?
                .unwrap_or_else(|| json!({}));
            Ok(json!({
                "name": skill.name,
                "required": required.contains(&skill.name),
                "inputs": inputs,
            }))
        })
        .collect()
}

fn wants_workspace(task: &Task) -> bool {
    task.status == TaskStatus::Running
        || (task.status == TaskStatus::Waiting
            && matches!(task.wait_reason, Some(WaitReason::Skill | WaitReason::Chat)))
}

fn wakes_the_session(item: &OutboxItem) -> bool {
    item.delivered_at.is_none() && matches!(item.kind.as_str(), "user_message" | "exec")
}

pub async fn reconcile(
    worker: &Worker,
    tenant_id: Uuid,
    task_id: Uuid,
    run_id: Uuid,
) -> Result<usize> {
    let now = worker.now();
    let mut tx = worker.tx(tenant_id);
    let Some(task) = tx.get::<Task>(&keys::task_key(tenant_id, task_id)).await? else {
        return Ok(0);
    };
    let Some(run) = tx
        .get::<Run>(&keys::run_key(tenant_id, task_id, run_id))
        .await?
    else {
        return Ok(0);
    };
    let mut open = tx
        .query::<Workspace>(&keys::task_pk(tenant_id, task_id), "WS#")
        .await?
        .into_iter()
        .filter(|w| w.run_id == run_id && w.stopped_at.is_none())
        .collect::<Vec<_>>();
    open.sort_by_key(|a| (a.started_at, a.id));
    let pending = outbox(&mut tx, run_id).await?.iter().any(wakes_the_session);
    match open.into_iter().next() {
        None => {
            let startable = matches!(run.status.as_str(), "creating" | "idle")
                && wants_workspace(&task)
                && (run.status == "creating" || pending);
            if !startable {
                return Ok(0);
            }
            start_workspace(worker, tx, task, run).await
        }
        Some(workspace) if workspace.platform_ref.is_none() => {
            if run.is_finished() || task.status.is_terminal() {
                return stop_workspace(worker, tx, run, workspace).await;
            }
            drop(tx);
            launch_workspace(worker, &task, workspace).await
        }
        Some(workspace) => {
            let idle = matches!(task.status, TaskStatus::Waiting | TaskStatus::NeedsYou)
                && idle_seconds(&task).is_some_and(|idle| task.updated_at + idle <= now)
                && !pending;
            if !(run.is_finished() || idle) {
                return Ok(0);
            }
            stop_workspace(worker, tx, run, workspace).await
        }
    }
}

async fn checkout_repositories(
    tx: &mut TenantTx,
    task: &Task,
    session: &Session,
) -> Result<Vec<String>> {
    let Some(location) = locate(tx, task.trigger_id, REF_TRIGGER).await? else {
        return Ok(Vec::new());
    };
    let config_id = location
        .parent_id
        .ok_or_else(|| invalid("a trigger reference has no config"))?;
    let config = tx
        .require::<Config>(&keys::config_key(tx.tenant_id(), config_id))
        .await?;
    match &session.checkout {
        sanchoris_config::Checkout::Repos(repos) => repos
            .iter()
            .map(|name| {
                config
                    .repository_named(name)
                    .map(|repository| repository.full_name.clone())
                    .ok_or_else(|| {
                        invalid(format!("{name} is not a repository of the active config"))
                    })
            })
            .collect(),
        sanchoris_config::Checkout::Project(project) => {
            let revision_id = config
                .active_revision_id
                .ok_or_else(|| invalid(format!("config {config_id} has no active revision")))?;
            let mut repositories = tx
                .query::<Repository>(
                    &keys::config_pk(tx.tenant_id(), config_id),
                    &keys::project_repositories_prefix(revision_id, project),
                )
                .await?;
            repositories.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(repositories.into_iter().map(|r| r.full_name).collect())
        }
    }
}

async fn start_workspace(
    worker: &Worker,
    mut tx: TenantTx,
    task: Task,
    mut run: Run,
) -> Result<usize> {
    let now = worker.now();
    let tenant_id = tx.tenant_id();
    let session = session_of(&task)?;
    let start = start_of(&task)?;
    let definition = definition_of(&task)?;
    let repositories = checkout_repositories(&mut tx, &task, &session).await?;
    let branch = task_branch(task.id);
    let (_, plan) = db::tenant_plan(&mut tx).await?;
    let workspace = Workspace {
        id: Uuid::new_v4(),
        tenant_id,
        task_id: task.id,
        run_id: run.id,
        compute: session.workspace.compute.as_str().to_string(),
        architecture: session.workspace.architecture.as_str().to_string(),
        cpu: plan.workspace_cpu,
        memory_mib: plan.workspace_memory_mib,
        platform: worker.deps.runner.platform().to_string(),
        platform_ref: None,
        capacity_provider: None,
        repositories: repositories.clone(),
        branch: branch.clone(),
        base_commit: None,
        started_at: now,
        signaled_at: None,
        stopped_at: None,
    };
    tx.insert(&workspace)?;
    for mut item in outbox(&mut tx, run.id).await? {
        if item.delivered_at.is_none() && matches!(item.kind.as_str(), "start" | "shutdown") {
            item.delivered_at = Some(now);
            tx.put(&item)?;
        }
    }
    let resume = match (run.status.as_str(), &run.record_s3_key) {
        ("idle", Some(key)) => json!({
            "sessionId": run.agent_session_id,
            "recordKey": key,
            "lastTurn": run.turns,
        }),
        _ => Value::Null,
    };
    let start_payload = task.start_payload.clone().unwrap_or(Value::Null);
    let instructed = run.status == "creating" || (run.record_s3_key.is_none() && run.turns == 0);
    let payload = json!({
        "agent": task.definition["Session"]["Definition"]["Worker"]["Agent"],
        "model": task.definition["Session"]["Definition"]["Worker"]["Model"],
        "systemPrompt": start_payload["systemPrompt"],
        "skills": skills_payload(&definition, &start.required_skills)?,
        "instruction": if instructed { start_payload["instruction"].clone() } else { Value::Null },
        "resume": resume,
        "heartbeatSeconds": heartbeat_interval(start.limits.heartbeat_seconds),
        "branch": branch,
        "repositories": repositories,
    });
    enqueue_outbox(
        &mut tx,
        run.id,
        OutboxType::Start,
        &payload,
        &format!("start:{}", workspace.id),
    )
    .await?;
    run.status = "creating".into();
    run.updated_at = now;
    tx.put(&run)?;
    worker.faults.check("workspace_start.before_commit")?;
    tx.commit().await?;
    launch_workspace(worker, &task, workspace).await
}

async fn launch_workspace(worker: &Worker, task: &Task, workspace: Workspace) -> Result<usize> {
    let session = session_of(task)?;
    let handle = worker
        .deps
        .runner
        .start_workspace(&WorkspaceSpec {
            run_id: workspace.run_id,
            tenant_id: workspace.tenant_id,
            task_id: workspace.task_id,
            workspace_id: workspace.id,
            started_at: workspace.started_at,
            branch: workspace.branch.clone(),
            compute: workspace.compute.clone(),
            architecture: workspace.architecture.clone(),
            cpu: workspace.cpu,
            memory_mib: workspace.memory_mib,
            repositories: workspace.repositories.clone(),
            network_allow: session.network.allow.clone(),
        })
        .await?;
    worker.faults.check("workspace_start.after_launch")?;
    let mut tx = worker.tx(workspace.tenant_id);
    let key = keys::workspace_key(workspace.tenant_id, workspace.task_id, workspace.id);
    let Some(mut recorded) = tx.get::<Workspace>(&key).await? else {
        return Err(invalid(format!("workspace {} disappeared", workspace.id)));
    };
    if recorded.stopped_at.is_some() {
        worker
            .deps
            .runner
            .stop_workspace(workspace.run_id, &handle.platform_ref)
            .await?;
        return Ok(1);
    }
    recorded.platform_ref = Some(handle.platform_ref);
    recorded.capacity_provider = handle.capacity_provider;
    tx.put(&recorded)?;
    tx.commit().await?;
    Ok(1)
}

fn close_workspace(
    tx: &mut TenantTx,
    workspace: &mut Workspace,
    counted_until: DateTime<Utc>,
) -> Result<()> {
    let weight = match workspace.compute.as_str() {
        "docker" => 3,
        _ => 1,
    };
    let seconds = (counted_until - workspace.started_at).num_seconds().max(0) * weight;
    workspace.stopped_at = Some(counted_until.max(workspace.started_at));
    tx.put(workspace)?;
    tx.update(
        &keys::tenant_key(tx.tenant_id()),
        Update::new().add("workspace_seconds_this_period", seconds),
    )?;
    Ok(())
}

async fn stop_workspace(
    worker: &Worker,
    mut tx: TenantTx,
    mut run: Run,
    mut workspace: Workspace,
) -> Result<usize> {
    let now = worker.now();
    if let Some(platform_ref) = &workspace.platform_ref {
        worker
            .deps
            .runner
            .stop_workspace(run.id, platform_ref)
            .await?;
    }
    close_workspace(&mut tx, &mut workspace, now)?;
    if !run.is_finished() {
        run.status = "idle".into();
        run.updated_at = now;
        tx.put(&run)?;
        enqueue_outbox(
            &mut tx,
            run.id,
            OutboxType::Shutdown,
            &json!({ "reason": "the session is idle" }),
            &format!("shutdown:{}", workspace.id),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(1)
}
