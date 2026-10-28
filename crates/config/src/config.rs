use crate::decode::Decoder;
use crate::error::{ValidationError, ValidationErrors};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const CONFIG_FILE: &str = "sanchoris.yaml";
pub const REQUESTER: &str = "requester";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct Config {
    pub version: u64,
    pub projects: IndexMap<String, Project>,
    pub sessions: IndexMap<String, Session>,
    pub triggers: IndexMap<String, Trigger>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct Project {
    pub system_prompt: String,
    pub repos: IndexMap<String, ProjectRepo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct ProjectRepo {
    pub repo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warm_hook: Option<Hook>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_hook: Option<Hook>,
}

impl ProjectRepo {
    pub fn full_name(&self) -> Option<&str> {
        let name = self.repo.strip_prefix("github.com/")?;
        is_repo_name(name).then_some(name)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct Hook {
    pub commands: Vec<String>,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct Session {
    pub worker: Worker,
    #[serde(flatten)]
    pub checkout: Checkout,
    #[serde(rename = "GitHub")]
    pub github: GitHubAccess,
    pub network: Network,
    pub usage_limit: UsageLimit,
    pub workspace: Workspace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct Worker {
    pub agent: Agent,
    pub model: String,
    pub credential: String,
}

impl Worker {
    pub fn uses_requester(&self) -> bool {
        self.credential == REQUESTER
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Checkout {
    Project(String),
    Repos(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct GitHubAccess {
    pub repos: Vec<String>,
    pub permissions: BTreeMap<String, PermissionLevel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionLevel {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct Network {
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct UsageLimit {
    pub resume: ResumeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResumeMode {
    Auto,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct Workspace {
    pub compute: Compute,
    pub architecture: Architecture,
    pub idle_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Architecture {
    #[serde(rename = "arm64")]
    Arm64,
    #[serde(rename = "x86_64")]
    X86_64,
}

impl Architecture {
    pub fn as_str(self) -> &'static str {
        match self {
            Architecture::Arm64 => "arm64",
            Architecture::X86_64 => "x86_64",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Compute {
    Standard,
    Docker,
}

impl Compute {
    pub fn weight(self) -> u64 {
        match self {
            Compute::Standard => 1,
            Compute::Docker => 3,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Compute::Standard => "standard",
            Compute::Docker => "docker",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct Trigger {
    pub state: TriggerState,
    #[serde(flatten)]
    pub source: TriggerSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_transformer: Option<InputTransformer>,
    pub start: Start,
}

impl Trigger {
    pub fn event_sources(&self) -> Vec<&str> {
        match &self.source {
            TriggerSource::EventPattern(p) => p
                .get("source")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default(),
            TriggerSource::ScheduleExpression(_) => Vec::new(),
        }
    }

    pub fn is_github(&self) -> bool {
        self.event_sources().contains(&"github")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TriggerState {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TriggerSource {
    EventPattern(Value),
    ScheduleExpression(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct InputTransformer {
    pub input_paths_map: IndexMap<String, String>,
    pub input_template: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct Start {
    pub session: String,
    pub instruction: String,
    pub skills: Vec<String>,
    pub required_skills: Vec<String>,
    pub max_nudges: u64,
    pub limits: Limits,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct Limits {
    pub turns: u64,
    pub cost_usd: f64,
    pub timeout_seconds: u64,
    pub heartbeat_seconds: u64,
    pub children: ChildLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct ChildLimits {
    pub max_depth: u64,
    pub max_count: u64,
}

pub(crate) fn is_repo_name(s: &str) -> bool {
    let Some((owner, name)) = s.split_once('/') else {
        return false;
    };
    let valid = |p: &str| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    valid(owner) && valid(name)
}

pub fn parse_config_yaml(text: &str) -> Result<Config, ValidationErrors> {
    let value: Value = serde_yaml_ng::from_str(text).map_err(|e| {
        ValidationErrors(vec![ValidationError::new(
            CONFIG_FILE,
            format!("invalid YAML: {e}"),
        )])
    })?;
    let mut errors = Vec::new();
    let config = decode_config(
        &mut Decoder {
            errors: &mut errors,
        },
        &value,
    );
    match config {
        Some(c) if errors.is_empty() => Ok(c),
        _ => Err(ValidationErrors(errors)),
    }
}

fn decode_config(d: &mut Decoder, v: &Value) -> Option<Config> {
    let m = d.object(v, "", &["Version", "Projects", "Sessions", "Triggers"])?;
    let version = d.field(m, "", "Version", |d, v, loc| {
        let n = d.u64(v, loc)?;
        if n != 1 {
            d.error(loc, "unsupported version; expected 1");
            return None;
        }
        Some(n)
    });
    let projects = d.field(m, "", "Projects", |d, v, loc| d.map(v, loc, decode_project));
    let sessions = d.field(m, "", "Sessions", |d, v, loc| d.map(v, loc, decode_session));
    let triggers = d.field(m, "", "Triggers", |d, v, loc| d.map(v, loc, decode_trigger));
    Some(Config {
        version: version?,
        projects: projects?,
        sessions: sessions?,
        triggers: triggers?,
    })
}

fn decode_project(d: &mut Decoder, v: &Value, loc: &str) -> Option<Project> {
    let m = d.object(v, loc, &["SystemPrompt", "Repos"])?;
    let system_prompt = d.field(m, loc, "SystemPrompt", |d, v, loc| d.text(v, loc));
    let repos = d.field(m, loc, "Repos", |d, v, loc| {
        let repos = d.map(v, loc, decode_project_repo)?;
        if repos.is_empty() {
            d.error(loc, "must list at least one repository");
            return None;
        }
        Some(repos)
    });
    Some(Project {
        system_prompt: system_prompt?,
        repos: repos?,
    })
}

fn decode_project_repo(d: &mut Decoder, v: &Value, loc: &str) -> Option<ProjectRepo> {
    let m = d.object(v, loc, &["Repo", "WarmHook", "StartHook"])?;
    let repo = d.field(m, loc, "Repo", |d, v, loc| {
        let s = d.string(v, loc)?;
        let ok = s.strip_prefix("github.com/").is_some_and(is_repo_name);
        if !ok {
            d.error(loc, format!("{s:?} must be github.com/<owner>/<name>"));
            return None;
        }
        Some(s)
    });
    let warm_hook = d.optional(m, loc, "WarmHook", decode_hook);
    let start_hook = d.optional(m, loc, "StartHook", decode_hook);
    Some(ProjectRepo {
        repo: repo?,
        warm_hook: warm_hook?,
        start_hook: start_hook?,
    })
}

fn decode_hook(d: &mut Decoder, v: &Value, loc: &str) -> Option<Hook> {
    let m = d.object(v, loc, &["Commands", "TimeoutSeconds"])?;
    let commands = d.field(m, loc, "Commands", |d, v, loc| {
        let c = d.strings(v, loc)?;
        if c.is_empty() {
            d.error(loc, "must list at least one command");
            return None;
        }
        Some(c)
    });
    let timeout_seconds = d.field(m, loc, "TimeoutSeconds", |d, v, loc| d.positive(v, loc));
    Some(Hook {
        commands: commands?,
        timeout_seconds: timeout_seconds?,
    })
}

fn decode_session(d: &mut Decoder, v: &Value, loc: &str) -> Option<Session> {
    let m = d.object(
        v,
        loc,
        &[
            "Worker",
            "Project",
            "Repos",
            "GitHub",
            "Network",
            "UsageLimit",
            "Workspace",
        ],
    )?;
    let worker = d.field(m, loc, "Worker", decode_worker);
    let checkout = match (m.get("Project"), m.get("Repos")) {
        (Some(_), Some(_)) => {
            d.error(loc, "exactly one of Project or Repos is allowed");
            None
        }
        (None, None) => {
            d.error(
                crate::error::at(loc, "Project"),
                "missing required field; set Project or Repos",
            );
            None
        }
        (Some(p), None) => d
            .string(p, &crate::error::at(loc, "Project"))
            .map(Checkout::Project),
        (None, Some(r)) => {
            let rloc = crate::error::at(loc, "Repos");
            d.list(r, &rloc, |d, v, loc| repo_name(d, v, loc))
                .and_then(|repos| {
                    if repos.is_empty() {
                        d.error(&rloc, "must list at least one repository");
                        None
                    } else {
                        Some(Checkout::Repos(repos))
                    }
                })
        }
    };
    let github = d.field(m, loc, "GitHub", decode_github);
    let network = d.field(m, loc, "Network", |d, v, loc| {
        let m = d.object(v, loc, &["Allow"])?;
        let allow = d.field(m, loc, "Allow", |d, v, loc| d.list(v, loc, host));
        Some(Network { allow: allow? })
    });
    let usage_limit = d.field(m, loc, "UsageLimit", |d, v, loc| {
        let m = d.object(v, loc, &["Resume"])?;
        let resume = d.field(m, loc, "Resume", |d, v, loc| {
            d.enumeration(
                v,
                loc,
                &[("auto", ResumeMode::Auto), ("manual", ResumeMode::Manual)],
            )
        });
        Some(UsageLimit { resume: resume? })
    });
    let workspace = d.field(m, loc, "Workspace", |d, v, loc| {
        let m = d.object(v, loc, &["Compute", "Architecture", "IdleSeconds"])?;
        let compute = d.field(m, loc, "Compute", |d, v, loc| {
            d.enumeration(
                v,
                loc,
                &[("standard", Compute::Standard), ("docker", Compute::Docker)],
            )
        });
        let architecture = d.field(m, loc, "Architecture", |d, v, loc| {
            d.enumeration(
                v,
                loc,
                &[
                    ("arm64", Architecture::Arm64),
                    ("x86_64", Architecture::X86_64),
                ],
            )
        });
        let idle_seconds = d.field(m, loc, "IdleSeconds", |d, v, loc| d.positive(v, loc));
        Some(Workspace {
            compute: compute?,
            architecture: architecture?,
            idle_seconds: idle_seconds?,
        })
    });
    Some(Session {
        worker: worker?,
        checkout: checkout?,
        github: github?,
        network: network?,
        usage_limit: usage_limit?,
        workspace: workspace?,
    })
}

fn decode_worker(d: &mut Decoder, v: &Value, loc: &str) -> Option<Worker> {
    let m = d.object(v, loc, &["Agent", "Model", "Credential"])?;
    let agent = d.field(m, loc, "Agent", |d, v, loc| {
        d.enumeration(
            v,
            loc,
            &[("claude", Agent::Claude), ("codex", Agent::Codex)],
        )
    });
    let model = d.field(m, loc, "Model", |d, v, loc| d.string(v, loc));
    let credential = d.field(m, loc, "Credential", |d, v, loc| d.string(v, loc));
    Some(Worker {
        agent: agent?,
        model: model?,
        credential: credential?,
    })
}

fn repo_name(d: &mut Decoder, v: &Value, loc: &str) -> Option<String> {
    let s = d.string(v, loc)?;
    if !is_repo_name(&s) {
        d.error(loc, format!("{s:?} must be <owner>/<name>"));
        return None;
    }
    Some(s)
}

fn repo_pattern(d: &mut Decoder, v: &Value, loc: &str) -> Option<String> {
    let s = d.string(v, loc)?;
    let ok = match s.strip_suffix("/*") {
        Some(owner) => is_repo_name(&format!("{owner}/x")),
        None => is_repo_name(&s),
    };
    if !ok {
        d.error(loc, format!("{s:?} must be <owner>/<name> or <owner>/*"));
        return None;
    }
    Some(s)
}

fn host(d: &mut Decoder, v: &Value, loc: &str) -> Option<String> {
    let s = d.string(v, loc)?;
    let name = s.strip_prefix("*.").unwrap_or(&s);
    let ok = name.contains('.')
        && name.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        });
    if !ok {
        d.error(
            loc,
            format!("{s:?} must be a lowercase host name such as github.com or *.example.com"),
        );
        return None;
    }
    Some(s)
}

fn decode_github(d: &mut Decoder, v: &Value, loc: &str) -> Option<GitHubAccess> {
    let m = d.object(v, loc, &["Repos", "Permissions"])?;
    let repos = d.field(m, loc, "Repos", |d, v, loc| {
        let repos = d.list(v, loc, repo_pattern)?;
        if repos.is_empty() {
            d.error(loc, "must list at least one repository");
            return None;
        }
        Some(repos)
    });
    let permissions = d.field(m, loc, "Permissions", |d, v, loc| {
        let map = d.map(v, loc, |d, v, loc| {
            d.enumeration(
                v,
                loc,
                &[
                    ("read", PermissionLevel::Read),
                    ("write", PermissionLevel::Write),
                ],
            )
        })?;
        let mut ok = true;
        for key in map.keys() {
            if !crate::permissions::REPOSITORY_PERMISSIONS.contains(&key.as_str()) {
                d.error(
                    crate::error::at(loc, key),
                    "unknown GitHub repository permission",
                );
                ok = false;
            }
        }
        ok.then(|| map.into_iter().collect())
    });
    Some(GitHubAccess {
        repos: repos?,
        permissions: permissions?,
    })
}

fn decode_trigger(d: &mut Decoder, v: &Value, loc: &str) -> Option<Trigger> {
    let m = d.object(
        v,
        loc,
        &[
            "State",
            "EventPattern",
            "ScheduleExpression",
            "InputTransformer",
            "Start",
        ],
    )?;
    let state = d.field(m, loc, "State", |d, v, loc| {
        d.enumeration(
            v,
            loc,
            &[
                ("ENABLED", TriggerState::Enabled),
                ("DISABLED", TriggerState::Disabled),
            ],
        )
    });
    let source = match (m.get("EventPattern"), m.get("ScheduleExpression")) {
        (Some(_), Some(_)) => {
            d.error(
                loc,
                "exactly one of EventPattern or ScheduleExpression is allowed",
            );
            None
        }
        (None, None) => {
            d.error(
                crate::error::at(loc, "EventPattern"),
                "missing required field; set EventPattern or ScheduleExpression",
            );
            None
        }
        (Some(p), None) => {
            let ploc = crate::error::at(loc, "EventPattern");
            if p.is_object() {
                Some(TriggerSource::EventPattern(p.clone()))
            } else {
                d.error(ploc, "expected a mapping");
                None
            }
        }
        (None, Some(s)) => {
            let sloc = crate::error::at(loc, "ScheduleExpression");
            d.string(s, &sloc)
                .and_then(|s| match crate::schedule::parse(&s) {
                    Ok(_) => Some(TriggerSource::ScheduleExpression(s)),
                    Err(e) => {
                        d.error(sloc, e.to_string());
                        None
                    }
                })
        }
    };
    let input_transformer = d.optional(m, loc, "InputTransformer", |d, v, loc| {
        let m = d.object(v, loc, &["InputPathsMap", "InputTemplate"])?;
        let input_paths_map = d.field(m, loc, "InputPathsMap", |d, v, loc| {
            d.map(v, loc, |d, v, loc| {
                let s = d.string(v, loc)?;
                if !s.starts_with('$') {
                    d.error(loc, "must be a JSONPath starting with $");
                    return None;
                }
                Some(s)
            })
        });
        let input_template = d.field(m, loc, "InputTemplate", |d, v, loc| d.string(v, loc));
        Some(InputTransformer {
            input_paths_map: input_paths_map?,
            input_template: input_template?,
        })
    });
    let start = d.field(m, loc, "Start", decode_start);
    Some(Trigger {
        state: state?,
        source: source?,
        input_transformer: input_transformer?,
        start: start?,
    })
}

fn decode_start(d: &mut Decoder, v: &Value, loc: &str) -> Option<Start> {
    let m = d.object(
        v,
        loc,
        &[
            "Session",
            "Instruction",
            "Skills",
            "RequiredSkills",
            "MaxNudges",
            "Limits",
        ],
    )?;
    let session = d.field(m, loc, "Session", |d, v, loc| d.string(v, loc));
    let instruction = d.field(m, loc, "Instruction", |d, v, loc| d.string(v, loc));
    let skills = d.field(m, loc, "Skills", |d, v, loc| d.strings(v, loc));
    let required_skills = d.field(m, loc, "RequiredSkills", |d, v, loc| d.strings(v, loc));
    let max_nudges = d.field(m, loc, "MaxNudges", |d, v, loc| d.u64(v, loc));
    let limits = d.field(m, loc, "Limits", |d, v, loc| {
        let m = d.object(
            v,
            loc,
            &[
                "Turns",
                "CostUsd",
                "TimeoutSeconds",
                "HeartbeatSeconds",
                "Children",
            ],
        )?;
        let turns = d.field(m, loc, "Turns", |d, v, loc| d.positive(v, loc));
        let cost_usd = d.field(m, loc, "CostUsd", |d, v, loc| d.f64(v, loc));
        let timeout_seconds = d.field(m, loc, "TimeoutSeconds", |d, v, loc| d.positive(v, loc));
        let heartbeat_seconds = d.field(m, loc, "HeartbeatSeconds", |d, v, loc| d.positive(v, loc));
        let children = d.field(m, loc, "Children", |d, v, loc| {
            let m = d.object(v, loc, &["MaxDepth", "MaxCount"])?;
            let max_depth = d.field(m, loc, "MaxDepth", |d, v, loc| d.u64(v, loc));
            let max_count = d.field(m, loc, "MaxCount", |d, v, loc| d.u64(v, loc));
            Some(ChildLimits {
                max_depth: max_depth?,
                max_count: max_count?,
            })
        });
        Some(Limits {
            turns: turns?,
            cost_usd: cost_usd?,
            timeout_seconds: timeout_seconds?,
            heartbeat_seconds: heartbeat_seconds?,
            children: children?,
        })
    });
    Some(Start {
        session: session?,
        instruction: instruction?,
        skills: skills?,
        required_skills: required_skills?,
        max_nudges: max_nudges?,
        limits: limits?,
    })
}
