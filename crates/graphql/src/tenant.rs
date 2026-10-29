use async_graphql::{Context, ID, Object, Result};
use chrono::{DateTime, Datelike, Duration, Months, TimeZone, Utc};
use sanchoris_core::item::format_time;
use sanchoris_core::items::{
    self, Integration as IntegrationItem, Output as OutputItem, Project as ProjectItem,
    Run as RunItem, Task as TaskItem, Trigger as TriggerItem, UsageRecord,
    Workspace as WorkspaceItem, github_repositories,
};
use sanchoris_core::keys;
use sanchoris_core::store::{Index, Listing, SortKey};
use serde_json::Value;
use uuid::Uuid;

use crate::account::{Credential, Invitation, Member, Plan};
use crate::audit::{AuditEvent, AuditFilter};
use crate::config::{ChangeRequest, Config, Project, Session};
use crate::context::{Access, Scope, now, tx, viewer_user};
use crate::enums::{
    ChangeRequestState, GitHubOwnerKind, IntegrationKind, IntegrationStatus, InvitationState,
    LimitPeriod, TenantState,
};
use crate::error::{not_found, parse_id};
use crate::scalars::{Timestamp, ts, ts_opt};
use crate::skill::Skill;
use crate::task::{Task, TaskFilter};
use crate::trigger::{Trigger, TriggerEvent};

pub(crate) struct Tenant {
    pub scope: Scope,
    pub row: items::Tenant,
}

pub(crate) fn midnight(at: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(at.year(), at.month(), at.day(), 0, 0, 0)
        .single()
        .expect("midnight is unambiguous in UTC")
}

impl Tenant {
    pub async fn load(ctx: &Context<'_>, scope: Scope) -> Result<Self> {
        let row = tx(ctx, scope.tenant_id)
            .get::<items::Tenant>(&keys::tenant_key(scope.tenant_id))
            .await?
            .ok_or_else(|| not_found(format!("tenant {}", scope.tenant_id)))?;
        Ok(Self { scope, row })
    }

    async fn integration_rows(&self, ctx: &Context<'_>) -> Result<Vec<IntegrationItem>> {
        let mut rows = tx(ctx, self.scope.tenant_id)
            .query::<IntegrationItem>(&keys::tenant_pk(self.scope.tenant_id), "INTEG#")
            .await?;
        rows.sort_by(|a, b| (&a.kind, a.created_at, a.id).cmp(&(&b.kind, b.created_at, b.id)));
        Ok(rows)
    }

    pub async fn setup_state(&self, ctx: &Context<'_>) -> Result<Setup> {
        let tenant_id = self.scope.tenant_id;
        let github_linked = viewer_user(ctx).await?.github_login.is_some();
        let integrations = self.integration_rows(ctx).await?;
        let connected = |kind: &str| {
            integrations
                .iter()
                .any(|row| row.kind == kind && row.status == "connected")
        };
        let mut tx = tx(ctx, tenant_id);
        let configs = tx
            .query::<items::Config>(&keys::tenant_pk(tenant_id), "CONFIG#")
            .await?;
        let credential_connected = tx
            .query::<items::Credential>(&keys::tenant_pk(tenant_id), "CRED#")
            .await?
            .iter()
            .any(|credential| credential.status == "connected");
        let mut trigger_enabled = false;
        let mut project_created = false;
        for config in &configs {
            let pk = keys::config_pk(tenant_id, config.id);
            trigger_enabled |= tx
                .query::<TriggerItem>(&pk, "TRIGGER#")
                .await?
                .iter()
                .filter_map(|trigger| trigger.in_revision(config.active_revision_id))
                .any(|trigger| trigger.is_active());
            if let Some(revision_id) = config.active_revision_id {
                project_created |= !tx
                    .query::<ProjectItem>(&pk, &keys::projects_prefix(revision_id))
                    .await?
                    .is_empty();
            }
        }
        let first_task_started = !tx
            .list::<TaskItem>(
                &Listing::index(Index::Gsi1, keys::tasks_index(tenant_id)).limit(1),
                |_| true,
            )
            .await?
            .is_empty();
        Ok(Setup {
            github_linked,
            app_installed: connected("github_installation"),
            config_added: !configs.is_empty(),
            credential_connected,
            trigger_enabled,
            first_task_started,
            project_created,
            slack_connected: connected("slack"),
        })
    }
}

#[Object]
impl Tenant {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn slug(&self) -> &str {
        &self.row.slug
    }

    async fn name(&self) -> &str {
        &self.row.name
    }

    async fn plan(&self, ctx: &Context<'_>) -> Result<Plan> {
        self.scope.operator_or_member()?;
        Plan::load(ctx, &self.row.plan_id).await
    }

    async fn state(&self) -> Result<TenantState> {
        self.scope.operator_or_member()?;
        TenantState::from_db(&self.row.status)
    }

    async fn suspended_reason(&self) -> Result<Option<&str>> {
        self.scope.operator_or_member()?;
        Ok(self.row.suspended_reason.as_deref())
    }

    async fn created_at(&self) -> Result<Timestamp> {
        self.scope.operator_or_member()?;
        Ok(ts(self.row.created_at))
    }

    async fn github_owner(&self, ctx: &Context<'_>) -> Result<Option<GitHubOwner>> {
        self.scope.operator_or_member()?;
        let rows = self.integration_rows(ctx).await?;
        let Some(row) = rows
            .iter()
            .find(|row| row.kind == "github_installation" && row.status == "connected")
        else {
            return Ok(None);
        };
        let login = detail_str(row, "account_login").ok_or_else(|| {
            async_graphql::Error::new("the GitHub installation has no details.account_login")
        })?;
        let kind = match detail_str(row, "account_type") {
            Some("Organization") => GitHubOwnerKind::Organization,
            _ => GitHubOwnerKind::User,
        };
        Ok(Some(GitHubOwner {
            kind,
            login: login.to_string(),
        }))
    }

    async fn viewer_member(&self, ctx: &Context<'_>) -> Result<Member> {
        let (member_id, _) = self.scope.member()?;
        Member::load(ctx, self.scope, member_id).await
    }

    async fn usage(&self, ctx: &Context<'_>) -> Result<TenantUsage> {
        self.scope.operator_or_member()?;
        let plan = Plan::load(ctx, &self.row.plan_id).await?;
        let seats = seats_used(ctx, self.scope.tenant_id, now(ctx)).await?;
        let running = running_runs(ctx, self.scope.tenant_id).await?;
        let period_end = match plan.period()? {
            LimitPeriod::Month => Some(
                self.row
                    .period_start
                    .checked_add_months(Months::new(1))
                    .expect("the period end is representable"),
            ),
            LimitPeriod::Total => None,
        };
        Ok(TenantUsage {
            period_start: self.row.period_start,
            period_end,
            tasks_this_period: i32::try_from(self.row.tasks_this_period)?,
            workspace_seconds_this_period: i32::try_from(self.row.workspace_seconds_this_period)?,
            seats_used: i32::try_from(seats)?,
            running_runs: running,
        })
    }

    async fn activity(&self, ctx: &Context<'_>) -> Result<Activity> {
        self.scope.member()?;
        let tenant_id = self.scope.tenant_id;
        let at = now(ctx);
        let today = format_time(midnight(at));
        let since = at - Duration::hours(24);
        let tx = tx(ctx, tenant_id);
        let earliest = format_time(since.min(midnight(at)));
        let runs = tx
            .list::<RunItem>(
                &Listing::index(Index::Gsi1, keys::runs_index(tenant_id))
                    .sort(SortKey::AtLeast(earliest)),
                |_| true,
            )
            .await?;
        let runs_today = runs
            .iter()
            .filter(|run| format_time(run.started_at) >= today)
            .count();
        let cost_today = tx
            .list::<UsageRecord>(
                &Listing::index(Index::Gsi1, keys::usage_index(tenant_id))
                    .sort(SortKey::AtLeast(today.clone())),
                |_| true,
            )
            .await?
            .iter()
            .map(|usage| usage.cost_usd)
            .sum();
        let pull_requests_today = tx
            .list::<OutputItem>(
                &Listing::index(Index::Gsi1, keys::outputs_index(tenant_id))
                    .sort(SortKey::AtLeast(today.clone())),
                |output| output.kind == "pull_request",
            )
            .await?
            .len();
        let mut buckets = vec![0; 24];
        for run in runs
            .iter()
            .filter(|run| run.started_at > since && run.started_at <= at)
        {
            let index =
                usize::try_from((run.started_at - since).num_seconds().saturating_sub(1) / 3600)
                    .expect("runs in the window are after its start");
            buckets[index.min(23)] += 1;
        }
        Ok(Activity {
            running_tasks: i32::try_from(self.row.running_tasks)?,
            runs_today: i32::try_from(runs_today)?,
            cost_today_usd: cost_today,
            pull_requests_opened_today: i32::try_from(pull_requests_today)?,
            runs_last_24h: buckets,
        })
    }

    async fn setup(&self, ctx: &Context<'_>) -> Result<Setup> {
        self.scope.member()?;
        self.setup_state(ctx).await
    }

    async fn members(&self, ctx: &Context<'_>) -> Result<Vec<Member>> {
        self.scope.member()?;
        Member::all(ctx, self.scope).await
    }

    async fn invitations(
        &self,
        ctx: &Context<'_>,
        state: Option<InvitationState>,
    ) -> Result<Vec<Invitation>> {
        self.scope.member()?;
        let at = now(ctx);
        let mut invitations = Vec::new();
        for invitation in Invitation::all(ctx, self.scope).await? {
            if state.is_none() || Some(invitation.state_at(at)?) == state {
                invitations.push(invitation);
            }
        }
        Ok(invitations)
    }

    async fn integrations(&self, ctx: &Context<'_>) -> Result<Vec<Integration>> {
        self.scope.member()?;
        let rows = self.integration_rows(ctx).await?;
        let mut integrations = Vec::new();
        for kind in [IntegrationKind::GithubInstallation, IntegrationKind::Slack] {
            let of_kind = rows
                .iter()
                .filter(|row| row.kind == kind.as_db())
                .cloned()
                .collect::<Vec<_>>();
            if of_kind.is_empty() {
                integrations.push(Integration::NotConnected {
                    tenant_id: self.row.id,
                    kind,
                });
            }
            integrations.extend(of_kind.into_iter().map(Integration::Row));
        }
        Ok(integrations)
    }

    async fn credentials(&self, ctx: &Context<'_>) -> Result<Vec<Credential>> {
        Credential::visible(ctx, self.scope).await
    }

    async fn configs(&self, ctx: &Context<'_>) -> Result<Vec<Config>> {
        self.scope.member()?;
        Config::all(ctx, self.scope).await
    }

    async fn projects(&self, ctx: &Context<'_>) -> Result<Vec<Project>> {
        self.scope.member()?;
        Project::all(ctx, self.scope).await
    }

    async fn sessions(&self, ctx: &Context<'_>) -> Result<Vec<Session>> {
        self.scope.member()?;
        Session::all(ctx, self.scope).await
    }

    async fn change_requests(
        &self,
        ctx: &Context<'_>,
        state: Option<ChangeRequestState>,
    ) -> Result<Vec<ChangeRequest>> {
        self.scope.member()?;
        ChangeRequest::query(ctx, self.scope, None, state).await
    }

    async fn triggers(&self, ctx: &Context<'_>) -> Result<Vec<Trigger>> {
        self.scope.member()?;
        Trigger::all(ctx, self.scope).await
    }

    async fn trigger_events(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 20)] limit: Option<i32>,
    ) -> Result<Vec<TriggerEvent>> {
        self.scope.member()?;
        TriggerEvent::recent(ctx, self.scope, None, limit.unwrap_or(20)).await
    }

    async fn skills(&self, ctx: &Context<'_>) -> Result<Vec<Skill>> {
        self.scope.member()?;
        Skill::all(ctx, self.scope).await
    }

    async fn skill(&self, ctx: &Context<'_>, name: String) -> Result<Option<Skill>> {
        self.scope.member()?;
        Skill::by_name(ctx, self.scope, &name).await
    }

    async fn tasks(&self, ctx: &Context<'_>, filter: Option<TaskFilter>) -> Result<Vec<Task>> {
        self.scope.member()?;
        Task::list(ctx, self.scope, filter.unwrap_or_default()).await
    }

    async fn task(&self, ctx: &Context<'_>, id: ID) -> Result<Option<Task>> {
        self.scope.member()?;
        Task::find(ctx, self.scope, parse_id(&id)?).await
    }

    async fn chats(&self, ctx: &Context<'_>) -> Result<Vec<Task>> {
        self.scope.member()?;
        Task::chats(ctx, self.scope).await
    }

    async fn audit_events(
        &self,
        ctx: &Context<'_>,
        filter: Option<AuditFilter>,
        #[graphql(default = 100)] limit: Option<i32>,
    ) -> Result<Vec<AuditEvent>> {
        self.scope
            .require(crate::context::Role::Admin, "reading the audit log")?;
        AuditEvent::list(
            ctx,
            self.scope,
            filter.unwrap_or_default(),
            Some(limit.unwrap_or(100)),
        )
        .await
    }
}

pub(crate) async fn seats_used(
    ctx: &Context<'_>,
    tenant_id: Uuid,
    at: DateTime<Utc>,
) -> Result<usize> {
    let mut tx = tx(ctx, tenant_id);
    let members = tx
        .query::<items::Member>(&keys::tenant_pk(tenant_id), "MEMBER#")
        .await?
        .len();
    let pending = tx
        .query::<items::Invitation>(&keys::tenant_pk(tenant_id), "INVITE#")
        .await?
        .iter()
        .filter(|invitation| invitation.is_pending_at(at))
        .count();
    Ok(members + pending)
}

pub(crate) async fn running_runs(ctx: &Context<'_>, tenant_id: Uuid) -> Result<i32> {
    let runs = tx(ctx, tenant_id)
        .list::<RunItem>(
            &Listing::index(Index::Gsi2, keys::open_runs_index(tenant_id)),
            |run| !run.is_finished(),
        )
        .await?;
    Ok(i32::try_from(runs.len())?)
}

fn detail_str<'a>(row: &'a IntegrationItem, key: &str) -> Option<&'a str> {
    row.details.get(key).and_then(Value::as_str)
}

pub(crate) struct GitHubOwner {
    kind: GitHubOwnerKind,
    login: String,
}

#[Object]
impl GitHubOwner {
    async fn kind(&self) -> GitHubOwnerKind {
        self.kind
    }

    async fn login(&self) -> &str {
        &self.login
    }
}

pub(crate) struct TenantUsage {
    period_start: DateTime<Utc>,
    period_end: Option<DateTime<Utc>>,
    tasks_this_period: i32,
    workspace_seconds_this_period: i32,
    seats_used: i32,
    running_runs: i32,
}

#[Object]
impl TenantUsage {
    async fn period_start(&self) -> Timestamp {
        ts(self.period_start)
    }

    async fn period_end(&self) -> Option<Timestamp> {
        ts_opt(self.period_end)
    }

    async fn tasks_this_period(&self) -> i32 {
        self.tasks_this_period
    }

    async fn workspace_seconds_this_period(&self) -> i32 {
        self.workspace_seconds_this_period
    }

    async fn seats_used(&self) -> i32 {
        self.seats_used
    }

    async fn running_runs(&self) -> i32 {
        self.running_runs
    }
}

pub(crate) struct Activity {
    running_tasks: i32,
    runs_today: i32,
    cost_today_usd: f64,
    pull_requests_opened_today: i32,
    runs_last_24h: Vec<i32>,
}

#[Object]
impl Activity {
    async fn running_tasks(&self) -> i32 {
        self.running_tasks
    }

    async fn runs_today(&self) -> i32 {
        self.runs_today
    }

    async fn cost_today_usd(&self) -> f64 {
        self.cost_today_usd
    }

    async fn pull_requests_opened_today(&self) -> i32 {
        self.pull_requests_opened_today
    }

    #[graphql(name = "runsLast24h")]
    async fn runs_last_24h(&self) -> &[i32] {
        &self.runs_last_24h
    }
}

pub(crate) struct Setup {
    pub github_linked: bool,
    pub app_installed: bool,
    pub config_added: bool,
    pub credential_connected: bool,
    pub trigger_enabled: bool,
    pub first_task_started: bool,
    pub project_created: bool,
    pub slack_connected: bool,
}

#[Object]
impl Setup {
    async fn github_linked(&self) -> bool {
        self.github_linked
    }

    async fn app_installed(&self) -> bool {
        self.app_installed
    }

    async fn config_added(&self) -> bool {
        self.config_added
    }

    async fn credential_connected(&self) -> bool {
        self.credential_connected
    }

    async fn trigger_enabled(&self) -> bool {
        self.trigger_enabled
    }

    async fn first_task_started(&self) -> bool {
        self.first_task_started
    }

    async fn project_created(&self) -> bool {
        self.project_created
    }

    async fn slack_connected(&self) -> bool {
        self.slack_connected
    }

    async fn complete(&self) -> bool {
        self.github_linked
            && self.app_installed
            && self.config_added
            && self.credential_connected
            && self.trigger_enabled
            && self.first_task_started
    }
}

pub(crate) enum Integration {
    Row(IntegrationItem),
    NotConnected {
        tenant_id: Uuid,
        kind: IntegrationKind,
    },
}

#[Object]
impl Integration {
    async fn id(&self) -> ID {
        match self {
            Self::Row(row) => ID(row.id.to_string()),
            Self::NotConnected { tenant_id, kind } => ID(format!("{tenant_id}/{}", kind.as_db())),
        }
    }

    async fn kind(&self) -> Result<IntegrationKind> {
        match self {
            Self::Row(row) => IntegrationKind::from_db(&row.kind),
            Self::NotConnected { kind, .. } => Ok(*kind),
        }
    }

    async fn status(&self) -> IntegrationStatus {
        match self {
            Self::Row(row) if detail_str(row, "error").is_some() => IntegrationStatus::Error,
            Self::Row(row) if row.status == "connected" => IntegrationStatus::Connected,
            Self::Row(_) | Self::NotConnected { .. } => IntegrationStatus::NotConnected,
        }
    }

    async fn account(&self) -> Option<&str> {
        match self {
            Self::Row(row) if row.kind == "slack" => detail_str(row, "team_name"),
            Self::Row(row) => detail_str(row, "account_login"),
            Self::NotConnected { .. } => None,
        }
    }

    async fn repositories(&self) -> Result<Vec<String>> {
        match self {
            Self::Row(row) => Ok(github_repositories(&row.details)?
                .into_iter()
                .map(|repository| repository.full_name)
                .collect()),
            Self::NotConnected { .. } => Ok(Vec::new()),
        }
    }

    async fn manage_url(&self) -> Option<String> {
        match self {
            Self::Row(row) => match (row.github_installation_id, detail_str(row, "account_type")) {
                (Some(id), Some("Organization")) => Some(format!(
                    "https://github.com/organizations/{}/settings/installations/{id}",
                    detail_str(row, "account_login")?
                )),
                (Some(id), _) => Some(format!("https://github.com/settings/installations/{id}")),
                (None, _) => detail_str(row, "manage_url").map(str::to_string),
            },
            Self::NotConnected { .. } => None,
        }
    }

    async fn last_synced_at(&self) -> Option<Timestamp> {
        match self {
            Self::Row(row) => Some(ts(row.updated_at)),
            Self::NotConnected { .. } => None,
        }
    }

    async fn error(&self) -> Option<&str> {
        match self {
            Self::Row(row) => detail_str(row, "error"),
            Self::NotConnected { .. } => None,
        }
    }
}

pub(crate) struct Operator {
    pub tenant_ids: Vec<Uuid>,
}

#[Object]
impl Operator {
    async fn tenants(&self, ctx: &Context<'_>) -> Result<Vec<Tenant>> {
        let mut tenants = Vec::new();
        for tenant_id in &self.tenant_ids {
            tenants.push(
                Tenant::load(
                    ctx,
                    Scope {
                        tenant_id: *tenant_id,
                        access: Access::Operator,
                    },
                )
                .await?,
            );
        }
        Ok(tenants)
    }

    async fn running_runs(&self, ctx: &Context<'_>) -> Result<i32> {
        let mut total = 0;
        for tenant_id in &self.tenant_ids {
            total += running_runs(ctx, *tenant_id).await?;
        }
        Ok(total)
    }

    async fn run_hours_this_month(&self, ctx: &Context<'_>) -> Result<f64> {
        let at = now(ctx);
        let month = Utc
            .with_ymd_and_hms(at.year(), at.month(), 1, 0, 0, 0)
            .single()
            .expect("the first of the month is unambiguous in UTC");
        let mut seconds = 0.0;
        for tenant_id in &self.tenant_ids {
            let workspaces = tx(ctx, *tenant_id)
                .list::<WorkspaceItem>(
                    &Listing::index(Index::Gsi1, keys::workspaces_index(*tenant_id)),
                    |workspace| workspace.stopped_at.unwrap_or(at) > month,
                )
                .await?;
            for workspace in workspaces {
                let end = workspace.stopped_at.unwrap_or(at);
                let start = workspace.started_at.max(month);
                seconds += (end - start).num_milliseconds() as f64 / 1000.0;
            }
        }
        Ok(seconds / 3600.0)
    }

    async fn revenue_this_month_usd(&self, ctx: &Context<'_>) -> Result<i32> {
        let mut cents = 0;
        for tenant_id in &self.tenant_ids {
            let tenant = Tenant::load(
                ctx,
                Scope {
                    tenant_id: *tenant_id,
                    access: Access::Operator,
                },
            )
            .await?;
            if tenant.row.status == "active" {
                cents += Plan::load(ctx, &tenant.row.plan_id).await?.0.price_cents;
            }
        }
        Ok(i32::try_from(cents / 100)?)
    }
}

pub(crate) async fn tenant_repositories(
    ctx: &Context<'_>,
    tenant_id: Uuid,
) -> Result<Vec<items::GitHubRepository>> {
    let mut repositories = Vec::new();
    for row in tx(ctx, tenant_id)
        .query::<IntegrationItem>(&keys::tenant_pk(tenant_id), "INTEG#")
        .await?
        .iter()
        .filter(|row| row.kind == "github_installation")
    {
        repositories.extend(github_repositories(&row.details)?);
    }
    Ok(repositories)
}

pub(crate) fn current_name(
    repositories: &[items::GitHubRepository],
    repository_id: i64,
    stored: &str,
) -> String {
    repositories
        .iter()
        .find(|repository| repository.id == repository_id)
        .map_or_else(
            || stored.to_string(),
            |repository| repository.full_name.clone(),
        )
}
