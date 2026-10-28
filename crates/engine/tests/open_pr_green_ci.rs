use crate::common;

use chrono::Duration;
use common::{Harness, RUN_ID, Summary, epoch};
use sanchoris_engine::{
    Disposition, Event, IdempotencyKey, IgnoreReason, Invocation, NotificationKind,
    ResourceOutcome, SkillRunStatus, StateEntryStatus, Timer,
};
use serde_json::{Value, json};
use std::collections::VecDeque;

const FIXTURE: &str = include_str!("../../config/tests/fixtures/open-pr-green-ci.skill.yaml");
const FAILED_CHECK: &str = "acme/shop-web#52 (checkout_spec::flaky)";
const CI_LOG: &str = "assertion failed: expected 3 items, got 2";

fn skill() -> String {
    FIXTURE.to_string()
}

fn input() -> Value {
    json!({"Repos": ["acme/shop-web"], "Issue": 35, "MaxFixes": 3})
}

fn pull_requests() -> Value {
    json!([{"Repo": "acme/shop-web", "Number": 52}])
}

#[derive(Default)]
struct World {
    push_failures: u32,
    checks: VecDeque<&'static str>,
    hang: Vec<&'static str>,
    agent_hangs: bool,
    deliver_twice: bool,
    fixes: u32,
}

impl World {
    fn respond(&mut self, invocation: &Invocation) -> Option<ResourceOutcome> {
        let name = invocation.resource.name();
        if self.hang.contains(&name.as_str()) {
            return None;
        }
        Some(match name.as_str() {
            "git:push" if self.push_failures > 0 => {
                self.push_failures -= 1;
                ResourceOutcome::Failed {
                    error: None,
                    cause: Some("remote rejected the push".to_string()),
                }
            }
            "git:push" => ResourceOutcome::Succeeded {
                output: json!({"Pushed": invocation.parameters["Repos"]}),
            },
            "github:createPullRequests" => ResourceOutcome::Succeeded {
                output: pull_requests(),
            },
            "github:checks" => ResourceOutcome::Succeeded {
                output: json!({"Watching": true}),
            },
            "github:commentOnIssue" => ResourceOutcome::Succeeded {
                output: json!({"CommentId": 9001, "Body": invocation.parameters["Body"]}),
            },
            other => panic!("unexpected resource {other}"),
        })
    }

    fn check_result(&mut self) -> Value {
        match self.checks.pop_front().unwrap_or("failure") {
            "success" => json!({"Conclusion": "success"}),
            _ => json!({"Conclusion": "failure", "Failed": FAILED_CHECK, "Log": CI_LOG}),
        }
    }
}

fn drive(h: &mut Harness, world: &mut World) {
    loop {
        if h.run().status.is_finished() {
            return;
        }
        if let Some(invocation) = h.pending.pop_front() {
            if let Some(outcome) = world.respond(&invocation) {
                let event = Event::ResourceResult {
                    key: invocation.idempotency_key,
                    outcome,
                };
                assert_eq!(h.send(event.clone()), Disposition::Applied);
                if world.deliver_twice {
                    assert!(matches!(h.send(event), Disposition::Ignored(_)));
                }
            }
            continue;
        }
        let run = h.run();
        let current = run.current.as_ref().map(|c| c.state_name.as_str());
        match (run.status, current) {
            (SkillRunStatus::Waiting, Some("WaitChecks"))
                if h.acknowledged() && !world.hang.contains(&"github:checks") =>
            {
                let output = world.check_result();
                let event = Event::TaskSuccess {
                    task_token: h.token(),
                    output,
                };
                assert_eq!(h.send(event.clone()), Disposition::Applied);
                if world.deliver_twice {
                    assert_eq!(
                        h.send(event),
                        Disposition::Ignored(IgnoreReason::UnknownTaskToken)
                    );
                }
                continue;
            }
            (SkillRunStatus::WaitingAgent, Some("HandOff")) if !world.agent_hangs => {
                world.fixes += 1;
                h.advance_clock(Duration::minutes(5));
                let event = Event::TaskSuccess {
                    task_token: h.token(),
                    output: json!({"Commit": format!("fix-{}", world.fixes)}),
                };
                assert_eq!(h.send(event), Disposition::Applied);
                continue;
            }
            (SkillRunStatus::WaitingAgent, _) => return,
            _ => {}
        }
        if h.fire_next_timer().is_none() {
            return;
        }
    }
}

fn scenario(restart: bool, world: World) -> (Harness, World) {
    let mut world = world;
    let mut h = Harness::new(&skill(), restart);
    h.start(input());
    drive(&mut h, &mut world);
    (h, world)
}

fn both(world: impl Fn() -> World) -> (Harness, Summary) {
    let (memory, _) = scenario(false, world());
    let (restarted, _) = scenario(true, world());
    let summary = restarted.summary();
    assert_eq!(memory.summary(), summary);
    (restarted, summary)
}

fn resources(summary: &Summary) -> Vec<String> {
    summary
        .invocations
        .iter()
        .map(|(_, i)| i.resource.name())
        .collect()
}

fn path(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

#[test]
fn succeeds_when_checks_pass() {
    let (h, summary) = both(|| World {
        checks: VecDeque::from(["success"]),
        ..World::default()
    });
    assert_eq!(
        summary.run.status,
        SkillRunStatus::Succeeded,
        "{:?}",
        summary.run
    );
    assert_eq!(
        h.path(),
        path(&["Init", "Push", "OpenPRs", "WaitChecks", "Green", "Report"])
    );
    assert_eq!(
        resources(&summary),
        path(&[
            "git:push",
            "github:createPullRequests",
            "github:checks",
            "github:commentOnIssue"
        ])
    );
    let keys: Vec<IdempotencyKey> = summary
        .invocations
        .iter()
        .map(|(_, i)| i.idempotency_key)
        .collect();
    assert_eq!(
        keys.iter()
            .map(|k| (k.skill_run_id, k.sequence, k.attempt))
            .collect::<Vec<_>>(),
        vec![
            (RUN_ID, 2, 1),
            (RUN_ID, 3, 1),
            (RUN_ID, 4, 1),
            (RUN_ID, 6, 1)
        ]
    );
    let checks = &summary.invocations[2].1;
    assert_eq!(
        checks.parameters["TaskToken"].as_str(),
        checks.task_token.as_deref()
    );
    let report = &summary.invocations[3].1;
    assert_eq!(
        report.parameters,
        json!({
            "Repo": "acme/shop-web",
            "Number": 35,
            "Mode": "update",
            "Body": format!("Pull requests are open and the checks pass: {}", pull_requests())
        })
    );
    let report_entry = &h.entry("Report")[0];
    assert_eq!(
        report_entry.input,
        json!({
            "Repos": ["acme/shop-web"],
            "Issue": 35,
            "MaxFixes": 3,
            "fixes": {"Count": 0},
            "pullRequests": pull_requests(),
            "checks": {"Conclusion": "success"}
        })
    );
    assert_eq!(summary.run.output.as_ref().unwrap()["CommentId"], 9001);
    assert!(
        summary
            .entries
            .iter()
            .all(|e| e.status == StateEntryStatus::Succeeded)
    );
    assert_eq!(summary.notifications.len(), 1);
    assert_eq!(
        summary.notifications[0].text,
        format!(
            "[sanchoris] Skill open-pr-green-ci (run 1) succeeded.\nOutput: {}",
            summary.run.output.as_ref().unwrap()
        )
    );
}

#[test]
fn hands_ci_failure_to_the_agent_and_succeeds_after_the_fix() {
    let (h, summary) = both(|| World {
        checks: VecDeque::from(["failure", "success"]),
        ..World::default()
    });
    assert_eq!(
        summary.run.status,
        SkillRunStatus::Succeeded,
        "{:?}",
        summary.run
    );
    assert_eq!(
        h.path(),
        path(&[
            "Init",
            "Push",
            "OpenPRs",
            "WaitChecks",
            "Green",
            "HandOff",
            "CountFix",
            "Push",
            "OpenPRs",
            "WaitChecks",
            "Green",
            "Report"
        ])
    );
    assert!(summary.statuses.contains(&SkillRunStatus::WaitingAgent));
    let needs_agent = &summary.notifications[0];
    assert_eq!(
        needs_agent.kind,
        NotificationKind::NeedsAgent {
            instruction: format!(
                "CI failed on {FAILED_CHECK}. Fix it, commit, and continue this skill. Log: {CI_LOG}"
            )
        }
    );
    assert_eq!(
        needs_agent.text,
        format!(
            "[sanchoris] Skill open-pr-green-ci (run 1) needs you: CI failed on {FAILED_CHECK}. Fix it, commit, and continue this skill. Log: {CI_LOG}\nWhen you are done, run `sanchoris skill continue 1`."
        )
    );
    let handoff = &h.entry("HandOff")[0];
    assert_eq!(handoff.status, StateEntryStatus::Succeeded);
    assert_eq!(handoff.output.as_ref(), Some(&handoff.input));
    assert_eq!(
        h.entry("CountFix")[0].output.as_ref().unwrap()["fixes"],
        json!({"Count": 1})
    );
    assert_eq!(
        resources(&summary)
            .iter()
            .filter(|r| *r == "git:push")
            .count(),
        2
    );
    assert_eq!(h.entry("Report")[0].input["fixes"], json!({"Count": 1}));
    assert_eq!(summary.notifications.len(), 2);
}

#[test]
fn fails_with_fixes_exhausted_when_checks_keep_failing() {
    let (h, summary) = both(World::default);
    assert_eq!(summary.run.status, SkillRunStatus::Failed);
    assert_eq!(summary.run.error.as_deref(), Some("Skill.FixesExhausted"));
    assert_eq!(
        summary.run.cause.as_deref(),
        Some("The checks still fail after 3 fixes")
    );
    assert_eq!(h.entry("HandOff").len(), 3);
    assert_eq!(h.entry("Push").len(), 4);
    let give_up = &h.entry("GiveUp")[0];
    assert_eq!(give_up.status, StateEntryStatus::Failed);
    assert_eq!(give_up.input["fixes"], json!({"Count": 3}));
    let last = summary.notifications.last().unwrap();
    assert_eq!(
        last.text,
        "[sanchoris] Skill open-pr-green-ci (run 1) failed: Skill.FixesExhausted.\nCause: The checks still fail after 3 fixes"
    );
    assert_eq!(
        summary
            .notifications
            .iter()
            .filter(|n| matches!(n.kind, NotificationKind::NeedsAgent { .. }))
            .count(),
        3
    );
}

#[test]
fn retries_push_with_backoff_then_succeeds() {
    let (h, summary) = both(|| World {
        push_failures: 2,
        checks: VecDeque::from(["success"]),
        ..World::default()
    });
    assert_eq!(
        summary.run.status,
        SkillRunStatus::Succeeded,
        "{:?}",
        summary.run
    );
    let push = &h.entry("Push")[0];
    assert_eq!(push.attempts, 3);
    assert_eq!(push.status, StateEntryStatus::Succeeded);
    let pushes: Vec<_> = summary
        .invocations
        .iter()
        .filter(|(_, i)| i.resource.name() == "git:push")
        .collect();
    assert_eq!(
        pushes
            .iter()
            .map(|(at, i)| (*at - epoch(), i.idempotency_key.attempt))
            .collect::<Vec<_>>(),
        vec![
            (Duration::zero(), 1),
            (Duration::seconds(10), 2),
            (Duration::seconds(30), 3)
        ]
    );
}

#[test]
fn fails_with_task_failed_when_push_retries_run_out() {
    let (h, summary) = both(|| World {
        push_failures: 10,
        ..World::default()
    });
    assert_eq!(summary.run.status, SkillRunStatus::Failed);
    assert_eq!(summary.run.error.as_deref(), Some("States.TaskFailed"));
    assert_eq!(
        summary.run.cause.as_deref(),
        Some("remote rejected the push")
    );
    let push = &h.entry("Push")[0];
    assert_eq!(push.attempts, 4);
    assert_eq!(push.status, StateEntryStatus::Failed);
    assert_eq!(push.retry_counts, vec![3]);
}

#[test]
fn task_timeout_fails_with_states_timeout() {
    let (h, summary) = both(|| World {
        hang: vec!["github:createPullRequests"],
        ..World::default()
    });
    assert_eq!(summary.run.status, SkillRunStatus::Failed);
    assert_eq!(summary.run.error.as_deref(), Some("States.Timeout"));
    let open = &h.entry("OpenPRs")[0];
    assert_eq!(open.status, StateEntryStatus::TimedOut);
    assert_eq!(open.exited_at, Some(epoch() + Duration::seconds(120)));
    assert_eq!(
        summary.run.finished_at,
        Some(epoch() + Duration::seconds(120))
    );
}

#[test]
fn checks_that_never_finish_time_out_after_an_hour() {
    let (h, summary) = both(|| World {
        hang: vec!["github:checks"],
        ..World::default()
    });
    assert_eq!(summary.run.error.as_deref(), Some("States.Timeout"));
    assert_eq!(h.entry("WaitChecks")[0].status, StateEntryStatus::TimedOut);
    assert_eq!(
        summary.run.finished_at,
        Some(epoch() + Duration::seconds(3600))
    );
}

fn heartbeat_scenario(restart: bool) -> Summary {
    let mut world = World {
        checks: VecDeque::from(["failure"]),
        agent_hangs: true,
        ..World::default()
    };
    let mut h = Harness::new(&skill(), restart);
    h.start(input());
    drive(&mut h, &mut world);
    assert_eq!(h.run().status, SkillRunStatus::WaitingAgent);
    let entered = h.run().current.unwrap().entered_at;
    h.advance_clock(Duration::seconds(600));
    assert_eq!(
        h.send(Event::TaskHeartbeat {
            task_token: "sanchoris-task:1:99:1".to_string()
        }),
        Disposition::Ignored(IgnoreReason::UnknownTaskToken)
    );
    let token = h.token();
    assert_eq!(
        h.send(Event::TaskHeartbeat { task_token: token }),
        Disposition::Applied
    );
    let (timer, disposition) = h.fire_next_timer().unwrap();
    assert!(matches!(timer, Timer::HeartbeatTimeout { .. }));
    assert_eq!(h.now, entered + Duration::seconds(900));
    assert_eq!(disposition, Disposition::Ignored(IgnoreReason::TimerNotDue));
    let (timer, disposition) = h.fire_next_timer().unwrap();
    assert!(matches!(timer, Timer::HeartbeatTimeout { .. }));
    assert_eq!(disposition, Disposition::Applied);
    assert_eq!(h.now, entered + Duration::seconds(1500));
    let run = h.run();
    assert_eq!(run.status, SkillRunStatus::Failed);
    assert_eq!(run.error.as_deref(), Some("States.HeartbeatTimeout"));
    assert_eq!(h.entry("HandOff")[0].status, StateEntryStatus::TimedOut);
    assert!(h.next_timer().is_none());
    h.summary()
}

#[test]
fn heartbeat_timeout_fails_the_handoff() {
    assert_eq!(heartbeat_scenario(false), heartbeat_scenario(true));
}

#[test]
fn duplicated_results_are_ignored() {
    let (_, single) = both(|| World {
        push_failures: 1,
        checks: VecDeque::from(["failure", "success"]),
        ..World::default()
    });
    let (h, doubled) = both(|| World {
        push_failures: 1,
        checks: VecDeque::from(["failure", "success"]),
        deliver_twice: true,
        ..World::default()
    });
    assert_eq!(single, doubled);
    assert!(!h.ignored.is_empty());
}

#[test]
fn stale_keys_tokens_and_timers_are_ignored() {
    let mut h = Harness::new(&skill(), true);
    h.start(input());
    let first = h.pending.pop_front().unwrap();
    assert_eq!(first.resource.name(), "git:push");
    let failure = ResourceOutcome::Failed {
        error: None,
        cause: None,
    };
    assert_eq!(
        h.send(Event::ResourceResult {
            key: first.idempotency_key,
            outcome: failure.clone()
        }),
        Disposition::Applied
    );
    assert_eq!(
        h.send(Event::ResourceResult {
            key: first.idempotency_key,
            outcome: ResourceOutcome::Succeeded { output: json!({}) }
        }),
        Disposition::Ignored(IgnoreReason::StaleIdempotencyKey)
    );
    assert_eq!(
        h.send(Event::TimerDue {
            timer: Timer::RetryBackoff {
                sequence: first.idempotency_key.sequence,
                attempt: 2
            }
        }),
        Disposition::Ignored(IgnoreReason::TimerNotDue)
    );
    assert_eq!(
        h.send(Event::TimerDue {
            timer: Timer::TaskTimeout {
                sequence: first.idempotency_key.sequence,
                attempt: 1
            }
        }),
        Disposition::Ignored(IgnoreReason::StaleTimer)
    );
    let (timer, disposition) = h.fire_next_timer().unwrap();
    assert!(matches!(timer, Timer::RetryBackoff { attempt: 2, .. }));
    assert_eq!(disposition, Disposition::Applied);
    let second = h.pending.pop_front().unwrap();
    assert_eq!(second.idempotency_key.attempt, 2);
    assert_eq!(
        h.send(Event::ResourceResult {
            key: first.idempotency_key,
            outcome: ResourceOutcome::Succeeded { output: json!({}) }
        }),
        Disposition::Ignored(IgnoreReason::StaleIdempotencyKey)
    );
    assert_eq!(
        h.send(Event::ResourceResult {
            key: IdempotencyKey {
                skill_run_id: RUN_ID + 1,
                ..second.idempotency_key
            },
            outcome: ResourceOutcome::Succeeded { output: json!({}) }
        }),
        Disposition::Ignored(IgnoreReason::StaleIdempotencyKey)
    );
    assert_eq!(
        h.send(Event::ResourceResult {
            key: second.idempotency_key,
            outcome: ResourceOutcome::Succeeded { output: json!({}) }
        }),
        Disposition::Applied
    );
    let open = h.pending.pop_front().unwrap();
    h.send(Event::ResourceResult {
        key: open.idempotency_key,
        outcome: ResourceOutcome::Succeeded {
            output: pull_requests(),
        },
    });
    let checks = h.pending.pop_front().unwrap();
    let ack = Event::ResourceResult {
        key: checks.idempotency_key,
        outcome: ResourceOutcome::Succeeded { output: json!({}) },
    };
    assert_eq!(h.send(ack.clone()), Disposition::Applied);
    assert_eq!(
        h.send(ack),
        Disposition::Ignored(IgnoreReason::DuplicateResult)
    );
    let token = h.token();
    assert_eq!(token, checks.task_token.clone().unwrap());
    assert_eq!(
        h.send(Event::TaskSuccess {
            task_token: "sanchoris-task:1:2:1".to_string(),
            output: json!({"Conclusion": "success"})
        }),
        Disposition::Ignored(IgnoreReason::UnknownTaskToken)
    );
    let success = Event::TaskSuccess {
        task_token: token,
        output: json!({"Conclusion": "success"}),
    };
    assert_eq!(h.send(success.clone()), Disposition::Applied);
    assert_eq!(
        h.send(success),
        Disposition::Ignored(IgnoreReason::UnknownTaskToken)
    );
    let report = h.pending.pop_front().unwrap();
    let done = Event::ResourceResult {
        key: report.idempotency_key,
        outcome: ResourceOutcome::Succeeded { output: json!({}) },
    };
    assert_eq!(h.send(done.clone()), Disposition::Applied);
    assert_eq!(h.run().status, SkillRunStatus::Succeeded);
    assert_eq!(
        h.send(done),
        Disposition::Ignored(IgnoreReason::RunFinished)
    );
    assert_eq!(
        h.send(Event::TimerDue {
            timer: Timer::SkillRunTimeout
        }),
        Disposition::Ignored(IgnoreReason::RunFinished)
    );
}

#[test]
fn skill_run_timeout_fails_the_run() {
    let mut h = Harness::new(&skill(), true);
    h.start(input());
    let push = h.pending.pop_front().unwrap();
    h.advance_clock(Duration::seconds(100));
    assert_eq!(
        h.send(Event::TimerDue {
            timer: Timer::SkillRunTimeout
        }),
        Disposition::Ignored(IgnoreReason::TimerNotDue)
    );
    h.advance_clock(Duration::seconds(14400));
    assert_eq!(
        h.send(Event::TimerDue {
            timer: Timer::SkillRunTimeout
        }),
        Disposition::Applied
    );
    let run = h.run();
    assert_eq!(run.status, SkillRunStatus::Failed);
    assert_eq!(run.error.as_deref(), Some("States.Timeout"));
    assert_eq!(h.entry("Push")[0].status, StateEntryStatus::TimedOut);
    assert!(run.current.is_none());
    assert!(h.next_timer().is_none());
    assert_eq!(
        h.send(Event::ResourceResult {
            key: push.idempotency_key,
            outcome: ResourceOutcome::Succeeded { output: json!({}) }
        }),
        Disposition::Ignored(IgnoreReason::RunFinished)
    );
}
