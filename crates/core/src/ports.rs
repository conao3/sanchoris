use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{port} failed: {message}")]
pub struct PortError {
    pub port: &'static str,
    pub message: String,
}

impl PortError {
    pub fn new(port: &'static str, message: impl Into<String>) -> Self {
        Self {
            port,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingMessage {
    pub body: String,
    pub group: String,
    pub dedupe_id: String,
    pub delay: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedMessage {
    pub body: String,
    pub receipt: String,
    pub receive_count: u32,
}

#[async_trait]
pub trait Queue: Send + Sync {
    async fn send(&self, message: &OutgoingMessage) -> Result<(), PortError>;
    async fn receive(&self, max: usize) -> Result<Vec<ReceivedMessage>, PortError>;
    async fn delete(&self, receipt: &str) -> Result<(), PortError>;
    async fn release(&self, receipt: &str) -> Result<(), PortError>;
}

#[async_trait]
pub trait Scheduler: Send + Sync {
    async fn schedule(
        &self,
        name: &str,
        at: DateTime<Utc>,
        message: &OutgoingMessage,
    ) -> Result<(), PortError>;
    async fn cancel(&self, name: &str) -> Result<(), PortError>;
}

#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn put(&self, key: &str, body: Vec<u8>) -> Result<(), PortError>;
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, PortError>;
    async fn delete(&self, key: &str) -> Result<(), PortError>;
    async fn delete_prefix(&self, prefix: &str, limit: usize) -> Result<usize, PortError>;
}
