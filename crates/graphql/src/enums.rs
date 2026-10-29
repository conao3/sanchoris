use async_graphql::{Enum, Error};

macro_rules! db_enum {
    ($name:ident, $kind:literal, { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant,)+
        }

        impl $name {
            pub fn from_db(text: &str) -> Result<Self, Error> {
                match text {
                    $($text => Ok(Self::$variant),)+
                    other => Err(Error::new(format!("unknown {} {other:?}", $kind))),
                }
            }

            #[allow(dead_code)]
            pub fn as_db(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }
        }
    };
}

db_enum!(InvitationState, "invitation state", {
    Pending => "pending",
    Accepted => "accepted",
    Revoked => "revoked",
    Expired => "expired",
});

db_enum!(BillingUnit, "billing unit", {
    Tenant => "tenant",
    Seat => "seat",
});

db_enum!(LimitPeriod, "limit period", {
    Total => "total",
    Month => "month",
});

db_enum!(TenantState, "tenant state", {
    Active => "active",
    Suspended => "suspended",
    Deleting => "deleting",
});

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubOwnerKind {
    Organization,
    User,
}

db_enum!(IntegrationKind, "integration kind", {
    GithubInstallation => "github_installation",
    Slack => "slack",
});

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationStatus {
    Connected,
    NotConnected,
    Error,
}

db_enum!(ConfigStatus, "config status", {
    Active => "active",
    Disabled => "disabled",
});

db_enum!(WorkspaceCompute, "workspace compute", {
    Standard => "standard",
    Docker => "docker",
});

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceArchitecture {
    #[graphql(name = "ARM64")]
    Arm64,
    #[graphql(name = "X86_64")]
    X86_64,
}

impl WorkspaceArchitecture {
    pub fn from_db(text: &str) -> Result<Self, Error> {
        match text {
            "arm64" => Ok(Self::Arm64),
            "x86_64" => Ok(Self::X86_64),
            other => Err(Error::new(format!(
                "unknown workspace architecture {other:?}"
            ))),
        }
    }
}

db_enum!(UsageLimitResume, "usage limit resume", {
    Auto => "auto",
    Manual => "manual",
});

db_enum!(ChangeRequestState, "change request state", {
    Open => "open",
    Merged => "merged",
    Closed => "closed",
});

db_enum!(ConfigObjectKind, "config object kind", {
    Trigger => "trigger",
    Session => "session",
    Skill => "skill",
    Project => "project",
});

db_enum!(Provider, "provider", {
    Claude => "claude",
    Codex => "codex",
});

db_enum!(CredentialKind, "credential kind", {
    Subscription => "subscription",
    ApiKey => "api_key",
    Bedrock => "bedrock",
});

db_enum!(CredentialScope, "credential scope", {
    Personal => "personal",
    Organization => "org",
});

db_enum!(CredentialStatus, "credential status", {
    Connected => "connected",
    Reconnect => "reconnect",
});

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeyProvider {
    Anthropic,
    Openai,
    Bedrock,
}

db_enum!(TriggerState, "trigger state", {
    Enabled => "ENABLED",
    Disabled => "DISABLED",
});

db_enum!(TriggerEventKind, "trigger event kind", {
    IssueLabeled => "Issue Labeled",
    IssueCommentCreated => "Issue Comment Created",
    CheckRunCompleted => "Check Run Completed",
    Message => "Message",
    Schedule => "Scheduled Event",
});

db_enum!(EventSource, "event source", {
    Github => "github",
    Slack => "slack",
    Web => "web",
    Schedule => "schedule",
});

db_enum!(TriggerEventResult, "trigger event result", {
    Started => "started",
    Skipped => "skipped",
    Discarded => "discarded",
});

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerEventReason {
    ChildLimit,
    SenderNotAllowed,
    ReceiveLimit,
    Duplicate,
}

db_enum!(SkillKind, "skill kind", {
    Reliable => "reliable",
    Plain => "regular",
});

db_enum!(StateType, "state type", {
    Task => "Task",
    Pass => "Pass",
    Choice => "Choice",
    Wait => "Wait",
    Succeed => "Succeed",
    Fail => "Fail",
});

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionKind {
    Next,
    Choice,
    Default,
    Catch,
}

db_enum!(SkillRunStatus, "skill run status", {
    Running => "RUNNING",
    WaitingAgent => "WAITING_AGENT",
    Waiting => "WAITING",
    Succeeded => "SUCCEEDED",
    Failed => "FAILED",
});

db_enum!(StateEntryStatus, "state entry status", {
    Running => "RUNNING",
    Waiting => "WAITING",
    Succeeded => "SUCCEEDED",
    Failed => "FAILED",
    Caught => "CAUGHT",
    TimedOut => "TIMED_OUT",
});

db_enum!(TaskStatus, "task status", {
    Pending => "PENDING",
    Running => "RUNNING",
    Waiting => "WAITING",
    NeedsYou => "NEEDS_YOU",
    Succeeded => "SUCCEEDED",
    Failed => "FAILED",
    Aborted => "ABORTED",
});

db_enum!(WaitReason, "wait reason", {
    Skill => "skill",
    UsageLimit => "usage_limit",
    Chat => "chat",
});

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingReason {
    Blocked,
    Concurrency,
    PlanLimit,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeedsYouReason {
    SkillFailed,
    NudgesExhausted,
    AgentAsked,
    ApprovalRequested,
    UsageLimit,
    LimitExceeded,
    WorkspaceStopped,
}

db_enum!(ChannelKind, "channel kind", {
    Web => "web",
    Slack => "slack",
});

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequesterKind {
    Member,
    Agent,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredSkillState {
    NotRun,
    Running,
    Succeeded,
    Failed,
}

db_enum!(RunStatus, "run status", {
    Creating => "creating",
    Running => "running",
    Idle => "idle",
    Completed => "completed",
    Failed => "failed",
});

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLineKind {
    System,
    Instruction,
    Agent,
    Tool,
    Event,
    User,
    Error,
}

db_enum!(OutputKind, "output kind", {
    PullRequest => "pull_request",
    Comment => "comment",
});

db_enum!(OutputState, "output state", {
    Open => "open",
    Merged => "merged",
    Closed => "closed",
});

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatAuthorKind {
    Member,
    Agent,
}

db_enum!(ActorKind, "actor kind", {
    Member => "member",
    Agent => "agent",
    Trigger => "trigger",
    System => "system",
    Operator => "operator",
});

db_enum!(AuditOrigin, "audit origin", {
    Web => "web",
    Github => "github",
    Slack => "slack",
    System => "system",
});
