use crate::config::{
    Checkout, Compute, Config, PermissionLevel, Session, Trigger, TriggerSource, is_repo_name,
};
use crate::error::{ValidationError, ValidationErrors, at, index};
use crate::repository::{ConfigFiles, ConfigRepository, Skill, parse_repository};
use crate::resource::parse_resource;
use crate::skill::{ChoiceRule, Retrier, SkillDefinition, State, TaskState, errors};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet, VecDeque};

#[derive(Debug, Clone, PartialEq)]
pub struct PlanLimits {
    pub max_turns: u64,
    pub max_cost_usd: f64,
    pub max_timeout_seconds: u64,
    pub max_idle_seconds: u64,
    pub max_children_depth: u64,
    pub max_children_count: u64,
    pub computes: BTreeSet<Compute>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidationContext {
    pub credentials: BTreeSet<String>,
    pub plan: PlanLimits,
}

pub const MAX_BACKOFF_RATE: f64 = 10.0;

pub const KNOWN_STATES_ERRORS: &[&str] = &[
    errors::ALL,
    errors::TIMEOUT,
    errors::TASK_FAILED,
    errors::HEARTBEAT_TIMEOUT,
    errors::NO_CHOICE_MATCHED,
];

pub const EVENT_TYPES: &[(&str, &[&str])] = &[
    (
        "github",
        &[
            "Issue Labeled",
            "Issue Comment Created",
            "Check Run Completed",
        ],
    ),
    ("slack", &["Message"]),
    ("web", &["Message"]),
];

pub const GITHUB_ACTOR_PATHS: &[&[&str]] = &[
    &["sender", "login"],
    &["issue", "author_association"],
    &["comment", "author_association"],
];

pub fn github_actor_conditions(detail_type: &str) -> &'static [&'static [&'static str]] {
    match detail_type {
        "Issue Labeled" => &[&["sender", "login"], &["issue", "author_association"]],
        "Issue Comment Created" => &[&["sender", "login"], &["comment", "author_association"]],
        "Check Run Completed" => &[&["sender", "login"]],
        _ => &[],
    }
}

pub fn is_exact_values(value: &Value) -> bool {
    matches!(value, Value::Array(items) if !items.is_empty() && items.iter().all(Value::is_string))
}

fn at_path<'a>(detail: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(detail, |v, key| v.get(*key))
        .filter(|v| !v.is_null())
}

pub fn constrains_actor(detail: &Value, detail_type: &str) -> bool {
    github_actor_conditions(detail_type)
        .iter()
        .any(|path| at_path(detail, path).is_some_and(is_exact_values))
}

fn actor_description(detail_type: &str) -> String {
    github_actor_conditions(detail_type)
        .iter()
        .map(|path| format!("detail.{}", path.join(".")))
        .collect::<Vec<_>>()
        .join(" or ")
}

pub fn load_repository(
    files: &ConfigFiles,
    context: &ValidationContext,
) -> Result<ConfigRepository, ValidationErrors> {
    let repository = parse_repository(files)?;
    validate(&repository, context)?;
    Ok(repository)
}

pub fn validate(
    repository: &ConfigRepository,
    context: &ValidationContext,
) -> Result<(), ValidationErrors> {
    let mut errors = Vec::new();
    let mut v = Validator {
        repository,
        context,
        errors: &mut errors,
    };
    v.config(&repository.config);
    for (name, skill) in &repository.skills {
        if let Some(definition) = &skill.definition {
            v.errors.extend(validate_skill_definition(
                definition,
                &at("Skills", name),
                context.plan.max_timeout_seconds,
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors(errors))
    }
}

struct Validator<'a> {
    repository: &'a ConfigRepository,
    context: &'a ValidationContext,
    errors: &'a mut Vec<ValidationError>,
}

fn covers(patterns: &[String], repo: &str) -> bool {
    patterns.iter().any(|p| match p.strip_suffix("/*") {
        Some(owner) => repo
            .split_once('/')
            .is_some_and(|(o, _)| o.eq_ignore_ascii_case(owner)),
        None => p.eq_ignore_ascii_case(repo),
    })
}

fn level_name(level: PermissionLevel) -> &'static str {
    match level {
        PermissionLevel::Read => "read",
        PermissionLevel::Write => "write",
    }
}

impl Validator<'_> {
    fn error(&mut self, location: impl Into<String>, message: impl Into<String>) {
        self.errors.push(ValidationError::new(location, message));
    }

    fn config(&mut self, config: &Config) {
        for (name, session) in &config.sessions {
            self.session(config, name, session);
        }
        for (name, trigger) in &config.triggers {
            self.trigger(config, name, trigger);
        }
    }

    fn resolve_repo(&self, config: &Config, session: &Session, name: &str) -> Option<String> {
        if is_repo_name(name) {
            return Some(name.to_string());
        }
        let Checkout::Project(project) = &session.checkout else {
            return None;
        };
        config
            .projects
            .get(project)?
            .repos
            .get(name)?
            .full_name()
            .map(str::to_string)
    }

    fn session(&mut self, config: &Config, name: &str, session: &Session) {
        let loc = at("Sessions", name);
        let credential = &session.worker.credential;
        if !session.worker.uses_requester() && !self.context.credentials.contains(credential) {
            self.error(
                at(&at(&loc, "Worker"), "Credential"),
                format!("unknown credential {credential:?}; use a credential name from Settings or requester"),
            );
        }
        let cloned: Vec<(String, String)> = match &session.checkout {
            Checkout::Project(project) => match config.projects.get(project) {
                Some(p) => p
                    .repos
                    .iter()
                    .filter_map(|(key, r)| r.full_name().map(|n| (key.clone(), n.to_string())))
                    .collect(),
                None => {
                    self.error(at(&loc, "Project"), format!("unknown project {project:?}"));
                    Vec::new()
                }
            },
            Checkout::Repos(repos) => repos.iter().map(|r| (r.clone(), r.clone())).collect(),
        };
        for (_, repo) in cloned {
            if !covers(&session.github.repos, &repo) {
                self.error(
                    at(&at(&loc, "GitHub"), "Repos"),
                    format!("does not include {repo}, which the session clones"),
                );
            }
        }
        let plan = &self.context.plan;
        let workspace = at(&loc, "Workspace");
        if !plan.computes.contains(&session.workspace.compute) {
            self.error(
                at(&workspace, "Compute"),
                format!(
                    "compute {} is not available on the plan",
                    session.workspace.compute.as_str()
                ),
            );
        }
        if session.workspace.idle_seconds > plan.max_idle_seconds {
            self.error(
                at(&workspace, "IdleSeconds"),
                format!(
                    "{} exceeds the plan limit of {}",
                    session.workspace.idle_seconds, plan.max_idle_seconds
                ),
            );
        }
    }

    fn trigger(&mut self, config: &Config, name: &str, trigger: &Trigger) {
        let loc = at("Triggers", name);
        let start = at(&loc, "Start");
        let session = config.sessions.get(&trigger.start.session);
        if session.is_none() {
            self.error(
                at(&start, "Session"),
                format!("unknown session {:?}", trigger.start.session),
            );
        }

        match &trigger.source {
            TriggerSource::EventPattern(pattern) => {
                self.event_pattern(config, session, &at(&loc, "EventPattern"), pattern)
            }
            TriggerSource::ScheduleExpression(_) => {}
        }

        self.placeholders(trigger, &loc);

        let skills_loc = at(&start, "Skills");
        let mut seen = HashSet::new();
        for (i, skill_name) in trigger.start.skills.iter().enumerate() {
            let sloc = index(&skills_loc, i);
            if !seen.insert(skill_name) {
                self.error(&sloc, format!("skill {skill_name:?} is listed twice"));
                continue;
            }
            match self.repository.skill(skill_name) {
                None => self.error(&sloc, format!("unknown skill {skill_name:?}")),
                Some(skill) => {
                    if let Some(session) = session {
                        self.skill_access(config, &trigger.start.session, session, skill, &sloc);
                    }
                }
            }
        }

        let required_loc = at(&start, "RequiredSkills");
        for (i, skill_name) in trigger.start.required_skills.iter().enumerate() {
            let rloc = index(&required_loc, i);
            match self.repository.skill(skill_name) {
                None => self.error(&rloc, format!("unknown skill {skill_name:?}")),
                Some(skill) if !skill.is_reliable() => self.error(
                    &rloc,
                    format!("skill {skill_name:?} has no skill.yaml; only reliable skills can be required"),
                ),
                Some(_) => {}
            }
            if !trigger.start.skills.contains(skill_name) {
                self.error(
                    &rloc,
                    format!("required skill {skill_name:?} must also be listed in Skills"),
                );
            }
        }

        self.limits(trigger, &at(&start, "Limits"));
    }

    fn limits(&mut self, trigger: &Trigger, loc: &str) {
        let limits = &trigger.start.limits;
        let plan = &self.context.plan;
        let mut over = Vec::new();
        if limits.turns > plan.max_turns {
            over.push((
                at(loc, "Turns"),
                format!(
                    "{} exceeds the plan limit of {}",
                    limits.turns, plan.max_turns
                ),
            ));
        }
        if limits.cost_usd > plan.max_cost_usd {
            over.push((
                at(loc, "CostUsd"),
                format!(
                    "{} exceeds the plan limit of {}",
                    limits.cost_usd, plan.max_cost_usd
                ),
            ));
        }
        if limits.timeout_seconds > plan.max_timeout_seconds {
            over.push((
                at(loc, "TimeoutSeconds"),
                format!(
                    "{} exceeds the plan limit of {}",
                    limits.timeout_seconds, plan.max_timeout_seconds
                ),
            ));
        }
        if limits.heartbeat_seconds > limits.timeout_seconds {
            over.push((
                at(loc, "HeartbeatSeconds"),
                "must not exceed TimeoutSeconds".to_string(),
            ));
        }
        let children = at(loc, "Children");
        if limits.children.max_depth > plan.max_children_depth {
            over.push((
                at(&children, "MaxDepth"),
                format!(
                    "{} exceeds the plan limit of {}",
                    limits.children.max_depth, plan.max_children_depth
                ),
            ));
        }
        if limits.children.max_count > plan.max_children_count {
            over.push((
                at(&children, "MaxCount"),
                format!(
                    "{} exceeds the plan limit of {}",
                    limits.children.max_count, plan.max_children_count
                ),
            ));
        }
        for (l, m) in over {
            self.error(l, m);
        }
    }

    fn placeholders(&mut self, trigger: &Trigger, loc: &str) {
        let keys: Vec<&str> = trigger
            .input_transformer
            .as_ref()
            .map(|t| t.input_paths_map.keys().map(String::as_str).collect())
            .unwrap_or_default();
        let check = |text: &str, location: String, errors: &mut Vec<ValidationError>| {
            for name in placeholder_names(text) {
                if !keys.contains(&name) {
                    errors.push(ValidationError::new(
                        &location,
                        format!(
                            "placeholder <{name}> is not defined in InputTransformer.InputPathsMap"
                        ),
                    ));
                }
            }
        };
        check(
            &trigger.start.instruction,
            at(&at(loc, "Start"), "Instruction"),
            self.errors,
        );
        if let Some(t) = &trigger.input_transformer {
            check(
                &t.input_template,
                at(&at(loc, "InputTransformer"), "InputTemplate"),
                self.errors,
            );
        }
    }

    fn event_pattern(
        &mut self,
        config: &Config,
        session: Option<&Session>,
        loc: &str,
        pattern: &Value,
    ) {
        let sources = match pattern.get("source").and_then(Value::as_array) {
            Some(items) if !items.is_empty() => {
                let mut out = Vec::new();
                for (i, item) in items.iter().enumerate() {
                    match item.as_str() {
                        Some(s) if EVENT_TYPES.iter().any(|(src, _)| *src == s) => out.push(s),
                        _ => self.error(
                            index(&at(loc, "source"), i),
                            "source must be one of github, slack, web",
                        ),
                    }
                }
                out
            }
            _ => {
                self.error(
                    at(loc, "source"),
                    "missing required field; list the event source (github, slack, or web)",
                );
                return;
            }
        };
        match pattern.get("detail-type").and_then(Value::as_array) {
            Some(items) if !items.is_empty() => {
                for (i, item) in items.iter().enumerate() {
                    let ok = item.as_str().is_some_and(|t| {
                        EVENT_TYPES
                            .iter()
                            .any(|(src, types)| sources.contains(src) && types.contains(&t))
                    });
                    if !ok {
                        self.error(
                            index(&at(loc, "detail-type"), i),
                            "unknown detail-type for the event source",
                        );
                    }
                }
            }
            _ => self.error(at(loc, "detail-type"), "missing required field"),
        }
        let detail_loc = at(loc, "detail");
        let Some(detail) = pattern.get("detail").filter(|d| d.is_object()) else {
            self.error(&detail_loc, "missing required field");
            return;
        };
        let has = |path: &[&str]| {
            path.iter()
                .try_fold(detail, |v, key| v.get(*key))
                .is_some_and(|v| !v.is_null())
        };
        if sources.contains(&"github") {
            for path in GITHUB_ACTOR_PATHS {
                if at_path(detail, path).is_some_and(|v| !is_exact_values(v)) {
                    self.error(
                        path.iter().fold(detail_loc.clone(), |loc, key| at(&loc, key)),
                        "actor conditions must be a list of exact values; exists, prefix, anything-but, and numeric also match actors outside the list",
                    );
                }
            }
            let detail_types = pattern
                .get("detail-type")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|t| !github_actor_conditions(t).is_empty());
            for detail_type in detail_types {
                if !constrains_actor(detail, detail_type) {
                    self.error(
                        &detail_loc,
                        format!(
                            "{detail_type} triggers must constrain the actor of the event with a list of exact values in {}",
                            actor_description(detail_type)
                        ),
                    );
                }
            }
            match detail.get("repository") {
                None | Some(Value::Null) => self.error(
                    at(&detail_loc, "repository"),
                    "missing required field; GitHub triggers must name the repositories",
                ),
                Some(Value::Array(items)) => {
                    for (i, item) in items.iter().enumerate() {
                        let Some(name) = item.as_str() else { continue };
                        let rloc = index(&at(&detail_loc, "repository"), i);
                        let Some(session) = session else { continue };
                        match self.resolve_repo(config, session, name) {
                            None => self.error(
                                rloc,
                                format!("{name:?} is neither owner/name nor a repository of the session's project"),
                            ),
                            Some(repo) if !covers(&session.github.repos, &repo) => self.error(
                                rloc,
                                format!("{repo} is outside the session's GitHub.Repos"),
                            ),
                            Some(_) => {}
                        }
                    }
                }
                Some(_) => {}
            }
        }
        if sources.contains(&"slack") {
            for key in ["channel", "user"] {
                if !has(&[key]) {
                    self.error(
                        at(&detail_loc, key),
                        "missing required field for Slack triggers",
                    );
                }
            }
        }
        if sources.contains(&"web") && !has(&["scope"]) {
            self.error(
                at(&detail_loc, "scope"),
                "missing required field for Web triggers",
            );
        }
    }

    fn skill_access(
        &mut self,
        config: &Config,
        session_name: &str,
        session: &Session,
        skill: &Skill,
        loc: &str,
    ) {
        let Some(definition) = &skill.definition else {
            return;
        };
        for (state_name, state) in &definition.states {
            let State::Task(task) = state else { continue };
            let Ok(resource) = parse_resource(&task.resource) else {
                continue;
            };
            for (permission, level) in resource.permissions() {
                let granted = session.github.permissions.get(*permission);
                if granted.is_none_or(|g| g < level) {
                    self.error(
                        loc,
                        format!(
                            "skill {} state {state_name} uses {} which needs {permission}: {}, but session {session_name} grants {}",
                            skill.name,
                            task.resource,
                            level_name(*level),
                            granted.map_or("nothing", |g| level_name(*g)),
                        ),
                    );
                }
            }
            for name in literal_repos(task) {
                match self.resolve_repo(config, session, &name) {
                    None => self.error(
                        loc,
                        format!(
                            "skill {} state {state_name} names repository {name:?}, which is neither owner/name nor a repository of session {session_name}'s project",
                            skill.name
                        ),
                    ),
                    Some(repo) if !covers(&session.github.repos, &repo) => self.error(
                        loc,
                        format!(
                            "skill {} state {state_name} uses {repo}, which is outside session {session_name}'s GitHub.Repos",
                            skill.name
                        ),
                    ),
                    Some(_) => {}
                }
            }
        }
    }
}

fn placeholder_names(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        match after.find('>') {
            Some(close) => {
                let name = &after[..close];
                if !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    out.push(name);
                }
                rest = &after[close + 1..];
            }
            None => break,
        }
    }
    out
}

fn literal_repos(task: &TaskState) -> Vec<String> {
    let Some(Value::Object(params)) = &task.parameters else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(Value::Array(items)) = params.get("Repos") {
        out.extend(items.iter().filter_map(Value::as_str).map(str::to_string));
    }
    if let Some(Value::String(s)) = params.get("Repo") {
        out.push(s.clone());
    }
    out
}

pub fn validate_skill_definition(
    definition: &SkillDefinition,
    loc: &str,
    max_seconds: u64,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let mut err = |l: String, m: String| errors.push(ValidationError::new(l, m));
    if definition.timeout_seconds == 0 {
        err(at(loc, "TimeoutSeconds"), "must be greater than 0".into());
    }
    at_most(
        &at(loc, "TimeoutSeconds"),
        definition.timeout_seconds,
        max_seconds,
        &mut err,
    );
    if definition.states.is_empty() {
        err(at(loc, "States"), "must define at least one state".into());
    }
    if !definition.states.contains_key(&definition.start_at) {
        err(
            at(loc, "StartAt"),
            format!("unknown state {:?}", definition.start_at),
        );
    }
    let states_loc = at(loc, "States");
    for (name, state) in &definition.states {
        let sloc = at(&states_loc, name);
        let mut targets: Vec<(String, &str)> = Vec::new();
        match state {
            State::Task(t) => {
                next_or_end(&sloc, t.next.as_deref(), t.end, &mut err);
                targets.extend(t.next.as_deref().map(|n| (at(&sloc, "Next"), n)));
                task_state(&sloc, t, max_seconds, &mut err);
                for (i, c) in t.catch.iter().enumerate() {
                    targets.push((at(&index(&at(&sloc, "Catch"), i), "Next"), c.next.as_str()));
                }
            }
            State::Pass(p) => {
                next_or_end(&sloc, p.next.as_deref(), p.end, &mut err);
                targets.extend(p.next.as_deref().map(|n| (at(&sloc, "Next"), n)));
            }
            State::Wait(w) => {
                next_or_end(&sloc, w.next.as_deref(), w.end, &mut err);
                targets.extend(w.next.as_deref().map(|n| (at(&sloc, "Next"), n)));
                match (w.seconds, &w.timestamp) {
                    (Some(_), Some(_)) | (None, None) => err(
                        sloc.clone(),
                        "Wait needs exactly one of Seconds or Timestamp".into(),
                    ),
                    _ => {}
                }
                if let Some(seconds) = w.seconds {
                    at_most(&at(&sloc, "Seconds"), seconds, max_seconds, &mut err);
                }
            }
            State::Choice(c) => {
                let cloc = at(&sloc, "Choices");
                if c.choices.is_empty() {
                    err(cloc.clone(), "must have at least one rule".into());
                }
                for (i, rule) in c.choices.iter().enumerate() {
                    let rloc = index(&cloc, i);
                    choice_rule(&rloc, rule, true, &mut err);
                    if let Some(n) = rule.next.as_deref() {
                        targets.push((at(&rloc, "Next"), n));
                    }
                }
                targets.extend(c.default.as_deref().map(|n| (at(&sloc, "Default"), n)));
            }
            State::Succeed(_) => {}
            State::Fail(f) => error_name(&at(&sloc, "Error"), &f.error, false, &mut err),
        }
        for (tloc, target) in targets {
            if !definition.states.contains_key(target) {
                err(tloc, format!("unknown state {target:?}"));
            }
        }
    }

    if definition.states.contains_key(&definition.start_at) {
        let mut reached = HashSet::new();
        let mut queue = VecDeque::from([definition.start_at.as_str()]);
        while let Some(name) = queue.pop_front() {
            if !reached.insert(name) {
                continue;
            }
            if let Some(state) = definition.states.get(name) {
                queue.extend(state.successors());
            }
        }
        for name in definition.states.keys() {
            if !reached.contains(name.as_str()) {
                err(
                    at(&states_loc, name),
                    format!("unreachable from StartAt {:?}", definition.start_at),
                );
            }
        }
    }
    if !definition.states.values().any(State::is_terminal) {
        err(
            states_loc.clone(),
            "must have a terminal state (End: true, Succeed, or Fail)".into(),
        );
    }
    if let Some(layout) = &definition.layout {
        for name in layout.keys() {
            if !definition.states.contains_key(name) {
                err(
                    at(&at(loc, "Layout"), name),
                    format!("unknown state {name:?}"),
                );
            }
        }
    }
    errors
}

fn next_or_end(
    loc: &str,
    next: Option<&str>,
    end: Option<bool>,
    err: &mut impl FnMut(String, String),
) {
    match (next, end) {
        (Some(_), None) | (None, Some(true)) => {}
        (None, Some(false)) => err(at(loc, "End"), "must be true when present".into()),
        (Some(_), Some(_)) => err(loc.to_string(), "set exactly one of Next or End".into()),
        (None, None) => err(
            at(loc, "Next"),
            "missing required field; set Next or End: true".into(),
        ),
    }
}

fn at_most(loc: &str, value: u64, max: u64, err: &mut impl FnMut(String, String)) {
    if value > max {
        err(
            loc.to_string(),
            format!("{value} exceeds the plan limit of {max}"),
        );
    }
}

fn task_state(loc: &str, task: &TaskState, max_seconds: u64, err: &mut impl FnMut(String, String)) {
    if task.timeout_seconds == 0 {
        err(at(loc, "TimeoutSeconds"), "must be greater than 0".into());
    }
    at_most(
        &at(loc, "TimeoutSeconds"),
        task.timeout_seconds,
        max_seconds,
        err,
    );
    if let Some(h) = task.heartbeat_seconds
        && (h == 0 || h >= task.timeout_seconds)
    {
        err(
            at(loc, "HeartbeatSeconds"),
            "must be greater than 0 and less than TimeoutSeconds".into(),
        );
    }
    let rloc = at(loc, "Resource");
    match parse_resource(&task.resource) {
        Err(e) => err(rloc, e.to_string()),
        Ok(resource) => {
            if resource.service == "agent" && task.heartbeat_seconds.is_none() {
                err(
                    at(loc, "HeartbeatSeconds"),
                    format!("missing required field for {}", task.resource),
                );
            }
            let ploc = at(loc, "Parameters");
            let params = match &task.parameters {
                Some(Value::Object(m)) => Some(m),
                Some(_) => {
                    err(ploc.clone(), "expected a mapping".into());
                    None
                }
                None => None,
            };
            let keys: Vec<(&str, &str)> = params
                .map(|m| {
                    m.keys()
                        .map(|k| (k.as_str(), k.strip_suffix(".$").unwrap_or(k)))
                        .collect()
                })
                .unwrap_or_default();
            for (raw, key) in &keys {
                if !resource.accepts_parameter(key) {
                    err(
                        at(&ploc, raw),
                        format!("unknown parameter for {}", task.resource),
                    );
                }
            }
            for required in resource.required_parameters {
                if !keys.iter().any(|(_, k)| k == required) {
                    let message = if resource.repo_parameter() == Some(*required) {
                        format!(
                            "missing required field; {} must name its repository",
                            task.resource
                        )
                    } else {
                        "missing required field".to_string()
                    };
                    err(at(&ploc, required), message);
                }
            }
            if let Some(m) = params
                && keys.iter().any(|(_, k)| *k == "TaskToken")
                && m.get("TaskToken.$").and_then(Value::as_str) != Some("$$.Task.Token")
            {
                err(at(&ploc, "TaskToken.$"), "must be $$.Task.Token".into());
            }
        }
    }
    retriers(&at(loc, "Retry"), &task.retry, max_seconds, err);
    let catch_lists: Vec<&Vec<String>> = task.catch.iter().map(|c| &c.error_equals).collect();
    error_lists(&at(loc, "Catch"), &catch_lists, err);
}

fn retriers(loc: &str, retry: &[Retrier], max_seconds: u64, err: &mut impl FnMut(String, String)) {
    for (i, r) in retry.iter().enumerate() {
        let rloc = index(loc, i);
        if r.interval_seconds == 0 {
            err(
                at(&rloc, "IntervalSeconds"),
                "must be greater than 0".into(),
            );
        }
        at_most(
            &at(&rloc, "IntervalSeconds"),
            r.interval_seconds,
            max_seconds,
            err,
        );
        if !(1.0..=MAX_BACKOFF_RATE).contains(&r.backoff_rate) {
            err(
                at(&rloc, "BackoffRate"),
                format!("must be between 1.0 and {MAX_BACKOFF_RATE}"),
            );
        }
        if r.max_delay_seconds == Some(0) {
            err(
                at(&rloc, "MaxDelaySeconds"),
                "must be greater than 0".into(),
            );
        }
        if let Some(max_delay) = r.max_delay_seconds {
            at_most(&at(&rloc, "MaxDelaySeconds"), max_delay, max_seconds, err);
        }
    }
    let lists: Vec<&Vec<String>> = retry.iter().map(|r| &r.error_equals).collect();
    error_lists(loc, &lists, err);
}

fn error_lists(loc: &str, lists: &[&Vec<String>], err: &mut impl FnMut(String, String)) {
    let last = lists.len().saturating_sub(1);
    for (i, list) in lists.iter().enumerate() {
        let eloc = at(&index(loc, i), "ErrorEquals");
        if list.is_empty() {
            err(eloc.clone(), "must list at least one error".into());
        }
        for (j, name) in list.iter().enumerate() {
            error_name(&index(&eloc, j), name, true, err);
        }
        if list.iter().any(|e| e == errors::ALL) && (list.len() > 1 || i != last) {
            err(
                eloc,
                "States.ALL must be the only error in the last entry".into(),
            );
        }
    }
}

fn error_name(loc: &str, name: &str, allow_all: bool, err: &mut impl FnMut(String, String)) {
    if name.is_empty() {
        err(loc.to_string(), "must not be empty".into());
    } else if name.starts_with("States.")
        && (!KNOWN_STATES_ERRORS.contains(&name) || (!allow_all && name == errors::ALL))
    {
        err(
            loc.to_string(),
            format!(
                "{name:?} is not a known States error; custom error names must not start with States."
            ),
        );
    }
}

fn choice_rule(loc: &str, rule: &ChoiceRule, top: bool, err: &mut impl FnMut(String, String)) {
    if top && rule.next.is_none() {
        err(at(loc, "Next"), "missing required field".into());
    }
    if !top && rule.next.is_some() {
        err(at(loc, "Next"), "nested rules must not have Next".into());
    }
    let boolean = [rule.and.is_some(), rule.or.is_some(), rule.not.is_some()]
        .iter()
        .filter(|b| **b)
        .count();
    let comparisons = [
        rule.string_equals.is_some(),
        rule.string_less_than.is_some(),
        rule.string_greater_than.is_some(),
        rule.string_matches.is_some(),
        rule.numeric_equals.is_some(),
        rule.numeric_less_than.is_some(),
        rule.numeric_less_than_equals.is_some(),
        rule.numeric_greater_than.is_some(),
        rule.numeric_greater_than_equals.is_some(),
        rule.boolean_equals.is_some(),
        rule.is_present.is_some(),
        rule.is_null.is_some(),
        rule.is_string.is_some(),
        rule.is_numeric.is_some(),
        rule.is_boolean.is_some(),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    if boolean > 0 {
        if boolean + comparisons > 1 {
            err(
                loc.to_string(),
                "a rule must have exactly one operator".into(),
            );
        }
        if rule.variable.is_some() {
            err(
                at(loc, "Variable"),
                "And, Or, and Not rules must not have Variable".into(),
            );
        }
        for (key, list) in [("And", &rule.and), ("Or", &rule.or)] {
            if let Some(list) = list {
                let lloc = at(loc, key);
                if list.is_empty() {
                    err(lloc.clone(), "must have at least one rule".into());
                }
                for (i, r) in list.iter().enumerate() {
                    choice_rule(&index(&lloc, i), r, false, err);
                }
            }
        }
        if let Some(r) = &rule.not {
            choice_rule(&at(loc, "Not"), r, false, err);
        }
    } else {
        match &rule.variable {
            None => err(at(loc, "Variable"), "missing required field".into()),
            Some(v) if !v.starts_with('$') => {
                err(at(loc, "Variable"), "must be a path starting with $".into())
            }
            Some(_) => {}
        }
        if comparisons != 1 {
            err(
                loc.to_string(),
                "a rule must have exactly one comparison operator".into(),
            );
        }
    }
}
