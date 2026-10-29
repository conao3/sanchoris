use std::process::ExitCode;
use std::sync::Arc;

use sanchoris_api::config::{GitHubAppConfig, Secrets};
use sanchoris_api::lambda;
use sanchoris_api::ports::ApiWorkspaces;
use sanchoris_core::clock::SystemClock;
use sanchoris_worker::backfill;
use sanchoris_worker::resources::GitHubResources;
use sanchoris_worker::{Deps, Settings, Worker};

const SECRETS: [&str; 2] = ["GITHUB_APP_ID", "GITHUB_APP_PRIVATE_KEY"];

async fn run() -> Result<(), String> {
    let sdk = aws_config::load_from_env().await;
    let secrets = Secrets::from_ssm(&sdk, &SECRETS)
        .await
        .map_err(|error| error.to_string())?;
    let github = Arc::new(
        GitHubAppConfig::load(&secrets)
            .and_then(|config| config.client())
            .map_err(|error| error.to_string())?,
    );
    let store = lambda::store(&sdk)?;
    let worker = Worker::new(
        store,
        Deps {
            clock: Arc::new(SystemClock),
            github: github.clone(),
            reader: github.clone(),
            config_source: github.clone(),
            resources: Arc::new(GitHubResources::new(github)),
            runner: Arc::new(ApiWorkspaces),
            pusher: Arc::new(ApiWorkspaces),
        },
        Settings::default(),
    );
    let report = backfill::stable_keys(&worker)
        .await
        .map_err(|error| error.to_string())?;
    for line in report.lines {
        println!("{line}");
    }
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sanchoris-backfill-stable-keys: {error}");
            ExitCode::FAILURE
        }
    }
}
