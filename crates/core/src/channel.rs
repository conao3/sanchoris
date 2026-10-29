use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::db::{DbError, REF_RUN, Recorded, Store, TenantTx, locate};
use crate::github::{GithubEventError, normalize_github_event};
use crate::items::{
    Guard, InboundEvent, Integration, OutboxItem, Pending, RunChannel, RunEventItem, uniq,
};
use crate::jobs::Job;
use crate::keys;
use crate::receive::{Admission, ReceiveLimited, SOURCE_GITHUB, admit_inbound};
use crate::store::retry;

pub const SEQ_STRIDE: i64 = 1_000_000_000;

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
    Stopping,
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
            Self::Stopping => "stopping",
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
            "stopping" => Self::Stopping,
            _ => return None,
        })
    }
}

async fn channel_of(tx: &mut TenantTx, run_id: Uuid) -> Result<RunChannel, DbError> {
    let key = keys::run_channel_key(tx.tenant_id(), run_id);
    tx.get::<RunChannel>(&key)
        .await?
        .ok_or_else(|| DbError::Missing(format!("run {run_id}")))
}

pub async fn enqueue_outbox(
    tx: &mut TenantTx,
    run_id: Uuid,
    kind: OutboxType,
    payload: &Value,
    dedupe_key: &str,
) -> Result<bool, DbError> {
    let tenant_id = tx.tenant_id();
    let dedupe = keys::outbox_dedupe_key(tenant_id, run_id, dedupe_key);
    if tx.get::<Guard>(&dedupe).await?.is_some() {
        return Ok(false);
    }
    let mut channel = channel_of(tx, run_id).await?;
    let seq = channel.next_outbox_seq;
    channel.next_outbox_seq += 1;
    tx.put(&channel)?;
    let id = Uuid::new_v4();
    tx.insert(&OutboxItem {
        id,
        tenant_id,
        run_id,
        seq,
        kind: kind.as_str().to_string(),
        payload: payload.clone(),
        dedupe_key: dedupe_key.to_string(),
        created_at: Utc::now(),
        delivered_at: None,
    })?;
    tx.insert(&Guard::new(dedupe, id))?;
    tx.emit(Job::DeliverOutbox { tenant_id, run_id });
    if matches!(kind, OutboxType::UserMessage | OutboxType::Exec) {
        tx.emit(Job::Reconcile {
            tenant_id,
            task_id: channel.task_id,
            run_id,
        });
    }
    Ok(true)
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

pub async fn outbox(tx: &mut TenantTx, run_id: Uuid) -> Result<Vec<OutboxItem>, DbError> {
    tx.query::<OutboxItem>(&keys::run_pk(tx.tenant_id(), run_id), "OUT#")
        .await
}

pub async fn undelivered_outbox(
    tx: &mut TenantTx,
    run_id: Uuid,
) -> Result<Vec<OutboxRow>, DbError> {
    Ok(outbox(tx, run_id)
        .await?
        .into_iter()
        .filter(|item| item.delivered_at.is_none())
        .map(|item| OutboxRow {
            id: item.id,
            run_id: item.run_id,
            seq: item.seq,
            kind: item.kind,
            payload: item.payload,
            created_at: item.created_at,
        })
        .collect())
}

pub async fn mark_outbox_delivered(
    tx: &mut TenantTx,
    run_id: Uuid,
    seqs: &[i64],
    at: DateTime<Utc>,
) -> Result<(), DbError> {
    let tenant_id = tx.tenant_id();
    for seq in seqs {
        let key = keys::outbox_key(tenant_id, run_id, *seq);
        if let Some(mut item) = tx.get::<OutboxItem>(&key).await?
            && item.delivered_at.is_none()
        {
            item.delivered_at = Some(at);
            tx.put(&item)?;
        }
    }
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
    let location = locate(tx, run_id, REF_RUN)
        .await?
        .ok_or_else(|| DbError::Missing(format!("run {run_id}")))?;
    let task_id = location
        .parent_id
        .ok_or_else(|| DbError::Decode(format!("run {run_id} has no task")))?;
    let id = Uuid::new_v4();
    let received_at = Utc::now();
    let key = keys::run_event_key(tenant_id, run_id, seq, received_at, id);
    if seq.is_some()
        && let Some(existing) = tx.get::<RunEventItem>(&key).await?
    {
        return Ok(Recorded::Duplicate(existing.id));
    }
    tx.insert(&RunEventItem {
        id,
        tenant_id,
        run_id,
        seq,
        kind: kind.as_str().to_string(),
        payload: payload.clone(),
        received_at,
        processed_at: None,
        error: None,
    })?;
    tx.insert(&Pending {
        key: keys::pending_run_event_key(tenant_id, run_id, received_at, id),
        target: key,
        id,
        received_at,
    })?;
    tx.emit(Job::RunEvents {
        tenant_id,
        task_id,
        run_id,
    });
    Ok(Recorded::New(id))
}

pub async fn record_inbound_event(
    tx: &mut TenantTx,
    source: &str,
    key: &str,
    event: &Value,
) -> Result<Recorded, DbError> {
    let tenant_id = tx.tenant_id();
    let item_key = keys::inbound_event_key(tenant_id, source, key);
    if let Some(existing) = tx.get::<InboundEvent>(&item_key).await? {
        return Ok(Recorded::Duplicate(existing.id));
    }
    let id = Uuid::new_v4();
    let received_at = Utc::now();
    tx.insert(&InboundEvent {
        id,
        tenant_id,
        source: source.to_string(),
        key: key.to_string(),
        event: event.clone(),
        received_at,
        processed_at: None,
        error: None,
    })?;
    tx.insert(&Pending {
        key: keys::pending_inbound_key(tenant_id, received_at, id),
        target: item_key,
        id,
        received_at,
    })?;
    tx.emit(Job::Inbound { tenant_id });
    Ok(Recorded::New(id))
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Event(#[from] GithubEventError),
    #[error("{}", .0.reason())]
    ReceiveLimit(ReceiveLimited),
    #[error("no connected tenant has GitHub installation {0}")]
    UnknownInstallation(i64),
}

pub async fn tenant_for_installation(
    store: &Store,
    installation_id: i64,
) -> Result<Option<Uuid>, DbError> {
    let key = keys::uniq_key(uniq::INSTALLATION, &installation_id.to_string());
    let Some(guard) = store.get::<Guard>(&key).await? else {
        return Ok(None);
    };
    let tenant_id = guard
        .tenant_id
        .ok_or_else(|| DbError::Decode(format!("installation {installation_id} has no tenant")))?;
    let integration = store
        .get::<Integration>(&keys::integration_key(tenant_id, guard.target))
        .await?;
    Ok(integration
        .filter(|i| i.status == "connected")
        .map(|i| i.tenant_id))
}

pub async fn ingest_github_delivery(
    store: &Store,
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
    let tenant_id = tenant_for_installation(store, installation_id)
        .await?
        .ok_or(IngestError::UnknownInstallation(installation_id))?;
    if let Some(existing) = store
        .get::<InboundEvent>(&keys::inbound_event_key(
            tenant_id,
            SOURCE_GITHUB,
            delivery_id,
        ))
        .await?
    {
        return Ok(Some(Recorded::Duplicate(existing.id)));
    }
    if let Admission::Dropped(limited) =
        admit_inbound(store, tenant_id, SOURCE_GITHUB, delivery_id, Utc::now()).await?
    {
        return Err(IngestError::ReceiveLimit(limited));
    }
    let event = normalized.to_value();
    let recorded = retry(8, || async {
        let mut tx = store.tx(tenant_id);
        let recorded = record_inbound_event(&mut tx, SOURCE_GITHUB, delivery_id, &event).await?;
        tx.commit().await?;
        Ok(recorded)
    })
    .await?;
    Ok(Some(recorded))
}
