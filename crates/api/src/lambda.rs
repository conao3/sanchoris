use std::collections::HashMap;
use std::sync::Arc;

use aws_sdk_dynamodb::types::AttributeValue;
use chrono::{DateTime, Utc};
use sanchoris_core::aws::{EventBridgeScheduler, S3BlobStore, SqsQueue};
use sanchoris_core::db::Store;
use sanchoris_core::item::Item;
use sanchoris_core::ports::{Queue, Scheduler};
use sanchoris_core::store::{Relay, relay, relayed_from_item};
use sanchoris_core::tokens::{RunClaims, RunTokenSigner};
use sanchoris_worker::Worker;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::channel::{Admission, Channel, admit};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizerContext {
    pub tenant_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSocketContext {
    pub route_key: String,
    pub connection_id: String,
    pub authorizer: Option<AuthorizerContext>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSocketEvent {
    pub request_context: WebSocketContext,
    pub body: Option<String>,
    #[serde(default)]
    pub is_base64_encoded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSocketResponse {
    pub status_code: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizerEvent {
    pub method_arn: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqsRecord {
    pub message_id: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SqsEvent {
    #[serde(rename = "Records")]
    pub records: Vec<SqsRecord>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StreamChange {
    #[serde(rename = "SequenceNumber")]
    pub sequence_number: String,
    #[serde(rename = "NewImage")]
    pub new_image: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamRecord {
    pub event_name: String,
    pub dynamodb: StreamChange,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StreamEvent {
    #[serde(rename = "Records")]
    pub records: Vec<StreamRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemFailure {
    pub item_identifier: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchResponse {
    pub batch_item_failures: Vec<ItemFailure>,
}

#[derive(Debug, thiserror::Error)]
pub enum LambdaError {
    #[error("Unauthorized")]
    Unauthorized,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Internal(String),
}

fn header<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn policy(principal: &str, effect: &str, resource: &str, context: Value) -> Value {
    json!({
        "principalId": principal,
        "policyDocument": {
            "Version": "2012-10-17",
            "Statement": [{
                "Action": "execute-api:Invoke",
                "Effect": effect,
                "Resource": resource,
            }],
        },
        "context": context,
    })
}

pub async fn authorize(
    store: &Store,
    run_tokens: &RunTokenSigner,
    event: &AuthorizerEvent,
    now: DateTime<Utc>,
) -> Result<Value, LambdaError> {
    let token = header(&event.headers, "authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty());
    match admit(store, run_tokens, token, now)
        .await
        .map_err(|error| LambdaError::Internal(error.to_string()))?
    {
        Admission::Accepted(claims) => Ok(policy(
            &claims.run_id.to_string(),
            "Allow",
            &event.method_arn,
            json!({
                "tenantId": claims.tenant_id.to_string(),
                "runId": claims.run_id.to_string(),
            }),
        )),
        Admission::Forbidden => Ok(policy("forbidden", "Deny", &event.method_arn, json!({}))),
        Admission::Unauthorized => Err(LambdaError::Unauthorized),
    }
}

fn claims_of(context: &WebSocketContext) -> Result<RunClaims, LambdaError> {
    let authorizer = context
        .authorizer
        .as_ref()
        .ok_or(LambdaError::Unauthorized)?;
    let parse = |value: &str| {
        value.parse().map_err(|_| {
            LambdaError::Invalid(format!("authorizer context {value:?} is not a UUID"))
        })
    };
    Ok(RunClaims {
        tenant_id: parse(&authorizer.tenant_id)?,
        run_id: parse(&authorizer.run_id)?,
    })
}

pub async fn websocket(
    channel: &Channel,
    event: &WebSocketEvent,
) -> Result<WebSocketResponse, LambdaError> {
    let context = &event.request_context;
    let claims = claims_of(context)?;
    let connection = context.connection_id.as_str();
    let internal = |error: crate::channel::ChannelError| LambdaError::Internal(error.to_string());
    match context.route_key.as_str() {
        "$connect" => channel
            .connect(claims, connection)
            .await
            .map_err(internal)?,
        "$disconnect" => channel
            .disconnect(claims, connection)
            .await
            .map_err(internal)?,
        "$default" => {
            if event.is_base64_encoded {
                return Err(LambdaError::Invalid(
                    "binary frames are not supported".into(),
                ));
            }
            let body = event
                .body
                .as_deref()
                .ok_or_else(|| LambdaError::Invalid("the message has no body".into()))?;
            channel
                .receive(claims, connection, body)
                .await
                .map_err(internal)?;
        }
        other => return Err(LambdaError::Invalid(format!("unknown route {other}"))),
    }
    Ok(WebSocketResponse { status_code: 200 })
}

fn failures_from<'a>(identifiers: impl Iterator<Item = &'a str>) -> Vec<ItemFailure> {
    identifiers
        .map(|identifier| ItemFailure {
            item_identifier: identifier.to_string(),
        })
        .collect()
}

pub const DEADLINE_MARGIN_SECONDS: i64 = 30;

pub fn invocation_deadline(deadline_millis: u64) -> Result<DateTime<Utc>, String> {
    let millis = i64::try_from(deadline_millis)
        .map_err(|_| format!("the invocation deadline {deadline_millis} is out of range"))?;
    DateTime::<Utc>::from_timestamp_millis(millis)
        .map(|deadline| deadline - chrono::Duration::seconds(DEADLINE_MARGIN_SECONDS))
        .ok_or_else(|| format!("the invocation deadline {deadline_millis} is out of range"))
}

pub async fn work(worker: &Worker, event: &SqsEvent) -> BatchResponse {
    for (index, record) in event.records.iter().enumerate() {
        if let Err(error) = worker.handle_body(&record.body).await {
            eprintln!(
                "sanchoris worker: message {} failed: {error}",
                record.message_id
            );
            return BatchResponse {
                batch_item_failures: failures_from(
                    event.records[index..]
                        .iter()
                        .map(|record| record.message_id.as_str()),
                ),
            };
        }
    }
    BatchResponse::default()
}

pub fn attribute(value: &Value) -> Result<AttributeValue, String> {
    let object = value
        .as_object()
        .filter(|object| object.len() == 1)
        .ok_or_else(|| format!("{value} is not a DynamoDB attribute value"))?;
    let (kind, inner) = object.iter().next().expect("one entry");
    let text = || {
        inner
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("{kind} needs a string"))
    };
    let texts = || {
        inner
            .as_array()
            .ok_or_else(|| format!("{kind} needs an array"))?
            .iter()
            .map(|entry| {
                entry
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("{kind} needs strings"))
            })
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(match kind.as_str() {
        "S" => AttributeValue::S(text()?),
        "N" => AttributeValue::N(text()?),
        "BOOL" => AttributeValue::Bool(
            inner
                .as_bool()
                .ok_or_else(|| "BOOL needs a boolean".to_string())?,
        ),
        "NULL" => AttributeValue::Null(true),
        "SS" => AttributeValue::Ss(texts()?),
        "NS" => AttributeValue::Ns(texts()?),
        "L" => AttributeValue::L(
            inner
                .as_array()
                .ok_or_else(|| "L needs an array".to_string())?
                .iter()
                .map(attribute)
                .collect::<Result<_, _>>()?,
        ),
        "M" => AttributeValue::M(image(inner)?),
        other => return Err(format!("attribute type {other} is not supported")),
    })
}

pub fn image(value: &Value) -> Result<Item, String> {
    value
        .as_object()
        .ok_or_else(|| "an image is a map".to_string())?
        .iter()
        .map(|(name, value)| Ok((name.clone(), attribute(value)?)))
        .collect()
}

pub async fn relay_stream(
    queue: &dyn Queue,
    scheduler: &dyn Scheduler,
    now: DateTime<Utc>,
    event: &StreamEvent,
) -> BatchResponse {
    for (index, record) in event.records.iter().enumerate() {
        let outcome = async {
            if record.event_name != "INSERT" {
                return Ok(());
            }
            let Some(new_image) = &record.dynamodb.new_image else {
                return Ok(());
            };
            let item = image(new_image)?;
            let Some(relayed) = relayed_from_item(&item).map_err(|error| error.to_string())? else {
                return Ok(());
            };
            relay(queue, scheduler, now, &relayed)
                .await
                .map_err(|error| error.to_string())
        }
        .await;
        if let Err(error) = outcome {
            eprintln!(
                "sanchoris relay: record {} failed: {error}",
                record.dynamodb.sequence_number
            );
            return BatchResponse {
                batch_item_failures: failures_from(
                    event.records[index..]
                        .iter()
                        .map(|record| record.dynamodb.sequence_number.as_str()),
                ),
            };
        }
    }
    BatchResponse::default()
}

pub fn store(sdk: &aws_config::SdkConfig) -> Result<Store, String> {
    let blobs = S3BlobStore::docs_from_env(sdk).map_err(|error| error.to_string())?;
    Store::from_env(sdk, Arc::new(blobs), Relay::Stream).map_err(|error| error.to_string())
}

pub struct RelayPorts {
    pub queue: SqsQueue,
    pub scheduler: EventBridgeScheduler,
}

impl RelayPorts {
    pub fn from_env(sdk: &aws_config::SdkConfig) -> Result<Self, String> {
        Ok(Self {
            queue: SqsQueue::from_env(sdk).map_err(|error| error.to_string())?,
            scheduler: EventBridgeScheduler::from_env(sdk).map_err(|error| error.to_string())?,
        })
    }
}
