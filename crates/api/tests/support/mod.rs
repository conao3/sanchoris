#![allow(dead_code)]

pub mod fake_github;
pub mod fake_stripe;

use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::Router;
use reqwest::redirect::Policy;
use sanchoris_api::app::{AppState, Services, router};
use sanchoris_api::auth::{AuthError, Identity, IdentityVerifier};
use sanchoris_api::config::{
    CognitoConfig, GitHubConfig, RecordsConfig, ServeConfig, StripeConfig,
};
use sanchoris_api::records::{LocalRecordStore, RecordStore};
use sanchoris_api::signing::HmacKey;
use sanchoris_core::testing::{SeededTenant, TestPostgres, WORKER_ROLE};
use sanchoris_core::tokens::{RunTokenSigner, Signer, TaskTokenSigner};
use sanchoris_graphql::AesGcmEncryptor;
use sanchoris_worker::Deps;
use sanchoris_worker::clock::SystemClock;
use sanchoris_worker::github::transport::ReqwestTransport;
use sanchoris_worker::github::{AppAuth, GitHubApi};
use sanchoris_worker::ports::{RunnerError, RunnerPort, WorkspaceHandle, WorkspaceSpec};
use sanchoris_worker::resources::GitHubResources;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use fake_github::FakeGitHub;
use fake_stripe::FakeStripe;

pub const RUN_KEY: &[u8] = b"http-test-run-token-signing-key-0123";
pub const STRIPE_SECRET: &str = "whsec_http";
pub const WEBHOOK_SECRET: &str = "http-webhook";

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

pub struct Api {
    pub url: String,
    pub http: reqwest::Client,
    pub stripe: Arc<FakeStripe>,
    pub records: Arc<LocalRecordStore>,
}

pub async fn start_api(
    pg: &TestPostgres,
    seeded: &Seeded,
    github: Arc<FakeGitHub>,
    dir: &Path,
) -> Api {
    let github_url = listen(github.router()).await;
    let stripe = FakeStripe::new();
    let stripe_url = listen(stripe.clone().router()).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    let url = format!("http://{address}");
    let records_key = HmacKey::strong(&[5u8; 32]).unwrap();
    let records_dir = dir.join("records");
    let database_url = pg.url(WORKER_ROLE);
    let config = ServeConfig {
        bind_addr: address,
        database_url: database_url.clone(),
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
        stripe: StripeConfig {
            secret_key: fake_stripe::SECRET_KEY.into(),
            api_url: stripe_url,
            webhook_secret: HmacKey::new(STRIPE_SECRET.as_bytes()).unwrap(),
        },
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
    let github_app = Arc::new(GitHubApi::new(
        &github_url,
        Arc::new(ReqwestTransport::new()),
        AppAuth::new("1", &rsa_key()).unwrap(),
    ));
    let state = AppState::new(Services {
        pool: seeded.pool.clone(),
        app_pool: sanchoris_api::app_pool(&database_url).await.unwrap(),
        config: Arc::new(config),
        verifier: Arc::new(StubVerifier),
        records: records.clone() as Arc<dyn RecordStore>,
        http: http.clone(),
        worker_deps: Deps {
            clock: Arc::new(SystemClock),
            github: github_app.clone(),
            reader: github_app.clone(),
            config_source: github_app.clone(),
            resources: Arc::new(GitHubResources::new(github_app.clone())),
            runner: Arc::new(NoWorkspaces),
        },
        github_app,
        encryptor: Arc::new(AesGcmEncryptor::new(&[3u8; 32])),
    });
    let app = router(state, Some(records.clone()));
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    Api {
        url,
        http,
        stripe,
        records,
    }
}

pub const INSTALLATION: i64 = 42;
pub const REPO: &str = "acme/shop-web";

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

pub async fn listen(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{address}")
}

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
    pub admin: PgPool,
    pub pool: PgPool,
    pub tenant: SeededTenant,
}

pub async fn seed(pg: &TestPostgres) -> Seeded {
    let admin = pg.admin_pool().await;
    let pool = pg.worker_pool().await;
    let tenant = pg.seed_tenant(&admin, "pro").await;
    sqlx::query(
        "UPDATE sanchoris.users SET github_user_id = 1001, github_login = 'mika-dev', display_name = 'Mika' WHERE id = $1",
    )
    .bind(tenant.user_id)
    .execute(&admin)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sanchoris.integrations (tenant_id, kind, status, github_installation_id, details) \
         VALUES ($1, 'github_installation', 'connected', $2, '{\"account_login\": \"acme\"}')",
    )
    .bind(tenant.tenant_id)
    .bind(INSTALLATION)
    .execute(&admin)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO sanchoris.credentials (tenant_id, scope, name, provider, kind, account, status, encrypted_value, kms_key_id) \
         VALUES ($1, 'org', 'acme-claude-team', 'claude', 'subscription', 'team@acme.test', 'connected', '\\x00', 'test-key')",
    )
    .bind(tenant.tenant_id)
    .execute(&admin)
    .await
    .unwrap();
    Seeded {
        admin,
        pool,
        tenant,
    }
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
