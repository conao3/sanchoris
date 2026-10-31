use sanchoris_core::conversations::load_conversation;
use sanchoris_core::db::tenant_plan;
use sanchoris_core::items::{
    AUTHOR_AGENT, ChatMessage, Conversation, ConversationTask, FiledIssue, HeldMessage, Output,
    Task, Tenant,
};
use sanchoris_core::jobs::Job;
use sanchoris_core::keys;
use sanchoris_core::model::{TaskDefinition, TaskStatus};
use sanchoris_core::store::retry;
use sanchoris_worker::conversation::{
    ChatError, Delivered, MemberMessage, NewConversation, post_member_message, start_conversation,
};
use serde_json::json;
use uuid::Uuid;

use crate::common::{self, APP_LOGIN, REPO, World};

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
    let world = World::new("pro").await;
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
    let definition: TaskDefinition = serde_json::from_value(task.definition.clone()).unwrap();
    let mut tx = world.store.tx(world.tenant());
    let (_, plan) = tenant_plan(&mut tx).await.unwrap();
    assert_eq!(definition.trigger, None);
    assert_eq!(definition.start.limits, plan.chat_limits);
    assert_eq!(definition.start.skills, ["file-issue"]);
    assert!(definition.start.required_skills.is_empty());
    let view = world.task_of(task.id).await;
    assert_eq!(view.status, "RUNNING");
    let starts = world.outbox(view.run_id.unwrap(), "start").await;
    let instruction = starts[0]["instruction"].as_str().unwrap();
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
    let world = World::new("pro").await;
    let delivered = start(&world, "shop", "Look at the checkout").await;
    world.worker.settle().await.unwrap();
    let view = world.task_of(delivered.task_id).await;
    let starts = world.outbox(view.run_id.unwrap(), "start").await;
    assert_eq!(
        starts[0]["systemPrompt"],
        "You are working on the Acme shop. Always write tests for new endpoints.\n"
    );
    let definition: TaskDefinition =
        serde_json::from_value(stored_task(&world, delivered.task_id).await.definition).unwrap();
    assert_eq!(definition.start.skills, ["file-issue", "open-pr-green-ci"]);
}

#[tokio::test]
async fn an_unknown_session_is_rejected() {
    let world = World::new("pro").await;
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
    let world = World::new("pro").await;
    let first = start(&world, "all", "Plan the next release").await;
    world.worker.settle().await.unwrap();
    let conversation_id = first.message.conversation_id;
    let run_id = world.task_of(first.task_id).await.run_id.unwrap();
    let second = post(&world, conversation_id, "Also check the changelog").await;
    world.worker.settle().await.unwrap();
    assert_eq!(second.task_id, first.task_id);
    assert_eq!(
        world.messages(run_id).await,
        vec!["[sanchoris] Message from Mika:\nAlso check the changelog".to_string()]
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
    let world = World::new("pro").await;
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
    let world = World::new("pro").await;
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
    let world = World::new("pro").await;
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
async fn file_issue_records_the_issue_and_its_task_becomes_a_child_of_the_conversation_task() {
    let world = World::new("pro").await;
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
