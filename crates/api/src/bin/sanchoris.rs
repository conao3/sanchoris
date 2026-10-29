use std::process::ExitCode;
use std::sync::Arc;

use sanchoris_api::app::{AppState, http_client};
use sanchoris_api::auth::cognito::CognitoVerifier;
use sanchoris_api::channel::credentials::IssuedCredentials;
use sanchoris_api::config::{LocalConfig, RecordsConfig, STORE_ENV, WORKER_ENV, require};
use sanchoris_api::frontend::Frontend;
use sanchoris_api::graphql::{self, GraphqlPorts};
use sanchoris_api::records::{LocalRecordStore, RecordStore, S3RecordStore, record_blobs};
use sanchoris_api::server::{LocalParts, LocalServer, local_channel, serve};
use sanchoris_core::clock::{Clock, SystemClock};
use sanchoris_core::db::Store;
use sanchoris_core::local::{DirBlobStore, MemoryQueue, MemoryScheduler};
use sanchoris_core::store::Relay;
use sanchoris_worker::resources::GitHubResources;
use sanchoris_worker::runner::{LocalProcessRunner, LocalRunnerConfig};
use sanchoris_worker::{Deps, Settings, Worker};
use tokio::sync::watch;

const USAGE: &str = "Usage: sanchoris all

  all  Run the HTTP API, the Run channel, the worker, and the built frontend
       in one process against DynamoDB Local (local development)";

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
    let frontend = Frontend::dir(config.frontend_dir.clone())?;
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
            clock: clock.clone(),
        },
    )
    .map_err(|error| error.to_string())?;
    let (records, local_records): (Arc<dyn RecordStore>, Option<Arc<LocalRecordStore>>) =
        match &config.channel.records {
            RecordsConfig::S3 { bucket } => (Arc::new(S3RecordStore::new(&sdk, bucket)), None),
            RecordsConfig::Local { dir, signing_key } => {
                let local = Arc::new(LocalRecordStore::new(
                    dir,
                    signing_key.clone(),
                    &config.api.public_url,
                ));
                (local.clone(), Some(local))
            }
        };
    let github_app = Arc::new(
        config
            .api
            .github_app
            .client()
            .map_err(|error| error.to_string())?,
    );
    let channel = local_channel(
        store.clone(),
        config.channel.run_tokens.clone(),
        records.clone(),
        Arc::new(IssuedCredentials::new(
            store.clone(),
            github_app.clone(),
            config.api.credential_key.0.clone(),
        )),
    );
    let worker = Arc::new(Worker::new(
        store.clone(),
        Deps {
            clock: clock.clone(),
            github: github_app.clone(),
            reader: github_app.clone(),
            config_source: github_app.clone(),
            resources: Arc::new(GitHubResources::new(github_app.clone())),
            runner: Arc::new(LocalProcessRunner::new(LocalRunnerConfig::from_env())),
            pusher: Arc::new(channel.channel.clone()),
            records: record_blobs(&config.channel.records, &sdk),
        },
        Settings::default(),
    ));
    let http = http_client()?;
    let verifier = CognitoVerifier::new(config.api.cognito.clone(), http.clone())
        .map_err(|error| format!("COGNITO_USER_POOL_ID: {error}"))?;
    let api = Arc::new(config.api);
    let schema = sanchoris_graphql::build_schema(
        store.clone(),
        graphql::deps(GraphqlPorts {
            store: store.clone(),
            config: api.clone(),
            http: http.clone(),
            worker: worker.clone(),
            github_app: github_app.clone(),
            records,
        }),
    );
    let server = LocalServer::new(LocalParts {
        state: AppState {
            store,
            config: api,
            verifier: Arc::new(verifier),
            http,
            github_app,
            schema,
        },
        frontend,
        channel,
        local_records,
        queue,
        scheduler,
        worker,
    });
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
