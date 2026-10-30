use async_graphql::{Context, ID, InputObject, Object, Result};
use sanchoris_core::db::{self, REF_RUN, REF_TRIGGER};
use sanchoris_core::item::format_time;
use sanchoris_core::items::{
    AuditEvent as AuditItem, Member, Run, Task as TaskItem, Trigger, User,
};
use sanchoris_core::keys;
use sanchoris_core::store::{Listing, SortKey};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::context::{Scope, store, tx};
use crate::enums::{ActorKind, AuditOrigin};
use crate::error::parse_id;
use crate::scalars::{Timestamp, ts};
use crate::task::Task;

#[derive(InputObject, Default, Clone)]
pub(crate) struct AuditFilter {
    pub actor_kind: Option<ActorKind>,
    pub member_id: Option<ID>,
    pub action: Option<String>,
    pub since: Option<Timestamp>,
}

pub(crate) struct AuditEvent {
    scope: Scope,
    row: AuditItem,
    actor_name: Option<String>,
}

async fn actor_name(ctx: &Context<'_>, scope: Scope, row: &AuditItem) -> Result<Option<String>> {
    let Ok(id) = row.actor_ref.parse::<Uuid>() else {
        return Ok(None);
    };
    let mut tx = tx(ctx, scope.tenant_id);
    let user_name = |user: Option<User>| user.map(|user| user.display_name);
    Ok(match row.actor_kind.as_str() {
        "member" => match tx
            .get::<Member>(&keys::member_key(scope.tenant_id, id))
            .await?
        {
            Some(member) => user_name(
                store(ctx)
                    .get::<User>(&keys::user_key(member.user_id))
                    .await?,
            ),
            None => None,
        },
        "operator" => user_name(store(ctx).get::<User>(&keys::user_key(id)).await?),
        "trigger" => match db::locate(&mut tx, id, REF_TRIGGER).await? {
            Some(location) => tx.get::<Trigger>(&location.target).await?.map(|t| t.name),
            None => None,
        },
        "agent" => match db::locate(&mut tx, id, REF_RUN).await? {
            Some(location) => match tx.get::<Run>(&location.target).await? {
                Some(run) => tx
                    .get::<TaskItem>(&keys::task_key(scope.tenant_id, run.task_id))
                    .await?
                    .map(|task| format!("run of {}", task.title)),
                None => None,
            },
            None => None,
        },
        _ => None,
    })
}

impl AuditEvent {
    pub async fn list(
        ctx: &Context<'_>,
        scope: Scope,
        filter: AuditFilter,
        limit: Option<i32>,
    ) -> Result<Vec<Self>> {
        let member_id = filter.member_id.as_ref().map(parse_id).transpose()?;
        let mut listing = Listing::audit(keys::tenant_pk(scope.tenant_id)).descending();
        if let Some(since) = filter.since {
            listing = listing.sort(SortKey::AtLeast(format_time(since.0)));
        }
        if let Some(limit) = limit {
            listing = listing.limit(usize::try_from(limit)?);
        }
        let actor_kind = filter.actor_kind.map(ActorKind::as_db);
        let rows = tx(ctx, scope.tenant_id)
            .list::<AuditItem>(&listing, |event| {
                actor_kind.is_none_or(|kind| event.actor_kind == kind)
                    && member_id.is_none_or(|id| {
                        event.actor_kind == "member" && event.actor_ref == id.to_string()
                    })
                    && filter
                        .action
                        .as_deref()
                        .is_none_or(|action| event.action == action)
            })
            .await?;
        let mut events = Vec::new();
        for row in rows {
            let actor_name = actor_name(ctx, scope, &row).await?;
            events.push(AuditEvent {
                scope,
                row,
                actor_name,
            });
        }
        Ok(events)
    }

    pub fn json_line(&self) -> Value {
        json!({
            "id": self.row.id,
            "at": self.row.at,
            "actorKind": self.row.actor_kind,
            "actorRef": self.row.actor_ref,
            "actorName": self.actor_name(),
            "action": self.row.action,
            "targetKind": self.row.target_kind,
            "targetRef": self.row.target_ref,
            "summary": self.row.summary,
            "ip": self.row.source_ip,
            "metadata": self.row.metadata,
        })
    }

    fn actor_name(&self) -> String {
        self.actor_name
            .clone()
            .unwrap_or_else(|| self.row.actor_ref.clone())
    }
}

#[Object]
impl AuditEvent {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn at(&self) -> Timestamp {
        ts(self.row.at)
    }

    async fn actor_kind(&self) -> Result<ActorKind> {
        ActorKind::from_db(&self.row.actor_kind)
    }

    /// The member name, the trigger name, or the run and task of an agent.
    #[graphql(name = "actorName")]
    async fn actor_name_field(&self) -> String {
        self.actor_name()
    }

    async fn action(&self) -> &str {
        &self.row.action
    }

    async fn target(&self) -> String {
        match self.row.metadata["target_name"].as_str() {
            Some(name) => format!("{} {name}", self.row.target_kind),
            None => format!("{} {}", self.row.target_kind, self.row.target_ref),
        }
    }

    async fn target_task(&self, ctx: &Context<'_>) -> Result<Option<Task>> {
        if self.row.target_kind != "task" {
            return Ok(None);
        }
        match self.row.target_ref.parse::<Uuid>() {
            Ok(task_id) => Task::find(ctx, self.scope, task_id).await,
            Err(_) => Ok(None),
        }
    }

    async fn summary(&self) -> Option<&str> {
        Some(self.row.summary.as_str())
    }

    async fn origin(&self) -> Result<AuditOrigin> {
        AuditOrigin::from_db(self.row.metadata["origin"].as_str().ok_or_else(|| {
            async_graphql::Error::new(format!("audit event {} has no origin", self.row.id))
        })?)
    }

    /// The client IP for WEB.
    async fn ip(&self) -> Option<&str> {
        self.row.source_ip.as_deref()
    }
}
