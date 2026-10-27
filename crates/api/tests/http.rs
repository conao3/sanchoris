mod support;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use reqwest::StatusCode;
use reqwest::redirect::Policy;
use sanchoris_api::app::{AppState, router};
use sanchoris_api::auth::{AuthError, Identity, IdentityVerifier};
use sanchoris_api::config::{CognitoConfig, GitHubConfig, RecordsConfig, ServeConfig};
use sanchoris_api::records::{LocalRecordStore, RecordStore, record_key};
use sanchoris_api::signing::HmacKey;
use sanchoris_core::billing::PAYMENT_PENDING;
use sanchoris_core::testing::TestPostgres;
use sanchoris_core::tokens::{RunTokenSigner, Signer, TaskTokenSigner};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use support::fake_github::FakeGitHub;
use support::{Seeded, listen, seed};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

const RUN_KEY: &[u8] = b"http-test-run-token-signing-key-0123";
const STRIPE_SECRET: &str = "whsec_http";

struct StubVerifier;

#[async_trait]
impl IdentityVerifier for StubVerifier {
    async fn verify(&self, token: &str) -> Result<Identity, AuthError> {
        let subject = token.strip_prefix("user:").ok_or(AuthError::Unauthorized)?;
        Ok(Identity {
            subject: subject.to_string(),
            email: Some(format!("{subject}@example.com")),
            display_name: subject.to_string(),
        })
    }
}

struct Server {
    url: String,
    http: reqwest::Client,
    seeded: Seeded,
    github: Arc<FakeGitHub>,
    records: Arc<LocalRecordStore>,
    _pg: TestPostgres,
    _dir: tempfile::TempDir,
}

impl Server {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let github = FakeGitHub::new(&dir.path().join("git"), BTreeMap::new());
        let github_url = listen(github.clone().router()).await;
        let pg = TestPostgres::start().await;
        let seeded = seed(&pg).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address: SocketAddr = listener.local_addr().unwrap();
        let url = format!("http://{address}");
        let records_key = HmacKey::strong(&[5u8; 32]).unwrap();
        let records_dir: PathBuf = dir.path().join("records");
        let config = ServeConfig {
            bind_addr: address,
            database_url: pg.url(sanchoris_core::testing::WORKER_ROLE),
            public_url: url.clone(),
            cognito: CognitoConfig {
                user_pool_id: "us-east-1_http".into(),
                client_id: "http-client".into(),
            },
            github: GitHubConfig {
                webhook_secret: HmacKey::new(b"http-webhook").unwrap(),
                client_id: "Iv1.http".into(),
                client_secret: "http-secret".into(),
                web_url: github_url.clone(),
                api_url: github_url,
            },
            stripe_webhook_secret: HmacKey::new(STRIPE_SECRET.as_bytes()).unwrap(),
            run_tokens: RunTokenSigner::new(Signer::new(RUN_KEY).unwrap()),
            task_tokens: TaskTokenSigner::new(Signer::new(&[6u8; 32]).unwrap()),
            oauth_state_key: HmacKey::strong(&[7u8; 32]).unwrap(),
            records: RecordsConfig::Local {
                dir: records_dir.clone(),
                signing_key: records_key.clone(),
            },
        };
        let records = Arc::new(LocalRecordStore::new(&records_dir, records_key, &url));
        let http = reqwest::Client::builder()
            .redirect(Policy::none())
            .build()
            .unwrap();
        let state = AppState::new(
            seeded.pool.clone(),
            config,
            Arc::new(StubVerifier),
            records.clone() as Arc<dyn RecordStore>,
            http.clone(),
        );
        let app = router(state, Some(records.clone()));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            url,
            http,
            seeded,
            github,
            records,
            _pg: pg,
            _dir: dir,
        }
    }

    async fn get(&self, path: &str, token: Option<&str>) -> reqwest::Response {
        let mut request = self.http.get(format!("{}{path}", self.url));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.unwrap()
    }

    async fn post(&self, path: &str, token: Option<&str>, body: &Value) -> reqwest::Response {
        let mut request = self.http.post(format!("{}{path}", self.url)).json(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.unwrap()
    }

    async fn task_with_run(&self) -> (Uuid, Uuid) {
        let admin = &self.seeded.admin;
        let tenant = &self.seeded.tenant;
        let definition = json!({
            "Trigger": { "Name": "labeled-issue", "Definition": {} },
            "Session": { "Name": "shop", "Definition": {} },
            "Skills": [],
        });
        let task_id: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.tasks (tenant_id, trigger_id, config_revision_id, name, title, status, definition, started_at) \
             VALUES ($1, $2, $3, 'acme/shop-web#1', 'Task', 'RUNNING', $4, now()) RETURNING id",
        )
        .bind(tenant.tenant_id)
        .bind(tenant.trigger_id)
        .bind(tenant.config_revision_id)
        .bind(definition)
        .fetch_one(admin)
        .await
        .unwrap();
        let run_id: Uuid = sqlx::query_scalar(
            "INSERT INTO sanchoris.runs (tenant_id, task_id, session_name, worker, model, status) \
             VALUES ($1, $2, 'shop', 'claude', 'claude-sonnet-5-5', 'creating') RETURNING id",
        )
        .bind(tenant.tenant_id)
        .bind(task_id)
        .fetch_one(admin)
        .await
        .unwrap();
        (task_id, run_id)
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
    let started: Value = server
        .post("/auth/github", Some("user:google_7"), &json!({}))
        .await
        .json()
        .await
        .unwrap();
    let authorize = reqwest::Url::parse(started["url"].as_str().unwrap()).unwrap();
    assert_eq!(authorize.path(), "/login/oauth/authorize");
    let query: BTreeMap<String, String> = authorize.query_pairs().into_owned().collect();
    assert_eq!(query["client_id"], "Iv1.http");
    assert_eq!(
        query["redirect_uri"],
        format!("{}/auth/github/callback", server.url)
    );

    let forged = server
        .get(
            &format!(
                "/auth/github/callback?code=code-1&state={}",
                query["state"].replace('.', "x")
            ),
            None,
        )
        .await;
    assert_eq!(forged.status(), StatusCode::BAD_REQUEST);

    let linked = server
        .get(
            &format!("/auth/github/callback?code=code-1&state={}", query["state"]),
            None,
        )
        .await;
    assert_eq!(linked.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        linked.headers()["location"],
        format!("{}/settings/integrations?github=linked", server.url).as_str()
    );
    let me: Value = server
        .get("/api/me", Some("user:google_7"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(me["githubLogin"], "sota-dev");

    let other: Value = server
        .post("/auth/github", Some("user:google_8"), &json!({}))
        .await
        .json()
        .await
        .unwrap();
    let other_state = reqwest::Url::parse(other["url"].as_str().unwrap())
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let in_use = server
        .get(
            &format!("/auth/github/callback?code=code-1&state={other_state}"),
            None,
        )
        .await;
    assert_eq!(
        in_use.headers()["location"],
        format!("{}/settings/integrations?github=in_use", server.url).as_str()
    );
    let denied = server
        .get(
            &format!("/auth/github/callback?error=access_denied&state={other_state}"),
            None,
        )
        .await;
    assert_eq!(
        denied.headers()["location"],
        format!("{}/settings/integrations?github=denied", server.url).as_str()
    );
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
        .http
        .post(format!("{}/webhooks/stripe", server.url))
        .header(
            "Stripe-Signature",
            signature.unwrap_or_else(|| stripe_header(&body)),
        )
        .body(body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn stripe_webhooks_are_verified_and_update_the_tenant() {
    let server = Server::start().await;
    let tenant_id = server.seeded.tenant.tenant_id;
    sqlx::query(
        "UPDATE sanchoris.tenants SET status = 'suspended', suspended_reason = $2 WHERE id = $1",
    )
    .bind(tenant_id)
    .bind(PAYMENT_PENDING)
    .execute(&server.seeded.admin)
    .await
    .unwrap();
    let checkout = json!({
        "id": "evt_1", "type": "checkout.session.completed", "created": Utc::now().timestamp(),
        "data": { "object": { "client_reference_id": tenant_id, "customer": "cus_http" } },
    });
    assert_eq!(
        stripe(&server, &checkout, Some("t=1,v1=00".into()))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let response = stripe(&server, &checkout, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<Value>().await.unwrap()["status"],
        "processed"
    );
    let row = sqlx::query("SELECT status, stripe_customer_id FROM sanchoris.tenants WHERE id = $1")
        .bind(tenant_id)
        .fetch_one(&server.seeded.pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("status"), "active");
    assert_eq!(
        row.get::<Option<String>, _>("stripe_customer_id")
            .as_deref(),
        Some("cus_http")
    );

    let failed = json!({
        "id": "evt_2", "type": "invoice.payment_failed", "created": Utc::now().timestamp(),
        "data": { "object": { "customer": "cus_http" } },
    });
    assert_eq!(
        stripe(&server, &failed, None).await.status(),
        StatusCode::OK
    );
    let status: String = sqlx::query_scalar("SELECT status FROM sanchoris.tenants WHERE id = $1")
        .bind(tenant_id)
        .fetch_one(&server.seeded.pool)
        .await
        .unwrap();
    assert_eq!(status, "suspended");

    let unknown = json!({
        "id": "evt_3", "type": "invoice.paid", "created": Utc::now().timestamp(),
        "data": { "object": { "customer": "cus_unknown" } },
    });
    assert_eq!(
        stripe(&server, &unknown, None).await.status(),
        StatusCode::CONFLICT
    );
    let ignored = json!({ "id": "evt_4", "type": "customer.created", "data": { "object": {} } });
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
async fn replies_and_approvals_need_a_member_of_the_tenant() {
    let server = Server::start().await;
    let (task_id, run_id) = server.task_with_run().await;
    let subject: String =
        sqlx::query_scalar("SELECT cognito_subject FROM sanchoris.users WHERE id = $1")
            .bind(server.seeded.tenant.user_id)
            .fetch_one(&server.seeded.pool)
            .await
            .unwrap();
    let member = format!("user:{subject}");
    let path = format!("/api/tasks/{task_id}/replies");
    assert_eq!(
        server
            .post(&path, Some("user:stranger"), &json!({ "text": "hi" }))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let response = server
        .post(
            &path,
            Some(&member),
            &json!({ "text": "Use the staging database." }),
        )
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let payload: Value = sqlx::query_scalar(
        "SELECT payload FROM sanchoris.run_events WHERE run_id = $1 AND type = 'human_reply'",
    )
    .bind(run_id)
    .fetch_one(&server.seeded.pool)
    .await
    .unwrap();
    assert_eq!(
        payload,
        json!({ "member": "Mika", "text": "Use the staging database." })
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
    let signed = TaskTokenSigner::new(Signer::new(&[6u8; 32]).unwrap())
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
async fn local_records_accept_only_signed_uploads_of_the_announced_content() {
    let server = Server::start().await;
    let tenant = server.seeded.tenant.tenant_id;
    let key = record_key(tenant, Uuid::new_v4(), "session-1", 1).unwrap();
    let body = b"{\"role\":\"user\"}\n".to_vec();
    let sha = hex::encode(Sha256::digest(&body));
    let put = server
        .records
        .presign_put(tenant, &key, body.len() as i64, &sha)
        .await
        .unwrap();
    assert!(
        server
            .records
            .presign_put(Uuid::new_v4(), &key, 1, &sha)
            .await
            .is_err()
    );
    let wrong = server
        .http
        .put(&put.url)
        .body(b"tampered\n".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
    let forged = server
        .http
        .put(put.url.replace("length=", "length=1"))
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(forged.status(), StatusCode::FORBIDDEN);
    let uploaded = server
        .http
        .put(&put.url)
        .header("content-type", &put.headers["content-type"])
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(uploaded.status(), StatusCode::OK);
    let url = server.records.presign_get(tenant, &key).await.unwrap();
    let downloaded = server.http.get(&url).send().await.unwrap();
    assert_eq!(downloaded.bytes().await.unwrap().to_vec(), body);
    let unsigned = server
        .http
        .get(url.split('?').next().unwrap())
        .send()
        .await
        .unwrap();
    assert!(unsigned.status().is_client_error());
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(server: &Server, token: &str) -> Result<Socket, u16> {
    let mut request = format!("{}/runs/channel", server.url.replace("http", "ws"))
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
async fn the_run_channel_delivers_the_outbox_and_dedupes_sequenced_messages() {
    let server = Server::start().await;
    let (_, run_id) = server.task_with_run().await;
    let tenant = server.seeded.tenant.tenant_id;
    let signer = RunTokenSigner::new(Signer::new(RUN_KEY).unwrap());
    let token = signer.sign(run_id, Utc::now() + chrono::Duration::hours(1));
    assert_eq!(connect(&server, "bogus").await.err(), Some(401));
    let expired = signer.sign(run_id, Utc::now() - chrono::Duration::seconds(1));
    assert_eq!(connect(&server, &expired).await.err(), Some(401));
    let unknown = signer.sign(Uuid::new_v4(), Utc::now() + chrono::Duration::hours(1));
    assert_eq!(connect(&server, &unknown).await.err(), Some(403));

    for (kind, payload, dedupe) in [
        (
            "start",
            json!({
                "agent": "claude", "model": "claude-sonnet-5-5", "systemPrompt": null, "skills": [],
                "instruction": "Do the thing.", "resume": null, "heartbeatSeconds": 60,
                "branch": "sanchoris/x", "repositories": ["acme/shop-web"],
            }),
            "start:1",
        ),
        ("user_message", json!({ "text": "Hello." }), "m:1"),
        (
            "chat_post",
            json!({ "text": "Posted.", "channel": null }),
            "chat:1",
        ),
    ] {
        sqlx::query(
            "INSERT INTO sanchoris.run_outbox (tenant_id, run_id, type, payload, dedupe_key) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(tenant)
        .bind(run_id)
        .bind(kind)
        .bind(payload)
        .bind(dedupe)
        .execute(&server.seeded.admin)
        .await
        .unwrap();
    }

    let mut socket = connect(&server, &token).await.unwrap();
    let run = run_id.to_string();
    let instance = Uuid::new_v4().to_string();
    send(
        &mut socket,
        json!({ "type": "hello", "runId": run, "protocolVersion": 1, "instanceId": instance, "started": false, "sessionId": null }),
    )
    .await;
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
        json!({ "type": "event", "runId": run, "seq": 4, "turn": 1, "kind": "text", "data": { "text": "hi" }, "at": "2026-10-27T03:15:02.000Z" }),
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
        json!({ "closed": sanchoris_api::channel::CLOSE_PROTOCOL_ERROR })
    );

    let events: Vec<(Option<i64>, String)> =
        sqlx::query_as("SELECT seq, type FROM sanchoris.run_events WHERE run_id = $1 ORDER BY seq")
            .bind(run_id)
            .fetch_all(&server.seeded.pool)
            .await
            .unwrap();
    assert_eq!(events, vec![(Some(1), "ready".to_string())]);
    let logs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sanchoris.run_log_entries WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&server.seeded.pool)
            .await
            .unwrap();
    assert_eq!(logs, 1);
    let undelivered: Vec<String> = sqlx::query_scalar(
        "SELECT type FROM sanchoris.run_outbox WHERE run_id = $1 AND delivered_at IS NULL ORDER BY seq",
    )
    .bind(run_id)
    .fetch_all(&server.seeded.pool)
    .await
    .unwrap();
    assert_eq!(undelivered, vec!["chat_post".to_string()]);
    assert_eq!(
        sanchoris_api::chat::relay_chat_posts(&server.seeded.pool)
            .await
            .unwrap(),
        1
    );
    let chat: String = sqlx::query_scalar("SELECT text FROM sanchoris.chat_messages")
        .fetch_one(&server.seeded.pool)
        .await
        .unwrap();
    assert_eq!(chat, "Posted.");

    let mut second = connect(&server, &token).await.unwrap();
    let next = Uuid::new_v4().to_string();
    send(
        &mut second,
        json!({ "type": "hello", "runId": run, "protocolVersion": 1, "instanceId": next, "started": true, "sessionId": "s-1" }),
    )
    .await;
    send(
        &mut second,
        json!({ "type": "ready", "runId": run, "seq": 1, "sessionId": "s-1", "agent": "claude", "model": "m", "resumed": true }),
    )
    .await;
    assert_eq!(
        receive(&mut second).await,
        json!({ "type": "ack", "runId": run, "seq": 1 })
    );
    let seqs: Vec<Option<i64>> = sqlx::query_scalar(
        "SELECT seq FROM sanchoris.run_events WHERE run_id = $1 AND type = 'ready' ORDER BY seq",
    )
    .bind(run_id)
    .fetch_all(&server.seeded.pool)
    .await
    .unwrap();
    assert_eq!(
        seqs,
        vec![Some(1), Some(sanchoris_api::channel::SEQ_STRIDE + 1)]
    );
}
