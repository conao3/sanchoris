use async_graphql::{Context, ID, InputObject, Object, Result};
use chrono::{DateTime, Utc};
use sanchoris_config::Start;
use sanchoris_core::db::{self, REF_CONFIG_REVISION, REF_RUN, REF_TRIGGER};
use sanchoris_core::item::format_time;
use sanchoris_core::items::{
    Child, ConfigRevision as RevisionItem, Dependency, InboundEvent, OutboxItem,
    Output as OutputItem, Run as RunItem, RunEventItem, SkillRunItem, Task as TaskItem, TaskName,
    Workspace as WorkspaceItem,
};
use sanchoris_core::keys;
use sanchoris_core::model::{TaskDefinition, TaskStatus as CoreStatus};
use sanchoris_core::store::{Index, Listing, SortKey};
use sanchoris_core::tasks::task_branch;
use serde_json::Value;
use uuid::Uuid;

use crate::account::Member;
use crate::config::Session;
use crate::context::{Scope, tx};
use crate::enums::{
    ChannelKind, ChatAuthorKind, LogLineKind, NeedsYouReason, OutputKind, OutputState,
    PendingReason, Provider, RequesterKind, RequiredSkillState, RunStatus, TaskStatus, WaitReason,
    WorkspaceCompute,
};
use crate::error::{not_found, parse_id};
use crate::scalars::{Timestamp, ts, ts_opt};
use crate::skill::{Skill, SkillRun};
use crate::trigger::{TaskLimits, Trigger, TriggerEvent, sender_login};

const WORKSPACE_VCPU: i32 = 2;
const WORKSPACE_MEMORY_MIB: i32 = 4096;

#[derive(InputObject, Default)]
pub(crate) struct TaskFilter {
    pub statuses: Option<Vec<TaskStatus>>,
    #[graphql(name = "triggerId")]
    pub trigger_id: Option<ID>,
    pub skill_name: Option<String>,
    pub active: Option<bool>,
}

pub(crate) struct Task {
    pub scope: Scope,
    pub row: TaskItem,
}

fn offers_skill(task: &TaskItem, name: &str) -> bool {
    task.definition["Skills"]
        .as_array()
        .is_some_and(|skills| skills.iter().any(|skill| skill["Name"] == name))
}

impl Task {
    async fn listed(
        ctx: &Context<'_>,
        scope: Scope,
        keep: impl Fn(&TaskItem) -> bool,
    ) -> Result<Vec<Self>> {
        Ok(tx(ctx, scope.tenant_id)
            .list::<TaskItem>(
                &Listing::index(Index::Gsi1, keys::tasks_index(scope.tenant_id)),
                keep,
            )
            .await?
            .into_iter()
            .map(|row| Task { scope, row })
            .collect())
    }

    pub async fn find(ctx: &Context<'_>, scope: Scope, id: Uuid) -> Result<Option<Self>> {
        Ok(tx(ctx, scope.tenant_id)
            .get::<TaskItem>(&keys::task_key(scope.tenant_id, id))
            .await?
            .map(|row| Task { scope, row }))
    }

    pub async fn load(ctx: &Context<'_>, scope: Scope, id: Uuid) -> Result<Self> {
        Self::find(ctx, scope, id)
            .await?
            .ok_or_else(|| not_found(format!("task {id}")))
    }

    pub async fn by_name(ctx: &Context<'_>, scope: Scope, name: &str) -> Result<Option<Self>> {
        let latest = tx(ctx, scope.tenant_id)
            .list::<TaskName>(
                &Listing::index(Index::Gsi1, keys::task_name_index(scope.tenant_id, name))
                    .descending()
                    .limit(1),
                |_| true,
            )
            .await?;
        match latest.into_iter().next() {
            Some(entry) => Self::find(ctx, scope, entry.task_id).await,
            None => Ok(None),
        }
    }

    pub async fn list(ctx: &Context<'_>, scope: Scope, filter: TaskFilter) -> Result<Vec<Self>> {
        let statuses = filter.statuses.map(|statuses| {
            statuses
                .into_iter()
                .map(|s| s.as_db().to_string())
                .collect::<Vec<_>>()
        });
        let trigger_id = filter.trigger_id.as_ref().map(parse_id).transpose()?;
        let mut tasks = Self::listed(ctx, scope, |task| {
            statuses
                .as_ref()
                .is_none_or(|statuses| statuses.iter().any(|s| s == task.status.as_str()))
                && trigger_id.is_none_or(|id| task.trigger_id == id)
                && filter
                    .skill_name
                    .as_deref()
                    .is_none_or(|name| offers_skill(task, name))
                && filter
                    .active
                    .is_none_or(|active| task.status.is_terminal() != active)
        })
        .await?;
        tasks.sort_by(|a, b| {
            let needs_you = |task: &Task| task.row.status == CoreStatus::NeedsYou;
            (needs_you(b), b.row.updated_at, a.row.id).cmp(&(
                needs_you(a),
                a.row.updated_at,
                b.row.id,
            ))
        });
        Ok(tasks)
    }

    pub async fn chats(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        let mut tasks = Self::listed(ctx, scope, |task| {
            task.channel
                .as_ref()
                .and_then(|channel| channel["source"].as_str())
                .is_some_and(|source| matches!(source, "web" | "slack"))
        })
        .await?;
        tasks.sort_by(|a, b| (b.row.created_at, a.row.id).cmp(&(a.row.created_at, b.row.id)));
        Ok(tasks)
    }

    async fn found(ctx: &Context<'_>, scope: Scope, ids: Vec<Uuid>) -> Result<Vec<Self>> {
        let mut tasks = Vec::new();
        for id in ids {
            tasks.push(Self::load(ctx, scope, id).await?);
        }
        tasks.sort_by(|a, b| (a.row.created_at, a.row.id).cmp(&(b.row.created_at, b.row.id)));
        Ok(tasks)
    }

    fn pk(&self) -> String {
        keys::task_pk(self.scope.tenant_id, self.row.id)
    }

    pub fn definition(&self) -> Result<TaskDefinition> {
        serde_json::from_value(self.row.definition.clone()).map_err(|e| {
            async_graphql::Error::new(format!(
                "task {} has an invalid definition: {e}",
                self.row.id
            ))
        })
    }

    pub fn start(&self) -> Result<Start> {
        let definition = self.definition()?;
        serde_json::from_value(definition.trigger.definition["Start"].clone()).map_err(|e| {
            async_graphql::Error::new(format!("task {} has an invalid Start: {e}", self.row.id))
        })
    }

    pub fn status_value(&self) -> Result<TaskStatus> {
        TaskStatus::from_db(self.row.status.as_str())
    }

    pub async fn config_id(&self, ctx: &Context<'_>) -> Result<Uuid> {
        let mut tx = tx(ctx, self.scope.tenant_id);
        db::locate(&mut tx, self.row.trigger_id, REF_TRIGGER)
            .await?
            .and_then(|location| location.parent_id)
            .ok_or_else(|| not_found(format!("trigger {}", self.row.trigger_id)))
    }

    pub async fn session_data(&self, ctx: &Context<'_>) -> Result<Session> {
        let definition = self.definition()?;
        let config_id = self.config_id(ctx).await?;
        let name = definition.session.name;
        let id = Session::by_name(ctx, self.scope, config_id, &name)
            .await?
            .map(|session| session.id);
        Ok(Session {
            scope: self.scope,
            id: id.unwrap_or_else(|| format!("{}/session", self.row.id)),
            config_id,
            name,
            definition: definition.session.definition,
        })
    }

    pub fn chat_thread(&self) -> Option<(ChannelKind, String)> {
        let channel = self.row.channel.as_ref()?;
        let kind = ChannelKind::from_db(channel["source"].as_str()?).ok()?;
        Some((kind, channel["thread"].as_str()?.to_string()))
    }

    pub async fn chat_scope(&self, ctx: &Context<'_>) -> Result<String> {
        if let Some(event) = self.trigger_event(ctx).await?
            && let Some(scope) = event.event()["detail"]["scope"].as_str()
        {
            return Ok(scope.to_string());
        }
        let session = self.session_data(ctx).await?;
        Ok(session
            .definition
            .get("Project")
            .and_then(Value::as_str)
            .unwrap_or("all")
            .to_string())
    }

    async fn trigger_event(&self, ctx: &Context<'_>) -> Result<Option<TriggerEvent>> {
        match self.row.trigger_event_id {
            Some(id) => TriggerEvent::find(ctx, self.scope, id).await,
            None => Ok(None),
        }
    }

    pub async fn runs(&self, ctx: &Context<'_>) -> Result<Vec<Run>> {
        let mut rows = tx(ctx, self.scope.tenant_id)
            .query::<RunItem>(&self.pk(), "RUN#")
            .await?;
        rows.sort_by(|a, b| (a.started_at, a.id).cmp(&(b.started_at, b.id)));
        Ok(rows
            .into_iter()
            .map(|row| Run {
                scope: self.scope,
                row,
            })
            .collect())
    }
}

#[Object]
impl Task {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn name(&self) -> &str {
        &self.row.name
    }

    async fn title(&self) -> &str {
        &self.row.title
    }

    async fn status(&self) -> Result<TaskStatus> {
        self.status_value()
    }

    async fn wait_reason(&self) -> Result<Option<WaitReason>> {
        self.row
            .wait_reason
            .map(|reason| WaitReason::from_db(reason.as_str()))
            .transpose()
    }

    async fn pending_reason(&self, ctx: &Context<'_>) -> Result<Option<PendingReason>> {
        if self.row.status != CoreStatus::Pending {
            return Ok(None);
        }
        let mut tx = tx(ctx, self.scope.tenant_id);
        let mut blocked = false;
        for dependency in tx.query::<Dependency>(&self.pk(), "DEP#").await? {
            let blocker = tx
                .get::<TaskItem>(&keys::task_key(
                    self.scope.tenant_id,
                    dependency.blocked_by_task_id,
                ))
                .await?;
            blocked |= blocker.is_some_and(|blocker| !blocker.status.is_terminal());
        }
        if blocked {
            return Ok(Some(PendingReason::Blocked));
        }
        let (tenant, plan) = db::tenant_plan(&mut tx).await?;
        let limited = tenant.status != "active"
            || tenant.tasks_this_period >= plan.task_limit
            || tenant.workspace_seconds_this_period >= plan.workspace_seconds_limit;
        Ok(Some(if limited {
            PendingReason::PlanLimit
        } else {
            PendingReason::Concurrency
        }))
    }

    async fn needs_you_reason(&self) -> Option<NeedsYouReason> {
        if self.row.status != CoreStatus::NeedsYou {
            return None;
        }
        Some(match self.row.error.as_deref() {
            None | Some(sanchoris_core::tasks::ERROR_APPROVAL_REQUESTED) => {
                NeedsYouReason::AgentAsked
            }
            Some(sanchoris_core::lifecycle::ERROR_NUDGES_EXHAUSTED) => {
                NeedsYouReason::NudgesExhausted
            }
            Some(sanchoris_core::lifecycle::ERROR_USAGE_LIMIT) => NeedsYouReason::UsageLimit,
            Some(
                sanchoris_core::lifecycle::ERROR_TURN_LIMIT
                | sanchoris_core::lifecycle::ERROR_COST_LIMIT
                | sanchoris_core::lifecycle::ERROR_TIMEOUT
                | sanchoris_core::lifecycle::ERROR_HEARTBEAT_TIMEOUT,
            ) => NeedsYouReason::LimitExceeded,
            Some(_) => NeedsYouReason::SkillFailed,
        })
    }

    async fn resume_at(&self) -> Option<Timestamp> {
        ts_opt(self.row.resume_at)
    }

    async fn auto_resume(&self) -> Result<bool> {
        Ok(self.definition()?.session.definition["UsageLimit"]["Resume"] == "auto")
    }

    async fn snoozed_until(&self) -> Option<Timestamp> {
        ts_opt(self.row.snoozed_until)
    }

    async fn nudges(&self) -> Result<i32> {
        Ok(i32::try_from(self.row.nudges)?)
    }

    async fn max_nudges(&self) -> Result<i32> {
        Ok(i32::try_from(self.start()?.max_nudges)?)
    }

    async fn error(&self) -> Option<&str> {
        self.row.error.as_deref()
    }

    async fn cause(&self) -> Option<&str> {
        self.row.cause.as_deref()
    }

    async fn created_at(&self) -> Timestamp {
        ts(self.row.created_at)
    }

    async fn started_at(&self) -> Option<Timestamp> {
        ts_opt(self.row.started_at)
    }

    async fn ended_at(&self) -> Option<Timestamp> {
        ts_opt(self.row.finished_at)
    }

    async fn updated_at(&self) -> Timestamp {
        ts(self.row.updated_at)
    }

    async fn trigger(&self, ctx: &Context<'_>) -> Result<Trigger> {
        Trigger::load(ctx, self.scope, self.row.trigger_id).await
    }

    #[graphql(name = "triggerEvent")]
    async fn trigger_event_field(&self, ctx: &Context<'_>) -> Result<Option<TriggerEvent>> {
        self.trigger_event(ctx).await
    }

    async fn requester(&self, ctx: &Context<'_>) -> Result<Option<Requester>> {
        if let Some(member_id) = self.row.requested_by_member_id {
            return Ok(Some(Requester {
                kind: RequesterKind::Member,
                member: Member::find(ctx, self.scope, member_id).await?,
                task: None,
                via_chat: self.chat_thread().is_some(),
            }));
        }
        let from_bot = self
            .trigger_event(ctx)
            .await?
            .and_then(|event| sender_login(event.event()))
            .is_some_and(|login| login.ends_with("[bot]"));
        if !from_bot {
            return Ok(None);
        }
        let task = match self.row.parent_task_id {
            Some(parent) => Task::find(ctx, self.scope, parent).await?,
            None => None,
        };
        Ok(Some(Requester {
            kind: RequesterKind::Agent,
            member: None,
            task,
            via_chat: false,
        }))
    }

    async fn session(&self, ctx: &Context<'_>) -> Result<Session> {
        self.session_data(ctx).await
    }

    async fn config_revision(&self, ctx: &Context<'_>) -> Result<i32> {
        let mut tx = tx(ctx, self.scope.tenant_id);
        let revision_id = self.row.config_revision_id;
        let location = db::locate(&mut tx, revision_id, REF_CONFIG_REVISION)
            .await?
            .ok_or_else(|| not_found(format!("config revision {revision_id}")))?;
        Ok(i32::try_from(
            tx.require::<RevisionItem>(&location.target).await?.number,
        )?)
    }

    async fn issue(&self, ctx: &Context<'_>) -> Result<Option<IssueRef>> {
        let Some(event) = self.trigger_event(ctx).await? else {
            return Ok(None);
        };
        Ok(IssueRef::from_detail(self.scope, &event.event()["detail"]))
    }

    async fn channel(&self, ctx: &Context<'_>) -> Result<Option<Channel>> {
        let Some((kind, thread_id)) = self.chat_thread() else {
            return Ok(None);
        };
        let slack_channel = self
            .row
            .channel
            .as_ref()
            .and_then(|channel| channel["channel"].as_str())
            .map(str::to_string);
        Ok(Some(Channel {
            kind,
            thread_id,
            scope: self.chat_scope(ctx).await?,
            slack_channel,
        }))
    }

    async fn parent(&self, ctx: &Context<'_>) -> Result<Option<Task>> {
        match self.row.parent_task_id {
            Some(parent) => Task::find(ctx, self.scope, parent).await,
            None => Ok(None),
        }
    }

    async fn children(&self, ctx: &Context<'_>) -> Result<Vec<Task>> {
        let children = tx(ctx, self.scope.tenant_id)
            .query::<Child>(&self.pk(), "CHILD#")
            .await?;
        Task::found(
            ctx,
            self.scope,
            children.into_iter().map(|child| child.task_id).collect(),
        )
        .await
    }

    async fn blocked_by(&self, ctx: &Context<'_>) -> Result<Vec<Task>> {
        let dependencies = tx(ctx, self.scope.tenant_id)
            .query::<Dependency>(&self.pk(), "DEP#")
            .await?;
        Task::found(
            ctx,
            self.scope,
            dependencies
                .into_iter()
                .map(|dependency| dependency.blocked_by_task_id)
                .collect(),
        )
        .await
    }

    async fn filed_tasks(&self) -> Vec<Task> {
        Vec::new()
    }

    async fn required_skills(&self, ctx: &Context<'_>) -> Result<Vec<RequiredSkillStatus>> {
        let definition = self.definition()?;
        let required = self.start()?.required_skills;
        let runs = SkillRun::of_task(ctx, self.scope, self.row.id).await?;
        let config_id = self.config_id(ctx).await?;
        let mut statuses = Vec::new();
        for name in required {
            let offered = definition
                .skills
                .iter()
                .find(|skill| skill.name == name)
                .ok_or_else(|| {
                    async_graphql::Error::new(format!(
                        "required skill {name} is not offered by task {}",
                        self.row.id
                    ))
                })?;
            let skill = Skill::for_run(
                ctx,
                self.scope,
                config_id,
                &name,
                &offered.definition["definition"],
            )
            .await?;
            let latest = runs
                .iter()
                .filter(|run| run.row.skill_name == name)
                .max_by_key(|run| run.row.number);
            let state = match latest.map(|run| run.row.status.as_str()) {
                None => RequiredSkillState::NotRun,
                Some("SUCCEEDED") => RequiredSkillState::Succeeded,
                Some("FAILED") => RequiredSkillState::Failed,
                Some(_) => RequiredSkillState::Running,
            };
            statuses.push(RequiredSkillStatus {
                skill,
                state,
                latest_run: latest.map(|run| SkillRun {
                    scope: self.scope,
                    row: run.row.clone(),
                }),
            });
        }
        Ok(statuses)
    }

    async fn skill_runs(&self, ctx: &Context<'_>) -> Result<Vec<SkillRun>> {
        SkillRun::of_task(ctx, self.scope, self.row.id).await
    }

    #[graphql(name = "runs")]
    async fn runs_field(&self, ctx: &Context<'_>) -> Result<Vec<Run>> {
        self.runs(ctx).await
    }

    async fn run(&self, ctx: &Context<'_>, number: i32) -> Result<Option<Run>> {
        Ok(self
            .runs(ctx)
            .await?
            .into_iter()
            .find(|run| run.row.number == i64::from(number)))
    }

    async fn outputs(&self, ctx: &Context<'_>) -> Result<Vec<Output>> {
        Output::of_task(ctx, self.scope, self.row.id, |_| true).await
    }

    async fn messages(&self, ctx: &Context<'_>) -> Result<Vec<ChatMessage>> {
        ChatMessage::for_task(ctx, self).await
    }

    async fn limits(&self) -> Result<TaskLimits> {
        Ok(TaskLimits(self.start()?.limits))
    }

    async fn turns(&self, ctx: &Context<'_>) -> Result<i32> {
        let turns = self
            .runs(ctx)
            .await?
            .iter()
            .map(|run| run.row.turns)
            .sum::<i64>();
        Ok(i32::try_from(turns)?)
    }

    async fn cost_usd(&self, ctx: &Context<'_>) -> Result<f64> {
        Ok(self
            .runs(ctx)
            .await?
            .iter()
            .map(|run| run.row.cost_usd)
            .sum())
    }
}

pub(crate) struct Channel {
    kind: ChannelKind,
    thread_id: String,
    scope: String,
    slack_channel: Option<String>,
}

#[Object]
impl Channel {
    async fn kind(&self) -> ChannelKind {
        self.kind
    }

    async fn thread_id(&self) -> &str {
        &self.thread_id
    }

    async fn scope(&self) -> &str {
        &self.scope
    }

    async fn slack_channel(&self) -> Option<&str> {
        self.slack_channel.as_deref()
    }
}

pub(crate) struct IssueRef {
    scope: Scope,
    repository: String,
    number: i32,
    title: String,
    url: String,
}

impl IssueRef {
    fn from_detail(scope: Scope, detail: &Value) -> Option<Self> {
        let issue = detail.get("issue")?;
        Some(Self {
            scope,
            repository: detail["repository"].as_str()?.to_string(),
            number: i32::try_from(issue["number"].as_i64()?).ok()?,
            title: issue["title"].as_str()?.to_string(),
            url: issue["url"].as_str()?.to_string(),
        })
    }
}

#[Object]
impl IssueRef {
    async fn repository(&self) -> &str {
        &self.repository
    }

    async fn number(&self) -> i32 {
        self.number
    }

    async fn title(&self) -> &str {
        &self.title
    }

    async fn url(&self) -> &str {
        &self.url
    }

    async fn task(&self, ctx: &Context<'_>) -> Result<Option<Task>> {
        Task::by_name(
            ctx,
            self.scope,
            &format!("{}#{}", self.repository, self.number),
        )
        .await
    }
}

pub(crate) struct Requester {
    kind: RequesterKind,
    member: Option<Member>,
    task: Option<Task>,
    via_chat: bool,
}

#[Object]
impl Requester {
    async fn kind(&self) -> RequesterKind {
        self.kind
    }

    async fn member(&self) -> Option<&Member> {
        self.member.as_ref()
    }

    async fn task(&self) -> Option<&Task> {
        self.task.as_ref()
    }

    async fn via_chat(&self) -> bool {
        self.via_chat
    }
}

pub(crate) struct RequiredSkillStatus {
    skill: Skill,
    state: RequiredSkillState,
    latest_run: Option<SkillRun>,
}

#[Object]
impl RequiredSkillStatus {
    async fn skill(&self) -> &Skill {
        &self.skill
    }

    async fn state(&self) -> RequiredSkillState {
        self.state
    }

    async fn latest_run(&self) -> Option<&SkillRun> {
        self.latest_run.as_ref()
    }
}

pub(crate) struct Run {
    pub scope: Scope,
    pub row: RunItem,
}

impl Run {
    pub async fn load(ctx: &Context<'_>, scope: Scope, run_id: Uuid) -> Result<Self> {
        let mut tx = tx(ctx, scope.tenant_id);
        let location = db::locate(&mut tx, run_id, REF_RUN)
            .await?
            .ok_or_else(|| not_found(format!("run {run_id}")))?;
        let row = tx.require::<RunItem>(&location.target).await?;
        Ok(Run { scope, row })
    }

    async fn workspaces(&self, ctx: &Context<'_>) -> Result<Vec<WorkspaceItem>> {
        let mut workspaces = tx(ctx, self.scope.tenant_id)
            .query::<WorkspaceItem>(
                &keys::task_pk(self.scope.tenant_id, self.row.task_id),
                "WS#",
            )
            .await?
            .into_iter()
            .filter(|workspace| workspace.run_id == self.row.id)
            .collect::<Vec<_>>();
        workspaces.sort_by(|a, b| (a.started_at, a.id).cmp(&(b.started_at, b.id)));
        Ok(workspaces)
    }

    async fn log_lines(&self, ctx: &Context<'_>) -> Result<Vec<LogLine>> {
        let mut entries: Vec<(DateTime<Utc>, u8, LogLineKind, String)> = Vec::new();
        for workspace in self.workspaces(ctx).await? {
            entries.push((
                workspace.started_at,
                0,
                LogLineKind::System,
                format!(
                    "workspace started · {} · {} · {WORKSPACE_VCPU} vCPU",
                    workspace.platform, workspace.compute
                ),
            ));
            if let Some(stopped) = workspace.stopped_at {
                entries.push((
                    stopped,
                    0,
                    LogLineKind::System,
                    "workspace stopped".to_string(),
                ));
            }
        }
        let mut tx = tx(ctx, self.scope.tenant_id);
        let run_pk = keys::run_pk(self.scope.tenant_id, self.row.id);
        for outbox in tx.query::<OutboxItem>(&run_pk, "OUT#").await? {
            let payload = &outbox.payload;
            let text = |key: &str| payload[key].as_str().unwrap_or("").to_string();
            let line = match outbox.kind.as_str() {
                "start" => match payload["instruction"].as_str() {
                    Some(instruction) => (LogLineKind::Instruction, instruction.to_string()),
                    None => (
                        LogLineKind::System,
                        "session restored from its record".to_string(),
                    ),
                },
                "user_message" => {
                    let message = text("text");
                    if message.starts_with("[sanchoris] Message from ") {
                        (LogLineKind::User, message)
                    } else {
                        (LogLineKind::Instruction, message)
                    }
                }
                "exec" => (
                    LogLineKind::Tool,
                    payload["commands"]
                        .as_array()
                        .map(|commands| {
                            commands
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default(),
                ),
                "interrupt" => (LogLineKind::System, "interrupted the session".to_string()),
                "shutdown" => (
                    LogLineKind::System,
                    format!("session stopped: {}", text("reason")),
                ),
                "chat_post" => (LogLineKind::Agent, text("text")),
                other => (LogLineKind::System, other.to_string()),
            };
            entries.push((outbox.created_at, 1, line.0, line.1));
        }
        let mut events = tx.query::<RunEventItem>(&run_pk, "EVT#").await?;
        events.sort_by(|a, b| (a.received_at, a.seq, a.id).cmp(&(b.received_at, b.seq, b.id)));
        for event in events {
            let payload = &event.payload;
            let line = match event.kind.as_str() {
                "ready" => Some((LogLineKind::System, "session ready".to_string())),
                "turn_end" => Some((
                    LogLineKind::Event,
                    match (payload["turn"].as_i64(), payload["summary"].as_str()) {
                        (Some(turn), Some(summary)) => format!("turn {turn} ended: {summary}"),
                        (Some(turn), None) => format!("turn {turn} ended"),
                        (None, _) => "turn ended".to_string(),
                    },
                )),
                "rate_limit" => Some((
                    LogLineKind::Event,
                    "the worker reached its usage limit".to_string(),
                )),
                "session_record" => Some((LogLineKind::System, "session record saved".to_string())),
                "exec_result" => Some((
                    LogLineKind::Tool,
                    if payload["ok"].as_bool() == Some(true) {
                        "commands succeeded".to_string()
                    } else {
                        "commands failed".to_string()
                    },
                )),
                _ => None,
            };
            if let Some((kind, text)) = line {
                entries.push((event.received_at, 2, kind, text));
            }
            if let Some(error) = event.error {
                entries.push((event.received_at, 3, LogLineKind::Error, error));
            }
        }
        if let Some(error) = &self.row.error {
            entries.push((
                self.row.finished_at.unwrap_or(self.row.updated_at),
                4,
                LogLineKind::Error,
                error.clone(),
            ));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        Ok(entries
            .into_iter()
            .enumerate()
            .map(|(index, (at, _, kind, text))| LogLine {
                sequence: i32::try_from(index + 1).expect("log lines fit in i32"),
                at,
                kind,
                text,
            })
            .collect())
    }
}

#[Object]
impl Run {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn number(&self) -> Result<i32> {
        Ok(i32::try_from(self.row.number)?)
    }

    async fn task(&self, ctx: &Context<'_>) -> Result<Task> {
        Task::load(ctx, self.scope, self.row.task_id).await
    }

    async fn session_name(&self) -> &str {
        &self.row.session_name
    }

    async fn agent(&self) -> Result<Provider> {
        Provider::from_db(&self.row.worker)
    }

    async fn model(&self) -> &str {
        &self.row.model
    }

    async fn credential_name(&self, ctx: &Context<'_>) -> Result<String> {
        let task = Task::load(ctx, self.scope, self.row.task_id).await?;
        task.definition()?.session.definition["Worker"]["Credential"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| async_graphql::Error::new("the task session has no Worker.Credential"))
    }

    async fn agent_session_id(&self) -> Option<&str> {
        self.row.agent_session_id.as_deref()
    }

    async fn status(&self) -> Result<RunStatus> {
        RunStatus::from_db(&self.row.status)
    }

    async fn started_at(&self) -> Timestamp {
        ts(self.row.started_at)
    }

    async fn ended_at(&self) -> Option<Timestamp> {
        ts_opt(self.row.finished_at)
    }

    async fn turns(&self) -> Result<i32> {
        Ok(i32::try_from(self.row.turns)?)
    }

    async fn cost_usd(&self) -> f64 {
        self.row.cost_usd
    }

    async fn summary(&self) -> Option<&str> {
        self.row.summary.as_deref()
    }

    async fn workspace(&self, ctx: &Context<'_>) -> Result<Workspace> {
        if let Some(latest) = self.workspaces(ctx).await?.pop() {
            return Ok(Workspace {
                platform: latest.platform,
                compute: WorkspaceCompute::from_db(&latest.compute)?,
                repositories: latest.repositories,
                branch: latest.branch,
                base_commit: latest.base_commit.unwrap_or_default(),
            });
        }
        let task = Task::load(ctx, self.scope, self.row.task_id).await?;
        let session = task.session_data(ctx).await?;
        let parsed = session.parsed()?;
        Ok(Workspace {
            platform: "pending".to_string(),
            compute: WorkspaceCompute::from_db(parsed.workspace.compute.as_str())?,
            repositories: session.repository_names(ctx).await?,
            branch: task_branch(task.row.id),
            base_commit: String::new(),
        })
    }

    async fn log(&self, ctx: &Context<'_>, after: Option<i32>) -> Result<Vec<LogLine>> {
        Ok(self
            .log_lines(ctx)
            .await?
            .into_iter()
            .filter(|line| after.is_none_or(|after| line.sequence > after))
            .collect())
    }

    async fn skill_runs(&self, ctx: &Context<'_>) -> Result<Vec<SkillRun>> {
        Ok(SkillRun::of_task(ctx, self.scope, self.row.task_id)
            .await?
            .into_iter()
            .filter(|run| run.row.run_id == Some(self.row.id))
            .collect())
    }

    async fn outputs(&self, ctx: &Context<'_>) -> Result<Vec<Output>> {
        let run_id = self.row.id;
        Output::of_task(ctx, self.scope, self.row.task_id, |run| run == Some(run_id)).await
    }
}

pub(crate) struct Workspace {
    platform: String,
    compute: WorkspaceCompute,
    repositories: Vec<String>,
    branch: String,
    base_commit: String,
}

#[Object]
impl Workspace {
    async fn platform(&self) -> &str {
        &self.platform
    }

    async fn compute(&self) -> WorkspaceCompute {
        self.compute
    }

    async fn vcpu(&self) -> i32 {
        WORKSPACE_VCPU
    }

    async fn memory_mib(&self) -> i32 {
        WORKSPACE_MEMORY_MIB
    }

    async fn repositories(&self) -> &[String] {
        &self.repositories
    }

    async fn branch(&self) -> &str {
        &self.branch
    }

    async fn base_commit(&self) -> &str {
        &self.base_commit
    }
}

pub(crate) struct LogLine {
    sequence: i32,
    at: DateTime<Utc>,
    kind: LogLineKind,
    text: String,
}

#[Object]
impl LogLine {
    async fn sequence(&self) -> i32 {
        self.sequence
    }

    async fn at(&self) -> Timestamp {
        ts(self.at)
    }

    async fn kind(&self) -> LogLineKind {
        self.kind
    }

    async fn text(&self) -> &str {
        &self.text
    }
}

pub(crate) struct Output {
    row: OutputItem,
}

impl Output {
    fn sorted(mut rows: Vec<OutputItem>) -> Vec<Self> {
        rows.sort_by(|a, b| (a.created_at, a.id).cmp(&(b.created_at, b.id)));
        rows.into_iter().map(|row| Output { row }).collect()
    }

    pub async fn of_task(
        ctx: &Context<'_>,
        scope: Scope,
        task_id: Uuid,
        run: impl Fn(Option<Uuid>) -> bool,
    ) -> Result<Vec<Self>> {
        let mut tx = tx(ctx, scope.tenant_id);
        let pk = keys::task_pk(scope.tenant_id, task_id);
        let skill_runs = tx.query::<SkillRunItem>(&pk, "SR#").await?;
        let outputs = tx
            .query::<OutputItem>(&pk, "SR#")
            .await?
            .into_iter()
            .filter(|output| {
                run(skill_runs
                    .iter()
                    .find(|skill_run| skill_run.id == output.skill_run_id)
                    .and_then(|skill_run| skill_run.run_id))
            })
            .collect();
        Ok(Self::sorted(outputs))
    }

    pub async fn of_skill_run(
        ctx: &Context<'_>,
        scope: Scope,
        task_id: Uuid,
        number: i32,
    ) -> Result<Vec<Self>> {
        let rows = tx(ctx, scope.tenant_id)
            .query::<OutputItem>(
                &keys::task_pk(scope.tenant_id, task_id),
                &format!("{}#OUT#", keys::skill_run_prefix(number)),
            )
            .await?;
        Ok(Self::sorted(rows))
    }
}

#[Object]
impl Output {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn kind(&self) -> Result<OutputKind> {
        OutputKind::from_db(&self.row.kind)
    }

    async fn repository(&self) -> &str {
        &self.row.repository
    }

    async fn number(&self) -> Result<i32> {
        Ok(i32::try_from(self.row.number)?)
    }

    async fn url(&self) -> &str {
        &self.row.url
    }

    async fn state(&self) -> Result<OutputState> {
        match &self.row.state {
            Some(state) => OutputState::from_db(state),
            None => Ok(OutputState::Open),
        }
    }

    async fn created_at(&self) -> Timestamp {
        ts(self.row.created_at)
    }
}

pub(crate) struct ChatMessage {
    scope: Scope,
    id: Uuid,
    author_kind: ChatAuthorKind,
    member_id: Option<Uuid>,
    text: String,
    at: DateTime<Utc>,
}

impl ChatMessage {
    pub async fn for_task(ctx: &Context<'_>, task: &Task) -> Result<Vec<Self>> {
        let Some((kind, thread)) = task.chat_thread() else {
            return Ok(Vec::new());
        };
        let tenant_id = task.scope.tenant_id;
        let mut tx = tx(ctx, tenant_id);
        let started = match task.trigger_event(ctx).await? {
            Some(event) => tx
                .get::<InboundEvent>(&keys::inbound_event_key(
                    tenant_id,
                    kind.as_db(),
                    event.key(),
                ))
                .await?
                .map_or(task.row.created_at, |inbound| inbound.received_at),
            None => task.row.created_at,
        };
        let next = tx
            .query::<TaskName>(
                &keys::trigger_pk(tenant_id, task.row.trigger_id),
                &format!("TASK#{thread}/"),
            )
            .await?
            .into_iter()
            .map(|name| name.created_at)
            .filter(|created| *created > task.row.created_at)
            .min();
        let inbound = tx
            .list::<InboundEvent>(
                &Listing::index(
                    Index::Gsi1,
                    keys::thread_index(tenant_id, kind.as_db(), &thread),
                )
                .sort(SortKey::AtLeast(format_time(started))),
                |event| {
                    event.received_at >= started && next.is_none_or(|next| event.received_at < next)
                },
            )
            .await?;
        let mut posts = Vec::new();
        for run in task.runs(ctx).await? {
            posts.extend(
                tx.query::<OutboxItem>(&keys::run_pk(tenant_id, run.row.id), "OUT#")
                    .await?
                    .into_iter()
                    .filter(|outbox| outbox.kind == "chat_post"),
            );
        }
        let mut messages = inbound
            .into_iter()
            .map(|event| ChatMessage {
                scope: task.scope,
                id: event.id,
                author_kind: ChatAuthorKind::Member,
                member_id: event.event["detail"]["member_id"]
                    .as_str()
                    .and_then(|id| id.parse().ok()),
                text: event.event["detail"]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                at: event.received_at,
            })
            .chain(posts.into_iter().map(|outbox| ChatMessage {
                scope: task.scope,
                id: outbox.id,
                author_kind: ChatAuthorKind::Agent,
                member_id: None,
                text: outbox.payload["text"].as_str().unwrap_or("").to_string(),
                at: outbox.created_at,
            }))
            .collect::<Vec<_>>();
        messages.sort_by(|a, b| a.at.cmp(&b.at).then(a.id.cmp(&b.id)));
        Ok(messages)
    }

    pub fn inbound(
        scope: Scope,
        id: Uuid,
        member_id: Uuid,
        text: String,
        at: DateTime<Utc>,
    ) -> Self {
        Self {
            scope,
            id,
            author_kind: ChatAuthorKind::Member,
            member_id: Some(member_id),
            text,
            at,
        }
    }
}

#[Object]
impl ChatMessage {
    async fn id(&self) -> ID {
        ID(self.id.to_string())
    }

    async fn author_kind(&self) -> ChatAuthorKind {
        self.author_kind
    }

    async fn member(&self, ctx: &Context<'_>) -> Result<Option<Member>> {
        match self.member_id {
            Some(member_id) => Member::find(ctx, self.scope, member_id).await,
            None => Ok(None),
        }
    }

    async fn text(&self) -> &str {
        &self.text
    }

    async fn at(&self) -> Timestamp {
        ts(self.at)
    }

    async fn issues(&self) -> Vec<IssueRef> {
        Vec::new()
    }
}
