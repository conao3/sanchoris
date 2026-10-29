use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Redirect, Response};
use chrono::{DateTime, Duration, Utc};
use sanchoris_core::audit::{AuditEntry, record_audit};
use sanchoris_core::db::{DbError, Store, TenantTx};
use sanchoris_core::items::{Guard, Integration, Member, Membership, User, uniq};
use sanchoris_core::keys;
use sanchoris_core::model::{ActorKind, AuditOrigin};
use sanchoris_core::store::retry;
use sanchoris_graphql::{GithubConnectUrl, GithubRedirect};
use sanchoris_worker::ports::ConfigSource as _;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::CurrentUser;
use crate::config::ApiConfig;
use crate::error::ApiError;
use crate::signing::HmacKey;

const ATTEMPTS: u32 = 8;

const STATE_TTL_MINUTES: i64 = 10;
pub const BROWSER_COOKIE: &str = "__Host-sanchoris-github";
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

pub fn new_browser_nonce() -> String {
    Uuid::new_v4().simple().to_string()
}

pub fn browser_cookie(nonce: &str) -> String {
    format!(
        "{BROWSER_COOKIE}={nonce}; Path=/; Max-Age={}; Secure; HttpOnly; SameSite=Lax",
        STATE_TTL_MINUTES * 60
    )
}

fn browser_nonce(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == BROWSER_COOKIE)
        .map(|(_, value)| value)
}

pub fn issue_state(key: &HmacKey, state: OAuthState, nonce: &str, now: DateTime<Utc>) -> String {
    let expires = (now + Duration::minutes(STATE_TTL_MINUTES)).timestamp();
    key.seal(&format!("{}.{expires}.{nonce}", state.encode()))
}

pub fn verify_state(
    key: &HmacKey,
    sealed: &str,
    now: DateTime<Utc>,
) -> Option<(OAuthState, String)> {
    let message = key.open(sealed)?;
    let parts = message.split('.').collect::<Vec<_>>();
    let [state @ .., expires, nonce] = parts.as_slice() else {
        return None;
    };
    let expires: i64 = expires.parse().ok()?;
    if now.timestamp() >= expires {
        return None;
    }
    Some((OAuthState::decode(state)?, (*nonce).to_string()))
}

fn verify_browser_state(
    state: &AppState,
    sealed: Option<&str>,
    headers: &HeaderMap,
) -> Result<(OAuthState, String), ApiError> {
    let (verified, nonce) = sealed
        .and_then(|value| verify_state(&state.config.oauth_state_key, value, Utc::now()))
        .ok_or_else(|| ApiError::BadRequest("the state is invalid or expired".into()))?;
    if browser_nonce(headers) != Some(nonce.as_str()) {
        return Err(ApiError::BadRequest(
            "the GitHub flow was started in another browser session".into(),
        ));
    }
    Ok((verified, nonce))
}

fn redirect_uri(config: &ApiConfig) -> String {
    format!("{}/auth/github/callback", config.public_url)
}

pub fn authorize_url(config: &ApiConfig, state: &str) -> String {
    let mut url = reqwest::Url::parse(&format!("{}/login/oauth/authorize", config.github.web_url))
        .expect("GITHUB_URL is a valid URL");
    url.query_pairs_mut()
        .append_pair("client_id", &config.github.client_id)
        .append_pair("redirect_uri", &redirect_uri(config))
        .append_pair("state", state);
    url.to_string()
}

pub fn install_url(config: &ApiConfig, state: &str) -> String {
    let mut url = reqwest::Url::parse(&format!(
        "{}/apps/{}/installations/new",
        config.github.web_url, config.github.app_slug
    ))
    .expect("GITHUB_URL is a valid URL");
    url.query_pairs_mut().append_pair("state", state);
    url.to_string()
}

pub struct GitHubLinks {
    config: Arc<ApiConfig>,
}

impl GitHubLinks {
    pub fn new(config: Arc<ApiConfig>) -> Self {
        Self { config }
    }
}

impl GithubConnectUrl for GitHubLinks {
    fn link_account(&self, user_id: Uuid) -> GithubRedirect {
        let nonce = new_browser_nonce();
        let state = issue_state(
            &self.config.oauth_state_key,
            OAuthState::Link { user_id },
            &nonce,
            Utc::now(),
        );
        GithubRedirect {
            url: authorize_url(&self.config, &state),
            cookie: browser_cookie(&nonce),
        }
    }

    fn install_app(&self, tenant_id: Uuid, user_id: Uuid) -> GithubRedirect {
        let nonce = new_browser_nonce();
        let state = issue_state(
            &self.config.oauth_state_key,
            OAuthState::Install { tenant_id, user_id },
            &nonce,
            Utc::now(),
        );
        GithubRedirect {
            url: install_url(&self.config, &state),
            cookie: browser_cookie(&nonce),
        }
    }
}

pub async fn start(State(state): State<AppState>, user: CurrentUser) -> Response {
    let redirect = GitHubLinks::new(state.config.clone()).link_account(user.user_id);
    (
        [(header::SET_COOKIE, redirect.cookie)],
        Json(json!({ "url": redirect.url })),
    )
        .into_response()
}

fn result(state: &AppState, outcome: &str) -> Response {
    (
        [(
            header::SET_COOKIE,
            format!("{BROWSER_COOKIE}=; Path=/; Max-Age=0; Secure; HttpOnly; SameSite=Lax"),
        )],
        Redirect::to(&format!(
            "{}{RESULT_PATH}?github={outcome}",
            state.config.public_url
        )),
    )
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
    headers: HeaderMap,
    Query(query): Query<Setup>,
) -> Result<Response, ApiError> {
    let (OAuthState::Install { tenant_id, user_id }, nonce) =
        verify_browser_state(&state, query.state.as_deref(), &headers)?
    else {
        return Err(ApiError::BadRequest(
            "the state does not belong to the installation setup".into(),
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
        &nonce,
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

async fn memberships(store: &Store, user_id: Uuid) -> Result<Vec<Member>, DbError> {
    let mut members = Vec::new();
    for membership in store
        .query::<Membership>(&keys::user_key(user_id).pk, "MEMBER#")
        .await?
    {
        if let Some(member) = store
            .get::<Member>(&keys::member_key(
                membership.tenant_id,
                membership.member_id,
            ))
            .await?
        {
            members.push(member);
        }
    }
    members.sort_by_key(|member| member.tenant_id);
    Ok(members)
}

struct Audit<'a> {
    member_id: Uuid,
    action: &'a str,
    target_kind: &'a str,
    target_ref: &'a str,
    summary: &'a str,
}

async fn audit(tx: &mut TenantTx, entry: Audit<'_>) -> Result<(), DbError> {
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

fn guard_owned_by(guard: Option<&Guard>, owner: Uuid) -> bool {
    guard.is_none_or(|guard| guard.target == owner)
}

async fn link_account(store: &Store, user_id: Uuid, github: &GitHubUser) -> Result<bool, ApiError> {
    retry(ATTEMPTS, || async {
        let mut tx = store.global_tx();
        let mut user = tx
            .get::<User>(&keys::user_key(user_id))
            .await?
            .ok_or_else(|| DbError::Missing(format!("user {user_id}")))?;
        let id_key = keys::uniq_key(uniq::GITHUB_USER, &github.id.to_string());
        let login_key = keys::uniq_key(uniq::GITHUB_LOGIN, &github.login.to_lowercase());
        let id_guard = tx.get::<Guard>(&id_key).await?;
        let login_guard = tx.get::<Guard>(&login_key).await?;
        if !guard_owned_by(id_guard.as_ref(), user_id)
            || !guard_owned_by(login_guard.as_ref(), user_id)
        {
            return Ok(false);
        }
        if let Some(previous) = user.github_user_id.filter(|id| *id != github.id) {
            tx.delete(&keys::uniq_key(uniq::GITHUB_USER, &previous.to_string()))?;
        }
        if let Some(previous) = user
            .github_login
            .as_deref()
            .map(str::to_lowercase)
            .filter(|login| *login != github.login.to_lowercase())
        {
            tx.delete(&keys::uniq_key(uniq::GITHUB_LOGIN, &previous))?;
        }
        if id_guard.is_none() {
            tx.insert(&Guard::new(id_key, user_id))?;
        }
        if login_guard.is_none() {
            tx.insert(&Guard::new(login_key, user_id))?;
        }
        user.github_user_id = Some(github.id);
        user.github_login = Some(github.login.clone());
        user.updated_at = Utc::now();
        tx.put(&user)?;
        tx.commit().await?;
        Ok(true)
    })
    .await
    .map_err(|error| match error {
        DbError::Missing(message) => ApiError::NotFound(message),
        other => other.into(),
    })
}

async fn link(state: &AppState, user_id: Uuid, github: &GitHubUser) -> Result<Response, ApiError> {
    if !link_account(&state.store, user_id, github).await? {
        return Ok(result(state, "in_use"));
    }
    for member in memberships(&state.store, user_id).await? {
        retry(ATTEMPTS, || async {
            let mut tx = state.store.tx(member.tenant_id);
            audit(
                &mut tx,
                Audit {
                    member_id: member.id,
                    action: "integration.github_link",
                    target_kind: "user",
                    target_ref: &user_id.to_string(),
                    summary: &format!("Linked the GitHub account {}", github.login),
                },
            )
            .await?;
            tx.commit().await
        })
        .await?;
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
    let Some(member) = memberships(&state.store, user_id)
        .await?
        .into_iter()
        .find(|member| member.tenant_id == tenant_id)
    else {
        return Ok(result(state, "not_member"));
    };
    if member.role == "member" {
        return Ok(result(state, "not_admin"));
    }
    let linked = state
        .store
        .get::<User>(&keys::user_key(user_id))
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("user {user_id}")))?
        .github_user_id;
    let github = github_user(state, token).await?;
    match linked {
        None => return Ok(result(state, "not_linked")),
        Some(id) if id != github.id => return Ok(result(state, "not_linked_account")),
        Some(_) => {}
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
    let connected = retry(ATTEMPTS, || async {
        let mut tx = state.store.tx(tenant_id);
        let guard_key = keys::uniq_key(uniq::INSTALLATION, &installation_id.to_string());
        let now = Utc::now();
        match tx.get::<Guard>(&guard_key).await? {
            Some(guard) if guard.tenant_id != Some(tenant_id) => return Ok(false),
            Some(guard) => {
                let mut integration = tx
                    .require::<Integration>(&keys::integration_key(tenant_id, guard.target))
                    .await?;
                integration.status = "connected".into();
                integration.details = details.clone();
                integration.updated_at = now;
                tx.put(&integration)?;
            }
            None => {
                let integration = Integration {
                    id: Uuid::new_v4(),
                    tenant_id,
                    kind: "github_installation".into(),
                    status: "connected".into(),
                    github_installation_id: Some(installation_id),
                    slack_team_id: None,
                    details: details.clone(),
                    created_at: now,
                    updated_at: now,
                };
                tx.insert(&integration)?;
                tx.insert(&Guard::of_tenant(guard_key, integration.id, tenant_id))?;
            }
        }
        audit(
            &mut tx,
            Audit {
                member_id: member.id,
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
        Ok(true)
    })
    .await?;
    if !connected {
        return Ok(result(state, "installation_in_use"));
    }
    Ok(result(state, "installed"))
}

pub async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<Callback>,
) -> Result<Response, ApiError> {
    let (verified, _) = verify_browser_state(&state, query.state.as_deref(), &headers)?;
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
        let nonce = new_browser_nonce();
        let sealed = issue_state(&key, link, &nonce, now);
        assert_eq!(
            verify_state(&key, &sealed, now),
            Some((link, nonce.clone()))
        );
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
        assert_ne!(issue_state(&key, link, &new_browser_nonce(), now), sealed);
        let installation = OAuthState::Installation {
            tenant_id: Uuid::new_v4(),
            user_id,
            installation_id: 42,
        };
        assert_eq!(
            verify_state(&key, &issue_state(&key, installation, &nonce, now), now),
            Some((installation, nonce))
        );
    }

    #[test]
    fn the_browser_nonce_is_read_from_a_host_only_cookie() {
        let mut headers = HeaderMap::new();
        assert_eq!(browser_nonce(&headers), None);
        headers.insert(
            header::COOKIE,
            format!("other=1; {BROWSER_COOKIE}=abc; last=2")
                .parse()
                .unwrap(),
        );
        assert_eq!(browser_nonce(&headers), Some("abc"));
        let cookie = browser_cookie("abc");
        assert!(cookie.starts_with("__Host-sanchoris-github=abc; Path=/;"));
        assert!(cookie.contains("; Secure; HttpOnly; SameSite=Lax"));
        assert!(!cookie.contains("Domain"));
    }
}
