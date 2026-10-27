use std::collections::{BTreeMap, BTreeSet};

use async_graphql::{Context, ID, Object, Result};
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use sanchoris_config::builtin_skill;
use sanchoris_config::skill::{ChoiceRule, SkillDefinition, State};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::config::{ChangeRequest, Config, edit_url};
use crate::context::{Scope, now, tenant_tx};
use crate::enums::{
    ConfigObjectKind, SkillKind, SkillRunStatus, StateEntryStatus, StateType, TransitionKind,
};
use crate::error::not_found;
use crate::scalars::{Timestamp, ts, ts_opt};
use crate::task::{Output, Run, Task};
use crate::trigger::Trigger;

#[derive(Debug, Clone, FromRow)]
struct SkillRow {
    id: Uuid,
    config_id: Uuid,
    name: String,
    kind: String,
    description: String,
    skill_md: String,
    skill_yaml: Option<String>,
    definition: Option<Value>,
}

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
    fn from_row(scope: Scope, row: SkillRow) -> Result<Self> {
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
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let rows = sqlx::query_as::<_, SkillRow>(
            "SELECT s.id, s.config_id, s.name, s.kind, s.description, s.skill_md, s.skill_yaml, s.definition \
             FROM sanchoris.skills s JOIN sanchoris.configs c ON c.id = s.config_id \
             WHERE s.tenant_id = $1 AND ($2::uuid IS NULL OR s.config_id = $2) \
               AND ($3::text[] IS NULL OR s.name = ANY($3)) \
             ORDER BY s.name, c.repository, c.root_path",
        )
        .bind(scope.tenant_id)
        .bind(config_id)
        .bind(names)
        .fetch_all(tx.conn())
        .await?;
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
        if let Some(skill) = Self::rows(ctx, scope, Some(config_id), Some(&[name.to_string()]))
            .await?
            .into_iter()
            .next()
        {
            return Ok(skill);
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
        limit: Option<i64>,
    ) -> Result<Vec<SkillRunRow>> {
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        Ok(sqlx::query_as::<_, SkillRunRow>(sql!(
            "{SKILL_RUN_SELECT} WHERE s.tenant_id = $1 AND s.skill_name = $2 \
               AND ($3::timestamptz IS NULL OR s.started_at >= $3) \
             ORDER BY s.started_at DESC, s.id DESC LIMIT $4"
        ))
        .bind(self.scope.tenant_id)
        .bind(&self.name)
        .bind(since)
        .bind(limit.unwrap_or(i64::MAX))
        .fetch_all(tx.conn())
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
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
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
            match run.status.as_str() {
                "SUCCEEDED" => succeeded += 1,
                "FAILED" => {
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
            .run_rows(ctx, None, Some(i64::from(limit.unwrap_or(10))))
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

#[derive(Debug, FromRow)]
struct ActivityRow {
    state: String,
    running: i64,
    with_agent: i64,
    needs_you: i64,
    succeeded_today: i64,
}

async fn state_activity(
    ctx: &Context<'_>,
    scope: Scope,
    skill_name: &str,
) -> Result<BTreeMap<String, StateActivity>> {
    let at = now(ctx);
    let today = Utc
        .with_ymd_and_hms(at.year(), at.month(), at.day(), 0, 0, 0)
        .single()
        .expect("midnight is unambiguous in UTC");
    let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
    let rows = sqlx::query_as::<_, ActivityRow>(
        "WITH current AS ( \
           SELECT s.current_state AS state, \
                  count(*) FILTER (WHERE s.status IN ('RUNNING', 'WAITING') AND t.status <> 'NEEDS_YOU') AS running, \
                  count(*) FILTER (WHERE s.status = 'WAITING_AGENT' AND t.status <> 'NEEDS_YOU') AS with_agent, \
                  count(*) FILTER (WHERE t.status = 'NEEDS_YOU') AS needs_you \
           FROM sanchoris.skill_runs s JOIN sanchoris.tasks t ON t.id = s.task_id \
           WHERE s.tenant_id = $1 AND s.skill_name = $2 AND s.current_state IS NOT NULL \
             AND s.status IN ('RUNNING', 'WAITING_AGENT', 'WAITING') \
           GROUP BY s.current_state), \
         done AS ( \
           SELECT e.state_name AS state, count(*) AS succeeded_today \
           FROM sanchoris.state_entries e JOIN sanchoris.skill_runs s ON s.id = e.skill_run_id \
           WHERE e.tenant_id = $1 AND s.skill_name = $2 AND e.status = 'SUCCEEDED' AND e.exited_at >= $3 \
           GROUP BY e.state_name) \
         SELECT coalesce(c.state, d.state) AS state, coalesce(c.running, 0) AS running, \
                coalesce(c.with_agent, 0) AS with_agent, coalesce(c.needs_you, 0) AS needs_you, \
                coalesce(d.succeeded_today, 0) AS succeeded_today \
         FROM current c FULL JOIN done d ON d.state = c.state",
    )
    .bind(scope.tenant_id)
    .bind(skill_name)
    .bind(today)
    .fetch_all(tx.conn())
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok((
                row.state,
                StateActivity {
                    running: i32::try_from(row.running)?,
                    with_agent: i32::try_from(row.with_agent)?,
                    needs_you: i32::try_from(row.needs_you)?,
                    succeeded_today: i32::try_from(row.succeeded_today)?,
                },
            ))
        })
        .collect()
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

pub(crate) const SKILL_RUN_SELECT: &str = "SELECT s.id, s.task_id, s.run_id, s.number, s.skill_name, s.definition, s.input, \
     s.output, s.status, s.current_state, s.error, s.cause, s.started_at, s.finished_at \
     FROM sanchoris.skill_runs s";

#[derive(Debug, Clone, FromRow)]
pub(crate) struct SkillRunRow {
    pub id: Uuid,
    pub task_id: Uuid,
    pub run_id: Option<Uuid>,
    pub number: i32,
    pub skill_name: String,
    pub definition: Value,
    pub input: Value,
    pub output: Option<Value>,
    pub status: String,
    pub current_state: Option<String>,
    pub error: Option<String>,
    pub cause: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

pub(crate) struct SkillRun {
    pub scope: Scope,
    pub row: SkillRunRow,
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

async fn count_handoffs(tx: &mut sanchoris_core::db::TenantTx, row: &SkillRunRow) -> Result<i32> {
    let states = handoff_states(&row.definition);
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM sanchoris.state_entries WHERE skill_run_id = $1 AND state_name = ANY($2)",
    )
    .bind(row.id)
    .bind(&states)
    .fetch_one(tx.conn())
    .await?;
    Ok(i32::try_from(count)?)
}

impl SkillRun {
    pub async fn query(
        ctx: &Context<'_>,
        scope: Scope,
        task_id: Option<Uuid>,
        run_id: Option<Uuid>,
    ) -> Result<Vec<Self>> {
        let mut tx = tenant_tx(ctx, scope.tenant_id).await?;
        let rows = sqlx::query_as::<_, SkillRunRow>(sql!(
            "{SKILL_RUN_SELECT} WHERE s.tenant_id = $1 AND ($2::uuid IS NULL OR s.task_id = $2) \
               AND ($3::uuid IS NULL OR s.run_id = $3) ORDER BY s.task_id, s.number"
        ))
        .bind(scope.tenant_id)
        .bind(task_id)
        .bind(run_id)
        .fetch_all(tx.conn())
        .await?;
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
        SkillRunStatus::from_db(&self.row.status)
    }

    async fn current_state(&self) -> Option<&str> {
        self.row.current_state.as_deref()
    }

    async fn handoffs(&self, ctx: &Context<'_>) -> Result<i32> {
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        count_handoffs(&mut tx, &self.row).await
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
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM sanchoris.state_entries WHERE skill_run_id = $1",
        )
        .bind(self.row.id)
        .fetch_one(tx.conn())
        .await?;
        Ok(i32::try_from(count)?)
    }

    async fn state_entries(&self, ctx: &Context<'_>, last: Option<i32>) -> Result<Vec<StateEntry>> {
        let mut tx = tenant_tx(ctx, self.scope.tenant_id).await?;
        let mut rows = sqlx::query_as::<_, StateEntryRow>(
            "SELECT id, sequence, state_name, state_type, status, entered_at, exited_at, attempts, \
                    input, output, error, cause, next_state \
             FROM sanchoris.state_entries WHERE skill_run_id = $1 \
             ORDER BY sequence DESC LIMIT $2",
        )
        .bind(self.row.id)
        .bind(last.map_or(i64::MAX, i64::from))
        .fetch_all(tx.conn())
        .await?;
        rows.reverse();
        Ok(rows.into_iter().map(StateEntry).collect())
    }

    async fn outputs(&self, ctx: &Context<'_>) -> Result<Vec<Output>> {
        Output::query(ctx, self.scope, None, Some(self.row.id)).await
    }
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct StateEntryRow {
    id: Uuid,
    sequence: i32,
    state_name: String,
    state_type: String,
    status: String,
    entered_at: DateTime<Utc>,
    exited_at: Option<DateTime<Utc>>,
    attempts: i32,
    input: Value,
    output: Option<Value>,
    error: Option<String>,
    cause: Option<String>,
    next_state: Option<String>,
}

pub(crate) struct StateEntry(StateEntryRow);

#[Object]
impl StateEntry {
    async fn id(&self) -> ID {
        ID(self.0.id.to_string())
    }

    async fn sequence(&self) -> i32 {
        self.0.sequence
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

    async fn attempts(&self) -> i32 {
        self.0.attempts
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
