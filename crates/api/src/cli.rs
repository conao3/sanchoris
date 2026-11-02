use aws_sdk_dynamodb::types::AttributeValue;
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use sanchoris_core::audit::{AuditEntry, record_audit};
use sanchoris_core::cli::{
    ACCESS_TOKEN_SECONDS, APPROVED, CliPurpose, CliRequest, DENIED, DEVICE_CODE_SECONDS, ISSUED,
    SLOW_DOWN_SECONDS, cli_request_key, display_user_code, issue_token, new_request,
    secret_matches, split_code,
};
use sanchoris_core::db::{DbError, TenantTx};
use sanchoris_core::item::{Entity, format_time};
use sanchoris_core::items::{Credential, Guard, Member, Tenant, User};
use sanchoris_core::keys;
use sanchoris_core::model::{ActorKind, AuditOrigin};
use sanchoris_core::store::Update;
use sanchoris_core::tenants::is_deleting;
use sanchoris_graphql::{ClientIp, Encryptor, mask_key};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::bearer;
use crate::channel::credentials::OAUTH_TOKEN_PREFIX;
use crate::error::ApiError;

const ATTEMPTS: u32 = 8;
const MAX_CLIENT_CHARS: usize = 64;
const ADMIN_ROLES: [&str; 2] = ["owner", "admin"];

#[derive(Deserialize)]
pub struct DeviceBody {
    client: String,
    purpose: String,
    #[serde(default)]
    scope: Option<String>,
}

pub async fn device(
    State(state): State<AppState>,
    Json(body): Json<DeviceBody>,
) -> Result<Json<Value>, ApiError> {
    let client = body.client.trim();
    if client.is_empty()
        || client.chars().count() > MAX_CLIENT_CHARS
        || !client.chars().all(|c| c.is_ascii_graphic() || c == ' ')
    {
        return Err(ApiError::BadRequest(format!(
            "client must be 1 to {MAX_CLIENT_CHARS} printable ASCII characters"
        )));
    }
    let purpose = CliPurpose::parse(&body.purpose).ok_or_else(|| {
        ApiError::BadRequest(format!(
            "purpose must be claude-credential or codex-credential, not {:?}",
            body.purpose
        ))
    })?;
    let scope = match body.scope.as_deref() {
        None | Some("personal") => "personal",
        Some("org") => "org",
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "scope must be personal or org, not {other:?}"
            )));
        }
    };
    let mut attempt = 0;
    let (request, device_code) = loop {
        attempt += 1;
        let (request, device_code) = new_request(client, purpose, scope, Utc::now());
        let mut tx = state.store.global_tx();
        tx.insert(&request)?;
        match tx.commit().await {
            Ok(()) => break (request, device_code),
            Err(error) if error.is_conflict() && attempt < ATTEMPTS => {}
            Err(error) => return Err(error.into()),
        }
    };
    let user_code = display_user_code(&request.user_code);
    let verification_uri = format!("{}/cli", state.config.public_url.trim_end_matches('/'));
    Ok(Json(json!({
        "deviceCode": device_code,
        "userCode": user_code,
        "verificationUri": verification_uri,
        "verificationUriComplete": format!("{verification_uri}?code={user_code}"),
        "expiresIn": DEVICE_CODE_SECONDS,
        "interval": request.interval_seconds,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBody {
    device_code: String,
}

enum Poll {
    Error(&'static str),
    Token(String),
}

fn grant_error(code: &'static str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": code }))).into_response()
}

pub async fn token(
    State(state): State<AppState>,
    Json(body): Json<TokenBody>,
) -> Result<Response, ApiError> {
    let Some((user_code, secret)) = split_code(&body.device_code) else {
        return Ok(grant_error("invalid_grant"));
    };
    let key = cli_request_key(&user_code);
    let mut attempt = 0;
    let poll = loop {
        attempt += 1;
        let now = Utc::now();
        let mut tx = state.store.global_tx();
        let Some(mut request) = tx.get::<CliRequest>(&key).await? else {
            break Poll::Error("invalid_grant");
        };
        if !secret_matches(secret, &request.device_code_hash) {
            break Poll::Error("invalid_grant");
        }
        if request.status == ISSUED {
            break Poll::Error("invalid_grant");
        }
        if request.is_expired(now) {
            break Poll::Error("expired_token");
        }
        if request.status == DENIED {
            break Poll::Error("access_denied");
        }
        let early = request
            .last_polled_at
            .is_some_and(|last| now - last < Duration::seconds(request.interval_seconds));
        request.last_polled_at = Some(now);
        let poll = if early {
            request.interval_seconds += SLOW_DOWN_SECONDS;
            Poll::Error("slow_down")
        } else if request.status == APPROVED {
            Poll::Token(issue_token(&mut request, now))
        } else {
            Poll::Error("authorization_pending")
        };
        tx.put(&request)?;
        match tx.commit().await {
            Ok(()) => break poll,
            Err(error) if error.is_conflict() && attempt < ATTEMPTS => {}
            Err(error) => return Err(error.into()),
        }
    };
    Ok(match poll {
        Poll::Error(code) => grant_error(code),
        Poll::Token(access_token) => Json(json!({
            "accessToken": access_token,
            "expiresIn": ACCESS_TOKEN_SECONDS,
        }))
        .into_response(),
    })
}

#[derive(Deserialize)]
pub struct ClaudeBody {
    token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexBody {
    auth_json: Value,
}

struct Secret {
    value: String,
    account: String,
}

pub async fn claude_credential(
    State(state): State<AppState>,
    Extension(ip): Extension<ClientIp>,
    headers: HeaderMap,
    Json(body): Json<ClaudeBody>,
) -> Result<Response, ApiError> {
    let token = body.token.trim();
    if !token.starts_with(OAUTH_TOKEN_PREFIX) {
        return Err(ApiError::BadRequest(
            "the token is not a Claude Code OAuth token from claude setup-token".into(),
        ));
    }
    let secret = Secret {
        value: token.to_string(),
        account: mask_key(token),
    };
    register(&state, ip, &headers, CliPurpose::ClaudeCredential, secret).await
}

pub async fn codex_credential(
    State(state): State<AppState>,
    Extension(ip): Extension<ClientIp>,
    headers: HeaderMap,
    Json(body): Json<CodexBody>,
) -> Result<Response, ApiError> {
    let secret = codex_login(&body.auth_json)?;
    register(&state, ip, &headers, CliPurpose::CodexCredential, secret).await
}

fn codex_login(auth_json: &Value) -> Result<Secret, ApiError> {
    let invalid = || {
        ApiError::BadRequest(
            "authJson must be the contents of ~/.codex/auth.json after codex login with ChatGPT"
                .into(),
        )
    };
    let parsed = match auth_json {
        Value::String(text) => serde_json::from_str::<Value>(text).map_err(|_| invalid())?,
        other => other.clone(),
    };
    let tokens = parsed
        .get("tokens")
        .filter(|t| t.is_object())
        .ok_or_else(invalid)?;
    let has = |name: &str| tokens[name].as_str().is_some_and(|text| !text.is_empty());
    if !has("refresh_token") && !has("access_token") {
        return Err(invalid());
    }
    let email = tokens["id_token"]
        .as_str()
        .and_then(|jwt| jwt.split('.').nth(1))
        .and_then(|payload| URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|claims| {
            claims["email"]
                .as_str()
                .or_else(|| claims["https://api.openai.com/profile"]["email"].as_str())
                .map(str::to_string)
        });
    let account = email.unwrap_or_else(|| {
        tokens["account_id"]
            .as_str()
            .map(mask_key)
            .unwrap_or_else(|| "ChatGPT".to_string())
    });
    Ok(Secret {
        value: serde_json::to_string(&parsed).map_err(|_| invalid())?,
        account,
    })
}

struct Grant {
    request_id: Uuid,
    token_hash: String,
    user_id: Uuid,
    tenant_id: Uuid,
    member_id: Uuid,
    scope: String,
}

async fn grant(
    state: &AppState,
    headers: &HeaderMap,
    purpose: CliPurpose,
) -> Result<(String, Grant), ApiError> {
    let (user_code, secret) = bearer(headers)
        .and_then(split_code)
        .ok_or(ApiError::Unauthorized)?;
    let request = state
        .store
        .get::<CliRequest>(&cli_request_key(&user_code))
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let token_hash = request.token_hash.clone().ok_or(ApiError::Unauthorized)?;
    let live = request
        .token_expires_at
        .is_some_and(|expires_at| Utc::now() < expires_at);
    if request.status != ISSUED || !live || !secret_matches(secret, &token_hash) {
        return Err(ApiError::Unauthorized);
    }
    if request.purpose()? != purpose {
        return Err(ApiError::Forbidden(format!(
            "the token was approved for {}",
            request.purpose
        )));
    }
    let missing = || ApiError::Internal(format!("cli request {} lacks its approval", request.id));
    Ok((
        user_code.clone(),
        Grant {
            request_id: request.id,
            token_hash,
            user_id: request.user_id.ok_or_else(missing)?,
            tenant_id: request.tenant_id.ok_or_else(missing)?,
            member_id: request.member_id.ok_or_else(missing)?,
            scope: request.scope.clone().ok_or_else(missing)?,
        },
    ))
}

fn credential_name(purpose: CliPurpose, scope: &str, email: &str) -> String {
    let product = match purpose {
        CliPurpose::ClaudeCredential => "Claude",
        CliPurpose::CodexCredential => "Codex",
    };
    match scope {
        "org" => format!("{product} subscription (sancli)"),
        _ => format!("{product} subscription (sancli, {email})"),
    }
}

async fn register(
    state: &AppState,
    ip: ClientIp,
    headers: &HeaderMap,
    purpose: CliPurpose,
    secret: Secret,
) -> Result<Response, ApiError> {
    let (user_code, grant) = grant(state, headers, purpose).await?;
    let tenant_id = grant.tenant_id;
    let mut check = state.store.tx(tenant_id);
    let tenant = check
        .get::<Tenant>(&keys::tenant_key(tenant_id))
        .await?
        .ok_or_else(|| ApiError::NotFound("the tenant does not exist".into()))?;
    if is_deleting(&tenant) {
        return Err(ApiError::Conflict(format!(
            "the tenant {} is being deleted",
            tenant.slug
        )));
    }
    let member = check
        .get::<Member>(&keys::member_key(tenant_id, grant.member_id))
        .await?
        .filter(|member| member.user_id == grant.user_id)
        .ok_or_else(|| ApiError::Forbidden("the approver is no longer a member".into()))?;
    if grant.scope == "org" && !ADMIN_ROLES.contains(&member.role.as_str()) {
        return Err(ApiError::Forbidden(
            "adding organization credentials needs the admin role or higher".into(),
        ));
    }
    let user = state
        .store
        .get::<User>(&keys::user_key(grant.user_id))
        .await?
        .ok_or_else(|| ApiError::Forbidden("the approver no longer exists".into()))?;
    let name = credential_name(purpose, &grant.scope, &user.email);
    let encrypted = state
        .config
        .credential_key
        .0
        .encrypt(tenant_id, secret.value.as_bytes())
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let encrypted_value = STANDARD.encode(&encrypted.ciphertext);
    let request_key = cli_request_key(&user_code);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let at = Utc::now();
        let mut tx = state.store.tx(tenant_id);
        let request = tx
            .get::<CliRequest>(&request_key)
            .await?
            .filter(|request| {
                request.id == grant.request_id
                    && request.status == ISSUED
                    && request.token_hash.as_deref() == Some(grant.token_hash.as_str())
            })
            .ok_or(ApiError::Unauthorized)?;
        tx.delete(&request.key())?;
        let guard_key = keys::credential_name_key(tenant_id, &name);
        let existing = match tx.get::<Guard>(&guard_key).await? {
            Some(guard) => Some(
                tx.require::<Credential>(&keys::credential_key(tenant_id, guard.target))
                    .await?,
            ),
            None => None,
        };
        let member_id = (grant.scope == "personal").then_some(grant.member_id);
        let (credential, replaced) = match existing {
            Some(mut credential) => {
                if credential.provider != purpose.provider()
                    || credential.kind != "subscription"
                    || credential.scope != grant.scope
                    || credential.member_id != member_id
                {
                    return Err(ApiError::Conflict(format!(
                        "a credential named {name} exists; remove it and run the command again"
                    )));
                }
                credential.account = secret.account.clone();
                credential.status = "connected".into();
                credential.rejected_at = None;
                credential.encrypted_value = encrypted_value.clone();
                credential.kms_key_id = encrypted.key_id.clone();
                credential.updated_at = at;
                tx.put(&credential)?;
                (credential, true)
            }
            None => {
                let credential = Credential {
                    id: Uuid::new_v4(),
                    tenant_id,
                    scope: grant.scope.clone(),
                    member_id,
                    name: name.clone(),
                    provider: purpose.provider().to_string(),
                    kind: "subscription".into(),
                    account: secret.account.clone(),
                    status: "connected".into(),
                    subscription_plan: None,
                    rejected_at: None,
                    encrypted_value: encrypted_value.clone(),
                    kms_key_id: encrypted.key_id.clone(),
                    last_used_at: None,
                    created_at: at,
                    updated_at: at,
                };
                tx.insert(&credential)?;
                tx.insert(&Guard::new(guard_key, credential.id))?;
                (credential, false)
            }
        };
        audit_connect(&mut tx, &grant, &credential, replaced, ip, at).await?;
        match tx.commit().await {
            Ok(()) => break,
            Err(error) if error.is_conflict() && attempt < ATTEMPTS => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok((StatusCode::CREATED, Json(json!({ "label": name }))).into_response())
}

async fn audit_connect(
    tx: &mut TenantTx,
    grant: &Grant,
    credential: &Credential,
    replaced: bool,
    ip: ClientIp,
    at: DateTime<Utc>,
) -> Result<(), DbError> {
    let mut metadata = Map::new();
    metadata.insert("target_name".into(), json!(credential.name));
    metadata.insert("user_id".into(), json!(grant.user_id));
    metadata.insert("cli_request_id".into(), json!(grant.request_id));
    let summary = format!(
        "{} the subscription credential {} with sancli",
        if replaced { "Replaced" } else { "Added" },
        credential.name
    );
    record_audit(
        tx,
        &AuditEntry {
            at,
            actor_kind: ActorKind::Member,
            actor_ref: &grant.member_id.to_string(),
            action: "credential.connect",
            target_kind: "credential",
            target_ref: &credential.id.to_string(),
            summary: &summary,
            origin: AuditOrigin::Cli,
            source_ip: Some(ip.0),
            metadata,
        },
    )
    .await?;
    tx.update(
        &keys::member_key(tx.tenant_id(), grant.member_id),
        Update::new().set("last_active_at", AttributeValue::S(format_time(at))),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: &Value) -> String {
        format!(
            "e30.{}.sig",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap())
        )
    }

    #[test]
    fn codex_logins_need_tokens_and_show_the_email() {
        let auth = json!({
            "OPENAI_API_KEY": null,
            "tokens": {
                "id_token": jwt(&json!({ "https://api.openai.com/profile": { "email": "ken@acme.test" } })),
                "access_token": "a",
                "refresh_token": "r",
                "account_id": "acct-0123456789abcdef",
            },
        });
        let login = codex_login(&Value::String(auth.to_string())).unwrap();
        assert_eq!(login.account, "ken@acme.test");
        assert_eq!(serde_json::from_str::<Value>(&login.value).unwrap(), auth);
        let without_email =
            json!({ "tokens": { "refresh_token": "r", "account_id": "acct-0123456789abcdef" } });
        assert_eq!(codex_login(&without_email).unwrap().account, "acct…cdef");
        assert!(codex_login(&json!({ "OPENAI_API_KEY": "sk-x" })).is_err());
        assert!(codex_login(&json!("not json")).is_err());
    }

    #[test]
    fn credentials_are_named_by_product_and_scope() {
        assert_eq!(
            credential_name(CliPurpose::ClaudeCredential, "org", "ken@acme.test"),
            "Claude subscription (sancli)"
        );
        assert_eq!(
            credential_name(CliPurpose::CodexCredential, "personal", "ken@acme.test"),
            "Codex subscription (sancli, ken@acme.test)"
        );
    }
}
