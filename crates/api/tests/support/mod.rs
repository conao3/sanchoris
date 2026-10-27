#![allow(dead_code)]

pub mod fake_github;
pub mod fake_stripe;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::Router;
use chrono::Utc;
use reqwest::redirect::Policy;
use sanchoris_api::app::AppState;
use sanchoris_api::auth::{AuthError, Identity, IdentityVerifier};
use sanchoris_api::config::{
    ApiConfig, CognitoConfig, CredentialKey, GitHubAppConfig, GitHubConfig, RecordsConfig,
    StripeConfig,
};
use sanchoris_api::frontend::Frontend;
use sanchoris_api::graphql::{self, GraphqlPorts};
use sanchoris_api::records::{LocalRecordStore, RecordStore};
use sanchoris_api::server::{LocalParts, LocalServer, drive, local_channel};
use sanchoris_api::signing::HmacKey;
use sanchoris_core::clock::{Clock, SystemClock};
use sanchoris_core::db::{REF_RUN, Store, insert_ref};
use sanchoris_core::item::Entity;
use sanchoris_core::items::{Credential, Guard, Integration, Run, RunChannel, Task, User, uniq};
use sanchoris_core::keys;
use sanchoris_core::model::TaskStatus;
use sanchoris_core::store::Relay;
use sanchoris_core::testing::{SeededTenant, TestDynamo};
use sanchoris_core::tokens::{RunTokenSigner, Signer, TaskTokenSigner};
use sanchoris_graphql::AesGcmEncryptor;
use sanchoris_worker::driver::LocalDriver;
use sanchoris_worker::ports::{RunnerError, RunnerPort, WorkspaceHandle, WorkspaceSpec};
use sanchoris_worker::resources::GitHubResources;
use sanchoris_worker::runner::{LocalProcessRunner, LocalRunnerConfig};
use sanchoris_worker::{Deps, Settings, Worker};
use serde_json::{Value, json};
use tokio::sync::watch;
use uuid::Uuid;

use fake_github::FakeGitHub;
use fake_stripe::FakeStripe;

pub const RUN_KEY: &[u8] = b"http-test-run-token-signing-key-0123";
pub const TASK_KEY: [u8; 32] = [6u8; 32];
pub const STRIPE_SECRET: &str = "whsec_http";
pub const WEBHOOK_SECRET: &str = "http-webhook";
pub const INSTALLATION: i64 = 42;
pub const REPO: &str = "acme/shop-web";

pub struct StubVerifier;

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

pub struct NoWorkspaces;

#[async_trait]
impl RunnerPort for NoWorkspaces {
    async fn start_workspace(&self, _spec: &WorkspaceSpec) -> Result<WorkspaceHandle, RunnerError> {
        Err(RunnerError("the HTTP tests start no workspaces".into()))
    }

    async fn stop_workspace(&self, _run_id: Uuid, _platform_ref: &str) -> Result<(), RunnerError> {
        Ok(())
    }
}

pub fn rsa_key() -> String {
    let output = Command::new("openssl")
        .args([
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:2048",
        ])
        .output()
        .expect("openssl is on PATH (run inside `nix develop`)");
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

pub enum Workspaces {
    None,
    Agent {
        command: Vec<String>,
        socket_dir: PathBuf,
        workspaces_dir: PathBuf,
        git_url: String,
    },
}

pub enum Driving {
    QueueOnly,
    Full,
}

pub struct Api {
    pub url: String,
    pub http: reqwest::Client,
    pub store: Store,
    pub records: Arc<LocalRecordStore>,
    pub driver: Arc<LocalDriver>,
    pub stripe: Arc<FakeStripe>,
    shutdown: watch::Sender<bool>,
}

impl Drop for Api {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

pub async fn listen(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{address}")
}

async fn drive_queue(driver: Arc<LocalDriver>, mut shutdown: watch::Receiver<bool>) {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            _ = shutdown.changed() => return,
        }
        if let Err(error) = driver.fire_due().await {
            eprintln!("test driver: {error}");
        }
        if let Err(error) = driver.drain().await {
            eprintln!("test driver: {error}");
        }
    }
}

pub async fn start_api(
    dynamo: &TestDynamo,
    github: Arc<FakeGitHub>,
    dir: &Path,
    workspaces: Workspaces,
    driving: Driving,
) -> Api {
    let github_url = listen(github.router()).await;
    let stripe = FakeStripe::new();
    let stripe_url = listen(stripe.clone().router()).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = format!("http://{address}");
    let records_dir = dir.join("records");
    let records_key = HmacKey::strong(&[5u8; 32]).unwrap();
    let private_key = rsa_key();
    let config = ApiConfig {
        public_url: url.clone(),
        cognito: CognitoConfig {
            user_pool_id: "us-east-1_http".into(),
            client_id: "http-client".into(),
        },
        github: GitHubConfig {
            webhook_secret: HmacKey::new(WEBHOOK_SECRET.as_bytes()).unwrap(),
            client_id: "Iv1.http".into(),
            client_secret: "http-secret".into(),
            app_slug: "sanchoris-test".into(),
            web_url: github_url.clone(),
            api_url: github_url.clone(),
        },
        github_app: GitHubAppConfig {
            app_id: "1".into(),
            private_key: private_key.clone(),
            api_url: github_url.clone(),
        },
        stripe: StripeConfig {
            webhook_secret: HmacKey::new(STRIPE_SECRET.as_bytes()).unwrap(),
            secret_key: fake_stripe::SECRET_KEY.into(),
            api_url: stripe_url,
        },
        task_tokens: TaskTokenSigner::new(Signer::new(&TASK_KEY).unwrap()),
        oauth_state_key: HmacKey::strong(&[7u8; 32]).unwrap(),
        credential_key: CredentialKey(Arc::new(AesGcmEncryptor::new(&CREDENTIAL_KEY))),
        records: RecordsConfig::Local {
            dir: records_dir.clone(),
            signing_key: records_key.clone(),
        },
    };
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let direct = dynamo.direct_store(clock.clone());
    let records = Arc::new(LocalRecordStore::new(&records_dir, records_key, &url));
    let run_tokens = RunTokenSigner::new(Signer::new(RUN_KEY).unwrap());
    let channel = local_channel(
        direct.store.clone(),
        run_tokens.clone(),
        records.clone() as Arc<dyn RecordStore>,
    );
    let http = reqwest::Client::builder()
        .redirect(Policy::none())
        .build()
        .unwrap();
    let github_app = Arc::new(config.github_app.client().unwrap());
    let runner: Arc<dyn RunnerPort> = match workspaces {
        Workspaces::None => Arc::new(NoWorkspaces),
        Workspaces::Agent {
            command,
            socket_dir,
            workspaces_dir,
            git_url,
        } => {
            let (program, args) = command.split_first().unwrap();
            Arc::new(LocalProcessRunner::new(
                LocalRunnerConfig {
                    program: PathBuf::from(program),
                    args: args.to_vec(),
                    control_url: format!("ws://{address}/runs/channel"),
                    socket_dir,
                    workspaces_dir,
                    git_url,
                    run_tokens: run_tokens.clone(),
                    token_ttl: chrono::Duration::hours(1),
                },
                clock.clone(),
            ))
        }
    };
    let deps = Deps {
        clock,
        github: github_app.clone(),
        reader: github_app.clone(),
        config_source: github_app.clone(),
        resources: Arc::new(GitHubResources::new(github_app.clone())),
        runner,
        pusher: Arc::new(channel.channel.clone()),
    };
    let worker = Arc::new(Worker::new(direct.store.clone(), deps, Settings::default()));
    let config = Arc::new(config);
    let schema = sanchoris_graphql::build_schema(
        direct.store.clone(),
        graphql::deps(GraphqlPorts {
            store: direct.store.clone(),
            config: config.clone(),
            http: http.clone(),
            worker: worker.clone(),
            github_app: github_app.clone(),
            records: records.clone(),
        }),
    );
    let frontend_dir = dir.join("frontend");
    std::fs::create_dir_all(frontend_dir.join("assets")).unwrap();
    std::fs::write(frontend_dir.join("index.html"), FRONTEND_INDEX).unwrap();
    std::fs::write(frontend_dir.join("assets/app.js"), FRONTEND_SCRIPT).unwrap();
    let server = LocalServer::new(LocalParts {
        state: AppState {
            store: direct.store.clone(),
            config,
            verifier: Arc::new(StubVerifier),
            http: http.clone(),
            github_app,
            schema,
        },
        frontend: Frontend::dir(frontend_dir).unwrap(),
        channel,
        local_records: Some(records.clone()),
        queue: direct.queue,
        scheduler: direct.scheduler,
        worker,
    });
    let (shutdown, receiver) = watch::channel(false);
    let driver = server.driver.clone();
    match driving {
        Driving::QueueOnly => tokio::spawn(drive_queue(driver.clone(), receiver)),
        Driving::Full => tokio::spawn(drive(driver.clone(), receiver)),
    };
    let router = server.router;
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    Api {
        url,
        http,
        store: direct.store,
        records,
        driver,
        stripe,
        shutdown,
    }
}

pub const FRONTEND_INDEX: &str = "<!doctype html><title>Sanchoris</title><script type=\"module\" src=\"/assets/app.js\"></script>";
pub const FRONTEND_SCRIPT: &str = "console.log('sanchoris');";
pub const CREDENTIAL_KEY: [u8; 32] = [3u8; 32];

pub const CONFIG_YAML: &str = r#"Version: 1
Projects:
  shop:
    SystemPrompt: |
      You are working on the Acme shop.
    Repos:
      frontend:
        Repo: github.com/acme/shop-web
Sessions:
  shop:
    Worker: { Agent: claude, Model: claude-sonnet-5-5, Credential: acme-claude-team }
    Project: shop
    GitHub:
      Repos: [acme/shop-web]
      Permissions: { contents: write, pull_requests: write, issues: write, checks: read }
    Network: { Allow: [github.com] }
    UsageLimit: { Resume: auto }
    Workspace: { Compute: standard, IdleSeconds: 900 }
Triggers:
  labeled-issue:
    State: ENABLED
    EventPattern:
      source: [github]
      detail-type: [Issue Labeled]
      detail:
        repository: [frontend]
        label: [sanchoris]
        sender: { login: [mika-dev] }
    InputTransformer:
      InputPathsMap: { repository: $.detail.repository, number: $.detail.issue.number, title: $.detail.issue.title }
      InputTemplate: '{ "repository": <repository>, "number": <number>, "title": <title> }'
    Start:
      Session: shop
      Instruction: 'Resolve issue <number> in <repository>: <title>. Write tests for the change.'
      Skills: [open-pr-green-ci]
      RequiredSkills: [open-pr-green-ci]
      MaxNudges: 2
      Limits: { Turns: 60, CostUsd: 20, TimeoutSeconds: 14400, HeartbeatSeconds: 900, Children: { MaxDepth: 1, MaxCount: 5 } }
"#;

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub fn run(args: &[&str], dir: &Path) -> String {
    let output = Command::new(args[0])
        .args(&args[1..])
        .current_dir(dir)
        .output()
        .unwrap_or_else(|error| panic!("{args:?}: {error}"));
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn bare_repo(git_root: &Path, repo: &str) {
    let bare = git_root.join(format!("{repo}.git"));
    std::fs::create_dir_all(&bare).unwrap();
    run(&["git", "init", "--quiet", "--bare", "-b", "main"], &bare);
    let seed = tempfile::tempdir().unwrap();
    run(&["git", "init", "--quiet", "-b", "main"], seed.path());
    std::fs::write(seed.path().join("README.md"), "# shop-web\n").unwrap();
    run(&["git", "add", "."], seed.path());
    run(
        &[
            "git",
            "-c",
            "user.name=seed",
            "-c",
            "user.email=seed@sanchoris.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "Initial commit",
        ],
        seed.path(),
    );
    let url = bare.display().to_string();
    run(&["git", "push", "--quiet", &url, "main"], seed.path());
}

pub struct Seeded {
    pub tenant: SeededTenant,
    pub subject: String,
}

impl Seeded {
    pub fn member_token(&self) -> String {
        format!("user:{}", self.subject)
    }
}

pub async fn seed(dynamo: &TestDynamo) -> Seeded {
    let tenant = dynamo.seed_tenant("pro").await;
    let subject = format!("subject-{}", tenant.user_id);
    let now = Utc::now();
    dynamo
        .modify::<User>(&keys::user_key(tenant.user_id), |user| {
            user.github_user_id = Some(1001);
            user.github_login = Some("mika-dev".into());
            user.display_name = "Mika".into();
        })
        .await;
    for (kind, value) in [
        (uniq::COGNITO_SUBJECT, subject.clone()),
        (uniq::EMAIL, format!("{}@example.com", tenant.user_id)),
        (uniq::GITHUB_USER, "1001".to_string()),
        (uniq::GITHUB_LOGIN, "mika-dev".to_string()),
    ] {
        dynamo
            .put(&Guard::new(keys::uniq_key(kind, &value), tenant.user_id))
            .await;
    }
    dynamo
        .put(&Guard::of_tenant(
            keys::uniq_key(uniq::SLUG, &format!("tenant-{}", tenant.user_id.simple())),
            tenant.tenant_id,
            tenant.tenant_id,
        ))
        .await;
    let integration_id = Uuid::new_v4();
    dynamo
        .put(&Integration {
            id: integration_id,
            tenant_id: tenant.tenant_id,
            kind: "github_installation".into(),
            status: "connected".into(),
            github_installation_id: Some(INSTALLATION),
            slack_team_id: None,
            details: json!({ "account_login": "acme" }),
            created_at: now,
            updated_at: now,
        })
        .await;
    dynamo
        .put(&Guard::of_tenant(
            keys::uniq_key(uniq::INSTALLATION, &INSTALLATION.to_string()),
            integration_id,
            tenant.tenant_id,
        ))
        .await;
    dynamo
        .put(&Credential {
            id: Uuid::new_v4(),
            tenant_id: tenant.tenant_id,
            scope: "org".into(),
            member_id: None,
            name: "acme-claude-team".into(),
            provider: "claude".into(),
            kind: "subscription".into(),
            account: "team@acme.test".into(),
            status: "connected".into(),
            subscription_plan: None,
            rejected_at: None,
            encrypted_value: "AA==".into(),
            kms_key_id: "test-key".into(),
            last_used_at: None,
            created_at: now,
            updated_at: now,
        })
        .await;
    Seeded { tenant, subject }
}

pub async fn task_with_run(dynamo: &TestDynamo, seeded: &Seeded) -> (Uuid, Uuid) {
    let tenant_id = seeded.tenant.tenant_id;
    let now = Utc::now();
    let task_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let store = dynamo.store(Relay::Stream);
    let mut tx = store.tx(tenant_id);
    tx.insert(&Task {
        id: task_id,
        tenant_id,
        trigger_id: seeded.tenant.trigger_id,
        trigger_event_id: None,
        config_revision_id: seeded.tenant.config_revision_id,
        name: "acme/shop-web#1".into(),
        title: "Task".into(),
        status: TaskStatus::Running,
        wait_reason: None,
        resume_at: None,
        snoozed_until: None,
        nudges: 0,
        definition: json!({
            "Trigger": { "Name": "labeled-issue", "Definition": {} },
            "Session": { "Name": "shop", "Definition": {} },
            "Skills": [],
        }),
        parent_task_id: None,
        channel: None,
        start_payload: None,
        requested_by_member_id: None,
        error: None,
        cause: None,
        runs: 1,
        skill_runs: 0,
        created_at: now,
        started_at: Some(now),
        finished_at: None,
        updated_at: now,
    })
    .unwrap();
    let run = Run {
        id: run_id,
        tenant_id,
        task_id,
        number: 1,
        session_name: "shop".into(),
        worker: "claude".into(),
        model: "claude-sonnet-5-5".into(),
        agent_session_id: None,
        status: "creating".into(),
        turns: 0,
        cost_usd: 0.0,
        summary: None,
        error: None,
        resume_branch: None,
        record_s3_key: None,
        log_s3_key: None,
        last_heartbeat_at: None,
        started_at: now,
        finished_at: None,
        updated_at: now,
    };
    tx.insert(&run).unwrap();
    insert_ref(&mut tx, run_id, REF_RUN, run.key(), Some(task_id)).unwrap();
    tx.insert(&RunChannel {
        tenant_id,
        run_id,
        task_id,
        next_outbox_seq: 1,
        instances: 0,
        connection_id: None,
        connection_instance: None,
        connection_started: false,
    })
    .unwrap();
    tx.commit().await.unwrap();
    (task_id, run_id)
}

pub fn github_signature(secret: &str, body: &[u8]) -> String {
    format!(
        "sha256={}",
        HmacKey::new(secret.as_bytes()).unwrap().hex(body)
    )
}

pub fn labeled_payload(number: i64, title: &str) -> Value {
    json!({
        "action": "labeled",
        "repository": { "full_name": REPO },
        "installation": { "id": INSTALLATION },
        "issue": {
            "number": number,
            "title": title,
            "body": "The checkout spec fails at random.",
            "state": "open",
            "html_url": format!("https://github.com/{REPO}/issues/{number}"),
            "author_association": "MEMBER",
            "user": { "login": "mika-dev" },
            "labels": [{ "name": "sanchoris" }],
        },
        "label": { "name": "sanchoris" },
        "sender": { "login": "mika-dev", "id": 1001, "type": "User" },
    })
}

pub fn check_run_payload(sha: &str, conclusion: &str, pull: i64) -> Value {
    json!({
        "action": "completed",
        "repository": { "full_name": REPO },
        "installation": { "id": INSTALLATION },
        "check_run": {
            "id": 7, "name": "ci/test", "status": "completed", "conclusion": conclusion,
            "head_sha": sha, "html_url": format!("https://ci.example/{sha}"), "app": { "slug": "actions" },
            "pull_requests": [{ "number": pull }],
        },
        "sender": { "login": "github-actions[bot]", "id": 41898282, "type": "Bot" },
    })
}

pub async fn eventually<T, F, Fut>(what: &str, timeout: Duration, mut probe: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = probe().await {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
