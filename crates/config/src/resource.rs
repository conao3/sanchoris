use crate::config::PermissionLevel;
use crate::permissions::required_permissions;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntegrationPattern {
    RequestResponse,
    Sync,
    WaitForTaskToken,
}

impl IntegrationPattern {
    pub fn suffix(self) -> &'static str {
        match self {
            IntegrationPattern::RequestResponse => "",
            IntegrationPattern::Sync => ".sync",
            IntegrationPattern::WaitForTaskToken => ".waitForTaskToken",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Resource {
    pub service: &'static str,
    pub action: &'static str,
    pub pattern: IntegrationPattern,
    pub required_parameters: &'static [&'static str],
    pub optional_parameters: &'static [&'static str],
}

impl Resource {
    pub fn arn(&self) -> String {
        format!(
            "{PREFIX}{}:{}{}",
            self.service,
            self.action,
            self.pattern.suffix()
        )
    }

    pub fn is_github(&self) -> bool {
        matches!(self.service, "github" | "git")
    }

    pub fn repo_parameter(&self) -> Option<&'static str> {
        self.required_parameters
            .iter()
            .copied()
            .find(|p| matches!(*p, "Repos" | "Repo"))
    }

    pub fn permissions(&self) -> &'static [(&'static str, PermissionLevel)] {
        required_permissions(self.service, self.action)
    }

    pub fn accepts_parameter(&self, name: &str) -> bool {
        self.required_parameters.contains(&name) || self.optional_parameters.contains(&name)
    }
}

pub const PREFIX: &str = "sanchoris:::";

pub const RESOURCES: &[Resource] = &[
    Resource {
        service: "workspace",
        action: "exec",
        pattern: IntegrationPattern::Sync,
        required_parameters: &["Commands"],
        optional_parameters: &[],
    },
    Resource {
        service: "git",
        action: "push",
        pattern: IntegrationPattern::RequestResponse,
        required_parameters: &["Repos"],
        optional_parameters: &[],
    },
    Resource {
        service: "github",
        action: "createPullRequests",
        pattern: IntegrationPattern::RequestResponse,
        required_parameters: &["Repos"],
        optional_parameters: &["Title", "Body"],
    },
    Resource {
        service: "github",
        action: "checks",
        pattern: IntegrationPattern::WaitForTaskToken,
        required_parameters: &["Repos", "TaskToken"],
        optional_parameters: &[],
    },
    Resource {
        service: "github",
        action: "commentOnIssue",
        pattern: IntegrationPattern::RequestResponse,
        required_parameters: &["Repo", "Number", "Body", "Mode"],
        optional_parameters: &[],
    },
    Resource {
        service: "github",
        action: "mergePullRequest",
        pattern: IntegrationPattern::RequestResponse,
        required_parameters: &["Repo", "Number"],
        optional_parameters: &[],
    },
    Resource {
        service: "chat",
        action: "post",
        pattern: IntegrationPattern::RequestResponse,
        required_parameters: &["Text"],
        optional_parameters: &[],
    },
    Resource {
        service: "agent",
        action: "handoff",
        pattern: IntegrationPattern::WaitForTaskToken,
        required_parameters: &["Instruction", "TaskToken"],
        optional_parameters: &[],
    },
    Resource {
        service: "approval",
        action: "request",
        pattern: IntegrationPattern::WaitForTaskToken,
        required_parameters: &["Message", "TaskToken"],
        optional_parameters: &[],
    },
];

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ResourceError {
    #[error("resource {0:?} must be sanchoris:::<service>:<action>[.sync|.waitForTaskToken]")]
    Syntax(String),
    #[error("unknown resource {0:?}")]
    Unknown(String),
}

pub fn parse_resource(text: &str) -> Result<&'static Resource, ResourceError> {
    let syntax = || ResourceError::Syntax(text.to_string());
    let rest = text.strip_prefix(PREFIX).ok_or_else(syntax)?;
    let (service, action) = rest.split_once(':').ok_or_else(syntax)?;
    let (action, pattern) = if let Some(a) = action.strip_suffix(".sync") {
        (a, IntegrationPattern::Sync)
    } else if let Some(a) = action.strip_suffix(".waitForTaskToken") {
        (a, IntegrationPattern::WaitForTaskToken)
    } else {
        (action, IntegrationPattern::RequestResponse)
    };
    let word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric());
    if !word(service) || !word(action) {
        return Err(syntax());
    }
    RESOURCES
        .iter()
        .find(|r| r.service == service && r.action == action && r.pattern == pattern)
        .ok_or_else(|| ResourceError::Unknown(text.to_string()))
}
