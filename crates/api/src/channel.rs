use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sanchoris_core::channel::{
    OutboxRow, RunEventType, mark_outbox_delivered, record_run_event, undelivered_outbox,
};
use sanchoris_core::db::{DbError, TenantTx};
use sanchoris_core::skillrun::{
    SkillRunError, accept_skill_request, continue_skill_run, skill_run_status,
};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::bearer;
use crate::records::{RecordError, record_key};

pub const SEQ_STRIDE: i64 = 1_000_000_000;
pub const CLOSE_PROTOCOL_ERROR: u16 = 4000;
pub const CLOSE_NO_START: u16 = 4001;
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const START_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub struct ChannelRegistry {
    state: Mutex<(u64, HashMap<Uuid, u64>)>,
}

impl ChannelRegistry {
    fn claim(&self, run_id: Uuid) -> u64 {
        let mut state = self.state.lock().expect("channel registry lock");
        state.0 += 1;
        let generation = state.0;
        state.1.insert(run_id, generation);
        generation
    }

    fn is_current(&self, run_id: Uuid, generation: u64) -> bool {
        self.state
            .lock()
            .expect("channel registry lock")
            .1
            .get(&run_id)
            == Some(&generation)
    }

    fn release(&self, run_id: Uuid, generation: u64) {
        let mut state = self.state.lock().expect("channel registry lock");
        if state.1.get(&run_id) == Some(&generation) {
            state.1.remove(&run_id);
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum ChannelError {
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("no start message is pending for this agent")]
    NoStart,
    #[error("the socket closed")]
    Closed,
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<sqlx::Error> for ChannelError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<DbError> for ChannelError {
    fn from(error: DbError) -> Self {
        Self::Internal(error.to_string())
    }
}

fn protocol(message: impl Into<String>) -> ChannelError {
    ChannelError::Protocol(message.into())
}

pub async fn upgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(run_id) =
        bearer(&headers).and_then(|token| state.config.run_tokens.verify(token, Utc::now()).ok())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let run = sqlx::query("SELECT tenant_id, status FROM sanchoris.runs WHERE id = $1")
        .bind(run_id)
        .fetch_optional(&state.pool)
        .await;
    let tenant_id = match run {
        Ok(Some(row)) => {
            let status: String = row.get("status");
            if matches!(status.as_str(), "completed" | "failed") {
                return StatusCode::FORBIDDEN.into_response();
            }
            row.get::<Uuid, _>("tenant_id")
        }
        Ok(None) => return StatusCode::FORBIDDEN.into_response(),
        Err(error) => {
            eprintln!("sanchoris api: run channel lookup failed: {error}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    ws.on_upgrade(move |socket| serve(state, run_id, tenant_id, socket))
}

async fn serve(state: AppState, run_id: Uuid, tenant_id: Uuid, mut socket: WebSocket) {
    let generation = state.channels.claim(run_id);
    let mut connection = Connection {
        state: state.clone(),
        run_id,
        tenant_id,
        generation,
        instance: None,
        started: false,
        connected_at: std::time::Instant::now(),
    };
    let outcome = connection.run(&mut socket).await;
    state.channels.release(run_id, generation);
    let close = match outcome {
        Ok(()) | Err(ChannelError::Closed) => return,
        Err(ChannelError::Protocol(message)) => (CLOSE_PROTOCOL_ERROR, message),
        Err(ChannelError::NoStart) => (CLOSE_NO_START, ChannelError::NoStart.to_string()),
        Err(ChannelError::Internal(message)) => {
            eprintln!("sanchoris api: run channel {run_id}: {message}");
            (1011, "internal error".to_string())
        }
    };
    let reason: String = close.1.chars().take(120).collect();
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: close.0,
            reason: reason.into(),
        })))
        .await;
}

struct Instance {
    instance_id: Uuid,
    seq_offset: i64,
    start_outbox_id: Option<Uuid>,
}

struct Connection {
    state: AppState,
    run_id: Uuid,
    tenant_id: Uuid,
    generation: u64,
    instance: Option<Instance>,
    started: bool,
    connected_at: std::time::Instant,
}

fn strip(mut message: Value) -> Value {
    if let Some(object) = message.as_object_mut() {
        object.remove("type");
        object.remove("runId");
        object.remove("seq");
    }
    message
}

impl Connection {
    fn run_id_text(&self) -> String {
        self.run_id.to_string()
    }

    fn instance(&self) -> &Instance {
        self.instance
            .as_ref()
            .expect("hello registered the instance")
    }

    async fn begin(&self) -> Result<TenantTx, ChannelError> {
        Ok(TenantTx::begin(&self.state.pool, self.tenant_id).await?)
    }

    async fn send(&self, socket: &mut WebSocket, message: Value) -> Result<(), ChannelError> {
        socket
            .send(Message::Text(message.to_string().into()))
            .await
            .map_err(|_| ChannelError::Closed)
    }

    async fn run(&mut self, socket: &mut WebSocket) -> Result<(), ChannelError> {
        let first = tokio::time::timeout(HELLO_TIMEOUT, socket.recv())
            .await
            .map_err(|_| protocol("no hello within 10 seconds"))?;
        let hello = match first {
            Some(Ok(Message::Text(text))) => self.parse(text.as_str())?,
            _ => return Err(ChannelError::Closed),
        };
        if hello["type"] != "hello" {
            return Err(protocol("the first message must be hello"));
        }
        if hello["protocolVersion"] != 1 {
            return Err(protocol("unsupported protocolVersion"));
        }
        let instance_id = hello["instanceId"]
            .as_str()
            .and_then(|id| id.parse::<Uuid>().ok())
            .ok_or_else(|| protocol("hello needs a UUID instanceId"))?;
        self.started = hello["started"]
            .as_bool()
            .ok_or_else(|| protocol("hello needs started"))?;
        self.register(instance_id).await?;
        self.deliver(socket).await?;
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        loop {
            tokio::select! {
                message = socket.recv() => match message {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return Ok(()),
                    Some(Ok(Message::Text(text))) => {
                        let message = self.parse(text.as_str())?;
                        self.handle(socket, message).await?;
                        self.deliver(socket).await?;
                    }
                    Some(Ok(Message::Binary(_))) => return Err(protocol("binary frames are not supported")),
                    Some(Ok(_)) => {}
                },
                _ = ticker.tick() => self.deliver(socket).await?,
            }
        }
    }

    fn parse(&self, text: &str) -> Result<Value, ChannelError> {
        let message: Value = serde_json::from_str(text)
            .map_err(|error| protocol(format!("invalid JSON: {error}")))?;
        if message["runId"].as_str() != Some(self.run_id_text().as_str()) {
            return Err(protocol(format!(
                "the message is not for run {}",
                self.run_id
            )));
        }
        if !message["type"].is_string() {
            return Err(protocol("the message has no type"));
        }
        Ok(message)
    }

    async fn register(&mut self, instance_id: Uuid) -> Result<(), ChannelError> {
        let mut tx = self.begin().await?;
        sqlx::query("SELECT id FROM sanchoris.runs WHERE id = $1 FOR UPDATE")
            .bind(self.run_id)
            .execute(tx.conn())
            .await?;
        let existing = sqlx::query(
            "SELECT seq_offset, start_outbox_id FROM sanchoris.run_channel_instances \
             WHERE run_id = $1 AND instance_id = $2",
        )
        .bind(self.run_id)
        .bind(instance_id)
        .fetch_optional(tx.conn())
        .await?;
        let instance = match existing {
            Some(row) => {
                sqlx::query(
                    "UPDATE sanchoris.run_channel_instances SET connected_at = $3 \
                     WHERE run_id = $1 AND instance_id = $2",
                )
                .bind(self.run_id)
                .bind(instance_id)
                .bind(Utc::now())
                .execute(tx.conn())
                .await?;
                Instance {
                    instance_id,
                    seq_offset: row.try_get("seq_offset")?,
                    start_outbox_id: row.try_get("start_outbox_id")?,
                }
            }
            None => {
                let seq_offset = sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) * $2 FROM sanchoris.run_channel_instances WHERE run_id = $1",
                )
                .bind(self.run_id)
                .bind(SEQ_STRIDE)
                .fetch_one(tx.conn())
                .await?;
                sqlx::query(
                    "INSERT INTO sanchoris.run_channel_instances (tenant_id, run_id, instance_id, seq_offset, connected_at) \
                     VALUES ($1, $2, $3, $4, $5)",
                )
                .bind(self.tenant_id)
                .bind(self.run_id)
                .bind(instance_id)
                .bind(seq_offset)
                .bind(Utc::now())
                .execute(tx.conn())
                .await?;
                Instance {
                    instance_id,
                    seq_offset,
                    start_outbox_id: None,
                }
            }
        };
        tx.commit().await?;
        self.instance = Some(instance);
        Ok(())
    }

    async fn handle(&mut self, socket: &mut WebSocket, message: Value) -> Result<(), ChannelError> {
        let kind = message["type"].as_str().unwrap_or_default().to_string();
        match kind.as_str() {
            "hello" => Err(protocol("hello was already received on this connection")),
            "heartbeat" => {
                let mut tx = self.begin().await?;
                record_run_event(
                    &mut tx,
                    self.run_id,
                    None,
                    RunEventType::Heartbeat,
                    &strip(message),
                )
                .await?;
                tx.commit().await?;
                Ok(())
            }
            "ready"
            | "event"
            | "rate_limit"
            | "skill_request"
            | "record_upload_request"
            | "session_record"
            | "turn_end"
            | "exec_result" => self.sequenced(socket, &kind, message).await,
            other => Err(protocol(format!("unknown message type {other}"))),
        }
    }

    async fn sequenced(
        &mut self,
        socket: &mut WebSocket,
        kind: &str,
        message: Value,
    ) -> Result<(), ChannelError> {
        let seq = message["seq"]
            .as_i64()
            .filter(|seq| *seq > 0 && *seq < SEQ_STRIDE)
            .ok_or_else(|| protocol(format!("{kind} needs a positive seq")))?;
        let instance_id = self.instance().instance_id;
        let mut tx = self.begin().await?;
        let processed = sqlx::query_scalar::<_, i64>(
            "SELECT processed_seq FROM sanchoris.run_channel_instances \
             WHERE run_id = $1 AND instance_id = $2 FOR UPDATE",
        )
        .bind(self.run_id)
        .bind(instance_id)
        .fetch_one(tx.conn())
        .await?;
        if seq <= processed {
            tx.rollback().await?;
            return self.ack(socket, processed).await;
        }
        if seq != processed + 1 {
            return Err(protocol(format!(
                "expected seq {}, received {seq}",
                processed + 1
            )));
        }
        let global = self.instance().seq_offset + seq;
        let payload = strip(message);
        let reply = match kind {
            "event" => {
                self.log(&mut tx, global, &payload).await?;
                None
            }
            "skill_request" => Some(self.skill(&mut tx, &payload).await?),
            "record_upload_request" => Some(self.upload(&mut tx, &payload).await?),
            other => {
                let kind = RunEventType::parse(other).expect("sequenced run event types parse");
                record_run_event(&mut tx, self.run_id, Some(global), kind, &payload).await?;
                None
            }
        };
        sqlx::query(
            "UPDATE sanchoris.run_channel_instances SET processed_seq = $3 \
             WHERE run_id = $1 AND instance_id = $2",
        )
        .bind(self.run_id)
        .bind(instance_id)
        .bind(seq)
        .execute(tx.conn())
        .await?;
        tx.commit().await?;
        self.ack(socket, seq).await?;
        if let Some(reply) = reply {
            self.send(socket, reply).await?;
        }
        Ok(())
    }

    async fn ack(&self, socket: &mut WebSocket, seq: i64) -> Result<(), ChannelError> {
        self.send(
            socket,
            json!({ "type": "ack", "runId": self.run_id_text(), "seq": seq }),
        )
        .await
    }

    async fn log(&self, tx: &mut TenantTx, seq: i64, payload: &Value) -> Result<(), ChannelError> {
        let turn = payload["turn"]
            .as_i64()
            .and_then(|turn| i32::try_from(turn).ok())
            .ok_or_else(|| protocol("event needs turn"))?;
        let at = payload["at"]
            .as_str()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .ok_or_else(|| protocol("event needs an ISO 8601 at"))?;
        sqlx::query(
            "INSERT INTO sanchoris.run_log_entries (tenant_id, run_id, seq, turn, kind, data, at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (run_id, seq) DO NOTHING",
        )
        .bind(self.tenant_id)
        .bind(self.run_id)
        .bind(seq)
        .bind(turn)
        .bind(
            payload["kind"]
                .as_str()
                .ok_or_else(|| protocol("event needs kind"))?,
        )
        .bind(&payload["data"])
        .bind(at.with_timezone(&Utc))
        .execute(tx.conn())
        .await
        .map_err(|error| match &error {
            sqlx::Error::Database(db) if db.is_check_violation() => {
                protocol(format!("invalid event: {error}"))
            }
            _ => error.into(),
        })?;
        Ok(())
    }

    fn reply(
        &self,
        kind: &str,
        request_id: &str,
        outcome: Result<Value, (String, String)>,
    ) -> Value {
        match outcome {
            Ok(result) => match kind {
                "skill_response" => json!({
                    "type": kind, "runId": self.run_id_text(), "requestId": request_id, "ok": true, "result": result,
                }),
                _ => {
                    let mut message = json!({
                        "type": kind, "runId": self.run_id_text(), "requestId": request_id, "ok": true,
                    });
                    message.as_object_mut().expect("object").extend(
                        result
                            .as_object()
                            .expect("upload result is an object")
                            .clone(),
                    );
                    message
                }
            },
            Err((code, message)) => json!({
                "type": kind, "runId": self.run_id_text(), "requestId": request_id, "ok": false,
                "error": { "code": code, "message": message },
            }),
        }
    }

    async fn skill(&self, tx: &mut TenantTx, payload: &Value) -> Result<Value, ChannelError> {
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
        let outcome = match payload["action"].as_str() {
            Some("run") => {
                let name = payload["name"]
                    .as_str()
                    .ok_or_else(|| protocol("skill_request run needs name"))?;
                accept_skill_request(tx, self.run_id, name, payload["input"].clone(), now)
                    .await
                    .map(|accepted| accepted.response())
            }
            Some("continue") => {
                continue_skill_run(tx, self.run_id, number()?, payload["output"].clone(), now).await
            }
            Some("status") => skill_run_status(tx, self.run_id, number()?).await,
            _ => return Err(protocol("skill_request has an unknown action")),
        };
        let outcome = match outcome {
            Ok(result) => Ok(result),
            Err(
                error @ (SkillRunError::Db(_) | SkillRunError::Json(_) | SkillRunError::Engine(_)),
            ) => {
                return Err(ChannelError::Internal(error.to_string()));
            }
            Err(error) => Err((error.code().to_string(), error.to_string())),
        };
        Ok(self.reply("skill_response", request_id, outcome))
    }

    async fn upload(&self, tx: &mut TenantTx, payload: &Value) -> Result<Value, ChannelError> {
        let request_id = payload["requestId"]
            .as_str()
            .ok_or_else(|| protocol("record_upload_request needs requestId"))?;
        let status =
            sqlx::query_scalar::<_, String>("SELECT status FROM sanchoris.runs WHERE id = $1")
                .bind(self.run_id)
                .fetch_one(tx.conn())
                .await?;
        if matches!(status.as_str(), "completed" | "failed") {
            return Ok(self.reply(
                "record_upload_response",
                request_id,
                Err((
                    "run_finished".into(),
                    format!("Run {} is no longer running", self.run_id),
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
        let presigned = match record_key(self.tenant_id, self.run_id, session_id, turn) {
            Ok(key) => self
                .state
                .records
                .presign_put(self.tenant_id, &key, length, sha256)
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
                    "sanchoris api: record storage for run {}: {message}",
                    self.run_id
                );
                Err(("storage_unavailable".into(), message))
            }
        };
        Ok(self.reply("record_upload_response", request_id, outcome))
    }

    async fn start_message(
        &self,
        tx: &mut TenantTx,
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
                let record = sqlx::query_scalar::<_, Value>(
                    "SELECT payload FROM sanchoris.run_events \
                     WHERE run_id = $1 AND type = 'session_record' AND payload->>'key' = $2 \
                     ORDER BY received_at DESC, seq DESC LIMIT 1",
                )
                .bind(self.run_id)
                .bind(key)
                .fetch_optional(tx.conn())
                .await?
                .ok_or_else(|| {
                    ChannelError::Internal(format!("no session_record has key {key}"))
                })?;
                let url = self
                    .state
                    .records
                    .presign_get(self.tenant_id, key)
                    .await
                    .map_err(|error| ChannelError::Internal(error.to_string()))?;
                json!({
                    "sessionId": session_id,
                    "url": url,
                    "relativePath": record["relativePath"],
                    "sha256": record["sha256"],
                    "lastTurn": resume["lastTurn"],
                })
            }
        };
        Ok(json!({
            "type": "start",
            "runId": self.run_id_text(),
            "agent": payload["agent"],
            "model": payload["model"],
            "systemPrompt": payload["systemPrompt"],
            "skills": payload["skills"],
            "instruction": payload["instruction"],
            "resume": resume,
            "heartbeatSeconds": payload["heartbeatSeconds"],
        }))
    }

    fn command(&self, row: &OutboxRow) -> Result<Value, ChannelError> {
        let payload = &row.payload;
        let run_id = self.run_id_text();
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

    async fn deliver(&mut self, socket: &mut WebSocket) -> Result<(), ChannelError> {
        if !self.state.channels.is_current(self.run_id, self.generation) {
            return Ok(());
        }
        let mut tx = self.begin().await?;
        let rows = undelivered_outbox(tx.conn(), self.run_id).await?;
        let starts: Vec<&OutboxRow> = rows.iter().filter(|row| row.kind == "start").collect();
        let mut after = 0;
        if !self.started {
            let start = match (starts.last(), self.instance().start_outbox_id) {
                (Some(row), _) => (*row).clone(),
                (None, Some(id)) => {
                    let row = sqlx::query(
                        "SELECT id, run_id, seq, type, payload, created_at FROM sanchoris.run_outbox WHERE id = $1",
                    )
                    .bind(id)
                    .fetch_one(tx.conn())
                    .await?;
                    OutboxRow {
                        id: row.try_get("id")?,
                        run_id: row.try_get("run_id")?,
                        seq: row.try_get("seq")?,
                        kind: row.try_get("type")?,
                        payload: row.try_get("payload")?,
                        created_at: row.try_get("created_at")?,
                    }
                }
                (None, None) if self.connected_at.elapsed() < START_TIMEOUT => {
                    tx.commit().await?;
                    return Ok(());
                }
                (None, None) => return Err(ChannelError::NoStart),
            };
            let message = self.start_message(&mut tx, &start).await?;
            self.send(socket, message).await?;
            let stale: Vec<Uuid> = rows
                .iter()
                .filter(|row| {
                    row.kind == "start" || (row.kind == "interrupt" && row.seq < start.seq)
                })
                .map(|row| row.id)
                .collect();
            mark_outbox_delivered(tx.conn(), &stale, Utc::now()).await?;
            sqlx::query(
                "UPDATE sanchoris.run_channel_instances SET start_outbox_id = $3 \
                 WHERE run_id = $1 AND instance_id = $2",
            )
            .bind(self.run_id)
            .bind(self.instance().instance_id)
            .bind(start.id)
            .execute(tx.conn())
            .await?;
            if let Some(instance) = self.instance.as_mut() {
                instance.start_outbox_id = Some(start.id);
            }
            self.started = true;
            after = start.seq;
        } else if !starts.is_empty() {
            tx.commit().await?;
            return Ok(());
        }
        for row in &rows {
            if matches!(row.kind.as_str(), "start" | "chat_post")
                || (row.kind == "interrupt" && row.seq < after)
            {
                continue;
            }
            let message = self.command(row)?;
            self.send(socket, message).await?;
            mark_outbox_delivered(tx.conn(), &[row.id], Utc::now()).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
