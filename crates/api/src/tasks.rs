use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use sanchoris_core::channel::{RunEventType, record_run_event};
use sanchoris_core::db::TenantTx;
use sanchoris_core::skillrun::{self, SkillRunError};
use sanchoris_core::tasks::latest_run;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::CurrentUser;
use crate::error::ApiError;

async fn require_member(
    state: &AppState,
    tenant_id: Uuid,
    user: &CurrentUser,
) -> Result<(), ApiError> {
    let member = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM sanchoris.members WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant_id)
    .bind(user.user_id)
    .fetch_optional(&state.pool)
    .await?;
    member
        .map(|_| ())
        .ok_or_else(|| ApiError::Forbidden("you are not a member of this tenant".into()))
}

impl From<SkillRunError> for ApiError {
    fn from(error: SkillRunError) -> Self {
        match error {
            SkillRunError::ApprovalNotWaiting => ApiError::Conflict(error.to_string()),
            SkillRunError::Token(_) => ApiError::BadRequest(error.to_string()),
            SkillRunError::Db(_) | SkillRunError::Json(_) | SkillRunError::Engine(_) => {
                ApiError::Internal(error.to_string())
            }
            other => ApiError::Unprocessable {
                code: other.code(),
                message: other.to_string(),
            },
        }
    }
}

#[derive(Deserialize)]
pub struct Approval {
    token: String,
    approved: bool,
}

pub async fn resolve_approval(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<Approval>,
) -> Result<Json<Value>, ApiError> {
    let verified = state
        .config
        .task_tokens
        .verify(&body.token)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let tenant_id =
        sqlx::query_scalar::<_, Uuid>("SELECT tenant_id FROM sanchoris.skill_runs WHERE id = $1")
            .bind(verified.skill_run_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| ApiError::NotFound("the approval does not exist".into()))?;
    require_member(&state, tenant_id, &user).await?;
    let mut tx = TenantTx::begin(&state.pool, tenant_id).await?;
    skillrun::resolve_approval(
        &mut tx,
        &state.config.task_tokens,
        &body.token,
        body.approved,
        &user.display_name,
        Utc::now(),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({
        "status": if body.approved { "approved" } else { "rejected" },
    })))
}

#[derive(Deserialize)]
pub struct Reply {
    text: String,
}

pub async fn reply(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(task_id): Path<Uuid>,
    Json(body): Json<Reply>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if body.text.trim().is_empty() {
        return Err(ApiError::BadRequest("the reply is empty".into()));
    }
    let tenant_id =
        sqlx::query_scalar::<_, Uuid>("SELECT tenant_id FROM sanchoris.tasks WHERE id = $1")
            .bind(task_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("task {task_id}")))?;
    require_member(&state, tenant_id, &user).await?;
    let mut tx = TenantTx::begin(&state.pool, tenant_id).await?;
    let run_id = latest_run(&mut tx, task_id)
        .await?
        .ok_or_else(|| ApiError::Conflict("the task has no session to reply to".into()))?;
    let recorded = record_run_event(
        &mut tx,
        run_id,
        None,
        RunEventType::HumanReply,
        &json!({ "member": user.display_name, "text": body.text }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "id": recorded.id(), "runId": run_id })),
    ))
}
