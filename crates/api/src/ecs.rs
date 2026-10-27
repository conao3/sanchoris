use std::sync::Arc;

use async_trait::async_trait;
use aws_sdk_ecs::error::DisplayErrorContext;
use aws_sdk_ecs::types::{
    AssignPublicIp, AwsVpcConfiguration, ContainerOverride, KeyValuePair, LaunchType,
    NetworkConfiguration, TaskOverride,
};
use chrono::Duration;
use sanchoris_core::clock::Clock;
use sanchoris_core::tokens::{RUN_TOKEN_SIGNING_KEY_ENV, RunTokenSigner, Signer};
use sanchoris_worker::ports::{RunnerError, RunnerPort, WorkspaceHandle, WorkspaceSpec};
use uuid::Uuid;

use crate::config::{ConfigError, Reader};

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
    pub run_tokens: RunTokenSigner,
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
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut env = Reader::new();
        let cluster = env.get("SANCHORIS_ECS_CLUSTER");
        let task_definition = env.get("SANCHORIS_ECS_TASK_DEFINITION");
        let container = env.get("SANCHORIS_ECS_CONTAINER");
        let subnets = env.get("SANCHORIS_ECS_SUBNETS");
        let security_groups = env.get("SANCHORIS_ECS_SECURITY_GROUPS");
        let control_url = env.get("SANCHORIS_CONTROL_URL");
        let git_url = env.get("SANCHORIS_GIT_URL");
        let run_key = env.get(RUN_TOKEN_SIGNING_KEY_ENV);
        env.finish()?;
        let signer = Signer::new(run_key.as_bytes()).map_err(|error| ConfigError::Invalid {
            name: RUN_TOKEN_SIGNING_KEY_ENV,
            message: error.to_string(),
        })?;
        Ok(Self {
            cluster,
            task_definition,
            container,
            subnets: list(&subnets),
            security_groups: list(&security_groups),
            control_url,
            git_url: git_url.trim_end_matches('/').to_string(),
            run_tokens: RunTokenSigner::new(signer),
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
        let token = self.config.run_tokens.sign(
            spec.tenant_id,
            spec.run_id,
            self.clock.now() + Duration::hours(TOKEN_TTL_HOURS),
        );
        vec![
            ("SANCHORIS_RUN_ID", spec.run_id.to_string()),
            ("SANCHORIS_RUN_TOKEN", token),
            ("SANCHORIS_CONTROL_URL", self.config.control_url.clone()),
            ("SANCHORIS_GIT_URL", self.config.git_url.clone()),
            ("SANCHORIS_BRANCH", spec.branch.clone()),
            ("SANCHORIS_REPOSITORIES", spec.repositories.join(",")),
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
