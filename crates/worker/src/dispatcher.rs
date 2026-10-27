use chrono::{DateTime, SecondsFormat, Utc};
use sanchoris_config::{
    Skill, SkillOrigin, Start, builtin_skill, validate::GITHUB_ACTOR_CONDITIONS,
};
use sanchoris_core::db::{
    self, NewTask, NewTriggerEvent, Recorded, TenantTx, TriggerEventOutcome, claim_due_jobs,
    complete_due_job,
};
use sanchoris_core::github::CHECK_RUN_COMPLETED;
use sanchoris_core::messages::{self, OfferedSkill, StartMessage};
use sanchoris_core::model::{NamedDefinition, TaskDefinition, TaskStatus};
use sanchoris_core::pattern::EventPattern;
use sanchoris_core::tasks::{latest_run, send_to_task};
use sanchoris_core::transform::{InputTransformer, render_instruction};
use serde_json::{Map, Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::config_sync::{DUE_TRIGGER_SCHEDULE, trigger_definition};
use crate::error::{Result, WorkerError};
use crate::schedule::next_fire;
use crate::{Worker, adopt};

pub const SCHEDULE_SOURCE: &str = "schedule";
pub const SCHEDULED_EVENT: &str = "Scheduled Event";
const TITLE_LIMIT: usize = 80;

struct Inbound {
    id: Uuid,
    source: String,
    key: String,
    event: Value,
}

struct TriggerRow {
    id: Uuid,
    config_id: Uuid,
    config_revision_id: Uuid,
    name: String,
    state: String,
    event_pattern: Option<Value>,
    schedule_expression: Option<String>,
    input_transformer: Option<Value>,
    start: Value,
}

pub async fn tick(worker: &Worker) -> Result<usize> {
    let mut processed = 0;
    while processed < worker.settings.batch {
        if !dispatch_one(worker).await? {
            break;
        }
        processed += 1;
    }
    Ok(usize::try_from(processed).expect("batch fits in usize"))
}

async fn dispatch_one(worker: &Worker) -> Result<bool> {
    let mut tx = worker.begin().await?;
    let Some(row) = sqlx::query(
        "SELECT id, tenant_id, source, key, event FROM sanchoris.inbound_events \
         WHERE processed_at IS NULL ORDER BY received_at, id LIMIT 1 FOR UPDATE SKIP LOCKED",
    )
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(false);
    };
    let inbound = Inbound {
        id: row.try_get("id")?,
        source: row.try_get("source")?,
        key: row.try_get("key")?,
        event: row.try_get("event")?,
    };
    let mut tx = adopt(tx, row.try_get("tenant_id")?).await?;
    process(worker, &mut tx, &inbound).await?;
    sqlx::query("UPDATE sanchoris.inbound_events SET processed_at = $2 WHERE id = $1")
        .bind(inbound.id)
        .bind(worker.now())
        .execute(tx.conn())
        .await?;
    db::dispatch_pending_tasks(&mut tx, worker.now()).await?;
    worker.faults.check("dispatcher.before_commit")?;
    tx.commit().await?;
    Ok(true)
}

async fn process(worker: &Worker, tx: &mut TenantTx, inbound: &Inbound) -> Result<()> {
    if inbound.event["detail-type"] == CHECK_RUN_COMPLETED {
        nudge_check_waits(worker, tx, &inbound.event).await?;
    }
    let triggers = if inbound.source == SCHEDULE_SOURCE {
        let trigger_id: Uuid = inbound.event["detail"]["trigger_id"]
            .as_str()
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| {
                WorkerError::Invalid("a schedule event needs detail.trigger_id".into())
            })?;
        load_triggers(tx, Some(trigger_id)).await?
    } else {
        load_triggers(tx, None).await?
    };
    for trigger in triggers {
        evaluate(worker, tx, inbound, &trigger).await?;
    }
    Ok(())
}

async fn nudge_check_waits(worker: &Worker, tx: &mut TenantTx, event: &Value) -> Result<()> {
    let detail = &event["detail"];
    let (Some(repo), Some(sha)) = (
        detail["repository"].as_str(),
        detail["check_run"]["head_sha"].as_str(),
    ) else {
        return Ok(());
    };
    sqlx::query(
        "UPDATE sanchoris.check_waits SET next_poll_at = $4 \
         WHERE tenant_id = $1 AND status = 'waiting' AND repos ? $2 \
           AND (head_shas->>$2 IS NULL OR head_shas->>$2 = $3)",
    )
    .bind(tx.tenant_id())
    .bind(repo)
    .bind(sha)
    .bind(worker.now())
    .execute(tx.conn())
    .await?;
    Ok(())
}

async fn load_triggers(tx: &mut TenantTx, only: Option<Uuid>) -> Result<Vec<TriggerRow>> {
    let rows = sqlx::query(
        "SELECT id, config_id, config_revision_id, name, state, event_pattern, schedule_expression, input_transformer, start \
         FROM sanchoris.triggers \
         WHERE tenant_id = $1 AND state = 'ENABLED' AND removed_at IS NULL \
           AND (($2::uuid IS NULL AND event_pattern IS NOT NULL) OR id = $2) \
         ORDER BY name",
    )
    .bind(tx.tenant_id())
    .bind(only)
    .fetch_all(tx.conn())
    .await?;
    rows.iter()
        .map(|row| {
            Ok(TriggerRow {
                id: row.try_get("id")?,
                config_id: row.try_get("config_id")?,
                config_revision_id: row.try_get("config_revision_id")?,
                name: row.try_get("name")?,
                state: row.try_get("state")?,
                event_pattern: row.try_get("event_pattern")?,
                schedule_expression: row.try_get("schedule_expression")?,
                input_transformer: row.try_get("input_transformer")?,
                start: row.try_get("start")?,
            })
        })
        .collect()
}

fn without_actor(pattern: &Value) -> Value {
    let mut pattern = pattern.clone();
    let Some(Value::Object(detail)) = pattern.get_mut("detail") else {
        return pattern;
    };
    for path in GITHUB_ACTOR_CONDITIONS {
        let [parent, leaf] = path else {
            panic!("actor conditions are two levels deep");
        };
        if let Some(Value::Object(map)) = detail.get_mut(*parent) {
            map.remove(*leaf);
            if map.is_empty() {
                detail.remove(*parent);
            }
        }
    }
    pattern
}

fn actor_reason(event: &Value) -> String {
    let detail = &event["detail"];
    let login = detail["sender"]["login"].as_str().unwrap_or("unknown");
    let association = detail["author_association"].as_str().unwrap_or("NONE");
    format!("the actor {login} ({association}) is not allowed by the trigger")
}

async fn evaluate(
    worker: &Worker,
    tx: &mut TenantTx,
    inbound: &Inbound,
    trigger: &TriggerRow,
) -> Result<()> {
    let pattern = match &trigger.event_pattern {
        Some(raw) => {
            let full = EventPattern::parse(raw)
                .map_err(|e| WorkerError::Invalid(format!("trigger {}: {e}", trigger.name)))?;
            let relevant = EventPattern::parse(&without_actor(raw))
                .map_err(|e| WorkerError::Invalid(format!("trigger {}: {e}", trigger.name)))?;
            if !relevant.matches(&inbound.event) {
                return Ok(());
            }
            Some(full)
        }
        None => None,
    };
    let recorded = db::record_trigger_event(
        tx,
        &NewTriggerEvent {
            trigger_id: trigger.id,
            key: &inbound.key,
            event: &inbound.event,
        },
    )
    .await?;
    let Recorded::New(trigger_event_id) = recorded else {
        return Ok(());
    };
    let outcome = match pattern {
        Some(pattern) if !pattern.matches(&inbound.event) => TriggerEventOutcome::Skipped {
            reason: actor_reason(&inbound.event),
        },
        _ => start(worker, tx, inbound, trigger, trigger_event_id).await?,
    };
    db::set_trigger_event_outcome(tx, trigger_event_id, &outcome).await?;
    Ok(())
}

fn discarded(reason: impl Into<String>) -> TriggerEventOutcome {
    TriggerEventOutcome::Discarded {
        reason: reason.into(),
    }
}

fn skipped(reason: impl Into<String>) -> TriggerEventOutcome {
    TriggerEventOutcome::Skipped {
        reason: reason.into(),
    }
}

fn shorten(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() <= TITLE_LIMIT {
        return line.to_string();
    }
    let mut short = line.chars().take(TITLE_LIMIT - 1).collect::<String>();
    short.push('…');
    short
}

struct Placement {
    name: String,
    title: String,
    parent_task_id: Option<Uuid>,
    blocked_by: Vec<Uuid>,
    channel: Option<Value>,
    requested_by_member_id: Option<Uuid>,
}

enum Plan {
    Create(Placement),
    Done(TriggerEventOutcome),
}

async fn start(
    worker: &Worker,
    tx: &mut TenantTx,
    inbound: &Inbound,
    trigger: &TriggerRow,
    trigger_event_id: Uuid,
) -> Result<TriggerEventOutcome> {
    let start: Start = serde_json::from_value(trigger.start.clone())?;
    let values = match &trigger.input_transformer {
        Some(raw) => {
            match InputTransformer::from_value(raw).and_then(|t| t.apply(&inbound.event)) {
                Ok(transformed) => transformed.values,
                Err(error) => return Ok(discarded(format!("InputTransformer failed: {error}"))),
            }
        }
        None => Map::new(),
    };
    let instruction = match render_instruction(&start.instruction, &values) {
        Ok(text) => text,
        Err(error) => return Ok(discarded(format!("Instruction failed to render: {error}"))),
    };
    let placement = match place(worker, tx, inbound, trigger, &start).await? {
        Plan::Create(placement) => placement,
        Plan::Done(outcome) => return Ok(outcome),
    };
    let Some((session_definition, system_prompt)) =
        load_session(tx, trigger.config_id, &start.session).await?
    else {
        return Ok(discarded(format!(
            "session {} does not exist",
            start.session
        )));
    };
    let mut skills = Vec::new();
    for name in &start.skills {
        let Some(skill) = load_skill(tx, trigger.config_id, name).await? else {
            return Ok(discarded(format!("skill {name} does not exist")));
        };
        skills.push(NamedDefinition {
            name: name.clone(),
            definition: serde_json::to_value(&skill)?,
        });
    }
    let definition = TaskDefinition {
        trigger: NamedDefinition {
            name: trigger.name.clone(),
            definition: trigger_definition(
                &trigger.state,
                trigger.event_pattern.as_ref(),
                trigger.schedule_expression.as_deref(),
                trigger.input_transformer.as_ref(),
                &trigger.start,
            ),
        },
        session: NamedDefinition {
            name: start.session.clone(),
            definition: session_definition,
        },
        skills,
    };
    let recorded = db::create_task(
        tx,
        &NewTask {
            trigger_id: trigger.id,
            trigger_event_id: Some(trigger_event_id),
            config_revision_id: trigger.config_revision_id,
            name: &placement.name,
            title: &placement.title,
            definition: &definition,
            parent_task_id: placement.parent_task_id,
            channel: placement.channel.as_ref(),
            requested_by_member_id: placement.requested_by_member_id,
            blocked_by: &placement.blocked_by,
        },
    )
    .await?;
    let Recorded::New(task_id) = recorded else {
        return Ok(discarded(format!("task {} already exists", placement.name)));
    };
    let offered = start
        .skills
        .iter()
        .map(|name| OfferedSkill {
            name: name.clone(),
            required: start.required_skills.contains(name),
        })
        .collect::<Vec<_>>();
    let text = messages::start_message(&StartMessage {
        trigger_name: &trigger.name,
        task_title: &placement.title,
        instruction: &instruction,
        skills: &offered,
    });
    sqlx::query("UPDATE sanchoris.tasks SET start_payload = $2 WHERE id = $1")
        .bind(task_id)
        .bind(json!({ "instruction": text, "systemPrompt": system_prompt }))
        .execute(tx.conn())
        .await?;
    Ok(TriggerEventOutcome::Started)
}

async fn place(
    worker: &Worker,
    tx: &mut TenantTx,
    inbound: &Inbound,
    trigger: &TriggerRow,
    start: &Start,
) -> Result<Plan> {
    let detail = &inbound.event["detail"];
    match inbound.source.as_str() {
        "github" => place_github(worker, tx, detail, start).await,
        "web" | "slack" => place_chat(tx, inbound, trigger, worker.now()).await,
        SCHEDULE_SOURCE => {
            let time = detail["time"].as_str().unwrap_or(&inbound.key).to_string();
            Ok(Plan::Create(Placement {
                name: format!("{}@{time}", trigger.name),
                title: format!("{} at {time}", trigger.name),
                parent_task_id: None,
                blocked_by: Vec::new(),
                channel: None,
                requested_by_member_id: None,
            }))
        }
        other => Ok(Plan::Done(discarded(format!(
            "unknown event source {other}"
        )))),
    }
}

async fn member_by_github_login(tx: &mut TenantTx, login: &str) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT m.id FROM sanchoris.members m JOIN sanchoris.users u ON u.id = m.user_id \
         WHERE m.tenant_id = $1 AND u.github_login = $2",
    )
    .bind(tx.tenant_id())
    .bind(login)
    .fetch_optional(tx.conn())
    .await?)
}

async fn task_by_name(tx: &mut TenantTx, name: &str) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM sanchoris.tasks WHERE tenant_id = $1 AND name = $2 \
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(tx.tenant_id())
    .bind(name)
    .fetch_optional(tx.conn())
    .await?)
}

async fn depth_of(tx: &mut TenantTx, task_id: Uuid) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "WITH RECURSIVE chain(id, parent, depth) AS ( \
           SELECT id, parent_task_id, 0::bigint FROM sanchoris.tasks WHERE id = $1 \
           UNION ALL \
           SELECT t.id, t.parent_task_id, c.depth + 1 FROM sanchoris.tasks t JOIN chain c ON t.id = c.parent \
             WHERE c.depth < 100) \
         SELECT max(depth) FROM chain",
    )
    .bind(task_id)
    .fetch_one(tx.conn())
    .await?)
}

async fn place_github(
    worker: &Worker,
    tx: &mut TenantTx,
    detail: &Value,
    start: &Start,
) -> Result<Plan> {
    let repo = detail["repository"]
        .as_str()
        .ok_or_else(|| WorkerError::Invalid("GitHub events carry detail.repository".into()))?;
    let requester = match detail["sender"]["login"].as_str() {
        Some(login) => member_by_github_login(tx, login).await?,
        None => None,
    };
    let channel = json!({ "source": "github", "repository": repo });
    let Some(number) = detail["issue"]["number"].as_i64() else {
        let check = &detail["check_run"];
        let id = check["id"].as_i64().unwrap_or_default();
        return Ok(Plan::Create(Placement {
            name: format!("{repo}@{}#{id}", check["head_sha"].as_str().unwrap_or("")),
            title: format!(
                "Check {} {} on {repo}",
                check["name"].as_str().unwrap_or(""),
                check["conclusion"].as_str().unwrap_or("completed")
            ),
            parent_task_id: None,
            blocked_by: Vec::new(),
            channel: Some(channel),
            requested_by_member_id: requester,
        }));
    };
    let installation = detail["installation"]["id"]
        .as_i64()
        .ok_or_else(|| WorkerError::Invalid("GitHub events carry detail.installation.id".into()))?;
    let relations = worker
        .deps
        .reader
        .issue_relations(installation, repo, number)
        .await?;
    let mut parent_task_id = None;
    if let Some(parent) = &relations.parent
        && let Some(parent_id) = task_by_name(tx, &parent.task_name()).await?
    {
        let limits = start.limits.children;
        let depth = depth_of(tx, parent_id).await? + 1;
        if u64::try_from(depth).expect("depth is positive") > limits.max_depth {
            return Ok(Plan::Done(skipped(format!(
                "children limit: depth {depth} exceeds MaxDepth {}",
                limits.max_depth
            ))));
        }
        let siblings = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM sanchoris.tasks WHERE tenant_id = $1 AND parent_task_id = $2",
        )
        .bind(tx.tenant_id())
        .bind(parent_id)
        .fetch_one(tx.conn())
        .await?;
        if u64::try_from(siblings).expect("count is non-negative") >= limits.max_count {
            return Ok(Plan::Done(skipped(format!(
                "children limit: {} already has {siblings} child tasks (MaxCount {})",
                parent.task_name(),
                limits.max_count
            ))));
        }
        parent_task_id = Some(parent_id);
    }
    let mut blocked_by = Vec::new();
    for blocker in &relations.blocked_by {
        if let Some(id) = task_by_name(tx, &blocker.task_name()).await?
            && !blocked_by.contains(&id)
        {
            blocked_by.push(id);
        }
    }
    let title = detail["issue"]["title"].as_str().unwrap_or("");
    Ok(Plan::Create(Placement {
        name: format!("{repo}#{number}"),
        title: format!("{title} ({repo}#{number})"),
        parent_task_id,
        blocked_by,
        channel: Some(channel),
        requested_by_member_id: requester,
    }))
}

async fn place_chat(
    tx: &mut TenantTx,
    inbound: &Inbound,
    trigger: &TriggerRow,
    now: DateTime<Utc>,
) -> Result<Plan> {
    let detail = &inbound.event["detail"];
    let Some(thread) = detail["thread"].as_str() else {
        return Ok(Plan::Done(discarded("chat events need detail.thread")));
    };
    let text = detail["text"].as_str().unwrap_or("");
    let latest = sqlx::query(
        "SELECT id, name, status FROM sanchoris.tasks \
         WHERE trigger_id = $1 AND starts_with(name, $2 || '/') \
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(trigger.id)
    .bind(thread)
    .fetch_optional(tx.conn())
    .await?;
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM sanchoris.tasks WHERE trigger_id = $1 AND starts_with(name, $2 || '/')",
    )
    .bind(trigger.id)
    .bind(thread)
    .fetch_one(tx.conn())
    .await?;
    let requested_by_member_id = detail["member_id"].as_str().and_then(|id| id.parse().ok());
    if let Some(row) = latest {
        let task_id: Uuid = row.try_get("id")?;
        let name: String = row.try_get("name")?;
        let status: TaskStatus = row
            .try_get::<String, _>("status")?
            .parse()
            .expect("task status is valid");
        if !status.is_terminal() {
            let Some(run_id) = latest_run(tx, task_id).await? else {
                return Ok(Plan::Done(skipped(format!(
                    "the thread's task {name} has not started yet"
                ))));
            };
            let user = detail["user"].as_str().unwrap_or("user");
            let message = messages::reply_message(user, text);
            send_to_task(
                tx,
                task_id,
                run_id,
                &message,
                &format!("chat:{}", inbound.key),
                now,
            )
            .await?;
            sqlx::query(
                "UPDATE sanchoris.tasks SET status = 'RUNNING', error = NULL, cause = NULL, nudges = 0, \
                 updated_at = $2 WHERE id = $1 AND status = 'NEEDS_YOU'",
            )
            .bind(task_id)
            .bind(now)
            .execute(tx.conn())
            .await?;
            return Ok(Plan::Done(discarded(format!(
                "delivered to the active task {name} as a message"
            ))));
        }
    }
    Ok(Plan::Create(Placement {
        name: format!("{thread}/{}", count + 1),
        title: shorten(text),
        parent_task_id: None,
        blocked_by: Vec::new(),
        channel: Some(
            json!({ "source": inbound.source, "thread": thread, "channel": detail.get("channel") }),
        ),
        requested_by_member_id,
    }))
}

async fn load_session(
    tx: &mut TenantTx,
    config_id: Uuid,
    name: &str,
) -> Result<Option<(Value, Option<String>)>> {
    let Some(definition) = sqlx::query_scalar::<_, Value>(
        "SELECT definition FROM sanchoris.sessions WHERE config_id = $1 AND name = $2",
    )
    .bind(config_id)
    .bind(name)
    .fetch_optional(tx.conn())
    .await?
    else {
        return Ok(None);
    };
    let system_prompt =
        match definition.get("Project").and_then(Value::as_str) {
            Some(project) => sqlx::query_scalar::<_, String>(
                "SELECT system_prompt FROM sanchoris.projects WHERE config_id = $1 AND name = $2",
            )
            .bind(config_id)
            .bind(project)
            .fetch_optional(tx.conn())
            .await?,
            None => None,
        };
    Ok(Some((definition, system_prompt)))
}

async fn load_skill(tx: &mut TenantTx, config_id: Uuid, name: &str) -> Result<Option<Skill>> {
    let Some(row) = sqlx::query(
        "SELECT skill_md, skill_yaml FROM sanchoris.skills WHERE config_id = $1 AND name = $2",
    )
    .bind(config_id)
    .bind(name)
    .fetch_optional(tx.conn())
    .await?
    else {
        return Ok(None);
    };
    let skill_md: String = row.try_get("skill_md")?;
    let skill_yaml: Option<String> = row.try_get("skill_yaml")?;
    let origin = match builtin_skill(name) {
        Some(builtin) if builtin.skill_md == skill_md && builtin.skill_yaml == skill_yaml => {
            SkillOrigin::BuiltIn
        }
        _ => SkillOrigin::Repository,
    };
    Skill::from_sources(&skill_md, skill_yaml.as_deref(), origin, name)
        .map(Some)
        .map_err(|errors| {
            WorkerError::Invalid(format!(
                "stored skill {name} no longer parses: {}",
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            ))
        })
}

pub async fn tick_schedules(worker: &Worker) -> Result<usize> {
    let now = worker.now();
    let mut tx = worker.begin().await?;
    let unscheduled = sqlx::query(
        "SELECT t.id, t.tenant_id, t.schedule_expression FROM sanchoris.triggers t \
         WHERE t.schedule_expression IS NOT NULL AND t.state = 'ENABLED' AND t.removed_at IS NULL \
           AND NOT EXISTS (SELECT 1 FROM sanchoris.due_jobs j WHERE j.kind = $1 AND j.ref_id = t.id)",
    )
    .bind(DUE_TRIGGER_SCHEDULE)
    .fetch_all(&mut *tx)
    .await?;
    let mut progressed = 0;
    for row in &unscheduled {
        let expression: String = row.try_get("schedule_expression")?;
        let due_at = next_fire(&expression, now)
            .map_err(|error| WorkerError::Invalid(format!("{expression}: {error}")))?;
        let inserted = sqlx::query(
            "INSERT INTO sanchoris.due_jobs (tenant_id, kind, ref_id, due_at) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (kind, ref_id) DO NOTHING",
        )
        .bind(row.try_get::<Uuid, _>("tenant_id")?)
        .bind(DUE_TRIGGER_SCHEDULE)
        .bind(row.try_get::<Uuid, _>("id")?)
        .bind(due_at)
        .execute(&mut *tx)
        .await?;
        progressed += usize::try_from(inserted.rows_affected()).expect("small count");
    }
    tx.commit().await?;
    loop {
        let mut tx = worker.begin().await?;
        let Some(job) = claim_due_jobs(&mut tx, DUE_TRIGGER_SCHEDULE, now, 1)
            .await?
            .into_iter()
            .next()
        else {
            break;
        };
        let mut tx = adopt(tx, job.tenant_id).await?;
        fire_schedule(&mut tx, &job.ref_id, job.id, job.due_at).await?;
        tx.commit().await?;
        progressed += 1;
    }
    Ok(progressed)
}

async fn fire_schedule(
    tx: &mut TenantTx,
    trigger_id: &Uuid,
    job_id: Uuid,
    due_at: DateTime<Utc>,
) -> Result<()> {
    let trigger = sqlx::query(
        "SELECT name, schedule_expression FROM sanchoris.triggers \
         WHERE id = $1 AND state = 'ENABLED' AND removed_at IS NULL AND schedule_expression IS NOT NULL",
    )
    .bind(trigger_id)
    .fetch_optional(tx.conn())
    .await?;
    let Some(trigger) = trigger else {
        complete_due_job(tx.conn(), job_id).await?;
        return Ok(());
    };
    let expression: String = trigger.try_get("schedule_expression")?;
    let name: String = trigger.try_get("name")?;
    let time = due_at.to_rfc3339_opts(SecondsFormat::Secs, true);
    let event = json!({
        "source": SCHEDULE_SOURCE,
        "detail-type": SCHEDULED_EVENT,
        "detail": { "trigger_id": trigger_id, "trigger": name, "time": time },
    });
    sanchoris_core::channel::record_inbound_event(
        tx,
        SCHEDULE_SOURCE,
        &format!("{trigger_id}:{time}"),
        &event,
    )
    .await?;
    let next = next_fire(&expression, due_at)
        .map_err(|error| WorkerError::Invalid(format!("{expression}: {error}")))?;
    sqlx::query("UPDATE sanchoris.due_jobs SET due_at = $2, attempts = 0 WHERE id = $1")
        .bind(job_id)
        .bind(next)
        .execute(tx.conn())
        .await?;
    Ok(())
}
