use std::sync::Arc;

use sanchoris_api::app::{AppState, http_client, router};
use sanchoris_api::auth::cognito::CognitoVerifier;
use sanchoris_api::config::{ApiConfig, require};
use sanchoris_api::lambda;
use sanchoris_worker::github::GitHubApi;

#[tokio::main]
async fn main() -> Result<(), lambda_http::Error> {
    let config = ApiConfig::from_env()?;
    require(&["GITHUB_APP_ID", "GITHUB_APP_PRIVATE_KEY"])?;
    let sdk = aws_config::load_from_env().await;
    let http = http_client()?;
    let verifier = CognitoVerifier::new(config.cognito.clone(), http.clone())
        .map_err(|error| format!("COGNITO_USER_POOL_ID: {error}"))?;
    let state = AppState {
        store: lambda::store(&sdk)?,
        config: Arc::new(config),
        verifier: Arc::new(verifier),
        http,
        github_app: Arc::new(GitHubApi::from_env()),
    };
    lambda_http::run(router(state)).await
}
