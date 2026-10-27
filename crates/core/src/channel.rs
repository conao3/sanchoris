use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::db::{DbError, Recorded, TenantTx};
use crate::github::{GithubEventError, normalize_github_event};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxType {
    Start,
    UserMessage,
    Exec,
    Interrupt,
    Shutdown,
    ChatPost,
}

impl OutboxType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::UserMessage => "user_message",
            Self::Exec => "exec",
            Self::Interrupt => "interrupt",
            Self::Shutdown => "shutdown",
            Self::ChatPost => "chat_post",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEventType {
    Ready,
    TurnEnd,
    RateLimit,
    Heartbeat,
    SessionRecord,
    ExecResult,
    HumanReply,
}

impl RunEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::TurnEnd => "turn_end",
            Self::RateLimit => "rate_limit",
            Self::Heartbeat => "heartbeat",
            Self::SessionRecord => "session_record",
            Self::ExecResult => "exec_result",
            Self::HumanReply => "human_reply",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "ready" => Self::Ready,
            "turn_end" => Self::TurnEnd,
            "rate_limit" => Self::RateLimit,
            "heartbeat" => Self::Heartbeat,
            "session_record" => Self::SessionRecord,
            "exec_result" => Self::ExecResult,
            "human_reply" => Self::HumanReply,
            _ => return None,
        })
    }
}

pub async fn enqueue_outbox(
    tx: &mut TenantTx,
    run_id: Uuid,
    kind: OutboxType,
    payload: &Value,
    dedupe_key: &str,
) -> Result<bool, DbError> {
    let tenant_id = tx.tenant_id();
    let inserted = sqlx::query(
        "INSERT INTO sanchoris.run_outbox (tenant_id, run_id, type, payload, dedupe_key) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (run_id, dedupe_key) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(run_id)
    .bind(kind.as_str())
    .bind(payload)
    .bind(dedupe_key)
    .execute(tx.conn())
    .await?;
    Ok(inserted.rows_affected() == 1)
}

pub async fn enqueue_user_message(
    tx: &mut TenantTx,
    run_id: Uuid,
    text: &str,
    dedupe_key: &str,
) -> Result<bool, DbError> {
    enqueue_outbox(
        tx,
        run_id,
        OutboxType::UserMessage,
        &json!({ "text": text }),
        dedupe_key,
    )
    .await
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutboxRow {
    pub id: Uuid,
    pub run_id: Uuid,
    pub seq: i64,
    pub kind: String,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

pub async fn undelivered_outbox(
    conn: &mut PgConnection,
    run_id: Uuid,
) -> Result<Vec<OutboxRow>, DbError> {
    let rows = sqlx::query(
        "SELECT id, run_id, seq, type, payload, created_at FROM sanchoris.run_outbox \
         WHERE run_id = $1 AND delivered_at IS NULL ORDER BY seq",
    )
    .bind(run_id)
    .fetch_all(conn)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(OutboxRow {
                id: row.try_get("id")?,
                run_id: row.try_get("run_id")?,
                seq: row.try_get("seq")?,
                kind: row.try_get("type")?,
                payload: row.try_get("payload")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect()
}

pub async fn mark_outbox_delivered(
    conn: &mut PgConnection,
    ids: &[Uuid],
    at: DateTime<Utc>,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE sanchoris.run_outbox SET delivered_at = $2 WHERE id = ANY($1) AND delivered_at IS NULL",
    )
    .bind(ids)
    .bind(at)
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn record_run_event(
    tx: &mut TenantTx,
    run_id: Uuid,
    seq: Option<i64>,
    kind: RunEventType,
    payload: &Value,
) -> Result<Recorded, DbError> {
    let tenant_id = tx.tenant_id();
    let inserted = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO sanchoris.run_events (tenant_id, run_id, seq, type, payload) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (run_id, seq) DO NOTHING \
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(run_id)
    .bind(seq)
    .bind(kind.as_str())
    .bind(payload)
    .fetch_optional(tx.conn())
    .await?;
    if let Some(id) = inserted {
        return Ok(Recorded::New(id));
    }
    let existing = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM sanchoris.run_events WHERE run_id = $1 AND seq = $2",
    )
    .bind(run_id)
    .bind(seq)
    .fetch_optional(tx.conn())
    .await?
    .ok_or(DbError::ConflictNotVisible("run event"))?;
    Ok(Recorded::Duplicate(existing))
}

pub async fn record_inbound_event(
    tx: &mut TenantTx,
    source: &str,
    key: &str,
    event: &Value,
) -> Result<Recorded, DbError> {
    let tenant_id = tx.tenant_id();
    let inserted = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO sanchoris.inbound_events (tenant_id, source, key, event) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (tenant_id, source, key) DO NOTHING \
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(source)
    .bind(key)
    .bind(event)
    .fetch_optional(tx.conn())
    .await?;
    if let Some(id) = inserted {
        return Ok(Recorded::New(id));
    }
    let existing = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM sanchoris.inbound_events WHERE tenant_id = $1 AND source = $2 AND key = $3",
    )
    .bind(tenant_id)
    .bind(source)
    .bind(key)
    .fetch_optional(tx.conn())
    .await?
    .ok_or(DbError::ConflictNotVisible("inbound event"))?;
    Ok(Recorded::Duplicate(existing))
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Event(#[from] GithubEventError),
    #[error("no connected tenant has GitHub installation {0}")]
    UnknownInstallation(i64),
}

pub async fn tenant_for_installation(
    worker: &mut PgConnection,
    installation_id: i64,
) -> Result<Option<Uuid>, DbError> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT tenant_id FROM sanchoris.integrations \
         WHERE github_installation_id = $1 AND status = 'connected'",
    )
    .bind(installation_id)
    .fetch_optional(worker)
    .await?)
}

pub async fn ingest_github_delivery(
    pool: &sqlx::PgPool,
    delivery_id: &str,
    event_name: &str,
    payload: &Value,
) -> Result<Option<Recorded>, IngestError> {
    let Some(normalized) = normalize_github_event(event_name, payload)? else {
        return Ok(None);
    };
    let installation_id = normalized.detail["installation"]["id"]
        .as_i64()
        .expect("normalized GitHub events carry the installation id");
    let mut conn = pool.acquire().await.map_err(DbError::from)?;
    let tenant_id = tenant_for_installation(&mut conn, installation_id)
        .await?
        .ok_or(IngestError::UnknownInstallation(installation_id))?;
    drop(conn);
    let mut tx = TenantTx::begin(pool, tenant_id).await?;
    let recorded =
        record_inbound_event(&mut tx, "github", delivery_id, &normalized.to_value()).await?;
    tx.commit().await?;
    Ok(Some(recorded))
}
