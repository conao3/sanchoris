pub mod backfill;
pub mod config_sync;
pub mod deadline;
pub mod dispatcher;
pub mod driver;
pub mod engine_worker;
pub mod error;
pub mod github;
pub mod ports;
pub mod resources;
pub mod runner;
pub mod schedule;
pub mod supervisor;
pub mod tenant_delete;
pub mod usage;

pub use sanchoris_core::clock;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use chrono::Duration;
use sanchoris_core::db::{DbError, Store, TenantTx};
use sanchoris_core::items::{Integration, github_repositories};
use sanchoris_core::jobs::{Envelope, Job};
use sanchoris_core::keys;
use sanchoris_core::ports::BlobStore;
use sanchoris_core::tenants::accepts_work;

use crate::clock::Clock;
use crate::error::{Result, WorkerError};
use crate::ports::{
    ConfigSource, GitHubClient, GitHubReader, Installation, OutboxPusher, ResourceInvoker,
    RunnerPort,
};

pub struct Deps {
    pub clock: Arc<dyn Clock>,
    pub github: Arc<dyn GitHubClient>,
    pub reader: Arc<dyn GitHubReader>,
    pub config_source: Arc<dyn ConfigSource>,
    pub resources: Arc<dyn ResourceInvoker>,
    pub runner: Arc<dyn RunnerPort>,
    pub pusher: Arc<dyn OutboxPusher>,
    pub records: Arc<dyn BlobStore>,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub check_poll_interval: Duration,
    pub config_poll_interval: Duration,
    pub usage_snapshot_interval: Duration,
    pub conflict_attempts: u32,
    pub trigger_starts_per_hour: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            check_poll_interval: Duration::minutes(5),
            config_poll_interval: Duration::minutes(5),
            usage_snapshot_interval: Duration::seconds(60),
            conflict_attempts: 20,
            trigger_starts_per_hour: 60,
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
    pub store: Store,
    pub deps: Deps,
    pub settings: Settings,
    pub faults: Faults,
}

impl Worker {
    pub fn new(store: Store, deps: Deps, settings: Settings) -> Self {
        Self {
            store,
            deps,
            settings,
            faults: Faults::default(),
        }
    }

    pub fn now(&self) -> chrono::DateTime<chrono::Utc> {
        self.deps.clock.now()
    }

    pub fn tx(&self, tenant_id: uuid::Uuid) -> TenantTx {
        self.store.tx(tenant_id)
    }

    pub async fn handle_body(&self, body: &str) -> Result<usize> {
        let envelope = Envelope::parse(body)
            .map_err(|e| WorkerError::Invalid(format!("unreadable message: {e}")))?;
        self.handle(&envelope).await
    }

    pub async fn handle(&self, envelope: &Envelope) -> Result<usize> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.handle_once(envelope).await {
                Err(error) if error.is_conflict() && attempt < self.settings.conflict_attempts => {
                    let jitter = u64::from(uuid::Uuid::new_v4().as_bytes()[0]) % 20;
                    tokio::time::sleep(std::time::Duration::from_millis(
                        5 * u64::from(attempt) + jitter,
                    ))
                    .await;
                }
                other => return other,
            }
        }
    }

    async fn handle_once(&self, envelope: &Envelope) -> Result<usize> {
        if let Some(tenant_id) = held_by_tenant(&envelope.job)
            && !accepts_work(&self.store, tenant_id).await?
        {
            return Ok(0);
        }
        match &envelope.job {
            Job::Inbound { tenant_id } => dispatcher::process_inbound(self, *tenant_id).await,
            Job::Dispatch { tenant_id } => dispatcher::dispatch(self, *tenant_id).await,
            Job::RunEvents {
                tenant_id, run_id, ..
            } => supervisor::process_run_events(self, *tenant_id, *run_id).await,
            Job::Invoke {
                tenant_id,
                invocation_id,
                ..
            } => engine_worker::invoke(self, *tenant_id, *invocation_id).await,
            Job::EngineTimer {
                tenant_id,
                task_id,
                skill_run_id,
                timer_id,
            } => {
                engine_worker::fire_timer(
                    self,
                    *tenant_id,
                    *task_id,
                    *skill_run_id,
                    *timer_id,
                    envelope.id,
                )
                .await
            }
            Job::TaskResume {
                tenant_id,
                task_id,
                resume_at,
            } => supervisor::resume(self, *tenant_id, *task_id, *resume_at).await,
            Job::CheckPoll {
                tenant_id, wait_id, ..
            } => engine_worker::poll_check(self, *tenant_id, *wait_id).await,
            Job::Reconcile {
                tenant_id,
                task_id,
                run_id,
            } => supervisor::reconcile(self, *tenant_id, *task_id, *run_id).await,
            Job::DeliverOutbox { tenant_id, run_id } => {
                self.deps.pusher.push(*tenant_id, *run_id).await?;
                Ok(1)
            }
            Job::ConfigSync {
                tenant_id,
                config_id,
            } => {
                config_sync::sync_config(self, *tenant_id, *config_id).await?;
                Ok(1)
            }
            Job::TriggerSchedule {
                tenant_id,
                config_id,
                trigger_id,
                due_at,
            } => {
                dispatcher::fire_schedule(
                    self,
                    *tenant_id,
                    *config_id,
                    *trigger_id,
                    *due_at,
                    envelope.id,
                )
                .await
            }
            Job::DeleteTenant { tenant_id, phase } => {
                tenant_delete::delete_tenant(self, *tenant_id, phase).await
            }
            Job::SweepConfigs => config_sync::sweep(self).await,
            Job::SweepChecks => engine_worker::sweep_checks(self).await,
            Job::SweepTaskLimits => supervisor::sweep_limits(self).await,
            Job::UsageAlerts => usage::sweep_alerts(self).await,
            Job::SweepWorkspaces => supervisor::sweep_workspaces(self).await,
        }
    }
}

fn held_by_tenant(job: &Job) -> Option<uuid::Uuid> {
    match job {
        Job::Inbound { tenant_id }
        | Job::Dispatch { tenant_id }
        | Job::Invoke { tenant_id, .. }
        | Job::EngineTimer { tenant_id, .. }
        | Job::TaskResume { tenant_id, .. }
        | Job::CheckPoll { tenant_id, .. }
        | Job::ConfigSync { tenant_id, .. }
        | Job::TriggerSchedule { tenant_id, .. } => Some(*tenant_id),
        Job::RunEvents { .. }
        | Job::Reconcile { .. }
        | Job::DeliverOutbox { .. }
        | Job::DeleteTenant { .. }
        | Job::SweepConfigs
        | Job::SweepChecks
        | Job::SweepTaskLimits
        | Job::UsageAlerts
        | Job::SweepWorkspaces => None,
    }
}

pub const PERIODIC_JOBS: [Job; 5] = [
    Job::SweepConfigs,
    Job::SweepChecks,
    Job::SweepTaskLimits,
    Job::UsageAlerts,
    Job::SweepWorkspaces,
];

pub async fn installations(tx: &mut TenantTx) -> std::result::Result<Vec<Installation>, DbError> {
    let mut integrations = tx
        .query::<Integration>(&keys::tenant_pk(tx.tenant_id()), "INTEG#")
        .await?
        .into_iter()
        .filter(|i| i.kind == "github_installation" && i.status == "connected")
        .collect::<Vec<_>>();
    integrations.sort_by_key(|a| (a.created_at, a.id));
    integrations
        .into_iter()
        .filter_map(|i| {
            i.github_installation_id.map(|id| {
                Ok(Installation {
                    id,
                    account_login: i.details["account_login"].as_str().map(str::to_string),
                    repositories: github_repositories(&i.details)?,
                })
            })
        })
        .collect()
}

pub(crate) fn installation_for(installations: &[Installation], repo: &str) -> Result<i64> {
    ports::installation_for(installations, repo).map_err(WorkerError::Invalid)
}

pub async fn installation_for_repo(tx: &mut TenantTx, repo: &str) -> Result<i64> {
    installation_for(&installations(tx).await?, repo)
}
