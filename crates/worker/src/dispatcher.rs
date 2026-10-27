use chrono::{DateTime, SecondsFormat, Utc};
use sanchoris_config::{
    Skill, SkillOrigin, Start, builtin_skill, validate::GITHUB_ACTOR_CONDITIONS,
};
use sanchoris_core::db::{
    self, NewTask, NewTriggerEvent, REF_CHECK_WAIT, REF_TRIGGER, Recorded, TenantTx,
    TriggerEventOutcome, locate,
};
use sanchoris_core::github::CHECK_RUN_COMPLETED;
use sanchoris_core::items::{
    CheckWait, CheckWaitMarker, Config, Guard, InboundEvent, Membership, Pending, Project,
    SessionDef, SkillDef, Task, TaskName, Trigger, uniq,
};
use sanchoris_core::jobs::Job;
use sanchoris_core::keys;
use sanchoris_core::messages::{self, OfferedSkill, StartMessage};
use sanchoris_core::model::{NamedDefinition, TaskDefinition, TaskStatus};
use sanchoris_core::pattern::EventPattern;
use sanchoris_core::store::{Index, SortKey};
use sanchoris_core::tasks::{latest_run, load_task, send_to_task, store_task};
use sanchoris_core::transform::{InputTransformer, render_instruction};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::Worker;
use crate::config_sync::trigger_definition;
use crate::error::{Result, WorkerError};
use crate::schedule::next_fire;

pub const SCHEDULE_SOURCE: &str = "schedule";
pub const SCHEDULED_EVENT: &str = "Scheduled Event";
const TITLE_LIMIT: usize = 80;
const DEPTH_LIMIT: i64 = 100;

pub async fn process_inbound(worker: &Worker, tenant_id: Uuid) -> Result<usize> {
    let mut processed = 0;
    while process_next(worker, tenant_id).await? {
        processed += 1;
    }
    Ok(processed)
}

pub async fn dispatch_tenant(worker: &Worker, tenant_id: Uuid) -> Result<usize> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match process_inbound(worker, tenant_id).await {
            Err(error) if error.is_conflict() && attempt < worker.settings.conflict_attempts => {}
            other => return other,
        }
    }
}

pub async fn dispatch(worker: &Worker, tenant_id: Uuid) -> Result<usize> {
    let mut tx = worker.tx(tenant_id);
    let dispatched = db::dispatch_pending_tasks(&mut tx, worker.now()).await?;
    tx.commit().await?;
    Ok(dispatched.len())
}

async fn process_next(worker: &Worker, tenant_id: Uuid) -> Result<bool> {
    let mut tx = worker.tx(tenant_id);
    let markers = tx
        .query::<Pending>(&keys::inbound_pk(tenant_id), "PEND#")
        .await?;
    let Some(marker) = markers.into_iter().next() else {
        return Ok(false);
    };
    tx.delete(&marker.key)?;
    let Some(mut inbound) = tx
        .get::<InboundEvent>(&marker.target)
        .await?
        .filter(|event| event.processed_at.is_none())
    else {
        tx.commit().await?;
        return Ok(true);
    };
    process(worker, &mut tx, &inbound).await?;
    inbound.processed_at = Some(worker.now());
    tx.put(&inbound)?;
    db::dispatch_pending_tasks(&mut tx, worker.now()).await?;
    worker.faults.check("dispatcher.before_commit")?;
    tx.commit().await?;
    Ok(true)
}

async fn process(worker: &Worker, tx: &mut TenantTx, inbound: &InboundEvent) -> Result<()> {
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
        load_trigger(tx, trigger_id)
            .await?
            .filter(Trigger::is_active)
            .into_iter()
            .collect()
    } else {
        event_triggers(tx).await?
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
    let tenant_id = tx.tenant_id();
    let markers = tx
        .query::<CheckWaitMarker>(&keys::tenant_pk(tenant_id), "CHECKWAIT#")
        .await?;
    for marker in markers
        .into_iter()
        .filter(|m| m.repos.iter().any(|r| r == repo))
    {
        let Some(location) = locate(tx, marker.wait_id, REF_CHECK_WAIT).await? else {
            continue;
        };
        let Some(mut wait) = tx.get::<CheckWait>(&location.target).await? else {
            continue;
        };
        let head = wait.head_shas[repo].as_str();
        if wait.status != "waiting" || head.is_some_and(|head| head != sha) {
            continue;
        }
        wait.next_poll_at = worker.now();
        tx.put(&wait)?;
        tx.emit(Job::CheckPoll {
            tenant_id,
            task_id: wait.task_id,
            wait_id: wait.id,
        });
    }
    Ok(())
}

pub async fn load_trigger(tx: &mut TenantTx, trigger_id: Uuid) -> Result<Option<Trigger>> {
    let Some(location) = locate(tx, trigger_id, REF_TRIGGER).await? else {
        return Ok(None);
    };
    Ok(tx.get::<Trigger>(&location.target).await?)
}

pub async fn all_triggers(tx: &mut TenantTx) -> Result<Vec<Trigger>> {
    let tenant_id = tx.tenant_id();
    let configs = tx
        .query::<Config>(&keys::tenant_pk(tenant_id), "CONFIG#")
        .await?;
    let mut triggers = Vec::new();
    for config in configs {
        triggers.extend(
            tx.query::<Trigger>(&keys::config_pk(tenant_id, config.id), "TRIGGER#")
                .await?,
        );
    }
    triggers.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));
    Ok(triggers)
}

async fn event_triggers(tx: &mut TenantTx) -> Result<Vec<Trigger>> {
    Ok(all_triggers(tx)
        .await?
        .into_iter()
        .filter(|t| t.is_active() && t.event_pattern.is_some())
        .collect())
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
    inbound: &InboundEvent,
    trigger: &Trigger,
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
    inbound: &InboundEvent,
    trigger: &Trigger,
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
    let mut task = load_task(tx, task_id).await?;
    task.start_payload = Some(json!({ "instruction": text, "systemPrompt": system_prompt }));
    tx.put(&task)?;
    Ok(TriggerEventOutcome::Started)
}

async fn place(
    worker: &Worker,
    tx: &mut TenantTx,
    inbound: &InboundEvent,
    trigger: &Trigger,
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
    let key = keys::uniq_key(uniq::GITHUB_LOGIN, &login.to_lowercase());
    let Some(guard) = tx.get::<Guard>(&key).await? else {
        return Ok(None);
    };
    let membership = tx
        .get::<Membership>(&keys::membership_key(guard.target, tx.tenant_id()))
        .await?;
    Ok(membership.map(|m| m.member_id))
}

async fn task_by_name(worker: &Worker, tx: &TenantTx, name: &str) -> Result<Option<Uuid>> {
    let names = worker
        .store
        .query_index::<TaskName>(
            Index::Gsi1,
            &keys::task_name_index(tx.tenant_id(), name),
            SortKey::All,
        )
        .await?;
    Ok(names
        .into_iter()
        .max_by(|a, b| (a.created_at, a.task_id).cmp(&(b.created_at, b.task_id)))
        .map(|n| n.task_id))
}

async fn depth_of(tx: &mut TenantTx, task_id: Uuid) -> Result<i64> {
    let mut depth = 0;
    let mut current = load_task(tx, task_id).await?;
    while let Some(parent) = current.parent_task_id {
        if depth >= DEPTH_LIMIT {
            break;
        }
        let Some(next) = tx
            .get::<Task>(&keys::task_key(tx.tenant_id(), parent))
            .await?
        else {
            break;
        };
        depth += 1;
        current = next;
    }
    Ok(depth)
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
        && let Some(parent_id) = task_by_name(worker, tx, &parent.task_name()).await?
    {
        let limits = start.limits.children;
        let depth = depth_of(tx, parent_id).await? + 1;
        if u64::try_from(depth).expect("depth is positive") > limits.max_depth {
            return Ok(Plan::Done(skipped(format!(
                "children limit: depth {depth} exceeds MaxDepth {}",
                limits.max_depth
            ))));
        }
        let siblings = tx
            .query::<sanchoris_core::items::Child>(
                &keys::task_pk(tx.tenant_id(), parent_id),
                "CHILD#",
            )
            .await?
            .len();
        if u64::try_from(siblings).expect("count fits") >= limits.max_count {
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
        if let Some(id) = task_by_name(worker, tx, &blocker.task_name()).await?
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
    inbound: &InboundEvent,
    trigger: &Trigger,
    now: DateTime<Utc>,
) -> Result<Plan> {
    let detail = &inbound.event["detail"];
    let Some(thread) = detail["thread"].as_str() else {
        return Ok(Plan::Done(discarded("chat events need detail.thread")));
    };
    let text = detail["text"].as_str().unwrap_or("");
    let names = tx
        .query::<TaskName>(
            &keys::trigger_pk(tx.tenant_id(), trigger.id),
            &format!("TASK#{thread}/"),
        )
        .await?;
    let count = names.len();
    let latest = names
        .into_iter()
        .max_by(|a, b| (a.created_at, a.task_id).cmp(&(b.created_at, b.task_id)));
    let requested_by_member_id = detail["member_id"].as_str().and_then(|id| id.parse().ok());
    if let Some(latest) = latest {
        let task = load_task(tx, latest.task_id).await?;
        if !task.status.is_terminal() {
            let Some(run_id) = latest_run(tx, task.id).await? else {
                return Ok(Plan::Done(skipped(format!(
                    "the thread's task {} has not started yet",
                    task.name
                ))));
            };
            let user = detail["user"].as_str().unwrap_or("user");
            let message = messages::reply_message(user, text);
            send_to_task(
                tx,
                task.id,
                run_id,
                &message,
                &format!("chat:{}", inbound.key),
                now,
            )
            .await?;
            let mut task = load_task(tx, task.id).await?;
            if task.status == TaskStatus::NeedsYou {
                task.status = TaskStatus::Running;
                task.error = None;
                task.cause = None;
                task.nudges = 0;
                task.updated_at = now;
                store_task(tx, &task, now).await?;
            }
            return Ok(Plan::Done(discarded(format!(
                "delivered to the active task {} as a message",
                task.name
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
    let tenant_id = tx.tenant_id();
    let Some(session) = tx
        .get::<SessionDef>(&keys::session_key(tenant_id, config_id, name))
        .await?
    else {
        return Ok(None);
    };
    let system_prompt = match session.definition.get("Project").and_then(Value::as_str) {
        Some(project) => tx
            .get::<Project>(&keys::project_key(tenant_id, config_id, project))
            .await?
            .map(|p| p.system_prompt),
        None => None,
    };
    Ok(Some((session.definition, system_prompt)))
}

async fn load_skill(tx: &mut TenantTx, config_id: Uuid, name: &str) -> Result<Option<Skill>> {
    let Some(stored) = tx
        .get::<SkillDef>(&keys::skill_key(tx.tenant_id(), config_id, name))
        .await?
    else {
        return Ok(None);
    };
    let origin = match builtin_skill(name) {
        Some(builtin)
            if builtin.skill_md == stored.skill_md && builtin.skill_yaml == stored.skill_yaml =>
        {
            SkillOrigin::BuiltIn
        }
        _ => SkillOrigin::Repository,
    };
    Skill::from_sources(&stored.skill_md, stored.skill_yaml.as_deref(), origin, name)
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

pub fn schedule_trigger(
    tx: &mut TenantTx,
    trigger: &mut Trigger,
    after: DateTime<Utc>,
) -> Result<()> {
    if let Some(previous) = trigger.schedule_message_id.take() {
        tx.cancel_schedule(previous);
    }
    trigger.next_run_at = None;
    let Some(expression) = trigger
        .schedule_expression
        .clone()
        .filter(|_| trigger.is_active())
    else {
        return Ok(());
    };
    let due_at = next_fire(&expression, after)
        .map_err(|error| WorkerError::Invalid(format!("{expression}: {error}")))?;
    let message_id = tx.emit_at(
        Job::TriggerSchedule {
            tenant_id: tx.tenant_id(),
            config_id: trigger.config_id,
            trigger_id: trigger.id,
            due_at,
        },
        due_at,
    );
    trigger.next_run_at = Some(due_at);
    trigger.schedule_message_id = Some(message_id);
    Ok(())
}

pub async fn fire_schedule(
    worker: &Worker,
    tenant_id: Uuid,
    config_id: Uuid,
    trigger_id: Uuid,
    due_at: DateTime<Utc>,
    message_id: Uuid,
) -> Result<usize> {
    let mut tx = worker.tx(tenant_id);
    let Some(mut trigger) = load_trigger(&mut tx, trigger_id).await? else {
        return Ok(0);
    };
    if trigger.config_id != config_id
        || trigger.schedule_message_id != Some(message_id)
        || !trigger.is_active()
    {
        return Ok(0);
    }
    let time = due_at.to_rfc3339_opts(SecondsFormat::Secs, true);
    let event = json!({
        "source": SCHEDULE_SOURCE,
        "detail-type": SCHEDULED_EVENT,
        "detail": { "trigger_id": trigger_id, "trigger": trigger.name, "time": time },
    });
    sanchoris_core::channel::record_inbound_event(
        &mut tx,
        SCHEDULE_SOURCE,
        &format!("{trigger_id}:{time}"),
        &event,
    )
    .await?;
    trigger.schedule_message_id = None;
    schedule_trigger(&mut tx, &mut trigger, due_at)?;
    tx.put(&trigger)?;
    tx.commit().await?;
    Ok(1)
}
