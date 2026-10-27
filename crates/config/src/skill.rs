//! Types for `skill.yaml`, the engine-executed part of a skill.
//!
//! The shape follows Amazon States Language (ASL). Fields that ASL makes optional but
//! sanchoris requires to be explicit (`TimeoutSeconds`, every `Retry` field) are
//! non-optional here so that a missing value fails to parse.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct SkillDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub inputs: BTreeMap<String, InputType>,
    pub start_at: String,
    pub timeout_seconds: u64,
    pub states: IndexMap<String, State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<BTreeMap<String, LayoutPoint>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputType {
    String,
    Number,
    Boolean,
    Array,
    Object,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "Type", deny_unknown_fields)]
pub enum State {
    Task(TaskState),
    Pass(PassState),
    Choice(ChoiceState),
    Wait(WaitState),
    Succeed(SucceedState),
    Fail(FailState),
}

impl State {
    pub fn type_name(&self) -> &'static str {
        match self {
            State::Task(_) => "Task",
            State::Pass(_) => "Pass",
            State::Choice(_) => "Choice",
            State::Wait(_) => "Wait",
            State::Succeed(_) => "Succeed",
            State::Fail(_) => "Fail",
        }
    }

    /// Names of the states this state can move to, in declaration order.
    pub fn successors(&self) -> Vec<&str> {
        let mut out = Vec::new();
        match self {
            State::Task(s) => {
                out.extend(s.next.as_deref());
                out.extend(s.catch.iter().map(|c| c.next.as_str()));
            }
            State::Pass(s) => out.extend(s.next.as_deref()),
            State::Wait(s) => out.extend(s.next.as_deref()),
            State::Choice(s) => {
                out.extend(s.choices.iter().filter_map(|c| c.next.as_deref()));
                out.extend(s.default.as_deref());
            }
            State::Succeed(_) | State::Fail(_) => {}
        }
        out
    }

    /// Whether the state ends the execution (`End: true`, `Succeed`, or `Fail`).
    pub fn is_terminal(&self) -> bool {
        match self {
            State::Task(s) => s.end == Some(true),
            State::Pass(s) => s.end == Some(true),
            State::Wait(s) => s.end == Some(true),
            State::Choice(_) => false,
            State::Succeed(_) | State::Fail(_) => true,
        }
    }
}

/// `ResultPath: null` discards the result; an absent `ResultPath` means `$`.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ResultPath {
    #[default]
    Root,
    Discard,
    Path(String),
}

pub(crate) mod result_path {
    use super::ResultPath;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<ResultPath, D::Error> {
        Ok(match Option::<String>::deserialize(d)? {
            None => ResultPath::Discard,
            Some(p) if p == "$" => ResultPath::Root,
            Some(p) => ResultPath::Path(p),
        })
    }

    pub fn serialize<S: Serializer>(v: &ResultPath, s: S) -> Result<S::Ok, S::Error> {
        match v {
            ResultPath::Root => s.serialize_str("$"),
            ResultPath::Discard => s.serialize_none(),
            ResultPath::Path(p) => s.serialize_str(p),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct TaskState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub resource: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    #[serde(default, with = "result_path")]
    pub result_path: ResultPath,
    pub timeout_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retry: Vec<Retrier>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub catch: Vec<Catcher>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct PassState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, with = "result_path")]
    pub result_path: ResultPath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct WaitState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct ChoiceState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub choices: Vec<ChoiceRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// A choice rule. Top-level rules carry `Next`; nested rules inside `And`/`Or`/`Not`
/// must not. A rule holds either one comparison (`Variable` plus exactly one operator)
/// or one boolean combinator.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct ChoiceRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub and: Option<Vec<ChoiceRule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub or: Option<Vec<ChoiceRule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not: Option<Box<ChoiceRule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub string_equals: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub string_less_than: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub string_greater_than: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub string_matches: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric_equals: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric_less_than: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric_less_than_equals: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric_greater_than: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric_greater_than_equals: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boolean_equals: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_present: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_null: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_string: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_numeric: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_boolean: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct SucceedState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct FailState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub error: String,
    pub cause: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct Retrier {
    pub error_equals: Vec<String>,
    pub interval_seconds: u64,
    pub max_attempts: u32,
    pub backoff_rate: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_delay_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct Catcher {
    pub error_equals: Vec<String>,
    pub next: String,
    #[serde(default, with = "result_path")]
    pub result_path: ResultPath,
}

/// Error names defined by ASL.
pub mod errors {
    pub const ALL: &str = "States.ALL";
    pub const TIMEOUT: &str = "States.Timeout";
    pub const TASK_FAILED: &str = "States.TaskFailed";
    pub const HEARTBEAT_TIMEOUT: &str = "States.HeartbeatTimeout";
    pub const NO_CHOICE_MATCHED: &str = "States.NoChoiceMatched";
}

pub fn parse_skill_yaml(text: &str) -> Result<SkillDefinition, serde_yaml_ng::Error> {
    serde_yaml_ng::from_str(text)
}
