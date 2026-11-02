use sanchoris_core::channel::RunEventType;
use sanchoris_core::conversations::load_conversation;
use sanchoris_core::db::tenant_plan;
use sanchoris_core::items::{
    AUTHOR_AGENT, AUTHOR_SANCHORIS, ChatMessage, ContextUsage, Conversation, ConversationTask,
    FiledIssue, HeldMessage, Output, Run, Task, Tenant,
};
use sanchoris_core::jobs::Job;
use sanchoris_core::keys;
use sanchoris_core::model::TaskStatus;
use sanchoris_core::modes::SessionModes;
use sanchoris_core::store::retry;
use sanchoris_worker::conversation::{
    ChatError, Delivered, Delivery, MemberMessage, NewConversation, fork, post_member_message,
    rewind, start_conversation,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::common::{self, APP_LOGIN, REPO, World, turn_end_payload};

async fn start(world: &World, session: &str, text: &str) -> Delivered {
    let config_id = world.seeded.config_id;
    let member_id = world.seeded.member_id;
    let id = Uuid::new_v4();
    let at = world.clock_now();
    retry(20, || async {
        let mut tx = world.store.tx(world.tenant());
        let delivered = start_conversation(
            &mut tx,
            &NewConversation {
                kind: "web",
                config_id,
                session_name: session,
                slack_thread_ts: None,
            },
            &MemberMessage {
                id,
                member_id,
                member_name: "Mika",
                text,
                modes: SessionModes::DEFAULT,
                delivery: Delivery::Queue,
                attachments: &[],
                at,
            },
        )
        .await
        .map_err(db_error)?;
        tx.commit().await?;
        Ok(delivered)
    })
    .await
    .unwrap()
}

async fn post(world: &World, conversation_id: Uuid, text: &str) -> Delivered {
    post_as(world, conversation_id, text, Delivery::Queue).await
}

async fn post_as(
    world: &World,
    conversation_id: Uuid,
    text: &str,
    delivery: Delivery,
) -> Delivered {
    let member_id = world.seeded.member_id;
    let id = Uuid::new_v4();
    let at = world.clock_now();
    retry(20, || async {
        let mut tx = world.store.tx(world.tenant());
        let delivered = post_member_message(
            &mut tx,
            conversation_id,
            &MemberMessage {
                id,
                member_id,
                member_name: "Mika",
                text,
                modes: SessionModes::DEFAULT,
                delivery,
                attachments: &[],
                at,
            },
        )
        .await
        .map_err(db_error)?;
        tx.commit().await?;
        Ok(delivered)
    })
    .await
    .unwrap()
}

fn db_error(error: ChatError) -> sanchoris_core::db::DbError {
    match error {
        ChatError::Failed(sanchoris_worker::error::WorkerError::Db(error)) => error,
        other => panic!("the chat failed: {other}"),
    }
}

async fn conversation(world: &World, id: Uuid) -> Conversation {
    let mut tx = world.store.tx(world.tenant());
    load_conversation(&mut tx, id).await.unwrap()
}

async fn stored_task(world: &World, id: Uuid) -> Task {
    world
        .store
        .get::<Task>(&keys::task_key(world.tenant(), id))
        .await
        .unwrap()
        .unwrap()
}

async fn messages(world: &World, conversation_id: Uuid) -> Vec<ChatMessage> {
    let mut rows = world
        .store
        .query::<ChatMessage>(
            &keys::conversation_pk(world.tenant(), conversation_id),
            "MSG#",
        )
        .await
        .unwrap();
    rows.sort_by_key(|row| (row.created_at, row.id));
    rows
}

#[tokio::test]
async fn a_new_conversation_starts_its_first_task_in_the_session_with_the_plan_chat_limits() {
    let world = World::new("max").await;
    let delivered = start(&world, "all", "Split the checkout work").await;
    world.worker.settle().await.unwrap();
    let conversation_id = delivered.message.conversation_id;
    let stored = conversation(&world, conversation_id).await;
    assert_eq!(
        (
            stored.session_name.as_str(),
            stored.tasks,
            stored.title.as_str()
        ),
        ("all", 1, "Split the checkout work")
    );
    let entry = world
        .store
        .get::<ConversationTask>(&keys::conversation_task_key(
            world.tenant(),
            conversation_id,
            1,
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entry.task_id, delivered.task_id);
    let task = stored_task(&world, delivered.task_id).await;
    assert_eq!(task.name, "1");
    assert_eq!(task.origin.conversation_id(), Some(conversation_id));
    assert_eq!(task.requested_by_member_id, Some(world.seeded.member_id));
    let definition = task.settings().unwrap();
    let mut tx = world.store.tx(world.tenant());
    let (_, plan) = tenant_plan(&mut tx).await.unwrap();
    assert_eq!(definition.trigger, None);
    assert_eq!(definition.start.limits, plan.chat_limits);
    assert_eq!(definition.start.skills, ["file-issue"]);
    assert_eq!(task.skill_names, ["file-issue"]);
    assert!(task.definition.get("Skills").is_none());
    assert!(definition.start.required_skills.is_empty());
    let view = world.task_of(task.id).await;
    assert_eq!(view.status, "RUNNING");
    let starts = world.outbox(view.run_id.unwrap(), "start").await;
    let instruction = starts[0]["instruction"].as_str().unwrap();
    assert_eq!(
        starts[0]["modes"],
        json!({ "interactionMode": "default", "runtimeMode": "full-access" })
    );
    assert!(
        instruction.starts_with("[sanchoris] Mika started this chat in session all."),
        "{instruction}"
    );
    assert!(instruction.contains("sanchoris skill run file-issue --input"));
    assert!(
        instruction.ends_with("\n\n[sanchoris] Message from Mika:\nSplit the checkout work"),
        "{instruction}"
    );
    assert_eq!(starts[0]["systemPrompt"], json!(null));
    assert_eq!(
        messages(&world, conversation_id)
            .await
            .iter()
            .map(|m| (m.author_kind.as_str(), m.text.as_str(), m.task_id))
            .collect::<Vec<_>>(),
        [("member", "Split the checkout work", Some(task.id))]
    );
}

#[tokio::test]
async fn a_project_session_sends_the_project_system_prompt() {
    let world = World::new("max").await;
    let delivered = start(&world, "shop", "Look at the checkout").await;
    world.worker.settle().await.unwrap();
    let view = world.task_of(delivered.task_id).await;
    let starts = world.outbox(view.run_id.unwrap(), "start").await;
    assert_eq!(
        starts[0]["systemPrompt"],
        "You are working on the Acme shop. Always write tests for new endpoints.\n"
    );
    let definition = stored_task(&world, delivered.task_id)
        .await
        .settings()
        .unwrap();
    assert_eq!(definition.start.skills, ["file-issue", "open-pr-green-ci"]);
}

#[tokio::test]
async fn an_unknown_session_is_rejected() {
    let world = World::new("max").await;
    let mut tx = world.store.tx(world.tenant());
    let error = start_conversation(
        &mut tx,
        &NewConversation {
            kind: "web",
            config_id: world.seeded.config_id,
            session_name: "nope",
            slack_thread_ts: None,
        },
        &MemberMessage {
            id: Uuid::new_v4(),
            member_id: world.seeded.member_id,
            member_name: "Mika",
            text: "hello",
            modes: SessionModes::DEFAULT,
            delivery: Delivery::Queue,
            attachments: &[],
            at: world.clock_now(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "session nope does not exist in config acme/sanchoris-config"
    );
}

#[tokio::test]
async fn messages_go_to_the_running_task_and_a_finished_task_starts_the_next() {
    let world = World::new("max").await;
    let first = start(&world, "all", "Plan the next release").await;
    world.worker.settle().await.unwrap();
    let conversation_id = first.message.conversation_id;
    let run_id = world.task_of(first.task_id).await.run_id.unwrap();
    let second = post(&world, conversation_id, "Also check the changelog").await;
    world.worker.settle().await.unwrap();
    assert_eq!(second.task_id, first.task_id);
    assert_eq!(second.message.delivery.as_deref(), Some("queued"));
    assert!(world.messages(run_id).await.is_empty());
    let steered = post_as(&world, conversation_id, "Skip the docs", Delivery::Steer).await;
    world.worker.settle().await.unwrap();
    assert_eq!(steered.message.delivery.as_deref(), Some("steer"));
    assert_eq!(
        world.outbox(run_id, "user_message").await,
        vec![
            json!({ "text": "[sanchoris] Message from Mika:\nSkip the docs", "steer": true, "compact": false, "attachments": [] })
        ]
    );
    let mut busy = turn_end_payload(1, Value::Null);
    busy["busy"] = json!(true);
    world
        .sequenced_event(run_id, 1, RunEventType::TurnEnd, busy)
        .await;
    world.worker.settle().await.unwrap();
    assert_eq!(
        messages(&world, conversation_id)
            .await
            .into_iter()
            .filter(|message| message.author_kind == AUTHOR_AGENT)
            .map(|message| message.outcome)
            .collect::<Vec<_>>(),
        [Some("continued".to_string())]
    );
    assert_eq!(
        world.messages(run_id).await,
        vec!["[sanchoris] Message from Mika:\nSkip the docs".to_string()]
    );
    world.turn_end(run_id, 2).await;
    world.worker.settle().await.unwrap();
    assert_eq!(
        world.messages(run_id).await,
        vec![
            "[sanchoris] Message from Mika:\nSkip the docs".to_string(),
            "[sanchoris] Message from Mika:\nAlso check the changelog".to_string(),
        ]
    );
    let delivered = messages(&world, conversation_id)
        .await
        .into_iter()
        .find(|message| message.id == second.message.id)
        .unwrap();
    assert_eq!(delivered.delivery, None);
    assert!(delivered.created_at >= second.message.created_at);
    assert_eq!(world.task_of(first.task_id).await.status, "RUNNING");
    assert_eq!(
        world.outbox(run_id, "session_mode").await,
        vec![json!({ "modes": { "interactionMode": "default", "runtimeMode": "full-access" } }); 2]
    );
    world
        .dynamo
        .modify::<Task>(&keys::task_key(world.tenant(), first.task_id), |task| {
            task.status = TaskStatus::Succeeded;
            task.finished_at = Some(chrono::Utc::now());
        })
        .await;
    let third = post(&world, conversation_id, "One more thing").await;
    world.worker.settle().await.unwrap();
    assert_ne!(third.task_id, first.task_id);
    let next = stored_task(&world, third.task_id).await;
    assert_eq!(next.name, "2");
    assert_eq!(next.title, "One more thing");
    assert_eq!(world.task_of(next.id).await.status, "RUNNING");
    assert_eq!(conversation(&world, conversation_id).await.tasks, 2);
}

#[tokio::test]
async fn a_message_resumes_a_task_that_needs_you_with_its_held_messages() {
    let world = World::new("max").await;
    let first = start(&world, "all", "Plan the next release").await;
    world.worker.settle().await.unwrap();
    let task_id = first.task_id;
    world
        .dynamo
        .modify::<Task>(&keys::task_key(world.tenant(), task_id), |task| {
            task.status = TaskStatus::NeedsYou;
            task.error = Some("Task.TurnLimitExceeded".into());
            task.cause = Some("stuck".into());
        })
        .await;
    world
        .dynamo
        .put(&HeldMessage {
            tenant_id: world.tenant(),
            task_id,
            created_at: world.clock_now(),
            dedupe_key: "skill:1:2:succeeded".into(),
            text: "[sanchoris] The skill succeeded.".into(),
        })
        .await;
    post(&world, first.message.conversation_id, "Go on").await;
    world.worker.settle().await.unwrap();
    let resumed = world.task_of(task_id).await;
    assert_eq!(resumed.status, "RUNNING");
    assert_eq!(resumed.error, None);
    assert_eq!(
        world.messages(resumed.run_id.unwrap()).await,
        vec![
            "[sanchoris] The skill succeeded.".to_string(),
            "[sanchoris] Message from Mika:\nGo on".to_string(),
        ]
    );
    assert!(world.dynamo.all::<HeldMessage>().await.is_empty());
}

#[tokio::test]
async fn messages_to_a_pending_task_join_its_start_instruction() {
    let world = World::new("max").await;
    let tenant_key = keys::tenant_key(world.tenant());
    world
        .dynamo
        .modify::<Tenant>(&tenant_key, |tenant| tenant.live_workspaces = 1000)
        .await;
    let first = start(&world, "all", "Plan the next release").await;
    world.worker.settle().await.unwrap();
    post(
        &world,
        first.message.conversation_id,
        "Also check the changelog",
    )
    .await;
    world.worker.settle().await.unwrap();
    assert_eq!(world.task_of(first.task_id).await.status, "PENDING");
    world
        .dynamo
        .modify::<Tenant>(&tenant_key, |tenant| tenant.live_workspaces = 0)
        .await;
    retry(20, || async {
        let mut tx = world.store.tx(world.tenant());
        tx.emit(Job::Dispatch {
            tenant_id: world.tenant(),
        });
        tx.commit().await
    })
    .await
    .unwrap();
    world.worker.settle().await.unwrap();
    let task = world.task_of(first.task_id).await;
    assert_eq!(task.status, "RUNNING");
    let starts = world.outbox(task.run_id.unwrap(), "start").await;
    let instruction = starts[0]["instruction"].as_str().unwrap();
    assert!(
        instruction.ends_with("\n\n[sanchoris] Message from Mika:\nAlso check the changelog"),
        "{instruction}"
    );
}

#[tokio::test]
async fn each_turn_end_posts_the_reply_once_and_the_task_waits_for_the_next_message() {
    let world = World::new("max").await;
    let first = start(&world, "all", "What is failing?").await;
    world.worker.settle().await.unwrap();
    let run_id = world.task_of(first.task_id).await.run_id.unwrap();
    world.turn_end(run_id, 1).await;
    world.worker.settle().await.unwrap();
    world.turn_end(run_id, 1).await;
    world.worker.settle().await.unwrap();
    let task = world.task_of(first.task_id).await;
    assert_eq!(
        (task.status.as_str(), task.wait_reason.as_deref()),
        ("WAITING", Some("message"))
    );
    let replies = messages(&world, first.message.conversation_id)
        .await
        .into_iter()
        .filter(|message| message.author_kind == AUTHOR_AGENT)
        .map(|message| (message.text, message.task_id, message.run_id))
        .collect::<Vec<_>>();
    assert_eq!(
        replies,
        [(
            "done for now".to_string(),
            Some(first.task_id),
            Some(run_id)
        )]
    );
    post(&world, first.message.conversation_id, "Thanks, go on").await;
    world.worker.settle().await.unwrap();
    assert_eq!(world.task_of(first.task_id).await.status, "RUNNING");
}

#[tokio::test]
async fn a_task_waiting_for_the_next_message_has_no_time_limit_and_the_next_message_restarts_it() {
    let world = World::new("max").await;
    let first = start(&world, "all", "What is failing?").await;
    world.worker.settle().await.unwrap();
    let run_id = world.task_of(first.task_id).await.run_id.unwrap();
    world.turn_end(run_id, 1).await;
    world.worker.settle().await.unwrap();
    world.clock.advance(chrono::Duration::days(2));
    world.worker.settle().await.unwrap();
    let task = world.task_of(first.task_id).await;
    assert_eq!(
        (task.status.as_str(), task.wait_reason.as_deref()),
        ("WAITING", Some("message"))
    );
    post(&world, first.message.conversation_id, "Thanks, go on").await;
    world.worker.settle().await.unwrap();
    let task = stored_task(&world, first.task_id).await;
    assert_eq!(
        (task.status, task.input_at),
        (TaskStatus::Running, Some(world.clock_now()))
    );
}

#[tokio::test]
async fn file_issue_records_the_issue_and_its_task_becomes_a_child_of_the_conversation_task() {
    let world = World::new("max").await;
    let first = start(&world, "all", "Split the checkout work").await;
    world.worker.settle().await.unwrap();
    let run_id = world.task_of(first.task_id).await.run_id.unwrap();
    let input = json!({
        "Repo": REPO,
        "Title": "Add a checkout spec",
        "Body": "The checkout flow has no spec.",
        "Labels": ["sanchoris"],
    });
    world
        .try_request_skill(run_id, "file-issue", input)
        .await
        .unwrap();
    world.drive().await;
    let (issue, labels) = {
        let state = world.github.state.lock().unwrap();
        state.issues[REPO][0].clone()
    };
    assert_eq!(issue.title, "Add a checkout spec");
    assert_eq!(labels, ["sanchoris"]);
    assert!(
        issue
            .body
            .starts_with("The checkout flow has no spec.\n\n<!-- sanchoris:")
    );
    let outputs = world.dynamo.all::<Output>().await;
    let output = outputs.iter().find(|o| o.kind == "issue").unwrap();
    assert_eq!(
        (output.task_id, output.number, output.title.as_deref()),
        (first.task_id, issue.number, Some("Add a checkout spec"))
    );
    let filed = world
        .store
        .get::<FiledIssue>(&keys::filed_issue_key(
            world.tenant(),
            common::SHOP_WEB_ID,
            issue.number,
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(filed.task_id, first.task_id);

    let mut payload = common::labeled_payload(issue.number, &issue.title, APP_LOGIN);
    payload["sender"]["type"] = json!("Bot");
    world.deliver("delivery-filed", "issues", &payload).await;
    world.worker.settle().await.unwrap();
    let child = world.task(&format!("{REPO}#{}", issue.number)).await;
    assert_eq!(child.parent_task_id, Some(first.task_id));
}

#[tokio::test]
async fn a_rewind_drops_a_message_and_what_follows_and_a_fork_copies_up_to_a_reply() {
    let world = World::new("max").await;
    let first = start(&world, "all", "Plan the release").await;
    world.worker.settle().await.unwrap();
    let conversation_id = first.message.conversation_id;
    let run_id = world.task_of(first.task_id).await.run_id.unwrap();
    world
        .dynamo
        .modify::<Run>(
            &keys::run_key(world.tenant(), first.task_id, run_id),
            |run| run.status = "running".into(),
        )
        .await;
    let record = |turn: i64| {
        json!({
            "sessionId": "session-1",
            "turn": turn,
            "key": format!("t/{}/runs/{run_id}/turn-{turn}.jsonl", world.tenant()),
            "relativePath": "session-1.jsonl",
            "sha256": "0".repeat(64),
            "contentLength": 2,
        })
    };
    world.clock.advance(chrono::Duration::seconds(10));
    world
        .sequenced_event(run_id, 101, RunEventType::SessionRecord, record(1))
        .await;
    world.turn_end(run_id, 1).await;
    world.worker.settle().await.unwrap();
    world.clock.advance(chrono::Duration::seconds(10));
    let second = post(&world, conversation_id, "Then tag it").await;
    world.worker.settle().await.unwrap();
    world.clock.advance(chrono::Duration::seconds(10));
    world
        .sequenced_event(run_id, 102, RunEventType::SessionRecord, record(2))
        .await;
    world.turn_end(run_id, 2).await;
    world.worker.settle().await.unwrap();
    let reply = messages(&world, conversation_id)
        .await
        .into_iter()
        .find(|message| message.author_kind == AUTHOR_AGENT)
        .unwrap();

    let rewound = retry(20, || async {
        let mut tx = world.store.tx(world.tenant());
        let rewound = rewind(&mut tx, conversation_id, second.message.id, true)
            .await
            .map_err(db_error)?;
        tx.commit().await?;
        Ok(rewound)
    })
    .await
    .unwrap();
    assert_eq!(rewound.text, "Then tag it");
    assert_eq!(
        messages(&world, conversation_id)
            .await
            .into_iter()
            .map(|message| message.text)
            .collect::<Vec<_>>(),
        ["Plan the release".to_string(), "done for now".to_string()]
    );
    let rewinds = world.outbox(run_id, "rewind").await;
    assert_eq!(rewinds.len(), 1);
    assert_eq!(rewinds[0]["resume"]["recordKey"], record(1)["key"]);
    assert_eq!(rewinds[0]["restoreTurn"], 2);

    let forked = retry(20, || async {
        let mut tx = world.store.tx(world.tenant());
        let forked = fork(
            &mut tx,
            conversation_id,
            reply.id,
            world.seeded.member_id,
            world.clock_now(),
        )
        .await
        .map_err(db_error)?;
        tx.commit().await?;
        Ok(forked)
    })
    .await
    .unwrap();
    assert_eq!(forked.title, "Plan the release fork");
    assert_eq!(forked.resume.as_ref().unwrap()["lastTurn"], 0);
    assert_eq!(
        forked.resume.as_ref().unwrap()["recordKey"],
        record(1)["key"]
    );
    assert_eq!(messages(&world, forked.id).await.len(), 2);
}

fn turn_end_with_context(turn: i64, used_tokens: i64) -> Value {
    let mut payload = turn_end_payload(turn, Value::Null);
    payload["context"] = json!({ "usedTokens": used_tokens, "maxTokens": 200_000 });
    payload
}

async fn compactions(world: &World, run_id: Uuid) -> Vec<Value> {
    world
        .outbox(run_id, "user_message")
        .await
        .into_iter()
        .filter(|payload| payload["compact"] == true)
        .collect()
}

#[tokio::test]
async fn an_idle_chat_whose_context_is_full_enough_compacts_once_after_the_idle_minutes() {
    let world = World::new("max").await;
    let first = start(&world, "all", "What is failing?").await;
    let conversation_id = first.message.conversation_id;
    world.worker.settle().await.unwrap();
    let run_id = world.task_of(first.task_id).await.run_id.unwrap();
    world
        .sequenced_event(
            run_id,
            1,
            RunEventType::TurnEnd,
            turn_end_with_context(1, 120_000),
        )
        .await;
    world.worker.settle().await.unwrap();
    assert_eq!(
        conversation(&world, conversation_id).await.context,
        Some(ContextUsage {
            used_tokens: 120_000,
            max_tokens: 200_000
        })
    );
    world.clock.advance(chrono::Duration::minutes(54));
    world.worker.settle().await.unwrap();
    assert!(compactions(&world, run_id).await.is_empty());
    world.clock.advance(chrono::Duration::minutes(2));
    world.worker.settle().await.unwrap();
    let sent = compactions(&world, run_id).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0]["text"],
        "/compact Keep the decisions and the next step."
    );
    assert_eq!(sent[0]["steer"], false);
    let last = messages(&world, conversation_id).await.pop().unwrap();
    assert_eq!(
        (
            last.author_kind.as_str(),
            last.text.as_str(),
            last.member_id
        ),
        (
            AUTHOR_SANCHORIS,
            "/compact Keep the decisions and the next step.",
            None
        )
    );
    assert_eq!(world.task_of(first.task_id).await.status, "RUNNING");
    world
        .sequenced_event(
            run_id,
            2,
            RunEventType::TurnEnd,
            turn_end_with_context(2, 110_000),
        )
        .await;
    world.worker.settle().await.unwrap();
    world.clock.advance(chrono::Duration::minutes(60));
    world.worker.settle().await.unwrap();
    assert_eq!(
        compactions(&world, run_id).await.len(),
        1,
        "a compaction never compacts again"
    );
}

#[tokio::test]
async fn a_message_within_the_idle_minutes_or_a_lighter_context_keeps_the_session_as_it_is() {
    let world = World::new("max").await;
    let first = start(&world, "all", "What is failing?").await;
    let conversation_id = first.message.conversation_id;
    world.worker.settle().await.unwrap();
    let run_id = world.task_of(first.task_id).await.run_id.unwrap();
    world
        .sequenced_event(
            run_id,
            1,
            RunEventType::TurnEnd,
            turn_end_with_context(1, 90_000),
        )
        .await;
    world.worker.settle().await.unwrap();
    world.clock.advance(chrono::Duration::minutes(60));
    world.worker.settle().await.unwrap();
    assert!(
        compactions(&world, run_id).await.is_empty(),
        "45% is under 50%"
    );
    post(&world, conversation_id, "Go on").await;
    world.worker.settle().await.unwrap();
    world
        .sequenced_event(
            run_id,
            2,
            RunEventType::TurnEnd,
            turn_end_with_context(2, 150_000),
        )
        .await;
    world.worker.settle().await.unwrap();
    world.clock.advance(chrono::Duration::minutes(30));
    post(&world, conversation_id, "And the docs").await;
    world.worker.settle().await.unwrap();
    world.clock.advance(chrono::Duration::minutes(30));
    world.worker.settle().await.unwrap();
    assert!(
        compactions(&world, run_id).await.is_empty(),
        "the member wrote in time"
    );
}

#[tokio::test]
async fn a_member_compacts_with_slash_compact_but_not_into_a_running_turn() {
    let world = World::new("max").await;
    let first = start(&world, "all", "What is failing?").await;
    let conversation_id = first.message.conversation_id;
    world.worker.settle().await.unwrap();
    let run_id = world.task_of(first.task_id).await.run_id.unwrap();
    post(&world, conversation_id, "Then fix it").await;
    let member_id = world.seeded.member_id;
    let at = world.clock_now();
    let mut tx = world.store.tx(world.tenant());
    let steered = post_member_message(
        &mut tx,
        conversation_id,
        &MemberMessage {
            id: Uuid::new_v4(),
            member_id,
            member_name: "Mika",
            text: "/compact",
            modes: SessionModes::DEFAULT,
            delivery: Delivery::Steer,
            attachments: &[],
            at,
        },
    )
    .await;
    assert!(matches!(steered, Err(ChatError::Rejected(_))));
    drop(tx);
    world.turn_end(run_id, 1).await;
    world.worker.settle().await.unwrap();
    world
        .sequenced_event(
            run_id,
            2,
            RunEventType::TurnEnd,
            turn_end_payload(2, Value::Null),
        )
        .await;
    world.worker.settle().await.unwrap();
    post(&world, conversation_id, " /COMPACT ").await;
    world.worker.settle().await.unwrap();
    world
        .sequenced_event(
            run_id,
            3,
            RunEventType::TurnEnd,
            turn_end_payload(3, Value::Null),
        )
        .await;
    world.worker.settle().await.unwrap();
    post(
        &world,
        conversation_id,
        "/compact Keep the failing test name",
    )
    .await;
    world.worker.settle().await.unwrap();
    let sent = compactions(&world, run_id)
        .await
        .into_iter()
        .map(|payload| payload["text"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        sent,
        [
            "/compact Keep the decisions and the next step.",
            "/compact Keep the decisions and the next step.\n\nKeep the failing test name",
        ]
    );
}
