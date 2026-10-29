use std::future::Future;
use std::net::IpAddr;

use async_graphql::{Context, Enum, Error, Result};
use aws_sdk_dynamodb::types::AttributeValue;
use chrono::{DateTime, Utc};
use sanchoris_core::audit::{AuditEntry, record_audit};
use sanchoris_core::db::{self, DbError, Store, TenantTx};
use sanchoris_core::item::{Entity, Key, format_time};
use sanchoris_core::items::{self, Member, Ref, Tenant, User};
use sanchoris_core::keys;
use sanchoris_core::model::{ActorKind, AuditOrigin};
use sanchoris_core::store::Update;
use sanchoris_core::tenants::is_deleting;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::error::{conflict, forbidden, not_found, unauthenticated};
use crate::ports::GraphqlDeps;

const WRITE_ATTEMPTS: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub cognito_subject: String,
    pub email: String,
    pub privileged: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

pub(crate) struct State {
    pub store: Store,
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

    pub fn rank(self) -> u8 {
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

pub(crate) fn store<'a>(ctx: &Context<'a>) -> &'a Store {
    &state(ctx).store
}

pub(crate) fn tx(ctx: &Context<'_>, tenant_id: Uuid) -> TenantTx {
    store(ctx).tx(tenant_id)
}

pub(crate) fn user_tx(ctx: &Context<'_>) -> TenantTx {
    store(ctx).tx(Uuid::nil())
}

pub(crate) fn auth<'a>(ctx: &Context<'a>) -> Result<&'a AuthUser> {
    ctx.data_opt::<AuthUser>().ok_or_else(unauthenticated)
}

pub(crate) fn now(ctx: &Context<'_>) -> DateTime<Utc> {
    state(ctx).deps.clock.now()
}

pub(crate) async fn viewer_user(ctx: &Context<'_>) -> Result<User> {
    let user_id = auth(ctx)?.user_id;
    store(ctx)
        .get::<User>(&keys::user_key(user_id))
        .await?
        .ok_or_else(|| not_found(format!("user {user_id}")))
}

pub(crate) async fn is_operator(ctx: &Context<'_>) -> Result<bool> {
    Ok(viewer_user(ctx).await?.is_operator)
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

async fn membership_of(
    ctx: &Context<'_>,
    item: &items::Membership,
) -> Result<(Tenant, Membership)> {
    let mut tx = tx(ctx, item.tenant_id);
    let member = tx
        .require::<Member>(&keys::member_key(item.tenant_id, item.member_id))
        .await?;
    let tenant = tx
        .get::<Tenant>(&keys::tenant_key(item.tenant_id))
        .await?
        .ok_or_else(|| not_found(format!("tenant {}", item.tenant_id)))?;
    Ok((
        tenant,
        Membership {
            member_id: member.id,
            tenant_id: item.tenant_id,
            role: Role::from_db(&member.role),
        },
    ))
}

pub(crate) fn being_deleted(tenant: &Tenant) -> Error {
    conflict(format!("the tenant {} is being deleted", tenant.slug))
}

async fn member_tenants(ctx: &Context<'_>) -> Result<Vec<(Tenant, Membership)>> {
    let user_id = auth(ctx)?.user_id;
    let items = store(ctx)
        .query::<items::Membership>(&keys::user_key(user_id).pk, "MEMBER#")
        .await?;
    let mut found = Vec::new();
    for item in &items {
        found.push(membership_of(ctx, item).await?);
    }
    found.sort_by(|a, b| (&a.0.name, a.1.tenant_id).cmp(&(&b.0.name, b.1.tenant_id)));
    Ok(found)
}

pub(crate) async fn memberships(ctx: &Context<'_>) -> Result<Vec<Membership>> {
    Ok(member_tenants(ctx)
        .await?
        .into_iter()
        .filter(|(tenant, _)| !is_deleting(tenant))
        .map(|(_, membership)| membership)
        .collect())
}

pub(crate) async fn membership_in(
    ctx: &Context<'_>,
    tenant_id: Uuid,
) -> Result<Option<Membership>> {
    let user_id = auth(ctx)?.user_id;
    let Some(item) = store(ctx)
        .get::<items::Membership>(&keys::membership_key(user_id, tenant_id))
        .await?
    else {
        return Ok(None);
    };
    let (tenant, membership) = membership_of(ctx, &item).await?;
    if is_deleting(&tenant) {
        return Err(being_deleted(&tenant));
    }
    Ok(Some(membership))
}

pub(crate) async fn scope_for_tenant(ctx: &Context<'_>, tenant_id: Uuid) -> Result<Scope> {
    if let Some(membership) = membership_in(ctx, tenant_id).await? {
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
    membership_in(ctx, tenant_id)
        .await?
        .map(|m| m.scope())
        .ok_or_else(|| not_found(format!("tenant {tenant_id}")))
}

pub(crate) async fn locate_item<E: Entity>(
    ctx: &Context<'_>,
    what: &str,
    id: Uuid,
    key: impl Fn(Uuid) -> Key,
) -> Result<(Scope, E)> {
    for (tenant, membership) in member_tenants(ctx).await? {
        if let Some(item) = tx(ctx, membership.tenant_id)
            .get::<E>(&key(membership.tenant_id))
            .await?
        {
            if is_deleting(&tenant) {
                return Err(being_deleted(&tenant));
            }
            return Ok((membership.scope(), item));
        }
    }
    Err(not_found(format!("{what} {id}")))
}

pub(crate) async fn locate_ref(
    ctx: &Context<'_>,
    what: &str,
    id: Uuid,
    kind: &str,
) -> Result<(Scope, Ref)> {
    for (tenant, membership) in member_tenants(ctx).await? {
        let mut tx = tx(ctx, membership.tenant_id);
        if let Some(found) = db::locate(&mut tx, id, kind).await? {
            if is_deleting(&tenant) {
                return Err(being_deleted(&tenant));
            }
            return Ok((membership.scope(), found));
        }
    }
    Err(not_found(format!("{what} {id}")))
}

pub(crate) enum Failure {
    Db(DbError),
    Api(Error),
}

impl From<DbError> for Failure {
    fn from(error: DbError) -> Self {
        Self::Db(error)
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Api(error)
    }
}

pub(crate) type Written<T> = std::result::Result<T, Failure>;

pub(crate) async fn write<T, F, Fut>(mut operation: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Written<T>>,
{
    let mut attempt = 0;
    loop {
        attempt += 1;
        match operation().await {
            Ok(value) => return Ok(value),
            Err(Failure::Db(DbError::Conflict)) if attempt < WRITE_ATTEMPTS => {}
            Err(Failure::Db(DbError::Conflict)) => {
                return Err(conflict(
                    "the change conflicted with concurrent changes; try again",
                ));
            }
            Err(Failure::Db(error)) => return Err(error.into()),
            Err(Failure::Api(error)) => return Err(error),
        }
    }
}

pub(crate) async fn serialize_tenant(tx: &mut TenantTx) -> Result<()> {
    let key = keys::tenant_key(tx.tenant_id());
    let item = tx
        .get_item(&key)
        .await?
        .ok_or_else(|| not_found(format!("tenant {}", tx.tenant_id())))?;
    let version = item
        .get(sanchoris_core::item::VERSION)
        .cloned()
        .ok_or_else(|| Error::new("the tenant has no version"))?;
    tx.update(
        &key,
        Update::new().require(sanchoris_core::item::VERSION, version),
    )?;
    Ok(())
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
    tx.update(
        &keys::member_key(tx.tenant_id(), member_id),
        Update::new().set("last_active_at", AttributeValue::S(format_time(at))),
    )?;
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
    .await?;
    Ok(())
}
