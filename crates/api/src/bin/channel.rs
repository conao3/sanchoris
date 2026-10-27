use std::sync::Arc;

use lambda_runtime::{LambdaEvent, service_fn};
use sanchoris_api::channel::Channel;
use sanchoris_api::channel::gateway::GatewaySender;
use sanchoris_api::config::{ChannelConfig, RUN_TOKEN_SECRETS, Secrets};
use sanchoris_api::lambda::{self, WebSocketEvent};
use sanchoris_api::records::s3_records;

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    let sdk = aws_config::load_from_env().await;
    let config = ChannelConfig::load(&Secrets::from_ssm(&sdk, &RUN_TOKEN_SECRETS).await?)?;
    let channel = Channel {
        store: lambda::store(&sdk)?,
        run_tokens: config.run_tokens,
        records: s3_records(&config.records, &sdk)?,
        sender: Arc::new(GatewaySender::from_env(&sdk)?),
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
