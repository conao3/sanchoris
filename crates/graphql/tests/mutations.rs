use crate::common;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use common::World;
use sanchoris_core::item::Table;
use sanchoris_core::items::{
    CONFIG_DISABLED, ChangeRequest, ChatMessage, Config, Credential, Guard, Invitation, Member,
    Membership, OutboxItem, Pending, ReceiveRate, Run, RunEventItem, SkillRunItem, Task,
    TaskOrigin, Tenant, Trigger, User,
};
use sanchoris_core::jobs::{Deletion, Job, Relayed};
use sanchoris_core::keys;
use sanchoris_core::model::{AuditOrigin, SkillRunStatus, TaskStatus};
use sanchoris_core::receive::window_start;
use sanchoris_core::tenants::{
    NewTenant, PendingSession, create_tenant, free_github_tenant_key, free_tenant_key,
    pending_checkout,
};
use sanchoris_graphql::{ConfigProblem, PlanChange};
use serde_json::{Value, json};
use uuid::Uuid;

fn iso(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[tokio::test]
async fn signing_out_is_audited_in_each_tenant_of_the_viewer() {
    let w = World::new().await;
    let joined = Uuid::new_v4();
    let mut tx = w.store.tx(w.globex);
    tx.insert(&Member {
        id: joined,
        tenant_id: w.globex,
        user_id: w.mika.user_id,
        role: "member".into(),
        joined_at: w.now,
        last_active_at: None,
    })
    .unwrap();
    tx.insert(&Membership {
        user_id: w.mika.user_id,
        tenant_id: w.globex,
        member_id: joined,
    })
    .unwrap();
    tx.commit().await.unwrap();
    let signed_out = w.ok(&w.mika, "SignOut", json!({})).await;
    assert_eq!(signed_out["signOut"], true);
    for (tenant, member) in [(w.acme, w.mika.member_id.unwrap()), (w.globex, joined)] {
        let event = w.audit(tenant, "auth.sign_out").await;
        assert_eq!(event.actor_kind, "member");
        assert_eq!(event.actor_ref, member.to_string());
        assert_eq!(event.target_ref, w.mika.user_id.to_string());
    }
}

#[tokio::test]
async fn owners_delete_a_tenant_by_typing_its_slug_and_its_work_stops() {
    let w = World::new().await;
    let delete = |slug: &str| json!({ "tenantId": w.acme, "slug": slug });
    for who in [&w.sota, &w.ken] {
        assert_eq!(
            w.error_code(who, "DeleteTenant", delete("acme")).await,
            "FORBIDDEN"
        );
    }
    for slug in ["globex", "Acme", ""] {
        assert_eq!(
            w.error_code(&w.mika, "DeleteTenant", delete(slug)).await,
            "BAD_USER_INPUT"
        );
    }
    assert!(w.recorder.billing_ended.lock().unwrap().is_empty());
    let tenant_key = keys::tenant_key(w.acme);
    assert_eq!(w.get::<Tenant>(&tenant_key).await.unwrap().status, "active");
    assert!(!w.task(w.acme, w.issue_task).await.status.is_terminal());

    let deleted = w.ok(&w.mika, "DeleteTenant", delete("acme")).await;
    assert_eq!(deleted["deleteTenant"], w.acme.to_string());
    assert_eq!(
        w.get::<Tenant>(&tenant_key).await.unwrap().status,
        "deleting"
    );
    assert_eq!(*w.recorder.billing_ended.lock().unwrap(), vec![w.acme]);
    let audit = w.audit(w.acme, "tenant.delete").await;
    assert_eq!(audit.actor_ref, w.mika.member_id.unwrap().to_string());
    for task in [w.issue_task, w.chat_task] {
        assert_eq!(w.task(w.acme, task).await.status, TaskStatus::Aborted);
    }
    let cleanup = w
        .messages()
        .await
        .into_iter()
        .find_map(|message| match message {
            Relayed::Send {
                envelope,
                deliver_at,
            } => match envelope.job {
                Job::DeleteTenant { tenant_id, phase } => Some((tenant_id, phase, deliver_at)),
                _ => None,
            },
            Relayed::Cancel { .. } => None,
        })
        .expect("the cleanup job is emitted");
    assert_eq!(cleanup.0, w.acme);
    assert_eq!(cleanup.1, Deletion::Release);
    assert_eq!(cleanup.2, Some(w.now + Duration::minutes(1)));

    for (operation, variables) in [
        ("DeleteTenant", delete("acme")),
        (
            "SetTriggerState",
            json!({ "triggerId": w.labeled_trigger, "state": "DISABLED" }),
        ),
        ("SyncConfig", json!({ "configId": w.config_id })),
        (
            "SendChatMessage",
            json!({ "input": { "tenantId": w.acme, "sessionName": "all", "text": "hi" } }),
        ),
        (
            "InviteMember",
            json!({ "input": { "tenantId": w.acme, "email": "x@acme.dev", "role": "MEMBER" } }),
        ),
        (
            "RemoveCredential",
            json!({ "credentialId": w.org_credential }),
        ),
    ] {
        assert_eq!(
            w.error_code(&w.mika, operation, variables.clone()).await,
            "CONFLICT",
            "{operation} {variables}"
        );
    }
    for who in [&w.mika, &w.sota] {
        let shell = w.ok(who, "Shell", json!({})).await;
        assert!(
            shell["viewer"]["memberships"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| m["tenant"]["slug"] != "acme")
        );
    }
    let tenants = w.ok(&w.operator, "OperatorTenants", json!({})).await;
    let acme = tenants["operator"]["tenants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tenant| tenant["id"] == w.acme.to_string())
        .unwrap()
        .clone();
    assert_eq!(acme["state"], "DELETING");
}

#[tokio::test]
async fn task_and_run_mutations_reach_the_agent_session() {
    let w = World::new().await;

    assert_eq!(
        w.error_code(
            &w.ken,
            "ResumeTask",
            json!({ "input": { "taskId": w.chat_task, "message": "go" } }),
        )
        .await,
        "CONFLICT"
    );
    let resumed = w
        .ok(
            &w.ken,
            "ResumeTask",
            json!({ "input": { "taskId": w.issue_task, "message": "Use the staging database." } }),
        )
        .await;
    assert_eq!(resumed["resumeTask"]["id"], w.issue_task.to_string());
    let reply = w
        .all::<RunEventItem>()
        .await
        .into_iter()
        .find(|event| event.run_id == w.issue_run && event.kind == "human_reply")
        .expect("the reply is recorded");
    assert_eq!(
        reply.payload,
        json!({ "member": "Ken", "text": "Use the staging database." })
    );
    assert_eq!(reply.seq, None);
    let pending = w
        .all::<Pending>()
        .await
        .into_iter()
        .filter(|marker| marker.id == reply.id)
        .count();
    assert_eq!(pending, 1);
    let audit = w.audit(w.acme, "task.resume").await;
    assert_eq!(audit.actor_kind, "member");
    assert_eq!(audit.actor_ref, w.ken.member_id.unwrap().to_string());
    assert_eq!(audit.target_ref, w.issue_task.to_string());
    let ken: Member = w
        .get(&keys::member_key(w.acme, w.ken.member_id.unwrap()))
        .await
        .unwrap();
    assert_eq!(ken.last_active_at, Some(w.now));

    let until = w.now + Duration::hours(3);
    let snoozed = w
        .ok(
            &w.mika,
            "SnoozeTask",
            json!({ "taskId": w.issue_task, "until": iso(until) }),
        )
        .await;
    assert_eq!(snoozed["snoozeTask"]["snoozedUntil"], iso(until));
    assert_eq!(
        w.task(w.acme, w.issue_task)
            .await
            .snoozed_until
            .map(|at| at.to_rfc3339_opts(SecondsFormat::Secs, true)),
        Some(iso(until))
    );
    w.audit(w.acme, "task.snooze").await;
    assert_eq!(
        w.error_code(
            &w.mika,
            "SnoozeTask",
            json!({ "taskId": w.chat_task, "until": iso(until) })
        )
        .await,
        "CONFLICT"
    );
    assert_eq!(
        w.error_code(
            &w.mika,
            "SnoozeTask",
            json!({ "taskId": w.issue_task, "until": iso(w.now - Duration::hours(1)) }),
        )
        .await,
        "BAD_USER_INPUT"
    );

    let stopped_run = w
        .ok(&w.ken, "StopRun", json!({ "runId": w.chat_run }))
        .await;
    assert_eq!(stopped_run["stopRun"]["status"], "COMPLETED");
    let shutdown = w
        .all::<OutboxItem>()
        .await
        .into_iter()
        .find(|outbox| outbox.run_id == w.chat_run && outbox.kind == "shutdown")
        .expect("the shutdown is queued");
    assert_eq!(
        shutdown.payload,
        json!({ "reason": "Ken stopped the session" })
    );
    assert_eq!(
        w.audit(w.acme, "run.stop").await.target_ref,
        w.chat_task.to_string()
    );
    assert_eq!(
        w.error_code(&w.ken, "StopRun", json!({ "runId": w.chat_run }))
            .await,
        "CONFLICT"
    );

    let stopped = w
        .ok(&w.mika, "StopTask", json!({ "taskId": w.issue_task }))
        .await;
    assert_eq!(stopped["stopTask"]["status"], "ABORTED");
    let run: Run = w
        .get(&keys::run_key(w.acme, w.issue_task, w.issue_run))
        .await
        .unwrap();
    assert_eq!(run.status, "completed");
    let task = w.task(w.acme, w.issue_task).await;
    assert_eq!(task.error, None);
    assert_eq!(task.status, TaskStatus::Aborted);
    w.audit(w.acme, "task.stop").await;
    assert_eq!(
        w.error_code(&w.mika, "StopTask", json!({ "taskId": w.issue_task }))
            .await,
        "CONFLICT"
    );
}

#[tokio::test]
async fn chat_messages_start_conversations_and_continue_them() {
    let w = World::new().await;

    let started = w
        .ok(
            &w.ken,
            "SendChatMessage",
            json!({ "input": { "tenantId": w.acme, "sessionName": "all", "text": "Add a dark mode" } }),
        )
        .await;
    let payload = &started["sendChatMessage"];
    let conversation_id: Uuid = payload["conversation"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let task_id: Uuid = payload["task"]["id"].as_str().unwrap().parse().unwrap();
    assert_ne!(task_id, w.chat_task);
    assert_eq!(payload["message"]["text"], "Add a dark mode");
    let task = w.task(w.acme, task_id).await;
    assert_eq!(task.requested_by_member_id, w.ken.member_id);
    assert_eq!(
        task.origin,
        TaskOrigin::Conversation {
            conversation_id,
            sequence: 1
        }
    );
    assert_eq!(task.status, TaskStatus::Pending);
    let audit = w.audit(w.acme, "chat.start").await;
    assert_eq!(audit.target_kind, "conversation");

    let chat = w
        .ok(
            &w.ken,
            "Chat",
            json!({ "tenant": "acme", "conversation": conversation_id, "withConversation": true }),
        )
        .await;
    let conversation = &chat["tenant"]["conversation"];
    assert_eq!(conversation["messages"][0]["text"], "Add a dark mode");
    assert_eq!(conversation["createdBy"]["user"]["name"], "Ken");
    assert_eq!(conversation["sessionName"], "all");
    assert_eq!(conversation["latestTask"]["id"], task_id.to_string());
    assert_eq!(
        chat["tenant"]["conversations"][0]["id"],
        conversation_id.to_string()
    );

    let continued = w
        .ok(
            &w.mika,
            "SendChatMessage",
            json!({ "input": { "tenantId": w.acme, "conversationId": w.chat_conversation, "text": "Any update?" } }),
        )
        .await;
    assert_eq!(
        continued["sendChatMessage"]["task"]["id"],
        w.chat_task.to_string()
    );
    assert_eq!(
        w.task(w.acme, w.chat_task).await.status,
        TaskStatus::Running
    );
    w.audit(w.acme, "chat.message_send").await;
    let chat = w
        .ok(
            &w.mika,
            "Chat",
            json!({ "tenant": "acme", "conversation": w.chat_conversation, "withConversation": true }),
        )
        .await;
    let texts = chat["tenant"]["conversation"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["text"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        texts,
        vec![
            "What is failing?".to_string(),
            "The checkout spec is flaky.".to_string(),
            "Any update?".to_string(),
        ]
    );

    w.dynamo
        .modify::<Task>(&keys::task_key(w.acme, w.chat_task), |task| {
            task.status = TaskStatus::Succeeded;
        })
        .await;
    let next = w
        .ok(
            &w.mika,
            "SendChatMessage",
            json!({ "input": { "tenantId": w.acme, "conversationId": w.chat_conversation, "text": "Next step" } }),
        )
        .await;
    let next_id: Uuid = next["sendChatMessage"]["task"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_ne!(next_id, w.chat_task);
    let next_task = w.task(w.acme, next_id).await;
    assert_eq!(next_task.name, "2");
    assert_eq!(next_task.requested_by_member_id, w.mika.member_id);

    for (input, code) in [
        (
            json!({ "tenantId": w.acme, "sessionName": "nope", "text": "hi" }),
            "BAD_USER_INPUT",
        ),
        (
            json!({ "tenantId": w.acme, "text": "hi" }),
            "BAD_USER_INPUT",
        ),
        (
            json!({ "tenantId": w.acme, "conversationId": Uuid::new_v4(), "text": "hi" }),
            "NOT_FOUND",
        ),
        (
            json!({ "tenantId": w.acme, "conversationId": w.chat_conversation, "text": "  " }),
            "BAD_USER_INPUT",
        ),
    ] {
        assert_eq!(
            w.error_code(&w.mika, "SendChatMessage", json!({ "input": input }))
                .await,
            code
        );
    }
}

#[tokio::test]
async fn trigger_and_config_mutations_change_the_config_records() {
    let w = World::new().await;

    let disabled = w
        .ok(
            &w.sota,
            "SetTriggerState",
            json!({ "triggerId": w.nightly_trigger, "state": "DISABLED" }),
        )
        .await;
    assert_eq!(disabled["setTriggerState"]["state"], "DISABLED");
    let nightly_key = keys::trigger_key(w.acme, w.config_id, "nightly");
    let disabled_item: Trigger = w.get(&nightly_key).await.unwrap();
    assert_eq!(disabled_item.state, "DISABLED");
    assert_eq!(disabled_item.next_run_at, None);
    assert_eq!(disabled_item.schedule_message_id, None);
    let messages = w.messages().await;
    assert_eq!(messages.len(), 1);
    assert!(matches!(&messages[0], Relayed::Cancel { .. }));
    w.audit(w.acme, "trigger.disable").await;
    w.ok(
        &w.sota,
        "SetTriggerState",
        json!({ "triggerId": w.nightly_trigger, "state": "ENABLED" }),
    )
    .await;
    let enabled: Trigger = w.get(&nightly_key).await.unwrap();
    assert_eq!(enabled.state, "ENABLED");
    let next_run_at = enabled.next_run_at.expect("the schedule is armed");
    assert!(next_run_at > w.now);
    let scheduled = w
        .messages()
        .await
        .into_iter()
        .find_map(|message| match message {
            Relayed::Send {
                envelope,
                deliver_at,
            } => Some((envelope, deliver_at)),
            Relayed::Cancel { .. } => None,
        })
        .expect("the next run is scheduled");
    assert_eq!(scheduled.0.id, enabled.schedule_message_id.unwrap());
    assert_eq!(scheduled.1, Some(next_run_at));
    assert!(matches!(
        scheduled.0.job,
        Job::TriggerSchedule { trigger_id, .. } if trigger_id == w.nightly_trigger
    ));
    w.audit(w.acme, "trigger.enable").await;

    let proposed = w
        .ok(
            &w.ken,
            "ProposeTrigger",
            json!({ "input": { "tenantId": w.acme, "configId": w.config_id, "templateId": "labeled-issue", "repository": "acme/shop-web" } }),
        )
        .await;
    assert_eq!(proposed["proposeTriggerFromTemplate"]["number"], 42);
    assert_eq!(
        proposed["proposeTriggerFromTemplate"]["url"],
        "https://github.com/acme/sanchoris-config/pull/42"
    );
    let proposal = w.recorder.proposals.lock().unwrap()[0].clone();
    assert_eq!(proposal.trigger_name, "labeled-issue-2");
    let yaml = sanchoris_config::parse_config_yaml(&format!(
        "Version: 1\nProjects: {{}}\nSessions: {{}}\nTriggers:\n{}",
        proposal
            .trigger_yaml
            .lines()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    ))
    .expect("the proposed trigger is valid config YAML");
    assert_eq!(yaml.triggers["labeled-issue-2"].start.session, "shop");
    let change: ChangeRequest = w
        .get(&keys::change_request_key(w.acme, w.config_id, 42))
        .await
        .unwrap();
    assert_eq!(
        change.changed_definitions,
        json!([{ "kind": "trigger", "name": "labeled-issue-2" }])
    );
    assert_eq!(change.requested_by_member_id, w.ken.member_id);
    assert_eq!(
        proposed["proposeTriggerFromTemplate"]["id"],
        change.id.to_string()
    );
    w.audit(w.acme, "change_request.open").await;
    assert_eq!(
        w.error_code(
            &w.ken,
            "ProposeTrigger",
            json!({ "input": { "tenantId": w.acme, "configId": w.config_id, "templateId": "labeled-issue", "repository": "acme/unknown" } }),
        )
        .await,
        "BAD_USER_INPUT"
    );

    *w.recorder.problems.lock().unwrap() = vec![ConfigProblem {
        path: "sanchoris.yaml".into(),
        line: Some(3),
        column: Some(5),
        message: "Version must be 1".into(),
    }];
    let failed = w
        .ok(
            &w.sota,
            "AddConfig",
            json!({ "input": { "tenantId": w.acme, "repository": "acme/platform", "root": "tools/sanchoris/", "branch": "main" } }),
        )
        .await;
    assert_eq!(failed["addConfig"]["config"], Value::Null);
    assert_eq!(failed["addConfig"]["errors"][0]["line"], 3);
    w.audit(w.acme, "config.validation_fail").await;
    w.recorder.problems.lock().unwrap().clear();
    let added = w
        .ok(
            &w.sota,
            "AddConfig",
            json!({ "input": { "tenantId": w.acme, "repository": "acme/platform", "root": "tools/sanchoris/", "branch": "release" } }),
        )
        .await;
    let config_id: Uuid = added["addConfig"]["config"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let added_config: Config = w.get(&keys::config_key(w.acme, config_id)).await.unwrap();
    assert_eq!(added_config.branch, "release");
    let guard: Guard = w
        .get(&keys::config_repository_key(
            w.acme,
            common::repository_id("acme/platform"),
            "tools/sanchoris/",
        ))
        .await
        .unwrap();
    assert_eq!(guard.target, config_id);
    assert_eq!(*w.recorder.syncs.lock().unwrap(), vec![config_id]);
    w.audit(w.acme, "config.add").await;
    assert_eq!(
        w.error_code(
            &w.sota,
            "AddConfig",
            json!({ "input": { "tenantId": w.acme, "repository": "acme/platform", "root": "tools/sanchoris/", "branch": "main" } }),
        )
        .await,
        "CONFLICT"
    );
    let configs = w.ok(&w.sota, "Configs", json!({ "tenant": "acme" })).await;
    let platform = configs["tenant"]["configs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["repository"] == "acme/platform")
        .unwrap()
        .clone();
    assert_eq!(platform["revision"]["number"], 0);
    assert_eq!(
        platform["url"],
        "https://github.com/acme/platform/tree/release/tools/sanchoris"
    );

    let synced = w
        .ok(&w.sota, "SyncConfig", json!({ "configId": w.config_id }))
        .await;
    assert_eq!(synced["syncConfig"]["id"], w.config_id.to_string());
    assert_eq!(w.recorder.syncs.lock().unwrap().len(), 2);
    w.audit(w.acme, "config.sync").await;
}

#[tokio::test]
async fn credentials_are_encrypted_and_removable_by_their_owner() {
    let w = World::new().await;
    let key = "sk-ant-api03-secretvalue9f2c";
    let added = w
        .ok(
            &w.ken,
            "AddCredential",
            json!({ "input": { "tenantId": w.acme, "name": "ken-key", "provider": "ANTHROPIC", "key": key, "scope": "PERSONAL" } }),
        )
        .await;
    let id: Uuid = added["addCredential"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let row: Credential = w.get(&keys::credential_key(w.acme, id)).await.unwrap();
    assert_eq!(row.member_id, w.ken.member_id);
    assert_eq!(row.scope, "personal");
    assert_eq!(row.provider, "claude");
    assert_eq!(row.kind, "api_key");
    assert_eq!(row.account, "sk-a…9f2c");
    assert_eq!(row.kms_key_id, sanchoris_graphql::LOCAL_KEY_ID);
    assert!(!row.encrypted_value.contains(key));
    let encrypted = STANDARD.decode(&row.encrypted_value).unwrap();
    assert!(!encrypted.windows(key.len()).any(|w| w == key.as_bytes()));
    let stored = w
        .dynamo
        .scan(Table::Main, "credential")
        .await
        .into_iter()
        .map(|item| format!("{item:?}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!stored.contains(key));
    let name_guard: Guard = w
        .get(&keys::credential_name_key(w.acme, "ken-key"))
        .await
        .unwrap();
    assert_eq!(name_guard.target, id);
    assert_eq!(
        w.encryptor.decrypt(w.acme, &encrypted).unwrap(),
        key.as_bytes()
    );
    let audit = w.audit(w.acme, "credential.connect").await;
    assert!(!audit.summary.contains(key));
    assert_eq!(
        w.error_code(
            &w.mika,
            "AddCredential",
            json!({ "input": { "tenantId": w.acme, "name": "ken-key", "provider": "OPENAI", "key": "sk-proj-example", "scope": "ORGANIZATION" } }),
        )
        .await,
        "CONFLICT"
    );
    for (provider, wrong) in [
        ("OPENAI", "not-a-real-key"),
        ("OPENAI", "sk-ant-api03-0123456789"),
        ("ANTHROPIC", "sk-proj-0123456789abcdef"),
    ] {
        assert_eq!(
            w.error_code(
                &w.ken,
                "AddCredential",
                json!({ "input": { "tenantId": w.acme, "name": "wrong-key", "provider": provider, "key": wrong, "scope": "PERSONAL" } }),
            )
            .await,
            "BAD_USER_INPUT",
            "{provider} {wrong}"
        );
    }
    w.ok(
        &w.ken,
        "AddCredential",
        json!({ "input": { "tenantId": w.acme, "name": "ken-oauth", "provider": "ANTHROPIC", "key": "sk-ant-oat01-0123456789abcd", "scope": "PERSONAL" } }),
    )
    .await;
    let org = w
        .ok(
            &w.sota,
            "AddCredential",
            json!({ "input": { "tenantId": w.acme, "name": "acme-bedrock", "provider": "BEDROCK", "key": "AKIAEXAMPLEKEY12345", "scope": "ORGANIZATION" } }),
        )
        .await;
    let org_id: Uuid = org["addCredential"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let org_row: Credential = w.get(&keys::credential_key(w.acme, org_id)).await.unwrap();
    assert_eq!(org_row.kind, "bedrock");
    assert_eq!(org_row.scope, "org");
    assert_eq!(org_row.member_id, None);

    assert_eq!(
        w.error_code(
            &w.ken,
            "RemoveCredential",
            json!({ "credentialId": w.mika_credential })
        )
        .await,
        "FORBIDDEN"
    );
    let removed = w
        .ok(&w.ken, "RemoveCredential", json!({ "credentialId": id }))
        .await;
    assert_eq!(removed["removeCredential"], id.to_string());
    w.ok(
        &w.sota,
        "RemoveCredential",
        json!({ "credentialId": w.org_credential }),
    )
    .await;
    let left = w
        .all::<Credential>()
        .await
        .into_iter()
        .filter(|credential| credential.tenant_id == w.acme)
        .count();
    assert_eq!(left, 4);
    assert!(
        w.get::<Guard>(&keys::credential_name_key(w.acme, "ken-key"))
            .await
            .is_none()
    );
    let deletions = w
        .audit_actions(w.acme)
        .await
        .into_iter()
        .filter(|action| action == "credential.delete")
        .count();
    assert_eq!(deletions, 2);
}

#[tokio::test]
async fn members_are_invited_promoted_and_removed() {
    let w = World::new().await;
    let invited = w
        .ok(
            &w.sota,
            "InviteMember",
            json!({ "input": { "tenantId": w.acme, "email": "New@Acme.dev", "role": "MEMBER" } }),
        )
        .await;
    assert_eq!(invited["inviteMember"]["email"], "new@acme.dev");
    assert_eq!(invited["inviteMember"]["state"], "PENDING");
    w.audit(w.acme, "member.invite").await;
    assert_eq!(
        w.error_code(
            &w.sota,
            "InviteMember",
            json!({ "input": { "tenantId": w.acme, "email": "new@acme.dev", "role": "MEMBER" } }),
        )
        .await,
        "CONFLICT"
    );
    assert_eq!(
        w.error_code(
            &w.sota,
            "InviteMember",
            json!({ "input": { "tenantId": w.acme, "email": "ken@acme.dev", "role": "MEMBER" } }),
        )
        .await,
        "CONFLICT"
    );
    let invitation_id = invited["inviteMember"]["id"].as_str().unwrap().to_string();
    let revoked = w
        .ok(
            &w.sota,
            "RevokeInvitation",
            json!({ "invitationId": invitation_id }),
        )
        .await;
    assert_eq!(revoked["revokeInvitation"]["state"], "REVOKED");
    w.audit(w.acme, "member.invitation_revoke").await;
    assert!(
        w.get::<Guard>(&keys::pending_invitation_key(w.acme, "new@acme.dev"))
            .await
            .is_none()
    );
    let again = w
        .ok(
            &w.sota,
            "InviteMember",
            json!({ "input": { "tenantId": w.acme, "email": "new@acme.dev", "role": "MEMBER" } }),
        )
        .await;
    assert_eq!(again["inviteMember"]["state"], "PENDING");
    w.ok(
        &w.sota,
        "RevokeInvitation",
        json!({ "invitationId": again["inviteMember"]["id"] }),
    )
    .await;

    let promoted = w
        .ok(
            &w.mika,
            "ChangeMemberRole",
            json!({ "memberId": w.ken.member_id, "role": "ADMIN" }),
        )
        .await;
    assert_eq!(promoted["changeMemberRole"]["role"], "ADMIN");
    assert!(
        w.audit(w.acme, "member.role_change")
            .await
            .summary
            .contains("from member to admin")
    );

    let accepted = w
        .ok(
            &w.ren,
            "AcceptInvitation",
            json!({ "invitationId": w.invitation }),
        )
        .await;
    assert_eq!(accepted["acceptInvitation"]["role"], "MEMBER");
    assert_eq!(accepted["acceptInvitation"]["tenant"]["slug"], "acme");
    let ren_member: Uuid = accepted["acceptInvitation"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        w.audit(w.acme, "member.invitation_accept").await.actor_ref,
        ren_member.to_string()
    );
    let membership: Membership = w
        .get(&keys::membership_key(w.ren.user_id, w.acme))
        .await
        .unwrap();
    assert_eq!(membership.member_id, ren_member);
    let invitation: Invitation = w
        .get(&keys::invitation_key(w.acme, w.invitation))
        .await
        .unwrap();
    assert_eq!(invitation.status, "accepted");
    assert_eq!(invitation.accepted_at, Some(w.now));
    assert!(
        w.get::<Guard>(&keys::pending_invitation_key(w.acme, "ren@acme.dev"))
            .await
            .is_none()
    );
    let shell = w.ok(&w.ren, "Shell", json!({})).await;
    assert_eq!(shell["viewer"]["memberships"][0]["tenant"]["slug"], "acme");
    let page = w.ok(&w.ren, "CreateTenantPage", json!({})).await;
    assert_eq!(page["viewer"]["invitations"], json!([]));
    assert_eq!(
        w.error_code(
            &w.ren,
            "AcceptInvitation",
            json!({ "invitationId": w.invitation })
        )
        .await,
        "CONFLICT"
    );
    assert_eq!(
        w.error_code(
            &w.hana,
            "AcceptInvitation",
            json!({ "invitationId": w.invitation })
        )
        .await,
        "NOT_FOUND"
    );

    let response = w
        .inline(
            &w.ken,
            "mutation($id: ID!) { removeMember(memberId: $id) }",
            json!({ "id": ren_member }),
        )
        .await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    assert!(
        w.get::<Member>(&keys::member_key(w.acme, ren_member))
            .await
            .is_none()
    );
    assert!(
        w.get::<Membership>(&keys::membership_key(w.ren.user_id, w.acme))
            .await
            .is_none()
    );
    let audit = w.audit(w.acme, "member.remove").await;
    assert_eq!(audit.actor_ref, w.ken.member_id.unwrap().to_string());
    assert_eq!(audit.metadata["target_name"], "Ren");
    let response = w
        .inline(
            &w.ken,
            "mutation($id: ID!) { removeMember(memberId: $id) }",
            json!({ "id": w.mika.member_id }),
        )
        .await;
    assert_eq!(
        response.errors[0]
            .extensions
            .as_ref()
            .unwrap()
            .get("code")
            .unwrap()
            .to_string(),
        "\"FORBIDDEN\""
    );
}

#[tokio::test]
async fn tenants_billing_and_operator_actions() {
    let w = World::new().await;

    let paid = w
        .ok(
            &w.ren,
            "CreateTenant",
            json!({ "input": { "name": "Ren Paid", "planId": "starter" } }),
        )
        .await;
    assert_eq!(paid["createTenant"]["tenant"]["slug"], "ren-paid");
    assert_eq!(
        paid["createTenant"]["checkoutUrl"],
        "https://checkout.stripe.test/starter/Ren Paid"
    );
    let created = w
        .ok(
            &w.ren,
            "CreateTenant",
            json!({ "input": { "name": "Ren Labs", "planId": "free" } }),
        )
        .await;
    assert_eq!(created["createTenant"]["tenant"]["slug"], "ren-labs");
    let tenant_id: Uuid = created["createTenant"]["tenant"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let audit = w.audit(tenant_id, "tenant.create").await;
    assert_eq!(audit.actor_kind, "member");
    let members = w
        .all::<Member>()
        .await
        .into_iter()
        .filter(|member| member.tenant_id == tenant_id)
        .collect::<Vec<_>>();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].role, "owner");
    assert_eq!(members[0].user_id, w.ren.user_id);
    assert_eq!(
        w.error_code(
            &w.ren,
            "CreateTenant",
            json!({ "input": { "name": "Again", "planId": "free" } })
        )
        .await,
        "CONFLICT"
    );
    let second = w
        .ok(
            &w.mika,
            "CreateTenant",
            json!({ "input": { "name": "Ren Labs", "planId": "free" } }),
        )
        .await;
    assert_eq!(second["createTenant"]["tenant"]["slug"], "ren-labs-2");
    let mika_tenant: Uuid = second["createTenant"]["tenant"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let checkout = w
        .ok(
            &w.mika,
            "ChangePlan",
            json!({ "tenantId": w.acme, "planId": "max" }),
        )
        .await;
    assert_eq!(
        checkout["changePlan"]["checkoutUrl"],
        "https://checkout.stripe.test/change/max"
    );
    assert_eq!(checkout["changePlan"]["tenant"]["plan"]["name"], "Pro");
    w.audit(w.acme, "tenant.plan_checkout_start").await;
    *w.recorder.plan_change.lock().unwrap() = Some(PlanChange::Applied);
    let applied = w
        .ok(
            &w.mika,
            "ChangePlan",
            json!({ "tenantId": w.acme, "planId": "max" }),
        )
        .await;
    assert_eq!(applied["changePlan"]["checkoutUrl"], Value::Null);
    assert_eq!(applied["changePlan"]["tenant"]["plan"]["name"], "Max");
    let tenant: Tenant = w.get(&keys::tenant_key(w.acme)).await.unwrap();
    assert_eq!(tenant.plan_id, "max");
    w.audit(w.acme, "tenant.plan_change").await;

    let portal = w
        .ok(&w.mika, "OpenBillingPortal", json!({ "tenantId": w.acme }))
        .await;
    assert_eq!(
        portal["openBillingPortal"]["url"],
        format!("https://billing.stripe.test/{}", w.acme)
    );
    w.audit(w.acme, "billing.portal_open").await;

    let slack = w
        .ok(&w.sota, "ConnectSlack", json!({ "tenantId": w.acme }))
        .await;
    assert!(
        slack["connectSlack"]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://slack.test/")
    );
    w.audit(w.acme, "integration.slack_connect_start").await;

    let github = w.run(&w.mika, "ConnectGitHub", json!({})).await;
    assert!(github.errors.is_empty(), "{:?}", github.errors);
    assert_eq!(
        github.http_headers["set-cookie"],
        format!("__Host-sanchoris-github={}", w.mika.user_id).as_str()
    );
    assert!(
        github.data.into_json().unwrap()["connectGitHub"]["url"]
            .as_str()
            .unwrap()
            .contains(&w.mika.user_id.to_string())
    );
    w.audit(w.acme, "integration.github_link_start").await;
    assert_eq!(
        w.error_code(&w.ken, "InstallGitHubApp", json!({ "tenantId": w.acme }))
            .await,
        "FORBIDDEN"
    );
    let install = w
        .run(&w.sota, "InstallGitHubApp", json!({ "tenantId": w.acme }))
        .await;
    assert!(install.errors.is_empty(), "{:?}", install.errors);
    assert_eq!(
        install.http_headers["set-cookie"],
        format!("__Host-sanchoris-github={}", w.acme).as_str()
    );
    assert!(
        install.data.into_json().unwrap()["installGitHubApp"]["url"]
            .as_str()
            .unwrap()
            .ends_with(&format!("state={}", w.acme))
    );
    w.audit(w.acme, "integration.github_install_start").await;
    let unlinked = w.ok(&w.mika, "UnlinkGitHub", json!({})).await;
    assert_eq!(unlinked["unlinkGitHub"]["user"]["githubLogin"], Value::Null);
    w.audit(w.acme, "integration.github_unlink").await;
    w.audit(mika_tenant, "integration.github_unlink").await;
    assert_eq!(
        w.error_code(&w.mika, "UnlinkGitHub", json!({})).await,
        "CONFLICT"
    );

    let exported = w
        .ok(
            &w.sota,
            "ExportAuditLog",
            json!({ "tenantId": w.acme, "filter": { "actorKind": "MEMBER", "action": "tenant.plan_change" } }),
        )
        .await;
    let url = exported["exportAuditLog"]["url"].as_str().unwrap();
    assert!(url.starts_with("https://exports.test/audit-"));
    let (_, body) = w.recorder.exports.lock().unwrap()[0].clone();
    let lines = String::from_utf8(body).unwrap();
    let lines = lines.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    let line: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(line["action"], "tenant.plan_change");
    assert_eq!(line["actorName"], "Mika");
    w.audit(w.acme, "audit_log.export").await;
    let filtered = w
        .ok(
            &w.sota,
            "AuditLog",
            json!({ "tenant": "acme", "filter": { "memberId": w.sota.member_id } }),
        )
        .await;
    let actions = filtered["tenant"]["auditEvents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(actions.len(), 3);
    assert!(actions.contains(&"audit_log.export".to_string()));
    assert!(actions.contains(&"integration.slack_connect_start".to_string()));
    assert!(actions.contains(&"integration.github_install_start".to_string()));

    let suspended = w
        .ok(
            &w.operator,
            "SuspendTenant",
            json!({ "tenantId": w.acme, "reason": "Crypto mining" }),
        )
        .await;
    assert_eq!(suspended["suspendTenant"]["state"], "SUSPENDED");
    let tenant: Tenant = w.get(&keys::tenant_key(w.acme)).await.unwrap();
    assert_eq!(tenant.status, "suspended");
    assert_eq!(tenant.suspended_reason.as_deref(), Some("Crypto mining"));
    assert_eq!(
        w.error_code(
            &w.operator,
            "SuspendTenant",
            json!({ "tenantId": w.acme, "reason": "again" })
        )
        .await,
        "CONFLICT"
    );
    assert_eq!(
        suspended["suspendTenant"]["suspendedReason"],
        "Crypto mining"
    );
    let audit = w.audit(w.acme, "tenant.suspend").await;
    assert_eq!(audit.actor_kind, "operator");
    assert_eq!(audit.actor_ref, w.operator.user_id.to_string());
    let pending = w.ok(&w.mika, "Home", json!({ "tenant": "acme" })).await;
    assert_eq!(pending["tenant"]["id"], w.acme.to_string());
    let unsuspended = w
        .ok(
            &w.operator,
            "UnsuspendTenant",
            json!({ "tenantId": w.acme }),
        )
        .await;
    assert_eq!(unsuspended["unsuspendTenant"]["state"], "ACTIVE");
    let tenant: Tenant = w.get(&keys::tenant_key(w.acme)).await.unwrap();
    assert_eq!(tenant.status, "active");
    assert_eq!(tenant.suspended_reason, None);
    assert_eq!(
        unsuspended["unsuspendTenant"]["suspendedReason"],
        Value::Null
    );
    assert_eq!(
        w.audit(w.acme, "tenant.unsuspend").await.actor_kind,
        "operator"
    );
}

async fn pending_tenant(w: &World, name: &str, plan_id: &str, session_id: &str) -> Uuid {
    create_tenant(
        &w.store,
        &NewTenant {
            id: Uuid::new_v4(),
            name,
            plan_id,
            user_id: w.ren.user_id,
            checkout: Some(PendingSession {
                session_id,
                expires_at: w.now + Duration::hours(1),
            }),
            origin: AuditOrigin::Web,
            source_ip: None,
            now: w.now,
        },
    )
    .await
    .unwrap()
    .tenant_id
}

#[tokio::test]
async fn a_linked_github_account_has_one_free_tenant_until_it_leaves_free() {
    let w = World::new().await;
    let free_tenant = json!({ "input": { "name": "Mika Free", "planId": "free" } });
    let created = w.ok(&w.mika, "CreateTenant", free_tenant).await;
    let free_id: Uuid = created["createTenant"]["tenant"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let mika: User = w.get(&keys::user_key(w.mika.user_id)).await.unwrap();
    let github_user_id = mika.github_user_id.unwrap();
    w.ok(&w.mika, "UnlinkGitHub", json!({})).await;
    w.dynamo
        .modify::<User>(&keys::user_key(w.sota.user_id), |user| {
            user.github_user_id = Some(github_user_id);
        })
        .await;
    let sota_free = json!({ "input": { "name": "Sota Free", "planId": "free" } });
    assert_eq!(
        w.error_code(&w.sota, "CreateTenant", sota_free.clone())
            .await,
        "CONFLICT"
    );

    *w.recorder.plan_change.lock().unwrap() = Some(PlanChange::Applied);
    w.ok(
        &w.mika,
        "ChangePlan",
        json!({ "tenantId": free_id, "planId": "pro" }),
    )
    .await;
    assert!(
        w.get::<Guard>(&free_github_tenant_key(github_user_id))
            .await
            .is_none()
    );
    let sota = w.ok(&w.sota, "CreateTenant", sota_free).await;
    let sota_id: Uuid = sota["createTenant"]["tenant"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let guard: Guard = w
        .get(&free_github_tenant_key(github_user_id))
        .await
        .unwrap();
    assert_eq!(guard.target, sota_id);
}

#[tokio::test]
async fn changing_to_free_holds_the_free_slot_before_the_subscription_is_cancelled() {
    let w = World::new().await;
    *w.recorder.plan_change.lock().unwrap() = Some(PlanChange::Applied);
    let slot = free_tenant_key(w.mika.user_id);
    let other = Uuid::new_v4();
    w.dynamo
        .put(&Guard::of_tenant(slot.clone(), other, other))
        .await;
    let to_free = json!({ "tenantId": w.acme, "planId": "free" });
    assert_eq!(
        w.error_code(&w.mika, "ChangePlan", to_free.clone()).await,
        "CONFLICT"
    );
    assert!(w.recorder.plan_change_calls.lock().unwrap().is_empty());

    let mut tx = w.store.tx(other);
    tx.delete(&slot).unwrap();
    tx.commit().await.unwrap();
    *w.recorder.plan_change_fails.lock().unwrap() = true;
    assert!(
        !w.run(&w.mika, "ChangePlan", to_free.clone())
            .await
            .errors
            .is_empty()
    );
    assert!(w.get::<Guard>(&slot).await.is_none());
    let tenant: Tenant = w.get(&keys::tenant_key(w.acme)).await.unwrap();
    assert_eq!(tenant.plan_id, "pro");

    *w.recorder.plan_change_fails.lock().unwrap() = false;
    *w.recorder.free_tenant_race.lock().unwrap() = Some((w.mika.user_id, other));
    let changed = w.ok(&w.mika, "ChangePlan", to_free).await;
    assert_eq!(changed["changePlan"]["tenant"]["plan"]["id"], "free");
    assert_eq!(
        *w.recorder.free_tenant_race_won.lock().unwrap(),
        Some(false)
    );
    let guard: Guard = w.get(&slot).await.unwrap();
    assert_eq!(guard.target, w.acme);
    assert_eq!(w.recorder.plan_change_calls.lock().unwrap().len(), 2);
    w.audit(w.acme, "tenant.plan_change").await;
}

#[tokio::test]
async fn changing_to_free_after_the_subscription_webhook_keeps_the_free_plan() {
    let w = World::new().await;
    *w.recorder.plan_change.lock().unwrap() = Some(PlanChange::Applied);
    *w.recorder.ended_first.lock().unwrap() = Some(w.acme);
    let changed = w
        .ok(
            &w.mika,
            "ChangePlan",
            json!({ "tenantId": w.acme, "planId": "free" }),
        )
        .await;
    assert_eq!(changed["changePlan"]["tenant"]["plan"]["id"], "free");
    let tenant: Tenant = w.get(&keys::tenant_key(w.acme)).await.unwrap();
    assert_eq!(
        (tenant.plan_id.as_str(), tenant.tasks_this_period),
        ("free", 7)
    );
    let guard: Guard = w.get(&free_tenant_key(w.mika.user_id)).await.unwrap();
    assert_eq!(guard.target, w.acme);
}

#[tokio::test]
async fn changing_to_free_activates_a_pending_tenant_and_resets_the_period() {
    let w = World::new().await;
    *w.recorder.plan_change.lock().unwrap() = Some(PlanChange::Applied);
    let pending = pending_tenant(&w, "Ren Pro", "pro", "cs_ren").await;
    w.dynamo
        .modify::<Tenant>(&keys::tenant_key(pending), |tenant| {
            tenant.tasks_this_period = 2;
            tenant.workspace_seconds_this_period = 50;
            tenant.period_start = w.now - Duration::days(9);
        })
        .await;
    let changed = w
        .ok(
            &w.ren,
            "ChangePlan",
            json!({ "tenantId": pending, "planId": "free" }),
        )
        .await;
    assert_eq!(changed["changePlan"]["tenant"]["plan"]["id"], "free");
    let tenant: Tenant = w.get(&keys::tenant_key(pending)).await.unwrap();
    assert_eq!(
        (
            tenant.status.as_str(),
            tenant.suspended_reason.as_deref(),
            tenant.tasks_this_period,
            tenant.workspace_seconds_this_period,
            tenant.period_start
        ),
        ("active", None, 0, 0, w.now)
    );
    let guard: Guard = w.get(&free_tenant_key(w.ren.user_id)).await.unwrap();
    assert_eq!(guard.target, pending);
    assert!(
        pending_checkout(&w.store, w.ren.user_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        w.error_code(
            &w.ren,
            "CreateTenant",
            json!({ "input": { "name": "Ren Free", "planId": "free" } })
        )
        .await,
        "CONFLICT"
    );

    let other = pending_tenant(&w, "Ren Max", "max", "cs_ren_2").await;
    assert_eq!(
        w.error_code(
            &w.ren,
            "ChangePlan",
            json!({ "tenantId": other, "planId": "free" })
        )
        .await,
        "CONFLICT"
    );
    let still: Tenant = w.get(&keys::tenant_key(other)).await.unwrap();
    assert_eq!(
        (still.status.as_str(), still.plan_id.as_str()),
        ("suspended", "max")
    );

    w.dynamo
        .modify::<Tenant>(&keys::tenant_key(w.acme), |tenant| {
            tenant.tasks_this_period = 9;
        })
        .await;
    w.ok(
        &w.mika,
        "ChangePlan",
        json!({ "tenantId": w.acme, "planId": "free" }),
    )
    .await;
    let acme: Tenant = w.get(&keys::tenant_key(w.acme)).await.unwrap();
    assert_eq!(
        (
            acme.plan_id.as_str(),
            acme.tasks_this_period,
            acme.period_start
        ),
        ("free", 0, w.now)
    );
}

#[tokio::test]
async fn a_portal_redirect_keeps_the_plan_and_returns_the_portal_url() {
    let w = World::new().await;
    *w.recorder.plan_change.lock().unwrap() = Some(PlanChange::Portal {
        url: "https://billing.stripe.test/settle".into(),
    });
    let changed = w
        .ok(
            &w.mika,
            "ChangePlan",
            json!({ "tenantId": w.acme, "planId": "max" }),
        )
        .await;
    assert_eq!(
        changed["changePlan"]["checkoutUrl"],
        "https://billing.stripe.test/settle"
    );
    assert_eq!(changed["changePlan"]["tenant"]["plan"]["id"], "pro");
    w.audit(w.acme, "billing.portal_open").await;
}

#[tokio::test]
async fn a_disabled_config_shows_why_and_refuses_to_sync() {
    let w = World::new().await;
    let reason = "acme/sanchoris-config was removed from the GitHub App installation";
    let configs = w.ok(&w.sota, "Configs", json!({ "tenant": "acme" })).await;
    let active = &configs["tenant"]["configs"][0];
    assert_eq!(active["status"], "ACTIVE");
    assert_eq!(active["syncError"], Value::Null);
    assert_eq!(
        active["repositoryId"],
        common::repository_id("acme/sanchoris-config").to_string()
    );

    w.dynamo
        .modify::<Config>(&keys::config_key(w.acme, w.config_id), |config| {
            config.status = CONFIG_DISABLED.into();
            config.sync_error = Some(reason.into());
        })
        .await;
    let configs = w.ok(&w.sota, "Configs", json!({ "tenant": "acme" })).await;
    let disabled = &configs["tenant"]["configs"][0];
    assert_eq!(disabled["status"], "DISABLED");
    assert_eq!(disabled["syncError"], reason);
    assert_eq!(
        w.error_code(&w.sota, "SyncConfig", json!({ "configId": w.config_id }))
            .await,
        "BAD_USER_INPUT"
    );
    assert!(w.recorder.syncs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_verified_sancode_email_creates_admin_tenants_that_billing_cannot_change() {
    let w = World::new().await;

    let ren_page = w.ok(&w.ren, "CreateTenantPage", json!({})).await;
    let ren_plans = ren_page["viewer"]["newTenantPlans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|plan| plan["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ren_plans, ["free", "starter", "pro", "max"]);
    let aya_page = w.ok(&w.aya, "CreateTenantPage", json!({})).await;
    assert_eq!(
        aya_page["viewer"]["newTenantPlans"],
        json!([{
            "id": "admin",
            "name": "Admin",
            "priceUsd": 0,
            "taskLimit": null,
            "workspaceHourLimit": null,
            "limitPeriod": "TOTAL",
            "memberLimit": null,
            "requiresPayment": false,
        }])
    );

    assert_eq!(
        w.error_code(
            &w.ren,
            "CreateTenant",
            json!({ "input": { "name": "Ren Admin", "planId": "admin" } })
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.error_code(
            &w.aya,
            "CreateTenant",
            json!({ "input": { "name": "Aya Free", "planId": "free" } })
        )
        .await,
        "BAD_USER_INPUT"
    );
    assert_eq!(
        w.error_code(
            &w.aya,
            "CreateTenant",
            json!({ "input": { "name": "Aya Max", "planId": "max" } })
        )
        .await,
        "BAD_USER_INPUT"
    );

    let created = w
        .ok(
            &w.aya,
            "CreateTenant",
            json!({ "input": { "name": "Aya Lab", "planId": "admin" } }),
        )
        .await;
    assert_eq!(created["createTenant"]["checkoutUrl"], Value::Null);
    let slug = created["createTenant"]["tenant"]["slug"].as_str().unwrap();
    let tenant_id: Uuid = created["createTenant"]["tenant"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let row = w.get::<Tenant>(&keys::tenant_key(tenant_id)).await.unwrap();
    assert_eq!(
        (
            row.plan_id.as_str(),
            row.status.as_str(),
            row.stripe_customer_id
        ),
        ("admin", "active", None)
    );
    assert!(
        w.get::<Guard>(&free_tenant_key(w.aya.user_id))
            .await
            .is_none()
    );
    let second = w
        .ok(
            &w.aya,
            "CreateTenant",
            json!({ "input": { "name": "Aya Lab", "planId": "admin" } }),
        )
        .await;
    assert_eq!(second["createTenant"]["tenant"]["slug"], "aya-lab-2");

    let billing = w.ok(&w.aya, "Billing", json!({ "tenant": slug })).await;
    assert_eq!(
        billing["tenant"]["plan"],
        json!({
            "id": "admin",
            "name": "Admin",
            "priceUsd": 0,
            "memberLimit": null,
            "concurrencyLimit": 10,
            "inboundEventsPerMinute": 300,
            "taskLimit": null,
            "workspaceHourLimit": null,
            "limitPeriod": "TOTAL",
            "isAdmin": true,
        })
    );
    assert_eq!(billing["tenant"]["usage"]["periodEnd"], Value::Null);
    let for_sale = billing["plans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|plan| plan["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(for_sale, ["free", "starter", "pro", "max"]);

    for plan in ["free", "pro"] {
        assert_eq!(
            w.error_code(
                &w.aya,
                "ChangePlan",
                json!({ "tenantId": tenant_id, "planId": plan })
            )
            .await,
            "CONFLICT"
        );
    }
    assert_eq!(
        w.error_code(
            &w.aya,
            "OpenBillingPortal",
            json!({ "tenantId": tenant_id })
        )
        .await,
        "CONFLICT"
    );
    assert_eq!(
        w.error_code(
            &w.mika,
            "ChangePlan",
            json!({ "tenantId": w.acme, "planId": "admin" })
        )
        .await,
        "FORBIDDEN"
    );
    assert_eq!(
        w.get::<Tenant>(&keys::tenant_key(tenant_id))
            .await
            .unwrap()
            .plan_id,
        "admin"
    );

    for email in ["one@acme.dev", "two@acme.dev", "three@acme.dev"] {
        w.ok(
            &w.aya,
            "InviteMember",
            json!({ "input": { "tenantId": tenant_id, "email": email, "role": "MEMBER" } }),
        )
        .await;
    }
}

#[tokio::test]
async fn a_sancode_member_joining_a_tenant_leaves_its_plan() {
    let w = World::new().await;
    let invitation = w
        .ok(
            &w.mika,
            "InviteMember",
            json!({ "input": { "tenantId": w.acme, "email": "aya@sancode.dev", "role": "ADMIN" } }),
        )
        .await;
    let joined = w
        .ok(
            &w.aya,
            "AcceptInvitation",
            json!({ "invitationId": invitation["inviteMember"]["id"] }),
        )
        .await;
    assert_eq!(joined["acceptInvitation"]["role"], "ADMIN");

    let tenant = w.get::<Tenant>(&keys::tenant_key(w.acme)).await.unwrap();
    assert_eq!(tenant.plan_id, "pro");
    let slug = tenant.slug;
    let billing = w.ok(&w.aya, "Billing", json!({ "tenant": slug })).await;
    assert_eq!(billing["tenant"]["plan"]["isAdmin"], false);
    assert_eq!(billing["tenant"]["plan"]["taskLimit"], 50);
}

#[tokio::test]
async fn chat_messages_and_resumes_over_the_receive_limit_are_rate_limited() {
    let w = World::new().await;
    let plan_id = w
        .get::<Tenant>(&keys::tenant_key(w.acme))
        .await
        .unwrap()
        .plan_id;
    let limit = sanchoris_core::db::plan(&w.store, &plan_id)
        .await
        .unwrap()
        .inbound_events_per_minute;
    let start = window_start(w.now);
    w.dynamo
        .put(&ReceiveRate {
            tenant_id: w.acme,
            window_start: start,
            received: limit,
            dropped: 0,
            expires_at: start + Duration::days(1),
        })
        .await;

    let response = w
        .run(
            &w.ken,
            "SendChatMessage",
            json!({ "input": { "tenantId": w.acme, "sessionName": "all", "text": "Flood" } }),
        )
        .await;
    let extensions = response.errors[0].extensions.as_ref().unwrap();
    assert_eq!(
        extensions.get("code").unwrap().to_string(),
        "\"RATE_LIMITED\""
    );
    assert_eq!(
        extensions.get("limit").unwrap().to_string(),
        limit.to_string()
    );
    assert!(response.errors[0].message.starts_with(&format!(
        "receive limit: the tenant already received {limit} events"
    )));
    assert!(
        w.all::<ChatMessage>()
            .await
            .iter()
            .all(|message| message.text != "Flood")
    );
    assert_eq!(
        w.error_code(
            &w.ken,
            "ResumeTask",
            json!({ "input": { "taskId": w.issue_task, "message": "Go on." } }),
        )
        .await,
        "RATE_LIMITED"
    );
    assert!(
        w.all::<RunEventItem>()
            .await
            .iter()
            .all(|event| event.kind != "human_reply")
    );
    let rate = w
        .get::<ReceiveRate>(&keys::receive_rate_key(w.acme, start))
        .await
        .unwrap();
    assert_eq!((rate.received, rate.dropped), (limit, 2));
    assert_eq!(
        w.audit(w.acme, "tenant.receive_limit").await.metadata["source"],
        "web"
    );
}

#[tokio::test]
async fn a_snoozed_task_leaves_the_needs_you_lists_until_the_snooze_ends() {
    let w = World::new().await;
    w.dynamo
        .modify::<SkillRunItem>(&keys::skill_run_key(w.acme, w.issue_task, 1), |run| {
            run.status = SkillRunStatus::Waiting;
            run.finished_at = None;
        })
        .await;
    let needs_you = || async {
        w.ok(
            &w.mika,
            "Tasks",
            json!({ "tenant": "acme", "filter": { "statuses": ["NEEDS_YOU"], "snoozed": false } }),
        )
        .await["tenant"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| task["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let skill_needs_you = || async {
        w.ok(
            &w.mika,
            "Skill",
            json!({ "tenant": "acme", "name": "open-pr-green-ci" }),
        )
        .await["tenant"]["skill"]["states"]
            .as_array()
            .unwrap()
            .iter()
            .map(|state| state["activity"]["needsYou"].as_i64().unwrap())
            .sum::<i64>()
    };
    let issue_task = w.issue_task.to_string();
    assert!(needs_you().await.contains(&issue_task));
    let counted = skill_needs_you().await;
    assert_eq!(counted, 1);

    let until = w.now + Duration::hours(3);
    w.ok(
        &w.mika,
        "SnoozeTask",
        json!({ "taskId": w.issue_task, "until": iso(until) }),
    )
    .await;
    assert!(!needs_you().await.contains(&issue_task));
    assert_eq!(skill_needs_you().await, 0);
    let snoozed = w
        .ok(
            &w.mika,
            "Tasks",
            json!({ "tenant": "acme", "filter": { "snoozed": true } }),
        )
        .await;
    let snoozed = snoozed["tenant"]["tasks"].as_array().unwrap();
    assert_eq!(snoozed.len(), 1);
    assert_eq!(snoozed[0]["id"], issue_task);
    assert_eq!(snoozed[0]["snoozedUntil"], iso(until));
    let ends = w
        .messages()
        .await
        .into_iter()
        .filter_map(|message| match message {
            Relayed::Send {
                envelope,
                deliver_at,
            } => match envelope.job {
                Job::TaskSnoozeEnd { task_id, until, .. } => Some((task_id, until, deliver_at)),
                _ => None,
            },
            Relayed::Cancel { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(ends, vec![(w.issue_task, until, Some(until))]);

    *w.clock.0.lock().unwrap() = until;
    assert!(needs_you().await.contains(&issue_task));
    assert_eq!(skill_needs_you().await, counted);
    let task = w
        .ok(
            &w.mika,
            "Task",
            json!({ "tenant": "acme", "id": w.issue_task }),
        )
        .await;
    assert_eq!(task["tenant"]["task"]["snoozedUntil"], Value::Null);
}
