use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use async_graphql::{Context, ID, Object, Result, SimpleObject};
use chrono::{DateTime, Utc};
use sanchoris_core::conversations::{DELIVERY_QUEUED, OUTCOME_CONTINUED, latest_task};
use sanchoris_core::items::{
    AUTHOR_AGENT, AUTHOR_MEMBER, AUTHOR_SANCHORIS, ChannelInstance, ChatMessage as ChatMessageItem,
    Conversation as ConversationItem, ConversationTask, Output as OutputItem, RunPartial,
    Workspace,
};
use sanchoris_core::keys;
use sanchoris_core::model::TaskStatus;
use sanchoris_core::modes::SessionModes;
use sanchoris_core::store::{Index, Listing};
use sanchoris_core::tasks::runs_of;
use uuid::Uuid;

use crate::account::Member;
use crate::attachment::ChatAttachment;
use crate::chat_work::{
    ChatDiff, ChatPartial, ChatStartup, ChatWork, ChatWorkKind, ChatWorking, RunWork,
};
use crate::config::Session;
use crate::context::{Scope, tx};
use crate::enums::{
    ChatAuthorKind, ChatInteractionMode, ChatMessageDelivery, ChatRuntimeMode, ChatTurnOutcome,
    ConversationKind,
};
use crate::error::not_found;
use crate::scalars::{Timestamp, ts};
use crate::skill::SkillRun;
use crate::task::{IssueRef, Task, filed_tasks_of, issue_outputs};
use crate::tenant::tenant_repositories;

pub(crate) const CONVERSATION_LISTING_LIMIT: usize = 200;

#[derive(SimpleObject)]
pub(crate) struct ChatContext {
    pub used_tokens: i64,
    pub max_tokens: i64,
}

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

    /// The mode of the last message sent to the conversation; DEFAULT before any.
    async fn interaction_mode(&self) -> ChatInteractionMode {
        self.row
            .modes
            .unwrap_or(SessionModes::DEFAULT)
            .interaction_mode
            .into()
    }

    /// The runtime mode of the last message sent to the conversation; FULL_ACCESS before any.
    async fn runtime_mode(&self) -> ChatRuntimeMode {
        self.row
            .modes
            .unwrap_or(SessionModes::DEFAULT)
            .runtime_mode
            .into()
    }

    /// How full the session's context was at the end of the last turn; null before the agent reported it and after a rewind.
    async fn context(&self) -> Option<ChatContext> {
        self.row.context.map(|context| ChatContext {
            used_tokens: context.used_tokens,
            max_tokens: context.max_tokens,
        })
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
        let mut runs: BTreeMap<Uuid, RunWork> = BTreeMap::new();
        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            let (work, diff, proposed_plan) = match (row.author_kind.as_str(), row.run_id, row.turn)
            {
                (AUTHOR_AGENT, Some(run_id), Some(turn)) => {
                    if let Entry::Vacant(entry) = runs.entry(run_id) {
                        entry.insert(
                            RunWork::load(&mut tx(ctx, self.scope.tenant_id), run_id).await?,
                        );
                    }
                    let run = &runs[&run_id];
                    (
                        run.turn(turn, Some(&row.text))?,
                        run.diff(turn)?,
                        run.proposed_plan(turn),
                    )
                }
                _ => (Vec::new(), None, None),
            };
            messages.push(ChatMessage {
                scope: self.scope,
                row,
                issues: Vec::new(),
                work,
                diff,
                proposed_plan,
                skill_runs: Vec::new(),
            });
        }
        for task_id in self.task_ids(ctx).await? {
            for output in issue_outputs(ctx, self.scope, task_id).await? {
                if let Some(message) = reply_after(&mut messages, task_id, output.created_at) {
                    message.issues.push(output);
                }
            }
            for skill_run in SkillRun::of_task(ctx, self.scope, task_id).await? {
                if let Some(message) = reply_after(&mut messages, task_id, skill_run.row.started_at)
                {
                    message.skill_runs.push(skill_run);
                }
            }
        }
        Ok(messages)
    }

    /// The turn the agent is working on, while the latest task runs one or waits on a question.
    async fn working(&self, ctx: &Context<'_>) -> Result<Option<ChatWorking>> {
        let mut tx = tx(ctx, self.scope.tenant_id);
        let Some(task) = latest_task(&mut tx, &self.row).await? else {
            return Ok(None);
        };
        if task.status != TaskStatus::Running && task.status != TaskStatus::Pending {
            return Ok(None);
        }
        let mut last_member_at = None;
        for message in tx
            .query::<ChatMessageItem>(&self.pk(), "MSG#")
            .await?
            .into_iter()
            .filter(|message| {
                message.task_id == Some(task.id)
                    && message.delivery.as_deref() != Some(DELIVERY_QUEUED)
            })
        {
            if message.author_kind != AUTHOR_AGENT {
                last_member_at = last_member_at.max(Some(message.created_at));
            } else if message.outcome.as_deref() != Some(OUTCOME_CONTINUED) {
                last_member_at = last_member_at.filter(|at| *at > message.created_at);
            }
        }
        let skill_runs = SkillRun::of_task(ctx, self.scope, task.id).await?;
        let skill_runs_since = |at: DateTime<Utc>| {
            skill_runs
                .iter()
                .filter(|skill_run| skill_run.row.started_at >= at)
                .cloned()
                .collect::<Vec<_>>()
        };
        let run = runs_of(&mut tx, task.id)
            .await?
            .into_iter()
            .rev()
            .find(|run| !run.is_finished());
        let Some(run) = run else {
            return Ok(last_member_at.map(|at| ChatWorking {
                started_at: ts(at),
                work: Vec::new(),
                question: None,
                approval: None,
                plan: None,
                partial: None,
                startup: None,
                skill_runs: skill_runs_since(at),
            }));
        };
        let startup = if run.status == "creating" {
            let workspace_started_at = tx
                .query::<Workspace>(&keys::task_pk(self.scope.tenant_id, task.id), "WS#")
                .await?
                .into_iter()
                .filter(|workspace| workspace.run_id == run.id && workspace.stopped_at.is_none())
                .map(|workspace| workspace.started_at)
                .max();
            let connected_at = match workspace_started_at {
                Some(started_at) => tx
                    .query::<ChannelInstance>(&keys::run_pk(self.scope.tenant_id, run.id), "INST#")
                    .await?
                    .into_iter()
                    .map(|instance| instance.connected_at)
                    .filter(|connected_at| *connected_at >= started_at)
                    .max(),
                None => None,
            };
            Some(ChatStartup {
                workspace_started_at: workspace_started_at.map(ts),
                connected_at: connected_at.map(ts),
            })
        } else {
            None
        };
        let turn = run.turns + 1;
        let work = RunWork::load(&mut tx, run.id).await?;
        let question = work.open_question(turn)?;
        let approval = work.open_approval(turn)?;
        let plan = work.plan(turn)?;
        let partial = tx
            .get::<RunPartial>(&keys::run_partial_key(self.scope.tenant_id, run.id))
            .await?
            .filter(|partial| {
                partial.turn == turn && work.last_at(turn).is_none_or(|last| partial.at > last)
            })
            .map(|partial| ChatPartial {
                kind: if partial.partial_kind == "thinking" {
                    ChatWorkKind::Thinking
                } else {
                    ChatWorkKind::Text
                },
                text: partial.text,
                at: ts(partial.at),
            });
        let entries = work.turn(turn, None)?;
        let Some(started_at) = last_member_at
            .or_else(|| work.first_at(turn))
            .or_else(|| question.as_ref().map(|question| question.asked_at.0))
            .or_else(|| approval.as_ref().map(|approval| approval.asked_at.0))
        else {
            return Ok(None);
        };
        if entries.is_empty()
            && question.is_none()
            && approval.is_none()
            && last_member_at.is_none()
        {
            return Ok(None);
        }
        Ok(Some(ChatWorking {
            started_at: ts(started_at),
            work: entries,
            question,
            approval,
            plan,
            partial,
            startup,
            skill_runs: skill_runs_since(started_at),
        }))
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

fn reply_after(
    messages: &mut [ChatMessage],
    task_id: Uuid,
    at: DateTime<Utc>,
) -> Option<&mut ChatMessage> {
    messages.iter_mut().find(|message| {
        message.row.task_id == Some(task_id)
            && message.row.author_kind == AUTHOR_AGENT
            && message.row.created_at >= at
    })
}

pub(crate) struct ChatMessage {
    pub scope: Scope,
    pub row: ChatMessageItem,
    pub issues: Vec<OutputItem>,
    pub work: Vec<ChatWork>,
    pub diff: Option<ChatDiff>,
    pub proposed_plan: Option<String>,
    pub skill_runs: Vec<SkillRun>,
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
            AUTHOR_SANCHORIS => Ok(ChatAuthorKind::Sanchoris),
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

    /// Images the member attached.
    async fn attachments(&self) -> Vec<ChatAttachment> {
        self.row
            .attachments
            .iter()
            .map(|item| ChatAttachment {
                tenant_id: self.scope.tenant_id,
                item: item.clone(),
            })
            .collect()
    }

    /// For members' messages: QUEUED while it waits for the agent's turn to end, STEERED when it went into a running turn.
    async fn delivery(&self) -> Result<Option<ChatMessageDelivery>> {
        self.row
            .delivery
            .as_deref()
            .map(ChatMessageDelivery::from_db)
            .transpose()
    }

    /// How the turn this reply ends finished. Null for members' messages and posts outside a turn.
    async fn outcome(&self) -> Result<Option<ChatTurnOutcome>> {
        self.row
            .outcome
            .as_deref()
            .map(ChatTurnOutcome::from_db)
            .transpose()
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

    /// The thinking, text and tool calls of the turn that this reply ends, oldest first. Empty for members' messages.
    async fn work(&self) -> &[ChatWork] {
        &self.work
    }

    /// The plan the agent proposed in plan mode during the turn this reply ends, as Markdown.
    async fn proposed_plan(&self) -> Option<&str> {
        self.proposed_plan.as_deref()
    }

    /// The files the turn this reply ends changed. Null when it changed none.
    async fn diff(&self) -> Option<&ChatDiff> {
        self.diff.as_ref()
    }

    /// Issues the agent filed with file-issue during the turn this reply ends.
    async fn issues(&self, ctx: &Context<'_>) -> Result<Vec<IssueRef>> {
        let repositories = tenant_repositories(ctx, self.scope.tenant_id).await?;
        self.issues
            .iter()
            .map(|output| IssueRef::from_output(self.scope, output, &repositories))
            .collect()
    }

    /// The skills the engine ran for the agent during the turn this reply ends, oldest first.
    async fn skill_runs(&self) -> &[SkillRun] {
        &self.skill_runs
    }
}
