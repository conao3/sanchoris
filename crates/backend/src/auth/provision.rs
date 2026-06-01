use crate::auth::verify::{AuthError, AuthIdentity};
use sqlx::PgPool;
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone)]
pub struct AuthContext {
    pub user_id: Uuid,
    pub cognito_subject: String,
    pub display_name: String,
    pub email: String,
}

pub async fn build_auth_context(
    pool: &PgPool,
    identity: &AuthIdentity,
) -> Result<AuthContext, AuthError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| AuthError::Internal(format!("failed to begin transaction: {error}")))?;

    sqlx::query("select pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("cognito:{}", identity.sub))
        .execute(&mut *tx)
        .await
        .map_err(|error| AuthError::Internal(format!("failed to acquire advisory lock: {error}")))?;

    let existing = sqlx::query(
        "SELECT id, email, display_name FROM sanchoris.users WHERE cognito_subject = $1",
    )
    .bind(&identity.sub)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| AuthError::Internal(format!("failed to query user: {error}")))?;

    let context = match existing {
        Some(row) => AuthContext {
            user_id: row
                .try_get("id")
                .map_err(|error| AuthError::Internal(format!("failed to read user id: {error}")))?,
            cognito_subject: identity.sub.clone(),
            email: row
                .try_get("email")
                .map_err(|error| AuthError::Internal(format!("failed to read email: {error}")))?,
            display_name: row.try_get("display_name").map_err(|error| {
                AuthError::Internal(format!("failed to read display_name: {error}"))
            })?,
        },
        None => {
            let user_id = Uuid::new_v4();
            let email = identity
                .email
                .clone()
                .unwrap_or_else(|| format!("{}@users.noreply.sanchoris", identity.sub));
            let display_name = identity.username.clone();

            let row = sqlx::query(
                "INSERT INTO sanchoris.users (id, cognito_subject, email, display_name) \
                 VALUES ($1, $2, $3, $4) \
                 RETURNING id, email, display_name",
            )
            .bind(user_id)
            .bind(&identity.sub)
            .bind(&email)
            .bind(&display_name)
            .fetch_one(&mut *tx)
            .await
            .map_err(|error| AuthError::Internal(format!("failed to insert user: {error}")))?;

            AuthContext {
                user_id: row.try_get("id").map_err(|error| {
                    AuthError::Internal(format!("failed to read user id: {error}"))
                })?,
                cognito_subject: identity.sub.clone(),
                email: row.try_get("email").map_err(|error| {
                    AuthError::Internal(format!("failed to read email: {error}"))
                })?,
                display_name: row.try_get("display_name").map_err(|error| {
                    AuthError::Internal(format!("failed to read display_name: {error}"))
                })?,
            }
        }
    };

    tx.commit()
        .await
        .map_err(|error| AuthError::Internal(format!("failed to commit transaction: {error}")))?;

    Ok(context)
}
