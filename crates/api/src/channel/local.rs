use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use sanchoris_core::tokens::RunClaims;
use serde_json::Value;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{Admission, CLOSE_PROTOCOL_ERROR, Channel, Connected, ConnectionSender, SendError};
use crate::auth::bearer;

enum Outbound {
    Text(String),
    Close(u16, String),
}

#[derive(Default)]
pub struct LocalConnections {
    sockets: Mutex<HashMap<String, mpsc::UnboundedSender<Outbound>>>,
}

impl LocalConnections {
    fn register(&self, connection_id: &str) -> mpsc::UnboundedReceiver<Outbound> {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.sockets
            .lock()
            .expect("connections lock")
            .insert(connection_id.to_string(), sender);
        receiver
    }

    fn remove(&self, connection_id: &str) {
        self.sockets
            .lock()
            .expect("connections lock")
            .remove(connection_id);
    }

    fn push(&self, connection_id: &str, outbound: Outbound) -> Result<(), SendError> {
        let sockets = self.sockets.lock().expect("connections lock");
        let sender = sockets.get(connection_id).ok_or(SendError::Gone)?;
        sender.send(outbound).map_err(|_| SendError::Gone)
    }
}

#[async_trait]
impl ConnectionSender for LocalConnections {
    async fn send(&self, connection_id: &str, message: &Value) -> Result<(), SendError> {
        self.push(connection_id, Outbound::Text(message.to_string()))
    }

    async fn close(&self, connection_id: &str, code: u16, reason: &str) -> Result<(), SendError> {
        self.push(connection_id, Outbound::Close(code, reason.to_string()))
    }
}

#[derive(Clone)]
pub struct LocalChannel {
    pub channel: Channel,
    pub connections: Arc<LocalConnections>,
}

pub fn router(state: LocalChannel) -> Router {
    Router::new()
        .route("/runs/channel", get(upgrade))
        .with_state(state)
}

async fn upgrade(
    State(state): State<LocalChannel>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let claims = match state.channel.admit(bearer(&headers), Utc::now()).await {
        Ok(Admission::Accepted(claims)) => claims,
        Ok(Admission::Unauthorized) => return StatusCode::UNAUTHORIZED.into_response(),
        Ok(Admission::Forbidden) => return StatusCode::FORBIDDEN.into_response(),
        Err(error) => {
            eprintln!("sanchoris channel: admission failed: {error}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let connection_id = Uuid::new_v4().to_string();
    let outbound = state.connections.register(&connection_id);
    let run = claims.run();
    match state.channel.connect(claims, &connection_id).await {
        Ok(Connected::Accepted) => {
            let failed = state.clone();
            let failed_id = connection_id.clone();
            ws.on_failed_upgrade(move |_| {
                tokio::spawn(async move {
                    failed.channel.disconnect(run, &failed_id).await.ok();
                    failed.connections.remove(&failed_id);
                });
            })
            .on_upgrade(move |socket| serve(state, run, connection_id, outbound, socket))
        }
        Ok(Connected::Spent) => {
            state.connections.remove(&connection_id);
            StatusCode::FORBIDDEN.into_response()
        }
        Err(error) => {
            eprintln!("sanchoris channel: connect run {}: {error}", run.run_id);
            state.connections.remove(&connection_id);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn serve(
    state: LocalChannel,
    claims: RunClaims,
    connection_id: String,
    mut outbound: mpsc::UnboundedReceiver<Outbound>,
    socket: WebSocket,
) {
    let (mut sink, mut stream) = socket.split();
    let writer = tokio::spawn(async move {
        while let Some(message) = outbound.recv().await {
            match message {
                Outbound::Text(text) => {
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        return;
                    }
                }
                Outbound::Close(code, reason) => {
                    let _ = sink
                        .send(Message::Close(Some(CloseFrame {
                            code,
                            reason: reason.into(),
                        })))
                        .await;
                    return;
                }
            }
        }
    });
    while let Some(message) = stream.next().await {
        match message {
            Ok(Message::Text(text)) => {
                if state
                    .channel
                    .receive(claims, &connection_id, text.as_str())
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(Message::Binary(_)) => {
                state
                    .channel
                    .sender
                    .close(
                        &connection_id,
                        CLOSE_PROTOCOL_ERROR,
                        "binary frames are not supported",
                    )
                    .await
                    .ok();
                break;
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
    if let Err(error) = state.channel.disconnect(claims, &connection_id).await {
        eprintln!(
            "sanchoris channel: disconnect run {}: {error}",
            claims.run_id
        );
    }
    state.connections.remove(&connection_id);
    let _ = writer.await;
}
