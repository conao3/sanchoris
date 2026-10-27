use async_trait::async_trait;
use aws_sdk_apigatewaymanagement::error::{DisplayErrorContext, SdkError};
use aws_sdk_apigatewaymanagement::primitives::Blob;
use serde_json::Value;

use super::{ConnectionSender, SendError};
use crate::config::{ConfigError, Reader};

pub const CHANNEL_ENDPOINT_ENV: &str = "SANCHORIS_CHANNEL_ENDPOINT";

pub struct GatewaySender {
    client: aws_sdk_apigatewaymanagement::Client,
}

impl GatewaySender {
    pub fn new(sdk: &aws_config::SdkConfig, endpoint: &str) -> Self {
        let config = aws_sdk_apigatewaymanagement::config::Builder::from(sdk)
            .endpoint_url(endpoint)
            .build();
        Self {
            client: aws_sdk_apigatewaymanagement::Client::from_conf(config),
        }
    }

    pub fn from_env(sdk: &aws_config::SdkConfig) -> Result<Self, ConfigError> {
        let mut env = Reader::new();
        let endpoint = env.get(CHANNEL_ENDPOINT_ENV);
        env.finish()?;
        Ok(Self::new(sdk, &endpoint))
    }
}

fn is_gone<E, R>(error: &SdkError<E, R>) -> bool
where
    E: aws_sdk_apigatewaymanagement::error::ProvideErrorMetadata,
{
    error
        .as_service_error()
        .and_then(|service| service.code())
        .is_some_and(|code| code == "GoneException")
}

#[async_trait]
impl ConnectionSender for GatewaySender {
    async fn send(&self, connection_id: &str, message: &Value) -> Result<(), SendError> {
        let result = self
            .client
            .post_to_connection()
            .connection_id(connection_id)
            .data(Blob::new(message.to_string().into_bytes()))
            .send()
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(error) if is_gone(&error) => Err(SendError::Gone),
            Err(error) => Err(SendError::Failed(DisplayErrorContext(error).to_string())),
        }
    }

    async fn close(&self, connection_id: &str, _code: u16, _reason: &str) -> Result<(), SendError> {
        let result = self
            .client
            .delete_connection()
            .connection_id(connection_id)
            .send()
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(error) if is_gone(&error) => Err(SendError::Gone),
            Err(error) => Err(SendError::Failed(DisplayErrorContext(error).to_string())),
        }
    }
}
