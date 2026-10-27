use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use sanchoris_graphql::{Encryptor, GraphqlDeps, SanchorisSchema, SystemClock, build_schema};
use sanchoris_worker::github::GitHubApi;
use sanchoris_worker::{Deps, Settings, Worker};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::auth::{CurrentUser, IdentityVerifier};
use crate::channel::{self, ChannelRegistry};
use crate::config::ServeConfig;
use crate::github_oauth::GitHubLinks;
use crate::ports::{
    GitHubConfigRepository, InlineChatDispatcher, InlineConfigSync, RecordsAuditExport,
    SlackUnavailable,
};
use crate::records::{LocalRecordStore, RecordStore};
use crate::stripe::StripeCheckout;
use crate::{github_oauth, graphql, tasks, webhooks};

const MAX_WEBHOOK_BYTES: usize = 25 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<ServeConfig>,
    pub verifier: Arc<dyn IdentityVerifier>,
    pub records: Arc<dyn RecordStore>,
    pub http: reqwest::Client,
    pub channels: Arc<ChannelRegistry>,
    pub github_app: Arc<GitHubApi>,
    pub schema: SanchorisSchema,
}

pub struct Services {
    pub pool: PgPool,
    pub app_pool: PgPool,
    pub config: Arc<ServeConfig>,
    pub verifier: Arc<dyn IdentityVerifier>,
    pub records: Arc<dyn RecordStore>,
    pub http: reqwest::Client,
    pub worker_deps: Deps,
    pub github_app: Arc<GitHubApi>,
    pub encryptor: Arc<dyn Encryptor>,
}

impl AppState {
    pub fn new(services: Services) -> Self {
        let worker = Arc::new(Worker::new(
            services.pool.clone(),
            services.worker_deps,
            Settings::default(),
        ));
        let deps = GraphqlDeps {
            checkout: Arc::new(StripeCheckout::new(
                services.http.clone(),
                services.config.clone(),
                services.pool.clone(),
            )),
            github: Arc::new(GitHubLinks::new(services.config.clone())),
            slack: Arc::new(SlackUnavailable),
            config_sync: Arc::new(InlineConfigSync::new(worker.clone())),
            config_repository: Arc::new(GitHubConfigRepository::new(
                worker.clone(),
                services.github_app.clone(),
            )),
            chat: Arc::new(InlineChatDispatcher::new(worker)),
            audit_export: Arc::new(RecordsAuditExport::new(services.records.clone())),
            encryptor: services.encryptor,
            clock: Arc::new(SystemClock),
        };
        Self {
            pool: services.pool,
            config: services.config,
            verifier: services.verifier,
            records: services.records,
            http: services.http,
            channels: Arc::new(ChannelRegistry::default()),
            github_app: services.github_app,
            schema: build_schema(services.app_pool, deps),
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
        .route("/api/graphql", post(graphql::execute))
        .route("/api/approvals", post(tasks::resolve_approval))
        .route("/api/tasks/{task_id}/replies", post(tasks::reply))
        .route("/auth/github", post(github_oauth::start))
        .route("/auth/github/callback", get(github_oauth::callback))
        .route("/auth/github/setup", get(github_oauth::setup))
        .route("/runs/channel", get(channel::upgrade))
        .merge(webhooks)
        .with_state(state);
    match local_records {
        Some(store) => router.nest_service("/records", store.router()),
        None => router,
    }
}
