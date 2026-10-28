use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use aws_sdk_dynamodb::types::{AttributeValue, ReturnValue};
use aws_sdk_ecs::error::DisplayErrorContext;
use aws_sdk_ecs::operation::run_task::RunTaskError;
use aws_sdk_ecs::types::{
    AssignPublicIp, AwsVpcConfiguration, CapacityProviderStrategyItem, ContainerOverride,
    DesiredStatus, KeyValuePair, NetworkConfiguration, Tag, Task, TaskField, TaskOverride,
    TaskStopCode,
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
    pub task_definition_arm64: String,
    pub task_definition_x86_64: String,
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
    pub fn task_definition(&self, architecture: &str) -> Result<&str, RunnerError> {
        match architecture {
            "arm64" => Ok(&self.task_definition_arm64),
            "x86_64" => Ok(&self.task_definition_x86_64),
            other => Err(RunnerError(format!(
                "architecture {other:?} has no Fargate task definition"
            ))),
        }
    }

    fn families(&self) -> [&str; 2] {
        [&self.task_definition_arm64, &self.task_definition_x86_64]
    }

    pub fn load(secrets: &Secrets) -> Result<Self, ConfigError> {
        let mut env = Reader::new();
        let cluster = env.get("SANCHORIS_ECS_CLUSTER");
        let task_definition_arm64 = env.get("SANCHORIS_ECS_TASK_DEFINITION_ARM64");
        let task_definition_x86_64 = env.get("SANCHORIS_ECS_TASK_DEFINITION_X86_64");
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
            task_definition_arm64,
            task_definition_x86_64,
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
                workspace_families: config.families().map(str::to_string).to_vec(),
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
        let family = self.config.task_definition(&spec.architecture)?;
        let spot = match self
            .run_task_on(spec, relay_host, family, CapacityProvider::Spot)
            .await?
        {
            Launch::Started(handle) => return Ok(handle),
            Launch::NoCapacity(reason) => reason,
        };
        eprintln!(
            "sanchoris worker: {} has no capacity for workspace {} ({spot}); starting it on {}",
            CapacityProvider::Spot.name(),
            spec.workspace_id,
            CapacityProvider::OnDemand.name()
        );
        match self
            .run_task_on(spec, relay_host, family, CapacityProvider::OnDemand)
            .await?
        {
            Launch::Started(handle) => Ok(handle),
            Launch::NoCapacity(reason) => Err(RunnerError(format!(
                "RunTask found no capacity on {} ({spot}) or on {} ({reason})",
                CapacityProvider::Spot.name(),
                CapacityProvider::OnDemand.name()
            ))),
        }
    }

    async fn run_task_on(
        &self,
        spec: &WorkspaceSpec,
        relay_host: &str,
        family: &str,
        provider: CapacityProvider,
    ) -> Result<Launch, RunnerError> {
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
        let strategy = CapacityProviderStrategyItem::builder()
            .capacity_provider(provider.name())
            .weight(1)
            .build()
            .map_err(runner_error)?;
        let result = self
            .client
            .run_task()
            .cluster(&self.config.cluster)
            .task_definition(family)
            .capacity_provider_strategy(strategy)
            .client_token(client_token(spec, provider))
            .started_by(holder(spec.run_id))
            .set_tags(Some(tags))
            .network_configuration(
                NetworkConfiguration::builder()
                    .awsvpc_configuration(network)
                    .build(),
            )
            .overrides(
                TaskOverride::builder()
                    .cpu(spec.cpu.to_string())
                    .memory(spec.memory_mib.to_string())
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
        let handle = |task: &str| {
            Launch::Started(WorkspaceHandle {
                platform_ref: task.to_string(),
                capacity_provider: Some(provider.name().to_string()),
            })
        };
        let output = match result {
            Ok(output) => output,
            Err(error) => {
                if let Some(RunTaskError::ConflictException(conflict)) = error.as_service_error()
                    && let Some(task) = conflict.resource_ids().first()
                {
                    return Ok(handle(task));
                }
                return Err(runner_error(error));
            }
        };
        if let Some(failure) = output.failures().first() {
            let reason = failure.reason().unwrap_or("without a reason");
            let detail = failure.detail().unwrap_or_default();
            if capacity_unavailable(reason) {
                return Ok(Launch::NoCapacity(
                    format!("{reason} {detail}").trim().to_string(),
                ));
            }
            return Err(RunnerError(format!("RunTask failed: {reason} {detail}")));
        }
        let task = output
            .tasks()
            .first()
            .and_then(|task| task.task_arn())
            .ok_or_else(|| RunnerError("RunTask returned no task".into()))?;
        Ok(handle(task))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityProvider {
    Spot,
    OnDemand,
}

impl CapacityProvider {
    pub fn name(self) -> &'static str {
        match self {
            CapacityProvider::Spot => "FARGATE_SPOT",
            CapacityProvider::OnDemand => "FARGATE",
        }
    }

    fn token_prefix(self) -> &'static str {
        match self {
            CapacityProvider::Spot => "spot",
            CapacityProvider::OnDemand => "ondemand",
        }
    }
}

enum Launch {
    Started(WorkspaceHandle),
    NoCapacity(String),
}

pub fn capacity_unavailable(reason: &str) -> bool {
    reason.starts_with("Capacity is unavailable") || reason.starts_with("RESOURCE:")
}

pub fn client_token(spec: &WorkspaceSpec, provider: CapacityProvider) -> String {
    format!("{}-{}", provider.token_prefix(), spec.workspace_id.simple())
}

pub fn interrupted_at(task: &Task) -> Option<DateTime<Utc>> {
    let interrupted = matches!(
        task.stop_code(),
        Some(TaskStopCode::SpotInterruption | TaskStopCode::TerminationNotice)
    );
    if !interrupted {
        return None;
    }
    task.stopping_at()
        .or(task.stopped_at())
        .and_then(|at| DateTime::from_timestamp(at.secs(), at.subsec_nanos()))
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
    workspace_families: Vec<String>,
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

    async fn workspace_tasks(&self) -> Result<Vec<Task>, RunnerError> {
        let mut tasks = Vec::new();
        for family in &self.workspace_families {
            tasks.extend(self.list(Some(family), None).await?);
        }
        Ok(tasks)
    }

    async fn running_workspaces(&self) -> Result<BTreeSet<String>, RunnerError> {
        Ok(self
            .workspace_tasks()
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
            .workspace_tasks()
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
                interrupted_at: interrupted_at(task),
            }));
        }
        Ok(Some(WorkspaceExit {
            reason: format!("the ECS task {platform_ref} is gone"),
            interrupted_at: None,
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
    use serde_json::{Value, json};

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
            architecture: "arm64".into(),
            cpu: 1024,
            memory_mib: 4096,
            repositories: vec![],
            network_allow: vec![],
        }
    }

    #[test]
    fn each_capacity_provider_has_its_own_client_token_for_the_workspace() {
        let spot = client_token(&spec(), CapacityProvider::Spot);
        let on_demand = client_token(&spec(), CapacityProvider::OnDemand);
        assert_eq!(spot, "spot-000000000000400080000000000000ff");
        assert_eq!(on_demand, "ondemand-000000000000400080000000000000ff");
        assert!(on_demand.len() <= 64);
        assert_eq!(client_token(&spec(), CapacityProvider::Spot), spot);
    }

    #[test]
    fn spot_capacity_failures_are_told_apart_from_other_failures() {
        assert!(capacity_unavailable(
            "Capacity is unavailable at this time. Please try again later or in a different availability zone"
        ));
        assert!(capacity_unavailable("RESOURCE:FARGATE"));
        assert!(!capacity_unavailable("MISSING"));
        assert!(!capacity_unavailable("ATTRIBUTE"));
    }

    #[test]
    fn only_platform_stops_are_interruptions() {
        let stopping = aws_sdk_ecs::primitives::DateTime::from_secs(1_800_000_000);
        let spot = Task::builder()
            .stop_code(TaskStopCode::SpotInterruption)
            .stopping_at(stopping)
            .stopped_at(aws_sdk_ecs::primitives::DateTime::from_secs(1_800_000_060))
            .build();
        assert_eq!(
            interrupted_at(&spot),
            DateTime::from_timestamp(1_800_000_000, 0)
        );
        let user = Task::builder()
            .stop_code(TaskStopCode::UserInitiated)
            .stopping_at(stopping)
            .build();
        assert_eq!(interrupted_at(&user), None);
        let exited = Task::builder()
            .stop_code(TaskStopCode::EssentialContainerExited)
            .build();
        assert_eq!(interrupted_at(&exited), None);
    }

    fn runner_config() -> EcsRunnerConfig {
        let signer =
            || sanchoris_core::tokens::Signer::new(b"0123456789abcdef0123456789abcdef").unwrap();
        EcsRunnerConfig {
            cluster: "c".into(),
            task_definition_arm64: "p-sanchoris-workspace-arm64".into(),
            task_definition_x86_64: "p-sanchoris-workspace-x86_64".into(),
            container: "workspace".into(),
            subnets: vec!["subnet-1".into()],
            security_groups: vec!["sg-1".into()],
            assign_public_ip: AssignPublicIp::Enabled,
            control_url: "wss://channel.example".into(),
            git_url: "https://github.com".into(),
            relay_service: "p-sanchoris-relay".into(),
            relay_port: 3128,
            run_tokens: RunTokenSigner::new(signer()),
            relay_tokens: RelayTokenSigner::new(signer()),
        }
    }

    #[test]
    fn the_session_architecture_picks_the_task_definition() {
        let config = runner_config();
        assert_eq!(
            config.task_definition("arm64").unwrap(),
            "p-sanchoris-workspace-arm64"
        );
        assert_eq!(
            config.task_definition("x86_64").unwrap(),
            "p-sanchoris-workspace-x86_64"
        );
        assert!(config.task_definition("riscv64").is_err());
    }

    fn fake_ecs(spot: Value) -> (EcsRunner, Arc<std::sync::Mutex<Vec<Value>>>) {
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = requests.clone();
        let http = aws_smithy_http_client::test_util::infallible_client_fn(move |request| {
            let body: Value =
                serde_json::from_slice(request.body().bytes().expect("a JSON body")).unwrap();
            let provider = body["capacityProviderStrategy"][0]["capacityProvider"].clone();
            seen.lock().unwrap().push(body);
            let response = if provider == "FARGATE_SPOT" {
                spot.clone()
            } else {
                json!({
                    "tasks": [{ "taskArn": "arn:aws:ecs:ap-northeast-1:1:task/c/on-demand" }],
                    "failures": []
                })
            };
            http::Response::builder()
                .status(200)
                .body(aws_smithy_types::body::SdkBody::from(response.to_string()))
                .unwrap()
        });
        let credentials = aws_sdk_ecs::config::Credentials::new("a", "b", None, None, "test");
        let sdk = aws_config::SdkConfig::builder()
            .http_client(http)
            .region(aws_sdk_ecs::config::Region::new("ap-northeast-1"))
            .credentials_provider(aws_sdk_ecs::config::SharedCredentialsProvider::new(
                credentials,
            ))
            .behavior_version(aws_config::BehaviorVersion::latest())
            .build();
        let client = aws_sdk_ecs::Client::new(&sdk);
        let config = runner_config();
        let runner = EcsRunner {
            relay: RelayLease {
                ecs: client.clone(),
                dynamodb: aws_sdk_dynamodb::Client::new(&sdk),
                table: "t".into(),
                cluster: config.cluster.clone(),
                service: config.relay_service.clone(),
                workspace_families: config.families().map(str::to_string).to_vec(),
                clock: Arc::new(sanchoris_core::clock::SystemClock),
            },
            client,
            config,
        };
        (runner, requests)
    }

    #[tokio::test]
    async fn a_spot_capacity_failure_starts_the_workspace_on_fargate_and_records_it() {
        let (runner, requests) = fake_ecs(json!({
            "tasks": [],
            "failures": [{
                "arn": "arn:aws:ecs:ap-northeast-1:1:task-definition/p-sanchoris-workspace-arm64",
                "reason": "Capacity is unavailable at this time. Please try again later or in a different availability zone"
            }]
        }));
        let handle = runner.run_task(&spec(), "10.1.0.5").await.unwrap();
        assert_eq!(
            handle,
            WorkspaceHandle {
                platform_ref: "arn:aws:ecs:ap-northeast-1:1:task/c/on-demand".into(),
                capacity_provider: Some("FARGATE".into()),
            }
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0]["capacityProviderStrategy"],
            json!([{ "capacityProvider": "FARGATE_SPOT", "weight": 1 }])
        );
        assert_eq!(
            requests[1]["capacityProviderStrategy"],
            json!([{ "capacityProvider": "FARGATE", "weight": 1 }])
        );
        assert_ne!(requests[0]["clientToken"], requests[1]["clientToken"]);
        for request in requests.iter() {
            assert_eq!(request["taskDefinition"], "p-sanchoris-workspace-arm64");
            assert_eq!(request["overrides"]["cpu"], "1024");
            assert_eq!(request["overrides"]["memory"], "4096");
            assert!(request.get("launchType").is_none());
        }
    }

    #[tokio::test]
    async fn spot_capacity_starts_the_workspace_on_spot_and_other_failures_fail_fast() {
        let (runner, requests) = fake_ecs(json!({
            "tasks": [{ "taskArn": "arn:aws:ecs:ap-northeast-1:1:task/c/spot" }],
            "failures": []
        }));
        let mut x86 = spec();
        x86.architecture = "x86_64".into();
        let handle = runner.run_task(&x86, "10.1.0.5").await.unwrap();
        assert_eq!(handle.capacity_provider.as_deref(), Some("FARGATE_SPOT"));
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert_eq!(
            requests.lock().unwrap()[0]["taskDefinition"],
            "p-sanchoris-workspace-x86_64"
        );

        let (runner, requests) = fake_ecs(json!({
            "tasks": [],
            "failures": [{ "arn": "arn:aws:ecs:ap-northeast-1:1:task-definition/x", "reason": "MISSING" }]
        }));
        let error = runner.run_task(&spec(), "10.1.0.5").await.unwrap_err();
        assert!(error.0.contains("MISSING"), "{}", error.0);
        assert_eq!(requests.lock().unwrap().len(), 1);
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
