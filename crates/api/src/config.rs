use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use aws_sdk_ssm::error::DisplayErrorContext;
use sanchoris_core::tokens::{
    RELAY_TOKEN_SIGNING_KEY_ENV, RUN_TOKEN_SIGNING_KEY_ENV, RunTokenSigner, Signer,
    TASK_TOKEN_SIGNING_KEY_ENV, TaskTokenSigner,
};
use sanchoris_graphql::AesGcmEncryptor;
use sanchoris_worker::github::transport::ReqwestTransport;
use sanchoris_worker::github::{AppAuth, GitHubApi};
use thiserror::Error;

use crate::graphql::is_cloudflare;
use crate::signing::HmacKey;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required environment variables: {0}")]
    Missing(String),
    #[error("missing SSM parameters: {0}")]
    MissingParameters(String),
    #[error("{name}: {message}")]
    Invalid { name: &'static str, message: String },
}

pub const SECRETS_PATH_ENV: &str = "SANCHORIS_SECRETS_PATH";
pub const CREDENTIAL_KEY_ENV: &str = "SANCHORIS_CREDENTIAL_KEY";
pub const SSM_BATCH: usize = 10;

pub const SECRET_PARAMETERS: [(&str, &str); 13] = [
    ("GITHUB_APP_ID", "github-app-id"),
    ("GITHUB_APP_PRIVATE_KEY", "github-app-private-key"),
    ("GITHUB_APP_CLIENT_ID", "github-app-client-id"),
    ("GITHUB_APP_CLIENT_SECRET", "github-app-client-secret"),
    ("GITHUB_APP_SLUG", "github-app-slug"),
    ("GITHUB_WEBHOOK_SECRET", "github-webhook-secret"),
    ("STRIPE_SECRET_KEY", "stripe-secret-key"),
    ("STRIPE_WEBHOOK_SECRET", "stripe-webhook-secret"),
    (RUN_TOKEN_SIGNING_KEY_ENV, "run-token-signing-key"),
    (TASK_TOKEN_SIGNING_KEY_ENV, "task-token-signing-key"),
    ("OAUTH_STATE_SIGNING_KEY", "oauth-state-signing-key"),
    (CREDENTIAL_KEY_ENV, "credential-key"),
    (RELAY_TOKEN_SIGNING_KEY_ENV, "relay-token-signing-key"),
];

pub const API_SECRETS: [&str; 11] = [
    "GITHUB_APP_ID",
    "GITHUB_APP_PRIVATE_KEY",
    "GITHUB_APP_CLIENT_ID",
    "GITHUB_APP_CLIENT_SECRET",
    "GITHUB_APP_SLUG",
    "GITHUB_WEBHOOK_SECRET",
    "STRIPE_SECRET_KEY",
    "STRIPE_WEBHOOK_SECRET",
    TASK_TOKEN_SIGNING_KEY_ENV,
    "OAUTH_STATE_SIGNING_KEY",
    CREDENTIAL_KEY_ENV,
];

pub const RUN_TOKEN_SECRETS: [&str; 1] = [RUN_TOKEN_SIGNING_KEY_ENV];

pub const CHANNEL_SECRETS: [&str; 4] = [
    RUN_TOKEN_SIGNING_KEY_ENV,
    "GITHUB_APP_ID",
    "GITHUB_APP_PRIVATE_KEY",
    CREDENTIAL_KEY_ENV,
];

pub const WORKER_SECRETS: [&str; 4] = [
    "GITHUB_APP_ID",
    "GITHUB_APP_PRIVATE_KEY",
    RUN_TOKEN_SIGNING_KEY_ENV,
    RELAY_TOKEN_SIGNING_KEY_ENV,
];

#[derive(Debug, Clone)]
pub enum Secrets {
    Env,
    Loaded(HashMap<&'static str, String>),
}

fn parameter_name(name: &str) -> &'static str {
    SECRET_PARAMETERS
        .iter()
        .find(|(env, _)| *env == name)
        .map(|(_, parameter)| *parameter)
        .unwrap_or_else(|| panic!("{name} is not in SECRET_PARAMETERS"))
}

impl Secrets {
    pub async fn from_ssm(
        sdk: &aws_config::SdkConfig,
        names: &[&'static str],
    ) -> Result<Self, ConfigError> {
        let mut env = Reader::new();
        let path = env.get(SECRETS_PATH_ENV);
        env.finish()?;
        if !(path.starts_with('/') && path.ends_with('/')) {
            return Err(ConfigError::Invalid {
                name: SECRETS_PATH_ENV,
                message: format!("{path:?} must start and end with /"),
            });
        }
        let by_parameter = names
            .iter()
            .map(|name| (format!("{path}{}", parameter_name(name)), *name))
            .collect::<HashMap<_, _>>();
        let mut parameters = by_parameter.keys().cloned().collect::<Vec<_>>();
        parameters.sort();
        let client = aws_sdk_ssm::Client::new(sdk);
        let mut values = HashMap::new();
        let mut missing = Vec::new();
        for batch in parameters.chunks(SSM_BATCH) {
            let output = client
                .get_parameters()
                .set_names(Some(batch.to_vec()))
                .with_decryption(true)
                .send()
                .await
                .map_err(|error| ConfigError::Invalid {
                    name: SECRETS_PATH_ENV,
                    message: format!("read the SSM parameters: {}", DisplayErrorContext(&error)),
                })?;
            for parameter in output.parameters() {
                let (Some(name), Some(value)) = (parameter.name(), parameter.value()) else {
                    continue;
                };
                if let Some(env_name) = by_parameter.get(name) {
                    values.insert(*env_name, value.to_string());
                }
            }
            missing.extend(output.invalid_parameters().iter().cloned());
        }
        missing.sort();
        if !missing.is_empty() {
            return Err(ConfigError::MissingParameters(missing.join(", ")));
        }
        Ok(Self::Loaded(values))
    }
}

#[derive(Debug, Clone)]
pub struct CognitoConfig {
    pub user_pool_id: String,
    pub client_id: String,
}

#[derive(Debug, Clone)]
pub struct GitHubAppConfig {
    pub app_id: String,
    pub private_key: String,
    pub api_url: String,
}

impl GitHubAppConfig {
    pub fn load(secrets: &Secrets) -> Result<Self, ConfigError> {
        let mut env = Reader::new();
        let app_id = env.secret(secrets, "GITHUB_APP_ID");
        let private_key = env.secret(secrets, "GITHUB_APP_PRIVATE_KEY");
        let api_url = env.get("GITHUB_API_URL");
        env.finish()?;
        Ok(Self {
            app_id,
            private_key,
            api_url: url("GITHUB_API_URL", &api_url)?,
        })
    }

    pub fn client(&self) -> Result<GitHubApi, ConfigError> {
        let auth = AppAuth::new(&self.app_id, &self.private_key).map_err(|error| {
            ConfigError::Invalid {
                name: "GITHUB_APP_PRIVATE_KEY",
                message: error.to_string(),
            }
        })?;
        Ok(GitHubApi::new(
            &self.api_url,
            Arc::new(ReqwestTransport::new()),
            auth,
        ))
    }
}

#[derive(Debug, Clone)]
pub struct GitHubConfig {
    pub webhook_secret: HmacKey,
    pub client_id: String,
    pub client_secret: String,
    pub app_slug: String,
    pub web_url: String,
    pub api_url: String,
}

#[derive(Debug, Clone)]
pub struct StripeConfig {
    pub webhook_secret: HmacKey,
    pub secret_key: String,
    pub api_url: String,
    pub portal_configuration: String,
}

#[derive(Debug, Clone)]
pub enum RecordsConfig {
    S3 { bucket: String },
    Local { dir: PathBuf, signing_key: HmacKey },
}

#[derive(Clone)]
pub struct CredentialKey(pub Arc<AesGcmEncryptor>);

impl std::fmt::Debug for CredentialKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialKey(..)")
    }
}

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub public_url: String,
    pub cognito: CognitoConfig,
    pub github: GitHubConfig,
    pub github_app: GitHubAppConfig,
    pub stripe: StripeConfig,
    pub task_tokens: TaskTokenSigner,
    pub oauth_state_key: HmacKey,
    pub credential_key: CredentialKey,
    pub records: RecordsConfig,
}

#[derive(Debug, Clone)]
pub struct ChannelConfig {
    pub run_tokens: RunTokenSigner,
    pub records: RecordsConfig,
}

#[derive(Debug, Clone)]
pub struct LocalConfig {
    pub bind_addr: SocketAddr,
    pub docs_dir: PathBuf,
    pub frontend_dir: PathBuf,
    pub api: ApiConfig,
    pub channel: ChannelConfig,
}

pub struct Reader {
    missing: Vec<&'static str>,
}

impl Default for Reader {
    fn default() -> Self {
        Self::new()
    }
}

fn from_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

impl Reader {
    pub fn new() -> Self {
        Self {
            missing: Vec::new(),
        }
    }

    fn take(&mut self, name: &'static str, value: Option<String>) -> String {
        value.unwrap_or_else(|| {
            self.missing.push(name);
            String::new()
        })
    }

    pub fn get(&mut self, name: &'static str) -> String {
        self.take(name, from_env(name))
    }

    pub fn secret(&mut self, secrets: &Secrets, name: &'static str) -> String {
        let value = match secrets {
            Secrets::Env => from_env(name),
            Secrets::Loaded(values) => values.get(name).filter(|value| !value.is_empty()).cloned(),
        };
        self.take(name, value)
    }

    pub fn finish(self) -> Result<(), ConfigError> {
        if self.missing.is_empty() {
            return Ok(());
        }
        Err(ConfigError::Missing(self.missing.join(", ")))
    }
}

fn key(name: &'static str, value: &str) -> Result<HmacKey, ConfigError> {
    HmacKey::new(value.as_bytes()).map_err(|message| ConfigError::Invalid { name, message })
}

fn strong_key(name: &'static str, value: &str) -> Result<HmacKey, ConfigError> {
    HmacKey::strong(value.as_bytes()).map_err(|message| ConfigError::Invalid { name, message })
}

pub fn signer(name: &'static str, value: &str) -> Result<Signer, ConfigError> {
    Signer::new(value.as_bytes()).map_err(|error| ConfigError::Invalid {
        name,
        message: error.to_string(),
    })
}

fn url(name: &'static str, value: &str) -> Result<String, ConfigError> {
    let trimmed = value.trim_end_matches('/');
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(ConfigError::Invalid {
            name,
            message: format!("{value:?} is not an http(s) URL"),
        });
    }
    Ok(trimmed.to_string())
}

pub const ALLOWED_SOURCES_ENV: &str = "SANCHORIS_ALLOWED_SOURCES";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowedSources {
    Any,
    Cloudflare,
}

impl AllowedSources {
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        match value {
            "any" => Ok(Self::Any),
            "cloudflare" => Ok(Self::Cloudflare),
            _ => Err(ConfigError::Invalid {
                name: ALLOWED_SOURCES_ENV,
                message: format!("{value:?} is neither any nor cloudflare"),
            }),
        }
    }

    pub fn load() -> Result<Self, ConfigError> {
        let mut env = Reader::new();
        let value = env.get(ALLOWED_SOURCES_ENV);
        env.finish()?;
        Self::parse(&value)
    }

    pub fn admits(self, source: Option<IpAddr>) -> bool {
        match self {
            Self::Any => true,
            Self::Cloudflare => source.is_some_and(is_cloudflare),
        }
    }
}

pub fn credential_key_of(value: &str) -> Result<CredentialKey, ConfigError> {
    Ok(CredentialKey(Arc::new(
        AesGcmEncryptor::from_base64(value).map_err(|message| ConfigError::Invalid {
            name: CREDENTIAL_KEY_ENV,
            message,
        })?,
    )))
}

pub fn credential_key(secrets: &Secrets) -> Result<CredentialKey, ConfigError> {
    let mut env = Reader::new();
    let value = env.secret(secrets, CREDENTIAL_KEY_ENV);
    env.finish()?;
    credential_key_of(&value)
}

pub fn run_tokens(secrets: &Secrets) -> Result<RunTokenSigner, ConfigError> {
    let mut env = Reader::new();
    let run_key = env.secret(secrets, RUN_TOKEN_SIGNING_KEY_ENV);
    env.finish()?;
    Ok(RunTokenSigner::new(signer(
        RUN_TOKEN_SIGNING_KEY_ENV,
        &run_key,
    )?))
}

impl ApiConfig {
    pub fn load(secrets: &Secrets) -> Result<Self, ConfigError> {
        let mut env = Reader::new();
        let public_url = env.get("SANCHORIS_PUBLIC_URL");
        let user_pool_id = env.get("COGNITO_USER_POOL_ID");
        let client_id = env.get("COGNITO_CLIENT_ID");
        let github_url = env.get("GITHUB_URL");
        let github_api_url = env.get("GITHUB_API_URL");
        let stripe_api_url = env.get("STRIPE_API_URL");
        let stripe_portal_configuration = env.get("STRIPE_PORTAL_CONFIGURATION");
        let webhook_secret = env.secret(secrets, "GITHUB_WEBHOOK_SECRET");
        let github_client_id = env.secret(secrets, "GITHUB_APP_CLIENT_ID");
        let github_client_secret = env.secret(secrets, "GITHUB_APP_CLIENT_SECRET");
        let github_app_slug = env.secret(secrets, "GITHUB_APP_SLUG");
        let stripe_secret_key = env.secret(secrets, "STRIPE_SECRET_KEY");
        let stripe_webhook_secret = env.secret(secrets, "STRIPE_WEBHOOK_SECRET");
        let task_key = env.secret(secrets, TASK_TOKEN_SIGNING_KEY_ENV);
        let state_key = env.secret(secrets, "OAUTH_STATE_SIGNING_KEY");
        let credential_key = env.secret(secrets, CREDENTIAL_KEY_ENV);
        env.finish()?;
        Ok(Self {
            public_url: url("SANCHORIS_PUBLIC_URL", &public_url)?,
            cognito: CognitoConfig {
                user_pool_id,
                client_id,
            },
            github: GitHubConfig {
                webhook_secret: key("GITHUB_WEBHOOK_SECRET", &webhook_secret)?,
                client_id: github_client_id,
                client_secret: github_client_secret,
                app_slug: github_app_slug,
                web_url: url("GITHUB_URL", &github_url)?,
                api_url: url("GITHUB_API_URL", &github_api_url)?,
            },
            github_app: GitHubAppConfig::load(secrets)?,
            stripe: StripeConfig {
                webhook_secret: key("STRIPE_WEBHOOK_SECRET", &stripe_webhook_secret)?,
                secret_key: stripe_secret_key,
                api_url: url("STRIPE_API_URL", &stripe_api_url)?,
                portal_configuration: stripe_portal_configuration,
            },
            task_tokens: TaskTokenSigner::new(signer(TASK_TOKEN_SIGNING_KEY_ENV, &task_key)?),
            oauth_state_key: strong_key("OAUTH_STATE_SIGNING_KEY", &state_key)?,
            credential_key: credential_key_of(&credential_key)?,
            records: RecordsConfig::from_env()?,
        })
    }
}

impl RecordsConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut env = Reader::new();
        let store = env.get("SANCHORIS_RECORDS_STORE");
        env.finish()?;
        match store.as_str() {
            "s3" => {
                let mut env = Reader::new();
                let bucket = env.get("SANCHORIS_RECORDS_BUCKET");
                env.finish()?;
                Ok(Self::S3 { bucket })
            }
            "local" => {
                let mut env = Reader::new();
                let dir = env.get("SANCHORIS_RECORDS_DIR");
                let signing_key = env.get("RECORDS_SIGNING_KEY");
                env.finish()?;
                Ok(Self::Local {
                    dir: PathBuf::from(dir),
                    signing_key: strong_key("RECORDS_SIGNING_KEY", &signing_key)?,
                })
            }
            other => Err(ConfigError::Invalid {
                name: "SANCHORIS_RECORDS_STORE",
                message: format!("{other:?} is not s3 or local"),
            }),
        }
    }
}

impl ChannelConfig {
    pub fn load(secrets: &Secrets) -> Result<Self, ConfigError> {
        Ok(Self {
            run_tokens: run_tokens(secrets)?,
            records: RecordsConfig::from_env()?,
        })
    }
}

impl LocalConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut env = Reader::new();
        let bind_addr = env.get("SANCHORIS_BIND_ADDR");
        let docs_dir = env.get("SANCHORIS_DOCS_DIR");
        let frontend_dir = env.get("SANCHORIS_FRONTEND_DIR");
        env.finish()?;
        Ok(Self {
            bind_addr: bind_addr.parse().map_err(|error| ConfigError::Invalid {
                name: "SANCHORIS_BIND_ADDR",
                message: format!("{bind_addr:?}: {error}"),
            })?,
            docs_dir: PathBuf::from(docs_dir),
            frontend_dir: PathBuf::from(frontend_dir),
            api: ApiConfig::load(&Secrets::Env)?,
            channel: ChannelConfig::load(&Secrets::Env)?,
        })
    }
}

pub fn require(names: &[&'static str]) -> Result<(), ConfigError> {
    let mut env = Reader::new();
    for name in names {
        env.get(name);
    }
    env.finish()
}

pub const STORE_ENV: [&str; 2] = ["SANCHORIS_TABLE_MAIN", "SANCHORIS_TABLE_AUDIT"];

pub const WORKER_ENV: [&str; 9] = [
    "GITHUB_APP_ID",
    "GITHUB_APP_PRIVATE_KEY",
    "GITHUB_API_URL",
    RUN_TOKEN_SIGNING_KEY_ENV,
    "SANCHORIS_WORKSPACE_COMMAND",
    "SANCHORIS_CONTROL_URL",
    "SANCHORIS_SOCKET_DIR",
    "SANCHORIS_WORKSPACES_DIR",
    "SANCHORIS_GIT_URL",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_sources_admit_cloudflare_only_when_set_to_cloudflare() {
        let cloudflare = AllowedSources::parse("cloudflare").unwrap();
        assert!(cloudflare.admits(Some("172.70.1.2".parse().unwrap())));
        assert!(!cloudflare.admits(Some("203.0.113.7".parse().unwrap())));
        assert!(!cloudflare.admits(None));
        let any = AllowedSources::parse("any").unwrap();
        assert!(any.admits(Some("203.0.113.7".parse().unwrap())));
        assert!(any.admits(None));
        assert!(AllowedSources::parse("true").is_err());
    }

    #[test]
    fn every_function_secret_has_a_parameter() {
        for name in API_SECRETS
            .iter()
            .chain(RUN_TOKEN_SECRETS.iter())
            .chain(CHANNEL_SECRETS.iter())
            .chain(WORKER_SECRETS.iter())
        {
            assert!(!parameter_name(name).is_empty());
        }
    }

    #[test]
    fn loaded_secrets_are_read_from_the_map_not_the_environment() {
        let secrets = Secrets::Loaded(HashMap::from([(
            "GITHUB_APP_SLUG",
            "sanchoris".to_string(),
        )]));
        let mut env = Reader::new();
        assert_eq!(env.secret(&secrets, "GITHUB_APP_SLUG"), "sanchoris");
        env.secret(&secrets, "STRIPE_SECRET_KEY");
        assert_eq!(
            env.finish().unwrap_err().to_string(),
            "missing required environment variables: STRIPE_SECRET_KEY"
        );
    }
}
