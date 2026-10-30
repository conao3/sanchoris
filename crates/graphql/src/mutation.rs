use std::collections::BTreeMap;

use async_graphql::{Context, Error, ID, InputObject, Object, Result, SimpleObject};
use aws_sdk_dynamodb::types::AttributeValue;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::Duration;
use sanchoris_core::billing::{ADMIN_PLAN, FREE_PLAN, PRIVILEGED_EMAIL_DOMAIN, stage_plan_change};
use sanchoris_core::channel::{RunEventType, record_inbound_event, record_run_event};
use sanchoris_core::db::{self, REF_RUN, REF_TRIGGER, TenantTx};
use sanchoris_core::item::format_time;
use sanchoris_core::items::{
    self, CONFIG_ACTIVE, Guard, InboundEvent, InvitationEmail, Membership as MembershipItem,
    Task as TaskItem, Trigger as TriggerItem, TriggerEvent as EventItem, User, uniq,
};
use sanchoris_core::jobs::{Deletion, Job};
use sanchoris_core::keys;
use sanchoris_core::model::{AuditOrigin, TaskStatus as CoreStatus};
use sanchoris_core::receive::{Admission, SOURCE_CHAT, SOURCE_REPLY, admit_inbound};
use sanchoris_core::skillrun::{SkillRunError, resolve_waiting_approval};
use sanchoris_core::store::{Index, SortKey, Update};
use sanchoris_core::tasks::{abort_task, latest_run, load_task, snoozable, snooze, stop_run};
use sanchoris_core::tenants::{
    NewTenant, TENANT_DELETING, TenantError, create_tenant, hold_free_tenant, is_deleting,
    pending_checkout, release_free_tenant,
};
use sanchoris_worker::dispatcher::schedule_trigger;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::account::{
    Credential, Invitation, Member, Plan, Viewer, key_matches_provider, mask_key,
};
use crate::audit::{AuditEvent, AuditFilter};
use crate::config::{ChangeRequest, Config, Session, ValidationError};
use crate::context::{
    Access, AuditTarget, ClientIp, Failure, Role, Scope, audit_member, audit_operator, auth,
    being_deleted, is_operator, locate_item, locate_ref, member_scope, membership_in, memberships,
    now, scope_for_tenant, serialize_tenant, state, store, tx, user_tx, viewer_user, write,
};
use crate::enums::{ApiKeyProvider, CredentialScope, EventSource, InvitationState, TriggerState};
use crate::error::{bad_input, conflict, forbidden, not_found, parse_id, port, rate_limited};
use crate::ports::{ConfigCheck, ConfigLocation, PlanChange, TenantCheckout, TriggerProposal};
use crate::scalars::Timestamp;
use crate::task::{ChatMessage, Run, Task, waiting_approval};
use crate::tenant::Tenant;
use crate::trigger::{TRIGGER_TEMPLATES, Trigger};

const INVITATION_DAYS: i64 = 7;
const DELETION_DELAY_SECONDS: i64 = 60;

#[derive(SimpleObject)]
pub(crate) struct RedirectPayload {
    url: String,
}

#[derive(InputObject)]
pub(crate) struct CreateTenantInput {
    name: String,
    #[graphql(name = "planId")]
    plan_id: ID,
}

#[derive(SimpleObject)]
pub(crate) struct CreateTenantPayload {
    /// The new tenant. On a paid plan it stays SUSPENDED with the reason payment_pending until the checkout completes.
    tenant: Option<Tenant>,
    /// Set when the plan is paid: the Stripe Checkout to open.
    checkout_url: Option<String>,
}

#[derive(InputObject)]
pub(crate) struct InviteMemberInput {
    #[graphql(name = "tenantId")]
    tenant_id: ID,
    email: String,
    role: Role,
}

#[derive(InputObject)]
pub(crate) struct ResumeTaskInput {
    #[graphql(name = "taskId")]
    task_id: ID,
    message: String,
}

#[derive(InputObject)]
pub(crate) struct SendChatMessageInput {
    #[graphql(name = "tenantId")]
    tenant_id: ID,
    #[graphql(name = "taskId")]
    task_id: Option<ID>,
    /// all or a project name. Required when taskId is not given.
    scope: Option<String>,
    text: String,
}

#[derive(SimpleObject)]
pub(crate) struct SendChatMessagePayload {
    task: Task,
    message: ChatMessage,
}

#[derive(InputObject)]
pub(crate) struct ProposeTriggerInput {
    #[graphql(name = "tenantId")]
    tenant_id: ID,
    #[graphql(name = "configId")]
    config_id: ID,
    #[graphql(name = "templateId")]
    template_id: ID,
    repository: String,
}

#[derive(InputObject)]
pub(crate) struct AddConfigInput {
    #[graphql(name = "tenantId")]
    tenant_id: ID,
    repository: String,
    root: String,
    branch: String,
}

#[derive(SimpleObject)]
pub(crate) struct AddConfigPayload {
    config: Option<Config>,
    errors: Vec<ValidationError>,
}

#[derive(InputObject)]
pub(crate) struct AddCredentialInput {
    #[graphql(name = "tenantId")]
    tenant_id: ID,
    name: String,
    provider: ApiKeyProvider,
    key: String,
    scope: CredentialScope,
}

#[derive(SimpleObject)]
pub(crate) struct ChangePlanPayload {
    tenant: Tenant,
    checkout_url: Option<String>,
}

pub struct MutationRoot;

fn target(kind: &str, id: Uuid, name: impl Into<String>) -> AuditTarget<'_> {
    AuditTarget {
        kind,
        id: id.to_string(),
        name: name.into(),
    }
}

fn is_repository(name: &str) -> bool {
    let Some((owner, repo)) = name.split_once('/') else {
        return false;
    };
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    valid(owner) && valid(repo)
}

fn s(value: &str) -> AttributeValue {
    AttributeValue::S(value.to_string())
}

async fn member_name(ctx: &Context<'_>, tx: &mut TenantTx, member_id: Uuid) -> Result<String> {
    let member = tx
        .require::<items::Member>(&keys::member_key(tx.tenant_id(), member_id))
        .await?;
    Ok(store(ctx)
        .get::<User>(&keys::user_key(member.user_id))
        .await?
        .ok_or_else(|| not_found(format!("user {}", member.user_id)))?
        .display_name)
}

async fn admit(ctx: &Context<'_>, tenant_id: Uuid, source: &str, key: &str) -> Result<()> {
    match admit_inbound(store(ctx), tenant_id, source, key, now(ctx)).await? {
        Admission::Admitted => Ok(()),
        Admission::Dropped(limited) => Err(rate_limited(&limited)),
    }
}

async fn tenant_scope(ctx: &Context<'_>, tenant_id: &ID) -> Result<Scope> {
    member_scope(ctx, parse_id(tenant_id)?).await
}

async fn audit_user_tenants(ctx: &Context<'_>, action: &str, summary: &str) -> Result<()> {
    let user = auth(ctx)?;
    for membership in memberships(ctx).await? {
        write(|| async {
            let mut tx = tx(ctx, membership.tenant_id);
            audit_member(
                ctx,
                &mut tx,
                membership.member_id,
                action,
                target("user", user.user_id, user.email.clone()),
                summary,
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
    }
    Ok(())
}

async fn owner_count(tx: &mut TenantTx) -> Result<usize> {
    Ok(tx
        .query::<items::Member>(&keys::tenant_pk(tx.tenant_id()), "MEMBER#")
        .await?
        .iter()
        .filter(|member| member.role == "owner")
        .count())
}

async fn suspend_or_resume(
    ctx: &Context<'_>,
    tenant_id: &ID,
    from: &str,
    update: Update,
    action: &str,
    summary: &str,
) -> Result<Tenant> {
    let tenant_id = parse_id(tenant_id)?;
    let to = if from == "active" {
        "suspended"
    } else {
        "active"
    };
    write(|| async {
        let mut tx = tx(ctx, tenant_id);
        let tenant = tx
            .get::<items::Tenant>(&keys::tenant_key(tenant_id))
            .await?
            .filter(|tenant| tenant.status == from)
            .ok_or_else(|| conflict(format!("the tenant does not exist or is not {from}")))?;
        tx.update(
            &keys::tenant_key(tenant_id),
            update
                .clone()
                .set("status", s(to))
                .set("updated_at", s(&format_time(now(ctx))))
                .require("status", s(from)),
        )?;
        audit_operator(
            ctx,
            &mut tx,
            &format!("tenant.{action}"),
            target("tenant", tenant_id, tenant.name.clone()),
            summary,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    })
    .await?;
    Tenant::load(ctx, scope_for_tenant(ctx, tenant_id).await?).await
}

fn worker_error(error: impl std::fmt::Display) -> Error {
    Error::new(error.to_string())
}

#[Object(name = "Mutation")]
impl MutationRoot {
    /// Creates a tenant owned by the viewer on a plan of Viewer.newTenantPlans. A paid plan returns a Stripe Checkout URL and creates the tenant after payment; Free and Admin return the tenant.
    async fn create_tenant(
        &self,
        ctx: &Context<'_>,
        input: CreateTenantInput,
    ) -> Result<CreateTenantPayload> {
        let user = auth(ctx)?;
        let name = input.name.trim();
        if name.is_empty() {
            return Err(bad_input("the tenant needs a name"));
        }
        let plan = Plan::load(ctx, input.plan_id.as_str()).await?;
        match (user.privileged, plan.is_admin()) {
            (true, false) => {
                return Err(bad_input(format!(
                    "a verified @{PRIVILEGED_EMAIL_DOMAIN} email creates tenants on the {ADMIN_PLAN} plan"
                )));
            }
            (false, true) => {
                return Err(forbidden(format!(
                    "the {ADMIN_PLAN} plan needs a verified @{PRIVILEGED_EMAIL_DOMAIN} email"
                )));
            }
            _ => {}
        }
        if plan.0.price_cents > 0 {
            let url = state(ctx)
                .deps
                .checkout
                .tenant_checkout(&TenantCheckout {
                    user_id: user.user_id,
                    email: user.email.clone(),
                    tenant_name: name.to_string(),
                    plan_id: plan.0.id.clone(),
                })
                .await
                .map_err(port)?;
            let pending = pending_checkout(store(ctx), user.user_id)
                .await?
                .ok_or_else(|| Error::new("the checkout left no pending tenant"))?;
            let tenant = Tenant::load(ctx, member_scope(ctx, pending.tenant_id).await?).await?;
            return Ok(CreateTenantPayload {
                tenant: Some(tenant),
                checkout_url: Some(url),
            });
        }
        let created = create_tenant(
            store(ctx),
            &NewTenant {
                id: Uuid::new_v4(),
                name,
                plan_id: &plan.0.id,
                user_id: user.user_id,
                checkout: None,
                origin: AuditOrigin::Web,
                source_ip: ctx.data_opt::<ClientIp>().map(|ip| ip.0),
                now: now(ctx),
            },
        )
        .await
        .map_err(|error| match error {
            TenantError::Db(error) => Error::from(error),
            TenantError::FreeTenantTaken | TenantError::CheckoutPending(_) => {
                conflict(error.to_string())
            }
        })?;
        let tenant = Tenant::load(ctx, member_scope(ctx, created.tenant_id).await?).await?;
        Ok(CreateTenantPayload {
            tenant: Some(tenant),
            checkout_url: None,
        })
    }

    /// Accepts an invitation addressed to the viewer and returns the new membership.
    async fn accept_invitation(&self, ctx: &Context<'_>, invitation_id: ID) -> Result<Member> {
        let invitation_id = parse_id(&invitation_id)?;
        let user = auth(ctx)?;
        let entry = store(ctx)
            .query::<InvitationEmail>(&keys::invitation_email_pk(&user.email), "INVITE#")
            .await?
            .into_iter()
            .find(|entry| entry.invitation_id == invitation_id)
            .ok_or_else(|| not_found(format!("invitation {invitation_id}")))?;
        let tenant_id = entry.tenant_id;
        if membership_in(ctx, tenant_id).await?.is_some() {
            return Err(conflict("the viewer is already a member of the tenant"));
        }
        let invitee = Scope {
            tenant_id,
            access: Access::Invitee,
        };
        let invitation = Invitation::load(ctx, invitee, invitation_id).await?;
        if invitation.state_at(now(ctx))? != InvitationState::Pending {
            return Err(conflict("the invitation is no longer pending"));
        }
        if !invitation.row.email.eq_ignore_ascii_case(&user.email) {
            return Err(forbidden("the invitation is addressed to another email"));
        }
        let at = now(ctx);
        let member_id = write(|| async {
            let mut tx = tx(ctx, tenant_id);
            let tenant = tx
                .get::<items::Tenant>(&keys::tenant_key(tenant_id))
                .await?
                .ok_or_else(|| not_found(format!("tenant {tenant_id}")))?;
            if is_deleting(&tenant) {
                return Err(being_deleted(&tenant).into());
            }
            if tx
                .get::<MembershipItem>(&keys::membership_key(user.user_id, tenant_id))
                .await?
                .is_some()
            {
                return Err(conflict("the viewer is already a member of the tenant").into());
            }
            let mut invitation = tx
                .require::<items::Invitation>(&keys::invitation_key(tenant_id, invitation_id))
                .await?;
            if !invitation.is_pending_at(at) {
                return Err(conflict("the invitation is no longer pending").into());
            }
            invitation.status = "accepted".into();
            invitation.accepted_at = Some(at);
            tx.put(&invitation)?;
            tx.delete(&keys::pending_invitation_key(tenant_id, &invitation.email))?;
            let member_id = Uuid::new_v4();
            tx.insert(&items::Member {
                id: member_id,
                tenant_id,
                user_id: user.user_id,
                role: invitation.role.clone(),
                joined_at: at,
                last_active_at: Some(at),
            })?;
            tx.insert(&MembershipItem {
                user_id: user.user_id,
                tenant_id,
                member_id,
            })?;
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "member.invitation_accept",
                target("invitation", invitation_id, invitation.email.clone()),
                &format!("Accepted the invitation as {}", invitation.role),
            )
            .await?;
            tx.commit().await?;
            Ok(member_id)
        })
        .await?;
        Member::load(ctx, member_scope(ctx, tenant_id).await?, member_id).await
    }

    /// Starts linking the viewer's GitHub account. Returns the GitHub authorize URL to open and sets the cookie that binds the flow to this browser.
    #[graphql(name = "connectGitHub")]
    async fn connect_github(&self, ctx: &Context<'_>) -> Result<RedirectPayload> {
        let user = auth(ctx)?;
        audit_user_tenants(
            ctx,
            "integration.github_link_start",
            "Started linking a GitHub account",
        )
        .await?;
        let redirect = state(ctx).deps.github.link_account(user.user_id);
        ctx.append_http_header("Set-Cookie", redirect.cookie);
        Ok(RedirectPayload { url: redirect.url })
    }

    /// Starts installing the GitHub App for a tenant (admins). Returns the GitHub install URL to open and sets the cookie that binds the flow to this browser.
    #[graphql(name = "installGitHubApp")]
    async fn install_github_app(
        &self,
        ctx: &Context<'_>,
        tenant_id: ID,
    ) -> Result<RedirectPayload> {
        let user = auth(ctx)?;
        let scope = tenant_scope(ctx, &tenant_id).await?;
        let member_id = scope.require(Role::Admin, "installing the GitHub App")?;
        let redirect = state(ctx)
            .deps
            .github
            .install_app(scope.tenant_id, user.user_id);
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "integration.github_install_start",
                target("tenant", scope.tenant_id, "github_installation"),
                "Started the GitHub App installation",
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        ctx.append_http_header("Set-Cookie", redirect.cookie);
        Ok(RedirectPayload { url: redirect.url })
    }

    /// Unlinks the viewer's GitHub account.
    #[graphql(name = "unlinkGitHub")]
    async fn unlink_github(&self, ctx: &Context<'_>) -> Result<Viewer> {
        let login = viewer_user(ctx)
            .await?
            .github_login
            .ok_or_else(|| conflict("no GitHub account is linked"))?;
        let user_id = auth(ctx)?.user_id;
        write(|| async {
            let mut tx = user_tx(ctx);
            let mut user = tx.require::<User>(&keys::user_key(user_id)).await?;
            let Some(login) = user.github_login.take() else {
                return Err(conflict("no GitHub account is linked").into());
            };
            if let Some(github_user) = user.github_user_id.take() {
                tx.delete(&keys::uniq_key(uniq::GITHUB_USER, &github_user.to_string()))?;
            }
            tx.delete(&keys::uniq_key(uniq::GITHUB_LOGIN, &login.to_lowercase()))?;
            user.updated_at = now(ctx);
            tx.put(&user)?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        audit_user_tenants(
            ctx,
            "integration.github_unlink",
            &format!("Unlinked the GitHub account {login}"),
        )
        .await?;
        Ok(Viewer)
    }

    /// Records the viewer's sign-out in the audit log of each of the viewer's tenants. The client then ends the Cognito session.
    async fn sign_out(&self, ctx: &Context<'_>) -> Result<bool> {
        audit_user_tenants(ctx, "auth.sign_out", "Signed out").await?;
        Ok(true)
    }

    /// Starts the Slack installation for a tenant. Returns the URL to open.
    async fn connect_slack(&self, ctx: &Context<'_>, tenant_id: ID) -> Result<RedirectPayload> {
        let scope = tenant_scope(ctx, &tenant_id).await?;
        let member_id = scope.require(Role::Admin, "connecting Slack")?;
        let url = state(ctx)
            .deps
            .slack
            .install(scope.tenant_id)
            .map_err(port)?;
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "integration.slack_connect_start",
                target("tenant", scope.tenant_id, "slack"),
                "Started the Slack installation",
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        Ok(RedirectPayload { url })
    }

    async fn invite_member(
        &self,
        ctx: &Context<'_>,
        input: InviteMemberInput,
    ) -> Result<Invitation> {
        let scope = tenant_scope(ctx, &input.tenant_id).await?;
        let member_id = scope.require(Role::Admin, "inviting members")?;
        if input.role != Role::Member {
            scope.require(Role::Owner, "inviting owners and admins")?;
        }
        let email = input.email.trim().to_lowercase();
        if !email.contains('@') {
            return Err(bad_input(format!("{email} is not an email address")));
        }
        let at = now(ctx);
        let tenant_id = scope.tenant_id;
        let email = &email;
        let invitation_id = write(|| async {
            let mut tx = tx(ctx, tenant_id);
            serialize_tenant(&mut tx).await?;
            let (_, plan) = db::tenant_plan(&mut tx).await?;
            let members = tx
                .query::<items::Member>(&keys::tenant_pk(tenant_id), "MEMBER#")
                .await?;
            for member in &members {
                let user = store(ctx)
                    .get::<User>(&keys::user_key(member.user_id))
                    .await?
                    .ok_or_else(|| not_found(format!("user {}", member.user_id)))?;
                if user.email.to_lowercase() == *email {
                    return Err(conflict(format!("{email} is already a member")).into());
                }
            }
            let invitations = tx
                .query::<items::Invitation>(&keys::tenant_pk(tenant_id), "INVITE#")
                .await?;
            let pending = invitations
                .iter()
                .filter(|invitation| invitation.is_pending_at(at))
                .count();
            let seats = i64::try_from(members.len() + pending).map_err(worker_error)?;
            if let Some(limit) = plan.member_limit
                && seats >= limit
            {
                return Err(conflict(format!(
                    "the plan allows {limit} members including pending invitations"
                ))
                .into());
            }
            let guard_key = keys::pending_invitation_key(tenant_id, email);
            if let Some(guard) = tx.get::<Guard>(&guard_key).await?
                && let Some(mut previous) = invitations
                    .into_iter()
                    .find(|invitation| invitation.id == guard.target)
                && previous.status == "pending"
            {
                if previous.is_pending_at(at) {
                    return Err(
                        conflict(format!("{email} already has a pending invitation")).into(),
                    );
                }
                previous.status = "expired".into();
                tx.put(&previous)?;
            }
            let invitation_id = Uuid::new_v4();
            tx.insert(&items::Invitation {
                id: invitation_id,
                tenant_id,
                email: email.clone(),
                role: input.role.as_db().to_string(),
                status: "pending".into(),
                invited_by_member_id: member_id,
                expires_at: at + Duration::days(INVITATION_DAYS),
                accepted_at: None,
                created_at: at,
            })?;
            tx.put(&Guard::new(guard_key, invitation_id))?;
            tx.insert(&InvitationEmail {
                email: email.clone(),
                tenant_id,
                invitation_id,
            })?;
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "member.invite",
                target("invitation", invitation_id, email.clone()),
                &format!("Invited {email} as {}", input.role.as_db()),
            )
            .await?;
            tx.commit().await?;
            Ok(invitation_id)
        })
        .await?;
        Invitation::load(ctx, scope, invitation_id).await
    }

    async fn revoke_invitation(&self, ctx: &Context<'_>, invitation_id: ID) -> Result<Invitation> {
        let invitation_id = parse_id(&invitation_id)?;
        let (scope, _) =
            locate_item::<items::Invitation>(ctx, "invitation", invitation_id, |tenant| {
                keys::invitation_key(tenant, invitation_id)
            })
            .await?;
        let member_id = scope.require(Role::Admin, "revoking invitations")?;
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            let mut invitation = tx
                .require::<items::Invitation>(&keys::invitation_key(scope.tenant_id, invitation_id))
                .await?;
            if invitation.status != "pending" {
                return Err(conflict("the invitation is no longer pending").into());
            }
            invitation.status = "revoked".into();
            tx.put(&invitation)?;
            let guard_key = keys::pending_invitation_key(scope.tenant_id, &invitation.email);
            if tx
                .get::<Guard>(&guard_key)
                .await?
                .is_some_and(|guard| guard.target == invitation_id)
            {
                tx.delete(&guard_key)?;
            }
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "member.invitation_revoke",
                target("invitation", invitation_id, invitation.email.clone()),
                &format!("Revoked the invitation of {}", invitation.email),
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        Invitation::load(ctx, scope, invitation_id).await
    }

    async fn change_member_role(
        &self,
        ctx: &Context<'_>,
        member_id: ID,
        role: Role,
    ) -> Result<Member> {
        let target_id = parse_id(&member_id)?;
        let (scope, _) = locate_item::<items::Member>(ctx, "member", target_id, |tenant| {
            keys::member_key(tenant, target_id)
        })
        .await?;
        let actor = scope.require(Role::Admin, "changing roles")?;
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            serialize_tenant(&mut tx).await?;
            let mut member = tx
                .require::<items::Member>(&keys::member_key(scope.tenant_id, target_id))
                .await?;
            let current = Role::from_db(&member.role);
            if current != Role::Member || role != Role::Member {
                scope.require(Role::Owner, "appointing or changing owners and admins")?;
            }
            if current == Role::Owner && role != Role::Owner && owner_count(&mut tx).await? <= 1 {
                return Err(conflict("the tenant needs at least one owner").into());
            }
            member.role = role.as_db().to_string();
            tx.put(&member)?;
            let name = member_name(ctx, &mut tx, target_id).await?;
            audit_member(
                ctx,
                &mut tx,
                actor,
                "member.role_change",
                target("member", target_id, name.clone()),
                &format!(
                    "Changed the role of {name} from {} to {}",
                    current.as_db(),
                    role.as_db()
                ),
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        Member::load(ctx, scope, target_id).await
    }

    async fn remove_member(&self, ctx: &Context<'_>, member_id: ID) -> Result<ID> {
        let target_id = parse_id(&member_id)?;
        let (scope, _) = locate_item::<items::Member>(ctx, "member", target_id, |tenant| {
            keys::member_key(tenant, target_id)
        })
        .await?;
        let actor = scope.require(Role::Admin, "removing members")?;
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            serialize_tenant(&mut tx).await?;
            let member = tx
                .require::<items::Member>(&keys::member_key(scope.tenant_id, target_id))
                .await?;
            let current = Role::from_db(&member.role);
            if current != Role::Member {
                scope.require(Role::Owner, "removing owners and admins")?;
            }
            if current == Role::Owner && owner_count(&mut tx).await? <= 1 {
                return Err(conflict("the tenant needs at least one owner").into());
            }
            let name = member_name(ctx, &mut tx, target_id).await?;
            audit_member(
                ctx,
                &mut tx,
                actor,
                "member.remove",
                target("member", target_id, name.clone()),
                &format!("Removed {name}"),
            )
            .await?;
            tx.delete(&member_key_of(&member))?;
            tx.delete(&keys::membership_key(member.user_id, scope.tenant_id))?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        Ok(member_id)
    }

    /// Sends the message to the task's agent session and resumes a NEEDS_YOU task.
    async fn resume_task(&self, ctx: &Context<'_>, input: ResumeTaskInput) -> Result<Task> {
        let task_id = parse_id(&input.task_id)?;
        let (scope, _) = locate_task(ctx, task_id).await?;
        let (member_id, _) = scope.member()?;
        let message = input.message.trim();
        if message.is_empty() {
            return Err(bad_input("the message is empty"));
        }
        admit(ctx, scope.tenant_id, SOURCE_REPLY, &task_id.to_string()).await?;
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            let mut task = load_task(&mut tx, task_id).await?;
            if task.status != CoreStatus::NeedsYou {
                return Err(conflict("only a task that needs you can be resumed").into());
            }
            let run_id = latest_run(&mut tx, task_id)
                .await?
                .ok_or_else(|| conflict("the task has no agent session yet"))?;
            let name = member_name(ctx, &mut tx, member_id).await?;
            record_run_event(
                &mut tx,
                run_id,
                None,
                RunEventType::HumanReply,
                &json!({ "member": name, "text": message }),
            )
            .await?;
            task.snoozed_until = None;
            tx.put(&task)?;
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "task.resume",
                target("task", task_id, task.title.clone()),
                "Resumed the task with a message",
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        Task::load(ctx, scope, task_id).await
    }

    /// Takes a NEEDS_YOU or usage-limited task out of the needs-you lists until the given time. A usage-limited task resumes no earlier than that time. Resuming or stopping the task, or a change of its status, ends the snooze.
    async fn snooze_task(&self, ctx: &Context<'_>, task_id: ID, until: Timestamp) -> Result<Task> {
        let task_id = parse_id(&task_id)?;
        let (scope, _) = locate_task(ctx, task_id).await?;
        let (member_id, _) = scope.member()?;
        if until.0 <= now(ctx) {
            return Err(bad_input("the snooze time must be in the future"));
        }
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            let mut task = load_task(&mut tx, task_id).await?;
            if !snoozable(&task) {
                return Err(conflict(
                    "only a task that needs you or waits for usage can be snoozed",
                )
                .into());
            }
            snooze(&mut tx, &mut task, until.0)?;
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "task.snooze",
                target("task", task_id, task.title.clone()),
                &format!("Snoozed the task until {}", until.0.to_rfc3339()),
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        Task::load(ctx, scope, task_id).await
    }

    /// Stops the task. Its status becomes ABORTED.
    async fn stop_task(&self, ctx: &Context<'_>, task_id: ID) -> Result<Task> {
        let task_id = parse_id(&task_id)?;
        let (scope, task) = locate_task(ctx, task_id).await?;
        let (member_id, _) = scope.member()?;
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            let name = member_name(ctx, &mut tx, member_id).await?;
            if !abort_task(
                &mut tx,
                task_id,
                &format!("{name} stopped the task"),
                now(ctx),
            )
            .await?
            {
                return Err(conflict("the task has already finished").into());
            }
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "task.stop",
                target("task", task_id, task.title.clone()),
                "Stopped the task",
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        Task::load(ctx, scope, task_id).await
    }

    /// Approves or rejects the approval that the task's skill waits for. Owners, admins, and the member who requested the task can resolve it.
    async fn resolve_approval(
        &self,
        ctx: &Context<'_>,
        task_id: ID,
        approved: bool,
    ) -> Result<Task> {
        let task_id = parse_id(&task_id)?;
        let (scope, task) = locate_task(ctx, task_id).await?;
        let (member_id, role) = scope.member()?;
        if !role.at_least(Role::Admin) && task.requested_by_member_id != Some(member_id) {
            return Err(forbidden(
                "only an owner, an admin, or the member who requested the task can resolve its approvals",
            ));
        }
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            let wait = waiting_approval(&mut tx, task_id)
                .await?
                .ok_or_else(|| conflict("the task has no approval waiting"))?;
            let name = member_name(ctx, &mut tx, member_id).await?;
            resolve_waiting_approval(
                &mut tx,
                wait.skill_run_id,
                &wait.task_token,
                approved,
                &name,
                now(ctx),
            )
            .await
            .map_err(approval_failure)?;
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "approval.resolve",
                target("task", task_id, task.title.clone()),
                if approved {
                    "Approved the request"
                } else {
                    "Rejected the request"
                },
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        Task::load(ctx, scope, task_id).await
    }

    /// Stops the agent session of a run.
    async fn stop_run(&self, ctx: &Context<'_>, run_id: ID) -> Result<Run> {
        let run_id = parse_id(&run_id)?;
        let (scope, _) = locate_ref(ctx, "run", run_id, REF_RUN).await?;
        let (member_id, _) = scope.member()?;
        let run = Run::load(ctx, scope, run_id).await?;
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            let name = member_name(ctx, &mut tx, member_id).await?;
            if !stop_run(
                &mut tx,
                run_id,
                &format!("{name} stopped the session"),
                now(ctx),
            )
            .await?
            {
                return Err(conflict("the session has already ended").into());
            }
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "run.stop",
                AuditTarget {
                    kind: "task",
                    id: run.row.task_id.to_string(),
                    name: format!("run {}", run.row.number),
                },
                &format!("Stopped run {}", run.row.number),
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        Run::load(ctx, scope, run_id).await
    }

    /// Sends a chat message. With taskId, the message goes to that task's agent session.
    /// Without taskId, a new chat starts in the given scope ("all" or a project name).
    async fn send_chat_message(
        &self,
        ctx: &Context<'_>,
        input: SendChatMessageInput,
    ) -> Result<SendChatMessagePayload> {
        let scope = tenant_scope(ctx, &input.tenant_id).await?;
        let (member_id, _) = scope.member()?;
        let text = input.text.trim().to_string();
        if text.is_empty() {
            return Err(bad_input("the message is empty"));
        }
        let triggers = Trigger::all(ctx, scope).await?;
        let (existing, thread, chat_scope) = match &input.task_id {
            Some(task_id) => {
                let task = Task::load(ctx, scope, parse_id(task_id)?).await?;
                let Some((crate::enums::ChannelKind::Web, thread)) = task.chat_thread() else {
                    return Err(bad_input("the task is not a web chat"));
                };
                let chat_scope = task.chat_scope(ctx).await?;
                (Some(task.row.id), thread, chat_scope)
            }
            None => {
                let chat_scope = input
                    .scope
                    .clone()
                    .ok_or_else(|| bad_input("a new chat needs a scope"))?;
                let accepts = triggers
                    .iter()
                    .filter(|trigger| trigger.row.state == "ENABLED")
                    .map(Trigger::trigger_match)
                    .collect::<Result<Vec<_>>>()?
                    .iter()
                    .any(|m| {
                        m.event_source() == EventSource::Web
                            && m.scope_name() == Some(chat_scope.as_str())
                    });
                if !accepts {
                    return Err(bad_input(format!(
                        "no enabled web chat trigger accepts the scope {chat_scope}"
                    )));
                }
                (None, Uuid::new_v4().to_string(), chat_scope)
            }
        };
        let key = Uuid::new_v4().to_string();
        admit(ctx, scope.tenant_id, SOURCE_CHAT, &key).await?;
        let (inbound_id, received_at) = write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            let user = member_name(ctx, &mut tx, member_id).await?;
            let event = json!({
                "source": "web",
                "detail-type": "Message",
                "detail": {
                    "scope": chat_scope,
                    "thread": thread,
                    "user": user,
                    "member_id": member_id,
                    "text": text,
                },
            });
            record_inbound_event(&mut tx, "web", &key, &event).await?;
            let inbound = tx
                .require::<InboundEvent>(&keys::inbound_event_key(scope.tenant_id, "web", &key))
                .await?;
            let audit_target = match existing {
                Some(task_id) => target("task", task_id, thread.clone()),
                None => AuditTarget {
                    kind: "chat",
                    id: thread.clone(),
                    name: chat_scope.clone(),
                },
            };
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "chat.message_send",
                audit_target,
                &format!("Sent a chat message in {chat_scope}"),
            )
            .await?;
            tx.commit().await?;
            Ok((inbound.id, inbound.received_at))
        })
        .await?;
        state(ctx)
            .deps
            .chat
            .dispatch(scope.tenant_id, "web", &key)
            .await
            .map_err(port)?;
        let task = match existing {
            Some(task_id) => Task::load(ctx, scope, task_id).await?,
            None => {
                let mut tx = tx(ctx, scope.tenant_id);
                let mut events = Vec::new();
                for trigger in &triggers {
                    if let Some(event) = tx
                        .get::<EventItem>(&keys::trigger_event_key(
                            scope.tenant_id,
                            trigger.row.id,
                            &key,
                        ))
                        .await?
                    {
                        events.push(event);
                    }
                }
                match events.iter().find_map(|event| event.task_id) {
                    Some(task_id) => Task::load(ctx, scope, task_id).await?,
                    None => match events.into_iter().next() {
                        Some(event) => {
                            return Err(conflict(format!(
                                "the chat trigger did not start a task: {}",
                                event
                                    .reason
                                    .unwrap_or_else(|| "the event is not evaluated yet".into())
                            )));
                        }
                        None => return Err(conflict("no chat trigger received the message")),
                    },
                }
            }
        };
        Ok(SendChatMessagePayload {
            task,
            message: ChatMessage::inbound(scope, inbound_id, member_id, text, received_at),
        })
    }

    async fn set_trigger_state(
        &self,
        ctx: &Context<'_>,
        trigger_id: ID,
        state: TriggerState,
    ) -> Result<Trigger> {
        let trigger_id = parse_id(&trigger_id)?;
        let (scope, location) = locate_ref(ctx, "trigger", trigger_id, REF_TRIGGER).await?;
        let member_id = scope.require(Role::Admin, "enabling and disabling triggers")?;
        let location = &location;
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            let mut trigger = tx.require::<TriggerItem>(&location.target).await?;
            trigger.state = state.as_db().to_string();
            if let Some(spec) = trigger
                .next
                .as_mut()
                .and_then(|next| next.definition.as_mut())
            {
                spec.state = state.as_db().to_string();
            }
            schedule_trigger(&mut tx, &mut trigger, now(ctx)).map_err(worker_error)?;
            tx.put(&trigger)?;
            let (action, verb) = match state {
                TriggerState::Enabled => ("trigger.enable", "Enabled"),
                TriggerState::Disabled => ("trigger.disable", "Disabled"),
            };
            audit_member(
                ctx,
                &mut tx,
                member_id,
                action,
                target("trigger", trigger_id, trigger.name.clone()),
                &format!("{verb} the trigger {}", trigger.name),
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        Trigger::load(ctx, scope, trigger_id).await
    }

    /// Opens a change request on a config repository that adds a trigger from a template.
    async fn propose_trigger_from_template(
        &self,
        ctx: &Context<'_>,
        input: ProposeTriggerInput,
    ) -> Result<ChangeRequest> {
        let scope = tenant_scope(ctx, &input.tenant_id).await?;
        let (member_id, _) = scope.member()?;
        let template = TRIGGER_TEMPLATES
            .iter()
            .find(|template| template.id == input.template_id.as_str())
            .ok_or_else(|| not_found(format!("trigger template {}", input.template_id.as_str())))?;
        if !is_repository(&input.repository) {
            return Err(bad_input(format!("{} is not owner/name", input.repository)));
        }
        let config = Config::load(ctx, scope, parse_id(&input.config_id)?).await?;
        let mut session = None;
        for candidate in Session::all(ctx, scope).await? {
            if candidate.config_id == config.row.id
                && candidate
                    .repository_names(ctx)
                    .await?
                    .contains(&input.repository)
            {
                session = Some(candidate.name);
                break;
            }
        }
        let session = session.ok_or_else(|| {
            bad_input(format!(
                "no session of {} clones {}",
                config.row.repository, input.repository
            ))
        })?;
        let taken = Trigger::all(ctx, scope)
            .await?
            .into_iter()
            .filter(|trigger| trigger.row.config_id == config.row.id)
            .map(|trigger| trigger.row.name)
            .collect::<Vec<_>>();
        let trigger_name = (1..)
            .map(|n| {
                if n == 1 {
                    template.name.to_string()
                } else {
                    format!("{}-{n}", template.name)
                }
            })
            .find(|name| !taken.contains(name))
            .expect("some suffix is free");
        let mut document = Map::new();
        document.insert(
            trigger_name.clone(),
            template.render(&input.repository, &session),
        );
        let trigger_yaml = serde_yaml_ng::to_string(&Value::Object(document))?;
        let title = format!("Add the {trigger_name} trigger for {}", input.repository);
        let number = state(ctx)
            .deps
            .config_repository
            .propose_trigger(
                scope.tenant_id,
                &TriggerProposal {
                    location: ConfigLocation {
                        repository: config.row.repository.clone(),
                        root: config.row.root_path.clone(),
                        branch: config.row.branch.clone(),
                    },
                    title: title.clone(),
                    trigger_name: trigger_name.clone(),
                    trigger_yaml,
                },
            )
            .await?;
        let config_row = &config.row;
        let row = write(|| async {
            let at = now(ctx);
            let row = items::ChangeRequest {
                id: Uuid::new_v4(),
                tenant_id: scope.tenant_id,
                config_id: config_row.id,
                pr_number: i64::from(number),
                title: title.clone(),
                state: "open".into(),
                changed_files: json!([format!(
                    "{}{}",
                    crate::config::root_prefix(&config_row.root_path),
                    sanchoris_config::CONFIG_FILE
                )]),
                changed_definitions: json!([{ "kind": "trigger", "name": trigger_name }]),
                validation_status: None,
                validation_messages: json!([]),
                requested_by_member_id: Some(member_id),
                requested_by_task_id: None,
                created_at: at,
                updated_at: at,
            };
            let mut tx = tx(ctx, scope.tenant_id);
            tx.insert(&row)?;
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "change_request.open",
                target("change_request", row.id, format!("#{number}")),
                &title,
            )
            .await?;
            tx.commit().await?;
            Ok(row)
        })
        .await?;
        Ok(ChangeRequest::new(scope, config.row.repository, row))
    }

    /// Adds a config after checking the repository. Returns the validation errors when the check fails.
    /// No mutation removes a config: removing its repository from the GitHub App installation on GitHub disables it.
    async fn add_config(
        &self,
        ctx: &Context<'_>,
        input: AddConfigInput,
    ) -> Result<AddConfigPayload> {
        let scope = tenant_scope(ctx, &input.tenant_id).await?;
        let member_id = scope.require(Role::Admin, "adding configs")?;
        let repository = input.repository.trim().to_string();
        let branch = input.branch.trim().to_string();
        if !is_repository(&repository) {
            return Err(bad_input(format!("{repository} is not owner/name")));
        }
        if branch.is_empty() {
            return Err(bad_input("the config needs a branch"));
        }
        let location = ConfigLocation {
            repository: repository.clone(),
            root: input.root.clone(),
            branch: branch.clone(),
        };
        let checked = state(ctx)
            .deps
            .config_repository
            .check(scope.tenant_id, &location)
            .await
            .map_err(port)?;
        let found = match checked {
            ConfigCheck::Valid(found) => found,
            ConfigCheck::Invalid(problems) => {
                let count = problems.len();
                write(|| async {
                    let mut tx = tx(ctx, scope.tenant_id);
                    audit_member(
                        ctx,
                        &mut tx,
                        member_id,
                        "config.validation_fail",
                        AuditTarget {
                            kind: "config",
                            id: format!("{repository}:{}", input.root),
                            name: repository.clone(),
                        },
                        &format!(
                            "The config at {repository} {} has {count} problems",
                            input.root
                        ),
                    )
                    .await?;
                    tx.commit().await?;
                    Ok(())
                })
                .await?;
                return Ok(AddConfigPayload {
                    config: None,
                    errors: problems
                        .into_iter()
                        .map(|problem| ValidationError {
                            path: problem.path,
                            line: problem.line,
                            column: problem.column,
                            message: problem.message,
                        })
                        .collect(),
                });
            }
        };
        let repository = found.full_name.clone();
        let at = now(ctx);
        let config_id = write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            let guard_key = keys::config_repository_key(scope.tenant_id, found.id, &input.root);
            if tx.get::<Guard>(&guard_key).await?.is_some() {
                return Err(
                    conflict(format!("{repository} {} is already a config", input.root)).into(),
                );
            }
            let config_id = Uuid::new_v4();
            tx.insert(&items::Config {
                id: config_id,
                tenant_id: scope.tenant_id,
                repository_id: Some(found.id),
                repository: repository.clone(),
                root_path: input.root.clone(),
                branch: branch.clone(),
                status: CONFIG_ACTIVE.into(),
                active_revision_id: None,
                revisions: 0,
                repositories: BTreeMap::new(),
                synced_at: None,
                sync_error: None,
                created_at: at,
                updated_at: at,
            })?;
            tx.insert(&Guard::new(guard_key, config_id))?;
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "config.add",
                target("config", config_id, repository.clone()),
                &format!("Added the config {repository} {} on {branch}", input.root),
            )
            .await?;
            tx.commit().await?;
            Ok(config_id)
        })
        .await?;
        state(ctx)
            .deps
            .config_sync
            .request_sync(scope.tenant_id, config_id)
            .await
            .map_err(port)?;
        Ok(AddConfigPayload {
            config: Some(Config::load(ctx, scope, config_id).await?),
            errors: Vec::new(),
        })
    }

    /// Reads the config repository again. A DISABLED config cannot be synced.
    async fn sync_config(&self, ctx: &Context<'_>, config_id: ID) -> Result<Config> {
        let config_id = parse_id(&config_id)?;
        let (scope, config) = locate_item::<items::Config>(ctx, "config", config_id, |tenant| {
            keys::config_key(tenant, config_id)
        })
        .await?;
        let member_id = scope.require(Role::Admin, "syncing configs")?;
        if !config.is_active() {
            return Err(bad_input(format!(
                "the config {} is disabled: {}",
                config.repository,
                config
                    .sync_error
                    .as_deref()
                    .unwrap_or("its repository left the GitHub App installation")
            )));
        }
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "config.sync",
                target("config", config_id, config.repository.clone()),
                "Requested a config sync",
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        state(ctx)
            .deps
            .config_sync
            .request_sync(scope.tenant_id, config_id)
            .await
            .map_err(port)?;
        Config::load(ctx, scope, config_id).await
    }

    /// Saves an API key credential. Subscriptions are connected with the sanchoris CLI.
    async fn add_credential(
        &self,
        ctx: &Context<'_>,
        input: AddCredentialInput,
    ) -> Result<Credential> {
        let scope = tenant_scope(ctx, &input.tenant_id).await?;
        let (member_id, _) = scope.member()?;
        if input.scope == CredentialScope::Organization {
            scope.require(Role::Admin, "adding organization credentials")?;
        }
        let name = input.name.trim().to_string();
        if name.is_empty() || name == sanchoris_config::REQUESTER {
            return Err(bad_input(format!("{name:?} cannot name a credential")));
        }
        let key = input.key.trim();
        if key.is_empty() {
            return Err(bad_input("the key is empty"));
        }
        if !key_matches_provider(input.provider, key) {
            return Err(bad_input(format!(
                "the key does not look like a {:?} key",
                input.provider
            )));
        }
        let (provider, kind) = match input.provider {
            ApiKeyProvider::Anthropic => ("claude", "api_key"),
            ApiKeyProvider::Openai => ("codex", "api_key"),
            ApiKeyProvider::Bedrock => ("claude", "bedrock"),
        };
        let encrypted = state(ctx)
            .deps
            .encryptor
            .encrypt(scope.tenant_id, key.as_bytes())
            .await
            .map_err(port)?;
        let at = now(ctx);
        let encrypted = &encrypted;
        let credential_id = write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            let guard_key = keys::credential_name_key(scope.tenant_id, &name);
            if tx.get::<Guard>(&guard_key).await?.is_some() {
                return Err(conflict(format!("a credential named {name} exists")).into());
            }
            let credential_id = Uuid::new_v4();
            tx.insert(&items::Credential {
                id: credential_id,
                tenant_id: scope.tenant_id,
                scope: input.scope.as_db().to_string(),
                member_id: (input.scope == CredentialScope::Personal).then_some(member_id),
                name: name.clone(),
                provider: provider.to_string(),
                kind: kind.to_string(),
                account: mask_key(key),
                status: "connected".into(),
                subscription_plan: None,
                rejected_at: None,
                encrypted_value: STANDARD.encode(&encrypted.ciphertext),
                kms_key_id: encrypted.key_id.clone(),
                last_used_at: None,
                created_at: at,
                updated_at: at,
            })?;
            tx.insert(&Guard::new(guard_key, credential_id))?;
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "credential.connect",
                target("credential", credential_id, name.clone()),
                &format!("Added the {kind} credential {name}"),
            )
            .await?;
            tx.commit().await?;
            Ok(credential_id)
        })
        .await?;
        Credential::by_id(ctx, scope, credential_id).await
    }

    /// Removes an API key, or disconnects a subscription login.
    async fn remove_credential(&self, ctx: &Context<'_>, credential_id: ID) -> Result<ID> {
        let id = parse_id(&credential_id)?;
        let (scope, credential) =
            locate_item::<items::Credential>(ctx, "credential", id, |tenant| {
                keys::credential_key(tenant, id)
            })
            .await?;
        let (member_id, role) = scope.member()?;
        let own = credential.member_id == Some(member_id);
        if !own && !role.at_least(Role::Admin) {
            return Err(forbidden(
                "removing this credential needs the admin role or higher",
            ));
        }
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            tx.delete(&keys::credential_key(scope.tenant_id, id))?;
            tx.delete(&keys::credential_name_key(
                scope.tenant_id,
                &credential.name,
            ))?;
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "credential.delete",
                target("credential", id, credential.name.clone()),
                &format!("Removed the credential {}", credential.name),
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        Ok(credential_id)
    }

    /// Changes the plan. Returns a Stripe Checkout URL when payment is needed. Refused for a tenant on the Admin plan and for the Admin plan as the target.
    async fn change_plan(
        &self,
        ctx: &Context<'_>,
        tenant_id: ID,
        plan_id: ID,
    ) -> Result<ChangePlanPayload> {
        let scope = tenant_scope(ctx, &tenant_id).await?;
        let member_id = scope.require(Role::Owner, "changing the plan")?;
        let plan = Plan::load(ctx, plan_id.as_str()).await?;
        let tenant = Tenant::load(ctx, scope).await?;
        if tenant.row.plan_id == ADMIN_PLAN {
            return Err(conflict(format!("the {ADMIN_PLAN} plan does not change")));
        }
        if plan.is_admin() {
            return Err(forbidden(format!("the {ADMIN_PLAN} plan is not for sale")));
        }
        if tenant.row.plan_id == plan.0.id {
            return Err(bad_input(format!(
                "the tenant is already on {}",
                plan.0.name
            )));
        }
        let to_free = plan.0.id == FREE_PLAN;
        if to_free {
            write(|| async {
                let mut tx = tx(ctx, scope.tenant_id);
                let current = db::tenant(&mut tx).await?;
                if !hold_free_tenant(&mut tx, &current).await? {
                    return Err(conflict(TenantError::FreeTenantTaken.to_string()).into());
                }
                tx.commit().await?;
                Ok(())
            })
            .await?;
        }
        let change = match state(ctx)
            .deps
            .checkout
            .change_plan(scope.tenant_id, &plan.0.id)
            .await
        {
            Ok(change) => change,
            Err(error) => {
                if to_free {
                    write(|| async {
                        let mut tx = tx(ctx, scope.tenant_id);
                        let current = db::tenant(&mut tx).await?;
                        if current.plan_id != FREE_PLAN {
                            release_free_tenant(&mut tx, &current).await?;
                            tx.commit().await?;
                        }
                        Ok(())
                    })
                    .await?;
                }
                return Err(port(error));
            }
        };
        let change = &change;
        let checkout_url = write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            let checkout_url = match change {
                PlanChange::Applied => {
                    let current = db::tenant(&mut tx).await?;
                    if current.plan_id != plan.0.id
                        && !stage_plan_change(&mut tx, &current, &plan.0.id, now(ctx), true).await?
                    {
                        return Err(conflict(TenantError::FreeTenantTaken.to_string()).into());
                    }
                    audit_member(
                        ctx,
                        &mut tx,
                        member_id,
                        "tenant.plan_change",
                        target("tenant", scope.tenant_id, tenant.row.name.clone()),
                        &format!(
                            "Changed the plan from {} to {}",
                            tenant.row.plan_id, plan.0.id
                        ),
                    )
                    .await?;
                    None
                }
                PlanChange::Checkout { url } => {
                    audit_member(
                        ctx,
                        &mut tx,
                        member_id,
                        "tenant.plan_checkout_start",
                        target("tenant", scope.tenant_id, tenant.row.name.clone()),
                        &format!("Started the checkout for {}", plan.0.id),
                    )
                    .await?;
                    Some(url.clone())
                }
                PlanChange::Portal { url } => {
                    audit_member(
                        ctx,
                        &mut tx,
                        member_id,
                        "billing.portal_open",
                        target("tenant", scope.tenant_id, "billing"),
                        &format!(
                            "Opened the billing portal to settle the subscription before {}",
                            plan.0.id
                        ),
                    )
                    .await?;
                    Some(url.clone())
                }
            };
            tx.commit().await?;
            Ok(checkout_url)
        })
        .await?;
        Ok(ChangePlanPayload {
            tenant: Tenant::load(ctx, scope).await?,
            checkout_url,
        })
    }

    /// Returns the URL of the Stripe customer portal. Refused for a tenant on the Admin plan.
    async fn open_billing_portal(
        &self,
        ctx: &Context<'_>,
        tenant_id: ID,
    ) -> Result<RedirectPayload> {
        let scope = tenant_scope(ctx, &tenant_id).await?;
        let member_id = scope.require(Role::Owner, "opening the billing portal")?;
        if Tenant::load(ctx, scope).await?.row.plan_id == ADMIN_PLAN {
            return Err(conflict(format!("the {ADMIN_PLAN} plan has no billing")));
        }
        let url = state(ctx)
            .deps
            .checkout
            .billing_portal(scope.tenant_id)
            .await
            .map_err(port)?;
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "billing.portal_open",
                target("tenant", scope.tenant_id, "billing"),
                "Opened the billing portal",
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        Ok(RedirectPayload { url })
    }

    /// Returns a short-lived URL of the audit log as JSON Lines.
    async fn export_audit_log(
        &self,
        ctx: &Context<'_>,
        tenant_id: ID,
        filter: Option<AuditFilter>,
    ) -> Result<RedirectPayload> {
        let scope = tenant_scope(ctx, &tenant_id).await?;
        let member_id = scope.require(Role::Admin, "exporting the audit log")?;
        let events = AuditEvent::list(ctx, scope, filter.unwrap_or_default(), None).await?;
        let mut body = Vec::new();
        for event in &events {
            serde_json::to_writer(&mut body, &event.json_line())?;
            body.push(b'\n');
        }
        let at = now(ctx);
        let file_name = format!(
            "audit-{}-{}.jsonl",
            scope.tenant_id,
            at.format("%Y%m%dT%H%M%SZ")
        );
        let url = state(ctx)
            .deps
            .audit_export
            .publish(scope.tenant_id, &file_name, body)
            .await
            .map_err(port)?;
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "audit_log.export",
                target("tenant", scope.tenant_id, file_name.clone()),
                &format!("Exported {} audit events", events.len()),
            )
            .await?;
            tx.commit().await?;
            Ok(())
        })
        .await?;
        Ok(RedirectPayload { url })
    }

    /// Deletes the tenant (owners). slug must equal the tenant's slug. The tenant becomes DELETING at once: its unfinished tasks stop, its subscription is cancelled, and the worker then removes its data and records.
    async fn delete_tenant(&self, ctx: &Context<'_>, tenant_id: ID, slug: String) -> Result<ID> {
        let scope = tenant_scope(ctx, &tenant_id).await?;
        let member_id = scope.require(Role::Owner, "deleting the tenant")?;
        let tenant = store(ctx)
            .get::<items::Tenant>(&keys::tenant_key(scope.tenant_id))
            .await?
            .ok_or_else(|| not_found(format!("tenant {}", scope.tenant_id)))?;
        if slug.trim() != tenant.slug {
            return Err(bad_input(format!(
                "type the tenant's slug {} to delete it",
                tenant.slug
            )));
        }
        if tenant.plan_id != ADMIN_PLAN {
            state(ctx)
                .deps
                .checkout
                .end_billing(scope.tenant_id)
                .await
                .map_err(port)?;
        }
        let at = now(ctx);
        write(|| async {
            let mut tx = tx(ctx, scope.tenant_id);
            let key = keys::tenant_key(scope.tenant_id);
            let current = tx.require::<items::Tenant>(&key).await?;
            if is_deleting(&current) {
                return Err(being_deleted(&current).into());
            }
            tx.update(
                &key,
                Update::new()
                    .set("status", s(TENANT_DELETING))
                    .set("updated_at", s(&format_time(at)))
                    .require("status", s(&current.status)),
            )?;
            audit_member(
                ctx,
                &mut tx,
                member_id,
                "tenant.delete",
                target("tenant", scope.tenant_id, current.name.clone()),
                &format!("Deleted tenant {}", current.name),
            )
            .await?;
            tx.emit_at(
                Job::DeleteTenant {
                    tenant_id: scope.tenant_id,
                    phase: Deletion::Release,
                },
                at + Duration::seconds(DELETION_DELAY_SECONDS),
            );
            tx.commit().await?;
            Ok(())
        })
        .await?;
        let unfinished = store(ctx)
            .query_index::<TaskItem>(
                Index::Gsi1,
                &keys::tasks_index(scope.tenant_id),
                SortKey::All,
            )
            .await?
            .into_iter()
            .filter(|task| !task.status.is_terminal());
        for task in unfinished {
            write(|| async {
                let mut tx = tx(ctx, scope.tenant_id);
                if abort_task(&mut tx, task.id, "the tenant is being deleted", at).await? {
                    tx.commit().await?;
                }
                Ok(())
            })
            .await?;
        }
        Ok(ID(scope.tenant_id.to_string()))
    }

    /// Operator only.
    async fn suspend_tenant(
        &self,
        ctx: &Context<'_>,
        tenant_id: ID,
        reason: String,
    ) -> Result<Tenant> {
        if !is_operator(ctx).await? {
            return Err(forbidden("only operators can suspend tenants"));
        }
        let reason = reason.trim().to_string();
        if reason.is_empty() {
            return Err(bad_input("suspending a tenant needs a reason"));
        }
        suspend_or_resume(
            ctx,
            &tenant_id,
            "active",
            Update::new().set("suspended_reason", s(&reason)),
            "suspend",
            &format!("Suspended the tenant: {reason}"),
        )
        .await
    }

    /// Operator only.
    async fn unsuspend_tenant(&self, ctx: &Context<'_>, tenant_id: ID) -> Result<Tenant> {
        if !is_operator(ctx).await? {
            return Err(forbidden("only operators can unsuspend tenants"));
        }
        suspend_or_resume(
            ctx,
            &tenant_id,
            "suspended",
            Update::new().remove("suspended_reason"),
            "unsuspend",
            "Unsuspended the tenant",
        )
        .await
    }
}

fn member_key_of(member: &items::Member) -> sanchoris_core::item::Key {
    keys::member_key(member.tenant_id, member.id)
}

fn approval_failure(error: SkillRunError) -> Failure {
    match error {
        SkillRunError::Db(error) => Failure::Db(error),
        SkillRunError::ApprovalNotWaiting => conflict(error.to_string()).into(),
        other => Error::new(other.to_string()).into(),
    }
}

async fn locate_task(ctx: &Context<'_>, task_id: Uuid) -> Result<(Scope, TaskItem)> {
    locate_item::<TaskItem>(ctx, "task", task_id, |tenant| {
        keys::task_key(tenant, task_id)
    })
    .await
}
