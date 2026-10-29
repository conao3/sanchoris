use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use sanchoris_core::db::Store;
use sanchoris_core::local::{MemoryQueue, MemoryScheduler};
use sanchoris_core::tokens::RunTokenSigner;
use sanchoris_worker::Worker;
use sanchoris_worker::driver::LocalDriver;
use sanchoris_worker::error::WorkerError;
use tokio::sync::watch;

use crate::app::{AppState, router};
use crate::channel::Channel;
use crate::channel::credentials::RunCredentials;
use crate::channel::local::{self as local_channel, LocalChannel, LocalConnections};
use crate::frontend::Frontend;
use crate::graphql;
use crate::records::{LocalRecordStore, RecordStore};

pub const QUEUE_INTERVAL: Duration = Duration::from_millis(100);
pub const PERIODIC_INTERVAL: Duration = Duration::from_secs(2);

pub fn local_channel(
    store: Store,
    run_tokens: RunTokenSigner,
    records: Arc<dyn RecordStore>,
    credentials: Arc<dyn RunCredentials>,
) -> LocalChannel {
    let connections = Arc::new(LocalConnections::default());
    LocalChannel {
        channel: Channel {
            store,
            run_tokens,
            relay_tokens: None,
            records,
            sender: connections.clone(),
            credentials,
        },
        connections,
    }
}

pub struct LocalServer {
    pub router: Router,
    pub driver: Arc<LocalDriver>,
}

impl LocalServer {
    pub fn new(parts: LocalParts) -> Self {
        let router =
            router(parts.state, parts.frontend).merge(local_channel::router(parts.channel));
        let router = match parts.local_records {
            Some(records) => router.nest_service("/records", records.router()),
            None => router,
        };
        Self {
            router: router.layer(axum::middleware::from_fn(graphql::peer_ip)),
            driver: Arc::new(LocalDriver::new(parts.worker, parts.queue, parts.scheduler)),
        }
    }
}

pub struct LocalParts {
    pub state: AppState,
    pub frontend: Frontend,
    pub channel: LocalChannel,
    pub local_records: Option<Arc<LocalRecordStore>>,
    pub queue: Arc<MemoryQueue>,
    pub scheduler: Arc<MemoryScheduler>,
    pub worker: Arc<Worker>,
}

fn report(result: Result<usize, WorkerError>) {
    if let Err(error) = result
        && !matches!(error, WorkerError::Injected(_))
    {
        eprintln!("sanchoris worker: {error}");
    }
}

pub async fn drive(driver: Arc<LocalDriver>, mut shutdown: watch::Receiver<bool>) {
    let mut queue = tokio::time::interval(QUEUE_INTERVAL);
    let mut periodic = tokio::time::interval(PERIODIC_INTERVAL);
    loop {
        tokio::select! {
            _ = queue.tick() => {
                report(driver.fire_due().await);
                report(driver.drain().await);
            }
            _ = periodic.tick() => {
                report(driver.run_periodic().await);
                report(driver.drain().await);
            }
            _ = shutdown.changed() => return,
        }
    }
}

async fn stopped(mut shutdown: watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

pub async fn serve(
    listener: tokio::net::TcpListener,
    server: LocalServer,
    shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    let driver = tokio::spawn(drive(server.driver.clone(), shutdown.clone()));
    let served = axum::serve(
        listener,
        server
            .router
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(stopped(shutdown))
    .await
    .map_err(|error| format!("serve: {error}"));
    driver
        .await
        .map_err(|error| format!("the worker stopped: {error}"))?;
    served
}
