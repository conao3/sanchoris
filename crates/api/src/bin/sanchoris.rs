use std::process::ExitCode;
use std::sync::Arc;

use sanchoris_api::app::{AppState, http_client};
use sanchoris_api::auth::cognito::CognitoVerifier;
use sanchoris_api::config::{LocalConfig, RecordsConfig, STORE_ENV, WORKER_ENV, require};
use sanchoris_api::records::{LocalRecordStore, RecordStore, S3RecordStore};
use sanchoris_api::server::{LocalServer, local_channel, serve};
use sanchoris_core::clock::{Clock, SystemClock};
use sanchoris_core::db::Store;
use sanchoris_core::local::{DirBlobStore, MemoryQueue, MemoryScheduler};
use sanchoris_core::store::Relay;
use sanchoris_worker::{Settings, deps_from_env};
use tokio::sync::watch;

const USAGE: &str = "Usage: sanchoris all

  all  Run the HTTP API, the Run channel, and the worker in one process
       against DynamoDB Local (local development)";

async fn signals(sender: watch::Sender<bool>) {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install the SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
    let _ = sender.send(true);
}

async fn run() -> Result<(), String> {
    let config = LocalConfig::from_env().map_err(|error| error.to_string())?;
    require(&STORE_ENV).map_err(|error| error.to_string())?;
    require(&WORKER_ENV).map_err(|error| error.to_string())?;
    let sdk = aws_config::load_from_env().await;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let queue = Arc::new(MemoryQueue::new(clock.clone()));
    let scheduler = Arc::new(MemoryScheduler::new());
    let store = Store::from_env(
        &sdk,
        Arc::new(DirBlobStore::new(&config.docs_dir)),
        Relay::Direct {
            queue: queue.clone(),
            scheduler: scheduler.clone(),
            clock,
        },
    )
    .map_err(|error| error.to_string())?;
    let (records, local_records): (Arc<dyn RecordStore>, Option<Arc<LocalRecordStore>>) =
        match &config.channel.records {
            RecordsConfig::S3 {
                bucket,
                kms_key_prefix,
            } => (
                Arc::new(S3RecordStore::new(&sdk, bucket, kms_key_prefix)),
                None,
            ),
            RecordsConfig::Local { dir, signing_key } => {
                let local = Arc::new(LocalRecordStore::new(
                    dir,
                    signing_key.clone(),
                    &config.api.public_url,
                ));
                (local.clone(), Some(local))
            }
        };
    let channel = local_channel(store.clone(), config.channel.run_tokens.clone(), records);
    let (deps, github_app) = deps_from_env(Arc::new(channel.channel.clone()));
    let http = http_client()?;
    let verifier = CognitoVerifier::new(config.api.cognito.clone(), http.clone())
        .map_err(|error| format!("COGNITO_USER_POOL_ID: {error}"))?;
    let state = AppState {
        store,
        config: Arc::new(config.api),
        verifier: Arc::new(verifier),
        http,
        github_app,
    };
    let server = LocalServer::new(
        state,
        channel,
        local_records,
        queue,
        scheduler,
        deps,
        Settings::default(),
    );
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .map_err(|error| format!("bind {}: {error}", config.bind_addr))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("read the bound address: {error}"))?;
    let (sender, receiver) = watch::channel(false);
    tokio::spawn(signals(sender));
    println!("sanchoris listening on http://{address}");
    serve(listener, server, receiver).await
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args != ["all"] {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sanchoris: {error}");
            ExitCode::FAILURE
        }
    }
}
