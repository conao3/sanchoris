use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::request::Parts;
use sanchoris_graphql::{AuthUser, ClientIp};

use crate::app::AppState;
use crate::auth::CurrentUser;

pub struct PeerIp(pub Option<ClientIp>);

impl<S: Send + Sync> FromRequestParts<S> for PeerIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|info| ClientIp(info.0.ip())),
        ))
    }
}

pub async fn execute(
    State(state): State<AppState>,
    user: CurrentUser,
    PeerIp(ip): PeerIp,
    Json(request): Json<async_graphql::Request>,
) -> Json<async_graphql::Response> {
    let mut request = request.data(AuthUser {
        user_id: user.user_id,
        cognito_subject: user.cognito_subject,
        email: user.email,
    });
    if let Some(ip) = ip {
        request = request.data(ip);
    }
    Json(state.schema.execute(request).await)
}
