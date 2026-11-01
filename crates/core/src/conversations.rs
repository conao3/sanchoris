use chrono::{DateTime, Utc};
use uuid::Uuid;

use serde_json::{Value, json};

use crate::channel::{OutboxType, enqueue_outbox};
use crate::db::{DbError, TenantTx};
use crate::item::Entity;
use crate::items::{
    AUTHOR_AGENT, ChatMessage, Conversation, ConversationTask, RunApproval, RunQuestion, Task,
};
use crate::keys;
use crate::tasks::load_task;

pub async fn load_conversation(
    tx: &mut TenantTx,
    conversation_id: Uuid,
) -> Result<Conversation, DbError> {
    tx.get::<Conversation>(&keys::conversation_key(tx.tenant_id(), conversation_id))
        .await?
        .ok_or_else(|| DbError::Missing(format!("conversation {conversation_id}")))
}

pub async fn latest_task(
    tx: &mut TenantTx,
    conversation: &Conversation,
) -> Result<Option<Task>, DbError> {
    if conversation.tasks == 0 {
        return Ok(None);
    }
    let key = keys::conversation_task_key(tx.tenant_id(), conversation.id, conversation.tasks);
    let entry = tx.get::<ConversationTask>(&key).await?.ok_or_else(|| {
        DbError::Missing(format!(
            "task {} of conversation {}",
            conversation.tasks, conversation.id
        ))
    })?;
    Ok(Some(load_task(tx, entry.task_id).await?))
}

pub async fn post_message(tx: &mut TenantTx, message: &ChatMessage) -> Result<bool, DbError> {
    if tx.get::<ChatMessage>(&message.key()).await?.is_some() {
        return Ok(false);
    }
    tx.insert(message)?;
    let mut conversation = load_conversation(tx, message.conversation_id).await?;
    conversation.updated_at = conversation.updated_at.max(message.created_at);
    tx.put(&conversation)?;
    Ok(true)
}

pub async fn post_agent_message(
    tx: &mut TenantTx,
    task: &Task,
    run_id: Option<Uuid>,
    turn: Option<(i64, &str)>,
    id: Uuid,
    text: &str,
    at: DateTime<Utc>,
) -> Result<bool, DbError> {
    let Some(conversation_id) = task.origin.conversation_id() else {
        return Ok(false);
    };
    post_message(
        tx,
        &ChatMessage {
            id,
            tenant_id: tx.tenant_id(),
            conversation_id,
            task_id: Some(task.id),
            run_id,
            turn: turn.map(|(turn, _)| turn),
            outcome: turn.map(|(_, outcome)| outcome.to_string()),
            delivery: None,
            attachments: Vec::new(),
            author_kind: AUTHOR_AGENT.to_string(),
            member_id: None,
            text: text.to_string(),
            created_at: at,
        },
    )
    .await
}

pub async fn answer_question(
    tx: &mut TenantTx,
    run_id: Uuid,
    request_id: &str,
    answers: &Value,
    member_id: Uuid,
    now: DateTime<Utc>,
) -> Result<RunQuestion, DbError> {
    let key = keys::run_question_key(tx.tenant_id(), run_id, request_id);
    let mut question = tx
        .get::<RunQuestion>(&key)
        .await?
        .ok_or_else(|| DbError::Missing(format!("question {request_id} of run {run_id}")))?;
    if question.answers.is_some() {
        return Err(DbError::Conflict);
    }
    question.answers = Some(answers.clone());
    question.answered_by_member_id = Some(member_id);
    question.answered_at = Some(now);
    tx.put(&question)?;
    enqueue_outbox(
        tx,
        run_id,
        OutboxType::UserInputResponse,
        &json!({ "requestId": request_id, "answers": answers }),
        &format!("answer:{request_id}"),
    )
    .await?;
    Ok(question)
}

pub async fn decide_approval(
    tx: &mut TenantTx,
    run_id: Uuid,
    request_id: &str,
    decision: &str,
    member_id: Uuid,
    now: DateTime<Utc>,
) -> Result<RunApproval, DbError> {
    let key = keys::run_approval_key(tx.tenant_id(), run_id, request_id);
    let mut approval = tx
        .get::<RunApproval>(&key)
        .await?
        .ok_or_else(|| DbError::Missing(format!("approval {request_id} of run {run_id}")))?;
    if approval.decision.is_some() {
        return Err(DbError::Conflict);
    }
    approval.decision = Some(decision.to_string());
    approval.decided_by_member_id = Some(member_id);
    approval.decided_at = Some(now);
    tx.put(&approval)?;
    enqueue_outbox(
        tx,
        run_id,
        OutboxType::ApprovalResponse,
        &json!({ "requestId": request_id, "decision": decision }),
        &format!("approval:{request_id}"),
    )
    .await?;
    Ok(approval)
}

/// The outcome of a reply whose turn ended while the agent had already started the next one.
pub const OUTCOME_CONTINUED: &str = "continued";

/// A member's message held while the agent works on a turn, delivered when the turn ends.
pub const DELIVERY_QUEUED: &str = "queued";
/// A member's message delivered into the turn the agent was working on.
pub const DELIVERY_STEER: &str = "steer";

async fn task_messages(
    tx: &mut TenantTx,
    conversation_id: Uuid,
    task_id: Uuid,
) -> Result<Vec<ChatMessage>, DbError> {
    let mut messages = tx
        .query::<ChatMessage>(
            &keys::conversation_pk(tx.tenant_id(), conversation_id),
            "MSG#",
        )
        .await?
        .into_iter()
        .filter(|message| message.task_id == Some(task_id))
        .collect::<Vec<_>>();
    messages.sort_by_key(|message| (message.created_at, message.id));
    Ok(messages)
}

/// Whether the agent works on a turn of the task: a member's delivered message came after its last reply.
pub async fn turn_in_progress(
    tx: &mut TenantTx,
    conversation_id: Uuid,
    task_id: Uuid,
) -> Result<bool, DbError> {
    let mut busy = false;
    for message in task_messages(tx, conversation_id, task_id).await? {
        if message.author_kind == AUTHOR_AGENT {
            busy = busy && message.outcome.as_deref() == Some(OUTCOME_CONTINUED);
        } else if message.delivery.as_deref() != Some(DELIVERY_QUEUED) {
            busy = true;
        }
    }
    Ok(busy)
}

/// The members' messages queued for the task, oldest first.
pub async fn queued_messages(
    tx: &mut TenantTx,
    conversation_id: Uuid,
    task_id: Uuid,
) -> Result<Vec<ChatMessage>, DbError> {
    Ok(task_messages(tx, conversation_id, task_id)
        .await?
        .into_iter()
        .filter(|message| message.delivery.as_deref() == Some(DELIVERY_QUEUED))
        .collect())
}
