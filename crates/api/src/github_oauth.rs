use axum::Json;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::CurrentUser;
use crate::error::ApiError;
use crate::signing::HmacKey;

const STATE_TTL_MINUTES: i64 = 10;
const RESULT_PATH: &str = "/settings/integrations";

pub fn issue_state(key: &HmacKey, user_id: Uuid, now: DateTime<Utc>) -> String {
    let expires = (now + Duration::minutes(STATE_TTL_MINUTES)).timestamp();
    key.seal(&format!("{user_id}.{expires}.{}", Uuid::new_v4().simple()))
}

pub fn verify_state(key: &HmacKey, state: &str, now: DateTime<Utc>) -> Option<Uuid> {
    let message = key.open(state)?;
    let mut parts = message.split('.');
    let user_id = parts.next()?.parse().ok()?;
    let expires: i64 = parts.next()?.parse().ok()?;
    parts.next()?;
    (parts.next().is_none() && now.timestamp() < expires).then_some(user_id)
}

fn redirect_uri(state: &AppState) -> String {
    format!("{}/auth/github/callback", state.config.public_url)
}

pub async fn start(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, ApiError> {
    let mut url = reqwest::Url::parse(&format!(
        "{}/login/oauth/authorize",
        state.config.github.web_url
    ))
    .map_err(|error| ApiError::Internal(error.to_string()))?;
    url.query_pairs_mut()
        .append_pair("client_id", &state.config.github.client_id)
        .append_pair("redirect_uri", &redirect_uri(&state))
        .append_pair(
            "state",
            &issue_state(&state.config.oauth_state_key, user.user_id, Utc::now()),
        );
    Ok(Json(json!({ "url": url.to_string() })))
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

fn result(state: &AppState, outcome: &str) -> Response {
    Redirect::to(&format!(
        "{}{RESULT_PATH}?github={outcome}",
        state.config.public_url
    ))
    .into_response()
}

async fn exchange(state: &AppState, code: &str) -> Result<GitHubUser, ApiError> {
    let github = &state.config.github;
    let token: Value = state
        .http
        .post(format!("{}/login/oauth/access_token", github.web_url))
        .header("Accept", "application/json")
        .json(&json!({
            "client_id": github.client_id,
            "client_secret": github.client_secret,
            "code": code,
            "redirect_uri": redirect_uri(state),
        }))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| ApiError::Internal(format!("GitHub token exchange: {error}")))?
        .json()
        .await
        .map_err(|error| ApiError::Internal(format!("GitHub token exchange: {error}")))?;
    let access_token = token
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::Unprocessable {
            code: "github_oauth_failed",
            message: format!("GitHub did not return an access token: {}", token["error"]),
        })?;
    state
        .http
        .get(format!("{}/user", github.api_url))
        .bearer_auth(access_token)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "sanchoris")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| ApiError::Internal(format!("GitHub /user: {error}")))?
        .json()
        .await
        .map_err(|error| ApiError::Internal(format!("GitHub /user: {error}")))
}

pub async fn callback(
    State(state): State<AppState>,
    Query(query): Query<Callback>,
) -> Result<Response, ApiError> {
    let user_id = query
        .state
        .as_deref()
        .and_then(|value| verify_state(&state.config.oauth_state_key, value, Utc::now()))
        .ok_or_else(|| ApiError::BadRequest("the state is invalid or expired".into()))?;
    if query.error.is_some() {
        return Ok(result(&state, "denied"));
    }
    let code = query
        .code
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest("missing code".into()))?;
    let github = exchange(&state, code).await?;
    let updated = sqlx::query(
        "UPDATE sanchoris.users SET github_user_id = $2, github_login = $3, updated_at = now() WHERE id = $1",
    )
    .bind(user_id)
    .bind(github.id)
    .bind(&github.login)
    .execute(&state.pool)
    .await;
    match updated {
        Ok(done) if done.rows_affected() == 1 => Ok(result(&state, "linked")),
        Ok(_) => Err(ApiError::NotFound(format!("user {user_id}"))),
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            Ok(result(&state, "in_use"))
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn states_are_bound_to_the_user_and_expire() {
        let key = HmacKey::strong(&[9u8; 32]).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap();
        let user = Uuid::new_v4();
        let state = issue_state(&key, user, now);
        assert_eq!(verify_state(&key, &state, now), Some(user));
        assert_eq!(
            verify_state(&key, &state, now + Duration::minutes(STATE_TTL_MINUTES)),
            None
        );
        let other = Uuid::new_v4();
        assert_eq!(
            verify_state(
                &key,
                &state.replace(&user.to_string(), &other.to_string()),
                now
            ),
            None
        );
        assert_ne!(issue_state(&key, user, now), state);
    }
}
