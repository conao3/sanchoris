use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use sanchoris_core::billing::{self, BillingError};
use sanchoris_core::channel::{IngestError, ingest_github_delivery};
use sanchoris_core::db::Recorded;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::app::AppState;
use crate::error::ApiError;
use crate::signing::HmacKey;

pub const STRIPE_TOLERANCE_SECONDS: i64 = 300;

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, ApiError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::BadRequest(format!("missing {name}")))
}

fn status(code: StatusCode, value: &str) -> Response {
    (code, Json(json!({ "status": value }))).into_response()
}

pub fn verify_github_signature(secret: &HmacKey, signature: &str, body: &[u8]) -> bool {
    signature
        .strip_prefix("sha256=")
        .is_some_and(|hex| secret.verify_hex(body, hex))
}

pub async fn github(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::Unauthorized)?;
    if !verify_github_signature(&state.config.github.webhook_secret, signature, &body) {
        return Err(ApiError::Unauthorized);
    }
    let event = header(&headers, "x-github-event")?;
    let delivery = header(&headers, "x-github-delivery")?;
    let payload: Value = serde_json::from_slice(&body)
        .map_err(|error| ApiError::BadRequest(format!("the body is not JSON: {error}")))?;
    match ingest_github_delivery(&state.pool, delivery, event, &payload).await {
        Ok(Some(Recorded::New(_))) => Ok(status(StatusCode::ACCEPTED, "accepted")),
        Ok(Some(Recorded::Duplicate(_))) => Ok(status(StatusCode::OK, "duplicate")),
        Ok(None) => Ok(status(StatusCode::OK, "ignored")),
        Err(IngestError::UnknownInstallation(id)) => {
            eprintln!("sanchoris api: GitHub delivery {delivery} is for unknown installation {id}");
            Ok(status(StatusCode::OK, "ignored"))
        }
        Err(IngestError::Event(error)) => Err(ApiError::BadRequest(error.to_string())),
        Err(IngestError::Db(error)) => Err(error.into()),
    }
}

pub fn verify_stripe_signature(
    secret: &HmacKey,
    header: &str,
    body: &[u8],
    now: DateTime<Utc>,
) -> Result<(), String> {
    let mut timestamp = None;
    let mut signatures = Vec::new();
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", value)) => timestamp = value.parse::<i64>().ok(),
            Some(("v1", value)) => signatures.push(value),
            _ => {}
        }
    }
    let timestamp = timestamp.ok_or("the signature has no timestamp")?;
    if (now.timestamp() - timestamp).abs() > STRIPE_TOLERANCE_SECONDS {
        return Err("the signature timestamp is outside the tolerance".into());
    }
    let mut signed = format!("{timestamp}.").into_bytes();
    signed.extend_from_slice(body);
    if signatures
        .iter()
        .any(|signature| secret.verify_hex(&signed, signature))
    {
        Ok(())
    } else {
        Err("no v1 signature matches".into())
    }
}

fn text<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, ApiError> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::BadRequest(format!("the event has no {pointer}")))
}

impl From<BillingError> for ApiError {
    fn from(error: BillingError) -> Self {
        match error {
            BillingError::Db(error) => error.into(),
            BillingError::UnknownCustomer(_) => ApiError::Conflict(error.to_string()),
            BillingError::TenantNotPayable(_) => ApiError::Unprocessable {
                code: "tenant_not_payable",
                message: error.to_string(),
            },
            BillingError::UnknownPrice(_) => ApiError::Internal(error.to_string()),
        }
    }
}

pub async fn stripe(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let signature = headers
        .get("stripe-signature")
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::Unauthorized)?;
    verify_stripe_signature(
        &state.config.stripe_webhook_secret,
        signature,
        &body,
        Utc::now(),
    )
    .map_err(|_| ApiError::Unauthorized)?;
    let event: Value = serde_json::from_slice(&body)
        .map_err(|error| ApiError::BadRequest(format!("the body is not JSON: {error}")))?;
    let kind = text(&event, "/type")?;
    let object = &event["data"]["object"];
    let mut tx = state.pool.begin().await?;
    match kind {
        "checkout.session.completed" => {
            let tenant_id: Uuid = text(object, "/client_reference_id")?.parse().map_err(|_| {
                ApiError::BadRequest("client_reference_id is not a tenant id".into())
            })?;
            billing::complete_checkout(&mut tx, tenant_id, text(object, "/customer")?).await?;
        }
        "customer.subscription.updated" => {
            let items = object
                .pointer("/items/data")
                .and_then(Value::as_array)
                .ok_or_else(|| ApiError::BadRequest("the subscription has no items".into()))?;
            let [item] = items.as_slice() else {
                return Err(ApiError::BadRequest(
                    "the subscription must have exactly one item".into(),
                ));
            };
            billing::change_plan(
                &mut tx,
                text(object, "/customer")?,
                text(item, "/price/id")?,
            )
            .await?;
        }
        "customer.subscription.deleted" => {
            billing::end_subscription(&mut tx, text(object, "/customer")?).await?;
        }
        "invoice.paid" => {
            let created = event
                .get("created")
                .and_then(Value::as_i64)
                .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
                .ok_or_else(|| ApiError::BadRequest("the event has no created".into()))?;
            billing::start_period(&mut tx, text(object, "/customer")?, created).await?;
        }
        "invoice.payment_failed" => {
            billing::fail_payment(&mut tx, text(object, "/customer")?).await?;
        }
        _ => return Ok(status(StatusCode::OK, "ignored")),
    }
    tx.commit().await?;
    Ok(status(StatusCode::OK, "processed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn stripe_signatures_check_the_timestamp_and_any_v1() {
        let secret = HmacKey::new(b"whsec_test").unwrap();
        let now = Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap();
        let body = br#"{"type":"invoice.paid"}"#;
        let t = now.timestamp();
        let mut signed = format!("{t}.").into_bytes();
        signed.extend_from_slice(body);
        let good = secret.hex(&signed);
        let header = format!("t={t},v1=deadbeef,v1={good},v0=ignored");
        assert!(verify_stripe_signature(&secret, &header, body, now).is_ok());
        assert!(
            verify_stripe_signature(&secret, &header, body, now + chrono::Duration::seconds(301))
                .is_err()
        );
        assert!(verify_stripe_signature(&secret, &format!("t={t},v1=00"), body, now).is_err());
        assert!(verify_stripe_signature(&secret, &format!("v1={good}"), body, now).is_err());
        assert!(verify_stripe_signature(&secret, &header, b"{}", now).is_err());
    }

    #[test]
    fn github_signatures_need_the_sha256_prefix() {
        let secret = HmacKey::new(b"It's a Secret to Everybody").unwrap();
        let signature = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        assert!(verify_github_signature(
            &secret,
            signature,
            b"Hello, World!"
        ));
        assert!(!verify_github_signature(
            &secret,
            &signature.replace("sha256=", ""),
            b"Hello, World!"
        ));
        assert!(!verify_github_signature(
            &secret,
            signature,
            b"Hello, World?"
        ));
    }
}
