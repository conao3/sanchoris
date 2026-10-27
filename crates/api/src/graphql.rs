use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use axum::{Extension, Json};
use lambda_http::RequestExt;
use lambda_http::request::RequestContext;
use sanchoris_core::db::Store;
use sanchoris_graphql::{AesGcmEncryptor, AuthUser, ClientIp, GraphqlDeps, SystemClock};
use sanchoris_worker::Worker;
use sanchoris_worker::github::GitHubApi;

use crate::app::AppState;
use crate::auth::CurrentUser;
use crate::config::ApiConfig;
use crate::github_oauth::GitHubLinks;
use crate::ports::{
    GitHubConfigRepository, InlineChatDispatcher, InlineConfigSync, RecordsAuditExport,
    SlackUnavailable,
};
use crate::records::RecordStore;
use crate::stripe::StripeCheckout;

pub struct GraphqlPorts {
    pub store: Store,
    pub config: Arc<ApiConfig>,
    pub http: reqwest::Client,
    pub worker: Arc<Worker>,
    pub github_app: Arc<GitHubApi>,
    pub records: Arc<dyn RecordStore>,
}

pub fn deps(ports: GraphqlPorts) -> GraphqlDeps {
    let encryptor: Arc<AesGcmEncryptor> = ports.config.credential_key.0.clone();
    GraphqlDeps {
        checkout: Arc::new(StripeCheckout::new(
            ports.http,
            ports.config.clone(),
            ports.store,
        )),
        github: Arc::new(GitHubLinks::new(ports.config)),
        slack: Arc::new(SlackUnavailable),
        config_sync: Arc::new(InlineConfigSync::new(ports.worker.clone())),
        config_repository: Arc::new(GitHubConfigRepository::new(
            ports.worker.clone(),
            ports.github_app,
        )),
        chat: Arc::new(InlineChatDispatcher::new(ports.worker)),
        audit_export: Arc::new(RecordsAuditExport::new(ports.records)),
        encryptor,
        clock: Arc::new(SystemClock),
    }
}

pub async fn execute(
    State(state): State<AppState>,
    user: CurrentUser,
    Extension(ip): Extension<ClientIp>,
    Json(request): Json<async_graphql::Request>,
) -> Json<async_graphql::Response> {
    let request = request
        .data(AuthUser {
            user_id: user.user_id,
            cognito_subject: user.cognito_subject,
            email: user.email,
        })
        .data(ip);
    Json(state.schema.execute(request).await)
}

pub async fn peer_ip(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    mut request: Request,
    next: Next,
) -> Response {
    request.extensions_mut().insert(ClientIp(peer.ip()));
    next.run(request).await
}

pub fn gateway_source_ip(context: &RequestContext) -> Option<IpAddr> {
    match context {
        RequestContext::ApiGatewayV2(context) => context.http.source_ip.as_deref()?.parse().ok(),
        _ => None,
    }
}

pub async fn gateway_ip(mut request: Request, next: Next) -> Response {
    if let Some(ip) = request.request_context_ref().and_then(gateway_source_ip) {
        request.extensions_mut().insert(ClientIp(ip));
    }
    next.run(request).await
}
