pub mod credentials;
pub mod gateway;
pub mod local;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sanchoris_core::channel::{
    OutboxRow, RunEventType, mark_outbox_delivered, record_run_event, undelivered_outbox,
};
use sanchoris_core::db::{DbError, Store, TenantTx, tenant_plan};
use sanchoris_core::item::Entity;
use sanchoris_core::items::{
    ChannelInstance, ChatMessage, OutboxItem, RunChannel, RunEventItem, RunLogEntry,
};
use sanchoris_core::jobs::Job;
use sanchoris_core::keys;
use sanchoris_core::messages::redact_tokens_in;
use sanchoris_core::ports::PortError;
use sanchoris_core::skillrun::{
    SkillRunError, accept_skill_request, continue_skill_run, skill_run_status,
};
use sanchoris_core::tasks::load_run;
use sanchoris_core::tokens::{RunClaims, RunTokenSigner};
use sanchoris_worker::ports::OutboxPusher;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::records::{MAX_RECORD_BYTES, RecordError, RecordStore, record_key};
use credentials::{CredentialError, RunCredentials};

pub use sanchoris_core::channel::SEQ_STRIDE;
pub const CLOSE_PROTOCOL_ERROR: u16 = 4000;
pub const CLOSE_INTERNAL_ERROR: u16 = 1011;
const ATTEMPTS: u32 = 20;
const REASON_LIMIT: usize = 120;

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("the connection is gone")]
    Gone,
    #[error("sending to the connection failed: {0}")]
    Failed(String),
}

#[async_trait]
pub trait ConnectionSender: Send + Sync {
    async fn send(&self, connection_id: &str, message: &Value) -> Result<(), SendError>;
    async fn close(&self, connection_id: &str, code: u16, reason: &str) -> Result<(), SendError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("the connection is gone")]
    Gone,
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("internal error: {0}")]
    Internal(String),
}

impl ChannelError {
    fn is_conflict(&self) -> bool {
        matches!(self, Self::Db(DbError::Conflict))
    }
}

impl From<SendError> for ChannelError {
    fn from(error: SendError) -> Self {
        match error {
            SendError::Gone => Self::Gone,
            SendError::Failed(message) => Self::Internal(message),
        }
    }
}

fn protocol(message: impl Into<String>) -> ChannelError {
    ChannelError::Protocol(message.into())
}

async fn retrying<T, F, Fut>(mut operation: F) -> Result<T, ChannelError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, ChannelError>>,
{
    let mut attempt = 0;
    loop {
        attempt += 1;
        match operation().await {
            Err(error) if error.is_conflict() && attempt < ATTEMPTS => {
                let jitter = u64::from(Uuid::new_v4().as_bytes()[0]) % 20;
                tokio::time::sleep(std::time::Duration::from_millis(
                    5 * u64::from(attempt) + jitter,
                ))
                .await;
            }
            other => return other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Accepted(RunClaims),
    Unauthorized,
    Forbidden,
}

#[derive(Clone)]
pub struct Channel {
    pub store: Store,
    pub run_tokens: RunTokenSigner,
    pub records: Arc<dyn RecordStore>,
    pub sender: Arc<dyn ConnectionSender>,
    pub credentials: Arc<dyn RunCredentials>,
}

fn strip(mut message: Value) -> Value {
    if let Some(object) = message.as_object_mut() {
        object.remove("type");
        object.remove("runId");
        object.remove("seq");
    }
    message
}

async fn last_recorded_turn(tx: &mut TenantTx, run_id: Uuid) -> Result<i64, ChannelError> {
    Ok(tx
        .query::<RunEventItem>(&keys::run_pk(tx.tenant_id(), run_id), "EVT#")
        .await?
        .into_iter()
        .filter(|event| event.kind == RunEventType::SessionRecord.as_str())
        .filter_map(|event| event.payload["turn"].as_i64())
        .max()
        .unwrap_or(0))
}

async fn channel_item(tx: &mut TenantTx, run_id: Uuid) -> Result<RunChannel, ChannelError> {
    tx.get::<RunChannel>(&keys::run_channel_key(tx.tenant_id(), run_id))
        .await?
        .ok_or_else(|| ChannelError::Internal(format!("run {run_id} has no channel")))
}

async fn current_instance(
    tx: &mut TenantTx,
    run_id: Uuid,
    connection_id: &str,
) -> Result<ChannelInstance, ChannelError> {
    let channel = channel_item(tx, run_id).await?;
    if channel.connection_id.as_deref() != Some(connection_id) {
        return Err(protocol(format!(
            "another connection took over run {run_id}"
        )));
    }
    let instance_id = channel
        .connection_instance
        .ok_or_else(|| protocol("the first message must be hello"))?;
    Ok(tx
        .require::<ChannelInstance>(&keys::channel_instance_key(
            tx.tenant_id(),
            run_id,
            instance_id,
        ))
        .await?)
}

enum Sequenced {
    Duplicate(i64),
    Processed { seq: i64, reply: Option<Value> },
}

struct Plan {
    messages: Vec<Value>,
    delivered: Vec<i64>,
    start_seq: Option<i64>,
}

pub async fn admit(
    store: &Store,
    run_tokens: &RunTokenSigner,
    token: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Admission, ChannelError> {
    let Some(claims) = token.and_then(|token| run_tokens.verify(token, now).ok()) else {
        return Ok(Admission::Unauthorized);
    };
    let mut tx = store.tx(claims.tenant_id);
    match load_run(&mut tx, claims.run_id).await? {
        Some(run) if !run.is_finished() => Ok(Admission::Accepted(claims)),
        _ => Ok(Admission::Forbidden),
    }
}

impl Channel {
    pub async fn admit(
        &self,
        token: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Admission, ChannelError> {
        admit(&self.store, &self.run_tokens, token, now).await
    }

    pub async fn connect(
        &self,
        claims: RunClaims,
        connection_id: &str,
    ) -> Result<(), ChannelError> {
        retrying(|| async {
            let mut tx = self.store.tx(claims.tenant_id);
            let mut channel = channel_item(&mut tx, claims.run_id).await?;
            channel.connection_id = Some(connection_id.to_string());
            channel.connection_instance = None;
            channel.connection_started = false;
            tx.put(&channel)?;
            tx.commit().await?;
            Ok(())
        })
        .await
    }

    pub async fn disconnect(
        &self,
        claims: RunClaims,
        connection_id: &str,
    ) -> Result<(), ChannelError> {
        self.forget(claims.tenant_id, claims.run_id, connection_id)
            .await
    }

    async fn forget(
        &self,
        tenant_id: Uuid,
        run_id: Uuid,
        connection_id: &str,
    ) -> Result<(), ChannelError> {
        retrying(|| async {
            let mut tx = self.store.tx(tenant_id);
            let mut channel = channel_item(&mut tx, run_id).await?;
            if channel.connection_id.as_deref() != Some(connection_id) {
                return Ok(());
            }
            channel.connection_id = None;
            channel.connection_instance = None;
            channel.connection_started = false;
            tx.put(&channel)?;
            tx.commit().await?;
            Ok(())
        })
        .await
    }

    pub async fn receive(
        &self,
        claims: RunClaims,
        connection_id: &str,
        text: &str,
    ) -> Result<(), ChannelError> {
        match self.handle(claims, connection_id, text).await {
            Ok(()) | Err(ChannelError::Gone) => Ok(()),
            Err(ChannelError::Protocol(message)) => {
                let reason: String = message.chars().take(REASON_LIMIT).collect();
                let close = json!({
                    "type": "close",
                    "runId": claims.run_id.to_string(),
                    "code": CLOSE_PROTOCOL_ERROR,
                    "reason": reason,
                });
                self.sender.send(connection_id, &close).await.ok();
                self.sender
                    .close(connection_id, CLOSE_PROTOCOL_ERROR, &reason)
                    .await
                    .ok();
                Ok(())
            }
            Err(error) => {
                eprintln!(
                    "sanchoris channel: run {} connection {connection_id}: {error}",
                    claims.run_id
                );
                self.sender
                    .close(connection_id, CLOSE_INTERNAL_ERROR, "internal error")
                    .await
                    .ok();
                Err(error)
            }
        }
    }

    async fn handle(
        &self,
        claims: RunClaims,
        connection_id: &str,
        text: &str,
    ) -> Result<(), ChannelError> {
        let message: Value = serde_json::from_str(text)
            .map_err(|error| protocol(format!("invalid JSON: {error}")))?;
        if message["runId"].as_str() != Some(claims.run_id.to_string().as_str()) {
            return Err(protocol(format!(
                "the message is not for run {}",
                claims.run_id
            )));
        }
        let kind = message["type"]
            .as_str()
            .ok_or_else(|| protocol("the message has no type"))?
            .to_string();
        match kind.as_str() {
            "hello" => self.hello(claims, connection_id, &message).await,
            "heartbeat" => {
                let payload = strip(message);
                retrying(|| async {
                    let mut tx = self.store.tx(claims.tenant_id);
                    current_instance(&mut tx, claims.run_id, connection_id).await?;
                    record_run_event(
                        &mut tx,
                        claims.run_id,
                        None,
                        RunEventType::Heartbeat,
                        &payload,
                    )
                    .await?;
                    tx.commit().await?;
                    Ok(())
                })
                .await
            }
            "ready"
            | "event"
            | "rate_limit"
            | "skill_request"
            | "record_upload_request"
            | "session_record"
            | "turn_end"
            | "stopping"
            | "exec_result"
            | "git_token_request"
            | "worker_credential_request" => {
                self.sequenced(claims, connection_id, &kind, message).await
            }
            other => Err(protocol(format!("unknown message type {other}"))),
        }
    }

    async fn hello(
        &self,
        claims: RunClaims,
        connection_id: &str,
        hello: &Value,
    ) -> Result<(), ChannelError> {
        if hello["protocolVersion"] != 1 {
            return Err(protocol("unsupported protocolVersion"));
        }
        let instance_id = hello["instanceId"]
            .as_str()
            .and_then(|id| id.parse::<Uuid>().ok())
            .ok_or_else(|| protocol("hello needs a UUID instanceId"))?;
        let started = hello["started"]
            .as_bool()
            .ok_or_else(|| protocol("hello needs started"))?;
        let processed = retrying(|| async {
            let tenant_id = claims.tenant_id;
            let mut tx = self.store.tx(tenant_id);
            let mut channel = channel_item(&mut tx, claims.run_id).await?;
            if channel.connection_id.as_deref() != Some(connection_id) {
                return Err(protocol(format!(
                    "another connection took over run {}",
                    claims.run_id
                )));
            }
            if channel.connection_instance.is_some() {
                return Err(protocol("hello was already received on this connection"));
            }
            let key = keys::channel_instance_key(tenant_id, claims.run_id, instance_id);
            let now = Utc::now();
            let processed = match tx.get::<ChannelInstance>(&key).await? {
                Some(mut instance) => {
                    instance.connected_at = now;
                    tx.put(&instance)?;
                    instance.processed_seq
                }
                None => {
                    tx.insert(&ChannelInstance {
                        tenant_id,
                        run_id: claims.run_id,
                        instance_id,
                        seq_offset: channel.instances * SEQ_STRIDE,
                        processed_seq: 0,
                        start_outbox_seq: None,
                        connected_at: now,
                    })?;
                    channel.instances += 1;
                    0
                }
            };
            channel.connection_instance = Some(instance_id);
            channel.connection_started = started;
            tx.put(&channel)?;
            tx.emit(Job::DeliverOutbox {
                tenant_id,
                run_id: claims.run_id,
            });
            tx.commit().await?;
            Ok(processed)
        })
        .await?;
        let ack = json!({ "type": "ack", "runId": claims.run_id.to_string(), "seq": processed });
        self.sender.send(connection_id, &ack).await?;
        Ok(())
    }

    async fn sequenced(
        &self,
        claims: RunClaims,
        connection_id: &str,
        kind: &str,
        message: Value,
    ) -> Result<(), ChannelError> {
        let seq = message["seq"]
            .as_i64()
            .filter(|seq| *seq > 0 && *seq < SEQ_STRIDE)
            .ok_or_else(|| protocol(format!("{kind} needs a positive seq")))?;
        let payload = strip(message);
        let outcome = retrying(|| async {
            let mut tx = self.store.tx(claims.tenant_id);
            let mut instance = current_instance(&mut tx, claims.run_id, connection_id).await?;
            if seq <= instance.processed_seq {
                return Ok(Sequenced::Duplicate(instance.processed_seq));
            }
            if seq != instance.processed_seq + 1 {
                return Err(protocol(format!(
                    "expected seq {}, received {seq}",
                    instance.processed_seq + 1
                )));
            }
            let global = instance.seq_offset + seq;
            let reply = match kind {
                "event" => {
                    self.log(&mut tx, claims, global, &payload).await?;
                    None
                }
                "skill_request" => Some(self.skill(&mut tx, claims, &payload).await?),
                "record_upload_request" => Some(self.upload(&mut tx, claims, &payload).await?),
                "git_token_request" => Some(self.git_token(claims, &payload).await?),
                "worker_credential_request" => {
                    Some(self.worker_credential(claims, &payload).await?)
                }
                other => {
                    let kind = RunEventType::parse(other).expect("sequenced run event types parse");
                    record_run_event(&mut tx, claims.run_id, Some(global), kind, &payload).await?;
                    None
                }
            };
            instance.processed_seq = seq;
            tx.put(&instance)?;
            tx.commit().await?;
            Ok(Sequenced::Processed { seq, reply })
        })
        .await?;
        let run = claims.run_id.to_string();
        match outcome {
            Sequenced::Duplicate(processed) => {
                self.sender
                    .send(
                        connection_id,
                        &json!({ "type": "ack", "runId": run, "seq": processed }),
                    )
                    .await?;
            }
            Sequenced::Processed { seq, reply } => {
                self.sender
                    .send(
                        connection_id,
                        &json!({ "type": "ack", "runId": run, "seq": seq }),
                    )
                    .await?;
                if let Some(reply) = reply {
                    self.sender.send(connection_id, &reply).await?;
                }
            }
        }
        Ok(())
    }

    async fn log(
        &self,
        tx: &mut TenantTx,
        claims: RunClaims,
        seq: i64,
        payload: &Value,
    ) -> Result<(), ChannelError> {
        let turn = payload["turn"]
            .as_i64()
            .filter(|turn| *turn > 0)
            .ok_or_else(|| protocol("event needs a positive turn"))?;
        let at = payload["at"]
            .as_str()
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
            .ok_or_else(|| protocol("event needs an ISO 8601 at"))?
            .with_timezone(&Utc);
        let kind = payload["kind"]
            .as_str()
            .filter(|kind| !kind.is_empty())
            .ok_or_else(|| protocol("event needs kind"))?;
        let key = keys::run_log_key(claims.tenant_id, claims.run_id, seq);
        if tx.get::<RunLogEntry>(&key).await?.is_some() {
            return Ok(());
        }
        let (_, plan) = tenant_plan(tx).await?;
        tx.insert(&RunLogEntry {
            tenant_id: claims.tenant_id,
            run_id: claims.run_id,
            seq,
            turn,
            kind: kind.to_string(),
            data: redact_tokens_in(&payload["data"]),
            at,
            expires_at: Utc::now() + Duration::days(plan.retention_days),
        })?;
        Ok(())
    }

    fn reply(
        claims: RunClaims,
        kind: &str,
        request_id: &str,
        outcome: Result<Value, (String, String)>,
    ) -> Value {
        let run = claims.run_id.to_string();
        match outcome {
            Ok(result) if kind == "skill_response" => json!({
                "type": kind, "runId": run, "requestId": request_id, "ok": true, "result": result,
            }),
            Ok(result) => {
                let mut message = json!({
                    "type": kind, "runId": run, "requestId": request_id, "ok": true,
                });
                if let (Some(target), Value::Object(fields)) = (message.as_object_mut(), result) {
                    target.extend(fields);
                }
                message
            }
            Err((code, message)) => json!({
                "type": kind, "runId": run, "requestId": request_id, "ok": false,
                "error": { "code": code, "message": message },
            }),
        }
    }

    async fn skill(
        &self,
        tx: &mut TenantTx,
        claims: RunClaims,
        payload: &Value,
    ) -> Result<Value, ChannelError> {
        let request_id = payload["requestId"]
            .as_str()
            .ok_or_else(|| protocol("skill_request needs requestId"))?;
        let number = || {
            payload["skillRunId"]
                .as_i64()
                .and_then(|id| i32::try_from(id).ok())
                .ok_or_else(|| protocol("skill_request needs skillRunId"))
        };
        let now = Utc::now();
        let run_id = claims.run_id;
        let outcome = match payload["action"].as_str() {
            Some("run") => {
                let name = payload["name"]
                    .as_str()
                    .ok_or_else(|| protocol("skill_request run needs name"))?;
                accept_skill_request(tx, run_id, name, payload["input"].clone(), now)
                    .await
                    .map(|accepted| accepted.response())
            }
            Some("continue") => {
                continue_skill_run(tx, run_id, number()?, payload["output"].clone(), now).await
            }
            Some("status") => skill_run_status(tx, run_id, number()?).await,
            _ => return Err(protocol("skill_request has an unknown action")),
        };
        let outcome = match outcome {
            Ok(result) => Ok(result),
            Err(SkillRunError::Db(error)) => return Err(error.into()),
            Err(error @ (SkillRunError::Json(_) | SkillRunError::Engine(_))) => {
                return Err(ChannelError::Internal(error.to_string()));
            }
            Err(error) => Err((error.code().to_string(), error.to_string())),
        };
        Ok(Self::reply(claims, "skill_response", request_id, outcome))
    }

    async fn upload(
        &self,
        tx: &mut TenantTx,
        claims: RunClaims,
        payload: &Value,
    ) -> Result<Value, ChannelError> {
        let request_id = payload["requestId"]
            .as_str()
            .ok_or_else(|| protocol("record_upload_request needs requestId"))?;
        let run = load_run(tx, claims.run_id)
            .await?
            .ok_or_else(|| ChannelError::Internal(format!("run {} is missing", claims.run_id)))?;
        if run.is_finished() {
            return Ok(Self::reply(
                claims,
                "record_upload_response",
                request_id,
                Err((
                    "run_finished".into(),
                    format!("Run {} is no longer running", claims.run_id),
                )),
            ));
        }
        let session_id = payload["sessionId"]
            .as_str()
            .ok_or_else(|| protocol("record_upload_request needs sessionId"))?;
        let turn = payload["turn"]
            .as_i64()
            .ok_or_else(|| protocol("record_upload_request needs turn"))?;
        let length = payload["contentLength"]
            .as_i64()
            .ok_or_else(|| protocol("record_upload_request needs contentLength"))?;
        let sha256 = payload["sha256"]
            .as_str()
            .ok_or_else(|| protocol("record_upload_request needs sha256"))?;
        let expected_turn = last_recorded_turn(tx, claims.run_id).await? + 1;
        let refused = if length < 0 {
            Some((
                "invalid_request",
                format!("contentLength {length} is negative"),
            ))
        } else if length > MAX_RECORD_BYTES {
            Some((
                "record_too_large",
                format!("the record has {length} bytes, more than the {MAX_RECORD_BYTES} allowed"),
            ))
        } else if turn != expected_turn {
            Some((
                "unexpected_turn",
                format!(
                    "the next record of run {} is turn {expected_turn}, not {turn}",
                    claims.run_id
                ),
            ))
        } else {
            None
        };
        if let Some((code, message)) = refused {
            return Ok(Self::reply(
                claims,
                "record_upload_response",
                request_id,
                Err((code.into(), message)),
            ));
        }
        let presigned = match record_key(claims.tenant_id, claims.run_id, session_id, turn) {
            Ok(key) => self
                .records
                .presign_put(claims.tenant_id, &key, length, sha256)
                .await
                .map(|presigned| (key, presigned)),
            Err(error) => Err(error),
        };
        let outcome = match presigned {
            Ok((key, presigned)) => Ok(json!({
                "url": presigned.url,
                "headers": presigned.headers,
                "key": key,
            })),
            Err(RecordError::InvalidKey(message)) => Err(("invalid_request".into(), message)),
            Err(RecordError::Storage(message)) => {
                eprintln!(
                    "sanchoris channel: record storage for run {}: {message}",
                    claims.run_id
                );
                Err(("storage_unavailable".into(), message))
            }
        };
        Ok(Self::reply(
            claims,
            "record_upload_response",
            request_id,
            outcome,
        ))
    }

    async fn git_token(&self, claims: RunClaims, payload: &Value) -> Result<Value, ChannelError> {
        let request_id = payload["requestId"]
            .as_str()
            .ok_or_else(|| protocol("git_token_request needs requestId"))?;
        let outcome = match self.credentials.git_token(claims).await {
            Ok(token) => Ok(json!({
                "token": token.token,
                "expiresAt": token.expires_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            })),
            Err(CredentialError::Db(error)) => return Err(error.into()),
            Err(CredentialError::Unavailable(message)) => {
                Err(("credential_unavailable".to_string(), message))
            }
        };
        Ok(Self::reply(
            claims,
            "git_token_response",
            request_id,
            outcome,
        ))
    }

    async fn worker_credential(
        &self,
        claims: RunClaims,
        payload: &Value,
    ) -> Result<Value, ChannelError> {
        let request_id = payload["requestId"]
            .as_str()
            .ok_or_else(|| protocol("worker_credential_request needs requestId"))?;
        let outcome = match self.credentials.worker_env(claims).await {
            Ok(env) => Ok(json!({ "env": env })),
            Err(CredentialError::Db(error)) => return Err(error.into()),
            Err(CredentialError::Unavailable(message)) => {
                Err(("credential_unavailable".to_string(), message))
            }
        };
        Ok(Self::reply(
            claims,
            "worker_credential_response",
            request_id,
            outcome,
        ))
    }

    async fn start_message(
        &self,
        tx: &mut TenantTx,
        run_id: Uuid,
        row: &OutboxRow,
    ) -> Result<Value, ChannelError> {
        let payload = &row.payload;
        let resume = match &payload["resume"] {
            Value::Null => Value::Null,
            resume => {
                let key = resume["recordKey"]
                    .as_str()
                    .ok_or_else(|| ChannelError::Internal("start.resume needs recordKey".into()))?;
                let session_id = resume["sessionId"]
                    .as_str()
                    .ok_or_else(|| ChannelError::Internal("start.resume needs sessionId".into()))?;
                let record = tx
                    .query::<RunEventItem>(&keys::run_pk(tx.tenant_id(), run_id), "EVT#")
                    .await?
                    .into_iter()
                    .filter(|event| {
                        event.kind == RunEventType::SessionRecord.as_str()
                            && event.payload["key"].as_str() == Some(key)
                    })
                    .max_by_key(|event| (event.received_at, event.seq))
                    .ok_or_else(|| {
                        ChannelError::Internal(format!("no session_record has key {key}"))
                    })?;
                let url = self
                    .records
                    .presign_get(tx.tenant_id(), key)
                    .await
                    .map_err(|error| ChannelError::Internal(error.to_string()))?;
                json!({
                    "sessionId": session_id,
                    "url": url,
                    "relativePath": record.payload["relativePath"],
                    "sha256": record.payload["sha256"],
                    "lastTurn": resume["lastTurn"],
                })
            }
        };
        Ok(json!({
            "type": "start",
            "runId": run_id.to_string(),
            "agent": payload["agent"],
            "model": payload["model"],
            "systemPrompt": payload["systemPrompt"],
            "skills": payload["skills"],
            "instruction": payload["instruction"],
            "resume": resume,
            "heartbeatSeconds": payload["heartbeatSeconds"],
        }))
    }

    fn command(run_id: Uuid, row: &OutboxRow) -> Result<Value, ChannelError> {
        let payload = &row.payload;
        let run_id = run_id.to_string();
        Ok(match row.kind.as_str() {
            "user_message" => json!({
                "type": "user_message", "runId": run_id, "messageId": row.id.to_string(), "text": payload["text"],
            }),
            "exec" => json!({
                "type": "exec", "runId": run_id, "requestId": payload["requestId"],
                "purpose": payload["purpose"], "commands": payload["commands"],
            }),
            "interrupt" => json!({ "type": "interrupt", "runId": run_id }),
            "shutdown" => {
                json!({ "type": "shutdown", "runId": run_id, "reason": payload["reason"] })
            }
            other => {
                return Err(ChannelError::Internal(format!(
                    "outbox type {other} is not sent to the agent"
                )));
            }
        })
    }

    async fn relay_chat_posts(&self, tenant_id: Uuid, run_id: Uuid) -> Result<(), ChannelError> {
        retrying(|| async {
            let mut tx = self.store.tx(tenant_id);
            let posts = undelivered_outbox(&mut tx, run_id)
                .await?
                .into_iter()
                .filter(|row| row.kind == "chat_post")
                .collect::<Vec<_>>();
            if posts.is_empty() {
                return Ok(());
            }
            let channel = channel_item(&mut tx, run_id).await?;
            let now = Utc::now();
            for post in &posts {
                let message = ChatMessage {
                    id: post.id,
                    tenant_id,
                    task_id: channel.task_id,
                    run_id: Some(run_id),
                    outbox_id: Some(post.id),
                    channel: Some(post.payload["channel"].clone()),
                    author_kind: "agent".into(),
                    text: post.payload["text"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    created_at: post.created_at,
                };
                if tx.get::<ChatMessage>(&message.key()).await?.is_none() {
                    tx.insert(&message)?;
                }
            }
            let seqs = posts.iter().map(|post| post.seq).collect::<Vec<_>>();
            mark_outbox_delivered(&mut tx, run_id, &seqs, now).await?;
            tx.commit().await?;
            Ok(())
        })
        .await
    }

    async fn plan(
        &self,
        tx: &mut TenantTx,
        run_id: Uuid,
        started: bool,
        instance: &ChannelInstance,
    ) -> Result<Option<Plan>, ChannelError> {
        let rows = undelivered_outbox(tx, run_id)
            .await?
            .into_iter()
            .filter(|row| row.kind != "chat_post")
            .collect::<Vec<_>>();
        let starts = rows
            .iter()
            .filter(|row| row.kind == "start")
            .collect::<Vec<_>>();
        let mut plan = Plan {
            messages: Vec::new(),
            delivered: Vec::new(),
            start_seq: None,
        };
        let mut after = 0;
        if !started {
            let start = match (starts.last(), instance.start_outbox_seq) {
                (Some(row), _) => (*row).clone(),
                (None, Some(seq)) => {
                    let item = tx
                        .require::<OutboxItem>(&keys::outbox_key(tx.tenant_id(), run_id, seq))
                        .await?;
                    OutboxRow {
                        id: item.id,
                        run_id: item.run_id,
                        seq: item.seq,
                        kind: item.kind,
                        payload: item.payload,
                        created_at: item.created_at,
                    }
                }
                (None, None) => return Ok(None),
            };
            plan.messages
                .push(self.start_message(tx, run_id, &start).await?);
            plan.delivered.extend(
                rows.iter()
                    .filter(|row| {
                        row.kind == "start" || (row.kind == "interrupt" && row.seq < start.seq)
                    })
                    .map(|row| row.seq),
            );
            plan.start_seq = Some(start.seq);
            after = start.seq;
        } else if !starts.is_empty() {
            return Ok(None);
        }
        for row in &rows {
            if row.kind == "start" || (row.kind == "interrupt" && row.seq < after) {
                continue;
            }
            plan.messages.push(Self::command(run_id, row)?);
            plan.delivered.push(row.seq);
        }
        Ok(Some(plan))
    }

    pub async fn deliver(&self, tenant_id: Uuid, run_id: Uuid) -> Result<(), ChannelError> {
        self.relay_chat_posts(tenant_id, run_id).await?;
        let mut tx = self.store.tx(tenant_id);
        let Some(channel) = tx
            .get::<RunChannel>(&keys::run_channel_key(tenant_id, run_id))
            .await?
        else {
            return Ok(());
        };
        let (Some(connection_id), Some(instance_id)) =
            (channel.connection_id.clone(), channel.connection_instance)
        else {
            return Ok(());
        };
        let instance = tx
            .require::<ChannelInstance>(&keys::channel_instance_key(tenant_id, run_id, instance_id))
            .await?;
        let Some(plan) = self
            .plan(&mut tx, run_id, channel.connection_started, &instance)
            .await?
        else {
            return Ok(());
        };
        if plan.messages.is_empty() {
            return Ok(());
        }
        for message in &plan.messages {
            match self.sender.send(&connection_id, message).await {
                Ok(()) => {}
                Err(SendError::Gone) => {
                    return self.forget(tenant_id, run_id, &connection_id).await;
                }
                Err(SendError::Failed(message)) => return Err(ChannelError::Internal(message)),
            }
        }
        retrying(|| async {
            let mut tx = self.store.tx(tenant_id);
            let mut channel = channel_item(&mut tx, run_id).await?;
            if channel.connection_id.as_deref() != Some(connection_id.as_str()) {
                return Ok(());
            }
            mark_outbox_delivered(&mut tx, run_id, &plan.delivered, Utc::now()).await?;
            if let Some(seq) = plan.start_seq {
                let key = keys::channel_instance_key(tenant_id, run_id, instance_id);
                let mut instance = tx.require::<ChannelInstance>(&key).await?;
                instance.start_outbox_seq = Some(seq);
                tx.put(&instance)?;
                channel.connection_started = true;
                tx.put(&channel)?;
            }
            tx.commit().await?;
            Ok(())
        })
        .await
    }
}

#[async_trait]
impl OutboxPusher for Channel {
    async fn push(&self, tenant_id: Uuid, run_id: Uuid) -> Result<(), PortError> {
        self.deliver(tenant_id, run_id)
            .await
            .map_err(|error| PortError::new("run channel", error.to_string()))
    }
}
