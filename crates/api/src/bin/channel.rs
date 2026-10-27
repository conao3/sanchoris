use std::sync::Arc;

use lambda_runtime::{LambdaEvent, service_fn};
use sanchoris_api::channel::Channel;
use sanchoris_api::channel::credentials::IssuedCredentials;
use sanchoris_api::channel::gateway::GatewaySender;
use sanchoris_api::config::{
    CHANNEL_SECRETS, ChannelConfig, GitHubAppConfig, Secrets, credential_key,
};
use sanchoris_api::lambda::{self, WebSocketEvent};
use sanchoris_api::records::s3_records;

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    let sdk = aws_config::load_from_env().await;
    let secrets = Secrets::from_ssm(&sdk, &CHANNEL_SECRETS).await?;
    let config = ChannelConfig::load(&secrets)?;
    let github = Arc::new(GitHubAppConfig::load(&secrets)?.client()?);
    let store = lambda::store(&sdk)?;
    let channel = Channel {
        store: store.clone(),
        run_tokens: config.run_tokens,
        records: s3_records(&config.records, &sdk)?,
        sender: Arc::new(GatewaySender::from_env(&sdk)?),
        credentials: Arc::new(IssuedCredentials::new(
            store,
            github,
            credential_key(&secrets)?.0,
        )),
    };
    let channel = &channel;
    lambda_runtime::run(service_fn(
        move |event: LambdaEvent<WebSocketEvent>| async move {
            Box::pin(lambda::websocket(channel, &event.payload))
                .await
                .map_err(lambda_runtime::Error::from)
        },
    ))
    .await
}
