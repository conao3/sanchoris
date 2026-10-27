use chrono::{DateTime, Utc};
use sanchoris_config::{Session, Start};
use sanchoris_core::channel::{OutboxType, RunEventType, enqueue_outbox};
use sanchoris_core::db::{
    self, TenantTx, claim_due_jobs, complete_due_job, tenants_with_pending_tasks,
};
use sanchoris_core::lifecycle::{
    RequiredSkill, TaskDecision, TaskLimits, TaskProgress, TurnEnd, check_limits,
    decide_human_reply, decide_turn_end, decide_usage_limit, decide_usage_limit_resume,
};
use sanchoris_core::model::{
    SkillRunStatus, TaskDefinition, TaskStatus, UsageLimitResume, WaitReason,
};
use sanchoris_core::skillrun::required_skills;
use sanchoris_core::tasks::{DUE_TASK_RESUME, apply_decision, send_to_task, task_branch};
use sanchoris_engine::ResourceOutcome;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::engine_worker::complete_invocation;
use crate::error::{Result, WorkerError};
use crate::ports::WorkspaceSpec;
use crate::{Worker, adopt, usage};

pub const ERROR_USAGE_LIMIT: &str = "Worker.UsageLimit";

struct RunEvent {
    id: Uuid,
    run_id: Uuid,
    kind: RunEventType,
    payload: Value,
}

struct TaskRow {
    id: Uuid,
    status: TaskStatus,
    nudges: i32,
    definition: TaskDefinition,
    started_at: Option<DateTime<Utc>>,
}

async fn lock_task(tx: &mut TenantTx, task_id: Uuid) -> Result<TaskRow> {
    let row = sqlx::query(
        "SELECT id, status, nudges, definition, started_at \
         FROM sanchoris.tasks WHERE id = $1 FOR UPDATE",
    )
    .bind(task_id)
    .fetch_one(tx.conn())
    .await?;
    Ok(TaskRow {
        id: row.try_get("id")?,
        status: row
            .try_get::<String, _>("status")?
            .parse()
            .expect("task status is valid"),
        nudges: row.try_get("nudges")?,
        definition: serde_json::from_value(row.try_get("definition")?)?,
        started_at: row.try_get("started_at")?,
    })
}

fn start_of(task: &TaskRow) -> Result<Start> {
    Ok(serde_json::from_value(
        task.definition.trigger.definition["Start"].clone(),
    )?)
}

fn session_of(task: &TaskRow) -> Result<Session> {
    Ok(serde_json::from_value(
        task.definition.session.definition.clone(),
    )?)
}

pub async fn tick_run_events(worker: &Worker) -> Result<usize> {
    let mut processed = 0;
    loop {
        let mut tx = worker.begin().await?;
        let Some(row) = sqlx::query(
            "SELECT id, tenant_id, run_id, type, payload FROM sanchoris.run_events \
             WHERE processed_at IS NULL ORDER BY received_at, seq NULLS LAST, id LIMIT 1 \
             FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(processed);
        };
        let kind: String = row.try_get("type")?;
        let event = RunEvent {
            id: row.try_get("id")?,
            run_id: row.try_get("run_id")?,
            kind: RunEventType::parse(&kind)
                .ok_or_else(|| WorkerError::Invalid(format!("unknown run event type {kind}")))?,
            payload: row.try_get("payload")?,
        };
        let mut tx = adopt(tx, row.try_get("tenant_id")?).await?;
        let task_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT task_id FROM sanchoris.runs WHERE id = $1 FOR UPDATE",
        )
        .bind(event.run_id)
        .fetch_one(tx.conn())
        .await?;
        let error = match handle(worker, &mut tx, task_id, &event).await {
            Ok(()) => None,
            Err(WorkerError::Invalid(message)) => Some(message),
            Err(other) => return Err(other),
        };
        sqlx::query("UPDATE sanchoris.run_events SET processed_at = $2, error = $3 WHERE id = $1")
            .bind(event.id)
            .bind(worker.now())
            .bind(error)
            .execute(tx.conn())
            .await?;
        worker.faults.check("run_event.before_commit")?;
        tx.commit().await?;
        processed += 1;
    }
}

fn invalid(message: impl Into<String>) -> WorkerError {
    WorkerError::Invalid(message.into())
}

async fn handle(worker: &Worker, tx: &mut TenantTx, task_id: Uuid, event: &RunEvent) -> Result<()> {
    let payload = &event.payload;
    match event.kind {
        RunEventType::Ready => {
            sqlx::query(
                "UPDATE sanchoris.runs SET status = 'running', agent_session_id = $2, \
                 last_heartbeat_at = $3, updated_at = now() WHERE id = $1 AND status IN ('creating', 'idle')",
            )
            .bind(event.run_id)
            .bind(payload["sessionId"].as_str())
            .bind(worker.now())
            .execute(tx.conn())
            .await?;
        }
        RunEventType::Heartbeat => {
            sqlx::query(
                "UPDATE sanchoris.runs SET last_heartbeat_at = $2, updated_at = now() WHERE id = $1",
            )
            .bind(event.run_id)
            .bind(worker.now())
            .execute(tx.conn())
            .await?;
        }
        RunEventType::SessionRecord => {
            let key = payload["key"]
                .as_str()
                .ok_or_else(|| invalid("session_record needs key"))?;
            sqlx::query(
                "UPDATE sanchoris.runs SET record_s3_key = $2, agent_session_id = coalesce($3, agent_session_id), \
                 updated_at = now() WHERE id = $1",
            )
            .bind(event.run_id)
            .bind(key)
            .bind(payload["sessionId"].as_str())
            .execute(tx.conn())
            .await?;
        }
        RunEventType::ExecResult => exec_result(worker, tx, payload).await?,
        RunEventType::RateLimit => {
            usage::record_rate_limit(worker, tx, task_id, event.run_id, payload).await?
        }
        RunEventType::TurnEnd => turn_end(worker, tx, task_id, event).await?,
        RunEventType::HumanReply => human_reply(worker, tx, task_id, event).await?,
    }
    Ok(())
}

async fn exec_result(worker: &Worker, tx: &mut TenantTx, payload: &Value) -> Result<()> {
    let request_id: Uuid = payload["requestId"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| invalid("exec_result needs requestId"))?;
    let invocation = sqlx::query(
        "SELECT resource, invocation FROM sanchoris.resource_invocations \
         WHERE id = $1 AND tenant_id = $2 AND status = 'awaiting' FOR UPDATE",
    )
    .bind(request_id)
    .bind(tx.tenant_id())
    .fetch_optional(tx.conn())
    .await?
    .ok_or_else(|| invalid(format!("no command is awaiting result {request_id}")))?;
    let resource: String = invocation.try_get("resource")?;
    let results = payload["results"].clone();
    let ok = payload["ok"]
        .as_bool()
        .ok_or_else(|| invalid("exec_result needs ok"))?;
    let outcome = if ok {
        let output = if resource == "git:push" {
            let parameters = &invocation.try_get::<Value, _>("invocation")?["parameters"];
            json!({ "Pushed": parameters["Repos"] })
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
    complete_invocation(worker, tx, request_id, outcome).await
}

async fn latest_skill_status(
    tx: &mut TenantTx,
    task_id: Uuid,
    skill: &str,
) -> Result<Option<SkillRunStatus>> {
    let status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM sanchoris.skill_runs WHERE task_id = $1 AND skill_name = $2 \
         ORDER BY number DESC LIMIT 1",
    )
    .bind(task_id)
    .bind(skill)
    .fetch_optional(tx.conn())
    .await?;
    Ok(status.map(|s| s.parse().expect("skill run status is valid")))
}

async fn turn_end(
    worker: &Worker,
    tx: &mut TenantTx,
    task_id: Uuid,
    event: &RunEvent,
) -> Result<()> {
    let now = worker.now();
    let payload = &event.payload;
    let turn = payload["turn"]
        .as_i64()
        .ok_or_else(|| invalid("turn_end needs turn"))?;
    let cost = payload["totalCostUsd"].as_f64().unwrap_or(0.0);
    let usage = &payload["usage"];
    let tokens = |name: &str| usage[name].as_i64().unwrap_or(0);
    let run = sqlx::query(
        "UPDATE sanchoris.runs SET turns = turns + 1, cost_usd = cost_usd + $2, summary = $3, \
         agent_session_id = coalesce($4, agent_session_id), last_heartbeat_at = $5, updated_at = now() \
         WHERE id = $1 RETURNING turns, cost_usd::float8 AS cost_usd",
    )
    .bind(event.run_id)
    .bind(cost)
    .bind(payload["summary"].as_str())
    .bind(payload["sessionId"].as_str())
    .bind(now)
    .fetch_one(tx.conn())
    .await?;
    sqlx::query(
        "INSERT INTO sanchoris.usage (tenant_id, run_id, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cost_usd, recorded_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(tx.tenant_id())
    .bind(event.run_id)
    .bind(tokens("inputTokens"))
    .bind(tokens("outputTokens"))
    .bind(tokens("cacheReadInputTokens"))
    .bind(tokens("cacheCreationInputTokens"))
    .bind(cost)
    .bind(now)
    .execute(tx.conn())
    .await?;
    let task = lock_task(tx, task_id).await?;
    if task.status != TaskStatus::Running {
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
    let names = required_skills(&task.definition);
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
                turns: u32::try_from(run.try_get::<i32, _>("turns")?).expect("turns fit"),
                cost_usd: run.try_get("cost_usd")?,
                started_at: task.started_at.unwrap_or(now),
                last_activity_at: now,
            },
            now,
        )
    {
        decision = limited;
    }
    apply_decision(tx, task.id, event.run_id, &decision, &dedupe, now).await?;
    if decision == TaskDecision::Succeed {
        db::dispatch_pending_tasks(tx, now).await?;
    }
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
    event: &RunEvent,
) -> Result<()> {
    let member = event.payload["member"]
        .as_str()
        .ok_or_else(|| invalid("human_reply needs member"))?;
    let text = event.payload["text"]
        .as_str()
        .ok_or_else(|| invalid("human_reply needs text"))?;
    let task = lock_task(tx, task_id).await?;
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

pub async fn tick_resumes(worker: &Worker) -> Result<usize> {
    let mut processed = 0;
    loop {
        let now = worker.now();
        let mut tx = worker.begin().await?;
        let Some(job) = claim_due_jobs(&mut tx, DUE_TASK_RESUME, now, 1)
            .await?
            .into_iter()
            .next()
        else {
            return Ok(processed);
        };
        complete_due_job(&mut tx, job.id).await?;
        let mut tx = adopt(tx, job.tenant_id).await?;
        let task = lock_task(&mut tx, job.ref_id).await?;
        let resume_at = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
            "SELECT resume_at FROM sanchoris.tasks WHERE id = $1 AND status = 'WAITING' AND wait_reason = $2",
        )
        .bind(task.id)
        .bind(WaitReason::UsageLimit.as_str())
        .fetch_optional(tx.conn())
        .await?
        .flatten();
        if let Some(resume_at) = resume_at
            && let Some(decision) = decide_usage_limit_resume(resume_at, now)
            && let Some(run_id) = sanchoris_core::tasks::latest_run(&mut tx, task.id).await?
        {
            apply_decision(
                &mut tx,
                task.id,
                run_id,
                &decision,
                &format!("resume:{}", resume_at.timestamp()),
                now,
            )
            .await?;
        }
        tx.commit().await?;
        processed += 1;
    }
}

pub async fn tick_limits(worker: &Worker) -> Result<usize> {
    let now = worker.now();
    let rows = sqlx::query(
        "SELECT id, tenant_id FROM sanchoris.tasks WHERE status IN ('RUNNING', 'WAITING') ORDER BY id",
    )
    .fetch_all(&worker.pool)
    .await?;
    let mut changed = 0;
    for row in rows {
        let task_id: Uuid = row.try_get("id")?;
        let tx = worker.begin().await?;
        let mut tx = adopt(tx, row.try_get("tenant_id")?).await?;
        let task = lock_task(&mut tx, task_id).await?;
        if !matches!(task.status, TaskStatus::Running | TaskStatus::Waiting) {
            continue;
        }
        let Some(run) = sqlx::query(
            "SELECT r.id, r.turns, r.cost_usd::float8 AS cost_usd, \
                    greatest(coalesce(r.last_heartbeat_at, r.started_at), t.updated_at) AS active_at \
             FROM sanchoris.runs r JOIN sanchoris.tasks t ON t.id = r.task_id \
             WHERE r.task_id = $1 ORDER BY r.started_at DESC, r.id DESC LIMIT 1",
        )
        .bind(task.id)
        .fetch_optional(tx.conn())
        .await?
        else {
            continue;
        };
        let start = start_of(&task)?;
        let last_activity_at = if task.status == TaskStatus::Running {
            run.try_get::<DateTime<Utc>, _>("active_at")?
        } else {
            now
        };
        let decision = check_limits(
            &limits_of(&start),
            &TaskProgress {
                turns: u32::try_from(run.try_get::<i32, _>("turns")?).expect("turns fit"),
                cost_usd: run.try_get("cost_usd")?,
                started_at: task.started_at.unwrap_or(now),
                last_activity_at,
            },
            now,
        );
        if let Some(decision) = decision {
            let run_id: Uuid = run.try_get("id")?;
            apply_decision(&mut tx, task.id, run_id, &decision, "limits", now).await?;
            enqueue_outbox(
                &mut tx,
                run_id,
                OutboxType::Interrupt,
                &json!({}),
                "interrupt:limits",
            )
            .await?;
            changed += 1;
        }
        tx.commit().await?;
    }
    Ok(changed)
}

pub async fn tick_pending_dispatch(worker: &Worker) -> Result<usize> {
    let mut conn = worker.pool.acquire().await?;
    let tenants = tenants_with_pending_tasks(&mut conn).await?;
    drop(conn);
    let mut dispatched = 0;
    for tenant_id in tenants {
        let mut tx = TenantTx::begin(&worker.pool, tenant_id).await?;
        dispatched += db::dispatch_pending_tasks(&mut tx, worker.now())
            .await?
            .len();
        tx.commit().await?;
    }
    Ok(dispatched)
}

pub async fn tick_workspaces(worker: &Worker) -> Result<usize> {
    let mut progressed = 0;
    while start_one(worker).await? {
        progressed += 1;
    }
    while stop_one(worker).await? {
        progressed += 1;
    }
    Ok(progressed)
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

async fn start_one(worker: &Worker) -> Result<bool> {
    let mut tx = worker.begin().await?;
    let Some(row) = sqlx::query(
        "SELECT r.id, r.tenant_id, r.task_id, r.status, r.agent_session_id, r.record_s3_key, r.turns \
         FROM sanchoris.runs r JOIN sanchoris.tasks t ON t.id = r.task_id \
         WHERE r.status IN ('creating', 'idle') \
           AND t.status NOT IN ('SUCCEEDED', 'FAILED', 'ABORTED') \
           AND NOT EXISTS (SELECT 1 FROM sanchoris.workspaces w WHERE w.run_id = r.id AND w.stopped_at IS NULL) \
           AND (r.status = 'creating' OR EXISTS (SELECT 1 FROM sanchoris.run_outbox o \
                WHERE o.run_id = r.id AND o.delivered_at IS NULL AND o.type IN ('user_message', 'exec'))) \
         ORDER BY r.started_at, r.id LIMIT 1 FOR UPDATE OF r SKIP LOCKED",
    )
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(false);
    };
    let run_id: Uuid = row.try_get("id")?;
    let task_id: Uuid = row.try_get("task_id")?;
    let mut tx = adopt(tx, row.try_get("tenant_id")?).await?;
    let open = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM sanchoris.workspaces WHERE run_id = $1 AND stopped_at IS NULL",
    )
    .bind(run_id)
    .fetch_one(tx.conn())
    .await?;
    if open > 0 {
        tx.commit().await?;
        return Ok(true);
    }
    let task = lock_task(&mut tx, task_id).await?;
    let session = session_of(&task)?;
    let start = start_of(&task)?;
    let repositories = match &session.checkout {
        sanchoris_config::Checkout::Repos(repos) => repos.clone(),
        sanchoris_config::Checkout::Project(project) => sqlx::query_scalar::<_, String>(
            "SELECT r.url FROM sanchoris.tasks t \
             JOIN sanchoris.triggers tr ON tr.id = t.trigger_id \
             JOIN sanchoris.projects p ON p.config_id = tr.config_id \
             JOIN sanchoris.repositories r ON r.project_id = p.id \
             WHERE t.id = $1 AND p.name = $2 ORDER BY r.name",
        )
        .bind(task.id)
        .bind(project)
        .fetch_all(tx.conn())
        .await?
        .into_iter()
        .map(|url| url.trim_start_matches("github.com/").to_string())
        .collect(),
    };
    let branch = task_branch(task.id);
    let handle = worker
        .deps
        .runner
        .start_workspace(&WorkspaceSpec {
            run_id,
            tenant_id: tx.tenant_id(),
            task_id: task.id,
            branch: branch.clone(),
            compute: session.workspace.compute.as_str().to_string(),
            repositories: repositories.clone(),
        })
        .await?;
    let workspace_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO sanchoris.workspaces (tenant_id, run_id, compute, platform, platform_ref, repositories, branch, started_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(tx.tenant_id())
    .bind(run_id)
    .bind(session.workspace.compute.as_str())
    .bind(handle.platform)
    .bind(&handle.platform_ref)
    .bind(json!(repositories))
    .bind(&branch)
    .bind(worker.now())
    .fetch_one(tx.conn())
    .await?;
    sqlx::query(
        "UPDATE sanchoris.run_outbox SET delivered_at = $2 \
         WHERE run_id = $1 AND delivered_at IS NULL AND type IN ('start', 'shutdown')",
    )
    .bind(run_id)
    .bind(worker.now())
    .execute(tx.conn())
    .await?;
    let status: String = row.try_get("status")?;
    let resume = match (
        status.as_str(),
        row.try_get::<Option<String>, _>("record_s3_key")?,
    ) {
        ("idle", Some(key)) => json!({
            "sessionId": row.try_get::<Option<String>, _>("agent_session_id")?,
            "recordKey": key,
            "lastTurn": row.try_get::<i32, _>("turns")?,
        }),
        _ => Value::Null,
    };
    let start_payload = sqlx::query_scalar::<_, Option<Value>>(
        "SELECT start_payload FROM sanchoris.tasks WHERE id = $1",
    )
    .bind(task.id)
    .fetch_one(tx.conn())
    .await?
    .unwrap_or(Value::Null);
    let first = status == "creating";
    let payload = json!({
        "agent": task.definition.session.definition["Worker"]["Agent"],
        "model": task.definition.session.definition["Worker"]["Model"],
        "systemPrompt": start_payload["systemPrompt"],
        "skills": skills_payload(&task.definition, &start.required_skills)?,
        "instruction": if first { start_payload["instruction"].clone() } else { Value::Null },
        "resume": resume,
        "heartbeatSeconds": start.limits.heartbeat_seconds,
        "branch": branch,
        "repositories": repositories,
    });
    enqueue_outbox(
        &mut tx,
        run_id,
        OutboxType::Start,
        &payload,
        &format!("start:{workspace_id}"),
    )
    .await?;
    sqlx::query("UPDATE sanchoris.runs SET status = 'creating', updated_at = now() WHERE id = $1")
        .bind(run_id)
        .execute(tx.conn())
        .await?;
    worker.faults.check("workspace_start.before_commit")?;
    tx.commit().await?;
    Ok(true)
}

async fn stop_one(worker: &Worker) -> Result<bool> {
    let now = worker.now();
    let mut tx = worker.begin().await?;
    let Some(row) = sqlx::query(
        "SELECT w.id, w.tenant_id, w.run_id, w.platform_ref, w.compute, w.started_at, r.status AS run_status \
         FROM sanchoris.workspaces w JOIN sanchoris.runs r ON r.id = w.run_id \
         JOIN sanchoris.tasks t ON t.id = r.task_id \
         WHERE w.stopped_at IS NULL AND ( \
           r.status IN ('completed', 'failed') \
           OR (t.status IN ('WAITING', 'NEEDS_YOU') \
               AND t.updated_at + make_interval(secs => (t.definition #>> '{Session,Definition,Workspace,IdleSeconds}')::double precision) <= $1 \
               AND NOT EXISTS (SELECT 1 FROM sanchoris.run_outbox o WHERE o.run_id = r.id \
                   AND o.delivered_at IS NULL AND o.type IN ('user_message', 'exec')))) \
         ORDER BY w.started_at, w.id LIMIT 1 FOR UPDATE OF w SKIP LOCKED",
    )
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(false);
    };
    let workspace_id: Uuid = row.try_get("id")?;
    let run_id: Uuid = row.try_get("run_id")?;
    let mut tx = adopt(tx, row.try_get("tenant_id")?).await?;
    let platform_ref: Option<String> = row.try_get("platform_ref")?;
    if let Some(platform_ref) = &platform_ref {
        worker
            .deps
            .runner
            .stop_workspace(run_id, platform_ref)
            .await?;
    }
    let started_at: DateTime<Utc> = row.try_get("started_at")?;
    let weight = match row.try_get::<String, _>("compute")?.as_str() {
        "docker" => 3,
        _ => 1,
    };
    let seconds = (now - started_at).num_seconds().max(0) * weight;
    sqlx::query("UPDATE sanchoris.workspaces SET stopped_at = $2 WHERE id = $1")
        .bind(workspace_id)
        .bind(now.max(started_at))
        .execute(tx.conn())
        .await?;
    sqlx::query(
        "UPDATE sanchoris.tenants SET workspace_seconds_this_period = workspace_seconds_this_period + $2, \
         updated_at = now() WHERE id = $1",
    )
    .bind(tx.tenant_id())
    .bind(seconds)
    .execute(tx.conn())
    .await?;
    let run_status: String = row.try_get("run_status")?;
    if !matches!(run_status.as_str(), "completed" | "failed") {
        sqlx::query("UPDATE sanchoris.runs SET status = 'idle', updated_at = now() WHERE id = $1")
            .bind(run_id)
            .execute(tx.conn())
            .await?;
        enqueue_outbox(
            &mut tx,
            run_id,
            OutboxType::Shutdown,
            &json!({ "reason": "the session is idle" }),
            &format!("shutdown:{workspace_id}"),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}
