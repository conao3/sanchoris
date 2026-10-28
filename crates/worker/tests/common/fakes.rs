use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use sanchoris_config::ConfigFiles;
use sanchoris_worker::ports::{
    CheckRun, ConfigSource, GitHubClient, GitHubError, GitHubReader, IssueComment, IssueRelations,
    MergeResult, NewPullRequest, PullRequest, RunnerError, RunnerPort, WorkspaceExit,
    WorkspaceHandle, WorkspaceSpec,
};
use uuid::Uuid;

pub const CONFIG_YAML: &str = include_str!("../../../config/tests/fixtures/example.sanchoris.yaml");
pub const INSTALLATION: i64 = 42;
pub const REPO: &str = "acme/shop-web";
pub const APP_LOGIN: &str = "sanchoris[bot]";

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
    pub relation_errors: HashMap<(String, i64), GitHubError>,
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
        let state = self.state.lock().unwrap();
        if let Some(error) = state.relation_errors.get(&(repo.to_string(), number)) {
            return Err(error.clone());
        }
        Ok(state
            .relations
            .get(&(repo.to_string(), number))
            .cloned()
            .unwrap_or_default())
    }

    async fn app_login(&self) -> Result<String, GitHubError> {
        Ok(APP_LOGIN.to_string())
    }
}

#[async_trait]
impl ConfigSource for FakeGitHub {
    async fn head_commit(&self, _: i64, _: &str, _: &str) -> Result<String, GitHubError> {
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
    pub refs: Mutex<Vec<String>>,
    pub exits: Mutex<HashMap<String, WorkspaceExit>>,
    pub sweeps: Mutex<usize>,
}

#[async_trait]
impl RunnerPort for FakeRunner {
    async fn start_workspace(&self, spec: &WorkspaceSpec) -> Result<WorkspaceHandle, RunnerError> {
        let mut started = self.started.lock().unwrap();
        started.push(spec.run_id);
        let platform_ref = format!("fake-{}-{}", spec.run_id, started.len());
        self.refs.lock().unwrap().push(platform_ref.clone());
        Ok(WorkspaceHandle {
            platform: "local",
            platform_ref,
        })
    }

    async fn stop_workspace(&self, run_id: Uuid, _: &str) -> Result<(), RunnerError> {
        self.stopped.lock().unwrap().push(run_id);
        Ok(())
    }

    async fn workspace_exit(
        &self,
        platform_ref: &str,
    ) -> Result<Option<WorkspaceExit>, RunnerError> {
        Ok(self.exits.lock().unwrap().get(platform_ref).cloned())
    }

    async fn sweep(&self) -> Result<(), RunnerError> {
        *self.sweeps.lock().unwrap() += 1;
        Ok(())
    }
}
