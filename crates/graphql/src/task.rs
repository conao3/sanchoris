use async_graphql::{Context, ID, InputObject, Object, Result};
use chrono::{DateTime, Utc};
use sanchoris_config::Start;
use sanchoris_core::model::TaskDefinition;
use sanchoris_core::tasks::task_branch;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::account::Member;
use crate::config::Session;
use crate::context::{Scope, tenant_tx};
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

#[derive(Debug, Clone, FromRow)]
pub(crate) struct TaskRow {
    pub id: Uuid,
    pub trigger_id: Uuid,
    pub trigger_event_id: Option<Uuid>,
    pub config_revision_id: Uuid,
    pub name: String,
    pub title: String,
    pub status: String,
    pub wait_reason: Option<String>,
    pub resume_at: Option<DateTime<Utc>>,
    pub snoozed_until: Option<DateTime<Utc>>,
    pub nudges: i32,
    pub definition: Value,
    pub parent_task_id: Option<Uuid>,
    pub channel: Option<Value>,
    pub requested_by_member_id: Option<Uuid>,
    pub error: Option<String>,
    pub cause: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

const TASK_SELECT: &str = "SELECT t.id, t.trigger_id, t.trigger_event_id, t.config_revision_id, t.name, t.title, \
     t.status, t.wait_reason, t.resume_at, t.snoozed_until, t.nudges, t.definition, t.parent_task_id, \
     t.channel, t.requested_by_member_id, t.error, t.cause, t.created_at, t.started_at, t.finished_at, \
     t.updated_at FROM sanchoris.tasks t";

const TERMINAL: &str = "('SUCCEEDED', 'FAILED', 'ABORTED')";

pub(crate) struct Task {
    pub scope: Scope,
    pub row: TaskRow,
}

impl Task {
    async fn select(
        ctx: &Context<'_>,
        scope: Scope,
        condition: &str,
        id: Option<Uuid>,
        text: Option<&str>,
    ) -> Result<Vec<Self>> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let rows = sqlx::query_as::<_, TaskRow>(sql!(
            "{TASK_SELECT} WHERE t.tenant_id = $1 AND {condition}"
        ))
        .bind(scope.tenant_id)
        .bind(id)
        .bind(text)
        .fetch_all(tx.conn())
        .await?;
        Ok(rows.into_iter().map(|row| Task { scope, row }).collect())
    }

    pub async fn find(ctx: &Context<'_>, scope: Scope, id: Uuid) -> Result<Option<Self>> {
        Ok(
            Self::select(ctx, scope, "t.id = $2 AND $3::text IS NULL", Some(id), None)
                .await?
                .into_iter()
                .next(),
        )
    }

    pub async fn load(ctx: &Context<'_>, scope: Scope, id: Uuid) -> Result<Self> {
        Self::find(ctx, scope, id)
            .await?
            .ok_or_else(|| not_found(format!("task {id}")))
    }

    pub async fn by_trigger_event(
        ctx: &Context<'_>,
        scope: Scope,
        event_id: Uuid,
    ) -> Result<Option<Self>> {
        Ok(Self::select(
            ctx,
            scope,
            "t.trigger_event_id = $2 AND $3::text IS NULL",
            Some(event_id),
            None,
        )
        .await?
        .into_iter()
        .next())
    }

    pub async fn by_name(ctx: &Context<'_>, scope: Scope, name: &str) -> Result<Option<Self>> {
        Ok(Self::select(
            ctx,
            scope,
            "$2::uuid IS NULL AND t.name = $3 ORDER BY t.created_at DESC, t.id DESC LIMIT 1",
            None,
            Some(name),
        )
        .await?
        .into_iter()
        .next())
    }

    pub async fn list(ctx: &Context<'_>, scope: Scope, filter: TaskFilter) -> Result<Vec<Self>> {
        let statuses = filter.statuses.map(|statuses| {
            statuses
                .into_iter()
                .map(|s| s.as_db().to_string())
                .collect::<Vec<_>>()
        });
        let trigger_id = filter.trigger_id.as_ref().map(parse_id).transpose()?;
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let rows = sqlx::query_as::<_, TaskRow>(sql!(
            "{TASK_SELECT} WHERE t.tenant_id = $1 \
               AND ($2::text[] IS NULL OR t.status = ANY($2)) \
               AND ($3::uuid IS NULL OR t.trigger_id = $3) \
               AND ($4::text IS NULL OR EXISTS ( \
                     SELECT 1 FROM jsonb_array_elements(t.definition->'Skills') s WHERE s->>'Name' = $4)) \
               AND ($5::boolean IS NULL OR (t.status NOT IN {TERMINAL}) = $5) \
             ORDER BY (t.status = 'NEEDS_YOU') DESC, t.updated_at DESC, t.id"
        ))
        .bind(scope.tenant_id)
        .bind(statuses)
        .bind(trigger_id)
        .bind(filter.skill_name)
        .bind(filter.active)
        .fetch_all(tx.conn())
        .await?;
        Ok(rows.into_iter().map(|row| Task { scope, row }).collect())
    }

    pub async fn chats(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        Self::select(
            ctx,
            scope,
            "t.channel->>'source' IN ('web', 'slack') AND $2::uuid IS NULL AND $3::text IS NULL \
             ORDER BY t.created_at DESC, t.id",
            None,
            None,
        )
        .await
    }

    async fn related(
        ctx: &Context<'_>,
        scope: Scope,
        condition: &str,
        id: Uuid,
    ) -> Result<Vec<Self>> {
        Self::select(
            ctx,
            scope,
            &format!("$3::text IS NULL AND {condition}"),
            Some(id),
            None,
        )
        .await
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
        TaskStatus::from_db(&self.row.status)
    }

    pub async fn config_id(&self, ctx: &Context<'_>) -> Result<Uuid> {
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        Ok(
            sqlx::query_scalar::<_, Uuid>("SELECT config_id FROM sanchoris.triggers WHERE id = $1")
                .bind(self.row.trigger_id)
                .fetch_one(tx.conn())
                .await?,
        )
    }

    pub async fn session_data(&self, ctx: &Context<'_>) -> Result<Session> {
        let definition = self.definition()?;
        let config_id = self.config_id(ctx).await?;
        let name = definition.session.name;
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        let id = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM sanchoris.sessions WHERE config_id = $1 AND name = $2",
        )
        .bind(config_id)
        .bind(&name)
        .fetch_optional(tx.conn())
        .await?;
        Ok(Session {
            scope: self.scope,
            id: id.map_or_else(|| format!("{}/session", self.row.id), |id| id.to_string()),
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
        if let Some(event_id) = self.row.trigger_event_id
            && let Some(event) = TriggerEvent::find(ctx, self.scope, event_id).await?
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
        Run::query(ctx, self.scope, Some(self.row.id), None).await
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
            .as_deref()
            .map(WaitReason::from_db)
            .transpose()
    }

    async fn pending_reason(&self, ctx: &Context<'_>) -> Result<Option<PendingReason>> {
        if self.row.status != "PENDING" {
            return Ok(None);
        }
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        let row = sqlx::query_as::<_, (bool, bool)>(sql!(
            "SELECT \
               EXISTS (SELECT 1 FROM sanchoris.task_dependencies d JOIN sanchoris.tasks b ON b.id = d.blocked_by_task_id \
                       WHERE d.task_id = $1 AND b.status NOT IN {TERMINAL}), \
               (SELECT t.status <> 'active' OR t.tasks_this_period >= p.task_limit \
                       OR t.workspace_seconds_this_period >= p.workspace_seconds_limit \
                FROM sanchoris.tenants t JOIN sanchoris.plans p ON p.id = t.plan_id WHERE t.id = $2)"
        ))
        .bind(self.row.id)
        .bind(self.scope.tenant_id)
        .fetch_one(tx.conn())
        .await?;
        Ok(Some(match row {
            (true, _) => PendingReason::Blocked,
            (false, true) => PendingReason::PlanLimit,
            (false, false) => PendingReason::Concurrency,
        }))
    }

    async fn needs_you_reason(&self) -> Option<NeedsYouReason> {
        if self.row.status != "NEEDS_YOU" {
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

    async fn nudges(&self) -> i32 {
        self.row.nudges
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
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        Ok(sqlx::query_scalar::<_, i32>(
            "SELECT (SELECT count(*) FROM sanchoris.config_revisions p WHERE p.config_id = r.config_id \
                       AND (p.created_at, p.id) <= (r.created_at, r.id))::int \
             FROM sanchoris.config_revisions r WHERE r.id = $1",
        )
        .bind(self.row.config_revision_id)
        .fetch_one(tx.conn())
        .await?)
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
        Task::related(
            ctx,
            self.scope,
            "t.parent_task_id = $2 ORDER BY t.created_at, t.id",
            self.row.id,
        )
        .await
    }

    async fn blocked_by(&self, ctx: &Context<'_>) -> Result<Vec<Task>> {
        Task::related(
            ctx,
            self.scope,
            "t.id IN (SELECT blocked_by_task_id FROM sanchoris.task_dependencies WHERE task_id = $2) \
             ORDER BY t.created_at, t.id",
            self.row.id,
        )
        .await
    }

    async fn filed_tasks(&self) -> Vec<Task> {
        Vec::new()
    }

    async fn required_skills(&self, ctx: &Context<'_>) -> Result<Vec<RequiredSkillStatus>> {
        let definition = self.definition()?;
        let required = self.start()?.required_skills;
        let runs = SkillRun::query(ctx, self.scope, Some(self.row.id), None).await?;
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
        SkillRun::query(ctx, self.scope, Some(self.row.id), None).await
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
            .find(|run| run.row.number == number))
    }

    async fn outputs(&self, ctx: &Context<'_>) -> Result<Vec<Output>> {
        Output::query(ctx, self.scope, Some(self.row.id), None).await
    }

    async fn messages(&self, ctx: &Context<'_>) -> Result<Vec<ChatMessage>> {
        ChatMessage::for_task(ctx, self).await
    }

    async fn limits(&self) -> Result<TaskLimits> {
        Ok(TaskLimits(self.start()?.limits))
    }

    async fn turns(&self, ctx: &Context<'_>) -> Result<i32> {
        Ok(self.runs(ctx).await?.iter().map(|run| run.row.turns).sum())
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

#[derive(Debug, Clone, FromRow)]
pub(crate) struct RunRow {
    pub id: Uuid,
    pub task_id: Uuid,
    pub session_name: String,
    pub worker: String,
    pub model: String,
    pub agent_session_id: Option<String>,
    pub status: String,
    pub turns: i32,
    pub cost_usd: f64,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub number: i32,
}

pub(crate) struct Run {
    pub scope: Scope,
    pub row: RunRow,
}

impl Run {
    pub async fn query(
        ctx: &Context<'_>,
        scope: Scope,
        task_id: Option<Uuid>,
        run_id: Option<Uuid>,
    ) -> Result<Vec<Self>> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let rows = sqlx::query_as::<_, RunRow>(
            "SELECT r.id, r.task_id, r.session_name, r.worker, r.model, r.agent_session_id, r.status, r.turns, \
                    r.cost_usd::float8 AS cost_usd, r.summary, r.error, r.started_at, r.finished_at, r.updated_at, \
                    (SELECT count(*) FROM sanchoris.runs p WHERE p.task_id = r.task_id \
                       AND (p.started_at, p.id) <= (r.started_at, r.id))::int AS number \
             FROM sanchoris.runs r \
             WHERE r.tenant_id = $1 AND ($2::uuid IS NULL OR r.task_id = $2) AND ($3::uuid IS NULL OR r.id = $3) \
             ORDER BY r.started_at, r.id",
        )
        .bind(scope.tenant_id)
        .bind(task_id)
        .bind(run_id)
        .fetch_all(tx.conn())
        .await?;
        Ok(rows.into_iter().map(|row| Run { scope, row }).collect())
    }

    pub async fn load(ctx: &Context<'_>, scope: Scope, run_id: Uuid) -> Result<Self> {
        Self::query(ctx, scope, None, Some(run_id))
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| not_found(format!("run {run_id}")))
    }

    async fn log_lines(&self, ctx: &Context<'_>) -> Result<Vec<LogLine>> {
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        let mut entries: Vec<(DateTime<Utc>, u8, LogLineKind, String)> = Vec::new();
        let workspaces = sqlx::query_as::<_, (DateTime<Utc>, Option<DateTime<Utc>>, String, String)>(
            "SELECT started_at, stopped_at, platform, compute FROM sanchoris.workspaces WHERE run_id = $1 ORDER BY started_at, id",
        )
        .bind(self.row.id)
        .fetch_all(tx.conn())
        .await?;
        for (started, stopped, platform, compute) in workspaces {
            entries.push((
                started,
                0,
                LogLineKind::System,
                format!("workspace started · {platform} · {compute} · {WORKSPACE_VCPU} vCPU"),
            ));
            if let Some(stopped) = stopped {
                entries.push((
                    stopped,
                    0,
                    LogLineKind::System,
                    "workspace stopped".to_string(),
                ));
            }
        }
        let outbox = sqlx::query_as::<_, (DateTime<Utc>, String, Value)>(
            "SELECT created_at, type, payload FROM sanchoris.run_outbox WHERE run_id = $1 ORDER BY seq",
        )
        .bind(self.row.id)
        .fetch_all(tx.conn())
        .await?;
        for (at, kind, payload) in outbox {
            let text = |key: &str| payload[key].as_str().unwrap_or("").to_string();
            let line = match kind.as_str() {
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
            entries.push((at, 1, line.0, line.1));
        }
        let events = sqlx::query_as::<_, (DateTime<Utc>, String, Value, Option<String>)>(
            "SELECT received_at, type, payload, error FROM sanchoris.run_events WHERE run_id = $1 ORDER BY received_at, seq, id",
        )
        .bind(self.row.id)
        .fetch_all(tx.conn())
        .await?;
        for (at, kind, payload, error) in events {
            let line = match kind.as_str() {
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
                entries.push((at, 2, kind, text));
            }
            if let Some(error) = error {
                entries.push((at, 3, LogLineKind::Error, error));
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

    async fn number(&self) -> i32 {
        self.row.number
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

    async fn turns(&self) -> i32 {
        self.row.turns
    }

    async fn cost_usd(&self) -> f64 {
        self.row.cost_usd
    }

    async fn summary(&self) -> Option<&str> {
        self.row.summary.as_deref()
    }

    async fn workspace(&self, ctx: &Context<'_>) -> Result<Workspace> {
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        let row = sqlx::query_as::<_, (String, String, Value, String, Option<String>)>(
            "SELECT platform, compute, repositories, branch, base_commit FROM sanchoris.workspaces \
             WHERE run_id = $1 ORDER BY started_at DESC, id DESC LIMIT 1",
        )
        .bind(self.row.id)
        .fetch_optional(tx.conn())
        .await?;
        drop(tx);
        if let Some((platform, compute, repositories, branch, base_commit)) = row {
            return Ok(Workspace {
                platform,
                compute: WorkspaceCompute::from_db(&compute)?,
                repositories: repositories
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
                branch,
                base_commit: base_commit.unwrap_or_default(),
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
        SkillRun::query(ctx, self.scope, None, Some(self.row.id)).await
    }

    async fn outputs(&self, ctx: &Context<'_>) -> Result<Vec<Output>> {
        Ok(Output::query(ctx, self.scope, Some(self.row.task_id), None)
            .await?
            .into_iter()
            .filter(|output| output.run_id == Some(self.row.id))
            .collect())
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

#[derive(Debug, Clone, FromRow)]
pub(crate) struct OutputRow {
    id: Uuid,
    run_id: Option<Uuid>,
    kind: String,
    repository: String,
    number: i32,
    url: String,
    state: Option<String>,
    created_at: DateTime<Utc>,
}

pub(crate) struct Output {
    run_id: Option<Uuid>,
    row: OutputRow,
}

impl Output {
    pub async fn query(
        ctx: &Context<'_>,
        scope: Scope,
        task_id: Option<Uuid>,
        skill_run_id: Option<Uuid>,
    ) -> Result<Vec<Self>> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let rows = sqlx::query_as::<_, OutputRow>(
            "SELECT o.id, s.run_id, o.kind, o.repository, o.number, o.url, o.state, o.created_at \
             FROM sanchoris.outputs o JOIN sanchoris.skill_runs s ON s.id = o.skill_run_id \
             WHERE o.tenant_id = $1 AND ($2::uuid IS NULL OR s.task_id = $2) AND ($3::uuid IS NULL OR o.skill_run_id = $3) \
             ORDER BY o.created_at, o.id",
        )
        .bind(scope.tenant_id)
        .bind(task_id)
        .bind(skill_run_id)
        .fetch_all(tx.conn())
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| Output {
                run_id: row.run_id,
                row,
            })
            .collect())
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

    async fn number(&self) -> i32 {
        self.row.number
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
        let mut tx = tenant_tx(ctx, task.scope.tenant_id).await?;
        let started = match task.row.trigger_event_id {
            Some(event_id) => sqlx::query_scalar::<_, DateTime<Utc>>(
                "SELECT i.received_at FROM sanchoris.trigger_events e \
                 JOIN sanchoris.inbound_events i ON i.tenant_id = e.tenant_id AND i.key = e.key AND i.source = $2 \
                 WHERE e.id = $1",
            )
            .bind(event_id)
            .bind(kind.as_db())
            .fetch_optional(tx.conn())
            .await?
            .unwrap_or(task.row.created_at),
            None => task.row.created_at,
        };
        let next = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT created_at FROM sanchoris.tasks WHERE trigger_id = $1 AND channel->>'thread' = $2 \
               AND created_at > $3 ORDER BY created_at LIMIT 1",
        )
        .bind(task.row.trigger_id)
        .bind(&thread)
        .bind(task.row.created_at)
        .fetch_optional(tx.conn())
        .await?;
        let inbound = sqlx::query_as::<_, (Uuid, DateTime<Utc>, Value)>(
            "SELECT id, received_at, event FROM sanchoris.inbound_events \
             WHERE tenant_id = $1 AND source = $2 AND event->'detail'->>'thread' = $3 \
               AND received_at >= $4 AND ($5::timestamptz IS NULL OR received_at < $5) \
             ORDER BY received_at, id",
        )
        .bind(task.scope.tenant_id)
        .bind(kind.as_db())
        .bind(&thread)
        .bind(started)
        .bind(next)
        .fetch_all(tx.conn())
        .await?;
        let posts = sqlx::query_as::<_, (Uuid, DateTime<Utc>, Value)>(
            "SELECT o.id, o.created_at, o.payload FROM sanchoris.run_outbox o JOIN sanchoris.runs r ON r.id = o.run_id \
             WHERE r.task_id = $1 AND o.type = 'chat_post' ORDER BY o.created_at, o.seq",
        )
        .bind(task.row.id)
        .fetch_all(tx.conn())
        .await?;
        let mut messages = inbound
            .into_iter()
            .map(|(id, at, event)| ChatMessage {
                scope: task.scope,
                id,
                author_kind: ChatAuthorKind::Member,
                member_id: event["detail"]["member_id"]
                    .as_str()
                    .and_then(|id| id.parse().ok()),
                text: event["detail"]["text"].as_str().unwrap_or("").to_string(),
                at,
            })
            .chain(posts.into_iter().map(|(id, at, payload)| ChatMessage {
                scope: task.scope,
                id,
                author_kind: ChatAuthorKind::Agent,
                member_id: None,
                text: payload["text"].as_str().unwrap_or("").to_string(),
                at,
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
