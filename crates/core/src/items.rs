use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::item::{Entity, ItemReader, ItemWriter, Key, format_time};
use crate::keys;
use crate::model::{SkillRunStatus, TaskStatus, WaitReason};
use crate::store::DbError;

pub const USAGE_SNAPSHOT_RETENTION_DAYS: i64 = 30;

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub id: String,
    pub name: String,
    pub price_cents: i64,
    pub currency: String,
    pub billing_unit: String,
    pub task_limit: Option<i64>,
    pub task_limit_period: String,
    pub workspace_seconds_limit: Option<i64>,
    pub workspace_limit_period: String,
    pub member_limit: Option<i64>,
    pub max_concurrency: i64,
    pub inbound_events_per_minute: i64,
    pub retention_days: i64,
    pub audit_retention_days: i64,
    pub max_task_turns: i64,
    pub max_task_cost_usd: f64,
    pub max_task_timeout_seconds: i64,
    pub max_idle_seconds: i64,
    pub max_children_depth: i64,
    pub max_children_count: i64,
    pub computes: Vec<String>,
    pub workspace_cpu: i64,
    pub workspace_memory_mib: i64,
    pub stripe_price_id: Option<String>,
}

impl Entity for Plan {
    const KIND: &'static str = "plan";

    fn key(&self) -> Key {
        keys::plan_key(&self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.s("id", &self.id)
            .s("name", &self.name)
            .n("price_cents", self.price_cents)
            .s("currency", &self.currency)
            .s("billing_unit", &self.billing_unit)
            .null_or_n("task_limit", self.task_limit)
            .s("task_limit_period", &self.task_limit_period)
            .null_or_n("workspace_seconds_limit", self.workspace_seconds_limit)
            .s("workspace_limit_period", &self.workspace_limit_period)
            .null_or_n("member_limit", self.member_limit)
            .n("max_concurrency", self.max_concurrency)
            .n("inbound_events_per_minute", self.inbound_events_per_minute)
            .n("retention_days", self.retention_days)
            .n("audit_retention_days", self.audit_retention_days)
            .n("max_task_turns", self.max_task_turns)
            .f("max_task_cost_usd", self.max_task_cost_usd)
            .n("max_task_timeout_seconds", self.max_task_timeout_seconds)
            .n("max_idle_seconds", self.max_idle_seconds)
            .n("max_children_depth", self.max_children_depth)
            .n("max_children_count", self.max_children_count)
            .strs("computes", &self.computes)
            .n("workspace_cpu", self.workspace_cpu)
            .n("workspace_memory_mib", self.workspace_memory_mib)
            .opt_s("stripe_price_id", self.stripe_price_id.as_ref());
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.s("id")?,
            name: r.s("name")?,
            price_cents: r.n("price_cents")?,
            currency: r.s("currency")?,
            billing_unit: r.s("billing_unit")?,
            task_limit: r.null_or_n("task_limit")?,
            task_limit_period: r.s("task_limit_period")?,
            workspace_seconds_limit: r.null_or_n("workspace_seconds_limit")?,
            workspace_limit_period: r.s("workspace_limit_period")?,
            member_limit: r.null_or_n("member_limit")?,
            max_concurrency: r.n("max_concurrency")?,
            inbound_events_per_minute: r.n("inbound_events_per_minute")?,
            retention_days: r.n("retention_days")?,
            audit_retention_days: r.n("audit_retention_days")?,
            max_task_turns: r.n("max_task_turns")?,
            max_task_cost_usd: r.f("max_task_cost_usd")?,
            max_task_timeout_seconds: r.n("max_task_timeout_seconds")?,
            max_idle_seconds: r.n("max_idle_seconds")?,
            max_children_depth: r.n("max_children_depth")?,
            max_children_count: r.n("max_children_count")?,
            computes: r.strs("computes")?,
            workspace_cpu: r.n("workspace_cpu")?,
            workspace_memory_mib: r.n("workspace_memory_mib")?,
            stripe_price_id: r.opt_s("stripe_price_id")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: Uuid,
    pub cognito_subject: String,
    pub email: String,
    pub display_name: String,
    pub github_user_id: Option<i64>,
    pub github_login: Option<String>,
    pub is_operator: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Entity for User {
    const KIND: &'static str = "user";

    fn key(&self) -> Key {
        keys::user_key(self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .s("cognito_subject", &self.cognito_subject)
            .s("email", &self.email)
            .s("display_name", &self.display_name)
            .opt_n("github_user_id", self.github_user_id)
            .opt_s("github_login", self.github_login.as_ref())
            .b("is_operator", self.is_operator)
            .time("created_at", self.created_at)
            .time("updated_at", self.updated_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            cognito_subject: r.s("cognito_subject")?,
            email: r.s("email")?,
            display_name: r.s("display_name")?,
            github_user_id: r.opt_n("github_user_id")?,
            github_login: r.opt_s("github_login")?,
            is_operator: r.b("is_operator")?,
            created_at: r.time("created_at")?,
            updated_at: r.time("updated_at")?,
        })
    }
}

pub mod uniq {
    pub const COGNITO_SUBJECT: &str = "cognito";
    pub const EMAIL: &str = "email";
    pub const GITHUB_USER: &str = "ghuser";
    pub const GITHUB_LOGIN: &str = "ghlogin";
    pub const SLUG: &str = "slug";
    pub const STRIPE_CUSTOMER: &str = "stripe";
    pub const STRIPE_EVENT: &str = "stripe_event";
    pub const FREE_TENANT: &str = "free_tenant";
    pub const PENDING_CHECKOUT: &str = "pending_checkout";
    pub const INSTALLATION: &str = "installation";
    pub const SLACK_TEAM: &str = "slack";
}

#[derive(Debug, Clone, PartialEq)]
pub struct Guard {
    pub key: Key,
    pub target: Uuid,
    pub tenant_id: Option<Uuid>,
    pub at: Option<DateTime<Utc>>,
}

impl Guard {
    pub fn new(key: Key, target: Uuid) -> Self {
        Self {
            key,
            target,
            tenant_id: None,
            at: None,
        }
    }

    pub fn of_tenant(key: Key, target: Uuid, tenant_id: Uuid) -> Self {
        Self {
            key,
            target,
            tenant_id: Some(tenant_id),
            at: None,
        }
    }
}

impl Entity for Guard {
    const KIND: &'static str = "guard";

    fn key(&self) -> Key {
        self.key.clone()
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("target", self.target)
            .opt_uuid("tenant_id", self.tenant_id)
            .opt_time("at", self.at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            key: Key::main(r.s("pk")?, r.s("sk")?),
            target: r.uuid("target")?,
            tenant_id: r.opt_uuid("tenant_id")?,
            at: r.opt_time("at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ref {
    pub tenant_id: Uuid,
    pub id: Uuid,
    pub kind: String,
    pub target: Key,
    pub parent_id: Option<Uuid>,
}

impl Entity for Ref {
    const KIND: &'static str = "ref";

    fn key(&self) -> Key {
        keys::ref_key(self.tenant_id, self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .uuid("id", self.id)
            .s("ref_kind", &self.kind)
            .s("target_pk", &self.target.pk)
            .s("target_sk", &self.target.sk)
            .opt_uuid("parent_id", self.parent_id);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            id: r.uuid("id")?,
            kind: r.s("ref_kind")?,
            target: Key::main(r.s("target_pk")?, r.s("target_sk")?),
            parent_id: r.opt_uuid("parent_id")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tenant {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub plan_id: String,
    pub period_start: DateTime<Utc>,
    pub tasks_this_period: i64,
    pub workspace_seconds_this_period: i64,
    pub running_tasks: i64,
    pub status: String,
    pub suspended_reason: Option<String>,
    pub stripe_customer_id: Option<String>,
    pub created_by_user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Entity for Tenant {
    const KIND: &'static str = "tenant";

    fn key(&self) -> Key {
        keys::tenant_key(self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .s("slug", &self.slug)
            .s("name", &self.name)
            .s("plan_id", &self.plan_id)
            .time("period_start", self.period_start)
            .n("tasks_this_period", self.tasks_this_period)
            .n(
                "workspace_seconds_this_period",
                self.workspace_seconds_this_period,
            )
            .n("running_tasks", self.running_tasks)
            .s("status", &self.status)
            .opt_s("suspended_reason", self.suspended_reason.as_ref())
            .opt_s("stripe_customer_id", self.stripe_customer_id.as_ref())
            .uuid("created_by_user_id", self.created_by_user_id)
            .time("created_at", self.created_at)
            .time("updated_at", self.updated_at)
            .s("gsi2pk", keys::TENANTS_INDEX)
            .s("gsi2sk", self.id.to_string());
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            slug: r.s("slug")?,
            name: r.s("name")?,
            plan_id: r.s("plan_id")?,
            period_start: r.time("period_start")?,
            tasks_this_period: r.n("tasks_this_period")?,
            workspace_seconds_this_period: r.n("workspace_seconds_this_period")?,
            running_tasks: r.n("running_tasks")?,
            status: r.s("status")?,
            suspended_reason: r.opt_s("suspended_reason")?,
            stripe_customer_id: r.opt_s("stripe_customer_id")?,
            created_by_user_id: r.uuid("created_by_user_id")?,
            created_at: r.time("created_at")?,
            updated_at: r.time("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub role: String,
    pub joined_at: DateTime<Utc>,
    pub last_active_at: Option<DateTime<Utc>>,
}

impl Entity for Member {
    const KIND: &'static str = "member";

    fn key(&self) -> Key {
        keys::member_key(self.tenant_id, self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("user_id", self.user_id)
            .s("role", &self.role)
            .time("joined_at", self.joined_at)
            .opt_time("last_active_at", self.last_active_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            user_id: r.uuid("user_id")?,
            role: r.s("role")?,
            joined_at: r.time("joined_at")?,
            last_active_at: r.opt_time("last_active_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Membership {
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub member_id: Uuid,
}

impl Entity for Membership {
    const KIND: &'static str = "membership";

    fn key(&self) -> Key {
        keys::membership_key(self.user_id, self.tenant_id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("user_id", self.user_id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("member_id", self.member_id);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            user_id: r.uuid("user_id")?,
            tenant_id: r.uuid("tenant_id")?,
            member_id: r.uuid("member_id")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Invitation {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub email: String,
    pub role: String,
    pub status: String,
    pub invited_by_member_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl Invitation {
    pub fn is_pending_at(&self, at: DateTime<Utc>) -> bool {
        self.status == "pending" && self.expires_at > at
    }
}

impl Entity for Invitation {
    const KIND: &'static str = "invitation";

    fn key(&self) -> Key {
        keys::invitation_key(self.tenant_id, self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .s("email", &self.email)
            .s("role", &self.role)
            .s("status", &self.status)
            .uuid("invited_by_member_id", self.invited_by_member_id)
            .time("expires_at", self.expires_at)
            .opt_time("accepted_at", self.accepted_at)
            .time("created_at", self.created_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            email: r.s("email")?,
            role: r.s("role")?,
            status: r.s("status")?,
            invited_by_member_id: r.uuid("invited_by_member_id")?,
            expires_at: r.time("expires_at")?,
            accepted_at: r.opt_time("accepted_at")?,
            created_at: r.time("created_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InvitationEmail {
    pub email: String,
    pub tenant_id: Uuid,
    pub invitation_id: Uuid,
}

impl Entity for InvitationEmail {
    const KIND: &'static str = "invitation_email";

    fn key(&self) -> Key {
        keys::invitation_email_key(&self.email, self.tenant_id, self.invitation_id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.s("email", self.email.to_lowercase())
            .uuid("tenant_id", self.tenant_id)
            .uuid("invitation_id", self.invitation_id);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            email: r.s("email")?,
            tenant_id: r.uuid("tenant_id")?,
            invitation_id: r.uuid("invitation_id")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Credential {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub scope: String,
    pub member_id: Option<Uuid>,
    pub name: String,
    pub provider: String,
    pub kind: String,
    pub account: String,
    pub status: String,
    pub subscription_plan: Option<String>,
    pub rejected_at: Option<DateTime<Utc>>,
    pub encrypted_value: String,
    pub kms_key_id: String,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Entity for Credential {
    const KIND: &'static str = "credential";

    fn key(&self) -> Key {
        keys::credential_key(self.tenant_id, self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .s("scope", &self.scope)
            .opt_uuid("member_id", self.member_id)
            .s("name", &self.name)
            .s("provider", &self.provider)
            .s("credential_kind", &self.kind)
            .s("account", &self.account)
            .s("status", &self.status)
            .opt_s("subscription_plan", self.subscription_plan.as_ref())
            .opt_time("rejected_at", self.rejected_at)
            .s("encrypted_value", &self.encrypted_value)
            .s("kms_key_id", &self.kms_key_id)
            .opt_time("last_used_at", self.last_used_at)
            .time("created_at", self.created_at)
            .time("updated_at", self.updated_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            scope: r.s("scope")?,
            member_id: r.opt_uuid("member_id")?,
            name: r.s("name")?,
            provider: r.s("provider")?,
            kind: r.s("credential_kind")?,
            account: r.s("account")?,
            status: r.s("status")?,
            subscription_plan: r.opt_s("subscription_plan")?,
            rejected_at: r.opt_time("rejected_at")?,
            encrypted_value: r.s("encrypted_value")?,
            kms_key_id: r.s("kms_key_id")?,
            last_used_at: r.opt_time("last_used_at")?,
            created_at: r.time("created_at")?,
            updated_at: r.time("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageSnapshot {
    pub tenant_id: Uuid,
    pub credential_id: Uuid,
    pub captured_at: DateTime<Utc>,
    pub five_hour_utilization: f64,
    pub five_hour_resets_at: DateTime<Utc>,
    pub seven_day_utilization: f64,
    pub seven_day_resets_at: DateTime<Utc>,
}

impl Entity for UsageSnapshot {
    const KIND: &'static str = "usage_snapshot";

    fn key(&self) -> Key {
        keys::usage_snapshot_key(self.tenant_id, self.credential_id, self.captured_at)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .uuid("credential_id", self.credential_id)
            .time("captured_at", self.captured_at)
            .f("five_hour_utilization", self.five_hour_utilization)
            .time("five_hour_resets_at", self.five_hour_resets_at)
            .f("seven_day_utilization", self.seven_day_utilization)
            .time("seven_day_resets_at", self.seven_day_resets_at)
            .ttl(self.captured_at + Duration::days(USAGE_SNAPSHOT_RETENTION_DAYS));
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            credential_id: r.uuid("credential_id")?,
            captured_at: r.time("captured_at")?,
            five_hour_utilization: r.f("five_hour_utilization")?,
            five_hour_resets_at: r.time("five_hour_resets_at")?,
            seven_day_utilization: r.f("seven_day_utilization")?,
            seven_day_resets_at: r.time("seven_day_resets_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Integration {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub kind: String,
    pub status: String,
    pub github_installation_id: Option<i64>,
    pub slack_team_id: Option<String>,
    pub details: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Entity for Integration {
    const KIND: &'static str = "integration";

    fn key(&self) -> Key {
        keys::integration_key(self.tenant_id, self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .s("integration_kind", &self.kind)
            .s("status", &self.status)
            .opt_n("github_installation_id", self.github_installation_id)
            .opt_s("slack_team_id", self.slack_team_id.as_ref())
            .json("details", &self.details)
            .time("created_at", self.created_at)
            .time("updated_at", self.updated_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            kind: r.s("integration_kind")?,
            status: r.s("status")?,
            github_installation_id: r.opt_n("github_installation_id")?,
            slack_team_id: r.opt_s("slack_team_id")?,
            details: r.json("details")?,
            created_at: r.time("created_at")?,
            updated_at: r.time("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub repository_id: Option<i64>,
    pub repository: String,
    pub root_path: String,
    pub branch: String,
    pub status: String,
    pub active_revision_id: Option<Uuid>,
    pub revisions: i64,
    pub repositories: BTreeMap<String, GitHubRepository>,
    pub synced_at: Option<DateTime<Utc>>,
    pub sync_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub const CONFIG_ACTIVE: &str = "active";
pub const CONFIG_DISABLED: &str = "disabled";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubRepository {
    pub id: i64,
    pub full_name: String,
}

impl Config {
    pub fn is_active(&self) -> bool {
        self.status == CONFIG_ACTIVE
    }

    pub fn repository_named(&self, name: &str) -> Option<&GitHubRepository> {
        self.repositories
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, repository)| repository)
    }
}

pub fn github_repositories(details: &Value) -> Result<Vec<GitHubRepository>, DbError> {
    match details.get("repositories") {
        None => Ok(Vec::new()),
        Some(list) => serde_json::from_value(list.clone()).map_err(|error| {
            DbError::Decode(format!(
                "integration details.repositories is not a list of id and full_name: {error}"
            ))
        }),
    }
}

impl Entity for Config {
    const KIND: &'static str = "config";

    fn key(&self) -> Key {
        keys::config_key(self.tenant_id, self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        let synced = self
            .synced_at
            .map(format_time)
            .unwrap_or_else(|| "0".into());
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .opt_n("repository_id", self.repository_id)
            .s("repository", &self.repository)
            .s("root_path", &self.root_path)
            .s("branch", &self.branch)
            .s("status", &self.status)
            .opt_uuid("active_revision_id", self.active_revision_id)
            .n("revisions", self.revisions)
            .json(
                "repositories",
                &serde_json::to_value(&self.repositories).expect("repositories serialize"),
            )
            .opt_time("synced_at", self.synced_at)
            .opt_s("sync_error", self.sync_error.as_ref())
            .time("created_at", self.created_at)
            .time("updated_at", self.updated_at);
        if self.is_active() {
            w.s("gsi2pk", keys::SWEEP_CONFIGS)
                .s("gsi2sk", format!("{synced}#{}#{}", self.tenant_id, self.id));
        }
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            repository_id: r.opt_n("repository_id")?,
            repository: r.s("repository")?,
            root_path: r.s("root_path")?,
            branch: r.s("branch")?,
            status: r.s("status")?,
            active_revision_id: r.opt_uuid("active_revision_id")?,
            revisions: r.n("revisions")?,
            repositories: serde_json::from_value(r.json("repositories")?).map_err(|error| {
                DbError::Decode(format!("config repositories do not decode: {error}"))
            })?,
            synced_at: r.opt_time("synced_at")?,
            sync_error: r.opt_s("sync_error")?,
            created_at: r.time("created_at")?,
            updated_at: r.time("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigRevision {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub config_id: Uuid,
    pub number: i64,
    pub commit_sha: String,
    pub parsed: Option<Value>,
    pub validation_status: String,
    pub validation_messages: Value,
    pub created_at: DateTime<Utc>,
}

impl Entity for ConfigRevision {
    const KIND: &'static str = "config_revision";

    fn key(&self) -> Key {
        keys::revision_key(self.tenant_id, self.config_id, self.number)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("config_id", self.config_id)
            .n("number", self.number)
            .s("commit_sha", &self.commit_sha)
            .opt_json("parsed", self.parsed.as_ref())
            .s("validation_status", &self.validation_status)
            .json("validation_messages", &self.validation_messages)
            .time("created_at", self.created_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            config_id: r.uuid("config_id")?,
            number: r.n("number")?,
            commit_sha: r.s("commit_sha")?,
            parsed: r.opt_json("parsed")?,
            validation_status: r.s("validation_status")?,
            validation_messages: r.json("validation_messages")?,
            created_at: r.time("created_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub config_id: Uuid,
    pub config_revision_id: Uuid,
    pub name: String,
    pub system_prompt: String,
    pub source_path: String,
}

impl Entity for Project {
    const KIND: &'static str = "project";

    fn key(&self) -> Key {
        keys::project_key(
            self.tenant_id,
            self.config_id,
            self.config_revision_id,
            &self.name,
        )
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("config_id", self.config_id)
            .uuid("config_revision_id", self.config_revision_id)
            .s("name", &self.name)
            .s("system_prompt", &self.system_prompt)
            .s("source_path", &self.source_path);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            config_id: r.uuid("config_id")?,
            config_revision_id: r.uuid("config_revision_id")?,
            name: r.s("name")?,
            system_prompt: r.s("system_prompt")?,
            source_path: r.s("source_path")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Repository {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub config_id: Uuid,
    pub config_revision_id: Uuid,
    pub project: String,
    pub name: String,
    pub url: String,
    pub github_repository_id: i64,
    pub full_name: String,
    pub warm_hook: Option<Value>,
    pub start_hook: Option<Value>,
    pub prebuild_status: Option<String>,
    pub prebuild_at: Option<DateTime<Utc>>,
}

impl Entity for Repository {
    const KIND: &'static str = "repository";

    fn key(&self) -> Key {
        keys::repository_key(
            self.tenant_id,
            self.config_id,
            self.config_revision_id,
            &self.project,
            &self.name,
        )
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("config_id", self.config_id)
            .uuid("config_revision_id", self.config_revision_id)
            .s("project", &self.project)
            .s("name", &self.name)
            .s("url", &self.url)
            .n("github_repository_id", self.github_repository_id)
            .s("full_name", &self.full_name)
            .opt_json("warm_hook", self.warm_hook.as_ref())
            .opt_json("start_hook", self.start_hook.as_ref())
            .opt_s("prebuild_status", self.prebuild_status.as_ref())
            .opt_time("prebuild_at", self.prebuild_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            config_id: r.uuid("config_id")?,
            config_revision_id: r.uuid("config_revision_id")?,
            project: r.s("project")?,
            name: r.s("name")?,
            url: r.s("url")?,
            github_repository_id: r.n("github_repository_id")?,
            full_name: r.s("full_name")?,
            warm_hook: r.opt_json("warm_hook")?,
            start_hook: r.opt_json("start_hook")?,
            prebuild_status: r.opt_s("prebuild_status")?,
            prebuild_at: r.opt_time("prebuild_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionDef {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub config_id: Uuid,
    pub config_revision_id: Uuid,
    pub name: String,
    pub definition: Value,
}

impl Entity for SessionDef {
    const KIND: &'static str = "session";

    fn key(&self) -> Key {
        keys::session_key(
            self.tenant_id,
            self.config_id,
            self.config_revision_id,
            &self.name,
        )
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("config_id", self.config_id)
            .uuid("config_revision_id", self.config_revision_id)
            .s("name", &self.name)
            .json("definition", &self.definition);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            config_id: r.uuid("config_id")?,
            config_revision_id: r.uuid("config_revision_id")?,
            name: r.s("name")?,
            definition: r.json("definition")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillDef {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub config_id: Uuid,
    pub config_revision_id: Uuid,
    pub name: String,
    pub kind: String,
    pub description: String,
    pub skill_md: String,
    pub skill_yaml: Option<String>,
    pub definition: Option<Value>,
    pub layout: Option<Value>,
}

impl Entity for SkillDef {
    const KIND: &'static str = "skill";

    fn key(&self) -> Key {
        keys::skill_key(
            self.tenant_id,
            self.config_id,
            self.config_revision_id,
            &self.name,
        )
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("config_id", self.config_id)
            .uuid("config_revision_id", self.config_revision_id)
            .s("name", &self.name)
            .s("skill_kind", &self.kind)
            .s("description", &self.description)
            .s("skill_md", &self.skill_md)
            .opt_s("skill_yaml", self.skill_yaml.as_ref())
            .opt_json("definition", self.definition.as_ref())
            .opt_json("layout", self.layout.as_ref());
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            config_id: r.uuid("config_id")?,
            config_revision_id: r.uuid("config_revision_id")?,
            name: r.s("name")?,
            kind: r.s("skill_kind")?,
            description: r.s("description")?,
            skill_md: r.s("skill_md")?,
            skill_yaml: r.opt_s("skill_yaml")?,
            definition: r.opt_json("definition")?,
            layout: r.opt_json("layout")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Trigger {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub config_id: Uuid,
    pub config_revision_id: Uuid,
    pub name: String,
    pub state: String,
    pub event_pattern: Option<Value>,
    pub schedule_expression: Option<String>,
    pub input_transformer: Option<Value>,
    pub start: Value,
    pub removed_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub schedule_message_id: Option<Uuid>,
    pub next: Option<NextTrigger>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextTrigger {
    pub config_revision_id: Uuid,
    pub definition: Option<TriggerSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerSpec {
    pub state: String,
    pub event_pattern: Option<Value>,
    pub schedule_expression: Option<String>,
    pub input_transformer: Option<Value>,
    pub start: Value,
}

impl Trigger {
    pub fn is_active(&self) -> bool {
        self.state == "ENABLED" && self.removed_at.is_none()
    }

    pub fn spec(&self) -> TriggerSpec {
        TriggerSpec {
            state: self.state.clone(),
            event_pattern: self.event_pattern.clone(),
            schedule_expression: self.schedule_expression.clone(),
            input_transformer: self.input_transformer.clone(),
            start: self.start.clone(),
        }
    }

    pub fn in_revision(&self, revision_id: Option<Uuid>) -> Option<Trigger> {
        let revision_id = revision_id?;
        match &self.next {
            Some(next) if next.config_revision_id == revision_id => {
                next.definition.as_ref().map(|spec| Trigger {
                    config_revision_id: revision_id,
                    state: spec.state.clone(),
                    event_pattern: spec.event_pattern.clone(),
                    schedule_expression: spec.schedule_expression.clone(),
                    input_transformer: spec.input_transformer.clone(),
                    start: spec.start.clone(),
                    removed_at: None,
                    next: None,
                    ..self.clone()
                })
            }
            _ => (self.config_revision_id == revision_id && self.removed_at.is_none()).then(|| {
                Trigger {
                    next: None,
                    ..self.clone()
                }
            }),
        }
    }
}

impl Entity for Trigger {
    const KIND: &'static str = "trigger";

    fn key(&self) -> Key {
        keys::trigger_key(self.tenant_id, self.config_id, &self.name)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("config_id", self.config_id)
            .uuid("config_revision_id", self.config_revision_id)
            .s("name", &self.name)
            .s("state", &self.state)
            .opt_json("event_pattern", self.event_pattern.as_ref())
            .opt_s("schedule_expression", self.schedule_expression.as_ref())
            .opt_json("input_transformer", self.input_transformer.as_ref())
            .json("start", &self.start)
            .opt_time("removed_at", self.removed_at)
            .opt_time("next_run_at", self.next_run_at)
            .opt_uuid("schedule_message_id", self.schedule_message_id)
            .opt_json(
                "next",
                self.next
                    .as_ref()
                    .map(|next| serde_json::to_value(next).expect("JSON values serialize"))
                    .as_ref(),
            );
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            config_id: r.uuid("config_id")?,
            config_revision_id: r.uuid("config_revision_id")?,
            name: r.s("name")?,
            state: r.s("state")?,
            event_pattern: r.opt_json("event_pattern")?,
            schedule_expression: r.opt_s("schedule_expression")?,
            input_transformer: r.opt_json("input_transformer")?,
            start: r.json("start")?,
            removed_at: r.opt_time("removed_at")?,
            next_run_at: r.opt_time("next_run_at")?,
            schedule_message_id: r.opt_uuid("schedule_message_id")?,
            next: r
                .opt_json("next")?
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| DbError::Decode(format!("trigger next: {e}")))?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChangeRequest {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub config_id: Uuid,
    pub pr_number: i64,
    pub title: String,
    pub state: String,
    pub changed_files: Value,
    pub changed_definitions: Value,
    pub validation_status: Option<String>,
    pub validation_messages: Value,
    pub requested_by_member_id: Option<Uuid>,
    pub requested_by_task_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Entity for ChangeRequest {
    const KIND: &'static str = "change_request";

    fn key(&self) -> Key {
        keys::change_request_key(self.tenant_id, self.config_id, self.pr_number)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("config_id", self.config_id)
            .n("pr_number", self.pr_number)
            .s("title", &self.title)
            .s("state", &self.state)
            .json("changed_files", &self.changed_files)
            .json("changed_definitions", &self.changed_definitions)
            .opt_s("validation_status", self.validation_status.as_ref())
            .json("validation_messages", &self.validation_messages)
            .opt_uuid("requested_by_member_id", self.requested_by_member_id)
            .opt_uuid("requested_by_task_id", self.requested_by_task_id)
            .time("created_at", self.created_at)
            .time("updated_at", self.updated_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            config_id: r.uuid("config_id")?,
            pr_number: r.n("pr_number")?,
            title: r.s("title")?,
            state: r.s("state")?,
            changed_files: r.json("changed_files")?,
            changed_definitions: r.json("changed_definitions")?,
            validation_status: r.opt_s("validation_status")?,
            validation_messages: r.json("validation_messages")?,
            requested_by_member_id: r.opt_uuid("requested_by_member_id")?,
            requested_by_task_id: r.opt_uuid("requested_by_task_id")?,
            created_at: r.time("created_at")?,
            updated_at: r.time("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerEvent {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub trigger_id: Uuid,
    pub key: String,
    pub event: Value,
    pub received_at: DateTime<Utc>,
    pub result: Option<String>,
    pub reason: Option<String>,
    pub evaluated_at: Option<DateTime<Utc>>,
    pub task_id: Option<Uuid>,
}

impl Entity for TriggerEvent {
    const KIND: &'static str = "trigger_event";

    fn key(&self) -> Key {
        keys::trigger_event_key(self.tenant_id, self.trigger_id, &self.key)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("trigger_id", self.trigger_id)
            .s("key", &self.key)
            .json("event", &self.event)
            .time("received_at", self.received_at)
            .opt_s("result", self.result.as_ref())
            .opt_s("reason", self.reason.as_ref())
            .opt_time("evaluated_at", self.evaluated_at)
            .opt_uuid("task_id", self.task_id)
            .s("gsi1pk", keys::trigger_events_index(self.tenant_id))
            .s("gsi1sk", keys::time_id(self.received_at, self.id));
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            trigger_id: r.uuid("trigger_id")?,
            key: r.s("key")?,
            event: r.json("event")?,
            received_at: r.time("received_at")?,
            result: r.opt_s("result")?,
            reason: r.opt_s("reason")?,
            evaluated_at: r.opt_time("evaluated_at")?,
            task_id: r.opt_uuid("task_id")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskName {
    pub tenant_id: Uuid,
    pub trigger_id: Uuid,
    pub name: String,
    pub task_id: Uuid,
    pub created_at: DateTime<Utc>,
}

impl Entity for TaskName {
    const KIND: &'static str = "task_name";

    fn key(&self) -> Key {
        keys::task_name_key(self.tenant_id, self.trigger_id, &self.name)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .uuid("trigger_id", self.trigger_id)
            .s("name", &self.name)
            .uuid("task_id", self.task_id)
            .time("created_at", self.created_at)
            .s("gsi1pk", keys::task_name_index(self.tenant_id, &self.name))
            .s("gsi1sk", keys::time_id(self.created_at, self.task_id));
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            trigger_id: r.uuid("trigger_id")?,
            name: r.s("name")?,
            task_id: r.uuid("task_id")?,
            created_at: r.time("created_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub trigger_id: Uuid,
    pub trigger_event_id: Option<Uuid>,
    pub config_revision_id: Uuid,
    pub name: String,
    pub title: String,
    pub status: TaskStatus,
    pub wait_reason: Option<WaitReason>,
    pub resume_at: Option<DateTime<Utc>>,
    pub snoozed_until: Option<DateTime<Utc>>,
    pub nudges: i64,
    pub definition: Value,
    pub parent_task_id: Option<Uuid>,
    pub channel: Option<Value>,
    pub start_payload: Option<Value>,
    pub requested_by_member_id: Option<Uuid>,
    pub error: Option<String>,
    pub cause: Option<String>,
    pub runs: i64,
    pub skill_runs: i64,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

pub const GITHUB_TASK_PREFIX: &str = "github:";

pub fn github_task_name(repository_id: i64, suffix: &str) -> String {
    format!("{GITHUB_TASK_PREFIX}{repository_id}{suffix}")
}

pub fn split_github_task_name(name: &str) -> Option<(i64, &str)> {
    let rest = name.strip_prefix(GITHUB_TASK_PREFIX)?;
    let end = rest.find(['#', '@']).unwrap_or(rest.len());
    Some((rest[..end].parse().ok()?, &rest[end..]))
}

impl Task {
    pub fn repository_id(&self) -> Option<i64> {
        split_github_task_name(&self.name).map(|(id, _)| id)
    }

    pub fn display_name(&self, repositories: &[GitHubRepository]) -> String {
        let Some((id, suffix)) = split_github_task_name(&self.name) else {
            return self.name.clone();
        };
        let current = repositories
            .iter()
            .find(|repository| repository.id == id)
            .map(|repository| repository.full_name.clone())
            .or_else(|| {
                self.channel
                    .as_ref()
                    .and_then(|channel| channel["repository"].as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("#{id}"));
        format!("{current}{suffix}")
    }
}

impl Entity for Task {
    const KIND: &'static str = "task";

    fn key(&self) -> Key {
        keys::task_key(self.tenant_id, self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("trigger_id", self.trigger_id)
            .opt_uuid("trigger_event_id", self.trigger_event_id)
            .uuid("config_revision_id", self.config_revision_id)
            .s("name", &self.name)
            .s("title", &self.title)
            .s("status", self.status.as_str())
            .opt_s("wait_reason", self.wait_reason.map(|w| w.as_str()))
            .opt_time("resume_at", self.resume_at)
            .opt_time("snoozed_until", self.snoozed_until)
            .n("nudges", self.nudges)
            .json("definition", &self.definition)
            .opt_uuid("parent_task_id", self.parent_task_id)
            .opt_json("channel", self.channel.as_ref())
            .opt_json("start_payload", self.start_payload.as_ref())
            .opt_uuid("requested_by_member_id", self.requested_by_member_id)
            .opt_s("error", self.error.as_ref())
            .opt_s("cause", self.cause.as_ref())
            .n("runs", self.runs)
            .n("skill_runs", self.skill_runs)
            .time("created_at", self.created_at)
            .opt_time("started_at", self.started_at)
            .opt_time("finished_at", self.finished_at)
            .time("updated_at", self.updated_at)
            .s("gsi1pk", keys::tasks_index(self.tenant_id))
            .s("gsi1sk", keys::time_id(self.created_at, self.id));
        if matches!(
            self.status,
            TaskStatus::Running | TaskStatus::Waiting | TaskStatus::NeedsYou
        ) {
            w.s("gsi2pk", keys::SWEEP_ACTIVE_TASKS)
                .s("gsi2sk", format!("{}#{}", self.tenant_id, self.id));
        }
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            trigger_id: r.uuid("trigger_id")?,
            trigger_event_id: r.opt_uuid("trigger_event_id")?,
            config_revision_id: r.uuid("config_revision_id")?,
            name: r.s("name")?,
            title: r.s("title")?,
            status: r.parse("status")?,
            wait_reason: r
                .opt_s("wait_reason")?
                .map(|s| {
                    s.parse()
                        .map_err(|_| DbError::Decode(format!("wait reason {s}")))
                })
                .transpose()?,
            resume_at: r.opt_time("resume_at")?,
            snoozed_until: r.opt_time("snoozed_until")?,
            nudges: r.n("nudges")?,
            definition: r.json("definition")?,
            parent_task_id: r.opt_uuid("parent_task_id")?,
            channel: r.opt_json("channel")?,
            start_payload: r.opt_json("start_payload")?,
            requested_by_member_id: r.opt_uuid("requested_by_member_id")?,
            error: r.opt_s("error")?,
            cause: r.opt_s("cause")?,
            runs: r.n("runs")?,
            skill_runs: r.n("skill_runs")?,
            created_at: r.time("created_at")?,
            started_at: r.opt_time("started_at")?,
            finished_at: r.opt_time("finished_at")?,
            updated_at: r.time("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dependency {
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub blocked_by_task_id: Uuid,
}

impl Entity for Dependency {
    const KIND: &'static str = "task_dependency";

    fn key(&self) -> Key {
        keys::dependency_key(self.tenant_id, self.task_id, self.blocked_by_task_id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .uuid("task_id", self.task_id)
            .uuid("blocked_by_task_id", self.blocked_by_task_id);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            task_id: r.uuid("task_id")?,
            blocked_by_task_id: r.uuid("blocked_by_task_id")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Child {
    pub tenant_id: Uuid,
    pub parent_task_id: Uuid,
    pub task_id: Uuid,
    pub created_at: DateTime<Utc>,
}

impl Entity for Child {
    const KIND: &'static str = "task_child";

    fn key(&self) -> Key {
        keys::child_key(
            self.tenant_id,
            self.parent_task_id,
            self.created_at,
            self.task_id,
        )
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .uuid("parent_task_id", self.parent_task_id)
            .uuid("task_id", self.task_id)
            .time("created_at", self.created_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            parent_task_id: r.uuid("parent_task_id")?,
            task_id: r.uuid("task_id")?,
            created_at: r.time("created_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingTask {
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub created_at: DateTime<Utc>,
}

impl Entity for PendingTask {
    const KIND: &'static str = "pending_task";

    fn key(&self) -> Key {
        keys::pending_task_key(self.tenant_id, self.created_at, self.task_id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .uuid("task_id", self.task_id)
            .time("created_at", self.created_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            task_id: r.uuid("task_id")?,
            created_at: r.time("created_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub number: i64,
    pub session_name: String,
    pub worker: String,
    pub model: String,
    pub agent_session_id: Option<String>,
    pub status: String,
    pub turns: i64,
    pub cost_usd: f64,
    pub instance_costs: BTreeMap<String, f64>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub resume_branch: Option<String>,
    pub record_s3_key: Option<String>,
    pub log_s3_key: Option<String>,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl Run {
    pub fn is_finished(&self) -> bool {
        matches!(self.status.as_str(), "completed" | "failed")
    }
}

impl Entity for Run {
    const KIND: &'static str = "run";

    fn key(&self) -> Key {
        keys::run_key(self.tenant_id, self.task_id, self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("task_id", self.task_id)
            .n("number", self.number)
            .s("session_name", &self.session_name)
            .s("worker", &self.worker)
            .s("model", &self.model)
            .opt_s("agent_session_id", self.agent_session_id.as_ref())
            .s("status", &self.status)
            .n("turns", self.turns)
            .f("cost_usd", self.cost_usd)
            .json(
                "instance_costs",
                &serde_json::to_value(&self.instance_costs).expect("costs serialize"),
            )
            .opt_s("summary", self.summary.as_ref())
            .opt_s("error", self.error.as_ref())
            .opt_s("resume_branch", self.resume_branch.as_ref())
            .opt_s("record_s3_key", self.record_s3_key.as_ref())
            .opt_s("log_s3_key", self.log_s3_key.as_ref())
            .opt_time("last_heartbeat_at", self.last_heartbeat_at)
            .time("started_at", self.started_at)
            .opt_time("finished_at", self.finished_at)
            .time("updated_at", self.updated_at)
            .s("gsi1pk", keys::runs_index(self.tenant_id))
            .s("gsi1sk", keys::time_id(self.started_at, self.id));
        if !self.is_finished() {
            w.s("gsi2pk", keys::open_runs_index(self.tenant_id))
                .s("gsi2sk", keys::time_id(self.started_at, self.id));
        }
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            task_id: r.uuid("task_id")?,
            number: r.n("number")?,
            session_name: r.s("session_name")?,
            worker: r.s("worker")?,
            model: r.s("model")?,
            agent_session_id: r.opt_s("agent_session_id")?,
            status: r.s("status")?,
            turns: r.n("turns")?,
            cost_usd: r.f("cost_usd")?,
            instance_costs: serde_json::from_value(r.json("instance_costs")?)
                .map_err(|e| DbError::Decode(format!("attribute instance_costs: {e}")))?,
            summary: r.opt_s("summary")?,
            error: r.opt_s("error")?,
            resume_branch: r.opt_s("resume_branch")?,
            record_s3_key: r.opt_s("record_s3_key")?,
            log_s3_key: r.opt_s("log_s3_key")?,
            last_heartbeat_at: r.opt_time("last_heartbeat_at")?,
            started_at: r.time("started_at")?,
            finished_at: r.opt_time("finished_at")?,
            updated_at: r.time("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub compute: String,
    pub architecture: String,
    pub cpu: i64,
    pub memory_mib: i64,
    pub platform: String,
    pub platform_ref: Option<String>,
    pub capacity_provider: Option<String>,
    pub repositories: Vec<String>,
    pub branch: String,
    pub base_commit: Option<String>,
    pub started_at: DateTime<Utc>,
    pub signaled_at: Option<DateTime<Utc>>,
    pub stopped_at: Option<DateTime<Utc>>,
}

impl Entity for Workspace {
    const KIND: &'static str = "workspace";

    fn key(&self) -> Key {
        keys::workspace_key(self.tenant_id, self.task_id, self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("task_id", self.task_id)
            .uuid("run_id", self.run_id)
            .s("compute", &self.compute)
            .s("architecture", &self.architecture)
            .n("cpu", self.cpu)
            .n("memory_mib", self.memory_mib)
            .s("platform", &self.platform)
            .opt_s("platform_ref", self.platform_ref.as_ref())
            .opt_s("capacity_provider", self.capacity_provider.as_ref())
            .strs("repositories", &self.repositories)
            .s("branch", &self.branch)
            .opt_s("base_commit", self.base_commit.as_ref())
            .time("started_at", self.started_at)
            .opt_time("signaled_at", self.signaled_at)
            .opt_time("stopped_at", self.stopped_at)
            .s("gsi1pk", keys::workspaces_index(self.tenant_id))
            .s("gsi1sk", keys::time_id(self.started_at, self.id));
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            task_id: r.uuid("task_id")?,
            run_id: r.uuid("run_id")?,
            compute: r.s("compute")?,
            architecture: r.s("architecture")?,
            cpu: r.n("cpu")?,
            memory_mib: r.n("memory_mib")?,
            platform: r.s("platform")?,
            platform_ref: r.opt_s("platform_ref")?,
            capacity_provider: r.opt_s("capacity_provider")?,
            repositories: r.strs("repositories")?,
            branch: r.s("branch")?,
            base_commit: r.opt_s("base_commit")?,
            started_at: r.time("started_at")?,
            signaled_at: r.opt_time("signaled_at")?,
            stopped_at: r.opt_time("stopped_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillRunItem {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Option<Uuid>,
    pub number: i32,
    pub skill_name: String,
    pub definition: Value,
    pub input: Value,
    pub output: Option<Value>,
    pub status: SkillRunStatus,
    pub current_state: Option<String>,
    pub state: Value,
    pub error: Option<String>,
    pub cause: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl Entity for SkillRunItem {
    const KIND: &'static str = "skill_run";

    fn key(&self) -> Key {
        keys::skill_run_key(self.tenant_id, self.task_id, self.number)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("task_id", self.task_id)
            .opt_uuid("run_id", self.run_id)
            .n("number", i64::from(self.number))
            .s("skill_name", &self.skill_name)
            .json("definition", &self.definition)
            .json("input", &self.input)
            .opt_json("output", self.output.as_ref())
            .s("status", self.status.as_str())
            .opt_s("current_state", self.current_state.as_ref())
            .json("state", &self.state)
            .opt_s("error", self.error.as_ref())
            .opt_s("cause", self.cause.as_ref())
            .time("started_at", self.started_at)
            .opt_time("finished_at", self.finished_at)
            .time("updated_at", self.updated_at)
            .s(
                "gsi1pk",
                keys::skill_runs_index(self.tenant_id, &self.skill_name),
            )
            .s("gsi1sk", keys::time_id(self.started_at, self.id));
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            task_id: r.uuid("task_id")?,
            run_id: r.opt_uuid("run_id")?,
            number: r.i32("number")?,
            skill_name: r.s("skill_name")?,
            definition: r.json("definition")?,
            input: r.json("input")?,
            output: r.opt_json("output")?,
            status: r.parse("status")?,
            current_state: r.opt_s("current_state")?,
            state: r.json("state")?,
            error: r.opt_s("error")?,
            cause: r.opt_s("cause")?,
            started_at: r.time("started_at")?,
            finished_at: r.opt_time("finished_at")?,
            updated_at: r.time("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StateEntryItem {
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub skill_run_id: Uuid,
    pub skill_run_number: i32,
    pub sequence: i64,
    pub state_name: String,
    pub state_type: String,
    pub status: String,
    pub entered_at: DateTime<Utc>,
    pub exited_at: Option<DateTime<Utc>>,
    pub input: Value,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub cause: Option<String>,
    pub attempts: i64,
    pub next_state: Option<String>,
}

impl Entity for StateEntryItem {
    const KIND: &'static str = "state_entry";

    fn key(&self) -> Key {
        keys::state_entry_key(
            self.tenant_id,
            self.task_id,
            self.skill_run_number,
            self.sequence,
        )
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .uuid("task_id", self.task_id)
            .uuid("skill_run_id", self.skill_run_id)
            .n("skill_run_number", i64::from(self.skill_run_number))
            .n("sequence", self.sequence)
            .s("state_name", &self.state_name)
            .s("state_type", &self.state_type)
            .s("status", &self.status)
            .time("entered_at", self.entered_at)
            .opt_time("exited_at", self.exited_at)
            .json("input", &self.input)
            .opt_json("output", self.output.as_ref())
            .opt_s("error", self.error.as_ref())
            .opt_s("cause", self.cause.as_ref())
            .n("attempts", self.attempts)
            .opt_s("next_state", self.next_state.as_ref());
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            task_id: r.uuid("task_id")?,
            skill_run_id: r.uuid("skill_run_id")?,
            skill_run_number: r.i32("skill_run_number")?,
            sequence: r.n("sequence")?,
            state_name: r.s("state_name")?,
            state_type: r.s("state_type")?,
            status: r.s("status")?,
            entered_at: r.time("entered_at")?,
            exited_at: r.opt_time("exited_at")?,
            input: r.json("input")?,
            output: r.opt_json("output")?,
            error: r.opt_s("error")?,
            cause: r.opt_s("cause")?,
            attempts: r.n("attempts")?,
            next_state: r.opt_s("next_state")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub skill_run_id: Uuid,
    pub skill_run_number: i32,
    pub kind: String,
    pub repository: String,
    pub repository_id: i64,
    pub number: i64,
    pub url: String,
    pub state: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Entity for Output {
    const KIND: &'static str = "output";

    fn key(&self) -> Key {
        keys::output_key(
            self.tenant_id,
            self.task_id,
            self.skill_run_number,
            &self.kind,
            self.repository_id,
            self.number,
        )
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("task_id", self.task_id)
            .uuid("skill_run_id", self.skill_run_id)
            .n("skill_run_number", i64::from(self.skill_run_number))
            .s("output_kind", &self.kind)
            .s("repository", &self.repository)
            .n("repository_id", self.repository_id)
            .n("number", self.number)
            .s("url", &self.url)
            .opt_s("state", self.state.as_ref())
            .time("created_at", self.created_at)
            .time("updated_at", self.updated_at)
            .s("gsi1pk", keys::outputs_index(self.tenant_id))
            .s("gsi1sk", keys::time_id(self.created_at, self.id));
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            task_id: r.uuid("task_id")?,
            skill_run_id: r.uuid("skill_run_id")?,
            skill_run_number: r.i32("skill_run_number")?,
            kind: r.s("output_kind")?,
            repository: r.s("repository")?,
            repository_id: r.n("repository_id")?,
            number: r.n("number")?,
            url: r.s("url")?,
            state: r.opt_s("state")?,
            created_at: r.time("created_at")?,
            updated_at: r.time("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub skill_run_id: Uuid,
    pub skill_run_number: i32,
    pub idempotency_key: String,
    pub resource: String,
    pub invocation: Value,
    pub status: String,
    pub result: Option<Value>,
    pub attempts: i64,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl Entity for Invocation {
    const KIND: &'static str = "resource_invocation";

    fn key(&self) -> Key {
        keys::invocation_key(
            self.tenant_id,
            self.task_id,
            self.skill_run_number,
            &self.idempotency_key,
        )
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("task_id", self.task_id)
            .uuid("skill_run_id", self.skill_run_id)
            .n("skill_run_number", i64::from(self.skill_run_number))
            .s("idempotency_key", &self.idempotency_key)
            .s("resource", &self.resource)
            .json("invocation", &self.invocation)
            .s("status", &self.status)
            .opt_json("result", self.result.as_ref())
            .n("attempts", self.attempts)
            .time("created_at", self.created_at)
            .opt_time("completed_at", self.completed_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            task_id: r.uuid("task_id")?,
            skill_run_id: r.uuid("skill_run_id")?,
            skill_run_number: r.i32("skill_run_number")?,
            idempotency_key: r.s("idempotency_key")?,
            resource: r.s("resource")?,
            invocation: r.json("invocation")?,
            status: r.s("status")?,
            result: r.opt_json("result")?,
            attempts: r.n("attempts")?,
            created_at: r.time("created_at")?,
            completed_at: r.opt_time("completed_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckWait {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub skill_run_id: Uuid,
    pub skill_run_number: i32,
    pub task_token: String,
    pub repos: Vec<String>,
    pub head_shas: Value,
    pub status: String,
    pub next_poll_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl Entity for CheckWait {
    const KIND: &'static str = "check_wait";

    fn key(&self) -> Key {
        keys::check_wait_key(
            self.tenant_id,
            self.task_id,
            self.skill_run_number,
            &self.task_token,
        )
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("task_id", self.task_id)
            .uuid("skill_run_id", self.skill_run_id)
            .n("skill_run_number", i64::from(self.skill_run_number))
            .s("task_token", &self.task_token)
            .strs("repos", &self.repos)
            .json("head_shas", &self.head_shas)
            .s("status", &self.status)
            .time("next_poll_at", self.next_poll_at)
            .time("created_at", self.created_at)
            .opt_time("completed_at", self.completed_at);
        if self.status == "waiting" {
            w.s("gsi2pk", keys::SWEEP_CHECKS).s(
                "gsi2sk",
                format!(
                    "{}#{}#{}",
                    format_time(self.next_poll_at),
                    self.tenant_id,
                    self.id
                ),
            );
        }
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            task_id: r.uuid("task_id")?,
            skill_run_id: r.uuid("skill_run_id")?,
            skill_run_number: r.i32("skill_run_number")?,
            task_token: r.s("task_token")?,
            repos: r.strs("repos")?,
            head_shas: r.json("head_shas")?,
            status: r.s("status")?,
            next_poll_at: r.time("next_poll_at")?,
            created_at: r.time("created_at")?,
            completed_at: r.opt_time("completed_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckWaitMarker {
    pub tenant_id: Uuid,
    pub wait_id: Uuid,
    pub task_id: Uuid,
    pub repos: Vec<String>,
    pub repository_ids: Vec<i64>,
}

impl Entity for CheckWaitMarker {
    const KIND: &'static str = "check_wait_marker";

    fn key(&self) -> Key {
        keys::check_wait_marker_key(self.tenant_id, self.wait_id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .uuid("wait_id", self.wait_id)
            .uuid("task_id", self.task_id)
            .strs("repos", &self.repos)
            .json("repository_ids", &serde_json::json!(self.repository_ids));
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            wait_id: r.uuid("wait_id")?,
            task_id: r.uuid("task_id")?,
            repos: r.strs("repos")?,
            repository_ids: serde_json::from_value(r.json("repository_ids")?)
                .map_err(|e| DbError::Decode(format!("check wait marker repository_ids: {e}")))?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalWait {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub skill_run_id: Uuid,
    pub skill_run_number: i32,
    pub task_token: String,
    pub message: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

impl Entity for ApprovalWait {
    const KIND: &'static str = "approval_wait";

    fn key(&self) -> Key {
        keys::approval_wait_key(
            self.tenant_id,
            self.task_id,
            self.skill_run_number,
            &self.task_token,
        )
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("task_id", self.task_id)
            .uuid("skill_run_id", self.skill_run_id)
            .n("skill_run_number", i64::from(self.skill_run_number))
            .s("task_token", &self.task_token)
            .s("message", &self.message)
            .s("status", &self.status)
            .time("created_at", self.created_at)
            .opt_time("resolved_at", self.resolved_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            task_id: r.uuid("task_id")?,
            skill_run_id: r.uuid("skill_run_id")?,
            skill_run_number: r.i32("skill_run_number")?,
            task_token: r.s("task_token")?,
            message: r.s("message")?,
            status: r.s("status")?,
            created_at: r.time("created_at")?,
            resolved_at: r.opt_time("resolved_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimerItem {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub skill_run_id: Uuid,
    pub skill_run_number: i32,
    pub sequence: i64,
    pub timer: Value,
    pub due_at: DateTime<Utc>,
    pub message_id: Uuid,
}

impl Entity for TimerItem {
    const KIND: &'static str = "engine_timer";

    fn key(&self) -> Key {
        keys::timer_key(self.tenant_id, self.task_id, self.skill_run_number, self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("task_id", self.task_id)
            .uuid("skill_run_id", self.skill_run_id)
            .n("skill_run_number", i64::from(self.skill_run_number))
            .n("sequence", self.sequence)
            .json("timer", &self.timer)
            .time("due_at", self.due_at)
            .uuid("message_id", self.message_id);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            task_id: r.uuid("task_id")?,
            skill_run_id: r.uuid("skill_run_id")?,
            skill_run_number: r.i32("skill_run_number")?,
            sequence: r.n("sequence")?,
            timer: r.json("timer")?,
            due_at: r.time("due_at")?,
            message_id: r.uuid("message_id")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeldMessage {
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub dedupe_key: String,
    pub text: String,
}

impl Entity for HeldMessage {
    const KIND: &'static str = "held_message";

    fn key(&self) -> Key {
        keys::held_message_key(
            self.tenant_id,
            self.task_id,
            self.created_at,
            &self.dedupe_key,
        )
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .uuid("task_id", self.task_id)
            .time("created_at", self.created_at)
            .s("dedupe_key", &self.dedupe_key)
            .s("text", &self.text);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            task_id: r.uuid("task_id")?,
            created_at: r.time("created_at")?,
            dedupe_key: r.s("dedupe_key")?,
            text: r.s("text")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageRecord {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub cost_usd: f64,
    pub recorded_at: DateTime<Utc>,
}

impl Entity for UsageRecord {
    const KIND: &'static str = "usage";

    fn key(&self) -> Key {
        keys::usage_key(self.tenant_id, self.task_id, self.recorded_at, self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("task_id", self.task_id)
            .uuid("run_id", self.run_id)
            .n("input_tokens", self.input_tokens)
            .n("output_tokens", self.output_tokens)
            .n("cache_read_tokens", self.cache_read_tokens)
            .n("cache_write_tokens", self.cache_write_tokens)
            .f("cost_usd", self.cost_usd)
            .time("recorded_at", self.recorded_at)
            .s("gsi1pk", keys::usage_index(self.tenant_id))
            .s("gsi1sk", keys::time_id(self.recorded_at, self.id));
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            task_id: r.uuid("task_id")?,
            run_id: r.uuid("run_id")?,
            input_tokens: r.n("input_tokens")?,
            output_tokens: r.n("output_tokens")?,
            cache_read_tokens: r.n("cache_read_tokens")?,
            cache_write_tokens: r.n("cache_write_tokens")?,
            cost_usd: r.f("cost_usd")?,
            recorded_at: r.time("recorded_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageAlert {
    pub tenant_id: Uuid,
    pub period_start: DateTime<Utc>,
    pub metric: String,
    pub threshold_percent: i64,
    pub used: i64,
    pub limit_value: i64,
    pub created_at: DateTime<Utc>,
}

impl Entity for UsageAlert {
    const KIND: &'static str = "usage_alert";

    fn key(&self) -> Key {
        keys::usage_alert_key(
            self.tenant_id,
            self.period_start,
            &self.metric,
            self.threshold_percent,
        )
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .time("period_start", self.period_start)
            .s("metric", &self.metric)
            .n("threshold_percent", self.threshold_percent)
            .n("used", self.used)
            .n("limit_value", self.limit_value)
            .time("created_at", self.created_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            period_start: r.time("period_start")?,
            metric: r.s("metric")?,
            threshold_percent: r.n("threshold_percent")?,
            used: r.n("used")?,
            limit_value: r.n("limit_value")?,
            created_at: r.time("created_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InboundEvent {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub source: String,
    pub key: String,
    pub event: Value,
    pub received_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

impl Entity for InboundEvent {
    const KIND: &'static str = "inbound_event";

    fn key(&self) -> Key {
        keys::inbound_event_key(self.tenant_id, &self.source, &self.key)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .s("source", &self.source)
            .s("key", &self.key)
            .json("event", &self.event)
            .time("received_at", self.received_at)
            .opt_time("processed_at", self.processed_at)
            .opt_s("error", self.error.as_ref());
        if let Some(thread) = self.event["detail"]["thread"].as_str() {
            w.s(
                "gsi1pk",
                keys::thread_index(self.tenant_id, &self.source, thread),
            )
            .s("gsi1sk", keys::time_id(self.received_at, self.id));
        }
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            source: r.s("source")?,
            key: r.s("key")?,
            event: r.json("event")?,
            received_at: r.time("received_at")?,
            processed_at: r.opt_time("processed_at")?,
            error: r.opt_s("error")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub key: Key,
    pub target: Key,
    pub id: Uuid,
    pub received_at: DateTime<Utc>,
}

impl Entity for Pending {
    const KIND: &'static str = "pending";

    fn key(&self) -> Key {
        self.key.clone()
    }

    fn write(&self, w: &mut ItemWriter) {
        w.s("target_pk", &self.target.pk)
            .s("target_sk", &self.target.sk)
            .uuid("id", self.id)
            .time("received_at", self.received_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            key: Key::main(r.s("pk")?, r.s("sk")?),
            target: Key::main(r.s("target_pk")?, r.s("target_sk")?),
            id: r.uuid("id")?,
            received_at: r.time("received_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunChannel {
    pub tenant_id: Uuid,
    pub run_id: Uuid,
    pub task_id: Uuid,
    pub next_outbox_seq: i64,
    pub instances: i64,
    pub connection_id: Option<String>,
    pub connection_instance: Option<Uuid>,
    pub connection_started: bool,
}

impl Entity for RunChannel {
    const KIND: &'static str = "run_channel";

    fn key(&self) -> Key {
        keys::run_channel_key(self.tenant_id, self.run_id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .uuid("run_id", self.run_id)
            .uuid("task_id", self.task_id)
            .n("next_outbox_seq", self.next_outbox_seq)
            .n("instances", self.instances)
            .opt_s("connection_id", self.connection_id.as_ref())
            .opt_uuid("connection_instance", self.connection_instance)
            .b("connection_started", self.connection_started);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            run_id: r.uuid("run_id")?,
            task_id: r.uuid("task_id")?,
            next_outbox_seq: r.n("next_outbox_seq")?,
            instances: r.n("instances")?,
            connection_id: r.opt_s("connection_id")?,
            connection_instance: r.opt_uuid("connection_instance")?,
            connection_started: r.b("connection_started")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelInstance {
    pub tenant_id: Uuid,
    pub run_id: Uuid,
    pub instance_id: Uuid,
    pub seq_offset: i64,
    pub processed_seq: i64,
    pub start_outbox_seq: Option<i64>,
    pub connected_at: DateTime<Utc>,
}

impl Entity for ChannelInstance {
    const KIND: &'static str = "channel_instance";

    fn key(&self) -> Key {
        keys::channel_instance_key(self.tenant_id, self.run_id, self.instance_id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .uuid("run_id", self.run_id)
            .uuid("instance_id", self.instance_id)
            .n("seq_offset", self.seq_offset)
            .n("processed_seq", self.processed_seq)
            .opt_n("start_outbox_seq", self.start_outbox_seq)
            .time("connected_at", self.connected_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            run_id: r.uuid("run_id")?,
            instance_id: r.uuid("instance_id")?,
            seq_offset: r.n("seq_offset")?,
            processed_seq: r.n("processed_seq")?,
            start_outbox_seq: r.opt_n("start_outbox_seq")?,
            connected_at: r.time("connected_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunLogEntry {
    pub tenant_id: Uuid,
    pub run_id: Uuid,
    pub seq: i64,
    pub turn: i64,
    pub kind: String,
    pub data: Value,
    pub at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl Entity for RunLogEntry {
    const KIND: &'static str = "run_log_entry";

    fn key(&self) -> Key {
        keys::run_log_key(self.tenant_id, self.run_id, self.seq)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .uuid("run_id", self.run_id)
            .n("seq", self.seq)
            .n("turn", self.turn)
            .s("log_kind", &self.kind)
            .json("data", &self.data)
            .time("at", self.at)
            .ttl(self.expires_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            run_id: r.uuid("run_id")?,
            seq: r.n("seq")?,
            turn: r.n("turn")?,
            kind: r.s("log_kind")?,
            data: r.json("data")?,
            at: r.time("at")?,
            expires_at: DateTime::from_timestamp(r.n(crate::item::TTL)?, 0)
                .ok_or_else(|| DbError::Decode("run log expires_at".into()))?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReceiveRate {
    pub tenant_id: Uuid,
    pub window_start: DateTime<Utc>,
    pub received: i64,
    pub dropped: i64,
    pub expires_at: DateTime<Utc>,
}

impl Entity for ReceiveRate {
    const KIND: &'static str = "receive_rate";

    fn key(&self) -> Key {
        keys::receive_rate_key(self.tenant_id, self.window_start)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .time("window_start", self.window_start)
            .n("received", self.received)
            .n("dropped", self.dropped)
            .ttl(self.expires_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            window_start: r.time("window_start")?,
            received: r.n("received")?,
            dropped: r.n("dropped")?,
            expires_at: DateTime::from_timestamp(r.n(crate::item::TTL)?, 0)
                .ok_or_else(|| DbError::Decode("receive rate expires_at".into()))?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerRate {
    pub tenant_id: Uuid,
    pub trigger_id: Uuid,
    pub window_start: DateTime<Utc>,
    pub starts: i64,
    pub discarded: i64,
    pub expires_at: DateTime<Utc>,
}

impl Entity for TriggerRate {
    const KIND: &'static str = "trigger_rate";

    fn key(&self) -> Key {
        keys::trigger_rate_key(self.tenant_id, self.trigger_id, self.window_start)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("tenant_id", self.tenant_id)
            .uuid("trigger_id", self.trigger_id)
            .time("window_start", self.window_start)
            .n("starts", self.starts)
            .n("discarded", self.discarded)
            .ttl(self.expires_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tenant_id: r.uuid("tenant_id")?,
            trigger_id: r.uuid("trigger_id")?,
            window_start: r.time("window_start")?,
            starts: r.n("starts")?,
            discarded: r.n("discarded")?,
            expires_at: DateTime::from_timestamp(r.n(crate::item::TTL)?, 0)
                .ok_or_else(|| DbError::Decode("trigger rate expires_at".into()))?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Option<Uuid>,
    pub outbox_id: Option<Uuid>,
    pub channel: Option<Value>,
    pub author_kind: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

impl Entity for ChatMessage {
    const KIND: &'static str = "chat_message";

    fn key(&self) -> Key {
        keys::chat_message_key(self.tenant_id, self.task_id, self.created_at, self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("task_id", self.task_id)
            .opt_uuid("run_id", self.run_id)
            .opt_uuid("outbox_id", self.outbox_id)
            .opt_json("channel", self.channel.as_ref())
            .s("author_kind", &self.author_kind)
            .s("text", &self.text)
            .time("created_at", self.created_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            task_id: r.uuid("task_id")?,
            run_id: r.opt_uuid("run_id")?,
            outbox_id: r.opt_uuid("outbox_id")?,
            channel: r.opt_json("channel")?,
            author_kind: r.s("author_kind")?,
            text: r.s("text")?,
            created_at: r.time("created_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunEventItem {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub run_id: Uuid,
    pub seq: Option<i64>,
    pub kind: String,
    pub payload: Value,
    pub received_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

impl Entity for RunEventItem {
    const KIND: &'static str = "run_event";

    fn key(&self) -> Key {
        keys::run_event_key(
            self.tenant_id,
            self.run_id,
            self.seq,
            self.received_at,
            self.id,
        )
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("run_id", self.run_id)
            .opt_n("seq", self.seq)
            .s("type", &self.kind)
            .json("payload", &self.payload)
            .time("received_at", self.received_at)
            .opt_time("processed_at", self.processed_at)
            .opt_s("error", self.error.as_ref());
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            run_id: r.uuid("run_id")?,
            seq: r.opt_n("seq")?,
            kind: r.s("type")?,
            payload: r.json("payload")?,
            received_at: r.time("received_at")?,
            processed_at: r.opt_time("processed_at")?,
            error: r.opt_s("error")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutboxItem {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub run_id: Uuid,
    pub seq: i64,
    pub kind: String,
    pub payload: Value,
    pub dedupe_key: String,
    pub created_at: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
}

impl Entity for OutboxItem {
    const KIND: &'static str = "outbox";

    fn key(&self) -> Key {
        keys::outbox_key(self.tenant_id, self.run_id, self.seq)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .uuid("run_id", self.run_id)
            .n("seq", self.seq)
            .s("type", &self.kind)
            .json("payload", &self.payload)
            .s("dedupe_key", &self.dedupe_key)
            .time("created_at", self.created_at)
            .opt_time("delivered_at", self.delivered_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            run_id: r.uuid("run_id")?,
            seq: r.n("seq")?,
            kind: r.s("type")?,
            payload: r.json("payload")?,
            dedupe_key: r.s("dedupe_key")?,
            created_at: r.time("created_at")?,
            delivered_at: r.opt_time("delivered_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditEvent {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub at: DateTime<Utc>,
    pub actor_kind: String,
    pub actor_ref: String,
    pub action: String,
    pub target_kind: String,
    pub target_ref: String,
    pub summary: String,
    pub source_ip: Option<String>,
    pub metadata: Value,
    pub expires_at: DateTime<Utc>,
}

impl Entity for AuditEvent {
    const KIND: &'static str = "audit_event";

    fn key(&self) -> Key {
        keys::audit_key(self.tenant_id, self.at, self.id)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .uuid("tenant_id", self.tenant_id)
            .time("at", self.at)
            .s("actor_kind", &self.actor_kind)
            .s("actor_ref", &self.actor_ref)
            .s("action", &self.action)
            .s("target_kind", &self.target_kind)
            .s("target_ref", &self.target_ref)
            .s("summary", &self.summary)
            .opt_s("source_ip", self.source_ip.as_ref())
            .json("metadata", &self.metadata)
            .ttl(self.expires_at);
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            tenant_id: r.uuid("tenant_id")?,
            at: r.time("at")?,
            actor_kind: r.s("actor_kind")?,
            actor_ref: r.s("actor_ref")?,
            action: r.s("action")?,
            target_kind: r.s("target_kind")?,
            target_ref: r.s("target_ref")?,
            summary: r.s("summary")?,
            source_ip: r.opt_s("source_ip")?,
            metadata: r.json("metadata")?,
            expires_at: DateTime::from_timestamp(r.n(crate::item::TTL)?, 0)
                .ok_or_else(|| DbError::Decode("audit expires_at".into()))?,
        })
    }
}
