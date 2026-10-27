use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::auth::{CurrentUser, IdentityVerifier};
use crate::channel::{self, ChannelRegistry};
use crate::config::ServeConfig;
use crate::records::{LocalRecordStore, RecordStore};
use crate::{github_oauth, tasks, webhooks};

const MAX_WEBHOOK_BYTES: usize = 25 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<ServeConfig>,
    pub verifier: Arc<dyn IdentityVerifier>,
    pub records: Arc<dyn RecordStore>,
    pub http: reqwest::Client,
    pub channels: Arc<ChannelRegistry>,
}

impl AppState {
    pub fn new(
        pool: PgPool,
        config: ServeConfig,
        verifier: Arc<dyn IdentityVerifier>,
        records: Arc<dyn RecordStore>,
        http: reqwest::Client,
    ) -> Self {
        Self {
            pool,
            config: Arc::new(config),
            verifier,
            records,
            http,
            channels: Arc::new(ChannelRegistry::default()),
        }
    }
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn me(user: CurrentUser) -> Json<Value> {
    Json(json!({
        "id": user.user_id,
        "cognitoSubject": user.cognito_subject,
        "displayName": user.display_name,
        "email": user.email,
        "githubLogin": user.github_login,
    }))
}

pub fn router(state: AppState, local_records: Option<Arc<LocalRecordStore>>) -> Router {
    let webhooks = Router::new()
        .route("/webhooks/github", post(webhooks::github))
        .route("/webhooks/stripe", post(webhooks::stripe))
        .layer(DefaultBodyLimit::max(MAX_WEBHOOK_BYTES));
    let router = Router::new()
        .route("/health", get(health))
        .route("/api/me", get(me))
        .route("/api/approvals", post(tasks::resolve_approval))
        .route("/api/tasks/{task_id}/replies", post(tasks::reply))
        .route("/auth/github", post(github_oauth::start))
        .route("/auth/github/callback", get(github_oauth::callback))
        .route("/runs/channel", get(channel::upgrade))
        .merge(webhooks)
        .with_state(state);
    match local_records {
        Some(store) => router.nest_service("/records", store.router()),
        None => router,
    }
}
