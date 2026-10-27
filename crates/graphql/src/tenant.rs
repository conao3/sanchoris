use async_graphql::{Context, ID, Object, Result};
use chrono::{DateTime, Datelike, Duration, Months, TimeZone, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::account::{Credential, INVITATION_COLUMNS, Invitation, Member, Plan};
use crate::audit::{AuditEvent, AuditFilter};
use crate::config::{ChangeRequest, Config, Project, Session};
use crate::context::{Scope, auth, now, state, tenant_tx};
use crate::enums::{
    ChangeRequestState, GitHubOwnerKind, IntegrationKind, IntegrationStatus, InvitationState,
    LimitPeriod, TenantState,
};
use crate::error::{not_found, parse_id};
use crate::scalars::{Timestamp, ts, ts_opt};
use crate::skill::Skill;
use crate::task::{Task, TaskFilter};
use crate::trigger::{Trigger, TriggerEvent};

#[derive(Debug, Clone, FromRow)]
pub(crate) struct TenantRow {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub plan_id: String,
    pub period_start: DateTime<Utc>,
    pub tasks_this_period: i32,
    pub workspace_seconds_this_period: i64,
    pub status: String,
    pub suspended_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub(crate) const TENANT_COLUMNS: &str = "id, slug, name, plan_id, period_start, tasks_this_period, \
     workspace_seconds_this_period, status, suspended_reason, created_at";

pub(crate) struct Tenant {
    pub scope: Scope,
    pub row: TenantRow,
}

impl Tenant {
    pub async fn load(ctx: &Context<'_>, scope: Scope) -> Result<Self> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let row = sqlx::query_as::<_, TenantRow>(sql!(
            "SELECT {TENANT_COLUMNS} FROM sanchoris.tenants WHERE id = $1"
        ))
        .bind(scope.tenant_id)
        .fetch_optional(tx.conn())
        .await?
        .ok_or_else(|| not_found(format!("tenant {}", scope.tenant_id)))?;
        Ok(Self { scope, row })
    }

    async fn integration_rows(&self, ctx: &Context<'_>) -> Result<Vec<IntegrationRow>> {
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        Ok(sqlx::query_as::<_, IntegrationRow>(
            "SELECT id, kind, status, github_installation_id, details, updated_at \
             FROM sanchoris.integrations WHERE tenant_id = $1 ORDER BY kind, created_at, id",
        )
        .bind(self.scope.tenant_id)
        .fetch_all(tx.conn())
        .await?)
    }

    pub async fn setup_state(&self, ctx: &Context<'_>) -> Result<Setup> {
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        let github_linked = sqlx::query_scalar::<_, bool>(
            "SELECT github_login IS NOT NULL FROM sanchoris.users WHERE id = $1",
        )
        .bind(auth(ctx)?.user_id)
        .fetch_one(tx.conn())
        .await?;
        let row = sqlx::query_as::<_, SetupRow>(
            "SELECT \
               EXISTS (SELECT 1 FROM sanchoris.integrations WHERE tenant_id = $1 AND kind = 'github_installation' AND status = 'connected') AS app_installed, \
               EXISTS (SELECT 1 FROM sanchoris.configs WHERE tenant_id = $1) AS config_added, \
               EXISTS (SELECT 1 FROM sanchoris.credentials WHERE tenant_id = $1 AND status = 'connected') AS credential_connected, \
               EXISTS (SELECT 1 FROM sanchoris.triggers WHERE tenant_id = $1 AND state = 'ENABLED' AND removed_at IS NULL) AS trigger_enabled, \
               EXISTS (SELECT 1 FROM sanchoris.tasks WHERE tenant_id = $1) AS first_task_started, \
               EXISTS (SELECT 1 FROM sanchoris.projects WHERE tenant_id = $1) AS project_created, \
               EXISTS (SELECT 1 FROM sanchoris.integrations WHERE tenant_id = $1 AND kind = 'slack' AND status = 'connected') AS slack_connected",
        )
        .bind(self.scope.tenant_id)
        .fetch_one(tx.conn())
        .await?;
        Ok(Setup {
            github_linked,
            app_installed: row.app_installed,
            config_added: row.config_added,
            credential_connected: row.credential_connected,
            trigger_enabled: row.trigger_enabled,
            first_task_started: row.first_task_started,
            project_created: row.project_created,
            slack_connected: row.slack_connected,
        })
    }
}

#[derive(Debug, FromRow)]
struct SetupRow {
    app_installed: bool,
    config_added: bool,
    credential_connected: bool,
    trigger_enabled: bool,
    first_task_started: bool,
    project_created: bool,
    slack_connected: bool,
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
        let login = row.detail_str("account_login").ok_or_else(|| {
            async_graphql::Error::new("the GitHub installation has no details.account_login")
        })?;
        let kind = match row.detail_str("account_type") {
            Some("Organization") => GitHubOwnerKind::Organization,
            _ => GitHubOwnerKind::User,
        };
        Ok(Some(GitHubOwner {
            kind,
            login: login.to_string(),
        }))
    }

    async fn github_app_install_url(&self, ctx: &Context<'_>) -> Result<String> {
        self.scope.member()?;
        Ok(state(ctx).deps.github.install_app(self.row.id))
    }

    async fn viewer_member(&self, ctx: &Context<'_>) -> Result<Member> {
        let (member_id, _) = self.scope.member()?;
        Member::load(ctx, self.scope, member_id).await
    }

    async fn usage(&self, ctx: &Context<'_>) -> Result<TenantUsage> {
        self.scope.operator_or_member()?;
        let plan = Plan::load(ctx, &self.row.plan_id).await?;
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        let seats = sqlx::query_scalar::<_, i64>(
            "SELECT (SELECT count(*) FROM sanchoris.members WHERE tenant_id = $1) \
                  + (SELECT count(*) FROM sanchoris.invitations WHERE tenant_id = $1 AND status = 'pending' AND expires_at > $2)",
        )
        .bind(self.scope.tenant_id)
        .bind(now(ctx))
        .fetch_one(tx.conn())
        .await?;
        let running = running_runs(&mut tx, self.scope.tenant_id).await?;
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
            tasks_this_period: self.row.tasks_this_period,
            workspace_seconds_this_period: i32::try_from(self.row.workspace_seconds_this_period)?,
            seats_used: i32::try_from(seats)?,
            running_runs: running,
        })
    }

    async fn activity(&self, ctx: &Context<'_>) -> Result<Activity> {
        self.scope.member()?;
        let at = now(ctx);
        let today = Utc
            .with_ymd_and_hms(at.year(), at.month(), at.day(), 0, 0, 0)
            .single()
            .expect("midnight is unambiguous in UTC");
        let since = at - Duration::hours(24);
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        let row = sqlx::query_as::<_, ActivityRow>(
            "SELECT \
               (SELECT count(*) FROM sanchoris.tasks WHERE tenant_id = $1 AND status = 'RUNNING') AS running_tasks, \
               (SELECT count(*) FROM sanchoris.runs WHERE tenant_id = $1 AND started_at >= $2) AS runs_today, \
               (SELECT coalesce(sum(cost_usd), 0)::float8 FROM sanchoris.usage WHERE tenant_id = $1 AND recorded_at >= $2) AS cost_today, \
               (SELECT count(*) FROM sanchoris.outputs WHERE tenant_id = $1 AND kind = 'pull_request' AND created_at >= $2) AS pull_requests_today",
        )
        .bind(self.scope.tenant_id)
        .bind(today)
        .fetch_one(tx.conn())
        .await?;
        let starts = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT started_at FROM sanchoris.runs WHERE tenant_id = $1 AND started_at > $2 AND started_at <= $3",
        )
        .bind(self.scope.tenant_id)
        .bind(since)
        .bind(at)
        .fetch_all(tx.conn())
        .await?;
        let mut buckets = vec![0; 24];
        for started in starts {
            let index = usize::try_from((started - since).num_seconds().saturating_sub(1) / 3600)
                .expect("runs in the window are after its start");
            buckets[index.min(23)] += 1;
        }
        Ok(Activity {
            running_tasks: i32::try_from(row.running_tasks)?,
            runs_today: i32::try_from(row.runs_today)?,
            cost_today_usd: row.cost_today,
            pull_requests_opened_today: i32::try_from(row.pull_requests_today)?,
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
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        let rows = sqlx::query_as::<_, crate::account::InvitationRow>(sql!(
            "SELECT {INVITATION_COLUMNS} FROM sanchoris.invitations i \
             WHERE i.tenant_id = $1 ORDER BY i.created_at DESC, i.id"
        ))
        .bind(self.scope.tenant_id)
        .fetch_all(tx.conn())
        .await?;
        let at = now(ctx);
        let mut invitations = Vec::new();
        for row in rows {
            let invitation = Invitation {
                scope: self.scope,
                row,
            };
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
        Project::query(ctx, self.scope, None).await
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

pub(crate) async fn running_runs(
    tx: &mut sanchoris_core::db::TenantTx,
    tenant_id: Uuid,
) -> Result<i32> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM sanchoris.runs WHERE tenant_id = $1 AND status IN ('creating', 'running', 'idle')",
    )
    .bind(tenant_id)
    .fetch_one(tx.conn())
    .await?;
    Ok(i32::try_from(count)?)
}

#[derive(Debug, FromRow)]
struct ActivityRow {
    running_tasks: i64,
    runs_today: i64,
    cost_today: f64,
    pull_requests_today: i64,
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

#[derive(Debug, Clone, FromRow)]
pub(crate) struct IntegrationRow {
    id: Uuid,
    kind: String,
    status: String,
    github_installation_id: Option<i64>,
    details: Value,
    updated_at: DateTime<Utc>,
}

impl IntegrationRow {
    fn detail_str(&self, key: &str) -> Option<&str> {
        self.details.get(key).and_then(Value::as_str)
    }
}

pub(crate) enum Integration {
    Row(IntegrationRow),
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
            Self::Row(row) if row.detail_str("error").is_some() => IntegrationStatus::Error,
            Self::Row(row) if row.status == "connected" => IntegrationStatus::Connected,
            Self::Row(_) | Self::NotConnected { .. } => IntegrationStatus::NotConnected,
        }
    }

    async fn account(&self) -> Option<&str> {
        match self {
            Self::Row(row) if row.kind == "slack" => row.detail_str("team_name"),
            Self::Row(row) => row.detail_str("account_login"),
            Self::NotConnected { .. } => None,
        }
    }

    async fn repositories(&self) -> Vec<String> {
        match self {
            Self::Row(row) => row
                .details
                .get("repositories")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            Self::NotConnected { .. } => Vec::new(),
        }
    }

    async fn manage_url(&self) -> Option<String> {
        match self {
            Self::Row(row) => match (row.github_installation_id, row.detail_str("account_type")) {
                (Some(id), Some("Organization")) => Some(format!(
                    "https://github.com/organizations/{}/settings/installations/{id}",
                    row.detail_str("account_login")?
                )),
                (Some(id), _) => Some(format!("https://github.com/settings/installations/{id}")),
                (None, _) => row.detail_str("manage_url").map(str::to_string),
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
            Self::Row(row) => row.detail_str("error"),
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
                        access: crate::context::Access::Operator,
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
            let mut tx = tenant_tx(ctx, *tenant_id).await?;
            total += running_runs(&mut tx, *tenant_id).await?;
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
            let mut tx = tenant_tx(ctx, *tenant_id).await?;
            seconds += sqlx::query_scalar::<_, f64>(
                "SELECT coalesce(sum(extract(epoch FROM (coalesce(stopped_at, $3) - greatest(started_at, $2)))), 0)::float8 \
                 FROM sanchoris.workspaces WHERE tenant_id = $1 AND coalesce(stopped_at, $3) > $2",
            )
            .bind(tenant_id)
            .bind(month)
            .bind(at)
            .fetch_one(tx.conn())
            .await?;
        }
        Ok(seconds / 3600.0)
    }

    async fn revenue_this_month_usd(&self, ctx: &Context<'_>) -> Result<i32> {
        let mut cents = 0;
        for tenant_id in &self.tenant_ids {
            let mut tx = tenant_tx(ctx, *tenant_id).await?;
            cents += sqlx::query_scalar::<_, i32>(
                "SELECT p.price_cents FROM sanchoris.tenants t JOIN sanchoris.plans p ON p.id = t.plan_id \
                 WHERE t.id = $1 AND t.status = 'active'",
            )
            .bind(tenant_id)
            .fetch_optional(tx.conn())
            .await?
            .unwrap_or(0);
        }
        Ok(cents / 100)
    }
}
