#![allow(dead_code)]

use chrono::{DateTime, Duration, TimeZone, Utc};
use sanchoris_config::skill::{SkillDefinition, parse_skill_yaml};
use sanchoris_engine::{
    Disposition, Effect, Event, IgnoreReason, Invocation, Notification, Progress, SkillRun,
    SkillRunStatus, StartRequest, StateEntry, Step, Timer, advance, start,
};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};

pub const RUN_ID: i64 = 1;

pub fn epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub run: SkillRun,
    pub entries: Vec<StateEntry>,
    pub invocations: Vec<(DateTime<Utc>, Invocation)>,
    pub notifications: Vec<Notification>,
    pub statuses: Vec<SkillRunStatus>,
}

pub struct Harness {
    yaml: String,
    definition: SkillDefinition,
    restart_between_steps: bool,
    stored_run: String,
    stored_entries: BTreeMap<u64, String>,
    memory_run: Option<SkillRun>,
    memory_entries: BTreeMap<u64, StateEntry>,
    pub now: DateTime<Utc>,
    timers: Vec<(Timer, DateTime<Utc>)>,
    pub pending: VecDeque<Invocation>,
    pub invocations: Vec<(DateTime<Utc>, Invocation)>,
    pub notifications: Vec<Notification>,
    pub statuses: Vec<SkillRunStatus>,
    pub ignored: Vec<IgnoreReason>,
}

impl Harness {
    pub fn new(yaml: &str, restart_between_steps: bool) -> Self {
        Harness {
            yaml: yaml.to_string(),
            definition: parse_skill_yaml(yaml).expect("the skill parses"),
            restart_between_steps,
            stored_run: String::new(),
            stored_entries: BTreeMap::new(),
            memory_run: None,
            memory_entries: BTreeMap::new(),
            now: epoch(),
            timers: Vec::new(),
            pending: VecDeque::new(),
            invocations: Vec::new(),
            notifications: Vec::new(),
            statuses: Vec::new(),
            ignored: Vec::new(),
        }
    }

    fn definition(&mut self) -> SkillDefinition {
        if self.restart_between_steps {
            self.definition = parse_skill_yaml(&self.yaml).expect("the skill parses");
        }
        self.definition.clone()
    }

    pub fn run(&self) -> SkillRun {
        if self.restart_between_steps {
            serde_json::from_str(&self.stored_run).expect("the stored run deserializes")
        } else {
            self.memory_run.clone().expect("the run has started")
        }
    }

    pub fn entries(&self) -> Vec<StateEntry> {
        if self.restart_between_steps {
            self.stored_entries
                .values()
                .map(|e| serde_json::from_str(e).expect("the stored entry deserializes"))
                .collect()
        } else {
            self.memory_entries.values().cloned().collect()
        }
    }

    pub fn entry(&self, state_name: &str) -> Vec<StateEntry> {
        self.entries()
            .into_iter()
            .filter(|e| e.state_name == state_name)
            .collect()
    }

    pub fn path(&self) -> Vec<String> {
        self.entries().into_iter().map(|e| e.state_name).collect()
    }

    pub fn start(&mut self, input: Value) {
        let definition = self.definition();
        let step = start(
            &definition,
            StartRequest {
                skill_run_id: RUN_ID,
                skill_name: "open-pr-green-ci".to_string(),
                input,
            },
            self.now,
        )
        .expect("the run starts");
        self.persist(step);
    }

    pub fn send(&mut self, event: Event) -> Disposition {
        let definition = self.definition();
        let run = self.run();
        let step = advance(&definition, &run, event, self.now).expect("the engine advances");
        let disposition = step.disposition;
        match disposition {
            Disposition::Applied => self.persist(step),
            Disposition::Ignored(reason) => {
                assert!(step.entries.is_empty() && step.effects.is_empty());
                assert_eq!(step.run, run);
                self.ignored.push(reason);
            }
        }
        disposition
    }

    fn persist(&mut self, step: Step) {
        let step: Step = if self.restart_between_steps {
            serde_json::from_str(&serde_json::to_string(&step).unwrap()).unwrap()
        } else {
            step
        };
        if self.restart_between_steps {
            self.stored_run = serde_json::to_string(&step.run).unwrap();
            for entry in &step.entries {
                self.stored_entries
                    .insert(entry.sequence, serde_json::to_string(entry).unwrap());
            }
        } else {
            self.memory_run = Some(step.run.clone());
            for entry in &step.entries {
                self.memory_entries.insert(entry.sequence, entry.clone());
            }
        }
        self.statuses.push(step.run.status);
        for effect in step.effects {
            match effect {
                Effect::InvokeResource(invocation) => {
                    self.invocations.push((self.now, invocation.clone()));
                    self.pending.push_back(invocation);
                }
                Effect::ScheduleTimer { timer, due_at } => self.timers.push((timer, due_at)),
                Effect::CancelTimers { sequence } => {
                    self.timers.retain(|(t, _)| t.sequence() != Some(sequence))
                }
                Effect::CancelAllTimers => self.timers.clear(),
                Effect::Notify(notification) => self.notifications.push(notification),
            }
        }
    }

    pub fn next_timer(&self) -> Option<(Timer, DateTime<Utc>)> {
        self.timers.iter().min_by_key(|(_, due)| *due).copied()
    }

    pub fn fire_next_timer(&mut self) -> Option<(Timer, Disposition)> {
        let (timer, due_at) = self.next_timer()?;
        let index = self
            .timers
            .iter()
            .position(|t| *t == (timer, due_at))
            .unwrap();
        self.timers.remove(index);
        self.now = self.now.max(due_at);
        Some((timer, self.send(Event::TimerDue { timer })))
    }

    pub fn advance_clock(&mut self, by: Duration) {
        self.now += by;
    }

    pub fn token(&self) -> String {
        self.run()
            .waiting_task_token()
            .expect("the run waits for a task token")
            .to_string()
    }

    pub fn acknowledged(&self) -> bool {
        matches!(
            self.run().current.and_then(|c| c.progress),
            Some(Progress::Invoking {
                acknowledged: true,
                ..
            })
        )
    }

    pub fn summary(&self) -> Summary {
        Summary {
            run: self.run(),
            entries: self.entries(),
            invocations: self.invocations.clone(),
            notifications: self.notifications.clone(),
            statuses: self.statuses.clone(),
        }
    }
}
