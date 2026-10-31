use async_graphql::{Context, ID, Object, Result};
use sanchoris_core::items::{
    AUTHOR_AGENT, AUTHOR_MEMBER, ChatMessage as ChatMessageItem, Conversation as ConversationItem,
    ConversationTask, Output as OutputItem,
};
use sanchoris_core::keys;
use sanchoris_core::store::{Index, Listing};
use uuid::Uuid;

use crate::account::Member;
use crate::config::Session;
use crate::context::{Scope, tx};
use crate::enums::{ChatAuthorKind, ConversationKind};
use crate::error::not_found;
use crate::scalars::{Timestamp, ts};
use crate::task::{IssueRef, Task, filed_tasks_of, issue_outputs};
use crate::tenant::tenant_repositories;

pub(crate) const CONVERSATION_LISTING_LIMIT: usize = 200;

pub(crate) struct Conversation {
    pub scope: Scope,
    pub row: ConversationItem,
}

impl Conversation {
    pub async fn find(ctx: &Context<'_>, scope: Scope, id: Uuid) -> Result<Option<Self>> {
        Ok(tx(ctx, scope.tenant_id)
            .get::<ConversationItem>(&keys::conversation_key(scope.tenant_id, id))
            .await?
            .map(|row| Conversation { scope, row }))
    }

    pub async fn load(ctx: &Context<'_>, scope: Scope, id: Uuid) -> Result<Self> {
        Self::find(ctx, scope, id)
            .await?
            .ok_or_else(|| not_found(format!("conversation {id}")))
    }

    pub async fn latest(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        Ok(tx(ctx, scope.tenant_id)
            .list::<ConversationItem>(
                &Listing::index(Index::Gsi1, keys::conversations_index(scope.tenant_id))
                    .descending()
                    .limit(CONVERSATION_LISTING_LIMIT),
                |_| true,
            )
            .await?
            .into_iter()
            .map(|row| Conversation { scope, row })
            .collect())
    }

    fn pk(&self) -> String {
        keys::conversation_pk(self.scope.tenant_id, self.row.id)
    }

    async fn task_ids(&self, ctx: &Context<'_>) -> Result<Vec<Uuid>> {
        let mut entries = tx(ctx, self.scope.tenant_id)
            .query::<ConversationTask>(&self.pk(), "TASK#")
            .await?;
        entries.sort_by_key(|entry| entry.sequence);
        Ok(entries.into_iter().map(|entry| entry.task_id).collect())
    }
}

#[Object]
impl Conversation {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn kind(&self) -> Result<ConversationKind> {
        ConversationKind::from_db(&self.row.kind)
    }

    /// The first line of the first message.
    async fn title(&self) -> &str {
        &self.row.title
    }

    async fn session_name(&self) -> &str {
        &self.row.session_name
    }

    /// The session in the config's active revision. Null when the config no longer has it.
    async fn session(&self, ctx: &Context<'_>) -> Result<Option<Session>> {
        Session::by_name(ctx, self.scope, self.row.config_id, &self.row.session_name).await
    }

    /// The Slack thread, for a Slack conversation.
    async fn slack_thread_ts(&self) -> Option<&str> {
        self.row.slack_thread_ts.as_deref()
    }

    /// The member who started the conversation; its tasks run with this member as the requester.
    async fn created_by(&self, ctx: &Context<'_>) -> Result<Option<Member>> {
        Member::find(ctx, self.scope, self.row.created_by_member_id).await
    }

    async fn created_at(&self) -> Timestamp {
        ts(self.row.created_at)
    }

    async fn updated_at(&self) -> Timestamp {
        ts(self.row.updated_at)
    }

    /// Members' messages and the agent's replies, oldest first.
    async fn messages(&self, ctx: &Context<'_>) -> Result<Vec<ChatMessage>> {
        let mut rows = tx(ctx, self.scope.tenant_id)
            .query::<ChatMessageItem>(&self.pk(), "MSG#")
            .await?;
        rows.sort_by_key(|row| (row.created_at, row.id));
        let mut messages = rows
            .into_iter()
            .map(|row| ChatMessage {
                scope: self.scope,
                row,
                issues: Vec::new(),
            })
            .collect::<Vec<_>>();
        for task_id in self.task_ids(ctx).await? {
            for output in issue_outputs(ctx, self.scope, task_id).await? {
                if let Some(message) = messages.iter_mut().find(|message| {
                    message.row.task_id == Some(task_id)
                        && message.row.author_kind == AUTHOR_AGENT
                        && message.row.created_at >= output.created_at
                }) {
                    message.issues.push(output);
                }
            }
        }
        Ok(messages)
    }

    /// The conversation's tasks, one per session, in order.
    async fn tasks(&self, ctx: &Context<'_>) -> Result<Vec<Task>> {
        let mut tasks = Vec::new();
        for task_id in self.task_ids(ctx).await? {
            tasks.push(Task::load(ctx, self.scope, task_id).await?);
        }
        Ok(tasks)
    }

    /// The task the next message goes to, unless it has finished.
    async fn latest_task(&self, ctx: &Context<'_>) -> Result<Option<Task>> {
        match self.task_ids(ctx).await?.last() {
            Some(task_id) => Task::find(ctx, self.scope, *task_id).await,
            None => Ok(None),
        }
    }

    /// Tasks started from issues that the conversation's tasks filed with file-issue, with their descendants.
    async fn filed_tasks(&self, ctx: &Context<'_>) -> Result<Vec<Task>> {
        filed_tasks_of(ctx, self.scope, &self.task_ids(ctx).await?).await
    }
}

pub(crate) struct ChatMessage {
    pub scope: Scope,
    pub row: ChatMessageItem,
    pub issues: Vec<OutputItem>,
}

#[Object]
impl ChatMessage {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn author_kind(&self) -> Result<ChatAuthorKind> {
        match self.row.author_kind.as_str() {
            AUTHOR_AGENT => Ok(ChatAuthorKind::Agent),
            AUTHOR_MEMBER => Ok(ChatAuthorKind::Member),
            other => Err(async_graphql::Error::new(format!(
                "chat message {} has the unknown author kind {other}",
                self.row.id
            ))),
        }
    }

    async fn member(&self, ctx: &Context<'_>) -> Result<Option<Member>> {
        match self.row.member_id {
            Some(member_id) => Member::find(ctx, self.scope, member_id).await,
            None => Ok(None),
        }
    }

    async fn text(&self) -> &str {
        &self.row.text
    }

    async fn at(&self) -> Timestamp {
        ts(self.row.created_at)
    }

    /// The task the message went to or came from.
    async fn task(&self, ctx: &Context<'_>) -> Result<Option<Task>> {
        match self.row.task_id {
            Some(task_id) => Task::find(ctx, self.scope, task_id).await,
            None => Ok(None),
        }
    }

    /// Issues the agent filed with file-issue during the turn this reply ends.
    async fn issues(&self, ctx: &Context<'_>) -> Result<Vec<IssueRef>> {
        let repositories = tenant_repositories(ctx, self.scope.tenant_id).await?;
        self.issues
            .iter()
            .map(|output| IssueRef::from_output(self.scope, output, &repositories))
            .collect()
    }
}
