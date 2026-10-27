use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Utc};
use sanchoris_config::{PermissionLevel, Session};
use sanchoris_core::db::{DbError, Store, TenantTx};
use sanchoris_core::items::Task;
use sanchoris_core::tasks::{load_run, load_task};
use sanchoris_core::tokens::RunClaims;
use sanchoris_graphql::AesGcmEncryptor;
use sanchoris_worker::error::WorkerError;
use sanchoris_worker::github::GitHubApi;
use sanchoris_worker::installation_for_repo;
use sanchoris_worker::usage::credential_of;

pub const OAUTH_TOKEN_PREFIX: &str = "sk-ant-oat";

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("{0}")]
    Unavailable(String),
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<WorkerError> for CredentialError {
    fn from(error: WorkerError) -> Self {
        match error {
            WorkerError::Db(error) => Self::Db(error),
            other => Self::Unavailable(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

#[async_trait]
pub trait RunCredentials: Send + Sync {
    async fn git_token(&self, claims: RunClaims) -> Result<GitToken, CredentialError>;
    async fn worker_env(
        &self,
        claims: RunClaims,
    ) -> Result<BTreeMap<String, String>, CredentialError>;
}

pub struct IssuedCredentials {
    store: Store,
    github: Arc<GitHubApi>,
    encryptor: Arc<AesGcmEncryptor>,
}

impl IssuedCredentials {
    pub fn new(store: Store, github: Arc<GitHubApi>, encryptor: Arc<AesGcmEncryptor>) -> Self {
        Self {
            store,
            github,
            encryptor,
        }
    }
}

fn unavailable(message: impl Into<String>) -> CredentialError {
    CredentialError::Unavailable(message.into())
}

async fn running_task(tx: &mut TenantTx, claims: RunClaims) -> Result<Task, CredentialError> {
    let run = load_run(tx, claims.run_id)
        .await?
        .ok_or_else(|| unavailable(format!("run {} does not exist", claims.run_id)))?;
    if run.is_finished() {
        return Err(unavailable(format!(
            "run {} is no longer running",
            claims.run_id
        )));
    }
    Ok(load_task(tx, run.task_id).await?)
}

fn session_of(task: &Task) -> Result<Session, CredentialError> {
    serde_json::from_value(task.definition["Session"]["Definition"].clone())
        .map_err(|error| unavailable(format!("the task's session does not parse: {error}")))
}

pub fn worker_variable(provider: &str, kind: &str, value: &str) -> Option<&'static str> {
    match (provider, kind) {
        ("claude", "api_key") if value.starts_with(OAUTH_TOKEN_PREFIX) => {
            Some("CLAUDE_CODE_OAUTH_TOKEN")
        }
        ("claude", "api_key") => Some("ANTHROPIC_API_KEY"),
        _ => None,
    }
}

pub fn scope_of(session: &Session) -> (Option<Vec<String>>, BTreeMap<String, String>) {
    let repositories = session
        .github
        .repos
        .iter()
        .map(|repo| repo.split_once('/').map_or(repo.as_str(), |(_, name)| name))
        .collect::<BTreeSet<_>>();
    let repositories = (!repositories.contains("*"))
        .then(|| repositories.into_iter().map(str::to_string).collect());
    let permissions = session
        .github
        .permissions
        .iter()
        .map(|(name, level)| {
            let level = match level {
                PermissionLevel::Read => "read",
                PermissionLevel::Write => "write",
            };
            (name.clone(), level.to_string())
        })
        .collect();
    (repositories, permissions)
}

#[async_trait]
impl RunCredentials for IssuedCredentials {
    async fn git_token(&self, claims: RunClaims) -> Result<GitToken, CredentialError> {
        let mut tx = self.store.tx(claims.tenant_id);
        let task = running_task(&mut tx, claims).await?;
        let session = session_of(&task)?;
        let mut installations = BTreeSet::new();
        for repo in &session.github.repos {
            installations.insert(installation_for_repo(&mut tx, repo).await?);
        }
        let installation = match installations.into_iter().collect::<Vec<_>>()[..] {
            [only] => only,
            [] => return Err(unavailable("the session's GitHub.Repos is empty")),
            _ => {
                return Err(unavailable(
                    "the session's GitHub.Repos spans more than one GitHub installation",
                ));
            }
        };
        let (repositories, permissions) = scope_of(&session);
        let (token, expires_at) = self
            .github
            .scoped_installation_token(installation, repositories.as_deref(), &permissions)
            .await
            .map_err(|error| unavailable(format!("GitHub refused the token: {error}")))?;
        Ok(GitToken { token, expires_at })
    }

    async fn worker_env(
        &self,
        claims: RunClaims,
    ) -> Result<BTreeMap<String, String>, CredentialError> {
        let mut tx = self.store.tx(claims.tenant_id);
        let task = running_task(&mut tx, claims).await?;
        let credential = credential_of(&mut tx, task.id).await?;
        let ciphertext = STANDARD
            .decode(&credential.encrypted_value)
            .map_err(|error| unavailable(format!("credential {}: {error}", credential.name)))?;
        let plaintext = self
            .encryptor
            .decrypt(claims.tenant_id, &ciphertext)
            .map_err(|error| unavailable(format!("credential {}: {error}", credential.name)))?;
        let value = String::from_utf8(plaintext)
            .map_err(|_| unavailable(format!("credential {} is not text", credential.name)))?;
        let variable =
            worker_variable(&credential.provider, &credential.kind, &value).ok_or_else(|| {
                unavailable(format!(
                    "credential {} ({} {}) cannot be used in a Workspace",
                    credential.name, credential.provider, credential.kind
                ))
            })?;
        Ok(BTreeMap::from([(variable.to_string(), value)]))
    }
}

pub struct NoRunCredentials;

pub const NO_RUN_CREDENTIALS: &str = "this process does not serve Run channel messages";

#[async_trait]
impl RunCredentials for NoRunCredentials {
    async fn git_token(&self, _claims: RunClaims) -> Result<GitToken, CredentialError> {
        Err(unavailable(NO_RUN_CREDENTIALS))
    }

    async fn worker_env(
        &self,
        _claims: RunClaims,
    ) -> Result<BTreeMap<String, String>, CredentialError> {
        Err(unavailable(NO_RUN_CREDENTIALS))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_keys_and_oauth_tokens_map_to_their_variables() {
        assert_eq!(
            worker_variable("claude", "api_key", "sk-ant-api03-x"),
            Some("ANTHROPIC_API_KEY")
        );
        assert_eq!(
            worker_variable("claude", "api_key", "sk-ant-oat01-x"),
            Some("CLAUDE_CODE_OAUTH_TOKEN")
        );
        assert_eq!(worker_variable("claude", "subscription", "x"), None);
        assert_eq!(worker_variable("codex", "api_key", "sk-x"), None);
    }

    fn session(repos: &[&str]) -> Session {
        serde_json::from_value(serde_json::json!({
            "Worker": { "Agent": "claude", "Model": "m", "Credential": "c" },
            "Repos": repos,
            "GitHub": { "Repos": repos, "Permissions": { "contents": "write", "issues": "read" } },
            "Network": { "Allow": [] },
            "UsageLimit": { "Resume": "auto" },
            "Workspace": { "Compute": "standard", "IdleSeconds": 60 },
        }))
        .unwrap()
    }

    #[test]
    fn the_scope_names_the_session_repositories_unless_a_wildcard_widens_it() {
        let (repositories, permissions) = scope_of(&session(&["acme/shop-web", "acme/shop-api"]));
        assert_eq!(
            repositories,
            Some(vec!["shop-api".to_string(), "shop-web".to_string()])
        );
        assert_eq!(
            permissions,
            BTreeMap::from([
                ("contents".to_string(), "write".to_string()),
                ("issues".to_string(), "read".to_string()),
            ])
        );
        let (repositories, _) = scope_of(&session(&["acme/*"]));
        assert_eq!(repositories, None);
    }
}
