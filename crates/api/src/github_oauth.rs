use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use chrono::{DateTime, Duration, Utc};
use sanchoris_core::audit::{AuditEntry, record_audit};
use sanchoris_core::db::TenantTx;
use sanchoris_core::model::{ActorKind, AuditOrigin};
use sanchoris_graphql::GithubConnectUrl;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::CurrentUser;
use crate::config::ServeConfig;
use crate::error::ApiError;
use crate::signing::HmacKey;

const STATE_TTL_MINUTES: i64 = 10;
const RESULT_PATH: &str = "/settings";
const PER_PAGE: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthState {
    Link {
        user_id: Uuid,
    },
    Install {
        tenant_id: Uuid,
        user_id: Uuid,
    },
    Installation {
        tenant_id: Uuid,
        user_id: Uuid,
        installation_id: i64,
    },
}

impl OAuthState {
    fn encode(&self) -> String {
        match self {
            Self::Link { user_id } => format!("link.{user_id}"),
            Self::Install { tenant_id, user_id } => format!("install.{tenant_id}.{user_id}"),
            Self::Installation {
                tenant_id,
                user_id,
                installation_id,
            } => format!("installation.{tenant_id}.{user_id}.{installation_id}"),
        }
    }

    fn decode(parts: &[&str]) -> Option<Self> {
        match parts {
            ["link", user_id] => Some(Self::Link {
                user_id: user_id.parse().ok()?,
            }),
            ["install", tenant_id, user_id] => Some(Self::Install {
                tenant_id: tenant_id.parse().ok()?,
                user_id: user_id.parse().ok()?,
            }),
            ["installation", tenant_id, user_id, installation_id] => Some(Self::Installation {
                tenant_id: tenant_id.parse().ok()?,
                user_id: user_id.parse().ok()?,
                installation_id: installation_id.parse().ok()?,
            }),
            _ => None,
        }
    }
}

pub fn issue_state(key: &HmacKey, state: OAuthState, now: DateTime<Utc>) -> String {
    let expires = (now + Duration::minutes(STATE_TTL_MINUTES)).timestamp();
    key.seal(&format!(
        "{}.{expires}.{}",
        state.encode(),
        Uuid::new_v4().simple()
    ))
}

pub fn verify_state(key: &HmacKey, sealed: &str, now: DateTime<Utc>) -> Option<OAuthState> {
    let message = key.open(sealed)?;
    let parts = message.split('.').collect::<Vec<_>>();
    let [state @ .., expires, _nonce] = parts.as_slice() else {
        return None;
    };
    let expires: i64 = expires.parse().ok()?;
    (now.timestamp() < expires)
        .then(|| OAuthState::decode(state))
        .flatten()
}

fn redirect_uri(config: &ServeConfig) -> String {
    format!("{}/auth/github/callback", config.public_url)
}

pub fn authorize_url(config: &ServeConfig, state: &str) -> String {
    let mut url = reqwest::Url::parse(&format!("{}/login/oauth/authorize", config.github.web_url))
        .expect("GITHUB_URL is a valid URL");
    url.query_pairs_mut()
        .append_pair("client_id", &config.github.client_id)
        .append_pair("redirect_uri", &redirect_uri(config))
        .append_pair("state", state);
    url.to_string()
}

pub fn install_url(config: &ServeConfig, state: &str) -> String {
    let mut url = reqwest::Url::parse(&format!(
        "{}/apps/{}/installations/new",
        config.github.web_url, config.github.app_slug
    ))
    .expect("GITHUB_URL is a valid URL");
    url.query_pairs_mut().append_pair("state", state);
    url.to_string()
}

pub struct GitHubLinks {
    config: Arc<ServeConfig>,
}

impl GitHubLinks {
    pub fn new(config: Arc<ServeConfig>) -> Self {
        Self { config }
    }
}

impl GithubConnectUrl for GitHubLinks {
    fn link_account(&self, user_id: Uuid) -> String {
        let state = issue_state(
            &self.config.oauth_state_key,
            OAuthState::Link { user_id },
            Utc::now(),
        );
        authorize_url(&self.config, &state)
    }

    fn install_app(&self, tenant_id: Uuid, user_id: Uuid) -> String {
        let state = issue_state(
            &self.config.oauth_state_key,
            OAuthState::Install { tenant_id, user_id },
            Utc::now(),
        );
        install_url(&self.config, &state)
    }
}

pub async fn start(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({
        "url": GitHubLinks::new(state.config.clone()).link_account(user.user_id),
    })))
}

fn result(state: &AppState, outcome: &str) -> Response {
    Redirect::to(&format!(
        "{}{RESULT_PATH}?github={outcome}",
        state.config.public_url
    ))
    .into_response()
}

#[derive(Deserialize)]
pub struct Setup {
    installation_id: Option<i64>,
    setup_action: Option<String>,
    state: Option<String>,
}

pub async fn setup(
    State(state): State<AppState>,
    Query(query): Query<Setup>,
) -> Result<Response, ApiError> {
    let Some(OAuthState::Install { tenant_id, user_id }) = query
        .state
        .as_deref()
        .and_then(|value| verify_state(&state.config.oauth_state_key, value, Utc::now()))
    else {
        return Err(ApiError::BadRequest(
            "the state is invalid or expired".into(),
        ));
    };
    let installation_id = match (query.setup_action.as_deref(), query.installation_id) {
        (Some("request"), _) | (_, None) => return Ok(result(&state, "requested")),
        (_, Some(id)) => id,
    };
    let next = issue_state(
        &state.config.oauth_state_key,
        OAuthState::Installation {
            tenant_id,
            user_id,
            installation_id,
        },
        Utc::now(),
    );
    Ok(Redirect::to(&authorize_url(&state.config, &next)).into_response())
}

#[derive(Deserialize)]
pub struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct GitHubUser {
    id: i64,
    login: String,
}

fn github_get(state: &AppState, path: &str, token: &str) -> reqwest::RequestBuilder {
    state
        .http
        .get(format!("{}{path}", state.config.github.api_url))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "sanchoris")
}

async fn exchange(state: &AppState, code: &str) -> Result<String, ApiError> {
    let github = &state.config.github;
    let token: Value = state
        .http
        .post(format!("{}/login/oauth/access_token", github.web_url))
        .header("Accept", "application/json")
        .json(&json!({
            "client_id": github.client_id,
            "client_secret": github.client_secret,
            "code": code,
            "redirect_uri": redirect_uri(&state.config),
        }))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| ApiError::Internal(format!("GitHub token exchange: {error}")))?
        .json()
        .await
        .map_err(|error| ApiError::Internal(format!("GitHub token exchange: {error}")))?;
    token
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ApiError::Unprocessable {
            code: "github_oauth_failed",
            message: format!("GitHub did not return an access token: {}", token["error"]),
        })
}

async fn github_user(state: &AppState, token: &str) -> Result<GitHubUser, ApiError> {
    github_get(state, "/user", token)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| ApiError::Internal(format!("GitHub /user: {error}")))?
        .json()
        .await
        .map_err(|error| ApiError::Internal(format!("GitHub /user: {error}")))
}

struct UserInstallation {
    login: String,
    kind: String,
}

async fn user_installation(
    state: &AppState,
    token: &str,
    installation_id: i64,
) -> Result<Option<UserInstallation>, ApiError> {
    for page in 1.. {
        let body: Value = github_get(
            state,
            &format!("/user/installations?per_page={PER_PAGE}&page={page}"),
            token,
        )
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| ApiError::Internal(format!("GitHub /user/installations: {error}")))?
        .json()
        .await
        .map_err(|error| ApiError::Internal(format!("GitHub /user/installations: {error}")))?;
        let installations = body["installations"].as_array().ok_or_else(|| {
            ApiError::Internal("GitHub /user/installations has no installations".into())
        })?;
        if let Some(found) = installations
            .iter()
            .find(|item| item["id"].as_i64() == Some(installation_id))
        {
            let text = |pointer: &str| {
                found
                    .pointer(pointer)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| {
                        ApiError::Internal(format!("the GitHub installation has no {pointer}"))
                    })
            };
            return Ok(Some(UserInstallation {
                login: text("/account/login")?,
                kind: text("/account/type")?,
            }));
        }
        if installations.len() < PER_PAGE {
            break;
        }
    }
    Ok(None)
}

async fn memberships(pool: &PgPool, user_id: Uuid) -> Result<Vec<(Uuid, Uuid, String)>, ApiError> {
    Ok(sqlx::query_as::<_, (Uuid, Uuid, String)>(
        "SELECT tenant_id, id, role FROM sanchoris.members WHERE user_id = $1 ORDER BY tenant_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

struct Audit<'a> {
    member_id: Uuid,
    action: &'a str,
    target_kind: &'a str,
    target_ref: &'a str,
    summary: &'a str,
}

async fn audit(tx: &mut TenantTx, entry: Audit<'_>) -> Result<(), ApiError> {
    record_audit(
        tx,
        &AuditEntry {
            at: Utc::now(),
            actor_kind: ActorKind::Member,
            actor_ref: &entry.member_id.to_string(),
            action: entry.action,
            target_kind: entry.target_kind,
            target_ref: entry.target_ref,
            summary: entry.summary,
            origin: AuditOrigin::Web,
            source_ip: None,
            metadata: Map::new(),
        },
    )
    .await?;
    Ok(())
}

async fn link(state: &AppState, user_id: Uuid, github: &GitHubUser) -> Result<Response, ApiError> {
    let updated = sqlx::query(
        "UPDATE sanchoris.users SET github_user_id = $2, github_login = $3, updated_at = now() WHERE id = $1",
    )
    .bind(user_id)
    .bind(github.id)
    .bind(&github.login)
    .execute(&state.pool)
    .await;
    match updated {
        Ok(done) if done.rows_affected() == 1 => {}
        Ok(_) => return Err(ApiError::NotFound(format!("user {user_id}"))),
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            return Ok(result(state, "in_use"));
        }
        Err(error) => return Err(error.into()),
    }
    for (tenant_id, member_id, _) in memberships(&state.pool, user_id).await? {
        let mut tx = TenantTx::begin(&state.pool, tenant_id).await?;
        audit(
            &mut tx,
            Audit {
                member_id,
                action: "integration.github_link",
                target_kind: "user",
                target_ref: &user_id.to_string(),
                summary: &format!("Linked the GitHub account {}", github.login),
            },
        )
        .await?;
        tx.commit().await?;
    }
    Ok(result(state, "linked"))
}

async fn connect_installation(
    state: &AppState,
    tenant_id: Uuid,
    user_id: Uuid,
    installation_id: i64,
    token: &str,
) -> Result<Response, ApiError> {
    let Some((_, member_id, role)) = memberships(&state.pool, user_id)
        .await?
        .into_iter()
        .find(|(tenant, _, _)| *tenant == tenant_id)
    else {
        return Ok(result(state, "not_member"));
    };
    if role == "member" {
        return Ok(result(state, "not_admin"));
    }
    let Some(account) = user_installation(state, token, installation_id).await? else {
        return Ok(result(state, "not_accessible"));
    };
    let repositories = state
        .github_app
        .installation_repositories(installation_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let details = json!({
        "account_login": account.login,
        "account_type": account.kind,
        "repositories": repositories,
    });
    let mut tx = TenantTx::begin(&state.pool, tenant_id).await?;
    let owner = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO sanchoris.integrations (tenant_id, kind, status, github_installation_id, details) \
         VALUES ($1, 'github_installation', 'connected', $2, $3) \
         ON CONFLICT (github_installation_id) DO UPDATE SET status = 'connected', details = EXCLUDED.details, updated_at = now() \
         WHERE sanchoris.integrations.tenant_id = EXCLUDED.tenant_id \
         RETURNING tenant_id",
    )
    .bind(tenant_id)
    .bind(installation_id)
    .bind(&details)
    .fetch_optional(tx.conn())
    .await?;
    if owner.is_none() {
        return Ok(result(state, "installation_in_use"));
    }
    audit(
        &mut tx,
        Audit {
            member_id,
            action: "integration.github_install",
            target_kind: "github_installation",
            target_ref: &installation_id.to_string(),
            summary: &format!(
                "Connected the GitHub App installation on {} ({} repositories)",
                account.login,
                repositories.len()
            ),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(result(state, "installed"))
}

pub async fn callback(
    State(state): State<AppState>,
    Query(query): Query<Callback>,
) -> Result<Response, ApiError> {
    let verified = query
        .state
        .as_deref()
        .and_then(|value| verify_state(&state.config.oauth_state_key, value, Utc::now()))
        .ok_or_else(|| ApiError::BadRequest("the state is invalid or expired".into()))?;
    if matches!(verified, OAuthState::Install { .. }) {
        return Err(ApiError::BadRequest(
            "the state belongs to the installation setup".into(),
        ));
    }
    if query.error.is_some() {
        return Ok(result(&state, "denied"));
    }
    let code = query
        .code
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest("missing code".into()))?;
    let token = exchange(&state, code).await?;
    match verified {
        OAuthState::Link { user_id } => {
            let github = github_user(&state, &token).await?;
            link(&state, user_id, &github).await
        }
        OAuthState::Installation {
            tenant_id,
            user_id,
            installation_id,
        } => connect_installation(&state, tenant_id, user_id, installation_id, &token).await,
        OAuthState::Install { .. } => unreachable!("install states are rejected above"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn states_are_bound_to_their_subjects_and_expire() {
        let key = HmacKey::strong(&[9u8; 32]).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap();
        let user_id = Uuid::new_v4();
        let link = OAuthState::Link { user_id };
        let sealed = issue_state(&key, link, now);
        assert_eq!(verify_state(&key, &sealed, now), Some(link));
        assert_eq!(
            verify_state(&key, &sealed, now + Duration::minutes(STATE_TTL_MINUTES)),
            None
        );
        let other = Uuid::new_v4();
        assert_eq!(
            verify_state(
                &key,
                &sealed.replace(&user_id.to_string(), &other.to_string()),
                now
            ),
            None
        );
        assert_ne!(issue_state(&key, link, now), sealed);
        let installation = OAuthState::Installation {
            tenant_id: Uuid::new_v4(),
            user_id,
            installation_id: 42,
        };
        assert_eq!(
            verify_state(&key, &issue_state(&key, installation, now), now),
            Some(installation)
        );
    }
}
