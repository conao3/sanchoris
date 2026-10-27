use async_trait::async_trait;
use aws_sdk_scheduler::types::{
    ActionAfterCompletion, FlexibleTimeWindow, FlexibleTimeWindowMode, SqsParameters, Target,
};
use aws_sdk_sqs::types::MessageSystemAttributeName;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::ports::{BlobStore, OutgoingMessage, PortError, Queue, ReceivedMessage, Scheduler};

pub const WORK_QUEUE_URL_ENV: &str = "SANCHORIS_WORK_QUEUE_URL";
pub const WORK_QUEUE_ARN_ENV: &str = "SANCHORIS_WORK_QUEUE_ARN";
pub const SCHEDULE_GROUP_ENV: &str = "SANCHORIS_SCHEDULE_GROUP";
pub const SCHEDULER_ROLE_ARN_ENV: &str = "SANCHORIS_SCHEDULER_ROLE_ARN";
pub const DOCS_BUCKET_ENV: &str = "SANCHORIS_DOCS_BUCKET";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("missing required environment variables: {}", .0.join(", "))]
pub struct MissingEnv(pub Vec<String>);

pub fn require_env<const N: usize>(names: [&str; N]) -> Result<[String; N], MissingEnv> {
    let values = names.map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()));
    let missing = names
        .iter()
        .zip(values.iter())
        .filter(|(_, value)| value.is_none())
        .map(|(name, _)| name.to_string())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(MissingEnv(missing));
    }
    Ok(values.map(|value| value.expect("checked above")))
}

fn port_error(port: &'static str, error: impl std::fmt::Display) -> PortError {
    PortError::new(port, error.to_string())
}

#[derive(Debug, Clone)]
pub struct SqsQueue {
    client: aws_sdk_sqs::Client,
    url: String,
}

impl SqsQueue {
    pub fn new(client: aws_sdk_sqs::Client, url: impl Into<String>) -> Self {
        Self {
            client,
            url: url.into(),
        }
    }

    pub fn from_env(config: &aws_config::SdkConfig) -> Result<Self, MissingEnv> {
        let [url] = require_env([WORK_QUEUE_URL_ENV])?;
        Ok(Self::new(aws_sdk_sqs::Client::new(config), url))
    }
}

#[async_trait]
impl Queue for SqsQueue {
    async fn send(&self, message: &OutgoingMessage) -> Result<(), PortError> {
        let mut request = self
            .client
            .send_message()
            .queue_url(&self.url)
            .message_body(&message.body)
            .message_group_id(&message.group)
            .message_deduplication_id(&message.dedupe_id);
        if let Some(delay) = message.delay {
            let seconds = i32::try_from(delay.as_secs())
                .map_err(|_| PortError::new("sqs", "delay does not fit in i32 seconds"))?;
            request = request.delay_seconds(seconds);
        }
        request.send().await.map_err(|e| port_error("sqs", e))?;
        Ok(())
    }

    async fn receive(&self, max: usize) -> Result<Vec<ReceivedMessage>, PortError> {
        let max = i32::try_from(max.clamp(1, 10)).expect("clamped");
        let output = self
            .client
            .receive_message()
            .queue_url(&self.url)
            .max_number_of_messages(max)
            .message_system_attribute_names(MessageSystemAttributeName::ApproximateReceiveCount)
            .send()
            .await
            .map_err(|e| port_error("sqs", e))?;
        output
            .messages
            .unwrap_or_default()
            .into_iter()
            .map(|message| {
                let receive_count = message
                    .attributes
                    .as_ref()
                    .and_then(|a| a.get(&MessageSystemAttributeName::ApproximateReceiveCount))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1);
                Ok(ReceivedMessage {
                    body: message
                        .body
                        .ok_or_else(|| PortError::new("sqs", "a message has no body"))?,
                    receipt: message
                        .receipt_handle
                        .ok_or_else(|| PortError::new("sqs", "a message has no receipt"))?,
                    receive_count,
                })
            })
            .collect()
    }

    async fn delete(&self, receipt: &str) -> Result<(), PortError> {
        self.client
            .delete_message()
            .queue_url(&self.url)
            .receipt_handle(receipt)
            .send()
            .await
            .map_err(|e| port_error("sqs", e))?;
        Ok(())
    }

    async fn release(&self, receipt: &str) -> Result<(), PortError> {
        self.client
            .change_message_visibility()
            .queue_url(&self.url)
            .receipt_handle(receipt)
            .visibility_timeout(0)
            .send()
            .await
            .map_err(|e| port_error("sqs", e))?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct EventBridgeScheduler {
    client: aws_sdk_scheduler::Client,
    group: String,
    queue_arn: String,
    role_arn: String,
}

impl EventBridgeScheduler {
    pub fn new(
        client: aws_sdk_scheduler::Client,
        group: impl Into<String>,
        queue_arn: impl Into<String>,
        role_arn: impl Into<String>,
    ) -> Self {
        Self {
            client,
            group: group.into(),
            queue_arn: queue_arn.into(),
            role_arn: role_arn.into(),
        }
    }

    pub fn from_env(config: &aws_config::SdkConfig) -> Result<Self, MissingEnv> {
        let [group, queue_arn, role_arn] = require_env([
            SCHEDULE_GROUP_ENV,
            WORK_QUEUE_ARN_ENV,
            SCHEDULER_ROLE_ARN_ENV,
        ])?;
        Ok(Self::new(
            aws_sdk_scheduler::Client::new(config),
            group,
            queue_arn,
            role_arn,
        ))
    }
}

pub fn at_expression(at: DateTime<Utc>) -> String {
    format!("at({})", at.format("%Y-%m-%dT%H:%M:%S"))
}

#[async_trait]
impl Scheduler for EventBridgeScheduler {
    async fn schedule(
        &self,
        name: &str,
        at: DateTime<Utc>,
        message: &OutgoingMessage,
    ) -> Result<(), PortError> {
        let window = FlexibleTimeWindow::builder()
            .mode(FlexibleTimeWindowMode::Off)
            .build()
            .map_err(|e| port_error("scheduler", e))?;
        let target = Target::builder()
            .arn(&self.queue_arn)
            .role_arn(&self.role_arn)
            .input(&message.body)
            .sqs_parameters(
                SqsParameters::builder()
                    .message_group_id(&message.group)
                    .build(),
            )
            .build()
            .map_err(|e| port_error("scheduler", e))?;
        let result = self
            .client
            .create_schedule()
            .name(name)
            .group_name(&self.group)
            .schedule_expression(at_expression(at))
            .schedule_expression_timezone("UTC")
            .flexible_time_window(window)
            .target(target)
            .action_after_completion(ActionAfterCompletion::Delete)
            .send()
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(|e| e.is_conflict_exception()) =>
            {
                Ok(())
            }
            Err(error) => Err(port_error(
                "scheduler",
                aws_sdk_scheduler::error::DisplayErrorContext(error),
            )),
        }
    }

    async fn cancel(&self, name: &str) -> Result<(), PortError> {
        let result = self
            .client
            .delete_schedule()
            .name(name)
            .group_name(&self.group)
            .send()
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(|e| e.is_resource_not_found_exception()) =>
            {
                Ok(())
            }
            Err(error) => Err(port_error(
                "scheduler",
                aws_sdk_scheduler::error::DisplayErrorContext(error),
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct S3BlobStore {
    client: aws_sdk_s3::Client,
    bucket: String,
}

impl S3BlobStore {
    pub fn new(client: aws_sdk_s3::Client, bucket: impl Into<String>) -> Self {
        Self {
            client,
            bucket: bucket.into(),
        }
    }

    pub fn docs_from_env(config: &aws_config::SdkConfig) -> Result<Self, MissingEnv> {
        let [bucket] = require_env([DOCS_BUCKET_ENV])?;
        Ok(Self::new(aws_sdk_s3::Client::new(config), bucket))
    }
}

#[async_trait]
impl BlobStore for S3BlobStore {
    async fn put(&self, key: &str, body: Vec<u8>) -> Result<(), PortError> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body.into())
            .send()
            .await
            .map_err(|e| port_error("s3", aws_sdk_s3::error::DisplayErrorContext(e)))?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, PortError> {
        let result = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await;
        match result {
            Ok(output) => {
                let bytes = output
                    .body
                    .collect()
                    .await
                    .map_err(|e| port_error("s3", e))?;
                Ok(Some(bytes.into_bytes().to_vec()))
            }
            Err(error) if error.as_service_error().is_some_and(|e| e.is_no_such_key()) => Ok(None),
            Err(error) => Err(port_error(
                "s3",
                aws_sdk_s3::error::DisplayErrorContext(error),
            )),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), PortError> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| port_error("s3", aws_sdk_s3::error::DisplayErrorContext(e)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn at_expressions_use_utc_seconds() {
        let at = Utc.with_ymd_and_hms(2026, 10, 27, 9, 30, 5).unwrap();
        assert_eq!(at_expression(at), "at(2026-10-27T09:30:05)");
    }
}
