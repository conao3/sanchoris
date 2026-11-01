use serde::{Deserialize, Serialize};

/// How the agent treats a chat message: work on it, or only plan and propose the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InteractionMode {
    Default,
    Plan,
}

/// What the agent may do without asking the people in the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeMode {
    ApprovalRequired,
    AutoAcceptEdits,
    FullAccess,
}

/// The `modes` of the agent's `start` and `session_mode` messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionModes {
    pub interaction_mode: InteractionMode,
    pub runtime_mode: RuntimeMode,
}

impl SessionModes {
    pub const DEFAULT: Self = Self {
        interaction_mode: InteractionMode::Default,
        runtime_mode: RuntimeMode::FullAccess,
    };
}
