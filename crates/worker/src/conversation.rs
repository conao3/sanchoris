use chrono::{DateTime, Utc};
use sanchoris_config::{Agent, Limits, Session, Start};
use sanchoris_core::channel::{
    ChatInput, OutboxType, enqueue_chat_message, enqueue_outbox, enqueue_session_mode, resume_from,
    session_records,
};
use sanchoris_core::conversations::{
    DELIVERY_QUEUED, DELIVERY_STEER, compact_request, compact_text, latest_task, load_conversation,
    post_message, queued_messages, turn_in_progress,
};
use sanchoris_core::db::{self, DbError, NewTask, Recorded, Store, TenantTx, tenant_plan};
use sanchoris_core::item::Entity;
use sanchoris_core::items::{
    AUTHOR_AGENT, AUTHOR_MEMBER, AUTHOR_SANCHORIS, CONVERSATION_WEB, ChatAttachment, ChatMessage,
    Config, Conversation, Member, SessionDef, Task, TaskOrigin, User,
};
use sanchoris_core::keys;
use sanchoris_core::lifecycle::TaskDecision;
use sanchoris_core::messages::{self, ChatStartMessage};
use sanchoris_core::model::{NamedDefinition, TaskDefinition, TaskStatus, WaitReason};
use sanchoris_core::modes::SessionModes;
use sanchoris_core::tasks::runs_of;
use sanchoris_core::tasks::{
    apply_decision, deliver_held_messages, latest_run, load_task, send_chat_to_task,
};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::dispatcher::{
    checkout_repositories, known_inputs, load_session, load_skill, offered_skills, shorten,
};
use crate::error::WorkerError;

const MAX_INSTRUCTION_BYTES: usize = 96 * 1024;
const NOTHING_TO_COMPACT: &str = "the session has not started yet; there is nothing to compact";

fn session_of(task: &Task) -> Result<Session> {
    Ok(serde_json::from_value(
        task.definition["Session"]["Definition"].clone(),
    )?)
}

#[derive(Debug, Error)]
pub enum ChatError {
    #[error("{0}")]
    Rejected(String),
    #[error(transparent)]
    Failed(#[from] WorkerError),
}

impl From<DbError> for ChatError {
    fn from(error: DbError) -> Self {
        Self::Failed(WorkerError::Db(error))
    }
}

impl From<serde_json::Error> for ChatError {
    fn from(error: serde_json::Error) -> Self {
        Self::Failed(WorkerError::Json(error))
    }
}

type Result<T> = std::result::Result<T, ChatError>;

fn rejected(message: impl Into<String>) -> ChatError {
    ChatError::Rejected(message.into())
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewConversation<'a> {
    pub kind: &'a str,
    pub config_id: Uuid,
    pub session_name: &'a str,
    pub slack_thread_ts: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemberMessage<'a> {
    pub id: Uuid,
    pub member_id: Uuid,
    pub member_name: &'a str,
    pub text: &'a str,
    pub modes: SessionModes,
    pub delivery: Delivery,
    pub attachments: &'a [ChatAttachment],
    pub at: DateTime<Utc>,
}

/// What happens to a message sent while the agent works on a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Hold it until the turn ends.
    Queue,
    /// Put it into the turn.
    Steer,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Delivered {
    pub message: ChatMessage,
    pub task_id: Uuid,
}

async fn active_config(tx: &mut TenantTx, config_id: Uuid) -> Result<(Config, Uuid)> {
    let config = tx
        .get::<Config>(&keys::config_key(tx.tenant_id(), config_id))
        .await?
        .filter(Config::is_active)
        .ok_or_else(|| rejected(format!("config {config_id} is not active")))?;
    let revision_id = config.active_revision_id.ok_or_else(|| {
        rejected(format!(
            "config {} has no synced revision",
            config.repository
        ))
    })?;
    Ok((config, revision_id))
}

pub async fn start_conversation(
    tx: &mut TenantTx,
    new: &NewConversation<'_>,
    message: &MemberMessage<'_>,
) -> Result<Delivered> {
    let (config, revision_id) = active_config(tx, new.config_id).await?;
    let session_key = keys::session_key(tx.tenant_id(), config.id, revision_id, new.session_name);
    if tx.get::<SessionDef>(&session_key).await?.is_none() {
        return Err(rejected(format!(
            "session {} does not exist in config {}",
            new.session_name, config.repository
        )));
    }
    let conversation = Conversation {
        id: Uuid::new_v4(),
        tenant_id: tx.tenant_id(),
        kind: new.kind.to_string(),
        config_id: config.id,
        session_name: new.session_name.to_string(),
        slack_thread_ts: new.slack_thread_ts.map(str::to_string),
        created_by_member_id: message.member_id,
        title: shorten(message.text),
        tasks: 0,
        modes: Some(message.modes),
        resume: None,
        context: None,
        created_at: message.at,
        updated_at: message.at,
    };
    tx.insert(&conversation)?;
    post_member_message(tx, conversation.id, message).await
}

pub async fn post_member_message(
    tx: &mut TenantTx,
    conversation_id: Uuid,
    message: &MemberMessage<'_>,
) -> Result<Delivered> {
    if compact_request(message.text).is_some() && !message.attachments.is_empty() {
        return Err(rejected("/compact takes no attachments"));
    }
    let mut conversation = load_conversation(tx, conversation_id).await?;
    if conversation.modes != Some(message.modes) {
        conversation.modes = Some(message.modes);
        tx.put(&conversation)?;
    }
    let item = ChatMessage {
        id: message.id,
        tenant_id: tx.tenant_id(),
        conversation_id,
        task_id: None,
        run_id: None,
        turn: None,
        outcome: None,
        delivery: None,
        attachments: message.attachments.to_vec(),
        author_kind: AUTHOR_MEMBER.to_string(),
        member_id: Some(message.member_id),
        text: message.text.to_string(),
        created_at: message.at,
    };
    let working = match latest_task(tx, &conversation).await? {
        Some(task) if task.status == TaskStatus::Running => {
            turn_in_progress(tx, conversation_id, task.id)
                .await?
                .then_some(task.id)
        }
        _ => None,
    };
    let (task_id, delivery) = match (working, message.delivery) {
        (Some(task_id), Delivery::Queue) => (task_id, Some(DELIVERY_QUEUED)),
        (Some(_), Delivery::Steer) => (
            deliver(tx, &conversation, message, true).await?,
            Some(DELIVERY_STEER),
        ),
        (None, _) => (deliver(tx, &conversation, message, false).await?, None),
    };
    let item = ChatMessage {
        task_id: Some(task_id),
        delivery: delivery.map(str::to_string),
        ..item
    };
    post_message(tx, &item).await?;
    Ok(Delivered {
        message: item,
        task_id,
    })
}

fn chat_input<'a>(
    text: &'a str,
    steer: bool,
    compact: bool,
    attachments: &'a [ChatAttachment],
) -> ChatInput<'a> {
    ChatInput {
        text,
        steer,
        compact,
        attachments,
    }
}

async fn deliver(
    tx: &mut TenantTx,
    conversation: &Conversation,
    message: &MemberMessage<'_>,
    steer: bool,
) -> Result<Uuid> {
    let compact = compact_request(message.text);
    if compact.is_some() && steer {
        return Err(rejected(
            "compaction waits for the turn to end; send /compact without steering",
        ));
    }
    let Some(mut task) = latest_task(tx, conversation)
        .await?
        .filter(|task| !task.status.is_terminal())
    else {
        if compact.is_some() {
            return Err(rejected(NOTHING_TO_COMPACT));
        }
        return start_task(tx, conversation, message).await;
    };
    let text = match compact {
        Some(instructions) => {
            let session = session_of(&task)?;
            if !instructions.is_empty() && session.worker.agent == Agent::Codex {
                return Err(rejected(
                    "Codex compacts with the session's CompactInstructions only; send /compact without instructions",
                ));
            }
            compact_text(session.compact_instructions.as_deref(), instructions)
        }
        None => messages::reply_message(message.member_name, message.text),
    };
    let compact = compact.is_some();
    let dedupe = format!("chat:{}", message.id);
    task.input_at = Some(message.at);
    if task.status == TaskStatus::Pending && task.runs == 0 {
        if compact {
            return Err(rejected(NOTHING_TO_COMPACT));
        }
        let payload = task.start_payload.as_mut().ok_or_else(|| {
            WorkerError::Invalid(format!("the pending task {} has no start payload", task.id))
        })?;
        let instruction = payload["instruction"].as_str().unwrap_or_default();
        if instruction.len() + text.len() > MAX_INSTRUCTION_BYTES {
            return Err(rejected(
                "the session has not started yet and holds as much as it can; wait for its first reply",
            ));
        }
        payload["instruction"] = json!(format!("{instruction}\n\n{text}"));
        let mut attachments: Vec<ChatAttachment> =
            serde_json::from_value(payload["attachments"].clone())?;
        attachments.extend_from_slice(message.attachments);
        payload["attachments"] = serde_json::to_value(attachments)?;
        payload["modes"] = serde_json::to_value(message.modes)?;
        task.updated_at = message.at;
        tx.put(&task)?;
        return Ok(task.id);
    }
    let run_id = latest_run(tx, task.id).await?.ok_or_else(|| {
        WorkerError::Invalid(format!("the {} task {} has no run", task.status, task.id))
    })?;
    let payload = task.start_payload.as_mut().ok_or_else(|| {
        WorkerError::Invalid(format!("the task {} has no start payload", task.id))
    })?;
    payload["modes"] = serde_json::to_value(message.modes)?;
    tx.put(&task)?;
    enqueue_session_mode(tx, run_id, message.modes, &format!("mode:{}", message.id)).await?;
    if task.status == TaskStatus::NeedsYou {
        deliver_held_messages(tx, task.id, run_id).await?;
        enqueue_chat_message(
            tx,
            run_id,
            chat_input(&text, false, compact, message.attachments),
            &dedupe,
        )
        .await?;
        let decision = TaskDecision::Resume {
            message: text,
            reset_nudges: true,
        };
        apply_decision(tx, task.id, run_id, &decision, &dedupe, message.at).await?;
    } else {
        send_chat_to_task(
            tx,
            task.id,
            run_id,
            chat_input(&text, steer, compact, message.attachments),
            &dedupe,
            message.at,
        )
        .await?;
    }
    Ok(task.id)
}

/// Delivers a queued message now: when its turn comes, or into the running turn with `steer`. It moves to the end of the conversation.
pub async fn deliver_queued(
    tx: &mut TenantTx,
    conversation_id: Uuid,
    queued: &ChatMessage,
    member_name: &str,
    steer: bool,
    at: DateTime<Utc>,
) -> Result<ChatMessage> {
    let conversation = load_conversation(tx, conversation_id).await?;
    let member_id = queued.member_id.ok_or_else(|| {
        WorkerError::Invalid(format!("the queued message {} has no member", queued.id))
    })?;
    let message = MemberMessage {
        id: queued.id,
        member_id,
        member_name,
        text: &queued.text,
        modes: conversation.modes.unwrap_or(SessionModes::DEFAULT),
        delivery: Delivery::Queue,
        attachments: &queued.attachments,
        at,
    };
    let task_id = deliver(tx, &conversation, &message, steer).await?;
    let item = ChatMessage {
        task_id: Some(task_id),
        delivery: steer.then(|| DELIVERY_STEER.to_string()),
        created_at: at.max(queued.created_at),
        ..queued.clone()
    };
    if item.key() == queued.key() {
        tx.put(&item)?;
    } else {
        tx.delete(&queued.key())?;
        post_message(tx, &item).await?;
    }
    Ok(item)
}

async fn start_task(
    tx: &mut TenantTx,
    conversation: &Conversation,
    message: &MemberMessage<'_>,
) -> Result<Uuid> {
    let (config, revision_id) = active_config(tx, conversation.config_id).await?;
    let session_name = conversation.session_name.as_str();
    let session = tx
        .get::<SessionDef>(&keys::session_key(
            tx.tenant_id(),
            config.id,
            revision_id,
            session_name,
        ))
        .await?
        .ok_or_else(|| {
            rejected(format!(
                "session {session_name} no longer exists in config {}",
                config.repository
            ))
        })?;
    let (session_definition, system_prompt) =
        load_session(tx, config.id, revision_id, session_name)
            .await?
            .ok_or_else(|| rejected(format!("session {session_name} does not exist")))?;
    let repositories =
        checkout_repositories(tx, config.id, revision_id, &session_definition).await?;
    let mut skills = Vec::new();
    let mut loaded = Vec::new();
    for name in &session.chat_skills {
        let skill = load_skill(tx, config.id, revision_id, name)
            .await?
            .ok_or_else(|| WorkerError::Invalid(format!("skill {name} does not exist")))?;
        skills.push(NamedDefinition {
            name: name.clone(),
            definition: serde_json::to_value(&skill)?,
        });
        loaded.push(skill);
    }
    let (_, plan) = tenant_plan(tx).await?;
    let limits: Limits = plan.chat_limits;
    let definition = TaskDefinition {
        trigger: None,
        start: Start {
            session: session_name.to_string(),
            instruction: message.text.to_string(),
            skills: session.chat_skills.clone(),
            required_skills: Vec::new(),
            max_nudges: 0,
            limits,
        },
        session: NamedDefinition {
            name: session_name.to_string(),
            definition: session_definition,
        },
        skills,
    };
    let sequence = conversation.tasks + 1;
    let name = sequence.to_string();
    let title = shorten(message.text);
    let recorded = db::create_task(
        tx,
        &NewTask {
            origin: TaskOrigin::Conversation {
                conversation_id: conversation.id,
                sequence,
            },
            config_revision_id: revision_id,
            name: &name,
            title: &title,
            definition: &definition,
            parent_task_id: None,
            requested_by_member_id: Some(conversation.created_by_member_id),
            requested_by_github_user_id: None,
            requested_by_github_login: None,
            blocked_by: &[],
        },
    )
    .await?;
    let Recorded::New(task_id) = recorded else {
        return Err(ChatError::Failed(WorkerError::Db(DbError::Conflict)));
    };
    let offered = offered_skills(&loaded, &[], &known_inputs(&repositories, &json!({})));
    let instruction = messages::chat_start_message(&ChatStartMessage {
        session_name,
        member: message.member_name,
        skills: &offered,
        text: message.text,
    });
    let mut task = load_task(tx, task_id).await?;
    task.start_payload = Some(json!({
        "instruction": instruction,
        "systemPrompt": system_prompt,
        "repositories": repositories,
        "modes": message.modes,
        "attachments": message.attachments,
        "resume": if conversation.tasks == 0 {
            conversation.resume.clone().unwrap_or(Value::Null)
        } else {
            Value::Null
        },
    }));
    tx.put(&task)?;
    let mut conversation = load_conversation(tx, conversation.id).await?;
    conversation.tasks = sequence;
    conversation.updated_at = conversation.updated_at.max(message.at);
    tx.put(&conversation)?;
    Ok(task_id)
}

/// Delivers the oldest message queued for the task, if any, when its turn ends.
pub async fn deliver_next_queued(
    store: &Store,
    tx: &mut TenantTx,
    task: &Task,
    at: DateTime<Utc>,
) -> Result<()> {
    let Some(conversation_id) = task.origin.conversation_id() else {
        return Ok(());
    };
    let Some(next) = queued_messages(tx, conversation_id, task.id)
        .await?
        .into_iter()
        .next()
    else {
        return Ok(());
    };
    let member_id = next.member_id.ok_or_else(|| {
        WorkerError::Invalid(format!("the queued message {} has no member", next.id))
    })?;
    let member = tx
        .require::<Member>(&keys::member_key(tx.tenant_id(), member_id))
        .await?;
    let user = store
        .get::<User>(&keys::user_key(member.user_id))
        .await?
        .ok_or_else(|| WorkerError::Invalid(format!("user {} is missing", member.user_id)))?;
    deliver_queued(tx, conversation_id, &next, &user.display_name, false, at).await?;
    Ok(())
}

/// The member's message a rewind took back, for the composer.
#[derive(Debug, Clone, PartialEq)]
pub struct Rewound {
    pub text: String,
    pub attachments: Vec<ChatAttachment>,
}

async fn conversation_messages(
    tx: &mut TenantTx,
    conversation_id: Uuid,
) -> Result<Vec<ChatMessage>> {
    let mut messages = tx
        .query::<ChatMessage>(
            &keys::conversation_pk(tx.tenant_id(), conversation_id),
            "MSG#",
        )
        .await?;
    messages.sort_by_key(|message| (message.created_at, message.id));
    Ok(messages)
}

/// Rewinds the conversation's task to before a member's message: the session resumes from the record of the turn before it, the message and everything after it leave the conversation, and with `restore_files` a running Workspace puts its repositories back as they were when that message's turn started.
pub async fn rewind(
    tx: &mut TenantTx,
    conversation_id: Uuid,
    message_id: Uuid,
    restore_files: bool,
) -> Result<Rewound> {
    let conversation = load_conversation(tx, conversation_id).await?;
    let task = latest_task(tx, &conversation)
        .await?
        .filter(|task| !task.status.is_terminal())
        .ok_or_else(|| rejected("the conversation's task has finished; send a new message"))?;
    if task.status == TaskStatus::Running && turn_in_progress(tx, conversation_id, task.id).await? {
        return Err(rejected(
            "Interrupt the current turn before reverting checkpoints.",
        ));
    }
    let messages: Vec<ChatMessage> = conversation_messages(tx, conversation_id)
        .await?
        .into_iter()
        .filter(|message| message.task_id == Some(task.id))
        .collect();
    let position = messages
        .iter()
        .position(|message| {
            message.id == message_id
                && message.author_kind == AUTHOR_MEMBER
                && message.delivery.as_deref() != Some(DELIVERY_QUEUED)
        })
        .ok_or_else(|| {
            rejected(format!(
                "message {message_id} is not a sent message of the task"
            ))
        })?;
    let since = messages[..position]
        .iter()
        .rposition(|message| message.author_kind == AUTHOR_AGENT && message.turn.is_some());
    let earlier = &messages[since.map_or(0, |index| index + 1)..position];
    if earlier.iter().any(|message| {
        message.author_kind == AUTHOR_MEMBER && message.delivery.as_deref() != Some(DELIVERY_QUEUED)
    }) {
        return Err(rejected(
            "the message went into a turn another message started; rewind from that message",
        ));
    }
    let previous = since.map(|index| &messages[index]);
    let mut run = runs_of(tx, task.id)
        .await?
        .into_iter()
        .last()
        .ok_or_else(|| rejected("the task has no session yet"))?;
    if !matches!(run.status.as_str(), "running" | "idle") {
        return Err(rejected("the session is starting; try again when it runs"));
    }
    let (record, restore_turn) = match previous {
        Some(reply) => {
            if reply.run_id != Some(run.id) {
                return Err(rejected(
                    "the message was answered in an earlier session; rewind to a later message",
                ));
            }
            let turn = reply.turn.expect("the reply has a turn");
            let record = session_records(tx, run.id)
                .await?
                .into_iter()
                .rfind(|event| event.payload["turn"].as_i64() == Some(turn))
                .ok_or_else(|| rejected("the agent saved no record after that turn"))?;
            (Some(record), turn + 1)
        }
        None => (None, 1),
    };
    run.record_s3_key = record
        .as_ref()
        .and_then(|event| event.payload["key"].as_str())
        .map(str::to_string);
    run.agent_session_id = record
        .as_ref()
        .and_then(|event| event.payload["sessionId"].as_str())
        .map(str::to_string);
    tx.put(&run)?;
    if run.status == "running" {
        enqueue_outbox(
            tx,
            run.id,
            OutboxType::Rewind,
            &json!({
                "resume": record.as_ref().map_or(Value::Null, |event| resume_from(event, run.turns)),
                "restoreTurn": restore_files.then_some(restore_turn),
            }),
            &format!("rewind:{message_id}"),
        )
        .await?;
    }
    for message in &messages[position..] {
        tx.delete(&message.key())?;
    }
    let mut conversation = load_conversation(tx, conversation_id).await?;
    conversation.context = None;
    tx.put(&conversation)?;
    let taken = &messages[position];
    Ok(Rewound {
        text: taken.text.clone(),
        attachments: taken.attachments.clone(),
    })
}

/// Starts a new conversation with the messages up to an agent's reply; its first task resumes the session from the record of that reply's turn.
pub async fn fork(
    tx: &mut TenantTx,
    conversation_id: Uuid,
    message_id: Uuid,
    member_id: Uuid,
    at: DateTime<Utc>,
) -> Result<Conversation> {
    let source = load_conversation(tx, conversation_id).await?;
    let messages = conversation_messages(tx, conversation_id).await?;
    let position = messages
        .iter()
        .position(|message| message.id == message_id && message.author_kind == AUTHOR_AGENT)
        .ok_or_else(|| rejected(format!("message {message_id} is not a reply of the agent")))?;
    let reply = &messages[position];
    let (Some(run_id), Some(turn)) = (reply.run_id, reply.turn) else {
        return Err(rejected("the reply does not end a turn of the agent"));
    };
    let record = session_records(tx, run_id)
        .await?
        .into_iter()
        .rfind(|event| event.payload["turn"].as_i64() == Some(turn))
        .ok_or_else(|| rejected("the agent saved no record after that turn"))?;
    let conversation = Conversation {
        id: Uuid::new_v4(),
        tenant_id: tx.tenant_id(),
        kind: CONVERSATION_WEB.to_string(),
        config_id: source.config_id,
        session_name: source.session_name.clone(),
        slack_thread_ts: None,
        created_by_member_id: member_id,
        title: format!("{} fork", shorten(&source.title)),
        tasks: 0,
        modes: source.modes,
        resume: Some(resume_from(&record, 0)),
        context: None,
        created_at: at,
        updated_at: at,
    };
    tx.insert(&conversation)?;
    for message in &messages[..=position] {
        tx.insert(&ChatMessage {
            id: Uuid::new_v4(),
            conversation_id: conversation.id,
            task_id: None,
            delivery: None,
            ..message.clone()
        })?;
    }
    Ok(conversation)
}

/// Compacts the session of a conversation that stayed idle after the reply of `turn`: sends `/compact` to the run as sanchoris. Does nothing when anything happened since, when the task no longer waits for a message, or when that turn was itself a compaction.
pub async fn auto_compact(
    tx: &mut TenantTx,
    task_id: Uuid,
    run_id: Uuid,
    turn: i64,
    at: DateTime<Utc>,
) -> Result<bool> {
    let task = load_task(tx, task_id).await?;
    let Some(conversation_id) = task.origin.conversation_id() else {
        return Ok(false);
    };
    if task.status != TaskStatus::Waiting
        || task.wait_reason != Some(WaitReason::Message)
        || latest_run(tx, task.id).await? != Some(run_id)
    {
        return Ok(false);
    }
    let messages = conversation_messages(tx, conversation_id).await?;
    let reply = Uuid::new_v5(&run_id, format!("turn:{turn}").as_bytes());
    let Some(replied_at) = messages
        .iter()
        .find(|message| message.id == reply)
        .map(|message| message.created_at)
    else {
        return Ok(false);
    };
    if messages
        .iter()
        .any(|message| message.created_at > replied_at)
    {
        return Ok(false);
    }
    let compacted = messages
        .iter()
        .rev()
        .find(|message| message.author_kind != AUTHOR_AGENT)
        .is_some_and(|message| compact_request(&message.text).is_some());
    if compacted {
        return Ok(false);
    }
    let id = Uuid::new_v5(&reply, b"auto-compact");
    let text = compact_text(session_of(&task)?.compact_instructions.as_deref(), "");
    send_chat_to_task(
        tx,
        task.id,
        run_id,
        chat_input(&text, false, true, &[]),
        &format!("chat:{id}"),
        at,
    )
    .await?;
    post_message(
        tx,
        &ChatMessage {
            id,
            tenant_id: tx.tenant_id(),
            conversation_id,
            task_id: Some(task.id),
            run_id: None,
            turn: None,
            outcome: None,
            delivery: None,
            attachments: Vec::new(),
            author_kind: AUTHOR_SANCHORIS.to_string(),
            member_id: None,
            text,
            created_at: at,
        },
    )
    .await?;
    Ok(true)
}
