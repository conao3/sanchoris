use std::sync::Arc;

use async_trait::async_trait;
use aws_sdk_ecs::error::DisplayErrorContext;
use aws_sdk_ecs::types::{
    AssignPublicIp, AwsVpcConfiguration, ContainerOverride, KeyValuePair, LaunchType,
    NetworkConfiguration, TaskOverride,
};
use chrono::Duration;
use sanchoris_core::clock::Clock;
use sanchoris_core::tokens::{RELAY_TOKEN_SIGNING_KEY_ENV, RelayTokenSigner, RunTokenSigner};
use sanchoris_worker::ports::{RunnerError, RunnerPort, WorkspaceHandle, WorkspaceSpec};
use uuid::Uuid;

use crate::config::{ConfigError, Reader, Secrets, run_tokens, signer};

pub const PLATFORM: &str = "ecs";
const TOKEN_TTL_HOURS: i64 = 12;

#[derive(Debug, Clone)]
pub struct EcsRunnerConfig {
    pub cluster: String,
    pub task_definition: String,
    pub container: String,
    pub subnets: Vec<String>,
    pub security_groups: Vec<String>,
    pub control_url: String,
    pub git_url: String,
    pub relay_address: String,
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
        let control_url = env.get("SANCHORIS_CONTROL_URL");
        let git_url = env.get("SANCHORIS_GIT_URL");
        let relay_url = env.get("SANCHORIS_RELAY_URL");
        let relay_key = env.secret(secrets, RELAY_TOKEN_SIGNING_KEY_ENV);
        env.finish()?;
        let (scheme, relay_host) = relay_url.split_once("://").ok_or(ConfigError::Invalid {
            name: "SANCHORIS_RELAY_URL",
            message: format!("{relay_url:?} is not http://<host>:<port>"),
        })?;
        if scheme != "http" || relay_host.is_empty() || relay_host.contains(['/', '@']) {
            return Err(ConfigError::Invalid {
                name: "SANCHORIS_RELAY_URL",
                message: format!("{relay_url:?} is not http://<host>:<port>"),
            });
        }
        Ok(Self {
            cluster,
            task_definition,
            container,
            subnets: list(&subnets),
            security_groups: list(&security_groups),
            control_url,
            git_url: git_url.trim_end_matches('/').to_string(),
            relay_address: relay_host.to_string(),
            run_tokens: run_tokens(secrets)?,
            relay_tokens: RelayTokenSigner::new(signer(RELAY_TOKEN_SIGNING_KEY_ENV, &relay_key)?),
        })
    }
}

pub struct EcsRunner {
    client: aws_sdk_ecs::Client,
    config: EcsRunnerConfig,
    clock: Arc<dyn Clock>,
}

impl EcsRunner {
    pub fn new(
        sdk: &aws_config::SdkConfig,
        config: EcsRunnerConfig,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            client: aws_sdk_ecs::Client::new(sdk),
            config,
            clock,
        }
    }

    pub fn environment(&self, spec: &WorkspaceSpec) -> Vec<(&'static str, String)> {
        let expires_at = self.clock.now() + Duration::hours(TOKEN_TTL_HOURS);
        let token = self
            .config
            .run_tokens
            .sign(spec.tenant_id, spec.run_id, expires_at);
        let relay_token =
            self.config
                .relay_tokens
                .sign(spec.run_id, &spec.network_allow, expires_at);
        let proxy = format!("http://run:{relay_token}@{}", self.config.relay_address);
        vec![
            ("SANCHORIS_RUN_ID", spec.run_id.to_string()),
            ("SANCHORIS_RUN_TOKEN", token),
            ("SANCHORIS_CONTROL_URL", self.config.control_url.clone()),
            ("SANCHORIS_GIT_URL", self.config.git_url.clone()),
            ("SANCHORIS_BRANCH", spec.branch.clone()),
            ("SANCHORIS_REPOSITORIES", spec.repositories.join(",")),
            ("SANCHORIS_CREDENTIAL_SOURCE", "channel".to_string()),
            ("HTTPS_PROXY", proxy.clone()),
            ("HTTP_PROXY", proxy),
        ]
    }
}

fn runner_error(error: impl std::error::Error) -> RunnerError {
    RunnerError(DisplayErrorContext(error).to_string())
}

#[async_trait]
impl RunnerPort for EcsRunner {
    async fn start_workspace(&self, spec: &WorkspaceSpec) -> Result<WorkspaceHandle, RunnerError> {
        if spec.compute != "standard" {
            return Err(RunnerError(format!(
                "compute {:?} has no Fargate task definition",
                spec.compute
            )));
        }
        let environment = self
            .environment(spec)
            .into_iter()
            .map(|(name, value)| KeyValuePair::builder().name(name).value(value).build())
            .collect::<Vec<_>>();
        let network = AwsVpcConfiguration::builder()
            .set_subnets(Some(self.config.subnets.clone()))
            .set_security_groups(Some(self.config.security_groups.clone()))
            .assign_public_ip(AssignPublicIp::Disabled)
            .build()
            .map_err(runner_error)?;
        let output = self
            .client
            .run_task()
            .cluster(&self.config.cluster)
            .task_definition(&self.config.task_definition)
            .launch_type(LaunchType::Fargate)
            .started_by(spec.run_id.simple().to_string())
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
            .await
            .map_err(runner_error)?;
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
            platform: PLATFORM,
            platform_ref: task.to_string(),
        })
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
        Ok(())
    }
}
