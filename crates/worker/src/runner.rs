use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    pub workspaces_dir: PathBuf,
    pub git_url: String,
    pub run_tokens: RunTokenSigner,
    pub token_ttl: Duration,
}

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set"))
}

impl LocalRunnerConfig {
    pub fn from_env() -> Self {
        let command = required("SANCHORIS_WORKSPACE_COMMAND");
        let mut parts = command.split_whitespace().map(str::to_string);
        let program = parts
            .next()
            .expect("SANCHORIS_WORKSPACE_COMMAND must name a program");
        Self {
            program: PathBuf::from(program),
            args: parts.collect(),
            control_url: required("SANCHORIS_CONTROL_URL"),
            socket_dir: PathBuf::from(required("SANCHORIS_SOCKET_DIR")),
            workspaces_dir: PathBuf::from(required("SANCHORIS_WORKSPACES_DIR")),
            git_url: required("SANCHORIS_GIT_URL")
                .trim_end_matches('/')
                .to_string(),
            run_tokens: RunTokenSigner::from_env(),
            token_ttl: Duration::hours(12),
        }
    }
}

struct LocalWorkspace {
    child: Child,
    dir: PathBuf,
}

pub struct LocalProcessRunner {
    config: LocalRunnerConfig,
    clock: Arc<dyn Clock>,
    workspaces: Mutex<HashMap<String, LocalWorkspace>>,
}

async fn git(args: &[&str]) -> Result<std::process::Output, RunnerError> {
    Command::new("git")
        .args(args)
        .output()
        .await
        .map_err(|error| RunnerError(format!("git {}: {error}", args.join(" "))))
}

async fn git_ok(args: &[&str]) -> Result<(), RunnerError> {
    let output = git(args).await?;
    if output.status.success() {
        return Ok(());
    }
    Err(RunnerError(format!(
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

fn repository_dir(root: &Path, repo: &str) -> Result<PathBuf, RunnerError> {
    let name = repo
        .split_once('/')
        .map(|(_, name)| name)
        .filter(|name| !name.is_empty() && !name.contains('/') && *name != "." && *name != "..")
        .ok_or_else(|| RunnerError(format!("{repo:?} is not owner/name")))?;
    Ok(root.join(name))
}

impl LocalProcessRunner {
    pub fn new(config: LocalRunnerConfig, clock: Arc<dyn Clock>) -> Self {
        Self {
            config,
            clock,
            workspaces: Mutex::new(HashMap::new()),
        }
    }

    fn socket_path(&self, run_id: Uuid) -> PathBuf {
        self.config.socket_dir.join(format!("{run_id}.sock"))
    }

    pub fn environment(
        &self,
        tenant_id: Uuid,
        run_id: Uuid,
        workspace_dir: &Path,
    ) -> Vec<(&'static str, String)> {
        let token = self.config.run_tokens.sign(
            tenant_id,
            run_id,
            self.clock.now() + self.config.token_ttl,
        );
        vec![
            ("SANCHORIS_RUN_ID", run_id.to_string()),
            ("SANCHORIS_RUN_TOKEN", token),
            ("SANCHORIS_CONTROL_URL", self.config.control_url.clone()),
            (
                "SANCHORIS_SOCKET_PATH",
                self.socket_path(run_id).display().to_string(),
            ),
            (
                "SANCHORIS_WORKSPACE_DIR",
                workspace_dir.display().to_string(),
            ),
        ]
    }

    async fn checkout(&self, root: &Path, spec: &WorkspaceSpec) -> Result<(), RunnerError> {
        for repo in &spec.repositories {
            let dir = repository_dir(root, repo)?.display().to_string();
            let url = format!("{}/{repo}.git", self.config.git_url);
            git_ok(&["clone", "--quiet", &url, &dir]).await?;
            let remote = format!("refs/remotes/origin/{}", spec.branch);
            let exists = git(&["-C", &dir, "rev-parse", "--verify", "--quiet", &remote])
                .await?
                .status
                .success();
            if exists {
                git_ok(&[
                    "-C",
                    &dir,
                    "checkout",
                    "--quiet",
                    "-B",
                    &spec.branch,
                    &remote,
                ])
                .await?;
            } else {
                git_ok(&["-C", &dir, "checkout", "--quiet", "-b", &spec.branch]).await?;
            }
        }
        Ok(())
    }

    async fn prepare(&self, dir: &Path, spec: &WorkspaceSpec) -> Result<(), RunnerError> {
        tokio::fs::create_dir_all(&self.config.socket_dir)
            .await
            .map_err(|error| {
                RunnerError(format!(
                    "create {}: {error}",
                    self.config.socket_dir.display()
                ))
            })?;
        self.checkout(dir, spec).await?;
        let socket = self.socket_path(spec.run_id);
        match tokio::fs::remove_file(&socket).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(RunnerError(format!("remove {}: {error}", socket.display()))),
        }
    }
}

#[async_trait]
impl RunnerPort for LocalProcessRunner {
    async fn start_workspace(&self, spec: &WorkspaceSpec) -> Result<WorkspaceHandle, RunnerError> {
        let dir = self.config.workspaces_dir.join(Uuid::new_v4().to_string());
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|error| RunnerError(format!("create {}: {error}", dir.display())))?;
        if let Err(error) = self.prepare(&dir, spec).await {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Err(error);
        }
        let mut command = Command::new(&self.config.program);
        command
            .args(&self.config.args)
            .current_dir(&dir)
            .kill_on_drop(true);
        for (name, value) in self.environment(spec.tenant_id, spec.run_id, &dir) {
            command.env(name, value);
        }
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = tokio::fs::remove_dir_all(&dir).await;
                return Err(RunnerError(format!(
                    "spawn {:?}: {error}",
                    self.config.program
                )));
            }
        };
        let platform_ref = child
            .id()
            .map(|pid| format!("pid:{pid}"))
            .ok_or_else(|| RunnerError("the workspace process exited at once".into()))?;
        self.workspaces
            .lock()
            .await
            .insert(platform_ref.clone(), LocalWorkspace { child, dir });
        Ok(WorkspaceHandle {
            platform: "local",
            platform_ref,
        })
    }

    async fn stop_workspace(&self, _run_id: Uuid, platform_ref: &str) -> Result<(), RunnerError> {
        let workspace = self.workspaces.lock().await.remove(platform_ref);
        if let Some(mut workspace) = workspace {
            workspace
                .child
                .kill()
                .await
                .map_err(|error| RunnerError(format!("kill {platform_ref}: {error}")))?;
            tokio::fs::remove_dir_all(&workspace.dir)
                .await
                .map_err(|error| {
                    RunnerError(format!("remove {}: {error}", workspace.dir.display()))
                })?;
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

    struct Fixture {
        runner: LocalProcessRunner,
        tokens: RunTokenSigner,
        root: tempfile::TempDir,
    }

    fn fixture(program: &str, args: &[&str]) -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let tokens = RunTokenSigner::new(Signer::new(&[3u8; 32]).unwrap());
        let config = LocalRunnerConfig {
            program: PathBuf::from(program),
            args: args.iter().map(|a| a.to_string()).collect(),
            control_url: "ws://127.0.0.1:1/runs/channel".into(),
            socket_dir: root.path().join("sockets"),
            workspaces_dir: root.path().join("workspaces"),
            git_url: format!("file://{}", root.path().join("git").display()),
            run_tokens: tokens.clone(),
            token_ttl: Duration::hours(1),
        };
        let clock = Arc::new(FakeClock::new(
            chrono::Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap(),
        ));
        Fixture {
            runner: LocalProcessRunner::new(config, clock),
            tokens,
            root,
        }
    }

    fn spec(repositories: Vec<String>) -> WorkspaceSpec {
        WorkspaceSpec {
            run_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            branch: "sanchoris/x".into(),
            compute: "standard".into(),
            repositories,
        }
    }

    fn run(args: &[&str], dir: &Path) -> String {
        let output = std::process::Command::new(args[0])
            .args(&args[1..])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn bare_repo(root: &Path, repo: &str, branches: &[&str]) {
        let bare = root.join("git").join(format!("{repo}.git"));
        std::fs::create_dir_all(&bare).unwrap();
        run(&["git", "init", "--quiet", "--bare", "-b", "main"], &bare);
        let seed = tempfile::tempdir().unwrap();
        run(&["git", "init", "--quiet", "-b", "main"], seed.path());
        std::fs::write(seed.path().join("README.md"), "hello\n").unwrap();
        run(&["git", "add", "."], seed.path());
        run(
            &[
                "git",
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--quiet",
                "-m",
                "init",
            ],
            seed.path(),
        );
        let url = bare.display().to_string();
        run(&["git", "push", "--quiet", &url, "main"], seed.path());
        for branch in branches {
            let refspec = format!("main:refs/heads/{branch}");
            run(&["git", "push", "--quiet", &url, &refspec], seed.path());
        }
    }

    fn entries(dir: &Path) -> usize {
        std::fs::read_dir(dir).unwrap().count()
    }

    #[test]
    fn environment_carries_a_verifiable_run_token() {
        let fixture = fixture("true", &[]);
        let run_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();
        let env = fixture
            .runner
            .environment(tenant_id, run_id, Path::new("/w/1"));
        let get = |name: &str| env.iter().find(|(k, _)| *k == name).unwrap().1.clone();
        assert_eq!(get("SANCHORIS_RUN_ID"), run_id.to_string());
        assert_eq!(
            get("SANCHORIS_CONTROL_URL"),
            "ws://127.0.0.1:1/runs/channel"
        );
        assert_eq!(get("SANCHORIS_WORKSPACE_DIR"), "/w/1");
        assert_eq!(
            PathBuf::from(get("SANCHORIS_SOCKET_PATH")),
            fixture
                .root
                .path()
                .join("sockets")
                .join(format!("{run_id}.sock"))
        );
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 10, 27, 9, 30, 0)
            .unwrap();
        assert_eq!(
            fixture
                .tokens
                .verify(&get("SANCHORIS_RUN_TOKEN"), now)
                .unwrap(),
            sanchoris_core::tokens::RunClaims { tenant_id, run_id }
        );
    }

    #[tokio::test]
    async fn starts_and_stops_a_local_process() {
        let fixture = fixture("sleep", &["30"]);
        let spec = spec(vec![]);
        let handle = fixture.runner.start_workspace(&spec).await.unwrap();
        assert_eq!(handle.platform, "local");
        assert!(handle.platform_ref.starts_with("pid:"));
        fixture
            .runner
            .stop_workspace(spec.run_id, &handle.platform_ref)
            .await
            .unwrap();
        assert!(fixture.runner.workspaces.lock().await.is_empty());
        assert_eq!(entries(&fixture.root.path().join("workspaces")), 0);
    }

    #[tokio::test]
    async fn clones_the_repositories_on_the_task_branch() {
        let fixture = fixture("sleep", &["30"]);
        bare_repo(fixture.root.path(), "acme/shop-web", &["sanchoris/x"]);
        bare_repo(fixture.root.path(), "acme/shop-api", &[]);
        let spec = spec(vec!["acme/shop-web".into(), "acme/shop-api".into()]);
        let handle = fixture.runner.start_workspace(&spec).await.unwrap();
        let dir = fixture.runner.workspaces.lock().await[&handle.platform_ref]
            .dir
            .clone();
        for name in ["shop-web", "shop-api"] {
            let branch = run(
                &["git", "rev-parse", "--abbrev-ref", "HEAD"],
                &dir.join(name),
            );
            assert_eq!(branch, "sanchoris/x");
            assert!(dir.join(name).join("README.md").exists());
        }
        fixture
            .runner
            .stop_workspace(spec.run_id, &handle.platform_ref)
            .await
            .unwrap();
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn a_missing_repository_fails_without_leaving_a_workspace() {
        let fixture = fixture("sleep", &["30"]);
        let error = fixture
            .runner
            .start_workspace(&spec(vec!["acme/missing".into()]))
            .await
            .unwrap_err();
        assert!(error.0.contains("clone"), "{error}");
        assert_eq!(entries(&fixture.root.path().join("workspaces")), 0);
    }
}
