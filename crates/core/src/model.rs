use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
#[error("unknown {kind} value {value:?}")]
pub struct UnknownValue {
    pub kind: &'static str,
    pub value: String,
}

macro_rules! string_enum {
    ($name:ident, $kind:literal, { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $(
                #[serde(rename = $text)]
                $variant,
            )+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }
        }

        impl std::str::FromStr for $name {
            type Err = UnknownValue;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($text => Ok(Self::$variant),)+
                    _ => Err(UnknownValue { kind: $kind, value: value.to_string() }),
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

string_enum!(TaskStatus, "task status", {
    Pending => "PENDING",
    Running => "RUNNING",
    Waiting => "WAITING",
    NeedsYou => "NEEDS_YOU",
    Succeeded => "SUCCEEDED",
    Failed => "FAILED",
    Aborted => "ABORTED",
});

string_enum!(WaitReason, "wait reason", {
    Skill => "skill",
    UsageLimit => "usage_limit",
    Chat => "chat",
});

string_enum!(SkillRunStatus, "skill run status", {
    Running => "RUNNING",
    WaitingAgent => "WAITING_AGENT",
    Waiting => "WAITING",
    Succeeded => "SUCCEEDED",
    Failed => "FAILED",
});

string_enum!(TriggerEventResult, "trigger event result", {
    Started => "started",
    Skipped => "skipped",
    Discarded => "discarded",
});

string_enum!(UsageLimitResume, "usage limit resume", {
    Auto => "auto",
    Manual => "manual",
});

string_enum!(ActorKind, "actor kind", {
    Member => "member",
    Agent => "agent",
    Trigger => "trigger",
    System => "system",
    Operator => "operator",
});

string_enum!(AuditOrigin, "audit origin", {
    Web => "web",
    Github => "github",
    Slack => "slack",
    System => "system",
});

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Aborted)
    }
}

impl SkillRunStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::WaitingAgent | Self::Waiting)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NamedDefinition {
    pub name: String,
    pub definition: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TaskDefinition {
    pub trigger: NamedDefinition,
    pub session: NamedDefinition,
    pub skills: Vec<NamedDefinition>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_round_trip_through_text() {
        for status in [
            TaskStatus::Pending,
            TaskStatus::Running,
            TaskStatus::Waiting,
            TaskStatus::NeedsYou,
            TaskStatus::Succeeded,
            TaskStatus::Failed,
            TaskStatus::Aborted,
        ] {
            assert_eq!(status.as_str().parse::<TaskStatus>().unwrap(), status);
        }
        assert_eq!(
            "DONE".parse::<TaskStatus>().unwrap_err(),
            UnknownValue {
                kind: "task status",
                value: "DONE".into()
            }
        );
        assert_eq!(
            serde_json::to_value(SkillRunStatus::WaitingAgent).unwrap(),
            "WAITING_AGENT"
        );
    }
}
