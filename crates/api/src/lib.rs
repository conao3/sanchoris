pub mod app;
pub mod auth;
pub mod channel;
pub mod chat;
pub mod config;
pub mod error;
pub mod github_oauth;
pub mod records;
pub mod signing;
pub mod tasks;
pub mod webhooks;

use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use tokio::sync::watch;

use crate::app::AppState;
use crate::auth::cognito::CognitoVerifier;
use crate::config::{RecordsConfig, ServeConfig};
use crate::records::{LocalRecordStore, RecordStore, S3RecordStore};

const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

async fn stopped(mut shutdown: watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

pub async fn serve(config: ServeConfig, shutdown: watch::Receiver<bool>) -> Result<(), String> {
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&config.database_url)
        .await
        .map_err(|error| format!("connect to DATABASE_URL: {error}"))?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("build the HTTP client: {error}"))?;
    let verifier = Arc::new(
        CognitoVerifier::new(config.cognito.clone(), http.clone())
            .map_err(|error| format!("COGNITO_USER_POOL_ID: {error}"))?,
    );
    let (records, local): (Arc<dyn RecordStore>, Option<Arc<LocalRecordStore>>) =
        match &config.records {
            RecordsConfig::S3 {
                region,
                bucket,
                kms_key_prefix,
            } => (
                Arc::new(S3RecordStore::new(region, bucket, kms_key_prefix).await),
                None,
            ),
            RecordsConfig::Local { dir, signing_key } => {
                let store = Arc::new(LocalRecordStore::new(
                    dir,
                    signing_key.clone(),
                    &config.public_url,
                ));
                (store.clone(), Some(store))
            }
        };
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .map_err(|error| format!("bind {}: {error}", config.bind_addr))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("read the bound address: {error}"))?;
    let state = AppState::new(pool.clone(), config, verifier, records, http);
    let router = app::router(state, local);
    let chat = tokio::spawn(chat::run(pool, shutdown.clone()));
    println!("sanchoris api listening on http://{address}");
    let server = axum::serve(listener, router).with_graceful_shutdown(stopped(shutdown.clone()));
    tokio::select! {
        result = server => result.map_err(|error| format!("serve: {error}"))?,
        _ = async {
            stopped(shutdown).await;
            tokio::time::sleep(SHUTDOWN_GRACE).await;
        } => {}
    }
    chat.await
        .map_err(|error| format!("the chat relay stopped: {error}"))
}

pub async fn worker(shutdown: watch::Receiver<bool>) {
    sanchoris_worker::run_from_env(shutdown).await;
}
