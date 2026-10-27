use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Duration;
use sanchoris_core::tokens::RunTokenSigner;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::clock::Clock;
use crate::ports::{RunnerError, RunnerPort, WorkspaceHandle, WorkspaceSpec};

#[derive(Debug, Clone)]
pub struct LocalRunnerConfig {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub control_url: String,
    pub socket_dir: PathBuf,
    pub run_tokens: RunTokenSigner,
    pub token_ttl: Duration,
}

impl LocalRunnerConfig {
    pub fn from_env() -> Self {
        let command = std::env::var("SANCHORIS_WORKSPACE_COMMAND")
            .expect("SANCHORIS_WORKSPACE_COMMAND must be set");
        let mut parts = command.split_whitespace().map(str::to_string);
        let program = parts
            .next()
            .expect("SANCHORIS_WORKSPACE_COMMAND must name a program");
        Self {
            program: PathBuf::from(program),
            args: parts.collect(),
            control_url: std::env::var("SANCHORIS_CONTROL_URL")
                .expect("SANCHORIS_CONTROL_URL must be set"),
            socket_dir: PathBuf::from(
                std::env::var("SANCHORIS_SOCKET_DIR").expect("SANCHORIS_SOCKET_DIR must be set"),
            ),
            run_tokens: RunTokenSigner::from_env(),
            token_ttl: Duration::hours(12),
        }
    }
}

pub struct LocalProcessRunner {
    config: LocalRunnerConfig,
    clock: Arc<dyn Clock>,
    children: Mutex<HashMap<String, Child>>,
}

impl LocalProcessRunner {
    pub fn new(config: LocalRunnerConfig, clock: Arc<dyn Clock>) -> Self {
        Self {
            config,
            clock,
            children: Mutex::new(HashMap::new()),
        }
    }

    pub fn environment(&self, run_id: Uuid) -> Vec<(&'static str, String)> {
        let token = self
            .config
            .run_tokens
            .sign(run_id, self.clock.now() + self.config.token_ttl);
        vec![
            ("SANCHORIS_RUN_ID", run_id.to_string()),
            ("SANCHORIS_RUN_TOKEN", token),
            ("SANCHORIS_CONTROL_URL", self.config.control_url.clone()),
            (
                "SANCHORIS_SOCKET_PATH",
                self.config
                    .socket_dir
                    .join(format!("{run_id}.sock"))
                    .display()
                    .to_string(),
            ),
        ]
    }
}

#[async_trait]
impl RunnerPort for LocalProcessRunner {
    async fn start_workspace(&self, spec: &WorkspaceSpec) -> Result<WorkspaceHandle, RunnerError> {
        let mut command = Command::new(&self.config.program);
        command.args(&self.config.args).kill_on_drop(true);
        for (name, value) in self.environment(spec.run_id) {
            command.env(name, value);
        }
        let child = command
            .spawn()
            .map_err(|error| RunnerError(format!("spawn {:?}: {error}", self.config.program)))?;
        let platform_ref = child
            .id()
            .map(|pid| format!("pid:{pid}"))
            .ok_or_else(|| RunnerError("the workspace process exited at once".into()))?;
        self.children
            .lock()
            .await
            .insert(platform_ref.clone(), child);
        Ok(WorkspaceHandle {
            platform: "local",
            platform_ref,
        })
    }

    async fn stop_workspace(&self, _run_id: Uuid, platform_ref: &str) -> Result<(), RunnerError> {
        let child = self.children.lock().await.remove(platform_ref);
        if let Some(mut child) = child {
            child
                .kill()
                .await
                .map_err(|error| RunnerError(format!("kill {platform_ref}: {error}")))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeClock;
    use chrono::TimeZone;
    use sanchoris_core::tokens::Signer;

    fn runner(program: &str, args: &[&str]) -> (LocalProcessRunner, RunTokenSigner) {
        let tokens = RunTokenSigner::new(Signer::new(&[3u8; 32]).unwrap());
        let config = LocalRunnerConfig {
            program: PathBuf::from(program),
            args: args.iter().map(|a| a.to_string()).collect(),
            control_url: "ws://127.0.0.1:1/runs".into(),
            socket_dir: PathBuf::from("/tmp/sanchoris"),
            run_tokens: tokens.clone(),
            token_ttl: Duration::hours(1),
        };
        let clock = Arc::new(FakeClock::new(
            chrono::Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap(),
        ));
        (LocalProcessRunner::new(config, clock), tokens)
    }

    #[test]
    fn environment_carries_a_verifiable_run_token() {
        let (runner, tokens) = runner("true", &[]);
        let run_id = Uuid::new_v4();
        let env = runner.environment(run_id);
        let get = |name: &str| env.iter().find(|(k, _)| *k == name).unwrap().1.clone();
        assert_eq!(get("SANCHORIS_RUN_ID"), run_id.to_string());
        assert_eq!(get("SANCHORIS_CONTROL_URL"), "ws://127.0.0.1:1/runs");
        assert_eq!(
            get("SANCHORIS_SOCKET_PATH"),
            format!("/tmp/sanchoris/{run_id}.sock")
        );
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 10, 27, 9, 30, 0)
            .unwrap();
        assert_eq!(
            tokens.verify(&get("SANCHORIS_RUN_TOKEN"), now).unwrap(),
            run_id
        );
    }

    #[tokio::test]
    async fn starts_and_stops_a_local_process() {
        let (runner, _) = runner("sleep", &["30"]);
        let spec = WorkspaceSpec {
            run_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            branch: "sanchoris/x".into(),
            compute: "standard".into(),
            repositories: vec![],
        };
        let handle = runner.start_workspace(&spec).await.unwrap();
        assert_eq!(handle.platform, "local");
        assert!(handle.platform_ref.starts_with("pid:"));
        runner
            .stop_workspace(spec.run_id, &handle.platform_ref)
            .await
            .unwrap();
        assert!(runner.children.lock().await.is_empty());
    }
}
