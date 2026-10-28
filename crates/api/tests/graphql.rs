mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use base64::Engine;
use chrono::{Duration, Utc};
use reqwest::StatusCode;
use sanchoris_api::signing::HmacKey;
use sanchoris_core::channel::{OutboxType, enqueue_outbox};
use sanchoris_core::items::{
    AuditEvent, ConfigRevision, Credential, Integration, Run, Task, Tenant, User,
};
use sanchoris_core::keys;
use sanchoris_core::model::TaskStatus;
use sanchoris_core::testing::TestDynamo;
use sanchoris_graphql::AesGcmEncryptor;
use serde_json::{Value, json};
use support::fake_github::{CONFIG_REPO, CONFIG_REPO_ID, CONFIG_SHA, FakeGitHub};
use support::{
    Api, CONFIG_YAML, CREDENTIAL_KEY, Driving, FRONTEND_INDEX, FRONTEND_SCRIPT, INSTALLATION,
    STRIPE_SECRET, Seeded, WEBHOOK_SECRET, Workspaces, bare_repo, eventually, github_signature,
    seed, start_api,
};
use uuid::Uuid;

const CHAT_TRIGGER: &str = r#"  chat-all:
    State: ENABLED
    EventPattern:
      source: [web]
      detail-type: [Message]
      detail:
        scope: [all]
    Start:
      Session: shop
      Instruction: 'Answer the question from the chat.'
      Skills: []
      RequiredSkills: []
      MaxNudges: 0
      Limits: { Turns: 60, CostUsd: 10, TimeoutSeconds: 86400, HeartbeatSeconds: 900, Children: { MaxDepth: 1, MaxCount: 5 } }
"#;

const SAFE_CODES: [&str; 4] = ["BAD_USER_INPUT", "CONFLICT", "NOT_FOUND", "FORBIDDEN"];
const PAID_PLANS: [&str; 3] = ["starter", "pro", "max"];

fn operations() -> String {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../apps/frontend/src/graphql"
    );
    let mut paths = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "graphql")
                && path
                    .file_name()
                    .is_some_and(|name| name != "schema.graphql")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

fn operation_names(document: &str) -> BTreeSet<String> {
    document
        .lines()
        .filter_map(|line| {
            line.strip_prefix("query ")
                .or_else(|| line.strip_prefix("mutation "))
        })
        .map(|rest| rest.split(['(', ' ', '{']).next().unwrap().to_string())
        .collect()
}

struct World {
    api: Api,
    github: Arc<FakeGitHub>,
    seeded: Seeded,
    dynamo: TestDynamo,
    document: String,
    executed: Mutex<BTreeSet<String>>,
    owner: String,
    slug: String,
    _dir: tempfile::TempDir,
}

impl World {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let git_root = dir.path().join("git");
        bare_repo(&git_root, "acme/ops");
        let config = format!("{CONFIG_YAML}{CHAT_TRIGGER}");
        let github = FakeGitHub::new(
            &git_root,
            BTreeMap::from([("sanchoris.yaml".to_string(), config)]),
        );
        let dynamo = TestDynamo::start().await;
        let seeded = seed(&dynamo).await;
        for plan in PAID_PLANS {
            dynamo
                .set_stripe_price(plan, &format!("price_{plan}"))
                .await;
        }
        let api = start_api(
            &dynamo,
            github.clone(),
            dir.path(),
            Workspaces::None,
            Driving::QueueOnly,
        )
        .await;
        let slug = Self::tenant_of(&dynamo, seeded.tenant.tenant_id).await.slug;
        Self {
            api,
            github,
            owner: seeded.member_token(),
            seeded,
            dynamo,
            document: operations(),
            executed: Mutex::new(BTreeSet::new()),
            slug,
            _dir: dir,
        }
    }

    async fn tenant_of(dynamo: &TestDynamo, tenant_id: Uuid) -> Tenant {
        dynamo
            .all::<Tenant>()
            .await
            .into_iter()
            .find(|tenant| tenant.id == tenant_id)
            .unwrap()
    }

    fn tenant(&self) -> Uuid {
        self.seeded.tenant.tenant_id
    }

    async fn post(&self, token: Option<&str>, body: &Value) -> reqwest::Response {
        let mut request = self
            .api
            .http
            .post(format!("{}/api/graphql", self.api.url))
            .json(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.unwrap()
    }

    async fn run(&self, token: &str, operation: &str, variables: Value) -> Value {
        self.executed.lock().unwrap().insert(operation.to_string());
        let response = self
            .post(
                Some(token),
                &json!({ "query": self.document, "operationName": operation, "variables": variables }),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK, "{operation}");
        let body: Value = response.json().await.unwrap();
        for error in body["errors"].as_array().into_iter().flatten() {
            let code = error["extensions"]["code"].as_str().unwrap_or("INTERNAL");
            assert!(
                SAFE_CODES.contains(&code),
                "{operation} failed internally: {error}"
            );
        }
        body
    }

    async fn ok(&self, token: &str, operation: &str, variables: Value) -> Value {
        let body = self.run(token, operation, variables).await;
        assert!(
            body["errors"].is_null(),
            "{operation} failed: {}",
            body["errors"]
        );
        body["data"].clone()
    }

    async fn error(&self, token: &str, operation: &str, variables: Value) -> (String, String) {
        let body = self.run(token, operation, variables).await;
        let error = &body["errors"][0];
        assert!(error.is_object(), "{operation} succeeded: {}", body["data"]);
        (
            error["extensions"]["code"]
                .as_str()
                .unwrap_or("INTERNAL")
                .to_string(),
            error["message"].as_str().unwrap().to_string(),
        )
    }

    async fn audit_actions(&self, tenant: Uuid) -> Vec<String> {
        let mut events = self
            .dynamo
            .all::<AuditEvent>()
            .await
            .into_iter()
            .filter(|event| event.tenant_id == tenant)
            .collect::<Vec<_>>();
        events.sort_by_key(|event| (event.at, event.id));
        events.into_iter().map(|event| event.action).collect()
    }

    async fn stripe_webhook(&self, event: &Value) -> reqwest::Response {
        let body = serde_json::to_vec(event).unwrap();
        let t = Utc::now().timestamp();
        let mut signed = format!("{t}.").into_bytes();
        signed.extend_from_slice(&body);
        let signature = HmacKey::new(STRIPE_SECRET.as_bytes()).unwrap().hex(&signed);
        self.api
            .http
            .post(format!("{}/webhooks/stripe", self.api.url))
            .header("stripe-signature", format!("t={t},v1={signature}"))
            .body(body)
            .send()
            .await
            .unwrap()
    }

    async fn github_webhook(&self, event: &str, payload: &Value) -> Value {
        let body = serde_json::to_vec(payload).unwrap();
        let response = self
            .api
            .http
            .post(format!("{}/webhooks/github", self.api.url))
            .header("x-github-event", event)
            .header("x-github-delivery", Uuid::new_v4().to_string())
            .header(
                "x-hub-signature-256",
                github_signature(WEBHOOK_SECRET, &body),
            )
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response.json().await.unwrap()
    }

    async fn get(&self, url: &str) -> reqwest::Response {
        self.api.http.get(url).send().await.unwrap()
    }
}

fn query_param(url: &str, name: &str) -> String {
    reqwest::Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == name)
        .unwrap_or_else(|| panic!("{url} has no {name}"))
        .1
        .into_owned()
}

#[tokio::test]
async fn the_frontend_operations_run_over_http_against_the_real_ports() {
    let w = World::start().await;
    let owner = w.owner.as_str();
    let tenant = w.tenant();
    let slug = w.slug.as_str();

    let anonymous = w
        .post(None, &json!({ "query": "{ viewer { user { id } } }" }))
        .await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let shell = w.ok(owner, "Shell", json!({})).await;
    let memberships = shell["viewer"]["memberships"].as_array().unwrap();
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0]["tenant"]["slug"], slug);

    let synced = w
        .ok(
            owner,
            "SyncConfig",
            json!({ "configId": w.seeded.tenant.config_id }),
        )
        .await;
    assert!(synced["syncConfig"]["syncedAt"].is_string());
    let revision = w
        .dynamo
        .all::<ConfigRevision>()
        .await
        .into_iter()
        .find(|revision| {
            revision.config_id == w.seeded.tenant.config_id && revision.commit_sha == CONFIG_SHA
        })
        .expect("the sync stored the revision");
    assert_eq!(
        revision.validation_status, "valid",
        "{}",
        revision.validation_messages
    );

    let home = w.ok(owner, "Home", json!({ "tenant": slug })).await;
    assert_eq!(home["tenant"]["id"], tenant.to_string());
    let triggers = w.ok(owner, "Triggers", json!({ "tenant": slug })).await;
    let chat_trigger = triggers["tenant"]["triggers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|trigger| trigger["name"] == "chat-all")
        .expect("the synced config has the chat trigger")["id"]
        .as_str()
        .unwrap()
        .to_string();
    w.ok(owner, "Skills", json!({ "tenant": slug })).await;
    w.ok(
        owner,
        "Skill",
        json!({ "tenant": slug, "name": "open-pr-green-ci" }),
    )
    .await;
    w.ok(owner, "CreateTenantPage", json!({})).await;

    let sent = w
        .ok(
            owner,
            "SendChatMessage",
            json!({ "input": { "tenantId": tenant, "scope": "all", "text": "Which issues are open?" } }),
        )
        .await;
    let task_id = sent["sendChatMessage"]["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let task_uuid: Uuid = task_id.parse().unwrap();
    assert_eq!(
        sent["sendChatMessage"]["message"]["text"],
        "Which issues are open?"
    );
    w.ok(
        owner,
        "SendChatMessage",
        json!({ "input": { "tenantId": tenant, "taskId": task_id, "text": "Only the labeled ones." } }),
    )
    .await;
    let tasks = w
        .ok(owner, "Tasks", json!({ "tenant": slug, "filter": null }))
        .await;
    assert!(
        tasks["tenant"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == task_id.as_str())
    );
    let task = w
        .ok(owner, "Task", json!({ "tenant": slug, "id": task_id }))
        .await;
    assert_eq!(task["tenant"]["task"]["id"], task_id.as_str());
    let chat = w
        .ok(
            owner,
            "Chat",
            json!({ "tenant": slug, "task": task_id, "withTask": true }),
        )
        .await;
    assert!(chat.to_string().contains("Which issues are open?"));
    let run_id = w
        .dynamo
        .all::<Run>()
        .await
        .into_iter()
        .find(|run| run.task_id == task_uuid && run.number == 1)
        .expect("the chat task has a first run")
        .id;
    let mut tx = w.api.store.tx(tenant);
    enqueue_outbox(
        &mut tx,
        run_id,
        OutboxType::ChatPost,
        &json!({ "text": "Two issues are open.", "channel": { "source": "web", "scope": "all" } }),
        "chat:agent-answer",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let posted = eventually(
        "the agent's chat post",
        StdDuration::from_secs(10),
        || async {
            let chat = w
                .ok(
                    owner,
                    "Chat",
                    json!({ "tenant": slug, "task": task_id, "withTask": true }),
                )
                .await;
            chat["tenant"]["task"]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|message| message["text"] == "Two issues are open.")
                .cloned()
        },
    )
    .await;
    assert_eq!(posted["authorKind"], "AGENT");
    let run = w
        .ok(
            owner,
            "Run",
            json!({ "tenant": slug, "task": task_id, "number": 1 }),
        )
        .await;
    assert_eq!(run["tenant"]["task"]["run"]["id"], run_id.to_string());
    w.dynamo
        .modify::<Task>(&keys::task_key(tenant, task_uuid), |task| {
            task.status = TaskStatus::NeedsYou;
            task.wait_reason = None;
        })
        .await;
    w.ok(
        owner,
        "SnoozeTask",
        json!({ "taskId": task_id, "until": (Utc::now() + Duration::hours(2)).to_rfc3339() }),
    )
    .await;
    let resumed = w
        .ok(
            owner,
            "ResumeTask",
            json!({ "input": { "taskId": task_id, "message": "Go on." } }),
        )
        .await;
    assert_eq!(resumed["resumeTask"]["id"], task_id.as_str());
    let stopped = w.ok(owner, "StopRun", json!({ "runId": run_id })).await;
    assert_eq!(stopped["stopRun"]["id"], run_id.to_string());
    w.ok(owner, "StopTask", json!({ "taskId": task_id })).await;

    let disabled = w
        .ok(
            owner,
            "SetTriggerState",
            json!({ "triggerId": chat_trigger, "state": "DISABLED" }),
        )
        .await;
    assert_eq!(disabled["setTriggerState"]["state"], "DISABLED");
    w.ok(
        owner,
        "SetTriggerState",
        json!({ "triggerId": chat_trigger, "state": "ENABLED" }),
    )
    .await;

    let proposed = w
        .ok(
            owner,
            "ProposeTrigger",
            json!({ "input": {
                "tenantId": tenant, "configId": w.seeded.tenant.config_id,
                "templateId": "weekly-deps", "repository": "acme/shop-web",
            } }),
        )
        .await;
    let pulls = w.github.pulls();
    let pull = pulls.last().unwrap();
    assert_eq!(pull.repo, CONFIG_REPO);
    assert_eq!(pull.base, "main");
    assert!(pull.head.starts_with("sanchoris/trigger-"));
    assert_eq!(
        proposed["proposeTriggerFromTemplate"]["number"],
        json!(pull.number)
    );
    let update = w.github.state.lock().unwrap().file_updates.clone();
    assert_eq!(update.len(), 1);
    assert_eq!(update[0].path, "sanchoris.yaml");
    assert_eq!(update[0].branch, pull.head);
    assert!(update[0].previous_sha.is_some());
    let proposed_config: serde_yaml_ng::Value = serde_yaml_ng::from_str(&update[0].text).unwrap();
    assert!(proposed_config["Triggers"]["chat-all"].is_mapping());
    assert!(
        proposed_config["Triggers"]
            .as_mapping()
            .unwrap()
            .keys()
            .any(|name| name
                .as_str()
                .is_some_and(|name| name.starts_with("weekly-deps")))
    );

    let missing = w
        .ok(
            owner,
            "AddConfig",
            json!({ "input": { "tenantId": tenant, "repository": "acme/missing", "root": "/", "branch": "main" } }),
        )
        .await;
    assert_eq!(missing["addConfig"]["config"], Value::Null);
    assert!(
        missing["addConfig"]["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("acme/missing is not a repository of the tenant's GitHub App installation")
    );
    let added = w
        .ok(
            owner,
            "AddConfig",
            json!({ "input": { "tenantId": tenant, "repository": "acme/ops", "root": "/", "branch": "main" } }),
        )
        .await;
    assert_eq!(added["addConfig"]["errors"], json!([]));
    assert_eq!(added["addConfig"]["config"]["repository"], "acme/ops");
    let configs = w.ok(owner, "Configs", json!({ "tenant": slug })).await;
    assert_eq!(configs["tenant"]["configs"].as_array().unwrap().len(), 2);

    let credential = w
        .ok(
            owner,
            "AddCredential",
            json!({ "input": { "tenantId": tenant, "name": "acme-api", "provider": "ANTHROPIC", "key": "sk-ant-api-0123456789", "scope": "ORGANIZATION" } }),
        )
        .await;
    let credential_id = credential["addCredential"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let stored = w
        .dynamo
        .all::<Credential>()
        .await
        .into_iter()
        .find(|stored| stored.id.to_string() == credential_id)
        .unwrap();
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(&stored.encrypted_value)
        .unwrap();
    assert!(!ciphertext.windows(10).any(|window| window == b"sk-ant-api"));
    assert_eq!(
        AesGcmEncryptor::new(&CREDENTIAL_KEY)
            .decrypt(tenant, &ciphertext)
            .unwrap(),
        b"sk-ant-api-0123456789"
    );
    w.ok(owner, "Credentials", json!({ "tenant": slug })).await;
    w.ok(
        owner,
        "RemoveCredential",
        json!({ "credentialId": credential_id }),
    )
    .await;

    let invited = w
        .ok(
            owner,
            "InviteMember",
            json!({ "input": { "tenantId": tenant, "email": "newbie@example.com", "role": "MEMBER" } }),
        )
        .await;
    let invitation = invited["inviteMember"]["id"].as_str().unwrap().to_string();
    let newbie = "user:newbie";
    let accepted = w
        .ok(
            newbie,
            "AcceptInvitation",
            json!({ "invitationId": invitation }),
        )
        .await;
    let newbie_member = accepted["acceptInvitation"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let promoted = w
        .ok(
            owner,
            "ChangeMemberRole",
            json!({ "memberId": newbie_member, "role": "ADMIN" }),
        )
        .await;
    assert_eq!(promoted["changeMemberRole"]["role"], "ADMIN");
    let second = w
        .ok(
            owner,
            "InviteMember",
            json!({ "input": { "tenantId": tenant, "email": "later@example.com", "role": "MEMBER" } }),
        )
        .await;
    let revoked = w
        .ok(
            owner,
            "RevokeInvitation",
            json!({ "invitationId": second["inviteMember"]["id"] }),
        )
        .await;
    assert_eq!(revoked["revokeInvitation"]["state"], "REVOKED");
    let members = w.ok(owner, "Members", json!({ "tenant": slug })).await;
    assert_eq!(members["tenant"]["members"].as_array().unwrap().len(), 2);

    let link = w.ok(owner, "ConnectGitHub", json!({})).await;
    let link_url = link["connectGitHub"]["url"].as_str().unwrap();
    assert!(link_url.contains("/login/oauth/authorize"));
    assert_eq!(query_param(link_url, "client_id"), "Iv1.http");
    let unlinked = w.ok(owner, "UnlinkGitHub", json!({})).await;
    assert_eq!(unlinked["unlinkGitHub"]["user"]["githubLogin"], Value::Null);

    let (code, message) = w
        .error(owner, "ConnectSlack", json!({ "tenantId": tenant }))
        .await;
    assert_eq!(code, "BAD_USER_INPUT");
    assert_eq!(message, "Slack is not available yet");

    w.ok(owner, "AuditLog", json!({ "tenant": slug, "filter": null }))
        .await;
    let export = w
        .ok(
            owner,
            "ExportAuditLog",
            json!({ "tenantId": tenant, "filter": null }),
        )
        .await;
    let exported = w
        .get(export["exportAuditLog"]["url"].as_str().unwrap())
        .await;
    assert_eq!(exported.status(), StatusCode::OK);
    let lines = exported.text().await.unwrap();
    assert!(lines.lines().count() > 5);
    assert!(
        lines
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok())
    );
    assert!(lines.contains("credential.connect"));

    w.ok(owner, "Billing", json!({ "tenant": slug })).await;
    let (_, no_customer) = w
        .error(owner, "OpenBillingPortal", json!({ "tenantId": tenant }))
        .await;
    assert!(no_customer.contains("no Stripe customer"));
    let upgrade = w
        .ok(
            owner,
            "ChangePlan",
            json!({ "tenantId": tenant, "planId": "max" }),
        )
        .await;
    assert!(
        upgrade["changePlan"]["checkoutUrl"]
            .as_str()
            .unwrap()
            .starts_with("https://checkout.stripe.test/")
    );
    let sessions = w.api.stripe.requests("/v1/checkout/sessions");
    let session = &sessions.last().unwrap().form;
    assert_eq!(session["client_reference_id"], tenant.to_string());
    assert_eq!(session["line_items[0][price]"], "price_max");
    assert_eq!(session["metadata[plan_id]"], "max");
    assert_eq!(session["mode"], "subscription");

    w.dynamo
        .modify::<Tenant>(&keys::tenant_key(tenant), |row| {
            row.stripe_customer_id = Some("cus_acme".into());
        })
        .await;
    w.api.stripe.state.lock().unwrap().subscriptions.insert(
        "cus_acme".into(),
        vec![
            json!({ "id": "sub_old", "status": "canceled", "items": { "data": [{ "id": "si_old" }] } }),
            json!({ "id": "sub_acme", "status": "active", "items": { "data": [{ "id": "si_acme" }] } }),
        ],
    );
    let applied = w
        .ok(
            owner,
            "ChangePlan",
            json!({ "tenantId": tenant, "planId": "starter" }),
        )
        .await;
    assert_eq!(applied["changePlan"]["checkoutUrl"], Value::Null);
    assert_eq!(applied["changePlan"]["tenant"]["plan"]["id"], "starter");
    let updates = w.api.stripe.requests("/v1/subscriptions/sub_acme");
    assert_eq!(updates[0].form["items[0][id]"], "si_acme");
    assert_eq!(updates[0].form["items[0][price]"], "price_starter");
    let portal = w
        .ok(owner, "OpenBillingPortal", json!({ "tenantId": tenant }))
        .await;
    assert_eq!(
        portal["openBillingPortal"]["url"],
        "https://billing.stripe.test/cus_acme"
    );
    let portals = w.api.stripe.requests("/v1/billing_portal/sessions");
    assert_eq!(
        portals[0].form["configuration"],
        support::fake_stripe::PORTAL_CONFIGURATION
    );
    assert_eq!(
        w.api.stripe.requests("/v1/subscriptions")[0].form["status"],
        "all"
    );

    w.api
        .stripe
        .state
        .lock()
        .unwrap()
        .subscriptions
        .get_mut("cus_acme")
        .unwrap()[1]["status"] = json!("past_due");
    let sessions_before = w.api.stripe.requests("/v1/checkout/sessions").len();
    let settle = w
        .ok(
            owner,
            "ChangePlan",
            json!({ "tenantId": tenant, "planId": "pro" }),
        )
        .await;
    assert_eq!(
        settle["changePlan"]["checkoutUrl"],
        "https://billing.stripe.test/cus_acme"
    );
    assert_eq!(settle["changePlan"]["tenant"]["plan"]["id"], "starter");
    assert_eq!(
        w.api.stripe.requests("/v1/checkout/sessions").len(),
        sessions_before
    );
    assert_eq!(w.api.stripe.requests("/v1/subscriptions/sub_acme").len(), 1);

    let free = w
        .ok(
            newbie,
            "CreateTenant",
            json!({ "input": { "name": "Newbie Lab", "planId": "free" } }),
        )
        .await;
    assert_eq!(free["createTenant"]["tenant"]["name"], "Newbie Lab");
    let paid = w
        .ok(
            newbie,
            "CreateTenant",
            json!({ "input": { "name": "Newbie Pro", "planId": "pro" } }),
        )
        .await;
    assert_eq!(paid["createTenant"]["tenant"], Value::Null);
    assert!(paid["createTenant"]["checkoutUrl"].is_string());
    let abandoned: Uuid = w
        .api
        .stripe
        .requests("/v1/checkout/sessions")
        .last()
        .unwrap()
        .form["client_reference_id"]
        .parse()
        .unwrap();
    let retried = w
        .ok(
            newbie,
            "CreateTenant",
            json!({ "input": { "name": "Newbie Pro", "planId": "pro" } }),
        )
        .await;
    let retried_url = retried["createTenant"]["checkoutUrl"].as_str().unwrap();
    let sessions = w.api.stripe.requests("/v1/checkout/sessions");
    let session = sessions.last().unwrap().form.clone();
    assert_eq!(session["customer_email"], "newbie@example.com");
    assert!(session.contains_key("expires_at"));
    let pending: Uuid = session["client_reference_id"].parse().unwrap();
    assert_ne!(pending, abandoned);
    let first_session = format!("cs_test_{}", sessions.len() - 1);
    assert_eq!(
        w.api.stripe.state.lock().unwrap().sessions[&first_session],
        "expired"
    );
    assert!(
        w.api
            .store
            .get::<Tenant>(&keys::tenant_key(abandoned))
            .await
            .unwrap()
            .is_none()
    );
    let held = World::tenant_of(&w.dynamo, pending).await;
    assert_eq!(
        (held.status.as_str(), held.suspended_reason.as_deref()),
        ("suspended", Some("payment_pending"))
    );
    let paid_session = retried_url.rsplit('/').next().unwrap().to_string();
    w.api
        .stripe
        .state
        .lock()
        .unwrap()
        .sessions
        .insert(paid_session, "complete".into());
    let checkout_completed = |payment_status: &str| {
        json!({
            "id": format!("evt_new_{payment_status}"), "type": "checkout.session.completed",
            "created": Utc::now().timestamp(),
            "data": { "object": {
                "client_reference_id": pending, "customer": "cus_newbie",
                "payment_status": payment_status,
                "metadata": { "plan_id": session["metadata[plan_id]"] },
            } },
        })
    };
    let unpaid = w.stripe_webhook(&checkout_completed("unpaid")).await;
    assert_eq!(unpaid.status(), StatusCode::OK);
    assert_eq!(
        World::tenant_of(&w.dynamo, pending)
            .await
            .suspended_reason
            .as_deref(),
        Some("payment_pending")
    );
    let completed = w.stripe_webhook(&checkout_completed("paid")).await;
    assert_eq!(completed.status(), StatusCode::OK);
    assert_eq!(World::tenant_of(&w.dynamo, pending).await.status, "active");

    let operator = "user:ops";
    w.ok(operator, "Shell", json!({})).await;
    let ops = w
        .dynamo
        .all::<User>()
        .await
        .into_iter()
        .find(|user| user.cognito_subject == "ops")
        .unwrap();
    w.dynamo
        .modify::<User>(&keys::user_key(ops.id), |user| user.is_operator = true)
        .await;
    let tenants = w.ok(operator, "OperatorTenants", json!({})).await;
    assert!(tenants["operator"]["tenants"].as_array().unwrap().len() >= 3);
    w.ok(
        operator,
        "SuspendTenant",
        json!({ "tenantId": pending, "reason": "abuse report" }),
    )
    .await;
    w.ok(operator, "UnsuspendTenant", json!({ "tenantId": pending }))
        .await;

    let integrations = w.ok(owner, "Integrations", json!({ "tenant": slug })).await;
    let install_url = integrations["tenant"]["githubAppInstallUrl"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(install_url.contains("/apps/sanchoris-test/installations/new"));
    {
        let mut github = w.github.state.lock().unwrap();
        github
            .oauth_users
            .insert("code-install".into(), (1001, "mika-dev".into()));
        github.user_installations.insert(
            "gho_code-install".into(),
            vec![json!({ "id": 77, "account": { "login": "acme-labs", "type": "Organization" } })],
        );
        github
            .installation_repositories
            .insert(77, vec![(8801, "acme-labs/api".into())]);
    }
    let setup = w
        .get(&format!(
            "{}/auth/github/setup?installation_id=77&setup_action=install&state={}",
            w.api.url,
            query_param(&install_url, "state")
        ))
        .await;
    assert_eq!(setup.status(), StatusCode::SEE_OTHER);
    let authorize = setup.headers()["location"].to_str().unwrap().to_string();
    let installed = w
        .get(&format!(
            "{}/auth/github/callback?code=code-install&state={}",
            w.api.url,
            query_param(&authorize, "state")
        ))
        .await;
    assert_eq!(
        installed.headers()["location"],
        format!("{}/settings?github=installed", w.api.url).as_str()
    );
    let foreign = w
        .get(&format!(
            "{}/auth/github/setup?installation_id=78&setup_action=install&state={}",
            w.api.url,
            query_param(&install_url, "state")
        ))
        .await;
    let foreign_state = query_param(foreign.headers()["location"].to_str().unwrap(), "state");
    let refused = w
        .get(&format!(
            "{}/auth/github/callback?code=code-install&state={foreign_state}",
            w.api.url
        ))
        .await;
    assert_eq!(
        refused.headers()["location"],
        format!("{}/settings?github=not_accessible", w.api.url).as_str()
    );
    let processed = w
        .github_webhook(
            "installation_repositories",
            &json!({
                "action": "added", "installation": { "id": 77 },
                "repositories_added": [{ "id": 8802, "full_name": "acme-labs/web" }], "repositories_removed": [],
            }),
        )
        .await;
    assert_eq!(processed["status"], "processed");
    let integrations = w.ok(owner, "Integrations", json!({ "tenant": slug })).await;
    let labs = integrations["tenant"]["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|integration| integration["account"] == "acme-labs")
        .expect("the installation is connected")
        .clone();
    assert_eq!(labs["status"], "CONNECTED");
    assert_eq!(
        labs["repositories"],
        json!(["acme-labs/api", "acme-labs/web"])
    );
    w.github_webhook(
        "installation",
        &json!({
            "action": "suspend",
            "installation": { "id": INSTALLATION, "account": { "login": "acme", "type": "Organization" } },
        }),
    )
    .await;
    let suspended = w
        .dynamo
        .all::<Integration>()
        .await
        .into_iter()
        .find(|integration| integration.github_installation_id == Some(INSTALLATION))
        .unwrap();
    assert_eq!(suspended.status, "not_connected");

    let actions = w.audit_actions(tenant).await;
    for action in [
        "config.sync",
        "chat.message_send",
        "change_request.open",
        "config.add",
        "member.invitation_accept",
        "integration.github_link_start",
        "integration.github_unlink",
        "audit_log.export",
        "tenant.plan_checkout_start",
        "tenant.plan_change",
        "billing.portal_open",
        "integration.github_install",
        "integration.github_installation_update",
    ] {
        assert!(
            actions.iter().any(|done| done == action),
            "{action} is not in {actions:?}"
        );
    }
    let audited_ip = w
        .dynamo
        .all::<AuditEvent>()
        .await
        .into_iter()
        .find(|event| event.tenant_id == tenant && event.action == "config.add")
        .unwrap()
        .source_ip;
    assert_eq!(audited_ip.as_deref(), Some("127.0.0.1"));

    let executed = w.executed.lock().unwrap().clone();
    assert_eq!(executed, operation_names(&w.document));
}

#[tokio::test]
async fn graphql_reads_only_the_tenants_of_the_signed_in_user() {
    let w = World::start().await;
    let other = w
        .ok("user:stranger", "Home", json!({ "tenant": w.slug }))
        .await;
    assert_eq!(other["tenant"], Value::Null);
    let shell = w.ok("user:stranger", "Shell", json!({})).await;
    assert_eq!(shell["viewer"]["memberships"], json!([]));
    let (code, _) = w
        .error(
            "user:stranger",
            "StopTask",
            json!({ "taskId": Uuid::new_v4() }),
        )
        .await;
    assert_eq!(code, "NOT_FOUND");
    let (code, _) = w
        .error(
            "user:stranger",
            "SyncConfig",
            json!({ "configId": w.seeded.tenant.config_id }),
        )
        .await;
    assert_eq!(code, "NOT_FOUND");
}

#[tokio::test]
async fn the_built_frontend_is_served_for_every_other_path() {
    let w = World::start().await;
    for path in ["/", "/tasks/acme", "/auth/callback?code=1"] {
        let page = w.get(&format!("{}{path}", w.api.url)).await;
        assert_eq!(page.status(), StatusCode::OK, "{path}");
        assert_eq!(page.headers()["cache-control"], "no-cache");
        assert!(
            page.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        assert_eq!(page.text().await.unwrap(), FRONTEND_INDEX);
    }
    let script = w.get(&format!("{}/assets/app.js", w.api.url)).await;
    assert_eq!(script.status(), StatusCode::OK);
    assert_eq!(
        script.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert!(
        script.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("javascript")
    );
    assert_eq!(script.text().await.unwrap(), FRONTEND_SCRIPT);
    for path in ["/assets/missing.js", "/api/unknown", "/webhooks/unknown"] {
        let missing = w.get(&format!("{}{path}", w.api.url)).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND, "{path}");
    }
    let health = w.get(&format!("{}/health", w.api.url)).await;
    assert_eq!(
        health.json::<Value>().await.unwrap(),
        json!({ "status": "ok" })
    );
}

#[tokio::test]
async fn repository_events_rename_and_disable_the_config_by_its_repository_id() {
    let w = World::start().await;
    let owner = w.owner.as_str();
    let slug = w.slug.as_str();
    w.ok(
        owner,
        "SyncConfig",
        json!({ "configId": w.seeded.tenant.config_id }),
    )
    .await;

    let renamed = w
        .github_webhook(
            "repository",
            &json!({
                "action": "renamed",
                "repository": { "id": CONFIG_REPO_ID, "full_name": "acme/platform-config" },
                "changes": { "repository": { "name": { "from": "sanchoris-config" } } },
                "installation": { "id": INSTALLATION },
            }),
        )
        .await;
    assert_eq!(renamed["status"], "processed");
    let configs = w.ok(owner, "Configs", json!({ "tenant": slug })).await;
    let config = &configs["tenant"]["configs"][0];
    assert_eq!(config["repository"], "acme/platform-config");
    assert_eq!(config["repositoryId"], CONFIG_REPO_ID.to_string());
    assert_eq!(config["status"], "ACTIVE");
    assert!(
        configs["tenant"]["integrations"][0]["repositories"]
            .as_array()
            .unwrap()
            .contains(&json!("acme/platform-config"))
    );
    assert_eq!(configs["tenant"]["projects"].as_array().unwrap().len(), 1);

    let removed = w
        .github_webhook(
            "installation_repositories",
            &json!({
                "action": "removed",
                "installation": { "id": INSTALLATION },
                "repositories_added": [],
                "repositories_removed": [{ "id": CONFIG_REPO_ID, "full_name": "acme/platform-config" }],
            }),
        )
        .await;
    assert_eq!(removed["status"], "processed");
    let configs = w.ok(owner, "Configs", json!({ "tenant": slug })).await;
    let config = &configs["tenant"]["configs"][0];
    assert_eq!(config["status"], "DISABLED");
    assert_eq!(
        config["syncError"],
        "acme/platform-config was removed from the GitHub App installation"
    );
    assert_eq!(config["triggerCount"], 0);
    assert_eq!(configs["tenant"]["projects"], json!([]));
    let (code, _) = w
        .error(
            owner,
            "SyncConfig",
            json!({ "configId": w.seeded.tenant.config_id }),
        )
        .await;
    assert_eq!(code, "BAD_USER_INPUT");
    assert!(
        w.audit_actions(w.tenant())
            .await
            .iter()
            .any(|action| action == "config.disable")
    );
}
