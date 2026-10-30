use async_graphql::{Context, ID, Object, Result};
use chrono::{DateTime, Duration, Utc};
use sanchoris_config::{Limits, Start};
use sanchoris_core::db::{self, REF_TRIGGER, REF_TRIGGER_EVENT};
use sanchoris_core::item::format_time;
use sanchoris_core::items::{Task as TaskItem, Trigger as TriggerItem, TriggerEvent as EventItem};
use sanchoris_core::keys;
use sanchoris_core::store::{Index, Listing, SortKey};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::config::{ChangeRequest, Config, Session, active_triggers, config_rows, edit_url};
use crate::context::{Scope, now, tx};
use crate::enums::{
    ConfigObjectKind, EventSource, TriggerEventKind, TriggerEventReason, TriggerEventResult,
    TriggerState,
};
use crate::error::not_found;
use crate::scalars::{Timestamp, ts, ts_opt};
use crate::skill::Skill;
use crate::task::{Task, latest_tasks};

pub(crate) struct Trigger {
    pub scope: Scope,
    pub row: TriggerItem,
}

impl Trigger {
    pub async fn all(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        let mut rows = Vec::new();
        for config in config_rows(ctx, scope).await? {
            rows.extend(active_triggers(ctx, scope, &config).await?);
        }
        rows.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));
        Ok(rows.into_iter().map(|row| Trigger { scope, row }).collect())
    }

    pub async fn load(ctx: &Context<'_>, scope: Scope, trigger_id: Uuid) -> Result<Self> {
        let mut tx = tx(ctx, scope.tenant_id);
        let location = db::locate(&mut tx, trigger_id, REF_TRIGGER)
            .await?
            .ok_or_else(|| not_found(format!("trigger {trigger_id}")))?;
        let row = tx.require::<TriggerItem>(&location.target).await?;
        let config = tx
            .get::<sanchoris_core::items::Config>(&keys::config_key(scope.tenant_id, row.config_id))
            .await?;
        let row = config
            .and_then(|config| row.in_revision(config.active_revision_id))
            .unwrap_or(TriggerItem { next: None, ..row });
        Ok(Trigger { scope, row })
    }

    pub fn start(&self) -> Result<Start> {
        serde_json::from_value(self.row.start.clone()).map_err(|e| {
            async_graphql::Error::new(format!(
                "trigger {} has an invalid Start: {e}",
                self.row.name
            ))
        })
    }

    fn definition(&self) -> Value {
        let mut definition = Map::new();
        definition.insert("State".into(), Value::String(self.row.state.clone()));
        if let Some(pattern) = &self.row.event_pattern {
            definition.insert("EventPattern".into(), pattern.clone());
        }
        if let Some(expression) = &self.row.schedule_expression {
            definition.insert(
                "ScheduleExpression".into(),
                Value::String(expression.clone()),
            );
        }
        if let Some(transformer) = &self.row.input_transformer {
            definition.insert("InputTransformer".into(), transformer.clone());
        }
        definition.insert("Start".into(), self.row.start.clone());
        Value::Object(definition)
    }

    pub fn trigger_match(&self) -> Result<TriggerMatch> {
        TriggerMatch::from_definition(
            self.row.event_pattern.as_ref(),
            self.row.schedule_expression.as_deref(),
            self.row.next_run_at,
        )
    }

    async fn task_count(
        &self,
        ctx: &Context<'_>,
        since: Option<DateTime<Utc>>,
        active_only: bool,
    ) -> Result<i32> {
        let trigger_id = self.row.id;
        let mut listing = latest_tasks(self.scope.tenant_id);
        if let Some(since) = since {
            listing = listing.sort(SortKey::AtLeast(format_time(since)));
        }
        let tasks = tx(ctx, self.scope.tenant_id)
            .list::<TaskItem>(&listing, |task| {
                task.trigger_id == trigger_id && (!active_only || !task.status.is_terminal())
            })
            .await?;
        Ok(i32::try_from(tasks.len())?)
    }
}

#[Object]
impl Trigger {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn name(&self) -> &str {
        &self.row.name
    }

    async fn state(&self) -> Result<TriggerState> {
        TriggerState::from_db(&self.row.state)
    }

    #[graphql(name = "match")]
    async fn trigger_match_field(&self) -> Result<TriggerMatch> {
        self.trigger_match()
    }

    /// The YAML of the trigger in the config repository.
    async fn yaml(&self) -> Result<String> {
        let mut document = Map::new();
        document.insert(self.row.name.clone(), self.definition());
        Ok(serde_yaml_ng::to_string(&Value::Object(document))?)
    }

    async fn session(&self, ctx: &Context<'_>) -> Result<Session> {
        let name = self.start()?.session;
        Session::by_name(ctx, self.scope, self.row.config_id, &name)
            .await?
            .ok_or_else(|| not_found(format!("session {name} of trigger {}", self.row.name)))
    }

    async fn instruction(&self) -> Result<String> {
        Ok(self.start()?.instruction)
    }

    async fn skills(&self, ctx: &Context<'_>) -> Result<Vec<Skill>> {
        Skill::by_names(ctx, self.scope, self.row.config_id, &self.start()?.skills).await
    }

    async fn required_skills(&self, ctx: &Context<'_>) -> Result<Vec<Skill>> {
        Skill::by_names(
            ctx,
            self.scope,
            self.row.config_id,
            &self.start()?.required_skills,
        )
        .await
    }

    async fn max_nudges(&self) -> Result<i32> {
        Ok(i32::try_from(self.start()?.max_nudges)?)
    }

    async fn limits(&self) -> Result<TaskLimits> {
        Ok(TaskLimits(self.start()?.limits))
    }

    async fn config(&self, ctx: &Context<'_>) -> Result<Config> {
        Config::load(ctx, self.scope, self.row.config_id).await
    }

    async fn edit_url(&self, ctx: &Context<'_>) -> Result<String> {
        let config = Config::load(ctx, self.scope, self.row.config_id).await?;
        Ok(edit_url(
            &config.row.repository,
            &config.row.branch,
            &config.row.root_path,
            sanchoris_config::CONFIG_FILE,
        ))
    }

    #[graphql(name = "tasksLast7d")]
    async fn tasks_last_7d(&self, ctx: &Context<'_>) -> Result<i32> {
        self.task_count(ctx, Some(now(ctx) - Duration::days(7)), false)
            .await
    }

    /// Tasks of this trigger that are not finished, counted up to 200.
    async fn active_tasks(&self, ctx: &Context<'_>) -> Result<i32> {
        self.task_count(ctx, None, true).await
    }

    async fn open_change_request(&self, ctx: &Context<'_>) -> Result<Option<ChangeRequest>> {
        ChangeRequest::open_for(ctx, self.scope, ConfigObjectKind::Trigger, &self.row.name).await
    }

    async fn events(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 20)] limit: Option<i32>,
    ) -> Result<Vec<TriggerEvent>> {
        TriggerEvent::recent(ctx, self.scope, Some(self.row.id), limit.unwrap_or(20)).await
    }
}

pub(crate) struct TaskLimits(pub Limits);

#[Object]
impl TaskLimits {
    async fn turns(&self) -> Result<i32> {
        Ok(i32::try_from(self.0.turns)?)
    }

    async fn cost_usd(&self) -> f64 {
        self.0.cost_usd
    }

    async fn timeout_seconds(&self) -> Result<i32> {
        Ok(i32::try_from(self.0.timeout_seconds)?)
    }

    async fn heartbeat_seconds(&self) -> Result<i32> {
        Ok(i32::try_from(self.0.heartbeat_seconds)?)
    }

    async fn children_max_depth(&self) -> Result<i32> {
        Ok(i32::try_from(self.0.children.max_depth)?)
    }

    async fn children_max_count(&self) -> Result<i32> {
        Ok(i32::try_from(self.0.children.max_count)?)
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) struct TriggerMatch {
    source: EventSource,
    kind: TriggerEventKind,
    repositories: Vec<String>,
    labels: Vec<String>,
    senders: Vec<String>,
    chat_scope: Option<String>,
    slack_channel: Option<String>,
    schedule_expression: Option<String>,
    next_run_at: Option<DateTime<Utc>>,
}

impl TriggerMatch {
    fn from_definition(
        pattern: Option<&Value>,
        schedule_expression: Option<&str>,
        next_run_at: Option<DateTime<Utc>>,
    ) -> Result<Self> {
        if let Some(expression) = schedule_expression {
            return Ok(Self {
                source: EventSource::Schedule,
                kind: TriggerEventKind::Schedule,
                repositories: Vec::new(),
                labels: Vec::new(),
                senders: Vec::new(),
                chat_scope: None,
                slack_channel: None,
                schedule_expression: Some(expression.to_string()),
                next_run_at,
            });
        }
        let pattern = pattern.ok_or_else(|| {
            async_graphql::Error::new("a trigger has neither EventPattern nor ScheduleExpression")
        })?;
        let first = |key: &str| strings(pattern.get(key)).into_iter().next();
        let source = first("source")
            .ok_or_else(|| async_graphql::Error::new("the EventPattern has no source"))?;
        let detail_type = first("detail-type")
            .ok_or_else(|| async_graphql::Error::new("the EventPattern has no detail-type"))?;
        let detail = pattern.get("detail");
        let field = |path: &[&str]| {
            let mut value = detail;
            for key in path {
                value = value.and_then(|v| v.get(*key));
            }
            strings(value)
        };
        Ok(Self {
            source: EventSource::from_db(&source)?,
            kind: TriggerEventKind::from_db(&detail_type)?,
            repositories: field(&["repository"]),
            labels: field(&["label"]),
            senders: field(&["sender", "login"]),
            chat_scope: field(&["scope"]).into_iter().next(),
            slack_channel: field(&["channel"]).into_iter().next(),
            schedule_expression: None,
            next_run_at: None,
        })
    }

    pub fn scope_name(&self) -> Option<&str> {
        self.chat_scope.as_deref()
    }

    pub fn event_source(&self) -> EventSource {
        self.source
    }
}

/// What the EventPattern or ScheduleExpression matches.
#[Object]
impl TriggerMatch {
    async fn source(&self) -> EventSource {
        self.source
    }

    async fn kind(&self) -> TriggerEventKind {
        self.kind
    }

    async fn repositories(&self) -> &[String] {
        &self.repositories
    }

    async fn labels(&self) -> &[String] {
        &self.labels
    }

    /// detail.sender.login values. Empty when the pattern uses author_association.
    async fn senders(&self) -> &[String] {
        &self.senders
    }

    /// The chat scope: all or a project name.
    async fn chat_scope(&self) -> Option<&str> {
        self.chat_scope.as_deref()
    }

    async fn slack_channel(&self) -> Option<&str> {
        self.slack_channel.as_deref()
    }

    async fn schedule_expression(&self) -> Option<&str> {
        self.schedule_expression.as_deref()
    }

    async fn next_run_at(&self) -> Option<Timestamp> {
        ts_opt(self.next_run_at)
    }
}

pub(crate) struct TriggerEvent {
    scope: Scope,
    row: EventItem,
}

impl TriggerEvent {
    pub async fn recent(
        ctx: &Context<'_>,
        scope: Scope,
        trigger_id: Option<Uuid>,
        limit: i32,
    ) -> Result<Vec<Self>> {
        let rows = tx(ctx, scope.tenant_id)
            .list::<EventItem>(
                &Listing::index(Index::Gsi1, keys::trigger_events_index(scope.tenant_id))
                    .descending()
                    .limit(usize::try_from(limit)?),
                |event| {
                    event.result.is_some() && trigger_id.is_none_or(|id| event.trigger_id == id)
                },
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| TriggerEvent { scope, row })
            .collect())
    }

    pub async fn find(ctx: &Context<'_>, scope: Scope, id: Uuid) -> Result<Option<Self>> {
        let mut tx = tx(ctx, scope.tenant_id);
        let Some(location) = db::locate(&mut tx, id, REF_TRIGGER_EVENT).await? else {
            return Ok(None);
        };
        Ok(tx
            .get::<EventItem>(&location.target)
            .await?
            .map(|row| TriggerEvent { scope, row }))
    }

    pub fn event(&self) -> &Value {
        &self.row.event
    }

    pub fn key(&self) -> &str {
        &self.row.key
    }

    fn reason_kind(&self) -> Option<TriggerEventReason> {
        let reason = self.row.reason.as_deref()?;
        if reason.starts_with("children limit") {
            Some(TriggerEventReason::ChildLimit)
        } else if reason.starts_with("the actor ") {
            Some(TriggerEventReason::SenderNotAllowed)
        } else if reason.starts_with("receive limit") {
            Some(TriggerEventReason::ReceiveLimit)
        } else if reason.starts_with("task ") && reason.ends_with(" already exists") {
            Some(TriggerEventReason::Duplicate)
        } else {
            None
        }
    }
}

pub(crate) fn sender_login(event: &Value) -> Option<String> {
    let detail = &event["detail"];
    detail["sender"]["login"]
        .as_str()
        .or_else(|| detail["user"].as_str())
        .map(str::to_string)
}

#[Object]
impl TriggerEvent {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn received_at(&self) -> Timestamp {
        ts(self.row.received_at)
    }

    async fn trigger(&self, ctx: &Context<'_>) -> Result<Trigger> {
        Trigger::load(ctx, self.scope, self.row.trigger_id).await
    }

    async fn result(&self) -> Result<TriggerEventResult> {
        TriggerEventResult::from_db(
            self.row
                .result
                .as_deref()
                .ok_or_else(|| async_graphql::Error::new("the trigger event is not evaluated"))?,
        )
    }

    /// Set for SKIPPED and DISCARDED.
    async fn reason(&self) -> Option<TriggerEventReason> {
        self.reason_kind()
    }

    /// The login that caused the event, such as who added the label.
    async fn sender_login(&self) -> Option<String> {
        sender_login(&self.row.event)
    }

    /// The started task.
    async fn task(&self, ctx: &Context<'_>) -> Result<Option<Task>> {
        match self.row.task_id {
            Some(task_id) => Task::find(ctx, self.scope, task_id).await,
            None => Ok(None),
        }
    }

    /// For CHILD_LIMIT, the task whose limit was reached.
    async fn limited_task(&self, ctx: &Context<'_>) -> Result<Option<Task>> {
        if self.reason_kind() != Some(TriggerEventReason::ChildLimit) {
            return Ok(None);
        }
        let Some(name) = self
            .row
            .reason
            .as_deref()
            .and_then(|reason| reason.strip_prefix("children limit: "))
            .and_then(|rest| rest.split_once(" already has "))
            .map(|(name, _)| name)
        else {
            return Ok(None);
        };
        Task::by_name(ctx, self.scope, name).await
    }

    async fn idempotency_key(&self) -> &str {
        &self.row.key
    }
}

pub(crate) struct TriggerTemplate {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

pub(crate) const TRIGGER_TEMPLATES: &[TriggerTemplate] = &[
    TriggerTemplate {
        id: "labeled-issue",
        name: "labeled-issue",
        description: "Labeled issue → agent → pull requests with green checks",
    },
    TriggerTemplate {
        id: "chat-all",
        name: "chat-all",
        description: "Web chat across all projects",
    },
    TriggerTemplate {
        id: "weekly-deps",
        name: "weekly-deps",
        description: "Weekly dependency updates with green checks",
    },
];

impl TriggerTemplate {
    pub fn render(&self, repository: &str, session: &str) -> Value {
        let limits = serde_json::json!({
            "Turns": 60, "CostUsd": 20, "TimeoutSeconds": 14400, "HeartbeatSeconds": 900,
            "Children": { "MaxDepth": 1, "MaxCount": 5 },
        });
        match self.id {
            "labeled-issue" => serde_json::json!({
                "State": "ENABLED",
                "EventPattern": {
                    "source": ["github"],
                    "detail-type": ["Issue Labeled"],
                    "detail": {
                        "repository": [repository],
                        "label": ["sanchoris"],
                        "issue": { "author_association": ["OWNER", "MEMBER", "COLLABORATOR"] },
                    },
                },
                "InputTransformer": {
                    "InputPathsMap": {
                        "repository": "$.detail.repository",
                        "number": "$.detail.issue.number",
                        "title": "$.detail.issue.title",
                    },
                    "InputTemplate": "{ \"repository\": <repository>, \"number\": <number>, \"title\": <title> }",
                },
                "Start": {
                    "Session": session,
                    "Instruction": "Resolve issue <number> in <repository>: <title>. Write tests for the change.",
                    "Skills": ["open-pr-green-ci"],
                    "RequiredSkills": ["open-pr-green-ci"],
                    "MaxNudges": 2,
                    "Limits": limits,
                },
            }),
            "chat-all" => serde_json::json!({
                "State": "ENABLED",
                "EventPattern": {
                    "source": ["web"],
                    "detail-type": ["Message"],
                    "detail": { "scope": ["all"] },
                },
                "InputTransformer": {
                    "InputPathsMap": { "text": "$.detail.text" },
                    "InputTemplate": "{ \"text\": <text> }",
                },
                "Start": {
                    "Session": session,
                    "Instruction": "<text>",
                    "Skills": [],
                    "RequiredSkills": [],
                    "MaxNudges": 0,
                    "Limits": limits,
                },
            }),
            "weekly-deps" => serde_json::json!({
                "State": "ENABLED",
                "ScheduleExpression": "cron(0 9 ? * MON *)",
                "Start": {
                    "Session": session,
                    "Instruction": format!("Update the dependencies of {repository} and open a pull request."),
                    "Skills": ["open-pr-green-ci"],
                    "RequiredSkills": ["open-pr-green-ci"],
                    "MaxNudges": 2,
                    "Limits": limits,
                },
            }),
            other => panic!("unknown trigger template {other}"),
        }
    }
}

#[Object]
impl TriggerTemplate {
    async fn id(&self) -> ID {
        ID(self.id.to_string())
    }

    async fn name(&self) -> &str {
        self.name
    }

    async fn description(&self) -> &str {
        self.description
    }
}

pub(crate) fn templates() -> Vec<&'static TriggerTemplate> {
    TRIGGER_TEMPLATES.iter().collect()
}
