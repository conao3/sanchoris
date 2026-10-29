use crate::support;

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use reqwest::StatusCode;
use sanchoris_api::channel::{CLOSE_PROTOCOL_ERROR, SEQ_STRIDE};
use sanchoris_api::records::{RecordStore, record_key};
use sanchoris_api::signing::HmacKey;
use sanchoris_core::audit::audit_events;
use sanchoris_core::billing::PAYMENT_PENDING;
use sanchoris_core::items::{
    ApprovalWait, ChatMessage, Config, InboundEvent, Integration, Member, OutboxItem, ReceiveRate,
    RunEventItem, RunLogEntry, Task, Tenant,
};
use sanchoris_core::jobs::{Envelope, Job};
use sanchoris_core::keys;
use sanchoris_core::receive::window_start;
use sanchoris_core::skillrun::accept_skill_request;
use sanchoris_core::testing::{SEED_CONFIG_REPOSITORY_ID, TestDynamo};
use sanchoris_core::tokens::{RunTokenSigner, Signer, TaskTokenSigner};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use support::fake_github::FakeGitHub;
use support::{
    Api, Driving, RUN_KEY, STRIPE_SECRET, Seeded, TASK_KEY, WEBHOOK_SECRET, Workspaces, eventually,
    flow_cookie, github_signature, labeled_payload, seed, start_api, task_with_run,
};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

struct Server {
    api: Api,
    seeded: Seeded,
    github: Arc<FakeGitHub>,
    dynamo: TestDynamo,
    _dir: tempfile::TempDir,
}

impl Server {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let github = FakeGitHub::new(&dir.path().join("git"), BTreeMap::new());
        let dynamo = TestDynamo::start().await;
        let seeded = seed(&dynamo).await;
        let api = start_api(
            &dynamo,
            github.clone(),
            dir.path(),
            Workspaces::None,
            Driving::QueueOnly,
        )
        .await;
        Self {
            api,
            seeded,
            github,
            dynamo,
            _dir: dir,
        }
    }

    fn tenant(&self) -> Uuid {
        self.seeded.tenant.tenant_id
    }

    async fn get(&self, path: &str, token: Option<&str>) -> reqwest::Response {
        let mut request = self.api.http.get(format!("{}{path}", self.api.url));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.unwrap()
    }

    async fn get_with_cookie(&self, path: &str, cookie: &str) -> reqwest::Response {
        self.api
            .http
            .get(format!("{}{path}", self.api.url))
            .header("cookie", cookie)
            .send()
            .await
            .unwrap()
    }

    async fn post(&self, path: &str, token: Option<&str>, body: &Value) -> reqwest::Response {
        let mut request = self
            .api
            .http
            .post(format!("{}{path}", self.api.url))
            .json(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.unwrap()
    }

    async fn webhook(&self, event: &str, delivery: &str, payload: &Value) -> reqwest::Response {
        let body = serde_json::to_vec(payload).unwrap();
        self.api
            .http
            .post(format!("{}/webhooks/github", self.api.url))
            .header("X-GitHub-Event", event)
            .header("X-GitHub-Delivery", delivery)
            .header(
                "X-Hub-Signature-256",
                github_signature(WEBHOOK_SECRET, &body),
            )
            .body(body)
            .send()
            .await
            .unwrap()
    }

    async fn task_with_run(&self) -> (Uuid, Uuid) {
        task_with_run(&self.dynamo, &self.seeded).await
    }
}

#[tokio::test]
async fn me_requires_a_token_and_provisions_the_user_once() {
    let server = Server::start().await;
    assert_eq!(
        server.get("/api/me", None).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        server.get("/api/me", Some("bogus")).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let first: Value = server
        .get("/api/me", Some("user:google_42"))
        .await
        .json()
        .await
        .unwrap();
    let second: Value = server
        .get("/api/me", Some("user:google_42"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(first["id"], second["id"]);
    assert_eq!(first["email"], "google_42@example.com");
    assert_eq!(first["githubLogin"], Value::Null);
    let member: Value = server
        .get("/api/me", Some(&server.seeded.member_token()))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        member["id"],
        json!(server.seeded.tenant.user_id.to_string())
    );
    assert_eq!(member["githubLogin"], "mika-dev");
    assert_eq!(server.get("/health", None).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn github_linking_stores_the_account_bound_to_the_signed_in_user() {
    let server = Server::start().await;
    server
        .github
        .state
        .lock()
        .unwrap()
        .oauth_users
        .insert("code-1".into(), (2002, "sota-dev".into()));
    assert_eq!(
        server.post("/auth/github", None, &json!({})).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let started = server
        .post("/auth/github", Some("user:google_7"), &json!({}))
        .await;
    let cookie = flow_cookie(&started);
    let started: Value = started.json().await.unwrap();
    let authorize = reqwest::Url::parse(started["url"].as_str().unwrap()).unwrap();
    assert_eq!(authorize.path(), "/login/oauth/authorize");
    let query: BTreeMap<String, String> = authorize.query_pairs().into_owned().collect();
    assert_eq!(query["client_id"], "Iv1.http");
    assert_eq!(
        query["redirect_uri"],
        format!("{}/auth/github/callback", server.api.url)
    );

    let forged = server
        .get_with_cookie(
            &format!(
                "/auth/github/callback?code=code-1&state={}",
                query["state"].replace('.', "x")
            ),
            &cookie,
        )
        .await;
    assert_eq!(forged.status(), StatusCode::BAD_REQUEST);

    let linked = server
        .get_with_cookie(
            &format!("/auth/github/callback?code=code-1&state={}", query["state"]),
            &cookie,
        )
        .await;
    assert_eq!(linked.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        linked.headers()["location"],
        format!("{}/settings?github=linked", server.api.url).as_str()
    );
    assert!(
        linked.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .starts_with("__Host-sanchoris-github=; Path=/; Max-Age=0")
    );
    let me: Value = server
        .get("/api/me", Some("user:google_7"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(me["githubLogin"], "sota-dev");

    let other = server
        .post("/auth/github", Some("user:google_8"), &json!({}))
        .await;
    let other_cookie = flow_cookie(&other);
    let other: Value = other.json().await.unwrap();
    let other_state = query_param(other["url"].as_str().unwrap(), "state");
    let in_use = server
        .get_with_cookie(
            &format!("/auth/github/callback?code=code-1&state={other_state}"),
            &other_cookie,
        )
        .await;
    assert_eq!(
        in_use.headers()["location"],
        format!("{}/settings?github=in_use", server.api.url).as_str()
    );
    let denied = server
        .get_with_cookie(
            &format!("/auth/github/callback?error=access_denied&state={other_state}"),
            &other_cookie,
        )
        .await;
    assert_eq!(
        denied.headers()["location"],
        format!("{}/settings?github=denied", server.api.url).as_str()
    );
}

fn query_param(url: &str, name: &str) -> String {
    reqwest::Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == name)
        .unwrap()
        .1
        .into_owned()
}

#[tokio::test]
async fn github_linking_completes_only_in_the_browser_that_started_it() {
    let server = Server::start().await;
    server
        .github
        .state
        .lock()
        .unwrap()
        .oauth_users
        .insert("victim-code".into(), (3003, "victim-dev".into()));
    let attacker = server
        .post("/auth/github", Some("user:google_66"), &json!({}))
        .await;
    let attacker_cookie = flow_cookie(&attacker);
    let attacker: Value = attacker.json().await.unwrap();
    let state = query_param(attacker["url"].as_str().unwrap(), "state");
    let victim = server
        .post("/auth/github", Some("user:google_67"), &json!({}))
        .await;
    let victim_cookie = flow_cookie(&victim);
    assert_ne!(victim_cookie, attacker_cookie);
    let path = format!("/auth/github/callback?code=victim-code&state={state}");
    assert_eq!(
        server.get(&path, None).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        server.get_with_cookie(&path, &victim_cookie).await.status(),
        StatusCode::BAD_REQUEST
    );
    let me: Value = server
        .get("/api/me", Some("user:google_66"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(me["githubLogin"], Value::Null);
}

#[tokio::test]
async fn github_webhooks_are_verified_recorded_once_and_update_installations() {
    let server = Server::start().await;
    let body = serde_json::to_vec(&labeled_payload(35, "Forged")).unwrap();
    let forged = server
        .api
        .http
        .post(format!("{}/webhooks/github", server.api.url))
        .header("X-GitHub-Event", "issues")
        .header("X-GitHub-Delivery", "forged")
        .header("X-Hub-Signature-256", "sha256=00")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(forged.status(), StatusCode::UNAUTHORIZED);

    let payload = labeled_payload(35, "Fix flaky checkout test");
    let first = server.webhook("issues", "delivery-35", &payload).await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    assert_eq!(first.json::<Value>().await.unwrap()["status"], "accepted");
    let again = server.webhook("issues", "delivery-35", &payload).await;
    assert_eq!(again.status(), StatusCode::OK);
    assert_eq!(again.json::<Value>().await.unwrap()["status"], "duplicate");
    let mut unknown = payload.clone();
    unknown["installation"]["id"] = json!(999);
    let ignored = server.webhook("issues", "delivery-999", &unknown).await;
    assert_eq!(ignored.json::<Value>().await.unwrap()["status"], "ignored");

    let suspend = json!({
        "action": "suspend",
        "installation": { "id": support::INSTALLATION, "account": { "login": "acme", "type": "Organization" } },
    });
    let suspended = server
        .webhook("installation", "delivery-suspend", &suspend)
        .await;
    assert_eq!(
        suspended.json::<Value>().await.unwrap()["status"],
        "processed"
    );
    let integrations = server.dynamo.all::<Integration>().await;
    assert_eq!(integrations.len(), 1);
    assert_eq!(integrations[0].status, "not_connected");
    let audit = server
        .dynamo
        .all::<sanchoris_core::items::AuditEvent>()
        .await;
    assert!(
        audit
            .iter()
            .any(|event| event.action == "integration.github_installation_update")
    );
}

fn push_payload(repository_id: i64, git_ref: &str) -> Value {
    json!({
        "ref": git_ref,
        "after": "c0ff1ce0000000000000000000000000000000c2",
        "repository": { "id": repository_id, "full_name": "acme/sanchoris-config" },
        "installation": { "id": support::INSTALLATION },
    })
}

fn queued_config_syncs(server: &Server) -> Vec<Uuid> {
    server
        .api
        .driver
        .queue
        .bodies()
        .iter()
        .filter_map(|body| match Envelope::parse(body).unwrap().job {
            Job::ConfigSync { config_id, .. } => Some(config_id),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_push_to_the_config_branch_syncs_the_config_right_away() {
    let server = Server::start().await;
    let config_id = server.seeded.tenant.config_id;

    let other_branch = server
        .webhook(
            "push",
            "push-feature",
            &push_payload(SEED_CONFIG_REPOSITORY_ID, "refs/heads/feature"),
        )
        .await;
    assert_eq!(
        other_branch.json::<Value>().await.unwrap()["status"],
        "ignored"
    );
    let other_repository = server
        .webhook("push", "push-other", &push_payload(9999, "refs/heads/main"))
        .await;
    assert_eq!(
        other_repository.json::<Value>().await.unwrap()["status"],
        "ignored"
    );
    assert!(queued_config_syncs(&server).is_empty());

    let pushed = server
        .webhook(
            "push",
            "push-main",
            &push_payload(SEED_CONFIG_REPOSITORY_ID, "refs/heads/main"),
        )
        .await;
    assert_eq!(pushed.status(), StatusCode::ACCEPTED);
    assert_eq!(
        pushed.json::<Value>().await.unwrap()["status"],
        "config_sync"
    );

    let config =
        eventually(
            "the config sync",
            std::time::Duration::from_secs(10),
            || async {
                server.dynamo.all::<Config>().await.into_iter().find(|c| {
                    c.id == config_id && (c.synced_at.is_some() || c.sync_error.is_some())
                })
            },
        )
        .await;
    assert_eq!(config.sync_error, None);
    assert!(config.synced_at.is_some());
}

async fn fill_receive_windows(server: &Server, received: i64) {
    let start = window_start(Utc::now());
    for window in [start, start + chrono::Duration::seconds(60)] {
        server
            .dynamo
            .put(&ReceiveRate {
                tenant_id: server.tenant(),
                window_start: window,
                received,
                dropped: 0,
                expires_at: window + chrono::Duration::days(1),
            })
            .await;
    }
}

#[tokio::test]
async fn inbound_events_over_the_receive_limit_are_answered_429_and_counted() {
    let server = Server::start().await;
    let (task_id, _) = server.task_with_run().await;
    let accepted = server
        .webhook("issues", "delivery-36", &labeled_payload(36, "Before"))
        .await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    fill_receive_windows(&server, 120).await;

    let dropped = server
        .webhook("issues", "delivery-37", &labeled_payload(37, "Flood"))
        .await;
    assert_eq!(dropped.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after: i64 = dropped.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry_after));
    let body = dropped.json::<Value>().await.unwrap();
    assert_eq!(body["status"], "dropped");
    assert_eq!(body["limit"], 120);
    assert_eq!(body["retryAfterSeconds"], retry_after);
    assert!(
        body["reason"].as_str().unwrap().starts_with(
            "receive limit: the tenant already received 120 events in the minute from "
        )
    );
    let inbound = server.dynamo.all::<InboundEvent>().await;
    assert!(inbound.iter().any(|e| e.key == "delivery-36"));
    assert!(inbound.iter().all(|e| e.key != "delivery-37"));
    let again = server
        .webhook("issues", "delivery-36", &labeled_payload(36, "Before"))
        .await;
    assert_eq!(again.json::<Value>().await.unwrap()["status"], "duplicate");

    let pushed = server
        .webhook(
            "push",
            "push-over-limit",
            &push_payload(SEED_CONFIG_REPOSITORY_ID, "refs/heads/main"),
        )
        .await;
    assert_eq!(
        pushed.json::<Value>().await.unwrap()["status"],
        "config_sync"
    );
    let renamed = json!({
        "action": "renamed",
        "repository": { "id": support::SHOP_WEB_ID, "full_name": "acme/shop-site" },
        "installation": { "id": support::INSTALLATION },
    });
    let processed = server
        .webhook("repository", "rename-over-limit", &renamed)
        .await;
    assert_eq!(
        processed.json::<Value>().await.unwrap()["status"],
        "processed"
    );

    let reply: Value = server
        .post(
            "/api/graphql",
            Some(&server.seeded.member_token()),
            &json!({
                "query": "mutation($input: ResumeTaskInput!) { resumeTask(input: $input) { id } }",
                "variables": { "input": { "taskId": task_id, "message": "One more thing." } },
            }),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(reply["errors"][0]["extensions"]["code"], "RATE_LIMITED");

    let rates = server.dynamo.all::<ReceiveRate>().await;
    let dropped: i64 = rates.iter().map(|rate| rate.dropped).sum();
    assert_eq!(dropped, 2);
    let audits = audit_events(&mut server.api.store.tx(server.tenant()))
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.action == "tenant.receive_limit")
        .collect::<Vec<_>>();
    assert!((1..=2).contains(&audits.len()));
    assert_eq!(audits[0].metadata["source"], "github");
    assert_eq!(audits[0].metadata["key"], "delivery-37");
}

fn stripe_header(body: &[u8]) -> String {
    let t = Utc::now().timestamp();
    let mut signed = format!("{t}.").into_bytes();
    signed.extend_from_slice(body);
    format!(
        "t={t},v1={}",
        HmacKey::new(STRIPE_SECRET.as_bytes()).unwrap().hex(&signed)
    )
}

async fn stripe(server: &Server, event: &Value, signature: Option<String>) -> reqwest::Response {
    let body = serde_json::to_vec(event).unwrap();
    server
        .api
        .http
        .post(format!("{}/webhooks/stripe", server.api.url))
        .header(
            "Stripe-Signature",
            signature.unwrap_or_else(|| stripe_header(&body)),
        )
        .body(body)
        .send()
        .await
        .unwrap()
}

async fn tenant_item(server: &Server) -> Tenant {
    server
        .api
        .store
        .get::<Tenant>(&keys::tenant_key(server.tenant()))
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn stripe_webhooks_are_verified_and_update_the_tenant() {
    let server = Server::start().await;
    let tenant_id = server.tenant();
    server
        .dynamo
        .modify::<Tenant>(&keys::tenant_key(tenant_id), |tenant| {
            tenant.status = "suspended".into();
            tenant.suspended_reason = Some(PAYMENT_PENDING.into());
        })
        .await;
    server.dynamo.set_stripe_price("pro", "price_pro").await;
    let now = Utc::now().timestamp();
    let checkout = |id: &str, tenant: Uuid, payment_status: &str| {
        json!({
            "id": id, "type": "checkout.session.completed", "created": now,
            "data": { "object": {
                "client_reference_id": tenant, "customer": "cus_http",
                "payment_status": payment_status, "metadata": { "plan_id": "pro" },
            } },
        })
    };
    let paid = checkout("evt_1", tenant_id, "paid");
    assert_eq!(
        stripe(&server, &paid, Some("t=1,v1=00".into()))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let unpaid = stripe(&server, &checkout("evt_0", tenant_id, "unpaid"), None).await;
    assert_eq!(unpaid.status(), StatusCode::OK);
    assert_eq!(tenant_item(&server).await.status, "suspended");
    let response = stripe(&server, &paid, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<Value>().await.unwrap()["status"],
        "processed"
    );
    let tenant = tenant_item(&server).await;
    assert_eq!(tenant.status, "active");
    assert_eq!(tenant.stripe_customer_id.as_deref(), Some("cus_http"));
    assert_eq!(
        stripe(&server, &paid, None)
            .await
            .json::<Value>()
            .await
            .unwrap()["status"],
        "duplicate"
    );

    let stranger = checkout("evt_stranger", Uuid::new_v4(), "paid");
    for expected in ["ignored", "duplicate"] {
        let response = stripe(&server, &stranger, None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.json::<Value>().await.unwrap()["status"], expected);
    }
    let incomplete = json!({
        "id": "evt_incomplete", "type": "customer.subscription.updated", "created": now,
        "data": { "object": {
            "customer": "cus_http", "status": "incomplete", "metadata": { "tenant_id": tenant_id },
            "items": { "data": [{ "price": { "id": "price_pro" } }] },
        } },
    });
    let body = stripe(&server, &incomplete, None)
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(
        (body["status"].as_str(), body["reason"].as_str()),
        (Some("ignored"), Some("the subscription is incomplete"))
    );

    let failed = json!({
        "id": "evt_2", "type": "invoice.payment_failed", "created": now,
        "data": { "object": { "customer": "cus_http" } },
    });
    assert_eq!(
        stripe(&server, &failed, None).await.status(),
        StatusCode::OK
    );
    assert_eq!(tenant_item(&server).await.status, "suspended");

    server
        .dynamo
        .modify::<Tenant>(&keys::tenant_key(tenant_id), |tenant| {
            tenant.tasks_this_period = 3;
        })
        .await;
    let renewal = now + 30 * 86_400;
    let invoice = |id: &str, reason: &str| {
        json!({
            "id": id, "type": "invoice.paid", "created": now,
            "data": { "object": {
                "customer": "cus_http", "billing_reason": reason,
                "parent": { "subscription_details": { "metadata": { "tenant_id": tenant_id } } },
                "lines": { "data": [
                    { "period": { "start": now, "end": renewal } },
                    { "period": { "start": renewal, "end": renewal + 30 * 86_400 } },
                ] },
            } },
        })
    };
    let proration = stripe(&server, &invoice("evt_3", "subscription_update"), None).await;
    assert_eq!(proration.status(), StatusCode::OK);
    let tenant = tenant_item(&server).await;
    assert_eq!(
        (tenant.status.as_str(), tenant.tasks_this_period),
        ("active", 3)
    );
    stripe(&server, &invoice("evt_4", "subscription_cycle"), None).await;
    let tenant = tenant_item(&server).await;
    assert_eq!(
        (tenant.tasks_this_period, tenant.period_start.timestamp()),
        (0, renewal)
    );

    let unknown = json!({
        "id": "evt_5", "type": "invoice.paid", "created": now,
        "data": { "object": {
            "customer": "cus_unknown", "billing_reason": "subscription_cycle", "lines": { "data": [] },
        } },
    });
    assert_eq!(
        stripe(&server, &unknown, None).await.status(),
        StatusCode::CONFLICT
    );
    let ignored = json!({
        "id": "evt_6", "type": "customer.created", "created": now, "data": { "object": {} },
    });
    assert_eq!(
        stripe(&server, &ignored, None)
            .await
            .json::<Value>()
            .await
            .unwrap()["status"],
        "ignored"
    );
}

#[tokio::test]
async fn replies_go_through_resume_task_and_approvals_need_a_member_of_the_tenant() {
    let server = Server::start().await;
    let (task_id, run_id) = server.task_with_run().await;
    let member = server.seeded.member_token();
    let response = server
        .post(
            &format!("/api/tasks/{task_id}/replies"),
            Some(&member),
            &json!({ "text": "Use the staging database." }),
        )
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        server
            .dynamo
            .all::<RunEventItem>()
            .await
            .into_iter()
            .all(|event| event.run_id != run_id || event.kind != "human_reply")
    );

    assert_eq!(
        server
            .post(
                "/api/approvals",
                Some(&member),
                &json!({ "token": "not-a-token", "approved": true })
            )
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let signed = TaskTokenSigner::new(Signer::new(&TASK_KEY).unwrap())
        .sign(Uuid::new_v4(), "sanchoris-task:1:4:1");
    assert_eq!(
        server
            .post(
                "/api/approvals",
                Some(&member),
                &json!({ "token": signed, "approved": true })
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn approvals_need_an_owner_an_admin_or_the_requester_and_are_audited() {
    let server = Server::start().await;
    let (task_id, run_id) = server.task_with_run().await;
    let tenant = server.tenant();
    let member_id = server.seeded.tenant.member_id;
    let yaml = "Inputs: {}\nStartAt: Ask\nTimeoutSeconds: 600\nStates:\n  Ask:\n    Type: Task\n    Resource: sanchoris:::approval:request.waitForTaskToken\n    Parameters: { Message: Ship it?, TaskToken.$: $$.Task.Token }\n    TimeoutSeconds: 300\n    End: true\n";
    let skill = sanchoris_config::Skill {
        name: "ship".into(),
        description: "Ship".into(),
        body: String::new(),
        skill_md: String::new(),
        skill_yaml: Some(yaml.into()),
        definition: Some(sanchoris_config::skill::parse_skill_yaml(yaml).unwrap()),
        origin: sanchoris_config::SkillOrigin::Repository,
    };
    server
        .dynamo
        .modify::<Task>(&keys::task_key(tenant, task_id), |task| {
            task.definition["Skills"]
                .as_array_mut()
                .unwrap()
                .push(json!({ "Name": "ship", "Definition": skill }));
        })
        .await;
    let mut tx = server.api.store.tx(tenant);
    let accepted = accept_skill_request(&mut tx, run_id, "ship", json!({}), Utc::now())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let wait = eventually(
        "the approval wait",
        std::time::Duration::from_secs(10),
        || async { server.dynamo.all::<ApprovalWait>().await.pop() },
    )
    .await;
    let token =
        TaskTokenSigner::new(Signer::new(&TASK_KEY).unwrap()).sign(accepted.id, &wait.task_token);
    let body = json!({ "token": token, "approved": true });
    let member = server.seeded.member_token();
    server
        .dynamo
        .modify::<Member>(&keys::member_key(tenant, member_id), |m| {
            m.role = "member".into()
        })
        .await;
    assert_eq!(
        server
            .post("/api/approvals", Some(&member), &body)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    server
        .dynamo
        .modify::<Task>(&keys::task_key(tenant, task_id), |task| {
            task.requested_by_member_id = Some(member_id)
        })
        .await;
    assert_eq!(
        server
            .post("/api/approvals", Some(&member), &body)
            .await
            .status(),
        StatusCode::OK
    );
    let mut tx = server.api.store.tx(tenant);
    let audits = audit_events(&mut tx)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.action == "approval.resolve")
        .collect::<Vec<_>>();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].actor_ref, member_id.to_string());
    assert_eq!(audits[0].target_ref, task_id.to_string());
    assert_eq!(audits[0].metadata["approved"], json!(true));
}

#[tokio::test]
async fn local_records_accept_only_signed_uploads_of_the_announced_content() {
    let server = Server::start().await;
    let tenant = server.tenant();
    let records = &server.api.records;
    let key = record_key(tenant, Uuid::new_v4(), "session-1", 1).unwrap();
    let body = b"{\"role\":\"user\"}\n".to_vec();
    let sha = hex::encode(Sha256::digest(&body));
    let put = records
        .presign_put(tenant, &key, body.len() as i64, &sha)
        .await
        .unwrap();
    assert!(
        records
            .presign_put(Uuid::new_v4(), &key, 1, &sha)
            .await
            .is_err()
    );
    let http = &server.api.http;
    let wrong = http
        .put(&put.url)
        .body(b"tampered\n".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
    let forged = http
        .put(put.url.replace("length=", "length=1"))
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(forged.status(), StatusCode::FORBIDDEN);
    let uploaded = http
        .put(&put.url)
        .header("content-type", &put.headers["content-type"])
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(uploaded.status(), StatusCode::OK);
    let url = records.presign_get(tenant, &key).await.unwrap();
    let downloaded = http.get(&url).send().await.unwrap();
    assert_eq!(downloaded.bytes().await.unwrap().to_vec(), body);
    let unsigned = http
        .get(url.split('?').next().unwrap())
        .send()
        .await
        .unwrap();
    assert!(unsigned.status().is_client_error());
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(server: &Server, token: &str) -> Result<Socket, u16> {
    let mut request = format!("{}/runs/channel", server.api.url.replace("http", "ws"))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _)) => Ok(socket),
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            Err(response.status().as_u16())
        }
        Err(error) => panic!("{error}"),
    }
}

async fn send(socket: &mut Socket, message: Value) {
    socket
        .send(Message::Text(message.to_string().into()))
        .await
        .unwrap();
}

async fn receive(socket: &mut Socket) -> Value {
    loop {
        let message = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
            .await
            .expect("a message within 10 seconds")
            .expect("the socket is open")
            .unwrap();
        match message {
            Message::Text(text) => return serde_json::from_str(text.as_str()).unwrap(),
            Message::Close(frame) => {
                return json!({ "closed": frame.map(|f| u16::from(f.code)) });
            }
            _ => {}
        }
    }
}

#[tokio::test]
async fn record_uploads_are_bounded_and_follow_the_last_recorded_turn() {
    let server = Server::start().await;
    let (_, run_id) = server.task_with_run().await;
    let tenant = server.tenant();
    let signer = RunTokenSigner::new(Signer::new(RUN_KEY).unwrap());
    let token = signer.sign(tenant, run_id, Utc::now() + chrono::Duration::hours(1));
    let mut socket = connect(&server, &token).await.unwrap();
    let run = run_id.to_string();
    send(
        &mut socket,
        json!({ "type": "hello", "runId": run, "protocolVersion": 1, "instanceId": Uuid::new_v4().to_string(), "started": false, "sessionId": null }),
    )
    .await;
    assert_eq!(receive(&mut socket).await["type"], "ack");
    let sha = "0".repeat(64);
    let mut upload = async |seq: i64, turn: i64, length: i64| {
        send(
            &mut socket,
            json!({ "type": "record_upload_request", "runId": run, "seq": seq, "requestId": format!("u-{seq}"), "sessionId": "s-1", "turn": turn, "contentLength": length, "sha256": sha }),
        )
        .await;
        assert_eq!(
            receive(&mut socket).await,
            json!({ "type": "ack", "runId": run, "seq": seq })
        );
        let reply = receive(&mut socket).await;
        assert_eq!(reply["type"], "record_upload_response");
        if reply["ok"] == true {
            Ok(reply["key"].as_str().unwrap().to_string())
        } else {
            Err(reply["error"]["code"].as_str().unwrap().to_string())
        }
    };
    assert_eq!(upload(1, 2, 2).await, Err("unexpected_turn".into()));
    assert_eq!(upload(2, 0, 2).await, Err("unexpected_turn".into()));
    assert_eq!(
        upload(3, 1, 64 * 1024 * 1024 + 1).await,
        Err("record_too_large".into())
    );
    assert_eq!(upload(4, 1, -1).await, Err("invalid_request".into()));
    let key = upload(5, 1, 64 * 1024 * 1024).await.unwrap();
    drop(upload);
    send(
        &mut socket,
        json!({ "type": "session_record", "runId": run, "seq": 6, "sessionId": "s-1", "turn": 1, "key": key, "relativePath": "s-1.jsonl", "contentLength": 2, "sha256": sha }),
    )
    .await;
    assert_eq!(
        receive(&mut socket).await,
        json!({ "type": "ack", "runId": run, "seq": 6 })
    );
    let mut upload = async |seq: i64, turn: i64| {
        send(
            &mut socket,
            json!({ "type": "record_upload_request", "runId": run, "seq": seq, "requestId": format!("u-{seq}"), "sessionId": "s-1", "turn": turn, "contentLength": 2, "sha256": sha }),
        )
        .await;
        receive(&mut socket).await;
        receive(&mut socket).await["ok"].clone()
    };
    assert_eq!(upload(7, 1).await, json!(false));
    assert_eq!(upload(8, 2).await, json!(true));
}

#[tokio::test]
async fn the_run_channel_delivers_the_outbox_and_dedupes_sequenced_messages() {
    let server = Server::start().await;
    let (task_id, run_id) = server.task_with_run().await;
    let tenant = server.tenant();
    let signer = RunTokenSigner::new(Signer::new(RUN_KEY).unwrap());
    let token = signer.sign(tenant, run_id, Utc::now() + chrono::Duration::hours(1));
    assert_eq!(connect(&server, "bogus").await.err(), Some(401));
    let expired = signer.sign(tenant, run_id, Utc::now() - chrono::Duration::seconds(1));
    assert_eq!(connect(&server, &expired).await.err(), Some(401));
    let unknown = signer.sign(
        tenant,
        Uuid::new_v4(),
        Utc::now() + chrono::Duration::hours(1),
    );
    assert_eq!(connect(&server, &unknown).await.err(), Some(403));

    let now = Utc::now();
    for (seq, kind, payload) in [
        (
            1,
            "start",
            json!({
                "agent": "claude", "model": "claude-sonnet-5-5", "systemPrompt": null, "skills": [],
                "instruction": "Do the thing.", "resume": null, "heartbeatSeconds": 60,
                "branch": "sanchoris/x", "repositories": ["acme/shop-web"],
            }),
        ),
        (2, "user_message", json!({ "text": "Hello." })),
        (
            3,
            "chat_post",
            json!({ "text": "Posted.", "channel": null }),
        ),
    ] {
        server
            .dynamo
            .put(&OutboxItem {
                id: Uuid::new_v4(),
                tenant_id: tenant,
                run_id,
                seq,
                kind: kind.into(),
                payload,
                dedupe_key: format!("{kind}:1"),
                created_at: now,
                delivered_at: None,
            })
            .await;
    }

    let mut socket = connect(&server, &token).await.unwrap();
    let run = run_id.to_string();
    let instance = Uuid::new_v4().to_string();
    send(
        &mut socket,
        json!({ "type": "ready", "runId": run, "seq": 1, "sessionId": "s-0", "agent": "claude", "model": "m", "resumed": false }),
    )
    .await;
    assert_eq!(
        receive(&mut socket).await,
        json!({ "type": "close", "runId": run, "code": CLOSE_PROTOCOL_ERROR, "reason": "the first message must be hello" })
    );
    assert_eq!(
        receive(&mut socket).await,
        json!({ "closed": CLOSE_PROTOCOL_ERROR })
    );

    let mut socket = connect(&server, &token).await.unwrap();
    send(
        &mut socket,
        json!({ "type": "hello", "runId": run, "protocolVersion": 1, "instanceId": instance, "started": false, "sessionId": null }),
    )
    .await;
    assert_eq!(
        receive(&mut socket).await,
        json!({ "type": "ack", "runId": run, "seq": 0 })
    );
    let start = receive(&mut socket).await;
    assert_eq!(
        start,
        json!({
            "type": "start", "runId": run, "agent": "claude", "model": "claude-sonnet-5-5", "systemPrompt": null,
            "skills": [], "instruction": "Do the thing.", "resume": null, "heartbeatSeconds": 60,
        })
    );
    let message = receive(&mut socket).await;
    assert_eq!(message["type"], "user_message");
    assert_eq!(message["text"], "Hello.");

    let ready = json!({ "type": "ready", "runId": run, "seq": 1, "sessionId": "s-1", "agent": "claude", "model": "m", "resumed": false });
    send(&mut socket, ready.clone()).await;
    assert_eq!(
        receive(&mut socket).await,
        json!({ "type": "ack", "runId": run, "seq": 1 })
    );
    send(&mut socket, ready).await;
    assert_eq!(
        receive(&mut socket).await,
        json!({ "type": "ack", "runId": run, "seq": 1 })
    );
    send(
        &mut socket,
        json!({ "type": "skill_request", "runId": run, "seq": 2, "requestId": "r-1", "action": "run", "name": "deploy", "input": {} }),
    )
    .await;
    assert_eq!(
        receive(&mut socket).await,
        json!({ "type": "ack", "runId": run, "seq": 2 })
    );
    let reply = receive(&mut socket).await;
    assert_eq!(reply["type"], "skill_response");
    assert_eq!(reply["requestId"], "r-1");
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "skill_not_offered");
    send(
        &mut socket,
        json!({ "type": "record_upload_request", "runId": run, "seq": 3, "requestId": "u-1", "sessionId": "s-1", "turn": 1, "contentLength": 2, "sha256": "0".repeat(64) }),
    )
    .await;
    receive(&mut socket).await;
    let upload = receive(&mut socket).await;
    assert_eq!(upload["ok"], true);
    assert_eq!(
        upload["key"],
        format!("t/{tenant}/runs/{run_id}/s-1/turn-1.jsonl")
    );
    send(
        &mut socket,
        json!({ "type": "event", "runId": run, "seq": 4, "turn": 1, "kind": "text", "data": {
            "text": format!(
                "push with ghs_{} as AKIA{}, task-0123456789abcdefghijklmn stays",
                "x".repeat(36),
                "Q".repeat(16)
            ),
            "calls": [{ "env": format!("ANTHROPIC_API_KEY=sk-ant-{}", "y".repeat(30)), "count": 2 }],
        }, "at": "2026-10-27T03:15:02.000Z" }),
    )
    .await;
    receive(&mut socket).await;
    send(
        &mut socket,
        json!({ "type": "turn_end", "runId": run, "seq": 9, "turn": 1 }),
    )
    .await;
    assert_eq!(
        receive(&mut socket).await,
        json!({ "type": "close", "runId": run, "code": CLOSE_PROTOCOL_ERROR, "reason": "expected seq 5, received 9" })
    );
    assert_eq!(
        receive(&mut socket).await,
        json!({ "closed": CLOSE_PROTOCOL_ERROR })
    );

    let events = server
        .dynamo
        .all::<RunEventItem>()
        .await
        .into_iter()
        .filter(|event| event.run_id == run_id)
        .map(|event| (event.seq, event.kind))
        .collect::<Vec<_>>();
    assert_eq!(events, vec![(Some(1), "ready".to_string())]);
    let logs = server.dynamo.all::<RunLogEntry>().await;
    assert_eq!(logs.len(), 1);
    assert_eq!(
        logs[0].data,
        json!({
            "text": "push with <redacted> as <redacted>, task-0123456789abcdefghijklmn stays",
            "calls": [{ "env": "ANTHROPIC_API_KEY=<redacted>", "count": 2 }],
        })
    );
    let undelivered = server
        .dynamo
        .all::<OutboxItem>()
        .await
        .into_iter()
        .filter(|item| item.seq <= 3 && item.delivered_at.is_none())
        .collect::<Vec<_>>();
    assert_eq!(undelivered, vec![]);
    let chat = server.dynamo.all::<ChatMessage>().await;
    assert_eq!(chat.len(), 1);
    assert_eq!(
        (chat[0].text.as_str(), chat[0].task_id),
        ("Posted.", task_id)
    );

    let mut second = connect(&server, &token).await.unwrap();
    let next = Uuid::new_v4().to_string();
    send(
        &mut second,
        json!({ "type": "hello", "runId": run, "protocolVersion": 1, "instanceId": next, "started": true, "sessionId": "s-1" }),
    )
    .await;
    assert_eq!(
        receive(&mut second).await,
        json!({ "type": "ack", "runId": run, "seq": 0 })
    );
    send(
        &mut second,
        json!({ "type": "ready", "runId": run, "seq": 1, "sessionId": "s-1", "agent": "claude", "model": "m", "resumed": true }),
    )
    .await;
    assert_eq!(
        receive(&mut second).await,
        json!({ "type": "ack", "runId": run, "seq": 1 })
    );
    let mut seqs = server
        .dynamo
        .all::<RunEventItem>()
        .await
        .into_iter()
        .filter(|event| event.run_id == run_id && event.kind == "ready")
        .map(|event| event.seq)
        .collect::<Vec<_>>();
    seqs.sort();
    assert_eq!(seqs, vec![Some(1), Some(SEQ_STRIDE + 1)]);
}
