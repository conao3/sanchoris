use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Utc};
use sanchoris_core::audit::{AuditEntry, record_audit};
use sanchoris_core::db::{DbError, REF_SKILL_RUN, Store, TenantTx, locate};
use sanchoris_core::items::{Member, Membership};
use sanchoris_core::keys;
use sanchoris_core::model::{ActorKind, AuditOrigin};
use sanchoris_core::skillrun::{self, SkillRunError};
use sanchoris_core::tasks::{load_task, requester_member};
use serde::Deserialize;
use serde_json::{Map, Value, json};
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

const APPROVER_ROLES: [&str; 2] = ["owner", "admin"];

async fn approver(
    tx: &mut TenantTx,
    user_id: Uuid,
    skill_run_id: Uuid,
) -> Result<(Uuid, Uuid), ApiError> {
    let membership = tx
        .get::<Membership>(&keys::membership_key(user_id, tx.tenant_id()))
        .await?
        .ok_or_else(|| ApiError::NotFound("the approval does not exist".into()))?;
    let member = tx
        .get::<Member>(&keys::member_key(tx.tenant_id(), membership.member_id))
        .await?
        .ok_or_else(|| ApiError::NotFound("the approval does not exist".into()))?;
    let skill_run = skillrun::skill_run_item(tx, skill_run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("the approval does not exist".into()))?;
    let task = load_task(tx, skill_run.task_id).await?;
    let allowed = APPROVER_ROLES.contains(&member.role.as_str())
        || requester_member(tx, &task).await? == Some(member.id);
    if !allowed {
        return Err(ApiError::Forbidden(
            "only an owner, an admin, or the member who requested the task can resolve its approvals"
                .into(),
        ));
    }
    Ok((member.id, task.id))
}

async fn audit_approval(
    tx: &mut TenantTx,
    member_id: Uuid,
    task_id: Uuid,
    skill_run_id: Uuid,
    approved: bool,
    now: DateTime<Utc>,
) -> Result<(), DbError> {
    let mut metadata = Map::new();
    metadata.insert("skill_run_id".into(), json!(skill_run_id));
    metadata.insert("approved".into(), json!(approved));
    record_audit(
        tx,
        &AuditEntry {
            at: now,
            actor_kind: ActorKind::Member,
            actor_ref: &member_id.to_string(),
            action: "approval.resolve",
            target_kind: "task",
            target_ref: &task_id.to_string(),
            summary: if approved {
                "Approved the request"
            } else {
                "Rejected the request"
            },
            origin: AuditOrigin::Web,
            source_ip: None,
            metadata,
        },
    )
    .await?;
    Ok(())
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
        let now = Utc::now();
        let mut tx = state.store.tx(tenant_id);
        let (member_id, task_id) = approver(&mut tx, user.user_id, verified.skill_run_id).await?;
        let resolved = skillrun::resolve_approval(
            &mut tx,
            &state.config.task_tokens,
            &body.token,
            body.approved,
            &user.display_name,
            now,
        )
        .await;
        let resolved = match resolved {
            Ok(_) => audit_approval(
                &mut tx,
                member_id,
                task_id,
                verified.skill_run_id,
                body.approved,
                now,
            )
            .await
            .map_err(SkillRunError::from),
            Err(error) => Err(error),
        };
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
