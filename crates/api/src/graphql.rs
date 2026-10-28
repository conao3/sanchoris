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

pub const CLOUDFLARE_RANGES: [&str; 22] = [
    "173.245.48.0/20",
    "103.21.244.0/22",
    "103.22.200.0/22",
    "103.31.4.0/22",
    "141.101.64.0/18",
    "108.162.192.0/18",
    "190.93.240.0/20",
    "188.114.96.0/20",
    "197.234.240.0/22",
    "198.41.128.0/17",
    "162.158.0.0/15",
    "104.16.0.0/13",
    "104.24.0.0/14",
    "172.64.0.0/13",
    "131.0.72.0/22",
    "2400:cb00::/32",
    "2606:4700::/32",
    "2803:f800::/32",
    "2405:b500::/32",
    "2405:8100::/32",
    "2a06:98c0::/29",
    "2c0f:f248::/32",
];

fn bits(ip: IpAddr) -> (u128, u32) {
    match ip {
        IpAddr::V4(v4) => (u128::from(u32::from(v4)), 32),
        IpAddr::V6(v6) => (u128::from(v6), 128),
    }
}

fn in_range(ip: IpAddr, range: &str) -> bool {
    let (network, prefix) = range.split_once('/').expect("a range is address/prefix");
    let network: IpAddr = network.parse().expect("a range starts with an address");
    let prefix: u32 = prefix.parse().expect("a range ends with a prefix length");
    let ((address, width), (network, network_width)) = (bits(ip), bits(network));
    if width != network_width {
        return false;
    }
    let shift = width - prefix;
    shift == width || address >> shift == network >> shift
}

pub fn is_cloudflare(ip: IpAddr) -> bool {
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    };
    CLOUDFLARE_RANGES.iter().any(|range| in_range(ip, range))
}

pub fn client_ip(source: IpAddr, cf_connecting_ip: Option<&str>) -> IpAddr {
    if !is_cloudflare(source) {
        return source;
    }
    cf_connecting_ip
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(source)
}

pub async fn gateway_ip(mut request: Request, next: Next) -> Response {
    if let Some(source) = request.request_context_ref().and_then(gateway_source_ip) {
        let forwarded = request
            .headers()
            .get("cf-connecting-ip")
            .and_then(|value| value.to_str().ok());
        let ip = client_ip(source, forwarded);
        request.extensions_mut().insert(ClientIp(ip));
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    #[test]
    fn every_cloudflare_range_parses_and_matches_its_own_network() {
        for range in CLOUDFLARE_RANGES {
            let network = ip(range.split_once('/').unwrap().0);
            assert!(is_cloudflare(network), "{range}");
        }
    }

    #[test]
    fn cloudflare_ranges_cover_their_edges_only() {
        assert!(is_cloudflare(ip("173.245.63.255")));
        assert!(!is_cloudflare(ip("173.245.64.0")));
        assert!(is_cloudflare(ip("2a06:98c7:ffff::1")));
        assert!(!is_cloudflare(ip("2a06:98c8::1")));
        assert!(is_cloudflare(ip("::ffff:104.16.0.1")));
        assert!(!is_cloudflare(ip("203.0.113.7")));
    }

    #[test]
    fn the_client_ip_trusts_cf_connecting_ip_only_from_cloudflare() {
        assert_eq!(
            client_ip(ip("172.70.1.2"), Some("198.51.100.4")),
            ip("198.51.100.4")
        );
        assert_eq!(
            client_ip(ip("203.0.113.7"), Some("198.51.100.4")),
            ip("203.0.113.7")
        );
        assert_eq!(client_ip(ip("172.70.1.2"), None), ip("172.70.1.2"));
        assert_eq!(
            client_ip(ip("172.70.1.2"), Some("not-an-ip")),
            ip("172.70.1.2")
        );
    }
}
