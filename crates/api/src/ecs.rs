use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use aws_sdk_dynamodb::types::{AttributeValue, ReturnValue};
use aws_sdk_ecs::error::DisplayErrorContext;
use aws_sdk_ecs::operation::run_task::RunTaskError;
use aws_sdk_ecs::types::{
    AssignPublicIp, AwsVpcConfiguration, ContainerOverride, DesiredStatus, KeyValuePair,
    LaunchType, NetworkConfiguration, Tag, Task, TaskField, TaskOverride,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use sanchoris_core::clock::Clock;
use sanchoris_core::store::Store;
use sanchoris_core::tokens::{RELAY_TOKEN_SIGNING_KEY_ENV, RelayTokenSigner, RunTokenSigner};
use sanchoris_worker::deadline;
use sanchoris_worker::ports::{
    RunnerError, RunnerPort, RunningWorkspace, WorkspaceExit, WorkspaceHandle, WorkspaceSpec,
};
use uuid::Uuid;

use crate::config::{ConfigError, Reader, Secrets, run_tokens, signer};

pub const PLATFORM: &str = "ecs";
const TAG_TENANT: &str = "sanchoris:tenant";
const TAG_TASK: &str = "sanchoris:task";
const TAG_RUN: &str = "sanchoris:run";
const TAG_WORKSPACE: &str = "sanchoris:workspace";
const TOKEN_TTL_HOURS: i64 = 12;
const RELAY_LEASE_PK: &str = "RELAY";
const RELAY_LEASE_SK: &str = "#";
const RELAY_LEASE_KIND: &str = "relay_lease";
const LOCK_SECONDS: i64 = 60;
const LOCK_WAIT_SECONDS: i64 = 90;
const RELAY_START_SECONDS: i64 = 240;
const HOLDER_GRACE_MINUTES: i64 = 10;
const POLL_MILLIS: u64 = 3000;

#[derive(Debug, Clone)]
pub struct EcsRunnerConfig {
    pub cluster: String,
    pub task_definition: String,
    pub container: String,
    pub subnets: Vec<String>,
    pub security_groups: Vec<String>,
    pub assign_public_ip: AssignPublicIp,
    pub control_url: String,
    pub git_url: String,
    pub relay_service: String,
    pub relay_port: u16,
    pub run_tokens: RunTokenSigner,
    pub relay_tokens: RelayTokenSigner,
}

fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

impl EcsRunnerConfig {
    pub fn load(secrets: &Secrets) -> Result<Self, ConfigError> {
        let mut env = Reader::new();
        let cluster = env.get("SANCHORIS_ECS_CLUSTER");
        let task_definition = env.get("SANCHORIS_ECS_TASK_DEFINITION");
        let container = env.get("SANCHORIS_ECS_CONTAINER");
        let subnets = env.get("SANCHORIS_ECS_SUBNETS");
        let security_groups = env.get("SANCHORIS_ECS_SECURITY_GROUPS");
        let assign_public_ip = env.get("SANCHORIS_ECS_ASSIGN_PUBLIC_IP");
        let control_url = env.get("SANCHORIS_CONTROL_URL");
        let git_url = env.get("SANCHORIS_GIT_URL");
        let relay_service = env.get("SANCHORIS_RELAY_SERVICE");
        let relay_port = env.get("SANCHORIS_RELAY_PORT");
        let relay_key = env.secret(secrets, RELAY_TOKEN_SIGNING_KEY_ENV);
        env.finish()?;
        let assign_public_ip = match assign_public_ip.as_str() {
            "ENABLED" => AssignPublicIp::Enabled,
            "DISABLED" => AssignPublicIp::Disabled,
            other => {
                return Err(ConfigError::Invalid {
                    name: "SANCHORIS_ECS_ASSIGN_PUBLIC_IP",
                    message: format!("{other:?} is not ENABLED or DISABLED"),
                });
            }
        };
        let relay_port = relay_port
            .parse::<u16>()
            .map_err(|_| ConfigError::Invalid {
                name: "SANCHORIS_RELAY_PORT",
                message: format!("{relay_port:?} is not a port number"),
            })?;
        Ok(Self {
            cluster,
            task_definition,
            container,
            subnets: list(&subnets),
            security_groups: list(&security_groups),
            assign_public_ip,
            control_url,
            git_url: git_url.trim_end_matches('/').to_string(),
            relay_service,
            relay_port,
            run_tokens: run_tokens(secrets)?,
            relay_tokens: RelayTokenSigner::new(signer(RELAY_TOKEN_SIGNING_KEY_ENV, &relay_key)?),
        })
    }
}

pub struct EcsRunner {
    client: aws_sdk_ecs::Client,
    relay: RelayLease,
    config: EcsRunnerConfig,
}

impl EcsRunner {
    pub fn new(
        sdk: &aws_config::SdkConfig,
        store: &Store,
        config: EcsRunnerConfig,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let client = aws_sdk_ecs::Client::new(sdk);
        Self {
            relay: RelayLease {
                ecs: client.clone(),
                dynamodb: store.client().clone(),
                table: store.tables().main.clone(),
                cluster: config.cluster.clone(),
                service: config.relay_service.clone(),
                workspace_family: config.task_definition.clone(),
                clock,
            },
            client,
            config,
        }
    }

    pub fn environment(
        &self,
        spec: &WorkspaceSpec,
        relay_host: &str,
    ) -> Vec<(&'static str, String)> {
        let expires_at = spec.started_at + Duration::hours(TOKEN_TTL_HOURS);
        let token = self
            .config
            .run_tokens
            .sign(spec.tenant_id, spec.run_id, expires_at);
        let relay_token =
            self.config
                .relay_tokens
                .sign(spec.run_id, &spec.network_allow, expires_at);
        let relay = format!(
            "http://run:{relay_token}@{relay_host}:{}",
            self.config.relay_port
        );
        vec![
            ("SANCHORIS_RUN_ID", spec.run_id.to_string()),
            ("SANCHORIS_RUN_TOKEN", token),
            ("SANCHORIS_CONTROL_URL", self.config.control_url.clone()),
            ("SANCHORIS_GIT_URL", self.config.git_url.clone()),
            ("SANCHORIS_BRANCH", spec.branch.clone()),
            ("SANCHORIS_REPOSITORIES", spec.repositories.join(",")),
            ("SANCHORIS_CREDENTIAL_SOURCE", "channel".to_string()),
            ("SANCHORIS_RELAY_URL", relay),
        ]
    }

    async fn run_task(
        &self,
        spec: &WorkspaceSpec,
        relay_host: &str,
    ) -> Result<WorkspaceHandle, RunnerError> {
        let environment = self
            .environment(spec, relay_host)
            .into_iter()
            .map(|(name, value)| KeyValuePair::builder().name(name).value(value).build())
            .collect::<Vec<_>>();
        let network = AwsVpcConfiguration::builder()
            .set_subnets(Some(self.config.subnets.clone()))
            .set_security_groups(Some(self.config.security_groups.clone()))
            .assign_public_ip(self.config.assign_public_ip.clone())
            .build()
            .map_err(runner_error)?;
        let tags = [
            (TAG_TENANT, spec.tenant_id),
            (TAG_TASK, spec.task_id),
            (TAG_RUN, spec.run_id),
            (TAG_WORKSPACE, spec.workspace_id),
        ]
        .into_iter()
        .map(|(key, id)| Tag::builder().key(key).value(id.to_string()).build())
        .collect::<Vec<_>>();
        let result = self
            .client
            .run_task()
            .cluster(&self.config.cluster)
            .task_definition(&self.config.task_definition)
            .launch_type(LaunchType::Fargate)
            .client_token(client_token(spec))
            .started_by(holder(spec.run_id))
            .set_tags(Some(tags))
            .network_configuration(
                NetworkConfiguration::builder()
                    .awsvpc_configuration(network)
                    .build(),
            )
            .overrides(
                TaskOverride::builder()
                    .container_overrides(
                        ContainerOverride::builder()
                            .name(&self.config.container)
                            .set_environment(Some(environment))
                            .build(),
                    )
                    .build(),
            )
            .send()
            .await;
        let output = match result {
            Ok(output) => output,
            Err(error) => {
                if let Some(RunTaskError::ConflictException(conflict)) = error.as_service_error()
                    && let Some(task) = conflict.resource_ids().first()
                {
                    return Ok(WorkspaceHandle {
                        platform_ref: task.clone(),
                    });
                }
                return Err(runner_error(error));
            }
        };
        if let Some(failure) = output.failures().first() {
            return Err(RunnerError(format!(
                "RunTask failed: {} {}",
                failure.reason().unwrap_or("without a reason"),
                failure.detail().unwrap_or_default()
            )));
        }
        let task = output
            .tasks()
            .first()
            .and_then(|task| task.task_arn())
            .ok_or_else(|| RunnerError("RunTask returned no task".into()))?;
        Ok(WorkspaceHandle {
            platform_ref: task.to_string(),
        })
    }
}

pub fn client_token(spec: &WorkspaceSpec) -> String {
    format!("{}{}", spec.run_id.simple(), spec.workspace_id.simple())
}

pub fn running_workspace(task: &Task) -> Option<RunningWorkspace> {
    let tag = |key: &str| {
        task.tags()
            .iter()
            .find(|tag| tag.key() == Some(key))
            .and_then(|tag| tag.value())
            .and_then(|value| Uuid::parse_str(value).ok())
    };
    Some(RunningWorkspace {
        platform_ref: task.task_arn()?.to_string(),
        tenant_id: tag(TAG_TENANT)?,
        task_id: tag(TAG_TASK)?,
        run_id: tag(TAG_RUN)?,
        workspace_id: tag(TAG_WORKSPACE)?,
    })
}

fn runner_error(error: impl std::error::Error) -> RunnerError {
    RunnerError(DisplayErrorContext(error).to_string())
}

fn holder(run_id: Uuid) -> String {
    run_id.simple().to_string()
}

fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

pub fn exit_reason(task: &Task, container: &str) -> String {
    let mut reason = task
        .stopped_reason()
        .unwrap_or("the ECS task stopped without a reason")
        .to_string();
    if let Some(code) = task.stop_code() {
        reason.push_str(&format!(" ({})", code.as_str()));
    }
    if let Some(container) = task
        .containers()
        .iter()
        .find(|c| c.name() == Some(container))
    {
        match container.exit_code() {
            Some(code) => reason.push_str(&format!("; container exit code {code}")),
            None => reason.push_str("; the container did not start"),
        }
        if let Some(detail) = container.reason() {
            reason.push_str(&format!(": {detail}"));
        }
    }
    reason
}

pub fn private_ip(task: &Task) -> Option<String> {
    task.attachments()
        .iter()
        .filter(|a| a.r#type() == Some("ElasticNetworkInterface"))
        .flat_map(|a| a.details())
        .find(|d| d.name() == Some("privateIPv4Address"))
        .and_then(|d| d.value())
        .map(str::to_string)
}

struct RelayLease {
    ecs: aws_sdk_ecs::Client,
    dynamodb: aws_sdk_dynamodb::Client,
    table: String,
    cluster: String,
    service: String,
    workspace_family: String,
    clock: Arc<dyn Clock>,
}

struct Lock {
    owner: String,
    holders: BTreeMap<String, String>,
}

impl RelayLease {
    fn key(&self) -> HashMap<String, AttributeValue> {
        HashMap::from([
            ("pk".to_string(), AttributeValue::S(RELAY_LEASE_PK.into())),
            ("sk".to_string(), AttributeValue::S(RELAY_LEASE_SK.into())),
        ])
    }

    async fn lock(&self) -> Result<Lock, RunnerError> {
        let owner = Uuid::new_v4().to_string();
        let deadline = deadline::bounded(self.clock.now() + Duration::seconds(LOCK_WAIT_SECONDS));
        loop {
            let now = self.clock.now();
            let result = self
                .dynamodb
                .update_item()
                .table_name(&self.table)
                .set_key(Some(self.key()))
                .update_expression(
                    "SET lock_owner = :owner, lock_until = :until, kind = :kind, holders = if_not_exists(holders, :empty)",
                )
                .condition_expression("attribute_not_exists(lock_until) OR lock_until < :now")
                .expression_attribute_values(":owner", AttributeValue::S(owner.clone()))
                .expression_attribute_values(
                    ":until",
                    AttributeValue::S(timestamp(now + Duration::seconds(LOCK_SECONDS))),
                )
                .expression_attribute_values(":now", AttributeValue::S(timestamp(now)))
                .expression_attribute_values(":kind", AttributeValue::S(RELAY_LEASE_KIND.into()))
                .expression_attribute_values(":empty", AttributeValue::M(HashMap::new()))
                .return_values(ReturnValue::AllNew)
                .send()
                .await;
            match result {
                Ok(output) => {
                    let holders = output
                        .attributes()
                        .and_then(|item| item.get("holders"))
                        .and_then(|value| value.as_m().ok())
                        .map(|map| {
                            map.iter()
                                .filter_map(|(run, at)| {
                                    at.as_s().ok().map(|at| (run.clone(), at.clone()))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    return Ok(Lock { owner, holders });
                }
                Err(error)
                    if error
                        .as_service_error()
                        .is_some_and(|e| e.is_conditional_check_failed_exception()) =>
                {
                    if self.clock.now() >= deadline {
                        return Err(RunnerError(format!(
                            "the relay lease stayed locked until {}",
                            timestamp(deadline)
                        )));
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                Err(error) => return Err(runner_error(error)),
            }
        }
    }

    async fn unlock(&self, lock: Lock) -> Result<(), RunnerError> {
        let holders = lock
            .holders
            .into_iter()
            .map(|(run, at)| (run, AttributeValue::S(at)))
            .collect::<HashMap<_, _>>();
        self.dynamodb
            .update_item()
            .table_name(&self.table)
            .set_key(Some(self.key()))
            .update_expression("SET holders = :holders REMOVE lock_owner, lock_until")
            .condition_expression("lock_owner = :owner")
            .expression_attribute_values(":holders", AttributeValue::M(holders))
            .expression_attribute_values(":owner", AttributeValue::S(lock.owner))
            .send()
            .await
            .map_err(|error| {
                RunnerError(format!(
                    "the relay lease was lost before it was released: {}",
                    DisplayErrorContext(error)
                ))
            })?;
        Ok(())
    }

    async fn desired_count(&self) -> Result<Option<i32>, RunnerError> {
        let output = self
            .ecs
            .describe_services()
            .cluster(&self.cluster)
            .services(&self.service)
            .send()
            .await
            .map_err(runner_error)?;
        Ok(output
            .services()
            .iter()
            .find(|s| s.status() == Some("ACTIVE"))
            .map(|s| s.desired_count()))
    }

    async fn scale(&self, desired: i32) -> Result<(), RunnerError> {
        let current = self.desired_count().await?.ok_or_else(|| {
            RunnerError(format!(
                "the egress relay service {} is not active in {} (deploy with Workspaces enabled)",
                self.service, self.cluster
            ))
        })?;
        if current == desired {
            return Ok(());
        }
        self.ecs
            .update_service()
            .cluster(&self.cluster)
            .service(&self.service)
            .desired_count(desired)
            .send()
            .await
            .map_err(runner_error)?;
        Ok(())
    }

    async fn tasks(&self, arns: Vec<String>) -> Result<Vec<Task>, RunnerError> {
        let mut tasks = Vec::new();
        for chunk in arns.chunks(100) {
            let output = self
                .ecs
                .describe_tasks()
                .cluster(&self.cluster)
                .set_tasks(Some(chunk.to_vec()))
                .include(TaskField::Tags)
                .send()
                .await
                .map_err(runner_error)?;
            tasks.extend(output.tasks().iter().cloned());
        }
        Ok(tasks)
    }

    async fn list(
        &self,
        family: Option<&str>,
        service: Option<&str>,
    ) -> Result<Vec<Task>, RunnerError> {
        let mut arns = Vec::new();
        let mut next = None;
        loop {
            let output = self
                .ecs
                .list_tasks()
                .cluster(&self.cluster)
                .set_family(family.map(str::to_string))
                .set_service_name(service.map(str::to_string))
                .desired_status(DesiredStatus::Running)
                .set_next_token(next)
                .send()
                .await
                .map_err(runner_error)?;
            arns.extend(output.task_arns().iter().cloned());
            next = output.next_token().map(str::to_string);
            if next.is_none() {
                break;
            }
        }
        self.tasks(arns).await
    }

    async fn running_workspaces(&self) -> Result<BTreeSet<String>, RunnerError> {
        Ok(self
            .list(Some(&self.workspace_family), None)
            .await?
            .iter()
            .filter_map(|task| task.started_by().map(str::to_string))
            .collect())
    }

    async fn acquire(&self, run_id: Uuid) -> Result<String, RunnerError> {
        let mut lock = self.lock().await?;
        let scaled = self.scale(1).await;
        if scaled.is_ok() {
            lock.holders
                .insert(holder(run_id), timestamp(self.clock.now()));
        }
        self.unlock(lock).await?;
        scaled?;
        self.relay_address().await
    }

    async fn relay_address(&self) -> Result<String, RunnerError> {
        let deadline = deadline::bounded(self.clock.now() + Duration::seconds(RELAY_START_SECONDS));
        loop {
            let tasks = self.list(None, Some(&self.service)).await?;
            if let Some(ip) = tasks
                .iter()
                .filter(|task| task.last_status() == Some("RUNNING"))
                .find_map(private_ip)
            {
                return Ok(ip);
            }
            if self.clock.now() >= deadline {
                return Err(RunnerError(format!(
                    "the egress relay did not reach RUNNING by {}",
                    timestamp(deadline)
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_MILLIS)).await;
        }
    }

    async fn release(&self, run_id: Option<Uuid>) -> Result<(), RunnerError> {
        let mut lock = self.lock().await?;
        if let Some(run_id) = run_id {
            lock.holders.remove(&holder(run_id));
        }
        let pruned = self.scale_down_if_unused(&mut lock.holders).await;
        self.unlock(lock).await?;
        pruned
    }

    async fn scale_down_if_unused(
        &self,
        holders: &mut BTreeMap<String, String>,
    ) -> Result<(), RunnerError> {
        let running = self.running_workspaces().await?;
        let fresh_after = timestamp(self.clock.now() - Duration::minutes(HOLDER_GRACE_MINUTES));
        holders.retain(|run, at| running.contains(run) || *at > fresh_after);
        if holders.is_empty() && running.is_empty() {
            self.scale(0).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl RunnerPort for EcsRunner {
    fn platform(&self) -> &'static str {
        PLATFORM
    }

    async fn running_workspaces(&self) -> Result<Vec<RunningWorkspace>, RunnerError> {
        Ok(self
            .relay
            .list(Some(&self.config.task_definition), None)
            .await?
            .iter()
            .filter_map(running_workspace)
            .collect())
    }

    async fn start_workspace(&self, spec: &WorkspaceSpec) -> Result<WorkspaceHandle, RunnerError> {
        if spec.compute != "standard" {
            return Err(RunnerError(format!(
                "compute {:?} has no Fargate task definition",
                spec.compute
            )));
        }
        let relay_host = self.relay.acquire(spec.run_id).await?;
        match self.run_task(spec, &relay_host).await {
            Ok(handle) => Ok(handle),
            Err(error) => match self.relay.release(Some(spec.run_id)).await {
                Ok(()) => Err(error),
                Err(release) => Err(RunnerError(format!("{}; {}", error.0, release.0))),
            },
        }
    }

    async fn stop_workspace(&self, run_id: Uuid, platform_ref: &str) -> Result<(), RunnerError> {
        self.client
            .stop_task()
            .cluster(&self.config.cluster)
            .task(platform_ref)
            .reason(format!("Run {run_id} no longer needs its workspace"))
            .send()
            .await
            .map_err(runner_error)?;
        self.relay.release(Some(run_id)).await
    }

    async fn workspace_exit(
        &self,
        platform_ref: &str,
    ) -> Result<Option<WorkspaceExit>, RunnerError> {
        let output = self
            .client
            .describe_tasks()
            .cluster(&self.config.cluster)
            .tasks(platform_ref)
            .send()
            .await
            .map_err(runner_error)?;
        if let Some(task) = output.tasks().first() {
            if task.last_status() != Some("STOPPED") {
                return Ok(None);
            }
            return Ok(Some(WorkspaceExit {
                reason: exit_reason(task, &self.config.container),
            }));
        }
        Ok(Some(WorkspaceExit {
            reason: format!("the ECS task {platform_ref} is gone"),
        }))
    }

    async fn sweep(&self) -> Result<(), RunnerError> {
        match self.relay.desired_count().await? {
            None | Some(0) => Ok(()),
            Some(_) => self.relay.release(None).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_ecs::types::{Attachment, Container, KeyValuePair, TaskStopCode};

    #[test]
    fn the_exit_reason_names_the_stop_and_the_container() {
        let task = Task::builder()
            .stopped_reason("Essential container in task exited")
            .stop_code(TaskStopCode::EssentialContainerExited)
            .containers(Container::builder().name("workspace").exit_code(1).build())
            .build();
        assert_eq!(
            exit_reason(&task, "workspace"),
            "Essential container in task exited (EssentialContainerExited); container exit code 1"
        );
        let pull = Task::builder()
            .stopped_reason("CannotPullContainerError: pull access denied")
            .stop_code(TaskStopCode::TaskFailedToStart)
            .containers(
                Container::builder()
                    .name("workspace")
                    .reason("CannotPullContainerError")
                    .build(),
            )
            .build();
        assert_eq!(
            exit_reason(&pull, "workspace"),
            "CannotPullContainerError: pull access denied (TaskFailedToStart); the container did not start: CannotPullContainerError"
        );
    }

    fn spec() -> WorkspaceSpec {
        WorkspaceSpec {
            run_id: Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap(),
            tenant_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            workspace_id: Uuid::parse_str("00000000-0000-4000-8000-0000000000ff").unwrap(),
            started_at: Utc::now(),
            branch: "sanchoris/x".into(),
            compute: "standard".into(),
            repositories: vec![],
            network_allow: vec![],
        }
    }

    #[test]
    fn the_client_token_is_the_run_and_the_workspace_within_the_ecs_limit() {
        let token = client_token(&spec());
        assert_eq!(
            token,
            "00000000000040008000000000000001000000000000400080000000000000ff"
        );
        assert_eq!(token.len(), 64);
        assert_eq!(client_token(&spec()), token);
    }

    #[test]
    fn running_workspaces_come_from_the_task_tags() {
        let spec = spec();
        let tag = |key: &str, id: Uuid| Tag::builder().key(key).value(id.to_string()).build();
        let task = Task::builder()
            .task_arn("arn:aws:ecs:ap-northeast-1:1:task/c/abc")
            .tags(tag(TAG_TENANT, spec.tenant_id))
            .tags(tag(TAG_TASK, spec.task_id))
            .tags(tag(TAG_RUN, spec.run_id))
            .tags(tag(TAG_WORKSPACE, spec.workspace_id))
            .build();
        assert_eq!(
            running_workspace(&task),
            Some(RunningWorkspace {
                platform_ref: "arn:aws:ecs:ap-northeast-1:1:task/c/abc".into(),
                tenant_id: spec.tenant_id,
                task_id: spec.task_id,
                run_id: spec.run_id,
                workspace_id: spec.workspace_id,
            })
        );
        let untagged = Task::builder()
            .task_arn("arn:aws:ecs:ap-northeast-1:1:task/c/def")
            .build();
        assert_eq!(running_workspace(&untagged), None);
    }

    #[test]
    fn the_relay_address_is_the_private_ip_of_the_task_eni() {
        let task = Task::builder()
            .attachments(
                Attachment::builder()
                    .r#type("ElasticNetworkInterface")
                    .details(
                        KeyValuePair::builder()
                            .name("subnetId")
                            .value("subnet-1")
                            .build(),
                    )
                    .details(
                        KeyValuePair::builder()
                            .name("privateIPv4Address")
                            .value("10.1.0.12")
                            .build(),
                    )
                    .build(),
            )
            .build();
        assert_eq!(private_ip(&task).as_deref(), Some("10.1.0.12"));
        assert_eq!(private_ip(&Task::builder().build()), None);
    }
}
