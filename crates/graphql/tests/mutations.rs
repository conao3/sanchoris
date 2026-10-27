mod common;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use common::World;
use sanchoris_graphql::{ConfigProblem, PlanChange};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

fn iso(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
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
    let reply = sqlx::query(
        "SELECT type, payload, seq FROM sanchoris.run_events WHERE run_id = $1 AND type = 'human_reply'",
    )
    .bind(w.issue_run)
    .fetch_one(&w.admin)
    .await
    .unwrap();
    assert_eq!(
        reply.get::<Value, _>("payload"),
        json!({ "member": "Ken", "text": "Use the staging database." })
    );
    assert_eq!(reply.get::<Option<i64>, _>("seq"), None);
    let audit = w.audit(w.acme, "task.resume").await;
    assert_eq!(audit.actor_kind, "member");
    assert_eq!(audit.actor_ref, w.ken.member_id.unwrap().to_string());
    assert_eq!(audit.target_ref, w.issue_task.to_string());
    let last_active: Option<DateTime<Utc>> = w
        .scalar(
            "SELECT last_active_at FROM sanchoris.members WHERE id = $1",
            w.ken.member_id.unwrap(),
        )
        .await;
    assert!(last_active.is_some());

    let until = w.now + Duration::hours(3);
    let snoozed = w
        .ok(
            &w.mika,
            "SnoozeTask",
            json!({ "taskId": w.issue_task, "until": iso(until) }),
        )
        .await;
    assert_eq!(snoozed["snoozeTask"]["snoozedUntil"], iso(until));
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
    let shutdown: Value = w
        .scalar(
            "SELECT payload FROM sanchoris.run_outbox WHERE run_id = $1 AND type = 'shutdown'",
            w.chat_run,
        )
        .await;
    assert_eq!(shutdown, json!({ "reason": "Ken stopped the session" }));
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
    let run_status: String = w
        .scalar(
            "SELECT status FROM sanchoris.runs WHERE id = $1",
            w.issue_run,
        )
        .await;
    assert_eq!(run_status, "completed");
    let error: Option<String> = w
        .scalar(
            "SELECT error FROM sanchoris.tasks WHERE id = $1",
            w.issue_task,
        )
        .await;
    assert_eq!(error, None);
    w.audit(w.acme, "task.stop").await;
    assert_eq!(
        w.error_code(&w.mika, "StopTask", json!({ "taskId": w.issue_task }))
            .await,
        "CONFLICT"
    );
}

#[tokio::test]
async fn chat_messages_start_threads_and_continue_them() {
    let w = World::new().await;

    let started = w
        .ok(
            &w.ken,
            "SendChatMessage",
            json!({ "input": { "tenantId": w.acme, "scope": "all", "text": "Add a dark mode" } }),
        )
        .await;
    let task_id: Uuid = started["sendChatMessage"]["task"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_ne!(task_id, w.chat_task);
    assert_eq!(
        started["sendChatMessage"]["message"]["text"],
        "Add a dark mode"
    );
    let event: Value = sqlx::query_scalar(
        "SELECT event FROM sanchoris.inbound_events WHERE tenant_id = $1 AND event->'detail'->>'text' = 'Add a dark mode'",
    )
    .bind(w.acme)
    .fetch_one(&w.admin)
    .await
    .unwrap();
    assert_eq!(event["source"], "web");
    assert_eq!(event["detail-type"], "Message");
    assert_eq!(event["detail"]["scope"], "all");
    assert_eq!(event["detail"]["user"], "Ken");
    assert_eq!(
        event["detail"]["member_id"],
        w.ken.member_id.unwrap().to_string()
    );
    let requester: Option<Uuid> = w
        .scalar(
            "SELECT requested_by_member_id FROM sanchoris.tasks WHERE id = $1",
            task_id,
        )
        .await;
    assert_eq!(requester, w.ken.member_id);
    let audit = w.audit(w.acme, "chat.message_send").await;
    assert_eq!(audit.target_kind, "chat");

    let chat = w
        .ok(
            &w.ken,
            "Chat",
            json!({ "tenant": "acme", "task": task_id, "withTask": true }),
        )
        .await;
    assert_eq!(
        chat["tenant"]["task"]["messages"][0]["text"],
        "Add a dark mode"
    );
    assert_eq!(
        chat["tenant"]["task"]["requester"]["member"]["user"]["name"],
        "Ken"
    );

    let continued = w
        .ok(
            &w.mika,
            "SendChatMessage",
            json!({ "input": { "tenantId": w.acme, "taskId": w.chat_task, "text": "Any update?" } }),
        )
        .await;
    assert_eq!(
        continued["sendChatMessage"]["task"]["id"],
        w.chat_task.to_string()
    );
    let thread: String = sqlx::query_scalar(
        "SELECT event->'detail'->>'thread' FROM sanchoris.inbound_events WHERE event->'detail'->>'text' = 'Any update?'",
    )
    .fetch_one(&w.admin)
    .await
    .unwrap();
    assert_eq!(thread, "thread-1");

    assert_eq!(
        w.error_code(
            &w.mika,
            "SendChatMessage",
            json!({ "input": { "tenantId": w.acme, "scope": "shop", "text": "hi" } }),
        )
        .await,
        "BAD_USER_INPUT"
    );
    assert_eq!(
        w.error_code(
            &w.mika,
            "SendChatMessage",
            json!({ "input": { "tenantId": w.acme, "taskId": w.issue_task, "text": "hi" } }),
        )
        .await,
        "BAD_USER_INPUT"
    );
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
    let jobs: i64 = w
        .scalar(
            "SELECT count(*) FROM sanchoris.due_jobs WHERE ref_id = $1",
            w.nightly_trigger,
        )
        .await;
    assert_eq!(jobs, 0);
    w.audit(w.acme, "trigger.disable").await;
    w.ok(
        &w.sota,
        "SetTriggerState",
        json!({ "triggerId": w.nightly_trigger, "state": "ENABLED" }),
    )
    .await;
    let state: String = w
        .scalar(
            "SELECT state FROM sanchoris.triggers WHERE id = $1",
            w.nightly_trigger,
        )
        .await;
    assert_eq!(state, "ENABLED");
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
    let changes: Value = sqlx::query_scalar(
        "SELECT changed_definitions FROM sanchoris.change_requests WHERE pr_number = 42",
    )
    .fetch_one(&w.admin)
    .await
    .unwrap();
    assert_eq!(
        changes,
        json!([{ "kind": "trigger", "name": "labeled-issue-2" }])
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
    let branch: String = w
        .scalar(
            "SELECT branch FROM sanchoris.configs WHERE id = $1",
            config_id,
        )
        .await;
    assert_eq!(branch, "release");
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
    let row = sqlx::query(
        "SELECT member_id, provider, kind, account, encrypted_value, kms_key_id FROM sanchoris.credentials WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&w.admin)
    .await
    .unwrap();
    assert_eq!(row.get::<Option<Uuid>, _>("member_id"), w.ken.member_id);
    assert_eq!(row.get::<String, _>("provider"), "claude");
    assert_eq!(row.get::<String, _>("kind"), "api_key");
    assert_eq!(row.get::<String, _>("account"), "sk-ant-…9f2c");
    assert_eq!(
        row.get::<String, _>("kms_key_id"),
        sanchoris_graphql::LOCAL_KEY_ID
    );
    let encrypted: Vec<u8> = row.get("encrypted_value");
    assert!(!encrypted.windows(key.len()).any(|w| w == key.as_bytes()));
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
            json!({ "input": { "tenantId": w.acme, "name": "ken-key", "provider": "OPENAI", "key": "not-a-real-key", "scope": "ORGANIZATION" } }),
        )
        .await,
        "CONFLICT"
    );
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
    let kind: String = w
        .scalar(
            "SELECT kind FROM sanchoris.credentials WHERE id = $1",
            org_id,
        )
        .await;
    assert_eq!(kind, "bedrock");

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
    let left: i64 = w
        .scalar(
            "SELECT count(*) FROM sanchoris.credentials WHERE tenant_id = $1",
            w.acme,
        )
        .await;
    assert_eq!(left, 3);
    let deletions: i64 = w
        .scalar(
            "SELECT count(*) FROM sanchoris.audit_events WHERE tenant_id = $1 AND action = 'credential.delete'",
            w.acme,
        )
        .await;
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
    let left: i64 = w
        .scalar(
            "SELECT count(*) FROM sanchoris.members WHERE id = $1",
            ren_member,
        )
        .await;
    assert_eq!(left, 0);
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
            json!({ "input": { "name": "Ren Labs", "planId": "starter" } }),
        )
        .await;
    assert_eq!(paid["createTenant"]["tenant"], Value::Null);
    assert_eq!(
        paid["createTenant"]["checkoutUrl"],
        "https://checkout.stripe.test/starter/Ren Labs"
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
    let role: String = w
        .scalar(
            "SELECT role FROM sanchoris.members WHERE tenant_id = $1",
            tenant_id,
        )
        .await;
    assert_eq!(role, "owner");
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

    let github = w.ok(&w.mika, "ConnectGitHub", json!({})).await;
    assert!(
        github["connectGitHub"]["url"]
            .as_str()
            .unwrap()
            .contains(&w.mika.user_id.to_string())
    );
    w.audit(w.acme, "integration.github_link_start").await;
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
    assert_eq!(actions.len(), 2);
    assert!(actions.contains(&"audit_log.export".to_string()));
    assert!(actions.contains(&"integration.slack_connect_start".to_string()));

    let suspended = w
        .ok(
            &w.operator,
            "SuspendTenant",
            json!({ "tenantId": w.acme, "reason": "Crypto mining" }),
        )
        .await;
    assert_eq!(suspended["suspendTenant"]["state"], "SUSPENDED");
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
    assert_eq!(
        unsuspended["unsuspendTenant"]["suspendedReason"],
        Value::Null
    );
    assert_eq!(
        w.audit(w.acme, "tenant.unsuspend").await.actor_kind,
        "operator"
    );
}
