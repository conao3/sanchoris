use async_graphql::{Context, ID, Object, Result};
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use crate::context::{
    Access, Role, Scope, auth, is_operator, memberships, now, tenant_tx, user_tx,
};
use crate::enums::{
    BillingUnit, CredentialKind, CredentialScope, CredentialStatus, InvitationState, LimitPeriod,
    Provider,
};
use crate::error::not_found;
use crate::scalars::{Timestamp, ts, ts_opt};
use crate::tenant::Tenant;

#[derive(Debug, Clone, FromRow)]
pub(crate) struct UserRow {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub github_login: Option<String>,
}

pub(crate) struct User(pub UserRow);

impl User {
    pub async fn load(ctx: &Context<'_>, tenant_id: Uuid, user_id: Uuid) -> Result<Self> {
        let mut tx = tenant_tx(ctx, tenant_id).await?;
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, email, display_name, github_login FROM sanchoris.users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(tx.conn())
        .await?
        .ok_or_else(|| not_found(format!("user {user_id}")))?;
        Ok(Self(row))
    }
}

#[Object]
impl User {
    async fn id(&self) -> ID {
        ID(self.0.id.to_string())
    }

    async fn email(&self) -> &str {
        &self.0.email
    }

    async fn name(&self) -> &str {
        &self.0.display_name
    }

    async fn github_login(&self) -> Option<&str> {
        self.0.github_login.as_deref()
    }
}

pub(crate) struct Viewer;

#[Object]
impl Viewer {
    async fn user(&self, ctx: &Context<'_>) -> Result<User> {
        let mut tx = user_tx(ctx).await?;
        let user_id = tx.user_id();
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, email, display_name, github_login FROM sanchoris.users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(tx.conn())
        .await?
        .ok_or_else(|| not_found(format!("user {user_id}")))?;
        Ok(User(row))
    }

    async fn memberships(&self, ctx: &Context<'_>) -> Result<Vec<Member>> {
        let mut members = Vec::new();
        for membership in memberships(ctx).await? {
            members.push(Member::load(ctx, membership.scope(), membership.member_id).await?);
        }
        Ok(members)
    }

    async fn invitations(&self, ctx: &Context<'_>) -> Result<Vec<Invitation>> {
        let mut tx = user_tx(ctx).await?;
        let rows = sqlx::query_as::<_, InvitationRow>(sql!(
            "SELECT {INVITATION_COLUMNS} FROM sanchoris.invitations i \
             WHERE i.status = 'pending' AND i.expires_at > $1 \
               AND NOT EXISTS (SELECT 1 FROM sanchoris.members m WHERE m.tenant_id = i.tenant_id AND m.user_id = $2) \
             ORDER BY i.created_at DESC, i.id"
        ))
        .bind(now(ctx))
        .bind(auth(ctx)?.user_id)
        .fetch_all(tx.conn())
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| Invitation {
                scope: Scope {
                    tenant_id: row.tenant_id,
                    access: Access::Invitee,
                },
                row,
            })
            .collect())
    }

    async fn is_operator(&self, ctx: &Context<'_>) -> Result<bool> {
        is_operator(ctx).await
    }
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PlanRow {
    pub id: String,
    pub name: String,
    pub price_cents: i32,
    pub billing_unit: String,
    pub task_limit: i32,
    pub task_limit_period: String,
    pub workspace_seconds_limit: i64,
    pub member_limit: i32,
    pub max_concurrency: i32,
    pub retention_days: i32,
    pub audit_retention_days: i32,
}

pub(crate) const PLAN_COLUMNS: &str = "id, name, price_cents, billing_unit, task_limit, task_limit_period, \
     workspace_seconds_limit, member_limit, max_concurrency, retention_days, audit_retention_days";

pub(crate) struct Plan(pub PlanRow);

impl Plan {
    pub async fn all(ctx: &Context<'_>) -> Result<Vec<Self>> {
        let mut tx = user_tx(ctx).await?;
        let rows = sqlx::query_as::<_, PlanRow>(sql!(
            "SELECT {PLAN_COLUMNS} FROM sanchoris.plans ORDER BY price_cents, id"
        ))
        .fetch_all(tx.conn())
        .await?;
        Ok(rows.into_iter().map(Plan).collect())
    }

    pub async fn load(ctx: &Context<'_>, plan_id: &str) -> Result<Self> {
        let mut tx = user_tx(ctx).await?;
        let row = sqlx::query_as::<_, PlanRow>(sql!(
            "SELECT {PLAN_COLUMNS} FROM sanchoris.plans WHERE id = $1"
        ))
        .bind(plan_id)
        .fetch_optional(tx.conn())
        .await?
        .ok_or_else(|| not_found(format!("plan {plan_id}")))?;
        Ok(Plan(row))
    }

    pub fn period(&self) -> Result<LimitPeriod> {
        LimitPeriod::from_db(&self.0.task_limit_period)
    }
}

#[Object]
impl Plan {
    async fn id(&self) -> ID {
        ID(self.0.id.clone())
    }

    async fn name(&self) -> &str {
        &self.0.name
    }

    async fn price_usd(&self) -> i32 {
        self.0.price_cents / 100
    }

    async fn billing_unit(&self) -> Result<BillingUnit> {
        BillingUnit::from_db(&self.0.billing_unit)
    }

    async fn task_limit(&self) -> i32 {
        self.0.task_limit
    }

    async fn workspace_hour_limit(&self) -> i32 {
        i32::try_from(self.0.workspace_seconds_limit / 3600).expect("hours fit in i32")
    }

    async fn limit_period(&self) -> Result<LimitPeriod> {
        self.period()
    }

    async fn member_limit(&self) -> i32 {
        self.0.member_limit
    }

    async fn concurrency_limit(&self) -> i32 {
        self.0.max_concurrency
    }

    async fn retention_days(&self) -> i32 {
        self.0.retention_days
    }

    async fn audit_retention_days(&self) -> i32 {
        self.0.audit_retention_days
    }

    async fn requires_payment(&self) -> bool {
        self.0.price_cents > 0
    }
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct MemberRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub role: String,
    pub joined_at: DateTime<Utc>,
    pub last_active_at: Option<DateTime<Utc>>,
}

pub(crate) const MEMBER_COLUMNS: &str = "id, user_id, role, joined_at, last_active_at";

pub(crate) struct Member {
    pub scope: Scope,
    pub row: MemberRow,
}

impl Member {
    pub async fn load(ctx: &Context<'_>, scope: Scope, member_id: Uuid) -> Result<Self> {
        Self::find(ctx, scope, member_id)
            .await?
            .ok_or_else(|| not_found(format!("member {member_id}")))
    }

    pub async fn find(ctx: &Context<'_>, scope: Scope, member_id: Uuid) -> Result<Option<Self>> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let row = sqlx::query_as::<_, MemberRow>(sql!(
            "SELECT {MEMBER_COLUMNS} FROM sanchoris.members WHERE id = $1 AND tenant_id = $2"
        ))
        .bind(member_id)
        .bind(scope.tenant_id)
        .fetch_optional(tx.conn())
        .await?;
        Ok(row.map(|row| Member { scope, row }))
    }

    pub async fn all(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let rows = sqlx::query_as::<_, MemberRow>(
            "SELECT m.id, m.user_id, m.role, m.joined_at, m.last_active_at FROM sanchoris.members m \
             JOIN sanchoris.users u ON u.id = m.user_id \
             WHERE m.tenant_id = $1 \
             ORDER BY CASE m.role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 ELSE 2 END, u.display_name, m.id",
        )
        .bind(scope.tenant_id)
        .fetch_all(tx.conn())
        .await?;
        Ok(rows.into_iter().map(|row| Member { scope, row }).collect())
    }
}

#[Object]
impl Member {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn user(&self, ctx: &Context<'_>) -> Result<User> {
        User::load(ctx, self.scope.tenant_id, self.row.user_id).await
    }

    async fn tenant(&self, ctx: &Context<'_>) -> Result<Tenant> {
        Tenant::load(ctx, self.scope).await
    }

    async fn role(&self) -> Role {
        Role::from_db(&self.row.role)
    }

    async fn joined_at(&self) -> Timestamp {
        ts(self.row.joined_at)
    }

    async fn last_active_at(&self) -> Option<Timestamp> {
        ts_opt(self.row.last_active_at)
    }

    async fn tasks_started(&self, ctx: &Context<'_>) -> Result<i32> {
        self.scope.member()?;
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM sanchoris.tasks WHERE tenant_id = $1 AND requested_by_member_id = $2",
        )
        .bind(self.scope.tenant_id)
        .bind(self.row.id)
        .fetch_one(tx.conn())
        .await?;
        Ok(i32::try_from(count)?)
    }

    async fn credentials(&self, ctx: &Context<'_>) -> Result<Vec<Credential>> {
        self.scope.member()?;
        Credential::query(
            ctx,
            self.scope,
            "c.scope = 'personal' AND c.member_id = $2",
            Some(self.row.id),
        )
        .await
    }
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct InvitationRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub email: String,
    pub role: String,
    pub status: String,
    pub invited_by_member_id: Uuid,
    pub expires_at: DateTime<Utc>,
}

pub(crate) const INVITATION_COLUMNS: &str =
    "i.id, i.tenant_id, i.email, i.role, i.status, i.invited_by_member_id, i.expires_at";

pub(crate) struct Invitation {
    pub scope: Scope,
    pub row: InvitationRow,
}

impl Invitation {
    pub async fn load(ctx: &Context<'_>, scope: Scope, invitation_id: Uuid) -> Result<Self> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let row = sqlx::query_as::<_, InvitationRow>(sql!(
            "SELECT {INVITATION_COLUMNS} FROM sanchoris.invitations i WHERE i.id = $1"
        ))
        .bind(invitation_id)
        .fetch_optional(tx.conn())
        .await?
        .ok_or_else(|| not_found(format!("invitation {invitation_id}")))?;
        Ok(Self { scope, row })
    }

    pub fn state_at(&self, at: DateTime<Utc>) -> Result<InvitationState> {
        let state = InvitationState::from_db(&self.row.status)?;
        Ok(match state {
            InvitationState::Pending if self.row.expires_at <= at => InvitationState::Expired,
            other => other,
        })
    }
}

#[Object]
impl Invitation {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn email(&self) -> &str {
        &self.row.email
    }

    async fn role(&self) -> Role {
        Role::from_db(&self.row.role)
    }

    async fn state(&self, ctx: &Context<'_>) -> Result<InvitationState> {
        self.state_at(now(ctx))
    }

    async fn invited_by(&self, ctx: &Context<'_>) -> Result<Member> {
        Member::load(ctx, self.scope, self.row.invited_by_member_id).await
    }

    async fn expires_at(&self) -> Timestamp {
        ts(self.row.expires_at)
    }

    async fn tenant_name(&self, ctx: &Context<'_>) -> Result<String> {
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        Ok(
            sqlx::query_scalar::<_, String>("SELECT name FROM sanchoris.tenants WHERE id = $1")
                .bind(self.scope.tenant_id)
                .fetch_one(tx.conn())
                .await?,
        )
    }
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct CredentialRow {
    pub id: Uuid,
    pub scope: String,
    pub member_id: Option<Uuid>,
    pub name: String,
    pub provider: String,
    pub kind: String,
    pub account: String,
    pub status: String,
    pub subscription_plan: Option<String>,
    pub rejected_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
}

pub(crate) struct Credential {
    pub scope: Scope,
    pub row: CredentialRow,
}

impl Credential {
    pub async fn query(
        ctx: &Context<'_>,
        scope: Scope,
        condition: &str,
        member_id: Option<Uuid>,
    ) -> Result<Vec<Self>> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let rows = sqlx::query_as::<_, CredentialRow>(sql!(
            "SELECT c.id, c.scope, c.member_id, c.name, c.provider, c.kind, c.account, c.status, \
                    c.subscription_plan, c.rejected_at, c.last_used_at \
             FROM sanchoris.credentials c \
             WHERE c.tenant_id = $1 AND {condition} ORDER BY c.scope, c.name"
        ))
        .bind(scope.tenant_id)
        .bind(member_id)
        .fetch_all(tx.conn())
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| Credential { scope, row })
            .collect())
    }

    pub async fn visible(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        let (member_id, _) = scope.member()?;
        Self::query(
            ctx,
            scope,
            "(c.scope = 'org' OR c.member_id = $2)",
            Some(member_id),
        )
        .await
    }

    pub async fn by_id(ctx: &Context<'_>, scope: Scope, id: Uuid) -> Result<Self> {
        Self::query(ctx, scope, "c.id = $2", Some(id))
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| not_found(format!("credential {id}")))
    }

    pub async fn by_name(ctx: &Context<'_>, scope: Scope, name: &str) -> Result<Option<Self>> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let id = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM sanchoris.credentials WHERE tenant_id = $1 AND name = $2",
        )
        .bind(scope.tenant_id)
        .bind(name)
        .fetch_optional(tx.conn())
        .await?;
        drop(tx);
        match id {
            Some(id) => Ok(Some(Self::by_id(ctx, scope, id).await?)),
            None => Ok(None),
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct SnapshotRow {
    captured_at: DateTime<Utc>,
    five_hour_utilization: f64,
    five_hour_resets_at: DateTime<Utc>,
    seven_day_utilization: f64,
    seven_day_resets_at: DateTime<Utc>,
}

pub(crate) struct UsageWindow {
    utilization: f64,
    resets_at: DateTime<Utc>,
}

#[Object]
impl UsageWindow {
    async fn utilization(&self) -> f64 {
        self.utilization
    }

    async fn resets_at(&self) -> Timestamp {
        ts(self.resets_at)
    }
}

pub(crate) struct UsageSnapshot(SnapshotRow);

#[Object]
impl UsageSnapshot {
    async fn taken_at(&self) -> Timestamp {
        ts(self.0.captured_at)
    }

    async fn five_hour(&self) -> Option<UsageWindow> {
        Some(UsageWindow {
            utilization: self.0.five_hour_utilization / 100.0,
            resets_at: self.0.five_hour_resets_at,
        })
    }

    async fn seven_day(&self) -> Option<UsageWindow> {
        Some(UsageWindow {
            utilization: self.0.seven_day_utilization / 100.0,
            resets_at: self.0.seven_day_resets_at,
        })
    }
}

#[Object]
impl Credential {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn name(&self) -> &str {
        &self.row.name
    }

    async fn provider(&self) -> Result<Provider> {
        Provider::from_db(&self.row.provider)
    }

    async fn kind(&self) -> Result<CredentialKind> {
        CredentialKind::from_db(&self.row.kind)
    }

    async fn subscription_plan(&self) -> Option<&str> {
        self.row.subscription_plan.as_deref()
    }

    async fn account(&self) -> &str {
        &self.row.account
    }

    async fn scope(&self) -> Result<CredentialScope> {
        CredentialScope::from_db(&self.row.scope)
    }

    async fn owner(&self, ctx: &Context<'_>) -> Result<Option<Member>> {
        match self.row.member_id {
            Some(member_id) => Ok(Some(Member::load(ctx, self.scope, member_id).await?)),
            None => Ok(None),
        }
    }

    async fn status(&self) -> Result<CredentialStatus> {
        CredentialStatus::from_db(&self.row.status)
    }

    async fn rejected_at(&self) -> Option<Timestamp> {
        ts_opt(self.row.rejected_at)
    }

    async fn last_used_at(&self) -> Option<Timestamp> {
        ts_opt(self.row.last_used_at)
    }

    async fn usage(&self, ctx: &Context<'_>) -> Result<Option<UsageSnapshot>> {
        if self.row.kind != "subscription" || self.row.status != "connected" {
            return Ok(None);
        }
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        let row = sqlx::query_as::<_, SnapshotRow>(
            "SELECT captured_at, five_hour_utilization::float8 AS five_hour_utilization, five_hour_resets_at, \
                    seven_day_utilization::float8 AS seven_day_utilization, seven_day_resets_at \
             FROM sanchoris.usage_snapshots WHERE credential_id = $1 ORDER BY captured_at DESC LIMIT 1",
        )
        .bind(self.row.id)
        .fetch_optional(tx.conn())
        .await?;
        Ok(row.map(UsageSnapshot))
    }
}

pub(crate) fn mask_key(key: &str) -> String {
    let chars = key.chars().collect::<Vec<_>>();
    if chars.len() <= 12 {
        return "…".to_string();
    }
    let head = chars[..7].iter().collect::<String>();
    let tail = chars[chars.len() - 4..].iter().collect::<String>();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_keys() {
        assert_eq!(mask_key("sk-ant-api03-abcdef9f2c"), "sk-ant-…9f2c");
        assert_eq!(mask_key("short"), "…");
    }
}
