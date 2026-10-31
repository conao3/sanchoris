use std::collections::{BTreeSet, VecDeque};

use async_graphql::{Context, ID, InputObject, Object, Result};
use chrono::{DateTime, Utc};
use sanchoris_config::Start;
use sanchoris_core::db::{self, REF_CONFIG_REVISION, REF_RUN};
use sanchoris_core::items::{
    ApprovalWait, Child, ConfigRevision as RevisionItem, Dependency, OUTPUT_ISSUE, OutboxItem,
    Output as OutputItem, Run as RunItem, RunEventItem, SkillRunItem, Task as TaskItem, TaskName,
    TaskOrigin, Workspace as WorkspaceItem, github_task_name,
};
use sanchoris_core::keys;
use sanchoris_core::model::{TaskDefinition, TaskStatus as CoreStatus};
use sanchoris_core::store::{Index, Listing};
use sanchoris_core::tasks::{linked_member, task_branch, task_config_id};
use serde_json::Value;
use uuid::Uuid;

use crate::account::Member;
use crate::config::Session;
use crate::context::{Scope, now, tx};
use crate::conversation::Conversation;
use crate::enums::{
    LogLineKind, NeedsYouReason, OutputKind, OutputState, PendingReason, Provider, RequesterKind,
    RequiredSkillState, RunStatus, TaskStatus, WaitReason, WorkspaceArchitecture, WorkspaceCompute,
};
use crate::error::{not_found, parse_id};
use crate::scalars::{Timestamp, ts, ts_opt};
use crate::skill::{Skill, SkillRun};
use crate::tenant::{current_name, tenant_repositories};
use crate::trigger::{TaskLimits, Trigger, TriggerEvent};

#[derive(InputObject, Default)]
pub(crate) struct TaskFilter {
    pub statuses: Option<Vec<TaskStatus>>,
    #[graphql(name = "triggerId")]
    pub trigger_id: Option<ID>,
    pub skill_name: Option<String>,
    /// Only tasks that are not finished.
    pub active: Option<bool>,
    /// Only tasks that are snoozed (true) or that are not (false).
    pub snoozed: Option<bool>,
}

pub(crate) struct Task {
    pub scope: Scope,
    pub row: TaskItem,
}

pub(crate) const TASK_LISTING_LIMIT: usize = 200;

pub(crate) fn latest_tasks(tenant_id: Uuid) -> Listing {
    Listing::index(Index::Gsi1, keys::tasks_index(tenant_id))
        .descending()
        .limit(TASK_LISTING_LIMIT)
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
            .list::<TaskItem>(&latest_tasks(scope.tenant_id), keep)
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
        let at = now(ctx);
        let mut tasks = Self::listed(ctx, scope, |task| {
            statuses
                .as_ref()
                .is_none_or(|statuses| statuses.iter().any(|s| s == task.status.as_str()))
                && trigger_id.is_none_or(|id| task.origin.trigger_id() == Some(id))
                && filter
                    .skill_name
                    .as_deref()
                    .is_none_or(|name| offers_skill(task, name))
                && filter
                    .active
                    .is_none_or(|active| task.status.is_terminal() != active)
                && filter
                    .snoozed
                    .is_none_or(|snoozed| task.is_snoozed(at) == snoozed)
        })
        .await?;
        tasks.sort_by(|a, b| {
            let needs_you =
                |task: &Task| task.row.status == CoreStatus::NeedsYou && !task.row.is_snoozed(at);
            (needs_you(b), b.row.updated_at, a.row.id).cmp(&(
                needs_you(a),
                a.row.updated_at,
                b.row.id,
            ))
        });
        Ok(tasks)
    }

    pub(crate) async fn found(
        ctx: &Context<'_>,
        scope: Scope,
        ids: Vec<Uuid>,
    ) -> Result<Vec<Self>> {
        let mut tasks = Vec::new();
        for id in ids {
            tasks.push(Self::load(ctx, scope, id).await?);
        }
        tasks.sort_by_key(|a| (a.row.created_at, a.row.id));
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
        Ok(self.definition()?.start)
    }

    pub fn status_value(&self) -> Result<TaskStatus> {
        TaskStatus::from_db(self.row.status.as_str())
    }

    pub async fn config_id(&self, ctx: &Context<'_>) -> Result<Uuid> {
        let mut tx = tx(ctx, self.scope.tenant_id);
        Ok(task_config_id(&mut tx, &self.row).await?)
    }

    pub async fn session_data(&self, ctx: &Context<'_>) -> Result<Session> {
        let definition = self.definition()?;
        let config_id = self.config_id(ctx).await?;
        let name = definition.session.name;
        let current = Session::by_name(ctx, self.scope, config_id, &name).await?;
        Ok(Session {
            scope: self.scope,
            id: current
                .as_ref()
                .map_or_else(|| format!("{}/session", self.row.id), |s| s.id.clone()),
            config_id,
            name,
            definition: definition.session.definition,
            chat_skills: current.map(|s| s.chat_skills).unwrap_or_default(),
        })
    }

    async fn trigger_event(&self, ctx: &Context<'_>) -> Result<Option<TriggerEvent>> {
        match self.row.origin.trigger_event_id() {
            Some(id) => TriggerEvent::find(ctx, self.scope, id).await,
            None => Ok(None),
        }
    }

    pub async fn runs(&self, ctx: &Context<'_>) -> Result<Vec<Run>> {
        let mut rows = tx(ctx, self.scope.tenant_id)
            .query::<RunItem>(&self.pk(), "RUN#")
            .await?;
        rows.sort_by_key(|a| (a.started_at, a.id));
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

    /// The idempotency key the trigger chose.
    async fn name(&self, ctx: &Context<'_>) -> Result<String> {
        Ok(self
            .row
            .display_name(&tenant_repositories(ctx, self.scope.tenant_id).await?))
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
            || (self.row.started_at.is_none()
                && plan
                    .task_limit
                    .is_some_and(|limit| tenant.tasks_this_period >= limit))
            || plan
                .workspace_seconds_limit
                .is_some_and(|limit| tenant.workspace_seconds_this_period >= limit);
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
            None => NeedsYouReason::AgentAsked,
            Some(sanchoris_core::tasks::ERROR_APPROVAL_REQUESTED) => {
                NeedsYouReason::ApprovalRequested
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
            Some(sanchoris_core::lifecycle::ERROR_WORKSPACE_STOPPED) => {
                NeedsYouReason::WorkspaceStopped
            }
            Some(_) => NeedsYouReason::SkillFailed,
        })
    }

    /// The approval that the task's skill waits for; null when none waits.
    async fn approval(&self, ctx: &Context<'_>) -> Result<Option<Approval>> {
        if self.row.status.is_terminal() {
            return Ok(None);
        }
        let mut tx = tx(ctx, self.scope.tenant_id);
        Ok(waiting_approval(&mut tx, self.row.id).await?.map(Approval))
    }

    /// When a usage-limited task resumes.
    async fn resume_at(&self) -> Option<Timestamp> {
        ts_opt(self.row.resume_at)
    }

    async fn auto_resume(&self) -> Result<bool> {
        Ok(self.definition()?.session.definition["UsageLimit"]["Resume"] == "auto")
    }

    /// Until when the task is out of the needs-you lists; null when it is not snoozed.
    async fn snoozed_until(&self, ctx: &Context<'_>) -> Option<Timestamp> {
        ts_opt(
            self.row
                .snoozed_until
                .filter(|_| self.row.is_snoozed(now(ctx))),
        )
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

    /// The trigger that started the task. Null for a conversation's task.
    async fn trigger(&self, ctx: &Context<'_>) -> Result<Option<Trigger>> {
        match self.row.origin.trigger_id() {
            Some(trigger_id) => Ok(Some(Trigger::load(ctx, self.scope, trigger_id).await?)),
            None => Ok(None),
        }
    }

    /// The conversation whose session the task is. Null for a trigger's task.
    async fn conversation(&self, ctx: &Context<'_>) -> Result<Option<Conversation>> {
        match self.row.origin.conversation_id() {
            Some(id) => Ok(Some(Conversation::load(ctx, self.scope, id).await?)),
            None => Ok(None),
        }
    }

    /// The task's place in its conversation, from 1. Null for a trigger's task.
    async fn conversation_sequence(&self) -> Result<Option<i32>> {
        match self.row.origin {
            TaskOrigin::Conversation { sequence, .. } => Ok(Some(i32::try_from(sequence)?)),
            TaskOrigin::Trigger { .. } => Ok(None),
        }
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
                github_user_id: None,
                github_login: None,
                task: None,
            }));
        }
        let (Some(github_user_id), Some(github_login)) = (
            self.row.requested_by_github_user_id,
            self.row.requested_by_github_login.clone(),
        ) else {
            return Ok(None);
        };
        if github_login.ends_with("[bot]") {
            let task = match self.row.parent_task_id {
                Some(parent) => Task::find(ctx, self.scope, parent).await?,
                None => None,
            };
            return Ok(Some(Requester {
                kind: RequesterKind::Agent,
                member: None,
                github_user_id: Some(github_user_id),
                github_login: Some(github_login),
                task,
            }));
        }
        let member = match linked_member(&mut tx(ctx, self.scope.tenant_id), github_user_id).await?
        {
            Some(member_id) => Member::find(ctx, self.scope, member_id).await?,
            None => None,
        };
        Ok(Some(Requester {
            kind: RequesterKind::Github,
            member,
            github_user_id: Some(github_user_id),
            github_login: Some(github_login),
            task: None,
        }))
    }

    async fn session(&self, ctx: &Context<'_>) -> Result<Session> {
        self.session_data(ctx).await
    }

    /// The config revision pinned when the task started.
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
        let repositories = tenant_repositories(ctx, self.scope.tenant_id).await?;
        Ok(IssueRef::from_detail(
            self.scope,
            &event.event()["detail"],
            &repositories,
        ))
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

    /// Tasks started from issues this task filed, with their descendants.
    async fn filed_tasks(&self, ctx: &Context<'_>) -> Result<Vec<Task>> {
        filed_tasks_of(ctx, self.scope, &[self.row.id]).await
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
                &offered.definition["Definition"],
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

pub(crate) async fn waiting_approval(
    tx: &mut db::TenantTx,
    task_id: Uuid,
) -> Result<Option<ApprovalWait>> {
    Ok(tx
        .query::<ApprovalWait>(&keys::task_pk(tx.tenant_id(), task_id), "SR#")
        .await?
        .into_iter()
        .filter(|wait| wait.status == "waiting")
        .max_by_key(|wait| wait.created_at))
}

pub(crate) struct Approval(ApprovalWait);

#[Object]
impl Approval {
    async fn message(&self) -> &str {
        &self.0.message
    }

    async fn requested_at(&self) -> Timestamp {
        ts(self.0.created_at)
    }
}

pub(crate) struct IssueRef {
    scope: Scope,
    repository: String,
    repository_id: i64,
    number: i32,
    title: String,
    url: String,
}

impl IssueRef {
    fn from_detail(
        scope: Scope,
        detail: &Value,
        repositories: &[sanchoris_core::items::GitHubRepository],
    ) -> Option<Self> {
        let issue = detail.get("issue")?;
        let repository_id = detail["repository_id"].as_i64()?;
        Some(Self {
            scope,
            repository: current_name(repositories, repository_id, detail["repository"].as_str()?),
            repository_id,
            number: i32::try_from(issue["number"].as_i64()?).ok()?,
            title: issue["title"].as_str()?.to_string(),
            url: issue["url"].as_str()?.to_string(),
        })
    }

    pub(crate) fn from_output(
        scope: Scope,
        output: &OutputItem,
        repositories: &[sanchoris_core::items::GitHubRepository],
    ) -> Result<Self> {
        Ok(Self {
            scope,
            repository: current_name(repositories, output.repository_id, &output.repository),
            repository_id: output.repository_id,
            number: i32::try_from(output.number)?,
            title: output.title.clone().ok_or_else(|| {
                async_graphql::Error::new(format!("issue output {} has no title", output.id))
            })?,
            url: output.url.clone(),
        })
    }
}

pub(crate) async fn issue_outputs(
    ctx: &Context<'_>,
    scope: Scope,
    task_id: Uuid,
) -> Result<Vec<OutputItem>> {
    let mut rows = tx(ctx, scope.tenant_id)
        .query::<OutputItem>(&keys::task_pk(scope.tenant_id, task_id), "SR#")
        .await?
        .into_iter()
        .filter(|output| output.kind == OUTPUT_ISSUE)
        .collect::<Vec<_>>();
    rows.sort_by_key(|a| (a.created_at, a.id));
    Ok(rows)
}

pub(crate) async fn filed_tasks_of(
    ctx: &Context<'_>,
    scope: Scope,
    task_ids: &[Uuid],
) -> Result<Vec<Task>> {
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    for task_id in task_ids {
        for output in issue_outputs(ctx, scope, *task_id).await? {
            let name = github_task_name(output.repository_id, &format!("#{}", output.number));
            if let Some(task) = Task::by_name(ctx, scope, &name).await?
                && seen.insert(task.row.id)
            {
                queue.push_back(task.row.id);
                found.push(task);
            }
        }
    }
    while let Some(parent) = queue.pop_front() {
        let children = tx(ctx, scope.tenant_id)
            .query::<Child>(&keys::task_pk(scope.tenant_id, parent), "CHILD#")
            .await?;
        for child in children {
            if seen.insert(child.task_id) {
                queue.push_back(child.task_id);
                found.push(Task::load(ctx, scope, child.task_id).await?);
            }
        }
    }
    found.sort_by_key(|a| (a.row.created_at, a.row.id));
    Ok(found)
}

#[Object]
impl IssueRef {
    /// owner/name
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

    /// The task the issue started, if any.
    async fn task(&self, ctx: &Context<'_>) -> Result<Option<Task>> {
        Task::by_name(
            ctx,
            self.scope,
            &sanchoris_core::items::github_task_name(
                self.repository_id,
                &format!("#{}", self.number),
            ),
        )
        .await
    }
}

pub(crate) struct Requester {
    kind: RequesterKind,
    member: Option<Member>,
    github_user_id: Option<i64>,
    github_login: Option<String>,
    task: Option<Task>,
}

#[Object]
impl Requester {
    async fn kind(&self) -> RequesterKind {
        self.kind
    }

    /// The member who asked, or for a GitHub user the member who linked that GitHub account in this tenant. Null when no member linked it.
    async fn member(&self) -> Option<&Member> {
        self.member.as_ref()
    }

    /// The GitHub user id of the sender of the event that started the task. Null for a conversation's task.
    async fn github_user_id(&self) -> Option<ID> {
        self.github_user_id.map(|id| ID(id.to_string()))
    }

    /// The GitHub login of the sender of the event that started the task, as of the event. Null for a conversation's task.
    async fn github_login(&self) -> Option<&str> {
        self.github_login.as_deref()
    }

    /// The task whose agent filed the issue.
    async fn task(&self) -> Option<&Task> {
        self.task.as_ref()
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
        workspaces.sort_by_key(|a| (a.started_at, a.id));
        Ok(workspaces)
    }

    async fn log_lines(&self, ctx: &Context<'_>) -> Result<Vec<LogLine>> {
        let mut entries: Vec<LogEntry> = Vec::new();
        for workspace in self.workspaces(ctx).await? {
            entries.push((
                workspace.started_at,
                0,
                LogLineKind::System,
                format!(
                    "workspace started · {} · {} · {} vCPU",
                    workspace.platform,
                    workspace.compute,
                    workspace.cpu / 1024
                ),
                None,
            ));
            if let Some(stopped) = workspace.stopped_at {
                entries.push((
                    stopped,
                    0,
                    LogLineKind::System,
                    "workspace stopped".to_string(),
                    None,
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
                other => (LogLineKind::System, other.to_string()),
            };
            entries.push((outbox.created_at, 1, line.0, line.1, None));
        }
        let mut events = tx.query::<RunEventItem>(&run_pk, "EVT#").await?;
        events.sort_by_key(|a| (a.received_at, a.seq, a.id));
        for event in events {
            let payload = &event.payload;
            if event.kind == "rate_limit" {
                let status = payload["status"].as_str().ok_or_else(|| {
                    async_graphql::Error::new(format!(
                        "rate_limit event {} has no status",
                        event.id
                    ))
                })?;
                entries.push((
                    event.received_at,
                    2,
                    LogLineKind::Event,
                    format!("usage limit status: {status}"),
                    Some(LogRateLimit {
                        status: status.to_string(),
                        resets_at: payload["resetsAt"]
                            .as_i64()
                            .and_then(|seconds| DateTime::from_timestamp(seconds, 0)),
                    }),
                ));
            }
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
                entries.push((event.received_at, 2, kind, text, None));
            }
            if let Some(error) = event.error {
                entries.push((event.received_at, 3, LogLineKind::Error, error, None));
            }
        }
        if let Some(error) = &self.row.error {
            entries.push((
                self.row.finished_at.unwrap_or(self.row.updated_at),
                4,
                LogLineKind::Error,
                error.clone(),
                None,
            ));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        Ok(entries
            .into_iter()
            .enumerate()
            .map(|(index, (at, _, kind, text, rate_limit))| LogLine {
                sequence: i32::try_from(index + 1).expect("log lines fit in i32"),
                at,
                kind,
                text,
                rate_limit,
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
                architecture: WorkspaceArchitecture::from_db(&latest.architecture)?,
                capacity_provider: latest.capacity_provider,
                cpu: latest.cpu,
                memory_mib: latest.memory_mib,
                repositories: latest.repositories,
                branch: latest.branch,
                base_commit: latest.base_commit.unwrap_or_default(),
            });
        }
        let task = Task::load(ctx, self.scope, self.row.task_id).await?;
        let session = task.session_data(ctx).await?;
        let parsed = session.parsed()?;
        let (_, plan) = db::tenant_plan(&mut tx(ctx, self.scope.tenant_id)).await?;
        Ok(Workspace {
            platform: "pending".to_string(),
            compute: WorkspaceCompute::from_db(parsed.workspace.compute.as_str())?,
            architecture: WorkspaceArchitecture::from_db(parsed.workspace.architecture.as_str())?,
            capacity_provider: None,
            cpu: plan.workspace_cpu,
            memory_mib: plan.workspace_memory_mib,
            repositories: session.repository_names(ctx).await?,
            branch: task_branch(task.row.id),
            base_commit: String::new(),
        })
    }

    /// Log lines ordered by sequence, after the given sequence.
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
    architecture: WorkspaceArchitecture,
    capacity_provider: Option<String>,
    cpu: i64,
    memory_mib: i64,
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

    async fn architecture(&self) -> WorkspaceArchitecture {
        self.architecture
    }

    /// The ECS capacity provider the workspace runs on (FARGATE_SPOT or FARGATE); null before it launches and for local workspaces.
    async fn capacity_provider(&self) -> Option<&str> {
        self.capacity_provider.as_deref()
    }

    async fn vcpu(&self) -> Result<i32> {
        Ok(i32::try_from(self.cpu / 1024)?)
    }

    async fn memory_mib(&self) -> Result<i32> {
        Ok(i32::try_from(self.memory_mib)?)
    }

    /// Repository names cloned into the workspace.
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

type LogEntry = (DateTime<Utc>, u8, LogLineKind, String, Option<LogRateLimit>);

pub(crate) struct LogLine {
    sequence: i32,
    at: DateTime<Utc>,
    kind: LogLineKind,
    text: String,
    rate_limit: Option<LogRateLimit>,
}

pub(crate) struct LogRateLimit {
    status: String,
    resets_at: Option<DateTime<Utc>>,
}

#[Object]
impl LogRateLimit {
    /// The provider's status: allowed, allowed_warning, or rejected; rejected means the usage limit was reached.
    async fn status(&self) -> &str {
        &self.status
    }

    async fn resets_at(&self) -> Option<Timestamp> {
        ts_opt(self.resets_at)
    }
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

    /// May span lines.
    async fn text(&self) -> &str {
        &self.text
    }

    /// Set on the usage-limit snapshots that the worker reports.
    async fn rate_limit(&self) -> Option<&LogRateLimit> {
        self.rate_limit.as_ref()
    }
}

pub(crate) struct Output {
    row: OutputItem,
}

impl Output {
    fn sorted(mut rows: Vec<OutputItem>) -> Vec<Self> {
        rows.sort_by_key(|a| (a.created_at, a.id));
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

    /// owner/name
    async fn repository(&self, ctx: &Context<'_>) -> Result<String> {
        Ok(current_name(
            &tenant_repositories(ctx, self.row.tenant_id).await?,
            self.row.repository_id,
            &self.row.repository,
        ))
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
