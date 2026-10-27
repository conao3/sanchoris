use chrono::Utc;
use lambda_runtime::{LambdaEvent, service_fn};
use sanchoris_api::config::{Reader, signer};
use sanchoris_api::lambda::{self, AuthorizerEvent};
use sanchoris_core::tokens::{RUN_TOKEN_SIGNING_KEY_ENV, RunTokenSigner};

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    let mut env = Reader::new();
    let run_key = env.get(RUN_TOKEN_SIGNING_KEY_ENV);
    env.finish()?;
    let run_tokens = RunTokenSigner::new(signer(RUN_TOKEN_SIGNING_KEY_ENV, &run_key)?);
    let sdk = aws_config::load_from_env().await;
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
