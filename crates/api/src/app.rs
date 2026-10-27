use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use sanchoris_core::db::Store;
use sanchoris_graphql::SanchorisSchema;
use sanchoris_worker::github::GitHubApi;
use serde_json::{Value, json};

use crate::auth::{CurrentUser, IdentityVerifier};
use crate::config::ApiConfig;
use crate::frontend::{self, Frontend};
use crate::{github_oauth, graphql, tasks, webhooks};

const MAX_WEBHOOK_BYTES: usize = 25 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub config: Arc<ApiConfig>,
    pub verifier: Arc<dyn IdentityVerifier>,
    pub http: reqwest::Client,
    pub github_app: Arc<GitHubApi>,
    pub schema: SanchorisSchema,
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

pub fn router(state: AppState, frontend: Frontend) -> Router {
    let webhooks = Router::new()
        .route("/webhooks/github", post(webhooks::github))
        .route("/webhooks/stripe", post(webhooks::stripe))
        .layer(DefaultBodyLimit::max(MAX_WEBHOOK_BYTES));
    let spa = Router::new().fallback(frontend::serve).with_state(frontend);
    Router::new()
        .route("/health", get(health))
        .route("/api/me", get(me))
        .route("/api/graphql", post(graphql::execute))
        .route("/api/approvals", post(tasks::resolve_approval))
        .route("/api/tasks/{task_id}/replies", post(tasks::reply))
        .route("/auth/github", post(github_oauth::start))
        .route("/auth/github/callback", get(github_oauth::callback))
        .route("/auth/github/setup", get(github_oauth::setup))
        .merge(webhooks)
        .with_state(state)
        .fallback_service(spa)
}

pub fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| format!("build the HTTP client: {error}"))
}
