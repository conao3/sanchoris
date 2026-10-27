use std::sync::Arc;

use sanchoris_api::app::{AppState, http_client, router};
use sanchoris_api::auth::cognito::CognitoVerifier;
use sanchoris_api::config::{API_SECRETS, ApiConfig, Secrets};
use sanchoris_api::frontend::Frontend;
use sanchoris_api::graphql::{self, GraphqlPorts};
use sanchoris_api::lambda;
use sanchoris_api::ports::ApiWorkspaces;
use sanchoris_api::records::s3_records;
use sanchoris_core::clock::SystemClock;
use sanchoris_worker::resources::GitHubResources;
use sanchoris_worker::{Deps, Settings, Worker};

#[tokio::main]
async fn main() -> Result<(), lambda_http::Error> {
    let sdk = aws_config::load_from_env().await;
    let secrets = Secrets::from_ssm(&sdk, &API_SECRETS).await?;
    let config = Arc::new(ApiConfig::load(&secrets)?);
    let frontend = Frontend::embedded()?;
    let store = lambda::store(&sdk)?;
    let http = http_client()?;
    let verifier = CognitoVerifier::new(config.cognito.clone(), http.clone())
        .map_err(|error| format!("COGNITO_USER_POOL_ID: {error}"))?;
    let github_app = Arc::new(config.github_app.client()?);
    let worker = Arc::new(Worker::new(
        store.clone(),
        Deps {
            clock: Arc::new(SystemClock),
            github: github_app.clone(),
            reader: github_app.clone(),
            config_source: github_app.clone(),
            resources: Arc::new(GitHubResources::new(github_app.clone())),
            runner: Arc::new(ApiWorkspaces),
            pusher: Arc::new(ApiWorkspaces),
        },
        Settings::default(),
    ));
    let schema = sanchoris_graphql::build_schema(
        store.clone(),
        graphql::deps(GraphqlPorts {
            store: store.clone(),
            config: config.clone(),
            http: http.clone(),
            worker,
            github_app: github_app.clone(),
            records: s3_records(&config.records, &sdk)?,
        }),
    );
    let state = AppState {
        store,
        config,
        verifier: Arc::new(verifier),
        http,
        github_app,
        schema,
    };
    lambda_http::run(router(state, frontend).layer(axum::middleware::from_fn(graphql::gateway_ip)))
        .await
}
