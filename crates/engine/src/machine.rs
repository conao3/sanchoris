use crate::choice::{self, ChoiceError};
use crate::model::{
    Disposition, Effect, EngineError, Event, IdempotencyKey, IgnoreReason, Integration, Invocation,
    NotificationKind, Progress, ResourceOutcome, ResourceRef, SkillRun, SkillRunStatus,
    StartRequest, StateEntry, StateEntryStatus, Step, Timer,
};
use crate::notify;
use crate::path::{PathError, ReferencePath};
use crate::payload;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use sanchoris_config::skill::{
    InputType, ResultPath, Retrier, SkillDefinition, State, TaskState, errors,
};
use serde_json::{Map, Value, json};

pub const RUNTIME_ERROR: &str = "States.Runtime";
pub const MAX_INLINE_TRANSITIONS: usize = 1000;

pub fn start(
    definition: &SkillDefinition,
    request: StartRequest,
    now: DateTime<Utc>,
) -> Result<Step, EngineError> {
    validate_input(definition, &request.input)?;
    let run = SkillRun {
        id: request.skill_run_id,
        skill_name: request.skill_name,
        status: SkillRunStatus::Running,
        input: request.input.clone(),
        output: None,
        error: None,
        cause: None,
        started_at: now,
        deadline_at: now + seconds(definition.timeout_seconds),
        finished_at: None,
        next_sequence: 1,
        current: None,
    };
    let mut machine = Machine::new(definition, run, now);
    machine.effects.push(Effect::ScheduleTimer {
        timer: Timer::SkillRunTimeout,
        due_at: machine.run.deadline_at,
    });
    machine.proceed(definition.start_at.clone(), request.input)?;
    Ok(machine.finish())
}

pub fn advance(
    definition: &SkillDefinition,
    run: &SkillRun,
    event: Event,
    now: DateTime<Utc>,
) -> Result<Step, EngineError> {
    let ignored = |reason| Step {
        run: run.clone(),
        entries: Vec::new(),
        effects: Vec::new(),
        disposition: Disposition::Ignored(reason),
    };
    if run.status.is_finished() {
        return Ok(ignored(IgnoreReason::RunFinished));
    }
    let mut machine = Machine::new(definition, run.clone(), now);
    match machine.handle(event)? {
        Disposition::Applied => Ok(machine.finish()),
        Disposition::Ignored(reason) => Ok(ignored(reason)),
    }
}

pub fn task_token(key: IdempotencyKey) -> String {
    format!("sanchoris-task:{key}")
}

pub fn error_matches(error_equals: &[String], error: &str) -> bool {
    if error == RUNTIME_ERROR {
        return false;
    }
    error_equals.iter().any(|pattern| {
        pattern == error
            || pattern == errors::ALL
            || (pattern == errors::TASK_FAILED && error != errors::TIMEOUT)
    })
}

pub fn retry_delay(retrier: &Retrier, retries_so_far: u32) -> Duration {
    let raw = retrier.interval_seconds as f64 * retrier.backoff_rate.powi(retries_so_far as i32);
    let capped = match retrier.max_delay_seconds {
        Some(max) => raw.min(max as f64),
        None => raw,
    };
    Duration::milliseconds((capped * 1000.0).round() as i64)
}

pub fn apply_result_path(
    result_path: &ResultPath,
    input: &Value,
    result: Value,
) -> Result<Value, PathError> {
    match result_path {
        ResultPath::Root => Ok(result),
        ResultPath::Discard => Ok(input.clone()),
        ResultPath::Path(path) => ReferencePath::parse(path)?.write(input, result),
    }
}

fn seconds(n: u64) -> Duration {
    Duration::seconds(i64::try_from(n).expect("seconds fit in i64"))
}

fn timestamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn validate_input(definition: &SkillDefinition, input: &Value) -> Result<(), EngineError> {
    let object = input
        .as_object()
        .ok_or_else(|| EngineError::InvalidInput("the input must be a JSON object".to_string()))?;
    for (name, ty) in &definition.inputs {
        let value = object
            .get(name)
            .ok_or_else(|| EngineError::InvalidInput(format!("{name} is required")))?;
        let ok = match ty {
            InputType::String => value.is_string(),
            InputType::Number => value.is_number(),
            InputType::Boolean => value.is_boolean(),
            InputType::Array => value.is_array(),
            InputType::Object => value.is_object(),
        };
        if !ok {
            return Err(EngineError::InvalidInput(format!(
                "{name} must be of type {ty:?}"
            )));
        }
    }
    if let Some(extra) = object.keys().find(|k| !definition.inputs.contains_key(*k)) {
        return Err(EngineError::InvalidInput(format!(
            "{extra} is not a declared input"
        )));
    }
    Ok(())
}

struct Machine<'d> {
    definition: &'d SkillDefinition,
    run: SkillRun,
    now: DateTime<Utc>,
    entries: Vec<StateEntry>,
    effects: Vec<Effect>,
}

impl<'d> Machine<'d> {
    fn new(definition: &'d SkillDefinition, run: SkillRun, now: DateTime<Utc>) -> Self {
        Machine {
            definition,
            run,
            now,
            entries: Vec::new(),
            effects: Vec::new(),
        }
    }

    fn finish(mut self) -> Step {
        if let Some(current) = self.run.current.clone() {
            self.record(current);
        }
        Step {
            run: self.run,
            entries: self.entries,
            effects: self.effects,
            disposition: Disposition::Applied,
        }
    }

    fn record(&mut self, entry: StateEntry) {
        match self
            .entries
            .iter_mut()
            .find(|e| e.sequence == entry.sequence)
        {
            Some(existing) => *existing = entry,
            None => self.entries.push(entry),
        }
    }

    fn state(&self, name: &str) -> Result<&'d State, EngineError> {
        self.definition
            .states
            .get(name)
            .ok_or_else(|| EngineError::UnknownState(name.to_string()))
    }

    fn current(&self) -> Result<&StateEntry, EngineError> {
        self.run
            .current
            .as_ref()
            .ok_or_else(|| EngineError::Inconsistent("the run has no current state".to_string()))
    }

    fn current_mut(&mut self) -> Result<&mut StateEntry, EngineError> {
        self.run
            .current
            .as_mut()
            .ok_or_else(|| EngineError::Inconsistent("the run has no current state".to_string()))
    }

    fn current_task(&self) -> Result<&'d TaskState, EngineError> {
        let name = &self.current()?.state_name;
        match self.state(name)? {
            State::Task(task) => Ok(task),
            other => Err(EngineError::Inconsistent(format!(
                "{name} is a {} state but the run is invoking a resource",
                other.type_name()
            ))),
        }
    }

    fn context(&self, task_token: Option<&str>) -> Value {
        let mut context = Map::new();
        context.insert(
            "Execution".to_string(),
            json!({
                "Id": self.run.id,
                "Name": format!("{}-{}", self.run.skill_name, self.run.id),
                "Input": self.run.input,
                "StartTime": timestamp(self.run.started_at),
            }),
        );
        if let Some(entry) = &self.run.current {
            context.insert(
                "State".to_string(),
                json!({
                    "Name": entry.state_name,
                    "EnteredTime": timestamp(entry.entered_at),
                    "RetryCount": entry.attempts.saturating_sub(1),
                }),
            );
        }
        if let Some(token) = task_token {
            context.insert("Task".to_string(), json!({ "Token": token }));
        }
        Value::Object(context)
    }

    fn close(
        &mut self,
        status: StateEntryStatus,
        output: Option<Value>,
        next_state: Option<String>,
        error: Option<String>,
        cause: Option<String>,
    ) -> Result<(), EngineError> {
        let mut entry =
            self.run.current.take().ok_or_else(|| {
                EngineError::Inconsistent("the run has no current state".to_string())
            })?;
        if entry.progress.take().is_some() {
            self.effects.push(Effect::CancelTimers {
                sequence: entry.sequence,
            });
        }
        entry.status = status;
        entry.exited_at = Some(self.now);
        entry.output = output;
        entry.next_state = next_state;
        entry.error = error;
        entry.cause = cause;
        self.record(entry);
        Ok(())
    }

    fn succeed_run(&mut self, output: Value) {
        self.run.status = SkillRunStatus::Succeeded;
        self.run.output = Some(output.clone());
        self.run.finished_at = Some(self.now);
        self.effects.push(Effect::CancelAllTimers);
        self.notify(NotificationKind::Succeeded { output });
    }

    fn fail_run(&mut self, error: String, cause: Option<String>) -> Result<(), EngineError> {
        if self.run.current.is_some() {
            self.close(
                StateEntryStatus::Failed,
                None,
                None,
                Some(error.clone()),
                cause.clone(),
            )?;
        }
        self.run.status = SkillRunStatus::Failed;
        self.run.error = Some(error.clone());
        self.run.cause = cause.clone();
        self.run.finished_at = Some(self.now);
        self.effects.push(Effect::CancelAllTimers);
        self.notify(NotificationKind::Failed { error, cause });
        Ok(())
    }

    fn runtime_failure(&mut self, cause: impl ToString) -> Result<(), EngineError> {
        self.fail_run(RUNTIME_ERROR.to_string(), Some(cause.to_string()))
    }

    fn notify(&mut self, kind: NotificationKind) {
        self.effects.push(Effect::Notify(notify::notification(
            self.run.id,
            &self.run.skill_name,
            kind,
        )));
    }

    fn follow(
        &mut self,
        next: Option<&str>,
        end: Option<bool>,
        output: Value,
    ) -> Result<Option<(String, Value)>, EngineError> {
        match (next, end) {
            (Some(next), None | Some(false)) => {
                self.close(
                    StateEntryStatus::Succeeded,
                    Some(output.clone()),
                    Some(next.to_string()),
                    None,
                    None,
                )?;
                Ok(Some((next.to_string(), output)))
            }
            (None, Some(true)) => {
                self.close(
                    StateEntryStatus::Succeeded,
                    Some(output.clone()),
                    None,
                    None,
                    None,
                )?;
                self.succeed_run(output);
                Ok(None)
            }
            _ => Err(EngineError::InvalidDefinition(format!(
                "state {} needs exactly one of Next or End: true",
                self.current()?.state_name
            ))),
        }
    }

    fn proceed(&mut self, name: String, input: Value) -> Result<(), EngineError> {
        let mut next = Some((name, input));
        for _ in 0..MAX_INLINE_TRANSITIONS {
            let Some((name, input)) = next.take() else {
                return Ok(());
            };
            next = self.enter(name, input)?;
        }
        if next.is_some() {
            self.runtime_failure(format!(
                "the skill made {MAX_INLINE_TRANSITIONS} transitions in a row without waiting on a Task or Wait state"
            ))?;
        }
        Ok(())
    }

    fn enter(
        &mut self,
        name: String,
        input: Value,
    ) -> Result<Option<(String, Value)>, EngineError> {
        let state = self.state(&name)?;
        let sequence = self.run.next_sequence;
        self.run.next_sequence += 1;
        self.run.status = SkillRunStatus::Running;
        self.run.current = Some(StateEntry {
            sequence,
            state_name: name,
            state_type: state.type_name().to_string(),
            status: StateEntryStatus::Running,
            entered_at: self.now,
            exited_at: None,
            input: input.clone(),
            output: None,
            error: None,
            cause: None,
            attempts: 0,
            next_state: None,
            progress: None,
            retry_counts: Vec::new(),
        });
        match state {
            State::Pass(pass) => {
                let context = self.context(None);
                let effective = match &pass.parameters {
                    Some(template) => payload::build(template, &input, &context),
                    None => Ok(input.clone()),
                };
                let effective = match effective {
                    Ok(value) => value,
                    Err(error) => {
                        self.runtime_failure(error)?;
                        return Ok(None);
                    }
                };
                let result = pass.result.clone().unwrap_or(effective);
                match apply_result_path(&pass.result_path, &input, result) {
                    Ok(output) => self.follow(pass.next.as_deref(), pass.end, output),
                    Err(error) => {
                        self.runtime_failure(error)?;
                        Ok(None)
                    }
                }
            }
            State::Choice(state) => {
                let context = self.context(None);
                match choice::choose(state, &input, &context) {
                    Ok(Some(next)) => {
                        self.close(
                            StateEntryStatus::Succeeded,
                            Some(input.clone()),
                            Some(next.to_string()),
                            None,
                            None,
                        )?;
                        Ok(Some((next.to_string(), input)))
                    }
                    Ok(None) => {
                        self.fail_run(
                            errors::NO_CHOICE_MATCHED.to_string(),
                            Some("No choice rule matched and the state has no Default".to_string()),
                        )?;
                        Ok(None)
                    }
                    Err(ChoiceError::Definition(reason)) => {
                        Err(EngineError::InvalidDefinition(reason))
                    }
                    Err(ChoiceError::Eval(error)) => {
                        self.runtime_failure(error)?;
                        Ok(None)
                    }
                }
            }
            State::Wait(wait) => {
                let until = match (wait.seconds, wait.timestamp.as_deref()) {
                    (Some(n), None) => self.now + seconds(n),
                    (None, Some(text)) => DateTime::parse_from_rfc3339(text)
                        .map_err(|e| {
                            EngineError::InvalidDefinition(format!(
                                "Timestamp {text:?} is not RFC 3339: {e}"
                            ))
                        })?
                        .with_timezone(&Utc),
                    _ => {
                        return Err(EngineError::InvalidDefinition(
                            "a Wait state needs exactly one of Seconds or Timestamp".to_string(),
                        ));
                    }
                };
                if until <= self.now {
                    return self.follow(wait.next.as_deref(), wait.end, input);
                }
                let entry = self.current_mut()?;
                entry.status = StateEntryStatus::Waiting;
                entry.progress = Some(Progress::Waiting { until });
                self.run.status = SkillRunStatus::Waiting;
                self.effects.push(Effect::ScheduleTimer {
                    timer: Timer::WaitElapsed { sequence },
                    due_at: until,
                });
                Ok(None)
            }
            State::Succeed(_) => {
                self.close(
                    StateEntryStatus::Succeeded,
                    Some(input.clone()),
                    None,
                    None,
                    None,
                )?;
                self.succeed_run(input);
                Ok(None)
            }
            State::Fail(fail) => {
                self.fail_run(fail.error.clone(), Some(fail.cause.clone()))?;
                Ok(None)
            }
            State::Task(task) => {
                self.start_attempt(task, 1)?;
                Ok(None)
            }
        }
    }

    fn start_attempt(&mut self, task: &'d TaskState, attempt: u32) -> Result<(), EngineError> {
        let resource = ResourceRef::parse(&task.resource)?;
        let entry = self.current_mut()?;
        entry.attempts = attempt;
        let key = IdempotencyKey {
            skill_run_id: self.run.id,
            sequence: self.current()?.sequence,
            attempt,
        };
        let token =
            (resource.integration == Integration::WaitForTaskToken).then(|| task_token(key));
        let context = self.context(token.as_deref());
        let input = self.current()?.input.clone();
        let parameters = match &task.parameters {
            Some(template) => payload::build(template, &input, &context),
            None => Ok(input),
        };
        let parameters = match parameters {
            Ok(value) => value,
            Err(error) => return self.runtime_failure(error),
        };
        let handoff = resource.is_agent_handoff();
        let instruction = if handoff {
            match parameters.get("Instruction") {
                Some(Value::String(text)) => Some(text.clone()),
                _ => {
                    return self
                        .runtime_failure("agent:handoff needs a string Instruction parameter");
                }
            }
        } else {
            None
        };
        let timeout_at = self.now + seconds(task.timeout_seconds);
        let heartbeat_at = task.heartbeat_seconds.map(|n| self.now + seconds(n));
        self.run.status = match (&token, handoff) {
            (Some(_), true) => SkillRunStatus::WaitingAgent,
            (Some(_), false) => SkillRunStatus::Waiting,
            (None, _) => SkillRunStatus::Running,
        };
        let entry = self.current_mut()?;
        entry.status = if token.is_some() {
            StateEntryStatus::Waiting
        } else {
            StateEntryStatus::Running
        };
        entry.progress = Some(Progress::Invoking {
            attempt,
            timeout_at,
            heartbeat_at,
            task_token: token.clone(),
            acknowledged: handoff,
        });
        match instruction {
            Some(instruction) => self.notify(NotificationKind::NeedsAgent { instruction }),
            None => self.effects.push(Effect::InvokeResource(Invocation {
                resource,
                parameters,
                idempotency_key: key,
                task_token: token,
            })),
        }
        self.effects.push(Effect::ScheduleTimer {
            timer: Timer::TaskTimeout {
                sequence: key.sequence,
                attempt,
            },
            due_at: timeout_at,
        });
        if let Some(due_at) = heartbeat_at {
            self.effects.push(Effect::ScheduleTimer {
                timer: Timer::HeartbeatTimeout {
                    sequence: key.sequence,
                    attempt,
                },
                due_at,
            });
        }
        Ok(())
    }

    fn complete_task(&mut self, result: Value) -> Result<(), EngineError> {
        let task = self.current_task()?;
        let input = self.current()?.input.clone();
        match apply_result_path(&task.result_path, &input, result) {
            Ok(output) => {
                if let Some((next, input)) = self.follow(task.next.as_deref(), task.end, output)? {
                    self.proceed(next, input)?;
                }
                Ok(())
            }
            Err(error) => self.runtime_failure(error),
        }
    }

    fn task_error(&mut self, error: String, cause: Option<String>) -> Result<(), EngineError> {
        let task = self.current_task()?;
        if error == RUNTIME_ERROR {
            return self.fail_run(error, cause);
        }
        if let Some((index, retrier)) = task
            .retry
            .iter()
            .enumerate()
            .find(|(_, r)| error_matches(&r.error_equals, &error))
        {
            let now = self.now;
            let entry = self.current_mut()?;
            entry.retry_counts.resize(task.retry.len(), 0);
            let used = entry.retry_counts[index];
            if used < retrier.max_attempts {
                entry.retry_counts[index] = used + 1;
                let retry_at = now + retry_delay(retrier, used);
                let next_attempt = entry.attempts + 1;
                let sequence = entry.sequence;
                entry.status = StateEntryStatus::Running;
                entry.error = Some(error);
                entry.cause = cause;
                entry.progress = Some(Progress::BackingOff {
                    next_attempt,
                    retry_at,
                });
                self.run.status = SkillRunStatus::Running;
                self.effects.push(Effect::CancelTimers { sequence });
                self.effects.push(Effect::ScheduleTimer {
                    timer: Timer::RetryBackoff {
                        sequence,
                        attempt: next_attempt,
                    },
                    due_at: retry_at,
                });
                return Ok(());
            }
        }
        if let Some(catcher) = task
            .catch
            .iter()
            .find(|c| error_matches(&c.error_equals, &error))
        {
            let mut error_output = Map::new();
            error_output.insert("Error".to_string(), Value::String(error.clone()));
            if let Some(cause) = &cause {
                error_output.insert("Cause".to_string(), Value::String(cause.clone()));
            }
            let input = self.current()?.input.clone();
            return match apply_result_path(
                &catcher.result_path,
                &input,
                Value::Object(error_output),
            ) {
                Ok(output) => {
                    self.close(
                        StateEntryStatus::Caught,
                        Some(output.clone()),
                        Some(catcher.next.clone()),
                        Some(error),
                        cause,
                    )?;
                    self.proceed(catcher.next.clone(), output)
                }
                Err(path_error) => self.runtime_failure(path_error),
            };
        }
        if error == errors::TIMEOUT || error == errors::HEARTBEAT_TIMEOUT {
            self.close(
                StateEntryStatus::TimedOut,
                None,
                None,
                Some(error.clone()),
                cause.clone(),
            )?;
        }
        self.fail_run(error, cause)
    }

    fn invoking(&self) -> Option<(u64, u32, Option<&str>, bool)> {
        let entry = self.run.current.as_ref()?;
        match entry.progress.as_ref()? {
            Progress::Invoking {
                attempt,
                task_token,
                acknowledged,
                ..
            } => Some((
                entry.sequence,
                *attempt,
                task_token.as_deref(),
                *acknowledged,
            )),
            _ => None,
        }
    }

    fn handle(&mut self, event: Event) -> Result<Disposition, EngineError> {
        match event {
            Event::ResourceResult { key, outcome } => {
                let Some((sequence, attempt, token, acknowledged)) = self.invoking() else {
                    return Ok(Disposition::Ignored(IgnoreReason::StaleIdempotencyKey));
                };
                if key.skill_run_id != self.run.id
                    || key.sequence != sequence
                    || key.attempt != attempt
                {
                    return Ok(Disposition::Ignored(IgnoreReason::StaleIdempotencyKey));
                }
                if acknowledged {
                    return Ok(Disposition::Ignored(IgnoreReason::DuplicateResult));
                }
                let waits_for_token = token.is_some();
                match outcome {
                    ResourceOutcome::Succeeded { .. } if waits_for_token => {
                        if let Some(Progress::Invoking { acknowledged, .. }) =
                            self.current_mut()?.progress.as_mut()
                        {
                            *acknowledged = true;
                        }
                    }
                    ResourceOutcome::Succeeded { output } => self.complete_task(output)?,
                    ResourceOutcome::Failed { error, cause } => self.task_error(
                        error.unwrap_or_else(|| errors::TASK_FAILED.to_string()),
                        cause,
                    )?,
                }
                Ok(Disposition::Applied)
            }
            Event::TaskSuccess { task_token, output } => {
                if !self.token_is_current(&task_token) {
                    return Ok(Disposition::Ignored(IgnoreReason::UnknownTaskToken));
                }
                self.complete_task(output)?;
                Ok(Disposition::Applied)
            }
            Event::TaskFailure {
                task_token,
                error,
                cause,
            } => {
                if !self.token_is_current(&task_token) {
                    return Ok(Disposition::Ignored(IgnoreReason::UnknownTaskToken));
                }
                self.task_error(
                    error.unwrap_or_else(|| errors::TASK_FAILED.to_string()),
                    cause,
                )?;
                Ok(Disposition::Applied)
            }
            Event::TaskHeartbeat { task_token } => {
                if !self.token_is_current(&task_token) {
                    return Ok(Disposition::Ignored(IgnoreReason::UnknownTaskToken));
                }
                let task = self.current_task()?;
                if let Some(n) = task.heartbeat_seconds {
                    let due_at = self.now + seconds(n);
                    let entry = self.current_mut()?;
                    let sequence = entry.sequence;
                    if let Some(Progress::Invoking {
                        attempt,
                        heartbeat_at,
                        ..
                    }) = entry.progress.as_mut()
                    {
                        *heartbeat_at = Some(due_at);
                        let attempt = *attempt;
                        self.effects.push(Effect::ScheduleTimer {
                            timer: Timer::HeartbeatTimeout { sequence, attempt },
                            due_at,
                        });
                    }
                }
                Ok(Disposition::Applied)
            }
            Event::TimerDue { timer } => self.timer_due(timer),
        }
    }

    fn token_is_current(&self, task_token: &str) -> bool {
        matches!(self.invoking(), Some((_, _, Some(current), _)) if current == task_token)
    }

    fn timer_due(&mut self, timer: Timer) -> Result<Disposition, EngineError> {
        let now = self.now;
        if timer == Timer::SkillRunTimeout {
            if now < self.run.deadline_at {
                return Ok(Disposition::Ignored(IgnoreReason::TimerNotDue));
            }
            let cause = Some(format!(
                "The skill run did not finish within {} seconds",
                self.definition.timeout_seconds
            ));
            let error = errors::TIMEOUT.to_string();
            if self.run.current.is_some() {
                self.close(
                    StateEntryStatus::TimedOut,
                    None,
                    None,
                    Some(error.clone()),
                    cause.clone(),
                )?;
            }
            self.fail_run(error, cause)?;
            return Ok(Disposition::Applied);
        }
        let Some(entry) = self.run.current.as_ref() else {
            return Ok(Disposition::Ignored(IgnoreReason::StaleTimer));
        };
        if timer.sequence() != Some(entry.sequence) {
            return Ok(Disposition::Ignored(IgnoreReason::StaleTimer));
        }
        let due_at = match (timer, entry.progress.as_ref()) {
            (Timer::WaitElapsed { .. }, Some(Progress::Waiting { until })) => *until,
            (
                Timer::RetryBackoff { attempt, .. },
                Some(Progress::BackingOff {
                    next_attempt,
                    retry_at,
                }),
            ) if attempt == *next_attempt => *retry_at,
            (
                Timer::TaskTimeout { attempt, .. },
                Some(Progress::Invoking {
                    attempt: current,
                    timeout_at,
                    ..
                }),
            ) if attempt == *current => *timeout_at,
            (
                Timer::HeartbeatTimeout { attempt, .. },
                Some(Progress::Invoking {
                    attempt: current,
                    heartbeat_at: Some(heartbeat_at),
                    ..
                }),
            ) if attempt == *current => *heartbeat_at,
            _ => return Ok(Disposition::Ignored(IgnoreReason::StaleTimer)),
        };
        if now < due_at {
            return Ok(Disposition::Ignored(IgnoreReason::TimerNotDue));
        }
        match timer {
            Timer::WaitElapsed { .. } => {
                let State::Wait(wait) = self.state(&entry.state_name)? else {
                    return Err(EngineError::Inconsistent(format!(
                        "{} is not a Wait state",
                        entry.state_name
                    )));
                };
                let input = entry.input.clone();
                if let Some((next, input)) = self.follow(wait.next.as_deref(), wait.end, input)? {
                    self.proceed(next, input)?;
                }
            }
            Timer::RetryBackoff { attempt, .. } => {
                let task = self.current_task()?;
                self.start_attempt(task, attempt)?;
            }
            Timer::TaskTimeout { .. } => {
                let task = self.current_task()?;
                self.task_error(
                    errors::TIMEOUT.to_string(),
                    Some(format!(
                        "The task did not finish within {} seconds",
                        task.timeout_seconds
                    )),
                )?;
            }
            Timer::HeartbeatTimeout { .. } => {
                let task = self.current_task()?;
                self.task_error(
                    errors::HEARTBEAT_TIMEOUT.to_string(),
                    Some(format!(
                        "No heartbeat arrived within {} seconds",
                        task.heartbeat_seconds.unwrap_or_default()
                    )),
                )?;
            }
            Timer::SkillRunTimeout => unreachable!("handled above"),
        }
        Ok(Disposition::Applied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_path_variants() {
        let input = json!({"a": 1});
        assert_eq!(
            apply_result_path(&ResultPath::Root, &input, json!(2)).unwrap(),
            json!(2)
        );
        assert_eq!(
            apply_result_path(&ResultPath::Discard, &input, json!(2)).unwrap(),
            input
        );
        assert_eq!(
            apply_result_path(&ResultPath::Path("$.b.c".into()), &input, json!(2)).unwrap(),
            json!({"a": 1, "b": {"c": 2}})
        );
        assert!(apply_result_path(&ResultPath::Path("$.a.c".into()), &input, json!(2)).is_err());
    }

    #[test]
    fn error_matching_follows_asl() {
        let all = vec![errors::ALL.to_string()];
        let task_failed = vec![errors::TASK_FAILED.to_string()];
        let named = vec!["GitHub.ChecksFailed".to_string()];
        assert!(error_matches(&all, errors::TIMEOUT));
        assert!(error_matches(&all, "Custom.Error"));
        assert!(!error_matches(&all, RUNTIME_ERROR));
        assert!(error_matches(&task_failed, errors::TASK_FAILED));
        assert!(error_matches(&task_failed, "Custom.Error"));
        assert!(error_matches(&task_failed, errors::HEARTBEAT_TIMEOUT));
        assert!(!error_matches(&task_failed, errors::TIMEOUT));
        assert!(error_matches(&named, "GitHub.ChecksFailed"));
        assert!(!error_matches(&named, "GitHub.Other"));
    }

    #[test]
    fn retry_delay_backs_off_and_caps() {
        let retrier = Retrier {
            error_equals: vec![errors::ALL.to_string()],
            interval_seconds: 10,
            max_attempts: 5,
            backoff_rate: 2.0,
            max_delay_seconds: Some(25),
        };
        assert_eq!(retry_delay(&retrier, 0), Duration::seconds(10));
        assert_eq!(retry_delay(&retrier, 1), Duration::seconds(20));
        assert_eq!(retry_delay(&retrier, 2), Duration::seconds(25));
        let fractional = Retrier {
            backoff_rate: 1.5,
            max_delay_seconds: None,
            ..retrier
        };
        assert_eq!(retry_delay(&fractional, 1), Duration::seconds(15));
        assert_eq!(retry_delay(&fractional, 2), Duration::milliseconds(22_500));
    }

    #[test]
    fn resource_names_parse() {
        let r = ResourceRef::parse("sanchoris:::github:checks.waitForTaskToken").unwrap();
        assert_eq!(
            (r.service.as_str(), r.action.as_str()),
            ("github", "checks")
        );
        assert_eq!(r.integration, Integration::WaitForTaskToken);
        assert_eq!(
            ResourceRef::parse("sanchoris:::workspace:exec.sync")
                .unwrap()
                .integration,
            Integration::Sync
        );
        assert_eq!(
            ResourceRef::parse("sanchoris:::git:push")
                .unwrap()
                .integration,
            Integration::RequestResponse
        );
        assert!(ResourceRef::parse("arn:aws:states:::lambda:invoke").is_err());
        assert!(ResourceRef::parse("sanchoris:::git").is_err());
        assert!(ResourceRef::parse("sanchoris:::agent:handoff").is_err());
    }
}
