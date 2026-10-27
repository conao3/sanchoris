mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use sanchoris_core::channel::{RunEventType, record_run_event};
use sanchoris_core::db::TenantTx;
use sanchoris_core::testing::{TestPostgres, WORKER_ROLE};
use serde_json::{Value, json};
use sqlx::Row;
use support::fake_github::FakeGitHub;
use support::{
    CONFIG_YAML, REPO, Seeded, bare_repo, check_run_payload, eventually, free_port,
    github_signature, labeled_payload, listen, rsa_key, run, seed,
};
use uuid::Uuid;

const WEBHOOK_SECRET: &str = "e2e-webhook-secret";
const STEP: Duration = Duration::from_secs(60);

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

struct Api(Child);

impl Drop for Api {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &self.0.id().to_string()])
            .status();
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.0.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Debug, Clone)]
struct TaskRow {
    id: Uuid,
    status: String,
    error: Option<String>,
    run_id: Option<Uuid>,
}

struct World {
    api: Option<Api>,
    url: String,
    http: reqwest::Client,
    github: Arc<FakeGitHub>,
    seeded: Seeded,
    records_dir: PathBuf,
    workspaces_dir: PathBuf,
    _pg: TestPostgres,
    _dir: tempfile::TempDir,
}

impl World {
    async fn start() -> Self {
        let root = repo_root();
        assert!(
            root.join("apps/workspace-agent/node_modules").exists(),
            "run `pnpm install` before the end-to-end test"
        );
        let dir = tempfile::tempdir().unwrap();
        let git_root = dir.path().join("git");
        bare_repo(&git_root, REPO);
        let github = FakeGitHub::new(
            &git_root,
            BTreeMap::from([("sanchoris.yaml".to_string(), CONFIG_YAML.to_string())]),
        );
        let github_url = listen(github.clone().router()).await;
        let pg = TestPostgres::start().await;
        let seeded = seed(&pg).await;
        let port = free_port();
        let url = format!("http://127.0.0.1:{port}");
        let records_dir = dir.path().join("records");
        let workspaces_dir = dir.path().join("workspaces");
        let agent = root.join("scripts/e2e/fake-agent.ts");
        let child = Command::new(env!("CARGO_BIN_EXE_sanchoris"))
            .arg("all")
            .env("DATABASE_URL", pg.url(WORKER_ROLE))
            .env("SANCHORIS_BIND_ADDR", format!("127.0.0.1:{port}"))
            .env("SANCHORIS_PUBLIC_URL", &url)
            .env("COGNITO_USER_POOL_ID", "us-east-1_e2e")
            .env("COGNITO_CLIENT_ID", "e2e-client")
            .env("GITHUB_WEBHOOK_SECRET", WEBHOOK_SECRET)
            .env("GITHUB_APP_ID", "1")
            .env("GITHUB_APP_PRIVATE_KEY", rsa_key())
            .env("GITHUB_APP_CLIENT_ID", "Iv1.e2e")
            .env("GITHUB_APP_CLIENT_SECRET", "e2e-client-secret")
            .env("GITHUB_APP_SLUG", "sanchoris-e2e")
            .env("GITHUB_URL", &github_url)
            .env("GITHUB_API_URL", &github_url)
            .env("STRIPE_WEBHOOK_SECRET", "whsec_e2e")
            .env("STRIPE_SECRET_KEY", "sk_test_e2e")
            .env("STRIPE_API_URL", "https://api.stripe.invalid")
            .env(
                "SANCHORIS_CREDENTIAL_KEY",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            )
            .env(
                "RUN_TOKEN_SIGNING_KEY",
                "e2e-run-token-signing-key-0123456789",
            )
            .env(
                "TASK_TOKEN_SIGNING_KEY",
                "e2e-task-token-signing-key-0123456789",
            )
            .env(
                "OAUTH_STATE_SIGNING_KEY",
                "e2e-oauth-state-signing-key-0123456789",
            )
            .env("SANCHORIS_RECORDS_STORE", "local")
            .env("SANCHORIS_RECORDS_DIR", &records_dir)
            .env(
                "RECORDS_SIGNING_KEY",
                "e2e-records-signing-key-0123456789ab",
            )
            .env(
                "SANCHORIS_WORKSPACE_COMMAND",
                format!("node --no-warnings {}", agent.display()),
            )
            .env(
                "SANCHORIS_CONTROL_URL",
                format!("ws://127.0.0.1:{port}/runs/channel"),
            )
            .env("SANCHORIS_SOCKET_DIR", dir.path().join("sockets"))
            .env("SANCHORIS_WORKSPACES_DIR", &workspaces_dir)
            .env(
                "SANCHORIS_GIT_URL",
                format!("file://{}", git_root.display()),
            )
            .stdin(Stdio::null())
            .spawn()
            .expect("start the sanchoris binary");
        let world = Self {
            api: Some(Api(child)),
            url,
            http: reqwest::Client::new(),
            github,
            seeded,
            records_dir,
            workspaces_dir,
            _pg: pg,
            _dir: dir,
        };
        eventually("the api to listen", STEP, || async {
            world
                .http
                .get(format!("{}/health", world.url))
                .send()
                .await
                .ok()
                .filter(|r| r.status().is_success())
        })
        .await;
        eventually("the config to sync", STEP, || async {
            let synced: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM sanchoris.config_revisions WHERE commit_sha = $1 AND validation_status = 'valid'",
            )
            .bind(support::fake_github::CONFIG_SHA)
            .fetch_one(&world.seeded.pool)
            .await
            .unwrap();
            (synced == 1).then_some(())
        })
        .await;
        world
    }

    async fn webhook(&self, event: &str, delivery: &str, payload: &Value) -> (StatusCode, Value) {
        let body = serde_json::to_vec(payload).unwrap();
        let response = self
            .http
            .post(format!("{}/webhooks/github", self.url))
            .header("X-GitHub-Event", event)
            .header("X-GitHub-Delivery", delivery)
            .header(
                "X-Hub-Signature-256",
                github_signature(WEBHOOK_SECRET, &body),
            )
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
        (response.status(), response.json().await.unwrap())
    }

    async fn task(&self, number: i64) -> Option<TaskRow> {
        sqlx::query(
            "SELECT t.id, t.status, t.error, \
                    (SELECT r.id FROM sanchoris.runs r WHERE r.task_id = t.id ORDER BY r.started_at DESC LIMIT 1) AS run_id \
             FROM sanchoris.tasks t WHERE t.name = $1",
        )
        .bind(format!("{REPO}#{number}"))
        .fetch_optional(&self.seeded.pool)
        .await
        .unwrap()
        .map(|row| TaskRow {
            id: row.get("id"),
            status: row.get("status"),
            error: row.get("error"),
            run_id: row.get("run_id"),
        })
    }

    async fn wait_status(&self, number: i64, status: &str) -> TaskRow {
        eventually(&format!("task #{number} to be {status}"), STEP, || async {
            self.task(number).await.filter(|task| task.status == status)
        })
        .await
    }

    async fn waiting_head(&self, task_id: Uuid, seen: &[String]) -> String {
        eventually("the checks wait on a new head", STEP, || async {
            let heads: Vec<Value> = sqlx::query_scalar(
                "SELECT cw.head_shas FROM sanchoris.check_waits cw \
                 JOIN sanchoris.skill_runs s ON s.id = cw.skill_run_id \
                 WHERE s.task_id = $1 AND cw.status = 'waiting'",
            )
            .bind(task_id)
            .fetch_all(&self.seeded.pool)
            .await
            .unwrap();
            heads
                .iter()
                .filter_map(|heads| heads[REPO].as_str())
                .find(|sha| !seen.iter().any(|s| s == sha))
                .map(str::to_string)
        })
        .await
    }

    async fn conclude(&self, sha: &str, conclusion: &str) {
        self.github.set_check(sha, conclusion);
        let pull = self.github.pulls()[0].number;
        let (status, _) = self
            .webhook(
                "check_run",
                &format!("check-{sha}-{conclusion}"),
                &check_run_payload(sha, conclusion, pull),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED);
    }

    async fn skill_runs(&self, task_id: Uuid) -> Vec<(i32, String, Option<String>)> {
        sqlx::query_as(
            "SELECT number, status, error FROM sanchoris.skill_runs WHERE task_id = $1 ORDER BY number",
        )
        .bind(task_id)
        .fetch_all(&self.seeded.pool)
        .await
        .unwrap()
    }

    async fn label(&self, number: i64) -> TaskRow {
        let (status, body) = self
            .webhook(
                "issues",
                &format!("delivery-{number}"),
                &labeled_payload(number, "Fix flaky checkout test"),
            )
            .await;
        assert_eq!(
            (status, body["status"].as_str()),
            (StatusCode::ACCEPTED, Some("accepted"))
        );
        self.wait_status(number, "RUNNING").await
    }

    async fn stop(&mut self) {
        let api = self.api.take();
        tokio::task::spawn_blocking(move || drop(api))
            .await
            .unwrap();
    }

    fn workspace_count(&self) -> usize {
        std::fs::read_dir(&self.workspaces_dir)
            .map(|entries| entries.count())
            .unwrap_or(0)
    }
}

fn branch(task: &TaskRow) -> String {
    format!("sanchoris/{}", task.id)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_labeled_issue_opens_a_pull_request_that_goes_green_after_a_fix() {
    let mut world = World::start().await;

    let unsigned = world
        .http
        .post(format!("{}/webhooks/github", world.url))
        .header("X-GitHub-Event", "issues")
        .header("X-GitHub-Delivery", "forged")
        .header("X-Hub-Signature-256", "sha256=00")
        .body(serde_json::to_vec(&labeled_payload(35, "Forged")).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(unsigned.status(), StatusCode::UNAUTHORIZED);

    let task = world.label(35).await;
    let (status, body) = world
        .webhook(
            "issues",
            "delivery-35",
            &labeled_payload(35, "Fix flaky checkout test"),
        )
        .await;
    assert_eq!(
        (status, body["status"].as_str()),
        (StatusCode::OK, Some("duplicate"))
    );

    let first = world.waiting_head(task.id, &[]).await;
    assert_eq!(
        world.github.branch_head(REPO, &branch(&task)).as_deref(),
        Some(first.as_str())
    );
    let pulls = world.github.pulls();
    assert_eq!(pulls.len(), 1);
    assert_eq!(
        (pulls[0].head.as_str(), pulls[0].base.as_str()),
        (branch(&task).as_str(), "main")
    );
    world.conclude(&first, "failure").await;

    let second = world
        .waiting_head(task.id, std::slice::from_ref(&first))
        .await;
    world.conclude(&second, "success").await;

    let done = world.wait_status(35, "SUCCEEDED").await;
    assert_eq!(done.error, None);
    assert_eq!(
        world.skill_runs(task.id).await,
        vec![(1, "SUCCEEDED".to_string(), None)]
    );
    assert_eq!(world.github.pulls().len(), 1);
    let comments = world.github.comments(REPO, 35);
    assert_eq!(comments.len(), 1);
    assert!(
        comments[0]
            .body
            .starts_with("Pull requests are open and the checks pass: "),
        "{}",
        comments[0].body
    );
    let log = run(
        &[
            "git",
            "--git-dir",
            &world.github.bare(REPO).display().to_string(),
            "log",
            "--format=%s",
            &format!("main..{}", branch(&task)),
        ],
        Path::new("/"),
    );
    assert_eq!(log, "Fix CI for issue 35\nFix issue 35");

    let run_id = done.run_id.unwrap();
    let pool = &world.seeded.pool;
    let record_key: Option<String> =
        sqlx::query_scalar("SELECT record_s3_key FROM sanchoris.runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(pool)
            .await
            .unwrap();
    let record_key = record_key.expect("the run saved a session record");
    assert!(world.records_dir.join(&record_key).exists());
    let logs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sanchoris.run_log_entries WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(logs > 0);
    let execs: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM sanchoris.run_events WHERE run_id = $1 AND type = 'exec_result' ORDER BY seq",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(execs.len(), 2);
    assert!(execs.iter().all(|e| e["ok"] == json!(true)));
    let unprocessed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sanchoris.run_events WHERE run_id = $1 AND error IS NOT NULL",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(unprocessed, 0);
    eventually("the workspace to stop", STEP, || async {
        let open: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sanchoris.workspaces WHERE run_id = $1 AND stopped_at IS NULL",
        )
        .bind(run_id)
        .fetch_one(pool)
        .await
        .unwrap();
        (open == 0 && world.workspace_count() == 0).then_some(())
    })
    .await;
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn checks_that_keep_failing_need_you_and_a_reply_resumes_the_task() {
    let mut world = World::start().await;
    let task = world.label(36).await;

    let mut seen = Vec::new();
    for _ in 0..4 {
        let sha = world.waiting_head(task.id, &seen).await;
        world.conclude(&sha, "failure").await;
        seen.push(sha);
    }
    let stuck = world.wait_status(36, "NEEDS_YOU").await;
    assert_eq!(stuck.error.as_deref(), Some("Skill.FixesExhausted"));
    assert_eq!(
        world.skill_runs(task.id).await,
        vec![(
            1,
            "FAILED".to_string(),
            Some("Skill.FixesExhausted".to_string())
        )]
    );

    let run_id = stuck.run_id.unwrap();
    let mut tx = TenantTx::begin(&world.seeded.pool, world.seeded.tenant.tenant_id)
        .await
        .unwrap();
    record_run_event(
        &mut tx,
        run_id,
        None,
        RunEventType::HumanReply,
        &json!({ "member": "Mika", "text": "The flaky spec needs a retry. Run the skill again." }),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    world.wait_status(36, "RUNNING").await;

    let retry = world.waiting_head(task.id, &seen).await;
    world.conclude(&retry, "success").await;
    let done = world.wait_status(36, "SUCCEEDED").await;
    assert_eq!(done.error, None);
    assert_eq!(
        world.skill_runs(task.id).await,
        vec![
            (
                1,
                "FAILED".to_string(),
                Some("Skill.FixesExhausted".to_string())
            ),
            (2, "SUCCEEDED".to_string(), None)
        ]
    );
    assert_eq!(world.github.pulls().len(), 1);
    assert_eq!(world.github.comments(REPO, 36).len(), 1);
    world.stop().await;
}
