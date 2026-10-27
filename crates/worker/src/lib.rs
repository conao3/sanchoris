pub mod clock;
pub mod config_sync;
pub mod dispatcher;
pub mod engine_worker;
pub mod error;
pub mod github;
pub mod ports;
pub mod resources;
pub mod runner;
pub mod schedule;
pub mod supervisor;
pub mod usage;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use chrono::Duration;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::{Result, WorkerError};
use crate::ports::{
    ConfigSource, GitHubClient, GitHubReader, Installation, ResourceInvoker, RunnerPort,
};
use sanchoris_core::db::TenantTx;

pub struct Deps {
    pub clock: Arc<dyn Clock>,
    pub github: Arc<dyn GitHubClient>,
    pub reader: Arc<dyn GitHubReader>,
    pub config_source: Arc<dyn ConfigSource>,
    pub resources: Arc<dyn ResourceInvoker>,
    pub runner: Arc<dyn RunnerPort>,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub loop_interval: StdDuration,
    pub check_poll_interval: Duration,
    pub config_poll_interval: Duration,
    pub usage_snapshot_interval: Duration,
    pub batch: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            loop_interval: StdDuration::from_secs(1),
            check_poll_interval: Duration::minutes(5),
            config_poll_interval: Duration::minutes(5),
            usage_snapshot_interval: Duration::seconds(60),
            batch: 50,
        }
    }
}

#[derive(Debug, Default)]
pub struct Faults {
    armed: Mutex<HashSet<&'static str>>,
}

impl Faults {
    pub fn arm(&self, point: &'static str) {
        self.armed.lock().expect("faults lock").insert(point);
    }

    pub fn clear(&self) {
        self.armed.lock().expect("faults lock").clear();
    }

    pub fn check(&self, point: &'static str) -> Result<()> {
        if self.armed.lock().expect("faults lock").remove(point) {
            return Err(WorkerError::Injected(point));
        }
        Ok(())
    }
}

pub struct Worker {
    pub pool: PgPool,
    pub deps: Deps,
    pub settings: Settings,
    pub faults: Faults,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loop {
    ConfigSyncer,
    Scheduler,
    TriggerDispatcher,
    EngineTimers,
    ResourceInvocations,
    CheckPoller,
    RunEvents,
    TaskResumes,
    TaskLimits,
    Workspaces,
    PendingDispatch,
    LimitsEnforcer,
}

impl Loop {
    pub const ALL: [Loop; 12] = [
        Loop::ConfigSyncer,
        Loop::Scheduler,
        Loop::TriggerDispatcher,
        Loop::EngineTimers,
        Loop::ResourceInvocations,
        Loop::CheckPoller,
        Loop::RunEvents,
        Loop::TaskResumes,
        Loop::TaskLimits,
        Loop::Workspaces,
        Loop::PendingDispatch,
        Loop::LimitsEnforcer,
    ];
}

impl Worker {
    pub fn new(pool: PgPool, deps: Deps, settings: Settings) -> Self {
        Self {
            pool,
            deps,
            settings,
            faults: Faults::default(),
        }
    }

    pub fn now(&self) -> chrono::DateTime<chrono::Utc> {
        self.deps.clock.now()
    }

    pub async fn tick(&self, which: Loop) -> Result<usize> {
        match which {
            Loop::ConfigSyncer => config_sync::tick(self).await,
            Loop::Scheduler => dispatcher::tick_schedules(self).await,
            Loop::TriggerDispatcher => dispatcher::tick(self).await,
            Loop::EngineTimers => engine_worker::tick_timers(self).await,
            Loop::ResourceInvocations => engine_worker::tick_invocations(self).await,
            Loop::CheckPoller => engine_worker::tick_checks(self).await,
            Loop::RunEvents => supervisor::tick_run_events(self).await,
            Loop::TaskResumes => supervisor::tick_resumes(self).await,
            Loop::TaskLimits => supervisor::tick_limits(self).await,
            Loop::Workspaces => supervisor::tick_workspaces(self).await,
            Loop::PendingDispatch => supervisor::tick_pending_dispatch(self).await,
            Loop::LimitsEnforcer => usage::tick_limits_enforcer(self).await,
        }
    }

    pub async fn settle(&self) -> Result<usize> {
        let mut total = 0;
        loop {
            let mut progressed = 0;
            for which in Loop::ALL {
                progressed += self.tick(which).await?;
            }
            if progressed == 0 {
                return Ok(total);
            }
            total += progressed;
        }
    }

    pub async fn run(self: Arc<Self>, shutdown: tokio::sync::watch::Receiver<bool>) {
        let handles = Loop::ALL
            .into_iter()
            .map(|which| {
                let worker = Arc::clone(&self);
                let mut shutdown = shutdown.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(worker.settings.loop_interval);
                    loop {
                        tokio::select! {
                            _ = interval.tick() => {}
                            _ = shutdown.changed() => return,
                        }
                        loop {
                            match worker.tick(which).await {
                                Ok(0) => break,
                                Ok(_) => continue,
                                Err(error) => {
                                    eprintln!("sanchoris worker {which:?}: {error}");
                                    break;
                                }
                            }
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            let _ = handle.await;
        }
    }

    pub(crate) async fn begin(&self) -> Result<Transaction<'static, Postgres>> {
        Ok(self.pool.begin().await?)
    }
}

pub(crate) async fn adopt(tx: Transaction<'static, Postgres>, tenant_id: Uuid) -> Result<TenantTx> {
    Ok(TenantTx::adopt(tx, tenant_id).await?)
}

pub(crate) async fn installations(tx: &mut TenantTx) -> Result<Vec<Installation>> {
    let tenant_id = tx.tenant_id();
    let rows = sqlx::query(
        "SELECT github_installation_id, details->>'account_login' AS login FROM sanchoris.integrations \
         WHERE tenant_id = $1 AND kind = 'github_installation' AND status = 'connected' \
         ORDER BY created_at, id",
    )
    .bind(tenant_id)
    .fetch_all(tx.conn())
    .await?;
    rows.iter()
        .map(|row| {
            Ok(Installation {
                id: row.try_get("github_installation_id")?,
                account_login: row.try_get("login")?,
            })
        })
        .collect()
}

pub(crate) fn installation_for(installations: &[Installation], repo: &str) -> Result<i64> {
    ports::installation_for(installations, repo).map_err(WorkerError::Invalid)
}

pub async fn run_from_env(shutdown: tokio::sync::watch::Receiver<bool>) {
    let control = runner::LocalRunnerConfig::from_env();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPool::connect(&database_url)
        .await
        .expect("connect to DATABASE_URL");
    let clock: Arc<dyn Clock> = Arc::new(clock::SystemClock);
    let api = Arc::new(github::GitHubApi::from_env());
    let deps = Deps {
        clock: Arc::clone(&clock),
        github: api.clone(),
        reader: api.clone(),
        config_source: api.clone(),
        resources: Arc::new(resources::GitHubResources::new(api)),
        runner: Arc::new(runner::LocalProcessRunner::new(control, clock)),
    };
    let worker = Arc::new(Worker::new(pool, deps, Settings::default()));
    worker.run(shutdown).await;
}
