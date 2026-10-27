use chrono::Utc;
use lambda_runtime::{LambdaEvent, service_fn};
use sanchoris_api::config::{RUN_TOKEN_SECRETS, Secrets, run_tokens};
use sanchoris_api::lambda::{self, AuthorizerEvent};

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    let sdk = aws_config::load_from_env().await;
    let run_tokens = run_tokens(&Secrets::from_ssm(&sdk, &RUN_TOKEN_SECRETS).await?)?;
    let store = lambda::store(&sdk)?;
    let (store, run_tokens) = (&store, &run_tokens);
    lambda_runtime::run(service_fn(
        move |event: LambdaEvent<AuthorizerEvent>| async move {
            Box::pin(lambda::authorize(
                store,
                run_tokens,
                &event.payload,
                Utc::now(),
            ))
            .await
            .map_err(lambda_runtime::Error::from)
        },
    ))
    .await
}
