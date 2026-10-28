pub mod cognito;

use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
use chrono::Utc;
use sanchoris_core::billing::is_privileged_email;
use sanchoris_core::db::{DbError, Store};
use sanchoris_core::items::{Guard, User, uniq};
use sanchoris_core::keys;
use sanchoris_core::store::retry;
use uuid::Uuid;

use crate::app::AppState;
use crate::error::ApiError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub display_name: String,
}

impl Identity {
    pub fn privileged(&self) -> bool {
        self.email_verified && self.email.as_deref().is_some_and(is_privileged_email)
    }
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
    pub privileged: bool,
}

impl CurrentUser {
    fn new(user: User, privileged: bool) -> Self {
        Self {
            user_id: user.id,
            cognito_subject: user.cognito_subject,
            email: user.email,
            display_name: user.display_name,
            github_login: user.github_login,
            privileged,
        }
    }
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

enum Provisioned {
    User(Box<User>),
    EmailTaken,
}

pub async fn provision(store: &Store, identity: &Identity) -> Result<CurrentUser, ApiError> {
    let email = identity
        .email
        .clone()
        .unwrap_or_else(|| format!("{}@users.noreply.sanchoris", identity.subject));
    let provisioned = retry(8, || async {
        let mut tx = store.global_tx();
        let subject = keys::uniq_key(uniq::COGNITO_SUBJECT, &identity.subject);
        if let Some(guard) = tx.get::<Guard>(&subject).await? {
            let user = tx.require::<User>(&keys::user_key(guard.target)).await?;
            return Ok::<_, DbError>(Provisioned::User(Box::new(user)));
        }
        let email_key = keys::uniq_key(uniq::EMAIL, &email.to_lowercase());
        if tx.get::<Guard>(&email_key).await?.is_some() {
            return Ok(Provisioned::EmailTaken);
        }
        let now = Utc::now();
        let user = User {
            id: Uuid::new_v4(),
            cognito_subject: identity.subject.clone(),
            email: email.clone(),
            display_name: identity.display_name.clone(),
            github_user_id: None,
            github_login: None,
            is_operator: false,
            created_at: now,
            updated_at: now,
        };
        tx.insert(&user)?;
        tx.insert(&Guard::new(subject, user.id))?;
        tx.insert(&Guard::new(email_key, user.id))?;
        tx.commit().await?;
        Ok(Provisioned::User(Box::new(user)))
    })
    .await?;
    match provisioned {
        Provisioned::User(user) => Ok(CurrentUser::new(*user, identity.privileged())),
        Provisioned::EmailTaken => Err(ApiError::Conflict(format!(
            "the email {email} belongs to another user"
        ))),
    }
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = bearer(&parts.headers).ok_or(ApiError::Unauthorized)?;
        let identity = state.verifier.verify(token).await?;
        provision(&state.store, &identity).await
    }
}
