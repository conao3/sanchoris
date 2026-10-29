use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sanchoris_core::items::GitHubRepository;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PortError {
    #[error("{0}")]
    Failed(String),
    #[error("{0}")]
    Rejected(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantCheckout {
    pub user_id: Uuid,
    pub email: String,
    pub tenant_name: String,
    pub plan_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanChange {
    Applied,
    Checkout { url: String },
    Portal { url: String },
}

#[async_trait]
pub trait CheckoutPort: Send + Sync {
    async fn tenant_checkout(&self, checkout: &TenantCheckout) -> Result<String, PortError>;
    async fn change_plan(&self, tenant_id: Uuid, plan_id: &str) -> Result<PlanChange, PortError>;
    async fn billing_portal(&self, tenant_id: Uuid) -> Result<String, PortError>;
    async fn end_billing(&self, tenant_id: Uuid) -> Result<(), PortError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubRedirect {
    pub url: String,
    pub cookie: String,
}

pub trait GithubConnectUrl: Send + Sync {
    fn link_account(&self, user_id: Uuid) -> GithubRedirect;
    fn install_app(&self, tenant_id: Uuid, user_id: Uuid) -> GithubRedirect;
}

pub trait SlackConnectUrl: Send + Sync {
    fn install(&self, tenant_id: Uuid) -> Result<String, PortError>;
}

#[async_trait]
pub trait ConfigSyncTrigger: Send + Sync {
    async fn request_sync(&self, tenant_id: Uuid, config_id: Uuid) -> Result<(), PortError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigLocation {
    pub repository: String,
    pub root: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigProblem {
    pub path: String,
    pub line: Option<i32>,
    pub column: Option<i32>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigCheck {
    Valid(GitHubRepository),
    Invalid(Vec<ConfigProblem>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerProposal {
    pub location: ConfigLocation,
    pub title: String,
    pub trigger_name: String,
    pub trigger_yaml: String,
}

#[async_trait]
pub trait ConfigRepositoryPort: Send + Sync {
    async fn check(
        &self,
        tenant_id: Uuid,
        location: &ConfigLocation,
    ) -> Result<ConfigCheck, PortError>;
    async fn propose_trigger(
        &self,
        tenant_id: Uuid,
        proposal: &TriggerProposal,
    ) -> Result<i32, PortError>;
}

#[async_trait]
pub trait ChatDispatcher: Send + Sync {
    async fn dispatch(&self, tenant_id: Uuid, source: &str, key: &str) -> Result<(), PortError>;
}

#[async_trait]
pub trait AuditExportPort: Send + Sync {
    async fn publish(
        &self,
        tenant_id: Uuid,
        file_name: &str,
        json_lines: Vec<u8>,
    ) -> Result<String, PortError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encrypted {
    pub ciphertext: Vec<u8>,
    pub key_id: String,
}

#[async_trait]
pub trait Encryptor: Send + Sync {
    async fn encrypt(&self, tenant_id: Uuid, plaintext: &[u8]) -> Result<Encrypted, PortError>;
}

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Clone)]
pub struct GraphqlDeps {
    pub checkout: Arc<dyn CheckoutPort>,
    pub github: Arc<dyn GithubConnectUrl>,
    pub slack: Arc<dyn SlackConnectUrl>,
    pub config_sync: Arc<dyn ConfigSyncTrigger>,
    pub config_repository: Arc<dyn ConfigRepositoryPort>,
    pub chat: Arc<dyn ChatDispatcher>,
    pub audit_export: Arc<dyn AuditExportPort>,
    pub encryptor: Arc<dyn Encryptor>,
    pub clock: Arc<dyn Clock>,
}
