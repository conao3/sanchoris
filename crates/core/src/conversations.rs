use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::db::{DbError, TenantTx};
use crate::item::Entity;
use crate::items::{AUTHOR_AGENT, ChatMessage, Conversation, ConversationTask, Task};
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
            author_kind: AUTHOR_AGENT.to_string(),
            member_id: None,
            text: text.to_string(),
            created_at: at,
        },
    )
    .await
}
