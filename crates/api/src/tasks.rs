use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use sanchoris_core::channel::{RunEventType, record_run_event};
use sanchoris_core::db::{DbError, REF_SKILL_RUN, Store, locate};
use sanchoris_core::items::{Membership, Task};
use sanchoris_core::keys;
use sanchoris_core::skillrun::{self, SkillRunError};
use sanchoris_core::store::retry;
use sanchoris_core::tasks::latest_run;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::CurrentUser;
use crate::error::ApiError;

const ATTEMPTS: u32 = 8;

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

pub async fn tenants_of(store: &Store, user_id: Uuid) -> Result<Vec<Uuid>, DbError> {
    Ok(store
        .query::<Membership>(&keys::user_key(user_id).pk, "MEMBER#")
        .await?
        .into_iter()
        .map(|membership| membership.tenant_id)
        .collect())
}

async fn tenant_of_skill_run(
    store: &Store,
    user_id: Uuid,
    skill_run_id: Uuid,
) -> Result<Option<Uuid>, DbError> {
    for tenant_id in tenants_of(store, user_id).await? {
        let mut tx = store.tx(tenant_id);
        if locate(&mut tx, skill_run_id, REF_SKILL_RUN)
            .await?
            .is_some()
        {
            return Ok(Some(tenant_id));
        }
    }
    Ok(None)
}

async fn tenant_of_task(
    store: &Store,
    user_id: Uuid,
    task_id: Uuid,
) -> Result<Option<Uuid>, DbError> {
    for tenant_id in tenants_of(store, user_id).await? {
        if store
            .get::<Task>(&keys::task_key(tenant_id, task_id))
            .await?
            .is_some()
        {
            return Ok(Some(tenant_id));
        }
    }
    Ok(None)
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
    let tenant_id = tenant_of_skill_run(&state.store, user.user_id, verified.skill_run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("the approval does not exist".into()))?;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let mut tx = state.store.tx(tenant_id);
        let resolved = skillrun::resolve_approval(
            &mut tx,
            &state.config.task_tokens,
            &body.token,
            body.approved,
            &user.display_name,
            Utc::now(),
        )
        .await;
        let committed = match resolved {
            Ok(_) => tx.commit().await.map_err(SkillRunError::from),
            Err(error) => Err(error),
        };
        match committed {
            Err(error) if error.is_conflict() && attempt < ATTEMPTS => {}
            Err(error) => return Err(error.into()),
            Ok(()) => break,
        }
    }
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
    let tenant_id = tenant_of_task(&state.store, user.user_id, task_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("task {task_id}")))?;
    let payload = json!({ "member": user.display_name, "text": body.text });
    let replied = retry(ATTEMPTS, || async {
        let mut tx = state.store.tx(tenant_id);
        let Some(run_id) = latest_run(&mut tx, task_id).await? else {
            return Ok(None);
        };
        let recorded =
            record_run_event(&mut tx, run_id, None, RunEventType::HumanReply, &payload).await?;
        tx.commit().await?;
        Ok(Some((recorded, run_id)))
    })
    .await?;
    let (recorded, run_id) =
        replied.ok_or_else(|| ApiError::Conflict("the task has no session to reply to".into()))?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "id": recorded.id(), "runId": run_id })),
    ))
}
