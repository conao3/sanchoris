use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("state {0:?} is not defined in the skill")]
    UnknownState(String),
    #[error("invalid resource {resource:?}: {reason}")]
    InvalidResource { resource: String, reason: String },
    #[error("invalid skill input: {0}")]
    InvalidInput(String),
    #[error("invalid skill definition: {0}")]
    InvalidDefinition(String),
    #[error("the skill run state is inconsistent with the definition: {0}")]
    Inconsistent(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SkillRunStatus {
    Running,
    WaitingAgent,
    Waiting,
    Succeeded,
    Failed,
}

impl SkillRunStatus {
    pub fn is_finished(self) -> bool {
        matches!(self, SkillRunStatus::Succeeded | SkillRunStatus::Failed)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SkillRunStatus::Running => "RUNNING",
            SkillRunStatus::WaitingAgent => "WAITING_AGENT",
            SkillRunStatus::Waiting => "WAITING",
            SkillRunStatus::Succeeded => "SUCCEEDED",
            SkillRunStatus::Failed => "FAILED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StateEntryStatus {
    Running,
    Waiting,
    Succeeded,
    Failed,
    Caught,
    TimedOut,
}

impl StateEntryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            StateEntryStatus::Running => "RUNNING",
            StateEntryStatus::Waiting => "WAITING",
            StateEntryStatus::Succeeded => "SUCCEEDED",
            StateEntryStatus::Failed => "FAILED",
            StateEntryStatus::Caught => "CAUGHT",
            StateEntryStatus::TimedOut => "TIMED_OUT",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillRun {
    pub id: i64,
    pub skill_name: String,
    pub status: SkillRunStatus,
    pub input: Value,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub cause: Option<String>,
    pub started_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub next_sequence: u64,
    pub current: Option<StateEntry>,
}

impl SkillRun {
    pub fn waiting_task_token(&self) -> Option<&str> {
        match self.current.as_ref()?.progress.as_ref()? {
            Progress::Invoking { task_token, .. } => task_token.as_deref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateEntry {
    pub sequence: u64,
    pub state_name: String,
    pub state_type: String,
    pub status: StateEntryStatus,
    pub entered_at: DateTime<Utc>,
    pub exited_at: Option<DateTime<Utc>>,
    pub input: Value,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub cause: Option<String>,
    pub attempts: u32,
    pub next_state: Option<String>,
    pub progress: Option<Progress>,
    pub retry_counts: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Progress {
    Waiting {
        until: DateTime<Utc>,
    },
    Invoking {
        attempt: u32,
        timeout_at: DateTime<Utc>,
        heartbeat_at: Option<DateTime<Utc>>,
        task_token: Option<String>,
        acknowledged: bool,
    },
    BackingOff {
        next_attempt: u32,
        retry_at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct IdempotencyKey {
    pub skill_run_id: i64,
    pub sequence: u64,
    pub attempt: u32,
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.skill_run_id, self.sequence, self.attempt
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Integration {
    RequestResponse,
    Sync,
    WaitForTaskToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRef {
    pub service: String,
    pub action: String,
    pub integration: Integration,
}

pub const RESOURCE_PREFIX: &str = "sanchoris:::";

impl ResourceRef {
    pub fn parse(resource: &str) -> Result<Self, EngineError> {
        let invalid = |reason: &str| EngineError::InvalidResource {
            resource: resource.to_string(),
            reason: reason.to_string(),
        };
        let rest = resource
            .strip_prefix(RESOURCE_PREFIX)
            .ok_or_else(|| invalid("a resource starts with sanchoris:::"))?;
        let (rest, integration) = if let Some(base) = rest.strip_suffix(".waitForTaskToken") {
            (base, Integration::WaitForTaskToken)
        } else if let Some(base) = rest.strip_suffix(".sync") {
            (base, Integration::Sync)
        } else {
            (rest, Integration::RequestResponse)
        };
        let (service, action) = rest
            .split_once(':')
            .ok_or_else(|| invalid("a resource has the form sanchoris:::{service}:{action}"))?;
        let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric());
        if !valid(service) || !valid(action) {
            return Err(invalid("service and action are alphanumeric"));
        }
        let parsed = ResourceRef {
            service: service.to_string(),
            action: action.to_string(),
            integration,
        };
        if parsed.is_agent_handoff() && integration != Integration::WaitForTaskToken {
            return Err(invalid(
                "agent:handoff is only available as .waitForTaskToken",
            ));
        }
        Ok(parsed)
    }

    pub fn name(&self) -> String {
        format!("{}:{}", self.service, self.action)
    }

    pub fn is_agent_handoff(&self) -> bool {
        self.service == "agent" && self.action == "handoff"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Timer {
    SkillRunTimeout,
    WaitElapsed { sequence: u64 },
    RetryBackoff { sequence: u64, attempt: u32 },
    TaskTimeout { sequence: u64, attempt: u32 },
    HeartbeatTimeout { sequence: u64, attempt: u32 },
}

impl Timer {
    pub fn sequence(&self) -> Option<u64> {
        match self {
            Timer::SkillRunTimeout => None,
            Timer::WaitElapsed { sequence }
            | Timer::RetryBackoff { sequence, .. }
            | Timer::TaskTimeout { sequence, .. }
            | Timer::HeartbeatTimeout { sequence, .. } => Some(*sequence),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Invocation {
    pub resource: ResourceRef,
    pub parameters: Value,
    pub idempotency_key: IdempotencyKey,
    pub task_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NotificationKind {
    Succeeded {
        output: Value,
    },
    Failed {
        error: String,
        cause: Option<String>,
    },
    NeedsAgent {
        instruction: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub skill_run_id: i64,
    pub skill_name: String,
    pub kind: NotificationKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum Effect {
    InvokeResource(Invocation),
    ScheduleTimer { timer: Timer, due_at: DateTime<Utc> },
    CancelTimers { sequence: u64 },
    CancelAllTimers,
    Notify(Notification),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ResourceOutcome {
    Succeeded {
        output: Value,
    },
    Failed {
        error: Option<String>,
        cause: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    ResourceResult {
        key: IdempotencyKey,
        outcome: ResourceOutcome,
    },
    TaskSuccess {
        task_token: String,
        output: Value,
    },
    TaskFailure {
        task_token: String,
        error: Option<String>,
        cause: Option<String>,
    },
    TaskHeartbeat {
        task_token: String,
    },
    TimerDue {
        timer: Timer,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IgnoreReason {
    RunFinished,
    StaleIdempotencyKey,
    DuplicateResult,
    UnknownTaskToken,
    StaleTimer,
    TimerNotDue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", content = "reason", rename_all = "snake_case")]
pub enum Disposition {
    Applied,
    Ignored(IgnoreReason),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub run: SkillRun,
    pub entries: Vec<StateEntry>,
    pub effects: Vec<Effect>,
    pub disposition: Disposition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartRequest {
    pub skill_run_id: i64,
    pub skill_name: String,
    pub input: Value,
}
