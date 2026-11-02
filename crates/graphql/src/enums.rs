use async_graphql::{Enum, Error};
use sanchoris_core::modes::{InteractionMode, RuntimeMode};

macro_rules! db_enum {
    ($name:ident, $kind:literal, { $($(#[$meta:meta])* $variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($(#[$meta])* $variant,)+
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
    /// The limit counts from the creation of the tenant.
    Total => "total",
    Month => "month",
});

db_enum!(TenantState, "tenant state", {
    Active => "active",
    Suspended => "suspended",
    /// Set by deleteTenant. Mutations, webhooks, triggers and dispatch stop for the tenant until the worker removes it.
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
    /// The repository was removed from the GitHub App installation or deleted. The config no longer syncs or starts tasks.
    Disabled => "disabled",
});

db_enum!(WorkspaceCompute, "workspace compute", {
    Standard => "standard",
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
    Schedule => "Scheduled Event",
});

db_enum!(EventSource, "event source", {
    Github => "github",
    Schedule => "schedule",
});

db_enum!(TriggerEventResult, "trigger event result", {
    Started => "started",
    Skipped => "skipped",
    Discarded => "discarded",
});

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerEventReason {
    /// The task tree is over Start.Limits.Children.
    ChildLimit,
    /// The sender is not in the event pattern.
    SenderNotAllowed,
    /// More events than the receive limit.
    ReceiveLimit,
    /// A task with the same name exists.
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
    /// The conversation task finished its turn and waits for the next message.
    Message => "message",
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

db_enum!(ConversationKind, "conversation kind", {
    Web => "web",
    Slack => "slack",
});

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequesterKind {
    /// A member who asked from the screen or a chat.
    Member,
    /// The GitHub App bot, on behalf of the agent of another task.
    Agent,
    /// The GitHub user whose event started the task.
    Github,
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
    /// Platform messages: workspace and session lifecycle.
    System,
    /// Instructions sanchoris sent to the session.
    Instruction,
    Agent,
    Tool,
    /// Skill and session events.
    Event,
    /// Messages people sent.
    User,
    Error,
}

db_enum!(OutputKind, "output kind", {
    PullRequest => "pull_request",
    Comment => "comment",
    Issue => "issue",
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
    /// sanchoris sent it on its own, such as `/compact` after the chat stayed idle.
    Sanchoris,
}

db_enum!(ChatTurnOutcome, "turn outcome", {
    Completed => "completed",
    /// Someone stopped the turn.
    Interrupted => "interrupted",
    Failed => "failed",
    /// The agent went straight on to another turn, such as one for a steer.
    Continued => "continued",
});

db_enum!(ActorKind, "actor kind", {
    Member => "member",
    Agent => "agent",
    Trigger => "trigger",
    System => "system",
    Operator => "operator",
});

db_enum!(AuditOrigin, "audit origin", {
    Web => "web",
    Cli => "cli",
    Github => "github",
    Slack => "slack",
    System => "system",
});

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatInteractionMode {
    Default,
    /// The agent plans without changing files and proposes the plan.
    Plan,
}

impl From<ChatInteractionMode> for InteractionMode {
    fn from(mode: ChatInteractionMode) -> Self {
        match mode {
            ChatInteractionMode::Default => Self::Default,
            ChatInteractionMode::Plan => Self::Plan,
        }
    }
}

impl From<InteractionMode> for ChatInteractionMode {
    fn from(mode: InteractionMode) -> Self {
        match mode {
            InteractionMode::Default => Self::Default,
            InteractionMode::Plan => Self::Plan,
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRuntimeMode {
    /// Ask before commands and file changes.
    ApprovalRequired,
    /// Edit files without asking; ask before other actions.
    AutoAcceptEdits,
    /// Run commands and edit files without asking.
    FullAccess,
}

impl From<ChatRuntimeMode> for RuntimeMode {
    fn from(mode: ChatRuntimeMode) -> Self {
        match mode {
            ChatRuntimeMode::ApprovalRequired => Self::ApprovalRequired,
            ChatRuntimeMode::AutoAcceptEdits => Self::AutoAcceptEdits,
            ChatRuntimeMode::FullAccess => Self::FullAccess,
        }
    }
}

impl From<RuntimeMode> for ChatRuntimeMode {
    fn from(mode: RuntimeMode) -> Self {
        match mode {
            RuntimeMode::ApprovalRequired => Self::ApprovalRequired,
            RuntimeMode::AutoAcceptEdits => Self::AutoAcceptEdits,
            RuntimeMode::FullAccess => Self::FullAccess,
        }
    }
}

db_enum!(ChatApprovalKind, "approval kind", {
    /// A shell command.
    Command => "command",
    /// Edits to files.
    FileChange => "file-change",
    /// Another tool.
    Tool => "tool",
});

db_enum!(ChatApprovalDecision, "approval decision", {
    /// Run it once.
    Accept => "accept",
    /// Run it and stop asking for the same kind of action in this session.
    AcceptForSession => "acceptForSession",
    /// Skip it; the turn goes on.
    Decline => "decline",
    /// Skip it and end the turn.
    Cancel => "cancel",
});

db_enum!(ChatPlanStepStatus, "plan step status", {
    Pending => "pending",
    Running => "running",
    Completed => "completed",
});

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatDelivery {
    /// Hold the message until the agent's turn ends.
    Queue,
    /// Put the message into the agent's running turn.
    Steer,
}

db_enum!(ChatMessageDelivery, "message delivery", {
    /// Held until the agent's turn ends.
    Queued => "queued",
    /// Put into the turn the agent was working on.
    Steered => "steer",
});
