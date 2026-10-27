use async_graphql::{Context, ID, InputObject, Object, Result};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::FromRow;
use uuid::Uuid;

use crate::context::{Scope, tenant_tx};
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

#[derive(Debug, Clone, FromRow)]
pub(crate) struct AuditRow {
    id: Uuid,
    at: DateTime<Utc>,
    actor_kind: String,
    actor_ref: String,
    actor_name: Option<String>,
    action: String,
    target_kind: String,
    target_ref: String,
    summary: String,
    ip: Option<String>,
    metadata: Value,
}

pub(crate) struct AuditEvent {
    scope: Scope,
    row: AuditRow,
}

impl AuditEvent {
    pub async fn list(
        ctx: &Context<'_>,
        scope: Scope,
        filter: AuditFilter,
        limit: Option<i32>,
    ) -> Result<Vec<Self>> {
        let member_id = filter.member_id.as_ref().map(parse_id).transpose()?;
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let rows = sqlx::query_as::<_, AuditRow>(
            "SELECT a.id, a.at, a.actor_kind, a.actor_ref, \
                    CASE a.actor_kind \
                      WHEN 'member' THEN (SELECT u.display_name FROM sanchoris.members m JOIN sanchoris.users u ON u.id = m.user_id \
                                          WHERE m.id::text = a.actor_ref) \
                      WHEN 'operator' THEN (SELECT u.display_name FROM sanchoris.users u WHERE u.id::text = a.actor_ref) \
                      WHEN 'trigger' THEN (SELECT t.name FROM sanchoris.triggers t WHERE t.id::text = a.actor_ref) \
                      WHEN 'agent' THEN (SELECT 'run of ' || t.title FROM sanchoris.runs r JOIN sanchoris.tasks t ON t.id = r.task_id \
                                         WHERE r.id::text = a.actor_ref) \
                    END AS actor_name, \
                    a.action, a.target_kind, a.target_ref, a.summary, host(a.source_ip) AS ip, a.metadata \
             FROM sanchoris.audit_events a \
             WHERE a.tenant_id = $1 \
               AND ($2::text IS NULL OR a.actor_kind = $2) \
               AND ($3::text IS NULL OR (a.actor_kind = 'member' AND a.actor_ref = $3)) \
               AND ($4::text IS NULL OR a.action = $4) \
               AND ($5::timestamptz IS NULL OR a.at >= $5) \
             ORDER BY a.at DESC, a.id DESC LIMIT $6",
        )
        .bind(scope.tenant_id)
        .bind(filter.actor_kind.map(ActorKind::as_db))
        .bind(member_id.map(|id| id.to_string()))
        .bind(filter.action)
        .bind(filter.since.map(|since| since.0))
        .bind(limit.map_or(i64::MAX, i64::from))
        .fetch_all(tx.conn())
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AuditEvent { scope, row })
            .collect())
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
            "ip": self.row.ip,
            "metadata": self.row.metadata,
        })
    }

    fn actor_name(&self) -> String {
        self.row
            .actor_name
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

    async fn ip(&self) -> Option<&str> {
        self.row.ip.as_deref()
    }
}
