use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use sanchoris_core::audit::{AuditEntry, record_audit};
use sanchoris_core::billing::{self, BillingError, Invoice, Outcome, Payer};
use sanchoris_core::channel::{IngestError, ingest_github_delivery};
use sanchoris_core::db::{DbError, Recorded};
use sanchoris_core::items::{
    Config, GitHubRepository, Guard, Integration, github_repositories, uniq,
};
use sanchoris_core::keys;
use sanchoris_core::model::{ActorKind, AuditOrigin};
use sanchoris_worker::config_sync::{disable_config, rename_repository};
use sanchoris_worker::error::WorkerError;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::app::AppState;
use crate::error::ApiError;
use crate::signing::HmacKey;

pub const STRIPE_TOLERANCE_SECONDS: i64 = 300;
const CONFLICT_ATTEMPTS: u32 = 8;

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
    if matches!(
        event,
        "installation" | "installation_repositories" | "repository"
    ) {
        return installation_event(&state, event, &payload).await;
    }
    match ingest_github_delivery(&state.store, delivery, event, &payload).await {
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

fn repository_of(value: &Value, key: &str) -> Result<GitHubRepository, ApiError> {
    match (value["id"].as_i64(), value["full_name"].as_str()) {
        (Some(id), Some(full_name)) => Ok(GitHubRepository {
            id,
            full_name: full_name.to_string(),
        }),
        _ => Err(ApiError::BadRequest(format!(
            "{key} has no id and full_name"
        ))),
    }
}

fn repository_list(payload: &Value, key: &str) -> Result<Vec<GitHubRepository>, ApiError> {
    payload
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::BadRequest(format!("the event has no {key}")))?
        .iter()
        .map(|repository| repository_of(repository, key))
        .collect()
}

enum ConfigEffect {
    Disable(Vec<GitHubRepository>, &'static str),
    Rename(GitHubRepository),
}

fn config_effect(
    event: &str,
    action: &str,
    payload: &Value,
) -> Result<Option<ConfigEffect>, ApiError> {
    Ok(match (event, action) {
        ("installation_repositories", "removed") => Some(ConfigEffect::Disable(
            repository_list(payload, "repositories_removed")?,
            "was removed from the GitHub App installation",
        )),
        ("repository", "deleted") => Some(ConfigEffect::Disable(
            vec![repository_of(&payload["repository"], "repository")?],
            "was deleted on GitHub",
        )),
        ("repository", "renamed" | "transferred") => Some(ConfigEffect::Rename(repository_of(
            &payload["repository"],
            "repository",
        )?)),
        _ => None,
    })
}

async fn apply_config_effect(
    state: &AppState,
    tenant_id: Uuid,
    effect: ConfigEffect,
) -> Result<(), ApiError> {
    let now = Utc::now();
    let worker_error = |error: WorkerError| ApiError::Internal(error.to_string());
    match effect {
        ConfigEffect::Rename(repository) => {
            rename_repository(&state.store, tenant_id, &repository, now)
                .await
                .map_err(worker_error)?;
        }
        ConfigEffect::Disable(repositories, what) => {
            let configs = state
                .store
                .query::<Config>(&keys::tenant_pk(tenant_id), "CONFIG#")
                .await?;
            for repository in repositories {
                for config in configs
                    .iter()
                    .filter(|c| c.is_active() && c.repository_id == Some(repository.id))
                {
                    let reason = format!("{} {what}", repository.full_name);
                    disable_config(&state.store, tenant_id, config.id, &reason, now)
                        .await
                        .map_err(worker_error)?;
                }
            }
        }
    }
    Ok(())
}

async fn installation_event(
    state: &AppState,
    event: &str,
    payload: &Value,
) -> Result<Response, ApiError> {
    let installation_id = payload
        .pointer("/installation/id")
        .and_then(Value::as_i64)
        .ok_or_else(|| ApiError::BadRequest("the event has no installation.id".into()))?;
    let action = text(payload, "/action")?;
    let Some(guard) = state
        .store
        .get::<Guard>(&keys::uniq_key(
            uniq::INSTALLATION,
            &installation_id.to_string(),
        ))
        .await?
    else {
        return Ok(status(StatusCode::OK, "ignored"));
    };
    let tenant_id = guard.tenant_id.ok_or_else(|| {
        ApiError::Internal(format!("installation {installation_id} has no tenant"))
    })?;
    let integration_key = keys::integration_key(tenant_id, guard.target);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let mut tx = state.store.tx(tenant_id);
        let Some(integration) = tx.get::<Integration>(&integration_key).await? else {
            return Ok(status(StatusCode::OK, "ignored"));
        };
        let Some(updated) = apply_installation_event(integration, event, action, payload)? else {
            return Ok(status(StatusCode::OK, "ignored"));
        };
        tx.put(&updated)?;
        let mut metadata = Map::new();
        metadata.insert("event".into(), json!(event));
        metadata.insert("action".into(), json!(action));
        record_audit(
            &mut tx,
            &AuditEntry {
                at: Utc::now(),
                actor_kind: ActorKind::System,
                actor_ref: "github",
                action: "integration.github_installation_update",
                target_kind: "github_installation",
                target_ref: &installation_id.to_string(),
                summary: &format!(
                    "GitHub sent {event}.{action} for installation {installation_id}"
                ),
                origin: AuditOrigin::Github,
                source_ip: None,
                metadata,
            },
        )
        .await?;
        match tx.commit().await {
            Err(DbError::Conflict) if attempt < CONFLICT_ATTEMPTS => {}
            Err(error) => return Err(error.into()),
            Ok(()) => break,
        }
    }
    if let Some(effect) = config_effect(event, action, payload)? {
        apply_config_effect(state, tenant_id, effect).await?;
    }
    Ok(status(StatusCode::OK, "processed"))
}

fn apply_installation_event(
    mut integration: Integration,
    event: &str,
    action: &str,
    payload: &Value,
) -> Result<Option<Integration>, ApiError> {
    let mut connected = integration.status == "connected";
    let mut repositories = github_repositories(&integration.details)?;
    match (event, action) {
        ("installation", "deleted" | "suspend") => connected = false,
        ("installation", "unsuspend") => connected = true,
        ("installation", "created" | "new_permissions_accepted") => {
            connected = true;
            integration.details["account_login"] =
                json!(text(payload, "/installation/account/login")?);
            integration.details["account_type"] =
                json!(text(payload, "/installation/account/type")?);
            if payload.get("repositories").is_some() {
                repositories = repository_list(payload, "repositories")?;
            }
        }
        ("installation_repositories", "added" | "removed") => {
            let removed = repository_list(payload, "repositories_removed")?;
            repositories.retain(|r| removed.iter().all(|gone| gone.id != r.id));
            for added in repository_list(payload, "repositories_added")? {
                repositories.retain(|r| r.id != added.id);
                repositories.push(added);
            }
        }
        ("repository", "renamed" | "transferred") => {
            let renamed = repository_of(&payload["repository"], "repository")?;
            for repository in repositories.iter_mut().filter(|r| r.id == renamed.id) {
                repository.full_name = renamed.full_name.clone();
            }
        }
        ("repository", "deleted") => {
            let deleted = repository_of(&payload["repository"], "repository")?;
            repositories.retain(|r| r.id != deleted.id);
        }
        _ => return Ok(None),
    }
    repositories.sort_by(|a, b| (&a.full_name, a.id).cmp(&(&b.full_name, b.id)));
    integration.details["repositories"] = json!(repositories);
    integration.status = if connected {
        "connected"
    } else {
        "not_connected"
    }
    .to_string();
    integration.updated_at = Utc::now();
    Ok(Some(integration))
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

fn stripe_time(value: &Value, pointer: &str) -> Result<DateTime<Utc>, ApiError> {
    value
        .pointer(pointer)
        .and_then(Value::as_i64)
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
        .ok_or_else(|| ApiError::BadRequest(format!("the event has no {pointer}")))
}

fn tenant_ref(value: &Value, pointer: &str) -> Result<Uuid, ApiError> {
    text(value, pointer)?
        .parse()
        .map_err(|_| ApiError::BadRequest(format!("{pointer} is not a tenant id")))
}

fn tenant_hint(value: &Value, pointer: &str) -> Result<Option<Uuid>, ApiError> {
    match value.pointer(pointer) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => tenant_ref(value, pointer).map(Some),
    }
}

fn subscription_price(subscription: &Value) -> Result<&str, ApiError> {
    let items = subscription
        .pointer("/items/data")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::BadRequest("the subscription has no items".into()))?;
    let [item] = items.as_slice() else {
        return Err(ApiError::BadRequest(
            "the subscription must have exactly one item".into(),
        ));
    };
    text(item, "/price/id")
}

fn invoice_period_start(invoice: &Value) -> Result<Option<DateTime<Utc>>, ApiError> {
    let lines = invoice
        .pointer("/lines/data")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::BadRequest("the invoice has no lines".into()))?;
    Ok(lines
        .iter()
        .map(|line| stripe_time(line, "/period/start"))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max())
}

fn payer<'a>(object: &'a Value, tenant_pointer: &str) -> Result<Payer<'a>, ApiError> {
    Ok(Payer {
        customer_id: text(object, "/customer")?,
        tenant_id: tenant_hint(object, tenant_pointer)?,
    })
}

fn checkout<'a>(session: &'a Value) -> Result<billing::Checkout<'a>, ApiError> {
    Ok(billing::Checkout {
        tenant_id: tenant_ref(session, "/client_reference_id")?,
        customer_id: text(session, "/customer")?,
        plan_id: text(session, "/metadata/plan_id")?,
        paid: text(session, "/payment_status")? == "paid",
    })
}

enum StripeFailure {
    Request(ApiError),
    Billing(BillingError),
}

impl From<ApiError> for StripeFailure {
    fn from(error: ApiError) -> Self {
        Self::Request(error)
    }
}

impl From<BillingError> for StripeFailure {
    fn from(error: BillingError) -> Self {
        Self::Billing(error)
    }
}

async fn apply_stripe_event(
    state: &AppState,
    event: billing::StripeEvent<'_>,
    object: &Value,
) -> Result<Option<Outcome>, StripeFailure> {
    let store = &state.store;
    let outcome = match event.kind {
        "checkout.session.completed" | "checkout.session.async_payment_succeeded" => {
            billing::complete_checkout(store, event, checkout(object)?).await?
        }
        "checkout.session.async_payment_failed" | "checkout.session.expired" => {
            billing::end_checkout(store, event, tenant_ref(object, "/client_reference_id")?).await?
        }
        "customer.subscription.updated" => {
            billing::change_plan(
                store,
                event,
                payer(object, "/metadata/tenant_id")?,
                text(object, "/status")?,
                subscription_price(object)?,
            )
            .await?
        }
        "customer.subscription.deleted" => {
            billing::end_subscription(store, event, payer(object, "/metadata/tenant_id")?).await?
        }
        "invoice.paid" => {
            billing::start_period(
                store,
                event,
                payer(object, "/parent/subscription_details/metadata/tenant_id")?,
                Invoice {
                    billing_reason: text(object, "/billing_reason")?,
                    period_start: invoice_period_start(object)?,
                },
            )
            .await?
        }
        "invoice.payment_failed" => {
            billing::fail_payment(
                store,
                event,
                payer(object, "/parent/subscription_details/metadata/tenant_id")?,
            )
            .await?
        }
        _ => return Ok(None),
    };
    Ok(Some(outcome))
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
        &state.config.stripe.webhook_secret,
        signature,
        &body,
        Utc::now(),
    )
    .map_err(|_| ApiError::Unauthorized)?;
    let event: Value = serde_json::from_slice(&body)
        .map_err(|error| ApiError::BadRequest(format!("the body is not JSON: {error}")))?;
    let event_ref = billing::StripeEvent {
        id: text(&event, "/id")?,
        kind: text(&event, "/type")?,
        created: stripe_time(&event, "/created")?,
    };
    if billing::recorded(&state.store, event_ref.id)
        .await?
        .is_some()
    {
        return Ok(status(StatusCode::OK, "duplicate"));
    }
    let object = &event["data"]["object"];
    let (tenant_id, reason) = match apply_stripe_event(&state, event_ref, object).await {
        Ok(None) => return Ok(status(StatusCode::OK, "ignored")),
        Ok(Some(Outcome::Processed)) => return Ok(status(StatusCode::OK, "processed")),
        Ok(Some(Outcome::Duplicate)) => return Ok(status(StatusCode::OK, "duplicate")),
        Ok(Some(Outcome::Ignored(reason))) => (None, reason),
        Err(StripeFailure::Request(error)) => return Err(error),
        Err(StripeFailure::Billing(error)) => match error {
            BillingError::TenantNotPayable(id) | BillingError::TenantGone(id) => {
                (Some(id), error.to_string())
            }
            BillingError::UnknownCustomer(_) => return Err(ApiError::Conflict(error.to_string())),
            BillingError::Db(error) => return Err(error.into()),
            BillingError::UnknownPrice(_) | BillingError::UnknownPlan(_) => {
                return Err(ApiError::Internal(error.to_string()));
            }
        },
    };
    billing::acknowledge(&state.store, event_ref, tenant_id, &reason).await?;
    Ok((
        StatusCode::OK,
        Json(json!({ "status": "ignored", "reason": reason })),
    )
        .into_response())
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
