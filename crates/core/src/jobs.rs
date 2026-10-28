use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Job {
    Inbound {
        tenant_id: Uuid,
    },
    Dispatch {
        tenant_id: Uuid,
    },
    RunEvents {
        tenant_id: Uuid,
        task_id: Uuid,
        run_id: Uuid,
    },
    Invoke {
        tenant_id: Uuid,
        task_id: Uuid,
        invocation_id: Uuid,
    },
    EngineTimer {
        tenant_id: Uuid,
        task_id: Uuid,
        skill_run_id: Uuid,
        timer_id: Uuid,
    },
    TaskResume {
        tenant_id: Uuid,
        task_id: Uuid,
        resume_at: DateTime<Utc>,
    },
    CheckPoll {
        tenant_id: Uuid,
        task_id: Uuid,
        wait_id: Uuid,
    },
    Reconcile {
        tenant_id: Uuid,
        task_id: Uuid,
        run_id: Uuid,
    },
    DeliverOutbox {
        tenant_id: Uuid,
        run_id: Uuid,
    },
    ConfigSync {
        tenant_id: Uuid,
        config_id: Uuid,
    },
    TriggerSchedule {
        tenant_id: Uuid,
        config_id: Uuid,
        trigger_id: Uuid,
        due_at: DateTime<Utc>,
    },
    SweepConfigs,
    SweepChecks,
    SweepTaskLimits,
    UsageAlerts,
    SweepWorkspaces,
}

impl Job {
    pub fn group(&self) -> String {
        match self {
            Self::Inbound { tenant_id }
            | Self::Dispatch { tenant_id }
            | Self::TriggerSchedule { tenant_id, .. } => format!("tenant:{tenant_id}"),
            Self::RunEvents { task_id, .. }
            | Self::Invoke { task_id, .. }
            | Self::EngineTimer { task_id, .. }
            | Self::TaskResume { task_id, .. }
            | Self::CheckPoll { task_id, .. }
            | Self::Reconcile { task_id, .. } => format!("task:{task_id}"),
            Self::DeliverOutbox { run_id, .. } => format!("run:{run_id}"),
            Self::ConfigSync { config_id, .. } => format!("config:{config_id}"),
            Self::SweepConfigs => "sweep:configs".into(),
            Self::SweepChecks => "sweep:checks".into(),
            Self::SweepTaskLimits => "sweep:task-limits".into(),
            Self::UsageAlerts => "sweep:usage-alerts".into(),
            Self::SweepWorkspaces => "sweep:workspaces".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub id: Uuid,
    pub job: Job,
}

impl Envelope {
    pub fn new(job: Job) -> Self {
        Self {
            id: Uuid::new_v4(),
            job,
        }
    }

    pub fn body(&self) -> String {
        serde_json::to_string(self).expect("envelopes serialize")
    }

    pub fn parse(body: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(body)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "relay", rename_all = "snake_case")]
pub enum Relayed {
    Send {
        envelope: Envelope,
        deliver_at: Option<DateTime<Utc>>,
    },
    Cancel {
        schedule: String,
    },
}

pub fn schedule_name(id: Uuid) -> String {
    format!("sanchoris-{}", id.simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelopes_round_trip_and_group_by_aggregate() {
        let tenant_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let envelope = Envelope::new(Job::Reconcile {
            tenant_id,
            task_id,
            run_id: Uuid::new_v4(),
        });
        assert_eq!(Envelope::parse(&envelope.body()).unwrap(), envelope);
        assert_eq!(envelope.job.group(), format!("task:{task_id}"));
        assert_eq!(
            Job::Inbound { tenant_id }.group(),
            format!("tenant:{tenant_id}")
        );
        assert!(schedule_name(envelope.id).len() <= 64);
    }

    #[test]
    fn rate_schedule_bodies_parse_with_their_scheduled_time() {
        let body = r#"{"id":"00000000-0000-0000-0000-000000000000","job":{"type":"sweep_checks"},"scheduledTime":"2026-10-27T09:00:00Z"}"#;
        let envelope = Envelope::parse(body).unwrap();
        assert_eq!(envelope.job, Job::SweepChecks);
        assert_eq!(envelope.job.group(), "sweep:checks");
    }
}
