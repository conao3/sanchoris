#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use sanchoris_config::ConfigFiles;
use sanchoris_core::channel::{RunEventType, ingest_github_delivery, record_run_event};
use sanchoris_core::db::{Recorded, TenantTx};
use sanchoris_core::skillrun::{AcceptedSkillRun, accept_skill_request, continue_skill_run};
use sanchoris_core::testing::{SeededTenant, TestPostgres};
use sanchoris_worker::clock::FakeClock;
use sanchoris_worker::ports::{
    CheckRun, ConfigSource, GitHubClient, GitHubError, GitHubReader, IssueComment, IssueRelations,
    MergeResult, NewPullRequest, PullRequest, RunnerError, RunnerPort, WorkspaceHandle,
    WorkspaceSpec,
};
use sanchoris_worker::resources::GitHubResources;
use sanchoris_worker::{Deps, Loop, Settings, Worker};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

pub const CONFIG_YAML: &str = include_str!("../../../config/tests/fixtures/example.sanchoris.yaml");
pub const INSTALLATION: i64 = 42;
pub const REPO: &str = "acme/shop-web";

pub fn epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap()
}

#[derive(Debug, Default)]
pub struct GitHubState {
    pub head_sha: String,
    pub files: ConfigFiles,
    pub default_branch_calls: usize,
    pub pulls: BTreeMap<(String, String), PullRequest>,
    pub next_number: i64,
    pub pushes: HashMap<(String, String), u32>,
    pub comments: BTreeMap<(String, i64), Vec<IssueComment>>,
    pub next_comment: i64,
    pub check_script: VecDeque<&'static str>,
    pub check_results: HashMap<(String, String), Vec<CheckRun>>,
    pub relations: HashMap<(String, i64), IssueRelations>,
    pub created_pulls: Vec<(String, String)>,
    pub created_comments: Vec<(String, i64)>,
    pub updated_comments: Vec<i64>,
    pub merges: Vec<(String, i64)>,
}

#[derive(Debug, Default)]
pub struct FakeGitHub {
    pub state: Mutex<GitHubState>,
}

impl FakeGitHub {
    pub fn new() -> Arc<Self> {
        let fake = Arc::new(Self::default());
        {
            let mut state = fake.state.lock().unwrap();
            state.head_sha = "config-1".into();
            state
                .files
                .insert("sanchoris.yaml".into(), CONFIG_YAML.into());
            state.next_number = 52;
            state.next_comment = 9000;
        }
        fake
    }

    pub fn push(&self, repo: &str, branch: &str) {
        let mut state = self.state.lock().unwrap();
        let key = (repo.to_string(), branch.to_string());
        let count = {
            let entry = state.pushes.entry(key.clone()).or_default();
            *entry += 1;
            *entry
        };
        if let Some(pr) = state.pulls.get_mut(&key) {
            pr.head_sha = format!("{branch}-sha-{count}");
        }
    }

    pub fn script_checks(&self, results: &[&'static str]) {
        self.state.lock().unwrap().check_script = results.iter().copied().collect();
    }

    pub fn set_parent(&self, number: i64, parent: Option<i64>) {
        self.state.lock().unwrap().relations.insert(
            (REPO.to_string(), number),
            IssueRelations {
                parent: parent.map(|n| sanchoris_worker::ports::IssueRef {
                    repository: REPO.into(),
                    number: n,
                }),
                blocked_by: Vec::new(),
            },
        );
    }
}

#[async_trait]
impl GitHubClient for FakeGitHub {
    async fn default_branch(&self, _: i64, _: &str) -> Result<String, GitHubError> {
        self.state.lock().unwrap().default_branch_calls += 1;
        Ok("main".into())
    }

    async fn find_open_pull_request(
        &self,
        _: i64,
        repo: &str,
        head_branch: &str,
    ) -> Result<Option<PullRequest>, GitHubError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .pulls
            .get(&(repo.to_string(), head_branch.to_string()))
            .cloned())
    }

    async fn get_pull_request(
        &self,
        _: i64,
        repo: &str,
        number: i64,
    ) -> Result<PullRequest, GitHubError> {
        self.state
            .lock()
            .unwrap()
            .pulls
            .iter()
            .find(|((r, _), pr)| r == repo && pr.number == number)
            .map(|(_, pr)| pr.clone())
            .ok_or(GitHubError::Status {
                status: 404,
                body: "Not Found".into(),
            })
    }

    async fn create_pull_request(
        &self,
        _: i64,
        repo: &str,
        new: &NewPullRequest<'_>,
    ) -> Result<PullRequest, GitHubError> {
        let mut state = self.state.lock().unwrap();
        let key = (repo.to_string(), new.head.to_string());
        let number = state.next_number;
        state.next_number += 1;
        let pushes = state.pushes.get(&key).copied().unwrap_or(0);
        let pr = PullRequest {
            number,
            url: format!("https://github.com/{repo}/pull/{number}"),
            head_ref: new.head.into(),
            head_sha: format!("{}-sha-{pushes}", new.head),
            state: "open".into(),
            merged: false,
        };
        state.created_pulls.push(key.clone());
        state.pulls.insert(key, pr.clone());
        Ok(pr)
    }

    async fn merge_pull_request(
        &self,
        _: i64,
        repo: &str,
        number: i64,
    ) -> Result<MergeResult, GitHubError> {
        let mut state = self.state.lock().unwrap();
        state.merges.push((repo.into(), number));
        Ok(MergeResult {
            merged: true,
            sha: Some("merged".into()),
        })
    }

    async fn list_issue_comments(
        &self,
        _: i64,
        repo: &str,
        number: i64,
    ) -> Result<Vec<IssueComment>, GitHubError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .comments
            .get(&(repo.to_string(), number))
            .cloned()
            .unwrap_or_default())
    }

    async fn create_issue_comment(
        &self,
        _: i64,
        repo: &str,
        number: i64,
        body: &str,
    ) -> Result<IssueComment, GitHubError> {
        let mut state = self.state.lock().unwrap();
        let id = state.next_comment;
        state.next_comment += 1;
        let comment = IssueComment {
            id,
            body: body.into(),
            url: format!("https://github.com/{repo}/issues/{number}#issuecomment-{id}"),
        };
        state.created_comments.push((repo.into(), number));
        state
            .comments
            .entry((repo.to_string(), number))
            .or_default()
            .push(comment.clone());
        Ok(comment)
    }

    async fn update_issue_comment(
        &self,
        _: i64,
        _: &str,
        comment_id: i64,
        body: &str,
    ) -> Result<IssueComment, GitHubError> {
        let mut state = self.state.lock().unwrap();
        state.updated_comments.push(comment_id);
        let comment = state
            .comments
            .values_mut()
            .flatten()
            .find(|c| c.id == comment_id)
            .expect("the comment exists");
        comment.body = body.into();
        Ok(comment.clone())
    }

    async fn list_check_runs(
        &self,
        _: i64,
        repo: &str,
        sha: &str,
    ) -> Result<Vec<CheckRun>, GitHubError> {
        let mut state = self.state.lock().unwrap();
        let key = (repo.to_string(), sha.to_string());
        if let Some(runs) = state.check_results.get(&key) {
            return Ok(runs.clone());
        }
        let Some(result) = state.check_script.pop_front() else {
            return Ok(vec![CheckRun {
                name: "ci/test".into(),
                status: "in_progress".into(),
                conclusion: None,
                url: "https://ci.example/1".into(),
                summary: None,
                text: None,
            }]);
        };
        let runs = vec![CheckRun {
            name: "ci/test".into(),
            status: "completed".into(),
            conclusion: Some(result.into()),
            url: "https://ci.example/1".into(),
            summary: (result != "success").then(|| "checkout_spec::flaky failed".to_string()),
            text: None,
        }];
        state.check_results.insert(key, runs.clone());
        Ok(runs)
    }
}

#[async_trait]
impl GitHubReader for FakeGitHub {
    async fn issue_relations(
        &self,
        _: i64,
        repo: &str,
        number: i64,
    ) -> Result<IssueRelations, GitHubError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .relations
            .get(&(repo.to_string(), number))
            .cloned()
            .unwrap_or_default())
    }
}

#[async_trait]
impl ConfigSource for FakeGitHub {
    async fn head_commit(&self, _: i64, _: &str) -> Result<String, GitHubError> {
        Ok(self.state.lock().unwrap().head_sha.clone())
    }

    async fn fetch(&self, _: i64, _: &str, _: &str, _: &str) -> Result<ConfigFiles, GitHubError> {
        Ok(self.state.lock().unwrap().files.clone())
    }
}

#[derive(Debug, Default)]
pub struct FakeRunner {
    pub started: Mutex<Vec<Uuid>>,
    pub stopped: Mutex<Vec<Uuid>>,
}

#[async_trait]
impl RunnerPort for FakeRunner {
    async fn start_workspace(&self, spec: &WorkspaceSpec) -> Result<WorkspaceHandle, RunnerError> {
        let mut started = self.started.lock().unwrap();
        started.push(spec.run_id);
        Ok(WorkspaceHandle {
            platform: "local",
            platform_ref: format!("fake-{}-{}", spec.run_id, started.len()),
        })
    }

    async fn stop_workspace(&self, run_id: Uuid, _: &str) -> Result<(), RunnerError> {
        self.stopped.lock().unwrap().push(run_id);
        Ok(())
    }
}

pub struct World {
    pub pg: TestPostgres,
    pub admin: PgPool,
    pub pool: PgPool,
    pub seeded: SeededTenant,
    pub clock: Arc<FakeClock>,
    pub github: Arc<FakeGitHub>,
    pub runner: Arc<FakeRunner>,
    pub worker: Worker,
}

pub fn worker_for(
    pool: &PgPool,
    clock: &Arc<FakeClock>,
    github: &Arc<FakeGitHub>,
    runner: &Arc<FakeRunner>,
) -> Worker {
    Worker::new(
        pool.clone(),
        Deps {
            clock: clock.clone(),
            github: github.clone(),
            reader: github.clone(),
            config_source: github.clone(),
            resources: Arc::new(GitHubResources::new(github.clone())),
            runner: runner.clone(),
        },
        Settings::default(),
    )
}

impl World {
    pub async fn new(plan: &str) -> Self {
        let pg = TestPostgres::start().await;
        let admin = pg.admin_pool().await;
        let pool = pg.worker_pool().await;
        let seeded = pg.seed_tenant(&admin, plan).await;
        sqlx::query("UPDATE sanchoris.users SET github_user_id = 1001, github_login = 'mika-dev', display_name = 'Mika' WHERE id = $1")
            .bind(seeded.user_id)
            .execute(&admin)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO sanchoris.integrations (tenant_id, kind, status, github_installation_id, details) \
             VALUES ($1, 'github_installation', 'connected', $2, '{\"account_login\": \"acme\"}')",
        )
        .bind(seeded.tenant_id)
        .bind(INSTALLATION)
        .execute(&admin)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sanchoris.credentials (tenant_id, scope, name, provider, kind, account, status, encrypted_value, kms_key_id) \
             VALUES ($1, 'org', 'acme-claude-team', 'claude', 'subscription', 'team@acme.test', 'connected', '\\x00', 'test-key')",
        )
        .bind(seeded.tenant_id)
        .execute(&admin)
        .await
        .unwrap();
        let clock = Arc::new(FakeClock::new(epoch()));
        let github = FakeGitHub::new();
        let runner = Arc::new(FakeRunner::default());
        let worker = worker_for(&pool, &clock, &github, &runner);
        let world = Self {
            pg,
            admin,
            pool,
            seeded,
            clock,
            github,
            runner,
            worker,
        };
        assert_eq!(world.worker.tick(Loop::ConfigSyncer).await.unwrap(), 1);
        world
    }

    pub fn tenant(&self) -> Uuid {
        self.seeded.tenant_id
    }

    pub async fn deliver(&self, delivery: &str, event: &str, payload: &Value) -> Recorded {
        ingest_github_delivery(&self.pool, delivery, event, payload)
            .await
            .unwrap()
            .expect("the event is relevant")
    }

    pub async fn label_issue(&self, delivery: &str, number: i64, title: &str, sender: &str) {
        self.deliver(delivery, "issues", &labeled_payload(number, title, sender))
            .await;
    }

    pub async fn run_event(&self, run_id: Uuid, kind: RunEventType, payload: Value) {
        let mut tx = TenantTx::begin(&self.pool, self.tenant()).await.unwrap();
        record_run_event(&mut tx, run_id, None, kind, &payload)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    pub async fn turn_end(&self, run_id: Uuid, turn: i64) {
        self.run_event(
            run_id,
            RunEventType::TurnEnd,
            turn_end_payload(turn, Value::Null),
        )
        .await;
    }

    pub async fn request_skill(&self, run_id: Uuid, input: Value) -> AcceptedSkillRun {
        let mut tx = TenantTx::begin(&self.pool, self.tenant()).await.unwrap();
        let accepted =
            accept_skill_request(&mut tx, run_id, "open-pr-green-ci", input, self.clock_now())
                .await
                .unwrap();
        tx.commit().await.unwrap();
        accepted
    }

    pub async fn continue_skill(&self, run_id: Uuid, number: i32) -> Value {
        let mut tx = TenantTx::begin(&self.pool, self.tenant()).await.unwrap();
        let status = continue_skill_run(&mut tx, run_id, number, json!({}), self.clock_now())
            .await
            .unwrap();
        tx.commit().await.unwrap();
        status
    }

    pub fn clock_now(&self) -> DateTime<Utc> {
        use sanchoris_worker::clock::Clock;
        self.clock.now()
    }

    pub async fn answer_execs(&self, ok: bool) -> usize {
        answer_execs(&self.pool, &self.github, ok).await
    }

    pub async fn drive(&self) {
        drive(&self.worker, &self.pool, &self.github).await;
    }

    pub async fn task(&self, name: &str) -> TaskView {
        task_view(&self.pool, name).await
    }

    pub async fn outbox(&self, run_id: Uuid, kind: &str) -> Vec<Value> {
        sqlx::query_scalar::<_, Value>(
            "SELECT payload FROM sanchoris.run_outbox WHERE run_id = $1 AND type = $2 ORDER BY seq",
        )
        .bind(run_id)
        .bind(kind)
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }

    pub async fn messages(&self, run_id: Uuid) -> Vec<String> {
        self.outbox(run_id, "user_message")
            .await
            .into_iter()
            .map(|p| p["text"].as_str().unwrap().to_string())
            .collect()
    }

    pub async fn trigger_event(&self, key: &str) -> (Option<String>, Option<String>) {
        let row = sqlx::query("SELECT result, reason FROM sanchoris.trigger_events WHERE key = $1")
            .bind(key)
            .fetch_one(&self.pool)
            .await
            .unwrap();
        (row.get("result"), row.get("reason"))
    }

    pub async fn started_task(&self, number: i64) -> TaskView {
        self.label_issue(
            &format!("delivery-{number}"),
            number,
            "Fix flaky checkout test",
            "mika-dev",
        )
        .await;
        self.worker.settle().await.unwrap();
        let task = self.task(&format!("{REPO}#{number}")).await;
        let run_id = task.run_id.expect("the task has a run");
        self.run_event(
            run_id,
            RunEventType::Ready,
            json!({ "sessionId": "session-1", "agent": "claude", "model": "claude-sonnet-5-5", "resumed": false }),
        )
        .await;
        self.worker.settle().await.unwrap();
        self.task(&format!("{REPO}#{number}")).await
    }
}

pub fn skill_input(number: i64) -> Value {
    json!({ "Repos": [REPO], "Repo": REPO, "Issue": number })
}

pub fn turn_end_payload(turn: i64, error: Value) -> Value {
    json!({
        "turn": turn,
        "sessionId": "session-1",
        "result": if error.is_null() { "completed" } else { "failed" },
        "summary": "done for now",
        "error": error,
        "usage": { "inputTokens": 100, "outputTokens": 20, "cacheReadInputTokens": 0, "cacheCreationInputTokens": 0 },
        "totalCostUsd": 0.25,
        "durationMs": 1000,
    })
}

pub fn labeled_payload(number: i64, title: &str, sender: &str) -> Value {
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
            "user": { "login": sender },
            "labels": [{ "name": "sanchoris" }],
        },
        "label": { "name": "sanchoris" },
        "sender": { "login": sender, "id": 1001, "type": "User" },
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskView {
    pub id: Uuid,
    pub status: String,
    pub wait_reason: Option<String>,
    pub nudges: i32,
    pub error: Option<String>,
    pub cause: Option<String>,
    pub resume_at: Option<DateTime<Utc>>,
    pub title: String,
    pub parent_task_id: Option<Uuid>,
    pub requested_by_member_id: Option<Uuid>,
    pub run_id: Option<Uuid>,
}

pub async fn task_view(pool: &PgPool, name: &str) -> TaskView {
    let row = sqlx::query(
        "SELECT t.id, t.status, t.wait_reason, t.nudges, t.error, t.cause, t.resume_at, t.title, \
                t.parent_task_id, t.requested_by_member_id, \
                (SELECT r.id FROM sanchoris.runs r WHERE r.task_id = t.id ORDER BY r.started_at DESC LIMIT 1) AS run_id \
         FROM sanchoris.tasks t WHERE t.name = $1",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap();
    TaskView {
        id: row.get("id"),
        status: row.get("status"),
        wait_reason: row.get("wait_reason"),
        nudges: row.get("nudges"),
        error: row.get("error"),
        cause: row.get("cause"),
        resume_at: row.get("resume_at"),
        title: row.get("title"),
        parent_task_id: row.get("parent_task_id"),
        requested_by_member_id: row.get("requested_by_member_id"),
        run_id: row.get("run_id"),
    }
}

pub async fn answer_execs(pool: &PgPool, github: &FakeGitHub, ok: bool) -> usize {
    let rows = sqlx::query(
        "SELECT o.id, o.run_id, o.payload, r.tenant_id, r.task_id FROM sanchoris.run_outbox o \
         JOIN sanchoris.runs r ON r.id = o.run_id \
         WHERE o.type = 'exec' AND o.delivered_at IS NULL ORDER BY o.seq",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    for row in &rows {
        let payload: Value = row.get("payload");
        let task_id: Uuid = row.get("task_id");
        if ok && payload["purpose"] == "git_push" {
            github.push(REPO, &format!("sanchoris/{task_id}"));
        }
        let results = payload["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| json!({ "command": c, "exitCode": if ok { 0 } else { 1 }, "stdout": "", "stderr": if ok { "" } else { "rejected" } }))
            .collect::<Vec<_>>();
        let mut tx = TenantTx::begin(pool, row.get("tenant_id")).await.unwrap();
        sqlx::query("UPDATE sanchoris.run_outbox SET delivered_at = now() WHERE id = $1")
            .bind(row.get::<Uuid, _>("id"))
            .execute(tx.conn())
            .await
            .unwrap();
        record_run_event(
            &mut tx,
            row.get("run_id"),
            None,
            RunEventType::ExecResult,
            &json!({ "requestId": payload["requestId"], "ok": ok, "results": results }),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
    rows.len()
}

pub async fn drive(worker: &Worker, pool: &PgPool, github: &FakeGitHub) {
    loop {
        worker.settle().await.unwrap();
        if answer_execs(pool, github, true).await == 0 {
            return;
        }
    }
}

pub async fn skill_run(pool: &PgPool, task_id: Uuid, number: i32) -> (String, Option<String>) {
    let row = sqlx::query(
        "SELECT status, error FROM sanchoris.skill_runs WHERE task_id = $1 AND number = $2",
    )
    .bind(task_id)
    .bind(number)
    .fetch_one(pool)
    .await
    .unwrap();
    (row.get("status"), row.get("error"))
}
