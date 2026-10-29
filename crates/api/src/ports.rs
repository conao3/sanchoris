use std::sync::Arc;

use async_trait::async_trait;
use sanchoris_config::{CONFIG_FILE, ConfigFiles, parse_repository, validate};
use sanchoris_graphql::{
    AuditExportPort, ChatDispatcher, ConfigCheck, ConfigLocation, ConfigProblem,
    ConfigRepositoryPort, ConfigSyncTrigger, PortError, SlackConnectUrl, TriggerProposal,
};
use sanchoris_worker::config_sync::resolve_repository;
use sanchoris_worker::error::WorkerError;
use sanchoris_worker::github::{FileChange, GitHubApi};
use sanchoris_worker::ports::{
    ConfigSource, GitHubClient, GitHubRepository, NewPullRequest, OutboxPusher, RunnerError,
    RunnerPort, RunningWorkspace, WorkspaceExit, WorkspaceHandle, WorkspaceSpec, installation_with,
};
use sanchoris_worker::{Worker, config_sync, dispatcher, installation_for_repo, installations};
use serde_yaml_ng::{Mapping, Value as Yaml};
use uuid::Uuid;

use crate::records::{RecordStore, audit_export_key};

fn port_error(error: impl std::fmt::Display) -> PortError {
    PortError::Failed(error.to_string())
}

pub struct SlackUnavailable;

impl SlackConnectUrl for SlackUnavailable {
    fn install(&self, _tenant_id: Uuid) -> Result<String, PortError> {
        Err(PortError::Rejected("Slack is not available yet".into()))
    }
}

pub struct InlineConfigSync {
    worker: Arc<Worker>,
}

impl InlineConfigSync {
    pub fn new(worker: Arc<Worker>) -> Self {
        Self { worker }
    }
}

#[async_trait]
impl ConfigSyncTrigger for InlineConfigSync {
    async fn request_sync(&self, tenant_id: Uuid, config_id: Uuid) -> Result<(), PortError> {
        config_sync::sync_config(&self.worker, tenant_id, config_id)
            .await
            .map_err(port_error)
    }
}

pub struct InlineChatDispatcher {
    worker: Arc<Worker>,
}

impl InlineChatDispatcher {
    pub fn new(worker: Arc<Worker>) -> Self {
        Self { worker }
    }
}

#[async_trait]
impl ChatDispatcher for InlineChatDispatcher {
    async fn dispatch(&self, tenant_id: Uuid, source: &str, key: &str) -> Result<(), PortError> {
        dispatcher::dispatch_inbound(&self.worker, tenant_id, source, key)
            .await
            .map(|_| ())
            .map_err(port_error)
    }
}

pub struct ApiWorkspaces;

const API_RUNS_NO_WORKSPACES: &str = "the api function runs config sync and chat dispatch inline; workspaces, the outbox and tenant deletion belong to the worker function";

#[async_trait]
impl RunnerPort for ApiWorkspaces {
    fn platform(&self) -> &'static str {
        crate::ecs::PLATFORM
    }

    async fn start_workspace(&self, _spec: &WorkspaceSpec) -> Result<WorkspaceHandle, RunnerError> {
        Err(RunnerError(API_RUNS_NO_WORKSPACES.into()))
    }

    async fn running_workspaces(&self) -> Result<Vec<RunningWorkspace>, RunnerError> {
        Err(RunnerError(API_RUNS_NO_WORKSPACES.into()))
    }

    async fn stop_workspace(&self, _run_id: Uuid, _platform_ref: &str) -> Result<(), RunnerError> {
        Err(RunnerError(API_RUNS_NO_WORKSPACES.into()))
    }

    async fn workspace_exit(
        &self,
        _platform_ref: &str,
    ) -> Result<Option<WorkspaceExit>, RunnerError> {
        Err(RunnerError(API_RUNS_NO_WORKSPACES.into()))
    }

    async fn sweep(&self) -> Result<(), RunnerError> {
        Err(RunnerError(API_RUNS_NO_WORKSPACES.into()))
    }
}

#[async_trait]
impl OutboxPusher for ApiWorkspaces {
    async fn push(
        &self,
        _tenant_id: Uuid,
        _run_id: Uuid,
    ) -> Result<(), sanchoris_core::ports::PortError> {
        Err(sanchoris_core::ports::PortError::new(
            "outbox",
            API_RUNS_NO_WORKSPACES,
        ))
    }
}

#[async_trait]
impl sanchoris_core::ports::BlobStore for ApiWorkspaces {
    async fn put(
        &self,
        _key: &str,
        _body: Vec<u8>,
    ) -> Result<(), sanchoris_core::ports::PortError> {
        Err(sanchoris_core::ports::PortError::new(
            "records",
            API_RUNS_NO_WORKSPACES,
        ))
    }

    async fn get(&self, _key: &str) -> Result<Option<Vec<u8>>, sanchoris_core::ports::PortError> {
        Err(sanchoris_core::ports::PortError::new(
            "records",
            API_RUNS_NO_WORKSPACES,
        ))
    }

    async fn delete(&self, _key: &str) -> Result<(), sanchoris_core::ports::PortError> {
        Err(sanchoris_core::ports::PortError::new(
            "records",
            API_RUNS_NO_WORKSPACES,
        ))
    }

    async fn delete_prefix(
        &self,
        _prefix: &str,
        _limit: usize,
    ) -> Result<usize, sanchoris_core::ports::PortError> {
        Err(sanchoris_core::ports::PortError::new(
            "records",
            API_RUNS_NO_WORKSPACES,
        ))
    }
}

pub struct RecordsAuditExport {
    records: Arc<dyn RecordStore>,
}

impl RecordsAuditExport {
    pub fn new(records: Arc<dyn RecordStore>) -> Self {
        Self { records }
    }
}

#[async_trait]
impl AuditExportPort for RecordsAuditExport {
    async fn publish(
        &self,
        tenant_id: Uuid,
        file_name: &str,
        json_lines: Vec<u8>,
    ) -> Result<String, PortError> {
        let key = audit_export_key(tenant_id, file_name).map_err(port_error)?;
        self.records
            .put(tenant_id, &key, json_lines)
            .await
            .map_err(port_error)?;
        self.records
            .presign_get(tenant_id, &key)
            .await
            .map_err(port_error)
    }
}

pub fn config_file_path(root: &str) -> String {
    let trimmed = root.trim_matches('/');
    if trimmed.is_empty() {
        CONFIG_FILE.to_string()
    } else {
        format!("{trimmed}/{CONFIG_FILE}")
    }
}

pub fn add_trigger(config_yaml: &str, trigger_yaml: &str) -> Result<String, String> {
    let mut document: Yaml = serde_yaml_ng::from_str(config_yaml)
        .map_err(|error| format!("{CONFIG_FILE} is not YAML: {error}"))?;
    let addition: Mapping = serde_yaml_ng::from_str(trigger_yaml)
        .map_err(|error| format!("the trigger is not a YAML mapping: {error}"))?;
    let root = document
        .as_mapping_mut()
        .ok_or_else(|| format!("{CONFIG_FILE} is not a mapping"))?;
    let triggers = root
        .entry(Yaml::from("Triggers"))
        .or_insert_with(|| Yaml::Mapping(Mapping::new()))
        .as_mapping_mut()
        .ok_or_else(|| "Triggers is not a mapping".to_string())?;
    for (name, definition) in addition {
        if triggers.contains_key(&name) {
            return Err(format!(
                "Triggers already has {}",
                name.as_str().unwrap_or("the trigger")
            ));
        }
        triggers.insert(name, definition);
    }
    serde_yaml_ng::to_string(&document).map_err(|error| error.to_string())
}

pub struct GitHubConfigRepository {
    worker: Arc<Worker>,
    api: Arc<GitHubApi>,
}

impl GitHubConfigRepository {
    pub fn new(worker: Arc<Worker>, api: Arc<GitHubApi>) -> Self {
        Self { worker, api }
    }

    async fn installation(&self, tenant_id: Uuid, repository: &str) -> Result<i64, PortError> {
        let mut tx = self.worker.tx(tenant_id);
        installation_for_repo(&mut tx, repository)
            .await
            .map_err(|error| match error {
                WorkerError::Invalid(message) => PortError::Rejected(message),
                other => port_error(other),
            })
    }

    async fn files(
        &self,
        installation: i64,
        repository: &GitHubRepository,
        location: &ConfigLocation,
    ) -> Result<Result<ConfigFiles, ConfigProblem>, PortError> {
        let sha = match self
            .api
            .head_commit(installation, repository.id, &location.branch)
            .await
        {
            Ok(sha) => sha,
            Err(error) if matches!(error.status(), Some(404 | 422)) => {
                return Ok(Err(ConfigProblem {
                    path: location.repository.clone(),
                    line: None,
                    column: None,
                    message: format!(
                        "{} has no branch {} that the GitHub App can read",
                        repository.full_name, location.branch
                    ),
                }));
            }
            Err(error) => return Err(port_error(error)),
        };
        self.api
            .fetch(installation, repository.id, &location.root, &sha)
            .await
            .map(Ok)
            .map_err(port_error)
    }
}

#[async_trait]
impl ConfigRepositoryPort for GitHubConfigRepository {
    async fn check(
        &self,
        tenant_id: Uuid,
        location: &ConfigLocation,
    ) -> Result<ConfigCheck, PortError> {
        let mut tx = self.worker.tx(tenant_id);
        let installations = installations(&mut tx).await.map_err(port_error)?;
        let repository =
            match resolve_repository(self.api.as_ref(), &installations, &location.repository).await
            {
                Ok(repository) => repository,
                Err(WorkerError::Invalid(message)) => {
                    return Ok(ConfigCheck::Invalid(vec![ConfigProblem {
                        path: location.repository.clone(),
                        line: None,
                        column: None,
                        message,
                    }]));
                }
                Err(error) => return Err(port_error(error)),
            };
        let installation = installation_with(&installations, repository.id)
            .map(|(installation, _)| installation.id)
            .ok_or_else(|| {
                PortError::Rejected(format!(
                    "{} is not a repository of the tenant's GitHub App installation",
                    repository.full_name
                ))
            })?;
        let files = match self.files(installation, &repository, location).await? {
            Ok(files) => files,
            Err(problem) => return Ok(ConfigCheck::Invalid(vec![problem])),
        };
        let context = config_sync::validation_context(&mut tx)
            .await
            .map_err(port_error)?;
        let errors = match parse_repository(&files) {
            Ok(repository) => validate(&repository, &context).err(),
            Err(errors) => Some(errors),
        };
        Ok(match errors {
            None => ConfigCheck::Valid(repository),
            Some(errors) => ConfigCheck::Invalid(
                errors
                    .0
                    .into_iter()
                    .map(|error| ConfigProblem {
                        path: error.location,
                        line: None,
                        column: None,
                        message: error.message,
                    })
                    .collect(),
            ),
        })
    }

    async fn propose_trigger(
        &self,
        tenant_id: Uuid,
        proposal: &TriggerProposal,
    ) -> Result<i32, PortError> {
        let location = &proposal.location;
        let repository = &location.repository;
        let installation = self.installation(tenant_id, repository).await?;
        let repository_id = self
            .api
            .repository(installation, repository)
            .await
            .map_err(port_error)?
            .ok_or_else(|| PortError::Rejected(format!("{repository} is not readable on GitHub")))?
            .id;
        let base = self
            .api
            .head_commit(installation, repository_id, &location.branch)
            .await
            .map_err(port_error)?;
        let path = config_file_path(&location.root);
        let file = self
            .api
            .file(installation, repository, &path, &base)
            .await
            .map_err(port_error)?
            .ok_or_else(|| {
                PortError::Rejected(format!("{repository} has no {path} on {}", location.branch))
            })?;
        let text = add_trigger(&file.text, &proposal.trigger_yaml).map_err(PortError::Rejected)?;
        let branch = format!(
            "sanchoris/trigger-{}-{}",
            proposal.trigger_name,
            &Uuid::new_v4().simple().to_string()[..8]
        );
        self.api
            .create_branch(installation, repository, &branch, &base)
            .await
            .map_err(port_error)?;
        self.api
            .put_file(
                installation,
                repository,
                &FileChange {
                    path: &path,
                    branch: &branch,
                    message: &proposal.title,
                    text: &text,
                    previous_sha: Some(&file.sha),
                },
            )
            .await
            .map_err(port_error)?;
        let body = format!(
            "Adds the `{}` trigger to `{path}`.\n\nOpened from the Sanchoris trigger templates.",
            proposal.trigger_name
        );
        let pull = self
            .api
            .create_pull_request(
                installation,
                repository,
                &NewPullRequest {
                    head: &branch,
                    base: &location.branch,
                    title: &proposal.title,
                    body: &body,
                },
            )
            .await
            .map_err(port_error)?;
        i32::try_from(pull.number).map_err(port_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers_are_added_to_the_config() {
        let config = "Version: 1\nTriggers:\n  existing:\n    State: ENABLED\n";
        let added = add_trigger(config, "nightly:\n  State: DISABLED\n").unwrap();
        let parsed: Yaml = serde_yaml_ng::from_str(&added).unwrap();
        assert_eq!(parsed["Triggers"]["existing"]["State"], "ENABLED");
        assert_eq!(parsed["Triggers"]["nightly"]["State"], "DISABLED");
        assert!(add_trigger(&added, "nightly:\n  State: ENABLED\n").is_err());
        let fresh = add_trigger("Version: 1\n", "chat:\n  State: ENABLED\n").unwrap();
        let parsed: Yaml = serde_yaml_ng::from_str(&fresh).unwrap();
        assert_eq!(parsed["Triggers"]["chat"]["State"], "ENABLED");
        assert_eq!(config_file_path("/"), "sanchoris.yaml");
        assert_eq!(config_file_path("/ops/"), "ops/sanchoris.yaml");
    }
}
