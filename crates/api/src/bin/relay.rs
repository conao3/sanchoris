use chrono::Utc;
use lambda_runtime::{LambdaEvent, service_fn};
use sanchoris_api::lambda::{self, RelayPorts, StreamEvent};

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    let sdk = aws_config::load_from_env().await;
    let ports = RelayPorts::from_env(&sdk)?;
    let ports = &ports;
    lambda_runtime::run(service_fn(
        move |event: LambdaEvent<StreamEvent>| async move {
            Ok::<_, lambda_runtime::Error>(
                lambda::relay_stream(&ports.queue, &ports.scheduler, Utc::now(), &event.payload)
                    .await,
            )
        },
    ))
    .await
}
