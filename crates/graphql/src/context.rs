use std::net::IpAddr;

use async_graphql::{Context, Enum, Error, Result};
use chrono::{DateTime, Utc};
use sanchoris_core::audit::{AuditEntry, record_audit};
use sanchoris_core::db::{TenantTx, UserTx};
use sanchoris_core::model::{ActorKind, AuditOrigin};
use serde_json::{Map, Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{forbidden, not_found, unauthenticated};
use crate::ports::GraphqlDeps;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub cognito_subject: String,
    pub email: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

pub(crate) struct State {
    pub pool: PgPool,
    pub deps: GraphqlDeps,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner,
    Admin,
    Member,
}

impl Role {
    pub fn from_db(text: &str) -> Self {
        match text {
            "owner" => Self::Owner,
            "admin" => Self::Admin,
            "member" => Self::Member,
            other => panic!("members.role has unknown value {other}"),
        }
    }

    pub fn as_db(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Owner => 3,
            Self::Admin => 2,
            Self::Member => 1,
        }
    }

    pub fn at_least(self, required: Role) -> bool {
        self.rank() >= required.rank()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    Member { member_id: Uuid, role: Role },
    Operator,
    Invitee,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Scope {
    pub tenant_id: Uuid,
    pub access: Access,
}

impl Scope {
    pub fn member(&self) -> Result<(Uuid, Role)> {
        match self.access {
            Access::Member { member_id, role } => Ok((member_id, role)),
            Access::Operator | Access::Invitee => {
                Err(forbidden("only members of the tenant can read this"))
            }
        }
    }

    pub fn require(&self, role: Role, action: &str) -> Result<Uuid> {
        let (member_id, actual) = self.member()?;
        if !actual.at_least(role) {
            return Err(forbidden(format!(
                "{action} needs the {} role or higher",
                role.as_db()
            )));
        }
        Ok(member_id)
    }

    pub fn operator_or_member(&self) -> Result<()> {
        match self.access {
            Access::Member { .. } | Access::Operator => Ok(()),
            Access::Invitee => Err(forbidden("only members of the tenant can read this")),
        }
    }
}

pub(crate) fn state<'a>(ctx: &Context<'a>) -> &'a State {
    ctx.data_unchecked::<State>()
}

pub(crate) fn auth<'a>(ctx: &Context<'a>) -> Result<&'a AuthUser> {
    ctx.data_opt::<AuthUser>().ok_or_else(unauthenticated)
}

pub(crate) fn now(ctx: &Context<'_>) -> DateTime<Utc> {
    state(ctx).deps.clock.now()
}

pub(crate) async fn tenant_tx(ctx: &Context<'_>, tenant_id: Uuid) -> Result<TenantTx> {
    Ok(TenantTx::begin(&state(ctx).pool, tenant_id).await?)
}

pub(crate) async fn user_tx(ctx: &Context<'_>) -> Result<UserTx> {
    let user = auth(ctx)?;
    Ok(UserTx::begin(&state(ctx).pool, user.user_id).await?)
}

pub(crate) async fn is_operator(ctx: &Context<'_>) -> Result<bool> {
    let mut tx = user_tx(ctx).await?;
    let user_id = tx.user_id();
    sqlx::query_scalar::<_, bool>("SELECT is_operator FROM sanchoris.users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(tx.conn())
        .await?
        .ok_or_else(|| not_found(format!("user {user_id}")))
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Membership {
    pub member_id: Uuid,
    pub tenant_id: Uuid,
    pub role: Role,
}

impl Membership {
    pub fn scope(&self) -> Scope {
        Scope {
            tenant_id: self.tenant_id,
            access: Access::Member {
                member_id: self.member_id,
                role: self.role,
            },
        }
    }
}

pub(crate) async fn memberships(ctx: &Context<'_>) -> Result<Vec<Membership>> {
    let mut tx = user_tx(ctx).await?;
    let rows = sqlx::query(
        "SELECT m.id, m.tenant_id, m.role FROM sanchoris.members m \
         JOIN sanchoris.tenants t ON t.id = m.tenant_id \
         WHERE m.user_id = $1 ORDER BY t.name, t.id",
    )
    .bind(tx.user_id())
    .fetch_all(tx.conn())
    .await?;
    rows.iter()
        .map(|row| {
            Ok(Membership {
                member_id: row.try_get("id")?,
                tenant_id: row.try_get("tenant_id")?,
                role: Role::from_db(row.try_get("role")?),
            })
        })
        .collect()
}

pub(crate) async fn scope_for_tenant(ctx: &Context<'_>, tenant_id: Uuid) -> Result<Scope> {
    if let Some(membership) = memberships(ctx)
        .await?
        .into_iter()
        .find(|m| m.tenant_id == tenant_id)
    {
        return Ok(membership.scope());
    }
    if is_operator(ctx).await? {
        return Ok(Scope {
            tenant_id,
            access: Access::Operator,
        });
    }
    Err(not_found(format!("tenant {tenant_id}")))
}

pub(crate) async fn member_scope(ctx: &Context<'_>, tenant_id: Uuid) -> Result<Scope> {
    memberships(ctx)
        .await?
        .into_iter()
        .find(|m| m.tenant_id == tenant_id)
        .map(|m| m.scope())
        .ok_or_else(|| not_found(format!("tenant {tenant_id}")))
}

pub(crate) async fn locate(ctx: &Context<'_>, table: &'static str, id: Uuid) -> Result<Scope> {
    for membership in memberships(ctx).await? {
        let mut tx = tenant_tx(ctx, membership.tenant_id).await?;
        let found = sqlx::query_scalar::<_, bool>(sql!(
            "SELECT EXISTS (SELECT 1 FROM sanchoris.{table} WHERE id = $1)"
        ))
        .bind(id)
        .fetch_one(tx.conn())
        .await?;
        if found {
            return Ok(membership.scope());
        }
    }
    Err(not_found(format!("{} {id}", table.trim_end_matches('s'))))
}

pub(crate) struct AuditTarget<'a> {
    pub kind: &'a str,
    pub id: String,
    pub name: String,
}

pub(crate) async fn audit_member(
    ctx: &Context<'_>,
    tx: &mut TenantTx,
    member_id: Uuid,
    action: &str,
    target: AuditTarget<'_>,
    summary: &str,
) -> Result<()> {
    let at = now(ctx);
    write_audit(
        ctx,
        tx,
        ActorKind::Member,
        &member_id.to_string(),
        action,
        target,
        summary,
        at,
    )
    .await?;
    sqlx::query("UPDATE sanchoris.members SET last_active_at = $2 WHERE id = $1")
        .bind(member_id)
        .bind(at)
        .execute(tx.conn())
        .await?;
    Ok(())
}

pub(crate) async fn audit_operator(
    ctx: &Context<'_>,
    tx: &mut TenantTx,
    action: &str,
    target: AuditTarget<'_>,
    summary: &str,
) -> Result<()> {
    let user = auth(ctx)?;
    write_audit(
        ctx,
        tx,
        ActorKind::Operator,
        &user.user_id.to_string(),
        action,
        target,
        summary,
        now(ctx),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn write_audit(
    ctx: &Context<'_>,
    tx: &mut TenantTx,
    actor_kind: ActorKind,
    actor_ref: &str,
    action: &str,
    target: AuditTarget<'_>,
    summary: &str,
    at: DateTime<Utc>,
) -> Result<()> {
    let mut metadata = Map::new();
    metadata.insert("target_name".into(), Value::String(target.name));
    metadata.insert("user_id".into(), json!(auth(ctx)?.user_id));
    record_audit(
        tx,
        &AuditEntry {
            at,
            actor_kind,
            actor_ref,
            action,
            target_kind: target.kind,
            target_ref: &target.id,
            summary,
            origin: AuditOrigin::Web,
            source_ip: ctx.data_opt::<ClientIp>().map(|ip| ip.0),
            metadata,
        },
    )
    .await
    .map_err(Error::from)?;
    Ok(())
}
