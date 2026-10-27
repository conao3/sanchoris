use async_graphql::{Context, ID, InputObject, Object, Result, SimpleObject};
use chrono::Duration;
use sanchoris_core::channel::{RunEventType, record_inbound_event, record_run_event};
use sanchoris_core::db::{DbError, TenantTx};
use sanchoris_core::model::AuditOrigin;
use sanchoris_core::tasks::{abort_task, latest_run, stop_run};
use sanchoris_core::tenants::{NewTenant, create_tenant};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::account::{Credential, Invitation, Member, Plan, Viewer, mask_key};
use crate::audit::{AuditEvent, AuditFilter};
use crate::config::{ChangeRequest, Config, Session, ValidationError};
use crate::context::{
    AuditTarget, ClientIp, Role, Scope, audit_member, audit_operator, auth, is_operator, locate,
    member_scope, memberships, now, scope_for_tenant, state, tenant_tx, user_tx,
};
use crate::enums::{
    ApiKeyProvider, CredentialScope, EventSource, InvitationState, TaskStatus, TriggerState,
};
use crate::error::{bad_input, conflict, forbidden, not_found, parse_id};
use crate::ports::{ConfigLocation, PlanChange, TenantCheckout, TriggerProposal};
use crate::scalars::Timestamp;
use crate::task::{ChatMessage, Run, Task};
use crate::tenant::Tenant;
use crate::trigger::{TRIGGER_TEMPLATES, Trigger};

const INVITATION_DAYS: i64 = 7;

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
    tenant: Option<Tenant>,
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

fn is_unique_violation(error: &DbError) -> bool {
    matches!(error, DbError::Sqlx(sqlx::Error::Database(db)) if db.is_unique_violation())
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

async fn member_name(tx: &mut TenantTx, member_id: Uuid) -> Result<String> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT u.display_name FROM sanchoris.members m JOIN sanchoris.users u ON u.id = m.user_id WHERE m.id = $1",
    )
    .bind(member_id)
    .fetch_one(tx.conn())
    .await?)
}

async fn tenant_scope(ctx: &Context<'_>, tenant_id: &ID) -> Result<Scope> {
    member_scope(ctx, parse_id(tenant_id)?).await
}

async fn audit_user_tenants(ctx: &Context<'_>, action: &str, summary: &str) -> Result<()> {
    let user_id = auth(ctx)?.user_id;
    for membership in memberships(ctx).await? {
        let mut tx = tenant_tx(ctx, membership.tenant_id).await?;
        audit_member(
            ctx,
            &mut tx,
            membership.member_id,
            action,
            target("user", user_id, auth(ctx)?.email.clone()),
            summary,
        )
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

async fn owner_count(tx: &mut TenantTx) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM sanchoris.members WHERE tenant_id = $1 AND role = 'owner'",
    )
    .bind(tx.tenant_id())
    .fetch_one(tx.conn())
    .await?)
}

#[Object(name = "Mutation")]
impl MutationRoot {
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
                .await?;
            return Ok(CreateTenantPayload {
                tenant: None,
                checkout_url: Some(url),
            });
        }
        let mut tx = user_tx(ctx).await?;
        let free_tenants = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM sanchoris.tenants t JOIN sanchoris.plans p ON p.id = t.plan_id \
             WHERE t.created_by_user_id = $1 AND p.price_cents = 0",
        )
        .bind(user.user_id)
        .fetch_one(tx.conn())
        .await?;
        drop(tx);
        if free_tenants > 0 {
            return Err(conflict("a user can create one tenant on a free plan"));
        }
        let created = create_tenant(
            &state(ctx).pool,
            &NewTenant {
                name,
                plan_id: &plan.0.id,
                user_id: user.user_id,
                stripe_customer_id: None,
                origin: AuditOrigin::Web,
                source_ip: ctx.data_opt::<ClientIp>().map(|ip| ip.0),
                now: now(ctx),
            },
        )
        .await?;
        let tenant = Tenant::load(ctx, member_scope(ctx, created.tenant_id).await?).await?;
        Ok(CreateTenantPayload {
            tenant: Some(tenant),
            checkout_url: None,
        })
    }

    async fn accept_invitation(&self, ctx: &Context<'_>, invitation_id: ID) -> Result<Member> {
        let invitation_id = parse_id(&invitation_id)?;
        let user = auth(ctx)?;
        let mut tx = user_tx(ctx).await?;
        let tenant_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT tenant_id FROM sanchoris.invitations WHERE id = $1",
        )
        .bind(invitation_id)
        .fetch_optional(tx.conn())
        .await?
        .ok_or_else(|| not_found(format!("invitation {invitation_id}")))?;
        drop(tx);
        if memberships(ctx)
            .await?
            .iter()
            .any(|m| m.tenant_id == tenant_id)
        {
            return Err(conflict("the viewer is already a member of the tenant"));
        }
        let invitee = Scope {
            tenant_id,
            access: crate::context::Access::Invitee,
        };
        let invitation = Invitation::load(ctx, invitee, invitation_id).await?;
        if invitation.state_at(now(ctx))? != InvitationState::Pending {
            return Err(conflict("the invitation is no longer pending"));
        }
        if !invitation.row.email.eq_ignore_ascii_case(&user.email) {
            return Err(forbidden("the invitation is addressed to another email"));
        }
        let at = now(ctx);
        let mut tx = tenant_tx(ctx, tenant_id).await?;
        let updated = sqlx::query(
            "UPDATE sanchoris.invitations SET status = 'accepted', accepted_at = $2 \
             WHERE id = $1 AND status = 'pending'",
        )
        .bind(invitation_id)
        .bind(at)
        .execute(tx.conn())
        .await?;
        if updated.rows_affected() != 1 {
            return Err(conflict("the invitation is no longer pending"));
        }
        let member_id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO sanchoris.members (tenant_id, user_id, role, joined_at, last_active_at) \
             VALUES ($1, $2, $3, $4, $4) RETURNING id",
        )
        .bind(tenant_id)
        .bind(user.user_id)
        .bind(&invitation.row.role)
        .bind(at)
        .fetch_one(tx.conn())
        .await?;
        audit_member(
            ctx,
            &mut tx,
            member_id,
            "member.invitation_accept",
            target("invitation", invitation_id, invitation.row.email.clone()),
            &format!("Accepted the invitation as {}", invitation.row.role),
        )
        .await?;
        tx.commit().await?;
        Member::load(ctx, member_scope(ctx, tenant_id).await?, member_id).await
    }

    #[graphql(name = "connectGitHub")]
    async fn connect_github(&self, ctx: &Context<'_>) -> Result<RedirectPayload> {
        let user = auth(ctx)?;
        audit_user_tenants(
            ctx,
            "integration.github_link_start",
            "Started linking a GitHub account",
        )
        .await?;
        Ok(RedirectPayload {
            url: state(ctx).deps.github.link_account(user.user_id),
        })
    }

    #[graphql(name = "unlinkGitHub")]
    async fn unlink_github(&self, ctx: &Context<'_>) -> Result<Viewer> {
        let mut tx = user_tx(ctx).await?;
        let login = sqlx::query_scalar::<_, Option<String>>(
            "SELECT github_login FROM sanchoris.users WHERE id = $1",
        )
        .bind(tx.user_id())
        .fetch_one(tx.conn())
        .await?
        .ok_or_else(|| conflict("no GitHub account is linked"))?;
        sqlx::query(
            "UPDATE sanchoris.users SET github_user_id = NULL, github_login = NULL, updated_at = $2 WHERE id = $1",
        )
        .bind(tx.user_id())
        .bind(now(ctx))
        .execute(tx.conn())
        .await?;
        tx.commit().await?;
        audit_user_tenants(
            ctx,
            "integration.github_unlink",
            &format!("Unlinked the GitHub account {login}"),
        )
        .await?;
        Ok(Viewer)
    }

    async fn connect_slack(&self, ctx: &Context<'_>, tenant_id: ID) -> Result<RedirectPayload> {
        let scope = tenant_scope(ctx, &tenant_id).await?;
        let member_id = scope.require(Role::Admin, "connecting Slack")?;
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
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
        Ok(RedirectPayload {
            url: state(ctx).deps.slack.install(scope.tenant_id),
        })
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
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        sqlx::query("SELECT id FROM sanchoris.tenants WHERE id = $1 FOR UPDATE")
            .bind(scope.tenant_id)
            .execute(tx.conn())
            .await?;
        let (seats, limit, already) = sqlx::query_as::<_, (i64, i32, bool)>(
            "SELECT (SELECT count(*) FROM sanchoris.members WHERE tenant_id = $1) \
                  + (SELECT count(*) FROM sanchoris.invitations WHERE tenant_id = $1 AND status = 'pending' AND expires_at > $2), \
                    (SELECT p.member_limit FROM sanchoris.tenants t JOIN sanchoris.plans p ON p.id = t.plan_id WHERE t.id = $1), \
                    EXISTS (SELECT 1 FROM sanchoris.members m JOIN sanchoris.users u ON u.id = m.user_id \
                            WHERE m.tenant_id = $1 AND lower(u.email) = $3)",
        )
        .bind(scope.tenant_id)
        .bind(at)
        .bind(&email)
        .fetch_one(tx.conn())
        .await?;
        if already {
            return Err(conflict(format!("{email} is already a member")));
        }
        if seats >= i64::from(limit) {
            return Err(conflict(format!(
                "the plan allows {limit} members including pending invitations"
            )));
        }
        sqlx::query(
            "UPDATE sanchoris.invitations SET status = 'expired' \
             WHERE tenant_id = $1 AND email = $2 AND status = 'pending' AND expires_at <= $3",
        )
        .bind(scope.tenant_id)
        .bind(&email)
        .bind(at)
        .execute(tx.conn())
        .await?;
        let invitation_id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO sanchoris.invitations (tenant_id, email, role, status, invited_by_member_id, expires_at, created_at) \
             VALUES ($1, $2, $3, 'pending', $4, $5, $6) RETURNING id",
        )
        .bind(scope.tenant_id)
        .bind(&email)
        .bind(input.role.as_db())
        .bind(member_id)
        .bind(at + Duration::days(INVITATION_DAYS))
        .bind(at)
        .fetch_one(tx.conn())
        .await
        .map_err(DbError::from)
        .map_err(|error| {
            if is_unique_violation(&error) {
                conflict(format!("{email} already has a pending invitation"))
            } else {
                error.into()
            }
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
        Invitation::load(ctx, scope, invitation_id).await
    }

    async fn revoke_invitation(&self, ctx: &Context<'_>, invitation_id: ID) -> Result<Invitation> {
        let invitation_id = parse_id(&invitation_id)?;
        let scope = locate(ctx, "invitations", invitation_id).await?;
        let member_id = scope.require(Role::Admin, "revoking invitations")?;
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let email = sqlx::query_scalar::<_, String>(
            "UPDATE sanchoris.invitations SET status = 'revoked' WHERE id = $1 AND status = 'pending' RETURNING email",
        )
        .bind(invitation_id)
        .fetch_optional(tx.conn())
        .await?
        .ok_or_else(|| conflict("the invitation is no longer pending"))?;
        audit_member(
            ctx,
            &mut tx,
            member_id,
            "member.invitation_revoke",
            target("invitation", invitation_id, email.clone()),
            &format!("Revoked the invitation of {email}"),
        )
        .await?;
        tx.commit().await?;
        Invitation::load(ctx, scope, invitation_id).await
    }

    async fn change_member_role(
        &self,
        ctx: &Context<'_>,
        member_id: ID,
        role: Role,
    ) -> Result<Member> {
        let target_id = parse_id(&member_id)?;
        let scope = locate(ctx, "members", target_id).await?;
        let actor = scope.require(Role::Admin, "changing roles")?;
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let current = Role::from_db(
            &sqlx::query_scalar::<_, String>(
                "SELECT role FROM sanchoris.members WHERE id = $1 FOR UPDATE",
            )
            .bind(target_id)
            .fetch_one(tx.conn())
            .await?,
        );
        if current != Role::Member || role != Role::Member {
            scope.require(Role::Owner, "appointing or changing owners and admins")?;
        }
        if current == Role::Owner && role != Role::Owner && owner_count(&mut tx).await? <= 1 {
            return Err(conflict("the tenant needs at least one owner"));
        }
        sqlx::query("UPDATE sanchoris.members SET role = $2 WHERE id = $1")
            .bind(target_id)
            .bind(role.as_db())
            .execute(tx.conn())
            .await?;
        let name = member_name(&mut tx, target_id).await?;
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
        Member::load(ctx, scope, target_id).await
    }

    async fn remove_member(&self, ctx: &Context<'_>, member_id: ID) -> Result<ID> {
        let target_id = parse_id(&member_id)?;
        let scope = locate(ctx, "members", target_id).await?;
        let actor = scope.require(Role::Admin, "removing members")?;
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let current = Role::from_db(
            &sqlx::query_scalar::<_, String>(
                "SELECT role FROM sanchoris.members WHERE id = $1 FOR UPDATE",
            )
            .bind(target_id)
            .fetch_one(tx.conn())
            .await?,
        );
        if current != Role::Member {
            scope.require(Role::Owner, "removing owners and admins")?;
        }
        if current == Role::Owner && owner_count(&mut tx).await? <= 1 {
            return Err(conflict("the tenant needs at least one owner"));
        }
        let name = member_name(&mut tx, target_id).await?;
        audit_member(
            ctx,
            &mut tx,
            actor,
            "member.remove",
            target("member", target_id, name.clone()),
            &format!("Removed {name}"),
        )
        .await?;
        sqlx::query("DELETE FROM sanchoris.members WHERE id = $1")
            .bind(target_id)
            .execute(tx.conn())
            .await?;
        tx.commit().await?;
        Ok(member_id)
    }

    async fn resume_task(&self, ctx: &Context<'_>, input: ResumeTaskInput) -> Result<Task> {
        let task_id = parse_id(&input.task_id)?;
        let scope = locate(ctx, "tasks", task_id).await?;
        let (member_id, _) = scope.member()?;
        let message = input.message.trim();
        if message.is_empty() {
            return Err(bad_input("the message is empty"));
        }
        let task = Task::load(ctx, scope, task_id).await?;
        if task.status_value()? != TaskStatus::NeedsYou {
            return Err(conflict("only a task that needs you can be resumed"));
        }
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let run_id = latest_run(&mut tx, task_id)
            .await?
            .ok_or_else(|| conflict("the task has no agent session yet"))?;
        let name = member_name(&mut tx, member_id).await?;
        record_run_event(
            &mut tx,
            run_id,
            None,
            RunEventType::HumanReply,
            &json!({ "member": name, "text": message }),
        )
        .await?;
        sqlx::query("UPDATE sanchoris.tasks SET snoozed_until = NULL WHERE id = $1")
            .bind(task_id)
            .execute(tx.conn())
            .await?;
        audit_member(
            ctx,
            &mut tx,
            member_id,
            "task.resume",
            target("task", task_id, task.row.title.clone()),
            "Resumed the task with a message",
        )
        .await?;
        tx.commit().await?;
        Task::load(ctx, scope, task_id).await
    }

    async fn snooze_task(&self, ctx: &Context<'_>, task_id: ID, until: Timestamp) -> Result<Task> {
        let task_id = parse_id(&task_id)?;
        let scope = locate(ctx, "tasks", task_id).await?;
        let (member_id, _) = scope.member()?;
        if until.0 <= now(ctx) {
            return Err(bad_input("the snooze time must be in the future"));
        }
        let task = Task::load(ctx, scope, task_id).await?;
        let snoozable = task.row.status == "NEEDS_YOU"
            || (task.row.status == "WAITING"
                && task.row.wait_reason.as_deref() == Some("usage_limit"));
        if !snoozable {
            return Err(conflict(
                "only a task that needs you or waits for usage can be snoozed",
            ));
        }
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        sqlx::query("UPDATE sanchoris.tasks SET snoozed_until = $2, updated_at = $3 WHERE id = $1")
            .bind(task_id)
            .bind(until.0)
            .bind(now(ctx))
            .execute(tx.conn())
            .await?;
        audit_member(
            ctx,
            &mut tx,
            member_id,
            "task.snooze",
            target("task", task_id, task.row.title.clone()),
            &format!("Snoozed the task until {}", until.0.to_rfc3339()),
        )
        .await?;
        tx.commit().await?;
        Task::load(ctx, scope, task_id).await
    }

    async fn stop_task(&self, ctx: &Context<'_>, task_id: ID) -> Result<Task> {
        let task_id = parse_id(&task_id)?;
        let scope = locate(ctx, "tasks", task_id).await?;
        let (member_id, _) = scope.member()?;
        let task = Task::load(ctx, scope, task_id).await?;
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let name = member_name(&mut tx, member_id).await?;
        if !abort_task(
            &mut tx,
            task_id,
            &format!("{name} stopped the task"),
            now(ctx),
        )
        .await?
        {
            return Err(conflict("the task has already finished"));
        }
        audit_member(
            ctx,
            &mut tx,
            member_id,
            "task.stop",
            target("task", task_id, task.row.title.clone()),
            "Stopped the task",
        )
        .await?;
        tx.commit().await?;
        Task::load(ctx, scope, task_id).await
    }

    async fn stop_run(&self, ctx: &Context<'_>, run_id: ID) -> Result<Run> {
        let run_id = parse_id(&run_id)?;
        let scope = locate(ctx, "runs", run_id).await?;
        let (member_id, _) = scope.member()?;
        let run = Run::load(ctx, scope, run_id).await?;
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let name = member_name(&mut tx, member_id).await?;
        if !stop_run(
            &mut tx,
            run_id,
            &format!("{name} stopped the session"),
            now(ctx),
        )
        .await?
        {
            return Err(conflict("the session has already ended"));
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
        Run::load(ctx, scope, run_id).await
    }

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
                let accepts = Trigger::all(ctx, scope)
                    .await?
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
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let user = member_name(&mut tx, member_id).await?;
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
        let inbound_id = record_inbound_event(&mut tx, "web", &key, &event)
            .await?
            .id();
        let received_at = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
            "SELECT received_at FROM sanchoris.inbound_events WHERE id = $1",
        )
        .bind(inbound_id)
        .fetch_one(tx.conn())
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
        state(ctx).deps.chat.dispatch(scope.tenant_id).await?;
        let task = match existing {
            Some(task_id) => Task::load(ctx, scope, task_id).await?,
            None => {
                let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
                let started = sqlx::query_as::<_, (Option<Uuid>, Option<String>)>(
                    "SELECT t.id, e.reason FROM sanchoris.trigger_events e \
                     LEFT JOIN sanchoris.tasks t ON t.trigger_event_id = e.id \
                     WHERE e.key = $1 ORDER BY t.id NULLS LAST LIMIT 1",
                )
                .bind(&key)
                .fetch_optional(tx.conn())
                .await?;
                drop(tx);
                match started {
                    Some((Some(task_id), _)) => Task::load(ctx, scope, task_id).await?,
                    Some((None, reason)) => {
                        return Err(conflict(format!(
                            "the chat trigger did not start a task: {}",
                            reason.unwrap_or_else(|| "the event is not evaluated yet".into())
                        )));
                    }
                    None => return Err(conflict("no chat trigger received the message")),
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
        let scope = locate(ctx, "triggers", trigger_id).await?;
        let member_id = scope.require(Role::Admin, "enabling and disabling triggers")?;
        let trigger = Trigger::load(ctx, scope, trigger_id).await?;
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        sqlx::query("UPDATE sanchoris.triggers SET state = $2 WHERE id = $1")
            .bind(trigger_id)
            .bind(state.as_db())
            .execute(tx.conn())
            .await?;
        if state == TriggerState::Disabled {
            sqlx::query("DELETE FROM sanchoris.due_jobs WHERE kind = $1 AND ref_id = $2")
                .bind(crate::trigger::DUE_TRIGGER_SCHEDULE)
                .bind(trigger_id)
                .execute(tx.conn())
                .await?;
        }
        let (action, verb) = match state {
            TriggerState::Enabled => ("trigger.enable", "Enabled"),
            TriggerState::Disabled => ("trigger.disable", "Disabled"),
        };
        audit_member(
            ctx,
            &mut tx,
            member_id,
            action,
            target("trigger", trigger_id, trigger.row.name.clone()),
            &format!("{verb} the trigger {}", trigger.row.name),
        )
        .await?;
        tx.commit().await?;
        Trigger::load(ctx, scope, trigger_id).await
    }

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
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let change_id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO sanchoris.change_requests \
             (tenant_id, config_id, pr_number, title, state, changed_files, changed_definitions, requested_by_member_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, 'open', $5, $6, $7, $8, $8) RETURNING id",
        )
        .bind(scope.tenant_id)
        .bind(config.row.id)
        .bind(number)
        .bind(&title)
        .bind(json!([format!(
            "{}{}",
            crate::config::root_prefix(&config.row.root_path),
            sanchoris_config::CONFIG_FILE
        )]))
        .bind(json!([{ "kind": "trigger", "name": trigger_name }]))
        .bind(member_id)
        .bind(now(ctx))
        .fetch_one(tx.conn())
        .await?;
        audit_member(
            ctx,
            &mut tx,
            member_id,
            "change_request.open",
            target("change_request", change_id, format!("#{number}")),
            &title,
        )
        .await?;
        tx.commit().await?;
        ChangeRequest::by_id(ctx, scope, change_id).await
    }

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
        let problems = state(ctx)
            .deps
            .config_repository
            .check(scope.tenant_id, &location)
            .await?;
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        if !problems.is_empty() {
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
                    "The config at {repository} {} has {} problems",
                    input.root,
                    problems.len()
                ),
            )
            .await?;
            tx.commit().await?;
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
        let at = now(ctx);
        let config_id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO sanchoris.configs (tenant_id, repository, root_path, branch, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $5) ON CONFLICT (tenant_id, repository, root_path) DO NOTHING RETURNING id",
        )
        .bind(scope.tenant_id)
        .bind(&repository)
        .bind(&input.root)
        .bind(&branch)
        .bind(at)
        .fetch_optional(tx.conn())
        .await?
        .ok_or_else(|| conflict(format!("{repository} {} is already a config", input.root)))?;
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
        state(ctx)
            .deps
            .config_sync
            .request_sync(scope.tenant_id, config_id)
            .await?;
        Ok(AddConfigPayload {
            config: Some(Config::load(ctx, scope, config_id).await?),
            errors: Vec::new(),
        })
    }

    async fn sync_config(&self, ctx: &Context<'_>, config_id: ID) -> Result<Config> {
        let config_id = parse_id(&config_id)?;
        let scope = locate(ctx, "configs", config_id).await?;
        let member_id = scope.require(Role::Admin, "syncing configs")?;
        let config = Config::load(ctx, scope, config_id).await?;
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        audit_member(
            ctx,
            &mut tx,
            member_id,
            "config.sync",
            target("config", config_id, config.row.repository.clone()),
            "Requested a config sync",
        )
        .await?;
        tx.commit().await?;
        state(ctx)
            .deps
            .config_sync
            .request_sync(scope.tenant_id, config_id)
            .await?;
        Config::load(ctx, scope, config_id).await
    }

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
        let (provider, kind) = match input.provider {
            ApiKeyProvider::Anthropic => ("claude", "api_key"),
            ApiKeyProvider::Openai => ("codex", "api_key"),
            ApiKeyProvider::Bedrock => ("claude", "bedrock"),
        };
        let encrypted = state(ctx)
            .deps
            .encryptor
            .encrypt(scope.tenant_id, key.as_bytes())
            .await?;
        let at = now(ctx);
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let credential_id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO sanchoris.credentials \
             (tenant_id, scope, member_id, name, provider, kind, account, status, encrypted_value, kms_key_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'connected', $8, $9, $10, $10) \
             ON CONFLICT (tenant_id, name) DO NOTHING RETURNING id",
        )
        .bind(scope.tenant_id)
        .bind(input.scope.as_db())
        .bind((input.scope == CredentialScope::Personal).then_some(member_id))
        .bind(&name)
        .bind(provider)
        .bind(kind)
        .bind(mask_key(key))
        .bind(&encrypted.ciphertext)
        .bind(&encrypted.key_id)
        .bind(at)
        .fetch_optional(tx.conn())
        .await?
        .ok_or_else(|| conflict(format!("a credential named {name} exists")))?;
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
        Credential::by_id(ctx, scope, credential_id).await
    }

    async fn remove_credential(&self, ctx: &Context<'_>, credential_id: ID) -> Result<ID> {
        let id = parse_id(&credential_id)?;
        let scope = locate(ctx, "credentials", id).await?;
        let (member_id, role) = scope.member()?;
        let credential = Credential::by_id(ctx, scope, id).await?;
        let own = credential.row.member_id == Some(member_id);
        if !own && !role.at_least(Role::Admin) {
            return Err(forbidden(
                "removing this credential needs the admin role or higher",
            ));
        }
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        sqlx::query("DELETE FROM sanchoris.credentials WHERE id = $1")
            .bind(id)
            .execute(tx.conn())
            .await?;
        audit_member(
            ctx,
            &mut tx,
            member_id,
            "credential.delete",
            target("credential", id, credential.row.name.clone()),
            &format!("Removed the credential {}", credential.row.name),
        )
        .await?;
        tx.commit().await?;
        Ok(credential_id)
    }

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
        if tenant.row.plan_id == plan.0.id {
            return Err(bad_input(format!(
                "the tenant is already on {}",
                plan.0.name
            )));
        }
        let change = state(ctx)
            .deps
            .checkout
            .change_plan(scope.tenant_id, &plan.0.id)
            .await?;
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let checkout_url = match change {
            PlanChange::Applied => {
                sqlx::query(
                    "UPDATE sanchoris.tenants SET plan_id = $2, updated_at = $3 WHERE id = $1",
                )
                .bind(scope.tenant_id)
                .bind(&plan.0.id)
                .bind(now(ctx))
                .execute(tx.conn())
                .await?;
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
                Some(url)
            }
        };
        tx.commit().await?;
        Ok(ChangePlanPayload {
            tenant: Tenant::load(ctx, scope).await?,
            checkout_url,
        })
    }

    async fn open_billing_portal(
        &self,
        ctx: &Context<'_>,
        tenant_id: ID,
    ) -> Result<RedirectPayload> {
        let scope = tenant_scope(ctx, &tenant_id).await?;
        let member_id = scope.require(Role::Owner, "opening the billing portal")?;
        let url = state(ctx)
            .deps
            .checkout
            .billing_portal(scope.tenant_id)
            .await?;
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
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
        Ok(RedirectPayload { url })
    }

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
            .await?;
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
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
        Ok(RedirectPayload { url })
    }

    async fn suspend_tenant(
        &self,
        ctx: &Context<'_>,
        tenant_id: ID,
        reason: String,
    ) -> Result<Tenant> {
        let tenant_id = parse_id(&tenant_id)?;
        if !is_operator(ctx).await? {
            return Err(forbidden("only operators can suspend tenants"));
        }
        let reason = reason.trim().to_string();
        if reason.is_empty() {
            return Err(bad_input("suspending a tenant needs a reason"));
        }
        let mut tx = tenant_tx(ctx, tenant_id).await?;
        let name = sqlx::query_scalar::<_, String>(
            "UPDATE sanchoris.tenants SET status = 'suspended', suspended_reason = $2, updated_at = $3 \
             WHERE id = $1 AND status = 'active' RETURNING name",
        )
        .bind(tenant_id)
        .bind(&reason)
        .bind(now(ctx))
        .fetch_optional(tx.conn())
        .await?
        .ok_or_else(|| conflict("the tenant does not exist or is already suspended"))?;
        audit_operator(
            ctx,
            &mut tx,
            "tenant.suspend",
            target("tenant", tenant_id, name),
            &format!("Suspended the tenant: {reason}"),
        )
        .await?;
        tx.commit().await?;
        Tenant::load(ctx, scope_for_tenant(ctx, tenant_id).await?).await
    }

    async fn unsuspend_tenant(&self, ctx: &Context<'_>, tenant_id: ID) -> Result<Tenant> {
        let tenant_id = parse_id(&tenant_id)?;
        if !is_operator(ctx).await? {
            return Err(forbidden("only operators can unsuspend tenants"));
        }
        let mut tx = tenant_tx(ctx, tenant_id).await?;
        let name = sqlx::query_scalar::<_, String>(
            "UPDATE sanchoris.tenants SET status = 'active', suspended_reason = NULL, updated_at = $2 \
             WHERE id = $1 AND status = 'suspended' RETURNING name",
        )
        .bind(tenant_id)
        .bind(now(ctx))
        .fetch_optional(tx.conn())
        .await?
        .ok_or_else(|| conflict("the tenant does not exist or is not suspended"))?;
        audit_operator(
            ctx,
            &mut tx,
            "tenant.unsuspend",
            target("tenant", tenant_id, name),
            "Unsuspended the tenant",
        )
        .await?;
        tx.commit().await?;
        Tenant::load(ctx, scope_for_tenant(ctx, tenant_id).await?).await
    }
}
