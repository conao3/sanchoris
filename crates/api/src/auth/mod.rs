pub mod cognito;

use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::app::AppState;
use crate::error::ApiError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub subject: String,
    pub email: Option<String>,
    pub display_name: String,
}

#[derive(Debug)]
pub enum AuthError {
    Unauthorized,
    Internal(String),
}

impl From<AuthError> for ApiError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::Unauthorized => ApiError::Unauthorized,
            AuthError::Internal(message) => ApiError::Internal(message),
        }
    }
}

#[async_trait]
pub trait IdentityVerifier: Send + Sync {
    async fn verify(&self, token: &str) -> Result<Identity, AuthError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentUser {
    pub user_id: Uuid,
    pub cognito_subject: String,
    pub email: String,
    pub display_name: String,
    pub github_login: Option<String>,
}

pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
}

fn user(row: &sqlx::postgres::PgRow) -> Result<CurrentUser, sqlx::Error> {
    Ok(CurrentUser {
        user_id: row.try_get("id")?,
        cognito_subject: row.try_get("cognito_subject")?,
        email: row.try_get("email")?,
        display_name: row.try_get("display_name")?,
        github_login: row.try_get("github_login")?,
    })
}

pub async fn provision(pool: &PgPool, identity: &Identity) -> Result<CurrentUser, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("cognito:{}", identity.subject))
        .execute(&mut *tx)
        .await?;
    let existing = sqlx::query(
        "SELECT id, cognito_subject, email, display_name, github_login FROM sanchoris.users \
         WHERE cognito_subject = $1",
    )
    .bind(&identity.subject)
    .fetch_optional(&mut *tx)
    .await?;
    let row = match existing {
        Some(row) => row,
        None => {
            let email = identity
                .email
                .clone()
                .unwrap_or_else(|| format!("{}@users.noreply.sanchoris", identity.subject));
            sqlx::query(
                "INSERT INTO sanchoris.users (id, cognito_subject, email, display_name) \
                 VALUES ($1, $2, $3, $4) RETURNING id, cognito_subject, email, display_name, github_login",
            )
            .bind(Uuid::new_v4())
            .bind(&identity.subject)
            .bind(email)
            .bind(&identity.display_name)
            .fetch_one(&mut *tx)
            .await?
        }
    };
    let current = user(&row)?;
    tx.commit().await?;
    Ok(current)
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = bearer(&parts.headers).ok_or(ApiError::Unauthorized)?;
        let identity = state.verifier.verify(token).await?;
        Ok(provision(&state.pool, &identity).await?)
    }
}
