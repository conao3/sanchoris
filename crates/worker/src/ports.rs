use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sanchoris_config::ConfigFiles;
use sanchoris_engine::{Invocation, ResourceOutcome};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum GitHubError {
    #[error("GitHub returned {status}: {body}")]
    Status { status: u16, body: String },
    #[error("GitHub request failed: {0}")]
    Transport(String),
    #[error("unexpected GitHub response: {0}")]
    Decode(String),
    #[error("GitHub App authentication failed: {0}")]
    Auth(String),
}

impl GitHubError {
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. } => Some(*status),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    pub number: i64,
    pub url: String,
    pub head_ref: String,
    pub head_sha: String,
    pub state: String,
    pub merged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPullRequest<'a> {
    pub head: &'a str,
    pub base: &'a str,
    pub title: &'a str,
    pub body: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueComment {
    pub id: i64,
    pub body: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRun {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: String,
    pub summary: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeResult {
    pub merged: bool,
    pub sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRef {
    pub repository: String,
    pub number: i64,
}

impl IssueRef {
    pub fn task_name(&self) -> String {
        format!("{}#{}", self.repository, self.number)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRelations {
    pub parent: Option<IssueRef>,
    pub blocked_by: Vec<IssueRef>,
}

#[async_trait]
pub trait GitHubClient: Send + Sync {
    async fn default_branch(&self, installation: i64, repo: &str) -> Result<String, GitHubError>;
    async fn find_open_pull_request(
        &self,
        installation: i64,
        repo: &str,
        head_branch: &str,
    ) -> Result<Option<PullRequest>, GitHubError>;
    async fn get_pull_request(
        &self,
        installation: i64,
        repo: &str,
        number: i64,
    ) -> Result<PullRequest, GitHubError>;
    async fn create_pull_request(
        &self,
        installation: i64,
        repo: &str,
        pull_request: &NewPullRequest<'_>,
    ) -> Result<PullRequest, GitHubError>;
    async fn merge_pull_request(
        &self,
        installation: i64,
        repo: &str,
        number: i64,
    ) -> Result<MergeResult, GitHubError>;
    async fn list_issue_comments(
        &self,
        installation: i64,
        repo: &str,
        number: i64,
    ) -> Result<Vec<IssueComment>, GitHubError>;
    async fn create_issue_comment(
        &self,
        installation: i64,
        repo: &str,
        number: i64,
        body: &str,
    ) -> Result<IssueComment, GitHubError>;
    async fn update_issue_comment(
        &self,
        installation: i64,
        repo: &str,
        comment_id: i64,
        body: &str,
    ) -> Result<IssueComment, GitHubError>;
    async fn list_check_runs(
        &self,
        installation: i64,
        repo: &str,
        sha: &str,
    ) -> Result<Vec<CheckRun>, GitHubError>;
}

#[async_trait]
pub trait GitHubReader: Send + Sync {
    async fn issue_relations(
        &self,
        installation: i64,
        repo: &str,
        number: i64,
    ) -> Result<IssueRelations, GitHubError>;
}

#[async_trait]
pub trait ConfigSource: Send + Sync {
    async fn head_commit(
        &self,
        installation: i64,
        repo: &str,
        branch: &str,
    ) -> Result<String, GitHubError>;
    async fn fetch(
        &self,
        installation: i64,
        repo: &str,
        root_path: &str,
        sha: &str,
    ) -> Result<ConfigFiles, GitHubError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installation {
    pub id: i64,
    pub account_login: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InvocationContext {
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub task_name: String,
    pub task_title: String,
    pub skill_run_id: Uuid,
    pub skill_run_number: i32,
    pub state_name: String,
    pub branch: String,
    pub installations: Vec<Installation>,
}

impl InvocationContext {
    pub fn installation_for(&self, repo: &str) -> Result<i64, String> {
        installation_for(&self.installations, repo)
    }
}

pub fn installation_for(installations: &[Installation], repo: &str) -> Result<i64, String> {
    let owner = repo
        .split_once('/')
        .map(|(owner, _)| owner)
        .ok_or_else(|| format!("{repo:?} is not owner/name"))?;
    if let [only] = installations {
        return Ok(only.id);
    }
    installations
        .iter()
        .find(|i| {
            i.account_login
                .as_deref()
                .is_some_and(|login| login.eq_ignore_ascii_case(owner))
        })
        .map(|i| i.id)
        .ok_or_else(|| format!("no GitHub installation of the tenant covers {repo}"))
}

#[async_trait]
pub trait ResourceInvoker: Send + Sync {
    async fn invoke(&self, context: &InvocationContext, invocation: &Invocation)
    -> ResourceOutcome;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSpec {
    pub run_id: Uuid,
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub workspace_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub branch: String,
    pub compute: String,
    pub repositories: Vec<String>,
    pub network_allow: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceHandle {
    pub platform_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningWorkspace {
    pub platform_ref: String,
    pub tenant_id: Uuid,
    pub task_id: Uuid,
    pub run_id: Uuid,
    pub workspace_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceExit {
    pub reason: String,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("runner failed: {0}")]
pub struct RunnerError(pub String);

#[async_trait]
pub trait RunnerPort: Send + Sync {
    fn platform(&self) -> &'static str;
    async fn start_workspace(&self, spec: &WorkspaceSpec) -> Result<WorkspaceHandle, RunnerError>;
    async fn running_workspaces(&self) -> Result<Vec<RunningWorkspace>, RunnerError>;
    async fn stop_workspace(&self, run_id: Uuid, platform_ref: &str) -> Result<(), RunnerError>;
    async fn workspace_exit(
        &self,
        platform_ref: &str,
    ) -> Result<Option<WorkspaceExit>, RunnerError>;
    async fn sweep(&self) -> Result<(), RunnerError>;
}

#[async_trait]
pub trait OutboxPusher: Send + Sync {
    async fn push(
        &self,
        tenant_id: Uuid,
        run_id: Uuid,
    ) -> Result<(), sanchoris_core::ports::PortError>;
}
