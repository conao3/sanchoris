use async_graphql::{Context, ID, Object, Result};
use chrono::{DateTime, Utc};
use sanchoris_core::db;
use sanchoris_core::items::{
    self, Guard, InvitationEmail, Membership as MembershipItem, Task as TaskItem,
    Tenant as TenantItem, UsageSnapshot as SnapshotItem, User as UserItem,
};
use sanchoris_core::keys;
use sanchoris_core::store::Listing;
use uuid::Uuid;

use crate::context::{
    Access, Role, Scope, auth, is_operator, memberships, now, store, tx, viewer_user,
};
use crate::enums::{
    ApiKeyProvider, BillingUnit, CredentialKind, CredentialScope, CredentialStatus,
    InvitationState, LimitPeriod, Provider,
};
use crate::error::not_found;
use crate::scalars::{Timestamp, ts, ts_opt};
use crate::task::latest_tasks;
use crate::tenant::Tenant;

pub(crate) struct User(pub UserItem);

impl User {
    pub async fn load(ctx: &Context<'_>, user_id: Uuid) -> Result<Self> {
        store(ctx)
            .get::<UserItem>(&keys::user_key(user_id))
            .await?
            .map(User)
            .ok_or_else(|| not_found(format!("user {user_id}")))
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
        Ok(User(viewer_user(ctx).await?))
    }

    async fn memberships(&self, ctx: &Context<'_>) -> Result<Vec<Member>> {
        let mut members = Vec::new();
        for membership in memberships(ctx).await? {
            members.push(Member::load(ctx, membership.scope(), membership.member_id).await?);
        }
        Ok(members)
    }

    async fn invitations(&self, ctx: &Context<'_>) -> Result<Vec<Invitation>> {
        let user = auth(ctx)?;
        let at = now(ctx);
        let addressed = store(ctx)
            .query::<InvitationEmail>(&keys::invitation_email_pk(&user.email), "INVITE#")
            .await?;
        let mut invitations = Vec::new();
        for entry in addressed {
            let Some(invitation) = tx(ctx, entry.tenant_id)
                .get::<items::Invitation>(&keys::invitation_key(
                    entry.tenant_id,
                    entry.invitation_id,
                ))
                .await?
            else {
                continue;
            };
            if !invitation.is_pending_at(at) {
                continue;
            }
            let member = store(ctx)
                .get::<MembershipItem>(&keys::membership_key(user.user_id, entry.tenant_id))
                .await?;
            if member.is_some() {
                continue;
            }
            invitations.push(Invitation {
                scope: Scope {
                    tenant_id: entry.tenant_id,
                    access: Access::Invitee,
                },
                row: invitation,
            });
        }
        invitations.sort_by(|a, b| (b.row.created_at, a.row.id).cmp(&(a.row.created_at, b.row.id)));
        Ok(invitations)
    }

    async fn is_operator(&self, ctx: &Context<'_>) -> Result<bool> {
        is_operator(ctx).await
    }
}

pub(crate) struct Plan(pub items::Plan);

impl Plan {
    pub async fn all(ctx: &Context<'_>) -> Result<Vec<Self>> {
        let mut plans = db::plans(store(ctx)).await?;
        plans.sort_by(|a, b| (a.price_cents, &a.id).cmp(&(b.price_cents, &b.id)));
        Ok(plans.into_iter().map(Plan).collect())
    }

    pub async fn load(ctx: &Context<'_>, plan_id: &str) -> Result<Self> {
        store(ctx)
            .get::<items::Plan>(&keys::plan_key(plan_id))
            .await?
            .map(Plan)
            .ok_or_else(|| not_found(format!("plan {plan_id}")))
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

    async fn price_usd(&self) -> Result<i32> {
        Ok(i32::try_from(self.0.price_cents / 100)?)
    }

    async fn billing_unit(&self) -> Result<BillingUnit> {
        BillingUnit::from_db(&self.0.billing_unit)
    }

    async fn task_limit(&self) -> Result<i32> {
        Ok(i32::try_from(self.0.task_limit)?)
    }

    async fn workspace_hour_limit(&self) -> Result<i32> {
        Ok(i32::try_from(self.0.workspace_seconds_limit / 3600)?)
    }

    async fn limit_period(&self) -> Result<LimitPeriod> {
        self.period()
    }

    async fn member_limit(&self) -> Result<i32> {
        Ok(i32::try_from(self.0.member_limit)?)
    }

    async fn concurrency_limit(&self) -> Result<i32> {
        Ok(i32::try_from(self.0.max_concurrency)?)
    }

    async fn retention_days(&self) -> Result<i32> {
        Ok(i32::try_from(self.0.retention_days)?)
    }

    async fn audit_retention_days(&self) -> Result<i32> {
        Ok(i32::try_from(self.0.audit_retention_days)?)
    }

    async fn requires_payment(&self) -> bool {
        self.0.price_cents > 0
    }
}

pub(crate) struct Member {
    pub scope: Scope,
    pub row: items::Member,
}

impl Member {
    pub async fn load(ctx: &Context<'_>, scope: Scope, member_id: Uuid) -> Result<Self> {
        Self::find(ctx, scope, member_id)
            .await?
            .ok_or_else(|| not_found(format!("member {member_id}")))
    }

    pub async fn find(ctx: &Context<'_>, scope: Scope, member_id: Uuid) -> Result<Option<Self>> {
        Ok(tx(ctx, scope.tenant_id)
            .get::<items::Member>(&keys::member_key(scope.tenant_id, member_id))
            .await?
            .map(|row| Member { scope, row }))
    }

    pub async fn all(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        let rows = tx(ctx, scope.tenant_id)
            .query::<items::Member>(&keys::tenant_pk(scope.tenant_id), "MEMBER#")
            .await?;
        let mut members = Vec::new();
        for row in rows {
            let user = User::load(ctx, row.user_id).await?;
            members.push((Role::from_db(&row.role), user.0.display_name, row));
        }
        members.sort_by(|a, b| (b.0.rank(), &a.1, a.2.id).cmp(&(a.0.rank(), &b.1, b.2.id)));
        Ok(members
            .into_iter()
            .map(|(_, _, row)| Member { scope, row })
            .collect())
    }
}

#[Object]
impl Member {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn user(&self, ctx: &Context<'_>) -> Result<User> {
        User::load(ctx, self.row.user_id).await
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
        let member_id = self.row.id;
        let tasks = tx(ctx, self.scope.tenant_id)
            .list::<TaskItem>(&latest_tasks(self.scope.tenant_id), |task| {
                task.requested_by_member_id == Some(member_id)
            })
            .await?;
        Ok(i32::try_from(tasks.len())?)
    }

    async fn credentials(&self, ctx: &Context<'_>) -> Result<Vec<Credential>> {
        let (viewer, role) = self.scope.member()?;
        let member_id = self.row.id;
        if viewer != member_id && !role.at_least(Role::Admin) {
            return Ok(Vec::new());
        }
        Credential::filtered(ctx, self.scope, |c| {
            c.scope == "personal" && c.member_id == Some(member_id)
        })
        .await
    }
}

pub(crate) struct Invitation {
    pub scope: Scope,
    pub row: items::Invitation,
}

impl Invitation {
    pub async fn load(ctx: &Context<'_>, scope: Scope, invitation_id: Uuid) -> Result<Self> {
        tx(ctx, scope.tenant_id)
            .get::<items::Invitation>(&keys::invitation_key(scope.tenant_id, invitation_id))
            .await?
            .map(|row| Invitation { scope, row })
            .ok_or_else(|| not_found(format!("invitation {invitation_id}")))
    }

    pub async fn all(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        let mut rows = tx(ctx, scope.tenant_id)
            .query::<items::Invitation>(&keys::tenant_pk(scope.tenant_id), "INVITE#")
            .await?;
        rows.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
        Ok(rows
            .into_iter()
            .map(|row| Invitation { scope, row })
            .collect())
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
        Ok(tx(ctx, self.scope.tenant_id)
            .get::<TenantItem>(&keys::tenant_key(self.scope.tenant_id))
            .await?
            .ok_or_else(|| not_found(format!("tenant {}", self.scope.tenant_id)))?
            .name)
    }
}

pub(crate) struct Credential {
    pub scope: Scope,
    pub row: items::Credential,
}

impl Credential {
    pub async fn filtered(
        ctx: &Context<'_>,
        scope: Scope,
        keep: impl Fn(&items::Credential) -> bool,
    ) -> Result<Vec<Self>> {
        let mut rows = tx(ctx, scope.tenant_id)
            .query::<items::Credential>(&keys::tenant_pk(scope.tenant_id), "CRED#")
            .await?
            .into_iter()
            .filter(|row| keep(row))
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| (&a.scope, &a.name).cmp(&(&b.scope, &b.name)));
        Ok(rows
            .into_iter()
            .map(|row| Credential { scope, row })
            .collect())
    }

    pub async fn visible(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        let (member_id, _) = scope.member()?;
        Self::filtered(ctx, scope, |c| {
            c.scope == "org" || c.member_id == Some(member_id)
        })
        .await
    }

    pub async fn by_id(ctx: &Context<'_>, scope: Scope, id: Uuid) -> Result<Self> {
        tx(ctx, scope.tenant_id)
            .get::<items::Credential>(&keys::credential_key(scope.tenant_id, id))
            .await?
            .map(|row| Credential { scope, row })
            .ok_or_else(|| not_found(format!("credential {id}")))
    }

    pub async fn by_name(ctx: &Context<'_>, scope: Scope, name: &str) -> Result<Option<Self>> {
        let guard = tx(ctx, scope.tenant_id)
            .get::<Guard>(&keys::credential_name_key(scope.tenant_id, name))
            .await?;
        match guard {
            Some(guard) => Ok(Some(Self::by_id(ctx, scope, guard.target).await?)),
            None => Ok(None),
        }
    }
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

pub(crate) struct UsageSnapshot(SnapshotItem);

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
        let latest = tx(ctx, self.scope.tenant_id)
            .list::<SnapshotItem>(
                &Listing::main(keys::tenant_pk(self.scope.tenant_id))
                    .prefix(&keys::usage_snapshot_prefix(self.row.id))
                    .descending()
                    .limit(1),
                |_| true,
            )
            .await?;
        Ok(latest.into_iter().next().map(UsageSnapshot))
    }
}

pub(crate) fn mask_key(key: &str) -> String {
    let chars = key.chars().collect::<Vec<_>>();
    if chars.len() <= 12 {
        return "…".to_string();
    }
    let head = chars[..4].iter().collect::<String>();
    let tail = chars[chars.len() - 4..].iter().collect::<String>();
    format!("{head}…{tail}")
}

pub(crate) fn key_matches_provider(provider: ApiKeyProvider, key: &str) -> bool {
    match provider {
        ApiKeyProvider::Anthropic => ["sk-ant-oat", "sk-ant-api"]
            .iter()
            .any(|prefix| key.starts_with(prefix)),
        ApiKeyProvider::Openai => key.starts_with("sk-") && !key.starts_with("sk-ant-"),
        ApiKeyProvider::Bedrock => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_keys() {
        assert_eq!(mask_key("sk-ant-api03-abcdef9f2c"), "sk-a…9f2c");
        assert_eq!(mask_key("short"), "…");
    }

    #[test]
    fn keys_carry_their_provider_prefix() {
        assert!(key_matches_provider(
            ApiKeyProvider::Anthropic,
            "sk-ant-api03-abc"
        ));
        assert!(key_matches_provider(
            ApiKeyProvider::Anthropic,
            "sk-ant-oat01-abc"
        ));
        assert!(!key_matches_provider(
            ApiKeyProvider::Anthropic,
            "sk-proj-abc"
        ));
        assert!(key_matches_provider(ApiKeyProvider::Openai, "sk-proj-abc"));
        assert!(!key_matches_provider(
            ApiKeyProvider::Openai,
            "sk-ant-api03-abc"
        ));
        assert!(!key_matches_provider(ApiKeyProvider::Openai, "not-a-key"));
        assert!(key_matches_provider(ApiKeyProvider::Bedrock, "ABSKexample"));
    }
}
