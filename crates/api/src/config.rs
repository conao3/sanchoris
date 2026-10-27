use std::net::SocketAddr;
use std::path::PathBuf;

use sanchoris_core::tokens::{
    RUN_TOKEN_SIGNING_KEY_ENV, RunTokenSigner, Signer, TASK_TOKEN_SIGNING_KEY_ENV, TaskTokenSigner,
};
use thiserror::Error;

use crate::signing::HmacKey;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required environment variables: {0}")]
    Missing(String),
    #[error("{name}: {message}")]
    Invalid { name: &'static str, message: String },
}

#[derive(Debug, Clone)]
pub struct CognitoConfig {
    pub user_pool_id: String,
    pub client_id: String,
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
}

#[derive(Debug, Clone)]
pub enum RecordsConfig {
    S3 {
        bucket: String,
        kms_key_prefix: String,
    },
    Local {
        dir: PathBuf,
        signing_key: HmacKey,
    },
}

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub public_url: String,
    pub cognito: CognitoConfig,
    pub github: GitHubConfig,
    pub stripe: StripeConfig,
    pub task_tokens: TaskTokenSigner,
    pub oauth_state_key: HmacKey,
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

impl Reader {
    pub fn new() -> Self {
        Self {
            missing: Vec::new(),
        }
    }

    pub fn get(&mut self, name: &'static str) -> String {
        match std::env::var(name) {
            Ok(value) if !value.is_empty() => value,
            _ => {
                self.missing.push(name);
                String::new()
            }
        }
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

impl ApiConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut env = Reader::new();
        let public_url = env.get("SANCHORIS_PUBLIC_URL");
        let user_pool_id = env.get("COGNITO_USER_POOL_ID");
        let client_id = env.get("COGNITO_CLIENT_ID");
        let webhook_secret = env.get("GITHUB_WEBHOOK_SECRET");
        let github_client_id = env.get("GITHUB_APP_CLIENT_ID");
        let github_client_secret = env.get("GITHUB_APP_CLIENT_SECRET");
        let github_app_slug = env.get("GITHUB_APP_SLUG");
        let github_url = env.get("GITHUB_URL");
        let github_api_url = env.get("GITHUB_API_URL");
        let stripe_secret = env.get("STRIPE_WEBHOOK_SECRET");
        let task_key = env.get(TASK_TOKEN_SIGNING_KEY_ENV);
        let state_key = env.get("OAUTH_STATE_SIGNING_KEY");
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
            stripe: StripeConfig {
                webhook_secret: key("STRIPE_WEBHOOK_SECRET", &stripe_secret)?,
            },
            task_tokens: TaskTokenSigner::new(signer(TASK_TOKEN_SIGNING_KEY_ENV, &task_key)?),
            oauth_state_key: strong_key("OAUTH_STATE_SIGNING_KEY", &state_key)?,
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
                let bucket = env.get("S3_RECORDS_BUCKET");
                let kms_key_prefix = env.get("KMS_KEY_PREFIX");
                env.finish()?;
                Ok(Self::S3 {
                    bucket,
                    kms_key_prefix,
                })
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
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut env = Reader::new();
        let run_key = env.get(RUN_TOKEN_SIGNING_KEY_ENV);
        env.finish()?;
        Ok(Self {
            run_tokens: RunTokenSigner::new(signer(RUN_TOKEN_SIGNING_KEY_ENV, &run_key)?),
            records: RecordsConfig::from_env()?,
        })
    }
}

impl LocalConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut env = Reader::new();
        let bind_addr = env.get("SANCHORIS_BIND_ADDR");
        let docs_dir = env.get("SANCHORIS_DOCS_DIR");
        env.finish()?;
        Ok(Self {
            bind_addr: bind_addr.parse().map_err(|error| ConfigError::Invalid {
                name: "SANCHORIS_BIND_ADDR",
                message: format!("{bind_addr:?}: {error}"),
            })?,
            docs_dir: PathBuf::from(docs_dir),
            api: ApiConfig::from_env()?,
            channel: ChannelConfig::from_env()?,
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
