use std::collections::{BTreeMap, BTreeSet};

use async_graphql::{Context, ID, Object, Result};
use chrono::{DateTime, Duration, Utc};
use sanchoris_config::builtin_skill;
use sanchoris_config::skill::{ChoiceRule, SkillDefinition, State};
use sanchoris_core::db::TenantTx;
use sanchoris_core::item::format_time;
use sanchoris_core::items::{SkillDef, SkillRunItem, StateEntryItem, Task as TaskItem};
use sanchoris_core::keys;
use sanchoris_core::model::{SkillRunStatus as CoreRunStatus, TaskStatus as CoreTaskStatus};
use sanchoris_core::store::{Index, Listing, SortKey};
use serde_json::Value;
use uuid::Uuid;

use crate::config::{ChangeRequest, Config, active_revision, config_rows, edit_url};
use crate::context::{Scope, now, tx};
use crate::enums::{
    ConfigObjectKind, SkillKind, SkillRunStatus, StateEntryStatus, StateType, TransitionKind,
};
use crate::error::not_found;
use crate::scalars::{Timestamp, ts, ts_opt};
use crate::task::{Output, Run, Task};
use crate::tenant::midnight;
use crate::trigger::Trigger;

pub(crate) struct Skill {
    scope: Scope,
    id: String,
    config_id: Option<Uuid>,
    name: String,
    kind: SkillKind,
    description: String,
    skill_md: String,
    skill_yaml: Option<String>,
    definition: Option<SkillDefinition>,
}

fn parse_definition(name: &str, value: Option<Value>) -> Result<Option<SkillDefinition>> {
    value
        .map(|value| {
            serde_json::from_value::<SkillDefinition>(value).map_err(|e| {
                async_graphql::Error::new(format!("skill {name} has an invalid definition: {e}"))
            })
        })
        .transpose()
}

impl Skill {
    fn from_row(scope: Scope, row: SkillDef) -> Result<Self> {
        Ok(Self {
            scope,
            id: row.id.to_string(),
            config_id: Some(row.config_id),
            kind: SkillKind::from_db(&row.kind)?,
            definition: parse_definition(&row.name, row.definition)?,
            name: row.name,
            description: row.description,
            skill_md: row.skill_md,
            skill_yaml: row.skill_yaml,
        })
    }

    async fn rows(
        ctx: &Context<'_>,
        scope: Scope,
        config_id: Option<Uuid>,
        names: Option<&[String]>,
    ) -> Result<Vec<Self>> {
        let mut tx = tx(ctx, scope.tenant_id);
        let mut rows = Vec::new();
        for config in config_rows(ctx, scope).await? {
            if config_id.is_some_and(|id| id != config.id) {
                continue;
            }
            let Some(revision_id) = config.active_revision_id else {
                continue;
            };
            rows.extend(
                tx.query::<SkillDef>(
                    &keys::config_pk(scope.tenant_id, config.id),
                    &keys::skills_prefix(revision_id),
                )
                .await?
                .into_iter()
                .filter(|row| names.is_none_or(|names| names.contains(&row.name))),
            );
        }
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        rows.into_iter()
            .map(|row| Self::from_row(scope, row))
            .collect()
    }

    pub async fn all(ctx: &Context<'_>, scope: Scope) -> Result<Vec<Self>> {
        let mut seen = BTreeSet::new();
        Ok(Self::rows(ctx, scope, None, None)
            .await?
            .into_iter()
            .filter(|skill| seen.insert(skill.name.clone()))
            .collect())
    }

    pub async fn by_name(ctx: &Context<'_>, scope: Scope, name: &str) -> Result<Option<Self>> {
        Ok(Self::rows(ctx, scope, None, Some(&[name.to_string()]))
            .await?
            .into_iter()
            .next())
    }

    pub async fn by_names(
        ctx: &Context<'_>,
        scope: Scope,
        config_id: Uuid,
        names: &[String],
    ) -> Result<Vec<Self>> {
        let found = Self::rows(ctx, scope, Some(config_id), Some(names)).await?;
        names
            .iter()
            .map(|name| {
                found
                    .iter()
                    .find(|skill| &skill.name == name)
                    .map(Skill::clone_shallow)
                    .ok_or_else(|| not_found(format!("skill {name}")))
            })
            .collect()
    }

    pub async fn for_run(
        ctx: &Context<'_>,
        scope: Scope,
        config_id: Uuid,
        name: &str,
        definition: &Value,
    ) -> Result<Self> {
        let row = match active_revision(ctx, scope, config_id).await? {
            Some(revision_id) => {
                tx(ctx, scope.tenant_id)
                    .get::<SkillDef>(&keys::skill_key(
                        scope.tenant_id,
                        config_id,
                        revision_id,
                        name,
                    ))
                    .await?
            }
            None => None,
        };
        if let Some(row) = row {
            return Self::from_row(scope, row);
        }
        Ok(Self {
            scope,
            id: format!("{config_id}/{name}"),
            config_id: None,
            name: name.to_string(),
            kind: SkillKind::Reliable,
            description: String::new(),
            skill_md: String::new(),
            skill_yaml: None,
            definition: parse_definition(name, Some(definition.clone()))?,
        })
    }

    fn clone_shallow(&self) -> Self {
        Self {
            scope: self.scope,
            id: self.id.clone(),
            config_id: self.config_id,
            name: self.name.clone(),
            kind: self.kind,
            description: self.description.clone(),
            skill_md: self.skill_md.clone(),
            skill_yaml: self.skill_yaml.clone(),
            definition: self.definition.clone(),
        }
    }

    fn is_builtin(&self) -> bool {
        builtin_skill(&self.name).is_some_and(|builtin| {
            builtin.skill_md == self.skill_md && builtin.skill_yaml == self.skill_yaml
        })
    }

    async fn run_rows(
        &self,
        ctx: &Context<'_>,
        since: Option<DateTime<Utc>>,
        limit: Option<usize>,
    ) -> Result<Vec<SkillRunItem>> {
        let mut listing = Listing::index(
            Index::Gsi1,
            keys::skill_runs_index(self.scope.tenant_id, &self.name),
        )
        .descending();
        if let Some(since) = since {
            listing = listing.sort(SortKey::AtLeast(format_time(since)));
        }
        if let Some(limit) = limit {
            listing = listing.limit(limit);
        }
        Ok(tx(ctx, self.scope.tenant_id)
            .list::<SkillRunItem>(&listing, |_| true)
            .await?)
    }
}

#[Object]
impl Skill {
    async fn id(&self) -> ID {
        ID(self.id.clone())
    }

    async fn name(&self) -> &str {
        &self.name
    }

    async fn kind(&self) -> SkillKind {
        self.kind
    }

    async fn description(&self) -> &str {
        &self.description
    }

    async fn comment(&self) -> Option<&str> {
        self.definition.as_ref()?.comment.as_deref()
    }

    async fn inputs(&self) -> Result<Vec<SkillInput>> {
        let Some(definition) = &self.definition else {
            return Ok(Vec::new());
        };
        definition
            .inputs
            .iter()
            .map(|(name, kind)| {
                Ok(SkillInput {
                    name: name.clone(),
                    kind: serde_json::to_value(kind)?
                        .as_str()
                        .expect("input types serialize as strings")
                        .to_string(),
                })
            })
            .collect()
    }

    async fn timeout_seconds(&self) -> Result<Option<i32>> {
        self.definition
            .as_ref()
            .map(|definition| Ok(i32::try_from(definition.timeout_seconds)?))
            .transpose()
    }

    async fn skill_md(&self) -> &str {
        &self.skill_md
    }

    async fn skill_yaml(&self) -> Option<&str> {
        self.skill_yaml.as_deref()
    }

    async fn start_at(&self) -> Option<&str> {
        self.definition
            .as_ref()
            .map(|definition| definition.start_at.as_str())
    }

    async fn states(&self, ctx: &Context<'_>) -> Result<Vec<SkillState>> {
        let Some(definition) = &self.definition else {
            return Ok(Vec::new());
        };
        let activity = state_activity(ctx, self.scope, &self.name).await?;
        definition
            .states
            .iter()
            .enumerate()
            .map(|(index, (name, state))| {
                let point = definition
                    .layout
                    .as_ref()
                    .and_then(|layout| layout.get(name));
                let (x, y) = match point {
                    Some(point) => (point.x.round() as i32, point.y.round() as i32),
                    None => (40, 40 + 120 * i32::try_from(index)?),
                };
                Ok(SkillState {
                    name: name.clone(),
                    kind: StateType::from_db(state.type_name())?,
                    resource: match state {
                        State::Task(task) => Some(task.resource.clone()),
                        _ => None,
                    },
                    summary: summary(state),
                    transitions: transitions(state),
                    x,
                    y,
                    activity: activity.get(name).cloned().unwrap_or_default(),
                })
            })
            .collect()
    }

    async fn required_by(&self, ctx: &Context<'_>) -> Result<Vec<Trigger>> {
        Ok(Trigger::all(ctx, self.scope)
            .await?
            .into_iter()
            .filter(|trigger| {
                trigger.row.start["RequiredSkills"]
                    .as_array()
                    .is_some_and(|names| names.iter().any(|n| n == self.name.as_str()))
            })
            .collect())
    }

    async fn config(&self, ctx: &Context<'_>) -> Result<Option<Config>> {
        match self.config_id {
            Some(config_id) => Ok(Some(Config::load(ctx, self.scope, config_id).await?)),
            None => Ok(None),
        }
    }

    async fn edit_url(&self, ctx: &Context<'_>) -> Result<Option<String>> {
        let Some(config_id) = self.config_id else {
            return Ok(None);
        };
        if self.is_builtin() {
            return Ok(None);
        }
        let config = Config::load(ctx, self.scope, config_id).await?;
        let file = if self.skill_yaml.is_some() {
            "skill.yaml"
        } else {
            "SKILL.md"
        };
        Ok(Some(edit_url(
            &config.row.repository,
            &config.row.branch,
            &config.row.root_path,
            &format!("skills/{}/{file}", self.name),
        )))
    }

    async fn stats(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 30)] days: Option<i32>,
    ) -> Result<SkillStats> {
        let since = now(ctx) - Duration::days(i64::from(days.unwrap_or(30)));
        let runs = self.run_rows(ctx, Some(since), None).await?;
        let mut tx = tx(ctx, self.scope.tenant_id);
        let mut durations = Vec::new();
        let mut handoffs = Vec::new();
        let mut errors = BTreeMap::<String, usize>::new();
        let mut succeeded = 0;
        let mut failed = 0;
        for run in &runs {
            if let Some(finished) = run.finished_at {
                durations.push((finished - run.started_at).num_seconds());
                handoffs.push(i64::from(count_handoffs(&mut tx, run).await?));
            }
            match run.status {
                CoreRunStatus::Succeeded => succeeded += 1,
                CoreRunStatus::Failed => {
                    failed += 1;
                    *errors
                        .entry(run.error.clone().unwrap_or_else(|| "States.Unknown".into()))
                        .or_default() += 1;
                }
                _ => {}
            }
        }
        let mut failure_errors = errors.into_iter().collect::<Vec<_>>();
        failure_errors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(SkillStats {
            days: days.unwrap_or(30),
            runs: i32::try_from(runs.len())?,
            succeeded,
            median_duration_seconds: median(durations).map(i32::try_from).transpose()?,
            median_handoffs: median(handoffs).map(i32::try_from).transpose()?,
            gave_up: failed,
            failure_errors: failure_errors.into_iter().map(|(error, _)| error).collect(),
        })
    }

    async fn recent_runs(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 10)] limit: Option<i32>,
    ) -> Result<Vec<SkillRun>> {
        Ok(self
            .run_rows(ctx, None, Some(usize::try_from(limit.unwrap_or(10))?))
            .await?
            .into_iter()
            .map(|row| SkillRun {
                scope: self.scope,
                row,
            })
            .collect())
    }

    async fn open_change_request(&self, ctx: &Context<'_>) -> Result<Option<ChangeRequest>> {
        ChangeRequest::open_for(ctx, self.scope, ConfigObjectKind::Skill, &self.name).await
    }
}

fn median(mut values: Vec<i64>) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    Some(values[values.len() / 2])
}

fn summary(state: &State) -> String {
    let comment = match state {
        State::Task(s) => s.comment.as_ref(),
        State::Pass(s) => s.comment.as_ref(),
        State::Choice(s) => s.comment.as_ref(),
        State::Wait(s) => s.comment.as_ref(),
        State::Succeed(s) => s.comment.as_ref(),
        State::Fail(s) => s.comment.as_ref(),
    };
    if let Some(comment) = comment {
        return comment.clone();
    }
    match state {
        State::Task(s) => s
            .resource
            .rsplit_once(":::")
            .map(|(_, action)| action.to_string())
            .unwrap_or_else(|| s.resource.clone()),
        State::Pass(_) => "pass".to_string(),
        State::Choice(_) => "choice".to_string(),
        State::Wait(s) => match (s.seconds, &s.timestamp) {
            (Some(seconds), _) => format!("wait {seconds}s"),
            (None, Some(timestamp)) => format!("wait until {timestamp}"),
            (None, None) => "wait".to_string(),
        },
        State::Succeed(_) => "succeed".to_string(),
        State::Fail(s) => s.error.clone(),
    }
}

fn rule_label(rule: &ChoiceRule) -> String {
    if let Some(rules) = &rule.and {
        return rules
            .iter()
            .map(rule_label)
            .collect::<Vec<_>>()
            .join(" and ");
    }
    if let Some(rules) = &rule.or {
        return rules
            .iter()
            .map(rule_label)
            .collect::<Vec<_>>()
            .join(" or ");
    }
    if let Some(inner) = &rule.not {
        return format!("not ({})", rule_label(inner));
    }
    let variable = rule
        .variable
        .as_deref()
        .unwrap_or("$")
        .trim_start_matches("$.")
        .to_string();
    let comparisons: [(&str, Option<String>); 15] = [
        ("=", rule.string_equals.clone()),
        ("<", rule.string_less_than.clone()),
        (">", rule.string_greater_than.clone()),
        ("matches", rule.string_matches.clone()),
        ("=", rule.numeric_equals.map(|n| n.to_string())),
        ("<", rule.numeric_less_than.map(|n| n.to_string())),
        ("≤", rule.numeric_less_than_equals.map(|n| n.to_string())),
        (">", rule.numeric_greater_than.map(|n| n.to_string())),
        ("≥", rule.numeric_greater_than_equals.map(|n| n.to_string())),
        ("=", rule.boolean_equals.map(|b| b.to_string())),
        ("present", rule.is_present.map(|b| b.to_string())),
        ("null", rule.is_null.map(|b| b.to_string())),
        ("string", rule.is_string.map(|b| b.to_string())),
        ("numeric", rule.is_numeric.map(|b| b.to_string())),
        ("boolean", rule.is_boolean.map(|b| b.to_string())),
    ];
    comparisons
        .into_iter()
        .find_map(|(op, value)| {
            value.map(|value| match op {
                "present" | "null" | "string" | "numeric" | "boolean" => {
                    format!("{variable} is {op}: {value}")
                }
                _ => format!("{variable} {op} {value}"),
            })
        })
        .unwrap_or(variable)
}

fn transitions(state: &State) -> Vec<StateTransition> {
    let next = |to: &Option<String>| {
        to.iter()
            .map(|to| StateTransition {
                to: to.clone(),
                kind: TransitionKind::Next,
                label: None,
            })
            .collect::<Vec<_>>()
    };
    match state {
        State::Task(s) => {
            let mut out = next(&s.next);
            out.extend(s.catch.iter().map(|catch| StateTransition {
                to: catch.next.clone(),
                kind: TransitionKind::Catch,
                label: Some(catch.error_equals.join(", ")),
            }));
            out
        }
        State::Pass(s) => next(&s.next),
        State::Wait(s) => next(&s.next),
        State::Choice(s) => {
            let mut out = s
                .choices
                .iter()
                .filter_map(|choice| {
                    choice.next.as_ref().map(|to| StateTransition {
                        to: to.clone(),
                        kind: TransitionKind::Choice,
                        label: Some(rule_label(choice)),
                    })
                })
                .collect::<Vec<_>>();
            out.extend(s.default.iter().map(|to| StateTransition {
                to: to.clone(),
                kind: TransitionKind::Default,
                label: None,
            }));
            out
        }
        State::Succeed(_) | State::Fail(_) => Vec::new(),
    }
}

async fn state_activity(
    ctx: &Context<'_>,
    scope: Scope,
    skill_name: &str,
) -> Result<BTreeMap<String, StateActivity>> {
    let today = midnight(now(ctx));
    let mut tx = tx(ctx, scope.tenant_id);
    let runs = tx
        .list::<SkillRunItem>(
            &Listing::index(
                Index::Gsi1,
                keys::skill_runs_index(scope.tenant_id, skill_name),
            ),
            |run| run.status.is_active() || run.finished_at.is_none_or(|at| at >= today),
        )
        .await?;
    let mut activity = BTreeMap::<String, StateActivity>::new();
    for run in &runs {
        if run.status.is_active()
            && let Some(state) = &run.current_state
        {
            let task = tx
                .require::<TaskItem>(&keys::task_key(scope.tenant_id, run.task_id))
                .await?;
            let entry = activity.entry(state.clone()).or_default();
            if task.status == CoreTaskStatus::NeedsYou {
                entry.needs_you += 1;
            } else if run.status == CoreRunStatus::WaitingAgent {
                entry.with_agent += 1;
            } else {
                entry.running += 1;
            }
        }
        for state_entry in state_entries(&mut tx, run).await? {
            if state_entry.status == "SUCCEEDED"
                && state_entry.exited_at.is_some_and(|at| at >= today)
            {
                activity
                    .entry(state_entry.state_name)
                    .or_default()
                    .succeeded_today += 1;
            }
        }
    }
    Ok(activity)
}

async fn state_entries(tx: &mut TenantTx, run: &SkillRunItem) -> Result<Vec<StateEntryItem>> {
    Ok(tx
        .query::<StateEntryItem>(
            &keys::task_pk(run.tenant_id, run.task_id),
            &format!("{}#E#", keys::skill_run_prefix(run.number)),
        )
        .await?)
}

pub(crate) struct SkillInput {
    name: String,
    kind: String,
}

#[Object]
impl SkillInput {
    async fn name(&self) -> &str {
        &self.name
    }

    #[graphql(name = "type")]
    async fn kind(&self) -> &str {
        &self.kind
    }
}

pub(crate) struct StateTransition {
    to: String,
    kind: TransitionKind,
    label: Option<String>,
}

#[Object]
impl StateTransition {
    async fn to(&self) -> &str {
        &self.to
    }

    async fn kind(&self) -> TransitionKind {
        self.kind
    }

    async fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct StateActivity {
    running: i32,
    with_agent: i32,
    needs_you: i32,
    succeeded_today: i32,
}

#[Object]
impl StateActivity {
    async fn running(&self) -> i32 {
        self.running
    }

    async fn with_agent(&self) -> i32 {
        self.with_agent
    }

    async fn needs_you(&self) -> i32 {
        self.needs_you
    }

    async fn succeeded_today(&self) -> i32 {
        self.succeeded_today
    }
}

pub(crate) struct SkillState {
    name: String,
    kind: StateType,
    resource: Option<String>,
    summary: String,
    transitions: Vec<StateTransition>,
    x: i32,
    y: i32,
    activity: StateActivity,
}

#[Object]
impl SkillState {
    async fn name(&self) -> &str {
        &self.name
    }

    #[graphql(name = "type")]
    async fn kind(&self) -> StateType {
        self.kind
    }

    async fn resource(&self) -> Option<&str> {
        self.resource.as_deref()
    }

    async fn summary(&self) -> &str {
        &self.summary
    }

    async fn transitions(&self) -> &[StateTransition] {
        &self.transitions
    }

    async fn x(&self) -> i32 {
        self.x
    }

    async fn y(&self) -> i32 {
        self.y
    }

    async fn activity(&self) -> &StateActivity {
        &self.activity
    }
}

pub(crate) struct SkillStats {
    days: i32,
    runs: i32,
    succeeded: i32,
    median_duration_seconds: Option<i32>,
    median_handoffs: Option<i32>,
    gave_up: i32,
    failure_errors: Vec<String>,
}

#[Object]
impl SkillStats {
    async fn days(&self) -> i32 {
        self.days
    }

    async fn runs(&self) -> i32 {
        self.runs
    }

    async fn succeeded(&self) -> i32 {
        self.succeeded
    }

    async fn median_duration_seconds(&self) -> Option<i32> {
        self.median_duration_seconds
    }

    async fn median_handoffs(&self) -> Option<i32> {
        self.median_handoffs
    }

    async fn gave_up(&self) -> i32 {
        self.gave_up
    }

    async fn failure_errors(&self) -> &[String] {
        &self.failure_errors
    }
}

pub(crate) struct SkillRun {
    pub scope: Scope,
    pub row: SkillRunItem,
}

fn handoff_states(definition: &Value) -> Vec<String> {
    definition["States"]
        .as_object()
        .map(|states| {
            states
                .iter()
                .filter(|(_, state)| {
                    state["Type"] == "Task"
                        && state["Resource"]
                            .as_str()
                            .is_some_and(|resource| resource.contains(":agent:"))
                })
                .map(|(name, _)| name.clone())
                .collect()
        })
        .unwrap_or_default()
}

async fn count_handoffs(tx: &mut TenantTx, run: &SkillRunItem) -> Result<i32> {
    let states = handoff_states(&run.definition);
    let count = state_entries(tx, run)
        .await?
        .iter()
        .filter(|entry| states.contains(&entry.state_name))
        .count();
    Ok(i32::try_from(count)?)
}

impl SkillRun {
    pub async fn of_task(ctx: &Context<'_>, scope: Scope, task_id: Uuid) -> Result<Vec<Self>> {
        let mut rows = tx(ctx, scope.tenant_id)
            .query::<SkillRunItem>(&keys::task_pk(scope.tenant_id, task_id), "SR#")
            .await?;
        rows.sort_by_key(|row| row.number);
        Ok(rows
            .into_iter()
            .map(|row| SkillRun { scope, row })
            .collect())
    }
}

fn json_text(value: &Value) -> String {
    serde_json::to_string(value).expect("JSON values serialize")
}

#[Object]
impl SkillRun {
    async fn id(&self) -> ID {
        ID(self.row.id.to_string())
    }

    async fn number(&self) -> i32 {
        self.row.number
    }

    async fn skill(&self, ctx: &Context<'_>) -> Result<Skill> {
        let task = Task::load(ctx, self.scope, self.row.task_id).await?;
        let config_id = task.config_id(ctx).await?;
        Skill::for_run(
            ctx,
            self.scope,
            config_id,
            &self.row.skill_name,
            &self.row.definition,
        )
        .await
    }

    async fn task(&self, ctx: &Context<'_>) -> Result<Task> {
        Task::load(ctx, self.scope, self.row.task_id).await
    }

    async fn run(&self, ctx: &Context<'_>) -> Result<Option<Run>> {
        match self.row.run_id {
            Some(run_id) => Ok(Some(Run::load(ctx, self.scope, run_id).await?)),
            None => Ok(None),
        }
    }

    async fn status(&self) -> Result<SkillRunStatus> {
        SkillRunStatus::from_db(self.row.status.as_str())
    }

    async fn current_state(&self) -> Option<&str> {
        self.row.current_state.as_deref()
    }

    async fn handoffs(&self, ctx: &Context<'_>) -> Result<i32> {
        count_handoffs(&mut tx(ctx, self.scope.tenant_id), &self.row).await
    }

    async fn input(&self) -> String {
        json_text(&self.row.input)
    }

    async fn output(&self) -> Option<String> {
        self.row.output.as_ref().map(json_text)
    }

    async fn error(&self) -> Option<&str> {
        self.row.error.as_deref()
    }

    async fn cause(&self) -> Option<&str> {
        self.row.cause.as_deref()
    }

    async fn started_at(&self) -> Timestamp {
        ts(self.row.started_at)
    }

    async fn ended_at(&self) -> Option<Timestamp> {
        ts_opt(self.row.finished_at)
    }

    async fn state_entry_count(&self, ctx: &Context<'_>) -> Result<i32> {
        let entries = state_entries(&mut tx(ctx, self.scope.tenant_id), &self.row).await?;
        Ok(i32::try_from(entries.len())?)
    }

    async fn state_entries(&self, ctx: &Context<'_>, last: Option<i32>) -> Result<Vec<StateEntry>> {
        let mut listing = Listing::main(keys::task_pk(self.scope.tenant_id, self.row.task_id))
            .prefix(&format!("{}#E#", keys::skill_run_prefix(self.row.number)))
            .descending();
        if let Some(last) = last {
            listing = listing.limit(usize::try_from(last)?);
        }
        let mut rows = tx(ctx, self.scope.tenant_id)
            .list::<StateEntryItem>(&listing, |_| true)
            .await?;
        rows.reverse();
        Ok(rows.into_iter().map(StateEntry).collect())
    }

    async fn outputs(&self, ctx: &Context<'_>) -> Result<Vec<Output>> {
        Output::of_skill_run(ctx, self.scope, self.row.task_id, self.row.number).await
    }
}

pub(crate) struct StateEntry(StateEntryItem);

#[Object]
impl StateEntry {
    async fn id(&self) -> ID {
        ID(Uuid::new_v5(&self.0.skill_run_id, &self.0.sequence.to_be_bytes()).to_string())
    }

    async fn sequence(&self) -> Result<i32> {
        Ok(i32::try_from(self.0.sequence)?)
    }

    async fn state_name(&self) -> &str {
        &self.0.state_name
    }

    async fn state_type(&self) -> Result<StateType> {
        StateType::from_db(&self.0.state_type)
    }

    async fn status(&self) -> Result<StateEntryStatus> {
        StateEntryStatus::from_db(&self.0.status)
    }

    async fn entered_at(&self) -> Timestamp {
        ts(self.0.entered_at)
    }

    async fn exited_at(&self) -> Option<Timestamp> {
        ts_opt(self.0.exited_at)
    }

    async fn attempts(&self) -> Result<i32> {
        Ok(i32::try_from(self.0.attempts)?)
    }

    async fn input(&self) -> Option<String> {
        Some(json_text(&self.0.input))
    }

    async fn output(&self) -> Option<String> {
        self.0.output.as_ref().map(json_text)
    }

    async fn error(&self) -> Option<&str> {
        self.0.error.as_deref()
    }

    async fn cause(&self) -> Option<&str> {
        self.0.cause.as_deref()
    }

    async fn next_state(&self) -> Option<&str> {
        self.0.next_state.as_deref()
    }

    async fn result(&self) -> Option<String> {
        if let Some(error) = &self.0.error {
            return Some(match &self.0.cause {
                Some(cause) => format!("{error}: {cause}"),
                None => error.clone(),
            });
        }
        if let Some(output) = &self.0.output {
            let text = json_text(output);
            return Some(if text.chars().count() > 120 {
                format!("{}…", text.chars().take(119).collect::<String>())
            } else {
                text
            });
        }
        self.0.next_state.as_ref().map(|next| format!("→ {next}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_skill_flow_has_labels() {
        let skill = builtin_skill("open-pr-green-ci").unwrap();
        let definition = skill.definition.unwrap();
        let green = &definition.states["Green"];
        let labels = transitions(green)
            .into_iter()
            .map(|t| (t.to, t.label))
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec![
                (
                    "Report".to_string(),
                    Some("checks.Conclusion = success".to_string())
                ),
                ("GiveUp".to_string(), Some("fixes.Count ≥ 3".to_string())),
                ("HandOff".to_string(), None),
            ]
        );
        assert_eq!(
            summary(&definition.states["OpenPRs"]),
            "github:createPullRequests"
        );
        let value = serde_json::to_value(&definition).unwrap();
        assert_eq!(handoff_states(&value), vec!["HandOff".to_string()]);
    }
}
