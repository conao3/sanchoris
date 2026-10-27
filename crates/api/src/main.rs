use std::process::ExitCode;

use sanchoris_api::config::{ServeConfig, check_worker_env};
use tokio::sync::watch;

const USAGE: &str = "Usage: sanchoris <serve|worker|all>

  serve   Run the HTTP API (GraphQL, webhooks, GitHub OAuth, Run channel, records)
  worker  Run the background loops
  all     Run both in one process (local development)";

async fn signals(sender: watch::Sender<bool>) {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install the SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
    let _ = sender.send(true);
}

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("sanchoris: {message}");
    ExitCode::FAILURE
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [command] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let serve_config = match command.as_str() {
        "serve" | "all" => match ServeConfig::from_env() {
            Ok(config) => Some(config),
            Err(error) => return fail(error),
        },
        "worker" => None,
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    if matches!(command.as_str(), "worker" | "all")
        && let Err(error) = check_worker_env()
    {
        return fail(error);
    }
    let (sender, receiver) = watch::channel(false);
    tokio::spawn(signals(sender));
    let result = match (command.as_str(), serve_config) {
        ("serve", Some(config)) => sanchoris_api::serve(config, receiver).await,
        ("worker", None) => {
            sanchoris_api::worker(receiver).await;
            Ok(())
        }
        ("all", Some(config)) => {
            let worker = tokio::spawn(sanchoris_api::worker(receiver.clone()));
            let served = sanchoris_api::serve(config, receiver).await;
            match worker.await {
                Ok(()) => served,
                Err(error) => Err(format!("the worker stopped: {error}")),
            }
        }
        _ => unreachable!("the command was matched above"),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(error),
    }
}
