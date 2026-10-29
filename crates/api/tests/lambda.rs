use crate::support;

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use sanchoris_api::channel::credentials::NoRunCredentials;
use sanchoris_api::channel::{Channel, ConnectionSender, SendError};
use sanchoris_api::config::AllowedSources;
use sanchoris_api::graphql;
use sanchoris_api::lambda::{
    self, AuthorizerEvent, BatchResponse, ItemFailure, LambdaError, SqsEvent, StreamEvent,
    WebSocketEvent, WebSocketResponse,
};
use sanchoris_api::records::LocalRecordStore;
use sanchoris_api::signing::HmacKey;
use sanchoris_core::clock::{Clock, SystemClock};
use sanchoris_core::items::{OutboxItem, RunChannel, RunEventItem};
use sanchoris_core::jobs::{Envelope, Job, Relayed};
use sanchoris_core::keys;
use sanchoris_core::local::{MemoryBlobStore, MemoryQueue, MemoryScheduler};
use sanchoris_core::testing::TestDynamo;
use sanchoris_core::tokens::{RunTokenSigner, Signer};
use sanchoris_graphql::ClientIp;
use sanchoris_worker::github::transport::ReqwestTransport;
use sanchoris_worker::github::{AppAuth, GitHubApi};
use sanchoris_worker::resources::GitHubResources;
use sanchoris_worker::{Deps, Settings, Worker};
use serde_json::{Value, json};
use support::{NoWorkspaces, RUN_KEY, rsa_key, seed, task_with_run};
use tower::ServiceExt;
use uuid::Uuid;

#[derive(Default)]
struct FakeSender {
    sent: Mutex<Vec<(String, Value)>>,
    closed: Mutex<Vec<(String, u16)>>,
    gone: Mutex<HashSet<String>>,
}

impl FakeSender {
    fn sent_to(&self, connection: &str) -> Vec<Value> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, _)| id == connection)
            .map(|(_, message)| message.clone())
            .collect()
    }
}

#[async_trait]
impl ConnectionSender for FakeSender {
    async fn send(&self, connection_id: &str, message: &Value) -> Result<(), SendError> {
        if self.gone.lock().unwrap().contains(connection_id) {
            return Err(SendError::Gone);
        }
        self.sent
            .lock()
            .unwrap()
            .push((connection_id.to_string(), message.clone()));
        Ok(())
    }

    async fn close(&self, connection_id: &str, code: u16, _reason: &str) -> Result<(), SendError> {
        self.closed
            .lock()
            .unwrap()
            .push((connection_id.to_string(), code));
        Ok(())
    }
}

fn fixture(name: &str, vars: &[(&str, String)]) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let mut text = std::fs::read_to_string(path).unwrap();
    for (name, value) in vars {
        text = text.replace(&format!("{{{{{name}}}}}"), value);
    }
    serde_json::from_str(&text).unwrap()
}

fn websocket_event(name: &str, vars: &[(&str, String)], body: Option<&Value>) -> WebSocketEvent {
    let mut event = fixture(name, vars);
    if let Some(body) = body {
        event["body"] = Value::String(body.to_string());
    }
    serde_json::from_value(event).unwrap()
}

fn sqs_event(vars: &[(&str, String)], bodies: &[(&str, String)]) -> SqsEvent {
    let mut event = fixture("sqs.json", vars);
    let template = event["Records"][0].clone();
    event["Records"] = bodies
        .iter()
        .map(|(id, body)| {
            let mut record = template.clone();
            record["messageId"] = json!(id);
            record["body"] = json!(body);
            record
        })
        .collect();
    serde_json::from_value(event).unwrap()
}

fn stream_event(records: &[(&str, &str, String)]) -> StreamEvent {
    let mut event = fixture("stream.json", &[]);
    let template = event["Records"][0].clone();
    event["Records"] = records
        .iter()
        .map(|(sequence, name, body)| {
            let mut record = template.clone();
            record["eventName"] = json!(name);
            record["dynamodb"]["SequenceNumber"] = json!(sequence);
            record["dynamodb"]["NewImage"]["body"]["S"] = json!(body);
            record
        })
        .collect();
    serde_json::from_value(event).unwrap()
}

fn failures(identifiers: &[&str]) -> BatchResponse {
    BatchResponse {
        batch_item_failures: identifiers
            .iter()
            .map(|identifier| ItemFailure {
                item_identifier: identifier.to_string(),
            })
            .collect(),
    }
}

#[tokio::test]
async fn the_lambda_handlers_run_the_channel_from_recorded_events() {
    let dynamo = TestDynamo::start().await;
    let seeded = seed(&dynamo).await;
    let tenant = seeded.tenant.tenant_id;
    let (_, run_id) = task_with_run(&dynamo, &seeded).await;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let direct = dynamo.direct_store(clock.clone());
    let dir = tempfile::tempdir().unwrap();
    let sender = Arc::new(FakeSender::default());
    let run_tokens = RunTokenSigner::new(Signer::new(RUN_KEY).unwrap());
    let channel = Channel {
        store: direct.store.clone(),
        run_tokens: run_tokens.clone(),
        records: Arc::new(LocalRecordStore::new(
            dir.path(),
            HmacKey::strong(&[5u8; 32]).unwrap(),
            "https://records.example",
        )),
        sender: sender.clone(),
        credentials: Arc::new(NoRunCredentials),
    };
    dynamo
        .put(&OutboxItem {
            id: Uuid::new_v4(),
            tenant_id: tenant,
            run_id,
            seq: 1,
            kind: "start".into(),
            payload: json!({
                "agent": "claude", "model": "claude-sonnet-5-5", "systemPrompt": null, "skills": [],
                "instruction": "Do the thing.", "resume": null, "heartbeatSeconds": 60,
            }),
            dedupe_key: "start:1".into(),
            created_at: Utc::now(),
            delivered_at: None,
        })
        .await;

    let token = run_tokens.sign(tenant, run_id, Utc::now() + Duration::hours(1));
    let connection = "L0SM9cOFvHcCIhw=".to_string();
    let vars = [
        ("token", token.clone()),
        ("connectionId", connection.clone()),
        ("tenantId", tenant.to_string()),
        ("runId", run_id.to_string()),
    ];
    let authorizer: AuthorizerEvent =
        serde_json::from_value(fixture("authorizer.json", &vars)).unwrap();
    let allowed = lambda::authorize(
        &direct.store,
        &run_tokens,
        AllowedSources::Any,
        &authorizer,
        Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(
        allowed["policyDocument"]["Statement"][0],
        json!({ "Action": "execute-api:Invoke", "Effect": "Allow", "Resource": authorizer.method_arn })
    );
    assert_eq!(
        allowed["context"],
        json!({ "tenantId": tenant.to_string(), "runId": run_id.to_string() })
    );
    let forged: AuthorizerEvent = serde_json::from_value(fixture(
        "authorizer.json",
        &[
            ("token", format!("{token}0")),
            ("connectionId", connection.clone()),
        ],
    ))
    .unwrap();
    let rejected = lambda::authorize(
        &direct.store,
        &run_tokens,
        AllowedSources::Any,
        &forged,
        Utc::now(),
    )
    .await;
    assert!(matches!(rejected, Err(LambdaError::Unauthorized)));
    assert_eq!(rejected.unwrap_err().to_string(), "Unauthorized");
    let other_run = run_tokens.sign(tenant, Uuid::new_v4(), Utc::now() + Duration::hours(1));
    let unknown: AuthorizerEvent = serde_json::from_value(fixture(
        "authorizer.json",
        &[("token", other_run), ("connectionId", connection.clone())],
    ))
    .unwrap();
    let denied = lambda::authorize(
        &direct.store,
        &run_tokens,
        AllowedSources::Any,
        &unknown,
        Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(
        denied["policyDocument"]["Statement"][0]["Effect"],
        json!("Deny")
    );
    let direct_source = lambda::authorize(
        &direct.store,
        &run_tokens,
        AllowedSources::Cloudflare,
        &authorizer,
        Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(
        direct_source["policyDocument"]["Statement"][0]["Effect"],
        json!("Deny")
    );
    let mut via_cloudflare = authorizer.clone();
    via_cloudflare.request_context.identity.source_ip = "172.70.1.2".into();
    let admitted = lambda::authorize(
        &direct.store,
        &run_tokens,
        AllowedSources::Cloudflare,
        &via_cloudflare,
        Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(
        admitted["policyDocument"]["Statement"][0]["Effect"],
        json!("Allow")
    );

    let ok = WebSocketResponse { status_code: 200 };
    let connect = websocket_event("websocket-connect.json", &vars, None);
    assert_eq!(lambda::websocket(&channel, &connect).await.unwrap(), ok);
    let channel_key = keys::run_channel_key(tenant, run_id);
    let stored = dynamo
        .store(sanchoris_core::store::Relay::Stream)
        .get::<RunChannel>(&channel_key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.connection_id.as_deref(), Some(connection.as_str()));

    let run = run_id.to_string();
    let hello = json!({ "type": "hello", "runId": run, "protocolVersion": 1, "instanceId": Uuid::new_v4(), "started": false, "sessionId": null });
    let message = websocket_event("websocket-message.json", &vars, Some(&hello));
    assert_eq!(lambda::websocket(&channel, &message).await.unwrap(), ok);
    let delivery = direct
        .queue
        .bodies()
        .into_iter()
        .find(|body| {
            Envelope::parse(body).unwrap().job
                == Job::DeliverOutbox {
                    tenant_id: tenant,
                    run_id,
                }
        })
        .expect("hello asks for the outbox");

    let github = Arc::new(GitHubApi::new(
        "http://127.0.0.1:9",
        Arc::new(ReqwestTransport::new()),
        AppAuth::new("1", &rsa_key()).unwrap(),
    ));
    let worker = Worker::new(
        direct.store.clone(),
        Deps {
            clock: clock.clone(),
            github: github.clone(),
            reader: github.clone(),
            config_source: github.clone(),
            resources: Arc::new(GitHubResources::new(github)),
            runner: Arc::new(NoWorkspaces),
            pusher: Arc::new(channel.clone()),
            records: Arc::new(MemoryBlobStore::default()),
        },
        Settings::default(),
    );
    let batch = sqs_event(
        &vars,
        &[
            ("message-1", delivery),
            ("message-2", "not an envelope".into()),
            ("message-3", Envelope::new(Job::SweepChecks).body()),
        ],
    );
    assert_eq!(
        lambda::work(&worker, &batch).await,
        failures(&["message-2", "message-3"])
    );
    let sent = sender.sent_to(&connection);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0], json!({ "type": "ack", "runId": run, "seq": 0 }));
    assert_eq!(sent[1]["type"], "start");
    assert_eq!(sent[1]["instruction"], "Do the thing.");

    let ready = json!({ "type": "ready", "runId": run, "seq": 1, "sessionId": "s-1", "agent": "claude", "model": "m", "resumed": false });
    for _ in 0..2 {
        let message = websocket_event("websocket-message.json", &vars, Some(&ready));
        assert_eq!(lambda::websocket(&channel, &message).await.unwrap(), ok);
        assert_eq!(
            sender.sent_to(&connection).last().unwrap(),
            &json!({ "type": "ack", "runId": run, "seq": 1 })
        );
    }
    let readies = dynamo
        .all::<RunEventItem>()
        .await
        .into_iter()
        .filter(|event| event.run_id == run_id && event.kind == "ready")
        .count();
    assert_eq!(readies, 1);

    dynamo
        .put(&OutboxItem {
            id: Uuid::new_v4(),
            tenant_id: tenant,
            run_id,
            seq: 2,
            kind: "user_message".into(),
            payload: json!({ "text": "Hello." }),
            dedupe_key: "m:1".into(),
            created_at: Utc::now(),
            delivered_at: None,
        })
        .await;
    sender.gone.lock().unwrap().insert(connection.clone());
    channel.deliver(tenant, run_id).await.unwrap();
    let store = dynamo.store(sanchoris_core::store::Relay::Stream);
    let stored = store
        .get::<RunChannel>(&channel_key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.connection_id, None);
    let pending = store
        .get::<OutboxItem>(&keys::outbox_key(tenant, run_id, 2))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.delivered_at, None);

    let reconnected = "M1TNadPGwIdDJix=".to_string();
    let next = [
        ("token", token.clone()),
        ("connectionId", reconnected.clone()),
        ("tenantId", tenant.to_string()),
        ("runId", run_id.to_string()),
    ];
    let connect = websocket_event("websocket-connect.json", &next, None);
    assert_eq!(lambda::websocket(&channel, &connect).await.unwrap(), ok);
    let stale = websocket_event("websocket-disconnect.json", &vars, None);
    assert_eq!(lambda::websocket(&channel, &stale).await.unwrap(), ok);
    let stored = store
        .get::<RunChannel>(&channel_key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.connection_id.as_deref(), Some(reconnected.as_str()));
    let out_of_order = json!({ "type": "ready", "runId": run, "seq": 1, "sessionId": "s-1", "agent": "claude", "model": "m", "resumed": true });
    let message = websocket_event("websocket-message.json", &next, Some(&out_of_order));
    assert_eq!(lambda::websocket(&channel, &message).await.unwrap(), ok);
    assert_eq!(
        sender.sent_to(&reconnected),
        vec![
            json!({ "type": "close", "runId": run, "code": 4000, "reason": "the first message must be hello" })
        ]
    );
    assert_eq!(
        sender.closed.lock().unwrap().clone(),
        vec![(reconnected.clone(), 4000)]
    );
    let disconnect = websocket_event("websocket-disconnect.json", &next, None);
    assert_eq!(lambda::websocket(&channel, &disconnect).await.unwrap(), ok);
    let stored = store
        .get::<RunChannel>(&channel_key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.connection_id, None);

    let mut anonymous = fixture("websocket-message.json", &vars);
    anonymous["requestContext"]
        .as_object_mut()
        .unwrap()
        .remove("authorizer");
    let anonymous: WebSocketEvent = serde_json::from_value(anonymous).unwrap();
    assert!(matches!(
        lambda::websocket(&channel, &anonymous).await,
        Err(LambdaError::Unauthorized)
    ));
}

#[tokio::test]
async fn the_stream_relay_sends_due_jobs_and_schedules_later_ones() {
    let now = Utc::now();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let queue = MemoryQueue::new(clock);
    let scheduler = MemoryScheduler::new();
    let tenant_id = Uuid::new_v4();
    let due = Relayed::Send {
        envelope: Envelope::new(Job::Dispatch { tenant_id }),
        deliver_at: None,
    };
    let later = Relayed::Send {
        envelope: Envelope::new(Job::Inbound { tenant_id }),
        deliver_at: Some(now + Duration::minutes(5)),
    };
    let cancel = Relayed::Cancel {
        schedule: "sanchoris-unknown".into(),
    };
    let body = |relayed: &Relayed| serde_json::to_string(relayed).unwrap();
    let event = stream_event(&[
        ("1", "INSERT", body(&due)),
        ("2", "INSERT", body(&later)),
        ("3", "INSERT", body(&cancel)),
        ("4", "MODIFY", body(&due)),
    ]);
    assert_eq!(
        lambda::relay_stream(&queue, &scheduler, now, &event).await,
        BatchResponse::default()
    );
    assert_eq!(queue.len(), 1);
    assert_eq!(
        Envelope::parse(&queue.bodies()[0]).unwrap().job,
        Job::Dispatch { tenant_id }
    );
    assert_eq!(scheduler.pending().len(), 1);

    let broken = stream_event(&[("5", "INSERT", "{".into()), ("6", "INSERT", body(&due))]);
    assert_eq!(
        lambda::relay_stream(&queue, &scheduler, now, &broken).await,
        failures(&["5", "6"])
    );
}

async fn client_ip_of(event: Value) -> (u16, String) {
    let request: lambda_http::request::LambdaRequest = serde_json::from_value(event).unwrap();
    let request =
        lambda_http::Request::from(request).map(|body| axum::body::Body::from(body.to_vec()));
    let router = axum::Router::new()
        .route(
            "/ip",
            axum::routing::get(
                |axum::Extension(ip): axum::Extension<ClientIp>| async move { ip.0.to_string() },
            ),
        )
        .layer(axum::middleware::from_fn(graphql::gateway_ip));
    let response = router.oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn the_api_function_takes_the_client_ip_from_the_http_api_request_context() {
    let event = fixture("http.json", &[("sourceIp", "203.0.113.7".to_string())]);
    assert_eq!(client_ip_of(event).await, (200, "203.0.113.7".to_string()));
    let unparsable = fixture("http.json", &[("sourceIp", "not-an-ip".to_string())]);
    assert_eq!(client_ip_of(unparsable).await.0, 500);

    let mut via_cloudflare = fixture("http.json", &[("sourceIp", "172.70.1.2".to_string())]);
    via_cloudflare["headers"]["cf-connecting-ip"] = json!("198.51.100.4");
    assert_eq!(
        client_ip_of(via_cloudflare).await,
        (200, "198.51.100.4".to_string())
    );
    let mut forged = fixture("http.json", &[("sourceIp", "203.0.113.7".to_string())]);
    forged["headers"]["cf-connecting-ip"] = json!("198.51.100.4");
    assert_eq!(client_ip_of(forged).await, (200, "203.0.113.7".to_string()));
}

async fn status_with_sources(allowed: AllowedSources, source_ip: &str) -> u16 {
    let event = fixture("http.json", &[("sourceIp", source_ip.to_string())]);
    let request: lambda_http::request::LambdaRequest = serde_json::from_value(event).unwrap();
    let request =
        lambda_http::Request::from(request).map(|body| axum::body::Body::from(body.to_vec()));
    let router = axum::Router::new()
        .route("/ip", axum::routing::get(|| async { "ok" }))
        .layer(axum::middleware::from_fn_with_state(
            allowed,
            graphql::admit_sources,
        ));
    router.oneshot(request).await.unwrap().status().as_u16()
}

#[tokio::test]
async fn the_api_function_admits_only_cloudflare_when_its_sources_are_cloudflare() {
    let cloudflare = AllowedSources::Cloudflare;
    assert_eq!(status_with_sources(cloudflare, "172.70.1.2").await, 200);
    assert_eq!(status_with_sources(cloudflare, "203.0.113.7").await, 403);
    assert_eq!(status_with_sources(cloudflare, "not-an-ip").await, 403);
    assert_eq!(
        status_with_sources(AllowedSources::Any, "203.0.113.7").await,
        200
    );
}

#[test]
fn the_invocation_deadline_keeps_a_margin_for_the_commit() {
    let deadline = lambda::invocation_deadline(1_793_061_000_000).unwrap();
    assert_eq!(
        deadline,
        chrono::DateTime::<Utc>::from_timestamp(1_793_061_000 - lambda::DEADLINE_MARGIN_SECONDS, 0)
            .unwrap()
    );
    assert!(lambda::invocation_deadline(u64::MAX).is_err());
}
