use crate::support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use sanchoris_core::items::{
    CheckWait, ConfigRevision, Output, Run, RunEventItem, RunLogEntry, SkillRunItem, Task,
    Workspace,
};
use sanchoris_core::testing::TestDynamo;
use serde_json::{Value, json};
use support::fake_github::FakeGitHub;
use support::{
    Api, CONFIG_YAML, Driving, REPO, SHOP_WEB_ID, Seeded, WEBHOOK_SECRET, Workspaces, bare_repo,
    check_run_payload, eventually, github_signature, labeled_payload, run, seed, start_api,
};
use uuid::Uuid;

const STEP: Duration = Duration::from_secs(60);

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
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
    github: Arc<FakeGitHub>,
    seeded: Seeded,
    records_dir: PathBuf,
    workspaces_dir: PathBuf,
    dynamo: TestDynamo,
    _dir: tempfile::TempDir,
}

impl World {
    async fn start() -> Self {
        let root = repo_root();
        assert!(
            root.join("apps/workspace-agent/node_modules").exists(),
            "run `vp install` before the end-to-end test"
        );
        let dir = tempfile::tempdir().unwrap();
        let git_root = dir.path().join("git");
        bare_repo(&git_root, REPO);
        let github = FakeGitHub::new(
            &git_root,
            BTreeMap::from([("sanchoris.yaml".to_string(), CONFIG_YAML.to_string())]),
        );
        let dynamo = TestDynamo::start().await;
        let seeded = seed(&dynamo).await;
        let workspaces_dir = dir.path().join("workspaces");
        let agent = root.join("scripts/e2e/fake-agent.ts");
        let api = start_api(
            &dynamo,
            github.clone(),
            dir.path(),
            Workspaces::Agent {
                command: vec![
                    "node".into(),
                    "--no-warnings".into(),
                    agent.display().to_string(),
                ],
                socket_dir: dir.path().join("sockets"),
                workspaces_dir: workspaces_dir.clone(),
                git_url: format!("file://{}", git_root.display()),
            },
            Driving::Full,
        )
        .await;
        let world = Self {
            api: Some(api),
            github,
            seeded,
            records_dir: dir.path().join("records"),
            workspaces_dir,
            dynamo,
            _dir: dir,
        };
        eventually("the config to sync", STEP, || async {
            world
                .dynamo
                .all::<ConfigRevision>()
                .await
                .iter()
                .any(|revision| {
                    revision.commit_sha == support::fake_github::CONFIG_SHA
                        && revision.validation_status == "valid"
                })
                .then_some(())
        })
        .await;
        world
    }

    fn api(&self) -> &Api {
        self.api.as_ref().expect("the api is running")
    }

    async fn webhook(&self, event: &str, delivery: &str, payload: &Value) -> (StatusCode, Value) {
        let body = serde_json::to_vec(payload).unwrap();
        let response = self
            .api()
            .http
            .post(format!("{}/webhooks/github", self.api().url))
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
        let name = format!("{REPO}#{number}");
        let task = self
            .dynamo
            .all::<Task>()
            .await
            .into_iter()
            .find(|task| task.display_name(&[]) == name)?;
        let run_id = self
            .dynamo
            .all::<Run>()
            .await
            .into_iter()
            .filter(|run| run.task_id == task.id)
            .max_by_key(|run| run.started_at)
            .map(|run| run.id);
        Some(TaskRow {
            id: task.id,
            status: task.status.as_str().to_string(),
            error: task.error,
            run_id,
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
            self.dynamo
                .all::<CheckWait>()
                .await
                .into_iter()
                .filter(|wait| wait.task_id == task_id && wait.status == "waiting")
                .filter_map(|wait| wait.head_shas[REPO].as_str().map(str::to_string))
                .find(|sha| !seen.iter().any(|s| s == sha))
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
        let mut runs = self
            .dynamo
            .all::<SkillRunItem>()
            .await
            .into_iter()
            .filter(|run| run.task_id == task_id)
            .map(|run| (run.number, run.status.as_str().to_string(), run.error))
            .collect::<Vec<_>>();
        runs.sort();
        runs
    }

    async fn run_events(&self, run_id: Uuid) -> Vec<RunEventItem> {
        let mut events = self
            .dynamo
            .all::<RunEventItem>()
            .await
            .into_iter()
            .filter(|event| event.run_id == run_id)
            .collect::<Vec<_>>();
        events.sort_by_key(|event| (event.seq, event.received_at));
        events
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

    async fn reply(&self, task_id: Uuid, text: &str) {
        let response: Value = self
            .api()
            .http
            .post(format!("{}/api/graphql", self.api().url))
            .bearer_auth(self.seeded.member_token())
            .json(&json!({
                "query": "mutation($input: ResumeTaskInput!) { resumeTask(input: $input) { id } }",
                "variables": { "input": { "taskId": task_id, "message": text } },
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(response["errors"].is_null(), "{response}");
    }

    fn aws_lookup(&self, number: i64, since_ms: i64) -> Value {
        let out = run(
            &[
                "node",
                "scripts/e2e/aws-lookup.mjs",
                "--endpoint",
                &self.dynamo.endpoint_url(),
                "--table",
                &self.dynamo.tables().main,
                "--slug",
                &format!("tenant-{}", self.seeded.tenant.user_id.simple()),
                "--repository-id",
                &SHOP_WEB_ID.to_string(),
                "--issue",
                &number.to_string(),
                "--since-ms",
                &since_ms.to_string(),
            ],
            &repo_root(),
        );
        serde_json::from_str(&out).unwrap()
    }

    async fn expected_lookup(&self, task_id: Uuid) -> Value {
        let task = self
            .dynamo
            .all::<Task>()
            .await
            .into_iter()
            .find(|task| task.id == task_id)
            .unwrap();
        let mut runs = Vec::new();
        for run in self.dynamo.all::<Run>().await {
            if run.task_id == task_id {
                let logs = self
                    .dynamo
                    .all::<RunLogEntry>()
                    .await
                    .iter()
                    .filter(|entry| entry.run_id == run.id)
                    .count();
                runs.push(json!({ "id": run.id, "status": run.status.as_str(), "logs": logs }));
            }
        }
        let workspaces = self
            .dynamo
            .all::<Workspace>()
            .await
            .into_iter()
            .filter(|workspace| workspace.task_id == task_id)
            .map(|workspace| json!({ "id": workspace.id, "run": workspace.run_id }))
            .collect::<Vec<_>>();
        let outputs = self
            .dynamo
            .all::<Output>()
            .await
            .into_iter()
            .filter(|output| output.task_id == task_id)
            .map(|output| json!({ "kind": output.kind, "url": output.url }))
            .collect::<Vec<_>>();
        assert!(!runs.is_empty() && !workspaces.is_empty() && !outputs.is_empty());
        json!({
            "tenant": self.seeded.tenant.tenant_id,
            "task": { "id": task.id, "status": task.status.as_str() },
            "runs": runs,
            "workspaces": workspaces,
            "outputs": outputs,
        })
    }

    fn workspace_count(&self) -> usize {
        std::fs::read_dir(&self.workspaces_dir)
            .map(|entries| entries.count())
            .unwrap_or(0)
    }

    async fn stop(&mut self) {
        drop(self.api.take());
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn sorted_lists(mut value: Value) -> Value {
    for list in value.as_object_mut().unwrap().values_mut() {
        if let Some(items) = list.as_array_mut() {
            items.sort_by_key(|item| item.to_string());
        }
    }
    value
}

fn branch(task: &TaskRow) -> String {
    format!("sanchoris/{}", task.id)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_labeled_issue_opens_a_pull_request_that_goes_green_after_a_fix() {
    let mut world = World::start().await;

    let since = chrono::Utc::now().timestamp_millis();
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
    let record_key = world
        .dynamo
        .all::<Run>()
        .await
        .into_iter()
        .find(|run| run.id == run_id)
        .and_then(|run| run.record_s3_key)
        .expect("the run saved a session record");
    assert!(world.records_dir.join(&record_key).exists());
    assert!(
        world
            .dynamo
            .all::<RunLogEntry>()
            .await
            .iter()
            .any(|entry| entry.run_id == run_id)
    );
    let events = world.run_events(run_id).await;
    let execs = events
        .iter()
        .filter(|event| event.kind == "exec_result")
        .collect::<Vec<_>>();
    assert_eq!(execs.len(), 2);
    assert!(execs.iter().all(|event| event.payload["ok"] == json!(true)));
    assert!(events.iter().all(|event| event.error.is_none()));
    eventually("the workspace to stop", STEP, || async {
        let open = world
            .dynamo
            .all::<Workspace>()
            .await
            .into_iter()
            .filter(|workspace| workspace.run_id == run_id && workspace.stopped_at.is_none())
            .count();
        (open == 0 && world.workspace_count() == 0).then_some(())
    })
    .await;
    assert_eq!(
        sorted_lists(world.aws_lookup(35, since)),
        sorted_lists(world.expected_lookup(task.id).await)
    );
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

    world
        .reply(
            task.id,
            "The flaky spec needs a retry. Run the skill again.",
        )
        .await;
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
