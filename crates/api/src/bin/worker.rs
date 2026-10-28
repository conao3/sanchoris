use std::sync::Arc;

use lambda_runtime::{LambdaEvent, service_fn};
use sanchoris_api::channel::Channel;
use sanchoris_api::channel::credentials::NoRunCredentials;
use sanchoris_api::channel::gateway::GatewaySender;
use sanchoris_api::config::{ChannelConfig, GitHubAppConfig, Secrets, WORKER_SECRETS};
use sanchoris_api::ecs::{EcsRunner, EcsRunnerConfig};
use sanchoris_api::lambda::{self, SqsEvent};
use sanchoris_api::records::s3_records;
use sanchoris_core::clock::{Clock, SystemClock};
use sanchoris_worker::resources::GitHubResources;
use sanchoris_worker::{Deps, Settings, Worker};

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    let sdk = aws_config::load_from_env().await;
    let secrets = Secrets::from_ssm(&sdk, &WORKER_SECRETS).await?;
    let config = ChannelConfig::load(&secrets)?;
    let runner = EcsRunnerConfig::load(&secrets)?;
    let github = Arc::new(GitHubAppConfig::load(&secrets)?.client()?);
    let store = lambda::store(&sdk)?;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let channel = Channel {
        store: store.clone(),
        run_tokens: config.run_tokens,
        records: s3_records(&config.records, &sdk)?,
        sender: Arc::new(GatewaySender::from_env(&sdk)?),
        credentials: Arc::new(NoRunCredentials),
    };
    let deps = Deps {
        clock: clock.clone(),
        github: github.clone(),
        reader: github.clone(),
        config_source: github.clone(),
        resources: Arc::new(GitHubResources::new(github)),
        runner: Arc::new(EcsRunner::new(&sdk, &store, runner, clock)),
        pusher: Arc::new(channel),
    };
    let worker = Worker::new(store, deps, Settings::default());
    let worker = &worker;
    lambda_runtime::run(service_fn(move |event: LambdaEvent<SqsEvent>| async move {
        Ok::<_, lambda_runtime::Error>(Box::pin(lambda::work(worker, &event.payload)).await)
    }))
    .await
}
