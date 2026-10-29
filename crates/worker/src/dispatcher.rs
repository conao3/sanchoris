use std::collections::BTreeMap;

use chrono::{DateTime, Duration, DurationRound, SecondsFormat, Utc};
use sanchoris_config::validate::{constrains_actor, github_actor_conditions};
use sanchoris_config::{Checkout, Skill, SkillOrigin, Start, builtin_skill};
use sanchoris_core::audit::{AuditEntry, record_audit};
use sanchoris_core::db::{
    self, DbError, NewTask, NewTriggerEvent, REF_CHECK_WAIT, REF_TRIGGER, Recorded, TenantTx,
    TriggerEventOutcome, locate,
};
use sanchoris_core::github::CHECK_RUN_COMPLETED;
use sanchoris_core::item::{Entity, Key};
use sanchoris_core::items::{
    CheckWait, CheckWaitMarker, Config, Guard, InboundEvent, Membership, Pending, Project,
    Repository, SessionDef, SkillDef, Task, TaskName, Trigger, TriggerRate, github_task_name, uniq,
};
use sanchoris_core::jobs::Job;
use sanchoris_core::keys;
use sanchoris_core::lifecycle::TaskDecision;
use sanchoris_core::messages::{self, OfferedInput, OfferedSkill, StartMessage};
use sanchoris_core::model::{ActorKind, AuditOrigin, NamedDefinition, TaskDefinition, TaskStatus};
use sanchoris_core::pattern::EventPattern;
use sanchoris_core::store::{Index, SortKey, Update};
use sanchoris_core::tasks::{apply_decision, latest_run, load_task, send_to_task};
use sanchoris_core::transform::{InputTransformer, render_instruction};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::Worker;
use crate::config_sync::trigger_definition;
use crate::error::{Result, WorkerError};
use crate::ports::{GitHubError, IssueRelations};
use crate::schedule::next_fire;

pub const SCHEDULE_SOURCE: &str = "schedule";
pub const SCHEDULED_EVENT: &str = "Scheduled Event";
const TITLE_LIMIT: usize = 80;
const DEPTH_LIMIT: i64 = 100;

pub async fn process_inbound(worker: &Worker, tenant_id: Uuid) -> Result<usize> {
    let mut processed = 0;
    loop {
        let mut tx = worker.tx(tenant_id);
        let markers = tx
            .query::<Pending>(&keys::inbound_pk(tenant_id), "PEND#")
            .await?;
        let Some(marker) = markers.into_iter().next() else {
            return Ok(processed);
        };
        process_marker(worker, tenant_id, &marker.key).await?;
        processed += 1;
    }
}

pub async fn dispatch_inbound(
    worker: &Worker,
    tenant_id: Uuid,
    source: &str,
    key: &str,
) -> Result<usize> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match dispatch_inbound_once(worker, tenant_id, source, key).await {
            Err(error) if error.is_conflict() && attempt < worker.settings.conflict_attempts => {}
            other => return other,
        }
    }
}

async fn dispatch_inbound_once(
    worker: &Worker,
    tenant_id: Uuid,
    source: &str,
    key: &str,
) -> Result<usize> {
    let inbound = worker
        .store
        .get::<InboundEvent>(&keys::inbound_event_key(tenant_id, source, key))
        .await?
        .ok_or_else(|| {
            WorkerError::Invalid(format!("inbound event {source}:{key} does not exist"))
        })?;
    if inbound.processed_at.is_some() {
        return Ok(0);
    }
    process_marker(
        worker,
        tenant_id,
        &keys::pending_inbound_key(tenant_id, inbound.received_at, inbound.id),
    )
    .await?;
    Ok(1)
}

pub async fn dispatch(worker: &Worker, tenant_id: Uuid) -> Result<usize> {
    let mut tx = worker.tx(tenant_id);
    let dispatched = db::dispatch_pending_tasks(&mut tx, worker.now()).await?;
    tx.commit().await?;
    Ok(dispatched.len())
}

struct Failure {
    trigger_id: Option<Uuid>,
    error: WorkerError,
}

impl From<WorkerError> for Failure {
    fn from(error: WorkerError) -> Self {
        Self {
            trigger_id: None,
            error,
        }
    }
}

impl From<DbError> for Failure {
    fn from(error: DbError) -> Self {
        WorkerError::from(error).into()
    }
}

fn is_poison(error: &WorkerError) -> bool {
    let transient = match error {
        WorkerError::Db(DbError::Dynamo(_) | DbError::Port(_))
        | WorkerError::Port(_)
        | WorkerError::Runner(_)
        | WorkerError::Injected(_)
        | WorkerError::GitHub(GitHubError::Transport(_)) => true,
        WorkerError::GitHub(GitHubError::Status { status, .. }) => *status == 429 || *status >= 500,
        _ => error.is_conflict(),
    };
    !transient
}

async fn process_marker(worker: &Worker, tenant_id: Uuid, marker_key: &Key) -> Result<()> {
    let mut failed = BTreeMap::new();
    loop {
        let error = match process_once(worker, tenant_id, marker_key, &failed).await {
            Ok(()) => return Ok(()),
            Err(Failure {
                trigger_id: Some(trigger_id),
                error,
            }) if is_poison(&error) && !failed.contains_key(&trigger_id) => {
                failed.insert(trigger_id, error.to_string());
                continue;
            }
            Err(Failure { error, .. }) => error,
        };
        if !is_poison(&error) {
            return Err(error);
        }
        return discard_inbound(worker, tenant_id, marker_key, &error.to_string()).await;
    }
}

async fn process_once(
    worker: &Worker,
    tenant_id: Uuid,
    marker_key: &Key,
    failed: &BTreeMap<Uuid, String>,
) -> std::result::Result<(), Failure> {
    let mut tx = worker.tx(tenant_id);
    let Some(marker) = tx.get::<Pending>(marker_key).await? else {
        return Ok(());
    };
    tx.delete(&marker.key)?;
    let Some(mut inbound) = tx
        .get::<InboundEvent>(&marker.target)
        .await?
        .filter(|event| event.processed_at.is_none())
    else {
        tx.commit().await?;
        return Ok(());
    };
    process(worker, &mut tx, &inbound, failed).await?;
    inbound.processed_at = Some(worker.now());
    tx.put(&inbound)?;
    db::dispatch_pending_tasks(&mut tx, worker.now()).await?;
    worker.faults.check("dispatcher.before_commit")?;
    tx.commit().await?;
    Ok(())
}

async fn discard_inbound(
    worker: &Worker,
    tenant_id: Uuid,
    marker_key: &Key,
    reason: &str,
) -> Result<()> {
    let mut tx = worker.tx(tenant_id);
    let Some(marker) = tx.get::<Pending>(marker_key).await? else {
        return Ok(());
    };
    tx.delete(&marker.key)?;
    if let Some(mut inbound) = tx
        .get::<InboundEvent>(&marker.target)
        .await?
        .filter(|event| event.processed_at.is_none())
    {
        inbound.processed_at = Some(worker.now());
        inbound.error = Some(reason.to_string());
        tx.put(&inbound)?;
    }
    tx.commit().await?;
    Ok(())
}

async fn process(
    worker: &Worker,
    tx: &mut TenantTx,
    inbound: &InboundEvent,
    failed: &BTreeMap<Uuid, String>,
) -> std::result::Result<(), Failure> {
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
            .map(|trigger| (trigger, inbound.event.clone()))
            .collect()
    } else {
        let mut matched = Vec::new();
        for (trigger, config) in event_triggers(tx).await? {
            let view = repository_view(&inbound.event, &config)?;
            matched.push((trigger, view));
        }
        matched
    };
    for (trigger, view) in triggers {
        match failed.get(&trigger.id) {
            Some(reason) => record_failed(tx, inbound, &trigger, reason).await?,
            None => evaluate(worker, tx, inbound, &view, &trigger)
                .await
                .map_err(|error| Failure {
                    trigger_id: Some(trigger.id),
                    error,
                })?,
        }
    }
    Ok(())
}

async fn record_failed(
    tx: &mut TenantTx,
    inbound: &InboundEvent,
    trigger: &Trigger,
    reason: &str,
) -> Result<()> {
    let recorded = db::record_trigger_event(
        tx,
        &NewTriggerEvent {
            trigger_id: trigger.id,
            key: &inbound.key,
            event: &inbound.event,
        },
    )
    .await?;
    if let Recorded::New(trigger_event_id) = recorded {
        db::set_trigger_event_outcome(
            tx,
            trigger_event_id,
            &discarded(format!("the trigger failed on this event: {reason}")),
        )
        .await?;
    }
    Ok(())
}

async fn nudge_check_waits(worker: &Worker, tx: &mut TenantTx, event: &Value) -> Result<()> {
    let detail = &event["detail"];
    let (Some(repository_id), Some(sha)) = (
        detail["repository_id"].as_i64(),
        detail["check_run"]["head_sha"].as_str(),
    ) else {
        return Ok(());
    };
    let tenant_id = tx.tenant_id();
    let markers = tx
        .query::<CheckWaitMarker>(&keys::tenant_pk(tenant_id), "CHECKWAIT#")
        .await?;
    for marker in markers {
        let Some(repo) = marker
            .repository_ids
            .iter()
            .position(|id| *id == repository_id)
            .and_then(|index| marker.repos.get(index))
        else {
            continue;
        };
        let Some(location) = locate(tx, marker.wait_id, REF_CHECK_WAIT).await? else {
            continue;
        };
        let Some(mut wait) = tx.get::<CheckWait>(&location.target).await? else {
            continue;
        };
        let head = wait.head_shas[repo.as_str()].as_str();
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
    let Some(trigger) = tx.get::<Trigger>(&location.target).await? else {
        return Ok(None);
    };
    let config_key = keys::config_key(tx.tenant_id(), trigger.config_id);
    let config = tx.get::<Config>(&config_key).await?;
    if config.is_some() {
        tx.assert_unchanged(&config_key)?;
    }
    Ok(config
        .filter(Config::is_active)
        .and_then(|config| trigger.in_revision(config.active_revision_id)))
}

async fn event_triggers(tx: &mut TenantTx) -> Result<Vec<(Trigger, Config)>> {
    let tenant_id = tx.tenant_id();
    let configs = tx
        .query::<Config>(&keys::tenant_pk(tenant_id), "CONFIG#")
        .await?;
    let mut triggers = Vec::new();
    for config in configs.into_iter().filter(Config::is_active) {
        tx.assert_unchanged(&config.key())?;
        for trigger in tx
            .query::<Trigger>(&keys::config_pk(tenant_id, config.id), "TRIGGER#")
            .await?
            .iter()
            .filter_map(|trigger| trigger.in_revision(config.active_revision_id))
        {
            if trigger.is_active() && trigger.event_pattern.is_some() {
                triggers.push((trigger, config.clone()));
            }
        }
    }
    triggers.sort_by(|(a, _), (b, _)| (&a.name, a.id).cmp(&(&b.name, b.id)));
    Ok(triggers)
}

pub fn repository_view(event: &Value, config: &Config) -> Result<Value> {
    if event["source"] != "github" {
        return Ok(event.clone());
    }
    let detail = &event["detail"];
    let Some(name) = detail["repository"].as_str() else {
        return Ok(event.clone());
    };
    let id = detail["repository_id"]
        .as_i64()
        .ok_or_else(|| WorkerError::Invalid("GitHub events carry detail.repository_id".into()))?;
    let seen = match config.repositories.iter().find(|(_, r)| r.id == id) {
        Some((named, _)) => named.clone(),
        None if config.repository_named(name).is_some() => format!("#{id}"),
        None => name.to_string(),
    };
    let mut view = event.clone();
    view["detail"]["repository"] = json!(seen);
    Ok(view)
}

fn without_actor(pattern: &Value, detail_type: &str) -> Value {
    let mut pattern = pattern.clone();
    let Some(Value::Object(detail)) = pattern.get_mut("detail") else {
        return pattern;
    };
    for path in github_actor_conditions(detail_type) {
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
    match detail["comment"]["author_association"].as_str() {
        Some(association) => {
            format!("the actor {login} ({association}) is not allowed by the trigger")
        }
        None => format!("the actor {login} is not allowed by the trigger"),
    }
}

fn unconstrained_actor(pattern: &Value, event: &Value) -> Option<String> {
    let detail_type = event["detail-type"].as_str()?;
    if event["source"] != "github" || github_actor_conditions(detail_type).is_empty() {
        return None;
    }
    (!constrains_actor(&pattern["detail"], detail_type))
        .then(|| format!("the trigger does not constrain the actor of {detail_type} events"))
}

async fn evaluate(
    worker: &Worker,
    tx: &mut TenantTx,
    inbound: &InboundEvent,
    view: &Value,
    trigger: &Trigger,
) -> Result<()> {
    let actor_check = match &trigger.event_pattern {
        Some(raw) => {
            let detail_type = inbound.event["detail-type"].as_str().unwrap_or("");
            let full = EventPattern::parse(raw)
                .map_err(|e| WorkerError::Invalid(format!("trigger {}: {e}", trigger.name)))?;
            let relevant = EventPattern::parse(&without_actor(raw, detail_type))
                .map_err(|e| WorkerError::Invalid(format!("trigger {}: {e}", trigger.name)))?;
            if !relevant.matches(view) {
                return Ok(());
            }
            match unconstrained_actor(raw, &inbound.event) {
                Some(reason) => Some(skipped(reason)),
                None => (!full.matches(view)).then(|| skipped(actor_reason(&inbound.event))),
            }
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
    let outcome = match actor_check {
        Some(outcome) => outcome,
        None => start(worker, tx, inbound, trigger, trigger_event_id).await?,
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
    display: String,
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
    let Some((session_definition, system_prompt)) = load_session(
        tx,
        trigger.config_id,
        trigger.config_revision_id,
        &start.session,
    )
    .await?
    else {
        return Ok(discarded(format!(
            "session {} does not exist",
            start.session
        )));
    };
    let mut skills = Vec::new();
    let mut loaded = Vec::new();
    for name in &start.skills {
        let Some(skill) =
            load_skill(tx, trigger.config_id, trigger.config_revision_id, name).await?
        else {
            return Ok(discarded(format!("skill {name} does not exist")));
        };
        skills.push(NamedDefinition {
            name: name.clone(),
            definition: serde_json::to_value(&skill)?,
        });
        loaded.push(skill);
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
            definition: session_definition.clone(),
        },
        skills,
    };
    let name_key = keys::task_name_key(tx.tenant_id(), trigger.id, &placement.name);
    if tx.get::<TaskName>(&name_key).await?.is_some() {
        return Ok(discarded(format!(
            "task {} already exists",
            placement.display
        )));
    }
    if let Some(outcome) = admit_start(worker, tx, trigger).await? {
        return Ok(outcome);
    }
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
        return Ok(discarded(format!(
            "task {} already exists",
            placement.display
        )));
    };
    let known = known_inputs(tx, trigger, &session_definition, &inbound.event).await?;
    let offered = loaded
        .iter()
        .map(|skill| OfferedSkill {
            name: skill.name.clone(),
            required: start.required_skills.contains(&skill.name),
            inputs: skill
                .definition
                .iter()
                .flat_map(|definition| &definition.inputs)
                .map(|(name, kind)| {
                    let kind = serde_json::to_value(kind)
                        .ok()
                        .and_then(|kind| kind.as_str().map(str::to_string))
                        .unwrap_or_default();
                    OfferedInput {
                        value: known
                            .get(name.as_str())
                            .filter(|(expected, _)| *expected == kind)
                            .map(|(_, value)| value.clone()),
                        name: name.clone(),
                        kind,
                    }
                })
                .collect(),
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

async fn known_inputs(
    tx: &mut TenantTx,
    trigger: &Trigger,
    session_definition: &Value,
    event: &Value,
) -> Result<BTreeMap<&'static str, (String, Value)>> {
    let mut known = BTreeMap::new();
    let session: sanchoris_config::Session = serde_json::from_value(session_definition.clone())?;
    let repositories = match session.checkout {
        Checkout::Repos(names) => {
            let config = tx
                .require::<Config>(&keys::config_key(tx.tenant_id(), trigger.config_id))
                .await?;
            names
                .iter()
                .map(|name| {
                    config
                        .repository_named(name)
                        .map_or_else(|| name.clone(), |r| r.full_name.clone())
                })
                .collect::<Vec<_>>()
        }
        Checkout::Project(project) => {
            let mut rows = tx
                .query::<Repository>(
                    &keys::config_pk(tx.tenant_id(), trigger.config_id),
                    &keys::project_repositories_prefix(trigger.config_revision_id, &project),
                )
                .await?;
            rows.sort_by(|a, b| a.name.cmp(&b.name));
            rows.into_iter().map(|r| r.full_name).collect()
        }
    };
    known.insert("Repos", ("array".to_string(), json!(repositories)));
    let detail = &event["detail"];
    if event["source"] == "github"
        && let (Some(repo), Some(number)) = (
            detail["repository"].as_str(),
            detail["issue"]["number"].as_i64(),
        )
    {
        known.insert("Repo", ("string".to_string(), json!(repo)));
        known.insert("Issue", ("number".to_string(), json!(number)));
    }
    Ok(known)
}

async fn admit_start(
    worker: &Worker,
    tx: &mut TenantTx,
    trigger: &Trigger,
) -> Result<Option<TriggerEventOutcome>> {
    let now = worker.now();
    let window_start = now
        .duration_trunc(Duration::hours(1))
        .expect("an hour divides the time");
    let limit = worker.settings.trigger_starts_per_hour;
    let key = keys::trigger_rate_key(tx.tenant_id(), trigger.id, window_start);
    let Some(rate) = tx.get::<TriggerRate>(&key).await? else {
        tx.insert(&TriggerRate {
            tenant_id: tx.tenant_id(),
            trigger_id: trigger.id,
            window_start,
            starts: 1,
            discarded: 0,
            expires_at: window_start + Duration::days(1),
        })?;
        return Ok(None);
    };
    if rate.starts < limit {
        tx.update(
            &key,
            Update::new().add("starts", 1).cap_after("starts", limit),
        )?;
        return Ok(None);
    }
    tx.update(&key, Update::new().add("discarded", 1))?;
    let window = window_start.to_rfc3339_opts(SecondsFormat::Secs, true);
    let reason = format!(
        "receive limit: trigger {} already started {limit} tasks in the hour from {window}",
        trigger.name
    );
    if rate.discarded == 0 {
        let trigger_ref = trigger.id.to_string();
        record_audit(
            tx,
            &AuditEntry {
                at: now,
                actor_kind: ActorKind::Trigger,
                actor_ref: &trigger_ref,
                action: "trigger.receive_limit",
                target_kind: "trigger",
                target_ref: &trigger_ref,
                summary: &format!("Discarded events over the receive limit: {reason}"),
                origin: AuditOrigin::System,
                source_ip: None,
                metadata: Map::new(),
            },
        )
        .await?;
    }
    Ok(Some(discarded(reason)))
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
                display: format!("{}@{time}", trigger.name),
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

async fn app_sender(worker: &Worker, detail: &Value) -> Result<Option<String>> {
    let sender = &detail["sender"];
    let Some(login) = sender["login"].as_str().filter(|_| sender["type"] == "Bot") else {
        return Ok(None);
    };
    let app = worker.deps.reader.app_login().await?;
    Ok(login.eq_ignore_ascii_case(&app).then_some(app))
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
    let repository_id = detail["repository_id"]
        .as_i64()
        .ok_or_else(|| WorkerError::Invalid("GitHub events carry detail.repository_id".into()))?;
    let channel = json!({ "source": "github", "repository": repo, "repository_id": repository_id });
    let Some(number) = detail["issue"]["number"].as_i64() else {
        let check = &detail["check_run"];
        let id = check["id"].as_i64().unwrap_or_default();
        let suffix = format!("@{}#{id}", check["head_sha"].as_str().unwrap_or(""));
        return Ok(Plan::Create(Placement {
            name: github_task_name(repository_id, &suffix),
            display: format!("{repo}{suffix}"),
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
    let Some(installation) = detail["installation"]["id"].as_i64() else {
        return Ok(Plan::Done(discarded(
            "GitHub events carry detail.installation.id",
        )));
    };
    let relations = if detail["issue"]["is_pull_request"] == true {
        IssueRelations::default()
    } else {
        match worker
            .deps
            .reader
            .issue_relations(installation, repo, number)
            .await
        {
            Ok(relations) => relations,
            Err(error) => {
                return Ok(Plan::Done(skipped(format!(
                    "children limit: the parent and blockers of {repo}#{number} could not be read: {error}"
                ))));
            }
        }
    };
    let parent = match &relations.parent {
        Some(parent) => task_by_name(worker, tx, &parent.task_name())
            .await?
            .map(|id| (format!("{}#{}", parent.repository, parent.number), id)),
        None => None,
    };
    let mut parent_task_id = None;
    match parent {
        Some((parent_name, parent_id)) => {
            let limits = start.limits.children;
            let depth = depth_of(tx, parent_id).await? + 1;
            if u64::try_from(depth).expect("depth is positive") > limits.max_depth {
                return Ok(Plan::Done(skipped(format!(
                    "children limit: depth {depth} exceeds MaxDepth {}",
                    limits.max_depth
                ))));
            }
            let parent_task = load_task(tx, parent_id).await?;
            let siblings = tx
                .query::<sanchoris_core::items::Child>(
                    &keys::task_pk(tx.tenant_id(), parent_id),
                    "CHILD#",
                )
                .await?
                .len();
            if u64::try_from(siblings).expect("count fits") >= limits.max_count {
                return Ok(Plan::Done(skipped(format!(
                    "children limit: {parent_name} already has {siblings} child tasks (MaxCount {})",
                    limits.max_count
                ))));
            }
            tx.put(&parent_task)?;
            parent_task_id = Some(parent_id);
        }
        None => {
            if let Some(bot) = app_sender(worker, detail).await? {
                return Ok(Plan::Done(skipped(format!(
                    "children limit: {bot} started this event on {repo}#{number}, which is not a sub-issue of a task's issue"
                ))));
            }
        }
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
        name: github_task_name(repository_id, &format!("#{number}")),
        display: format!("{repo}#{number}"),
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
            let user = detail["user"].as_str().unwrap_or("user");
            let message = messages::reply_message(user, text);
            let Some(run_id) = latest_run(tx, task.id).await? else {
                let mut task = task;
                let Some(payload) = task.start_payload.as_mut() else {
                    return Ok(Plan::Done(skipped(format!(
                        "the thread's task {} has no start instruction yet",
                        task.name
                    ))));
                };
                let instruction = payload["instruction"].as_str().unwrap_or("");
                payload["instruction"] = json!(format!("{instruction}\n\n{message}"));
                task.updated_at = now;
                tx.put(&task)?;
                return Ok(Plan::Done(discarded(format!(
                    "added to the start instruction of the pending task {}",
                    task.name
                ))));
            };
            let dedupe = format!("chat:{}", inbound.key);
            if task.status == TaskStatus::NeedsYou {
                let decision = TaskDecision::Resume {
                    message,
                    reset_nudges: true,
                };
                apply_decision(tx, task.id, run_id, &decision, &dedupe, now).await?;
            } else {
                send_to_task(tx, task.id, run_id, &message, &dedupe, now).await?;
            }
            return Ok(Plan::Done(discarded(format!(
                "delivered to the active task {} as a message",
                task.name
            ))));
        }
    }
    Ok(Plan::Create(Placement {
        name: format!("{thread}/{}", count + 1),
        display: format!("{thread}/{}", count + 1),
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
    revision_id: Uuid,
    name: &str,
) -> Result<Option<(Value, Option<String>)>> {
    let tenant_id = tx.tenant_id();
    let Some(session) = tx
        .get::<SessionDef>(&keys::session_key(tenant_id, config_id, revision_id, name))
        .await?
    else {
        return Ok(None);
    };
    let system_prompt = match session.definition.get("Project").and_then(Value::as_str) {
        Some(project) => tx
            .get::<Project>(&keys::project_key(
                tenant_id,
                config_id,
                revision_id,
                project,
            ))
            .await?
            .map(|p| p.system_prompt),
        None => None,
    };
    Ok(Some((session.definition, system_prompt)))
}

async fn load_skill(
    tx: &mut TenantTx,
    config_id: Uuid,
    revision_id: Uuid,
    name: &str,
) -> Result<Option<Skill>> {
    let Some(stored) = tx
        .get::<SkillDef>(&keys::skill_key(
            tx.tenant_id(),
            config_id,
            revision_id,
            name,
        ))
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
    schedule_trigger(&mut tx, &mut trigger, due_at.max(worker.now()))?;
    tx.put(&trigger)?;
    tx.commit().await?;
    Ok(1)
}
