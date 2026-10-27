mod common;

use chrono::Duration;
use common::{Harness, epoch};
use sanchoris_config::skill::parse_skill_yaml;
use sanchoris_engine::{
    Disposition, EngineError, Event, IgnoreReason, ResourceOutcome, SkillRunStatus, StartRequest,
    StateEntryStatus, Timer, start,
};
use serde_json::json;

fn started(yaml: &str, input: serde_json::Value) -> Harness {
    let mut h = Harness::new(yaml, true);
    h.start(input);
    h
}

#[test]
fn wait_state_blocks_until_its_timer() {
    let yaml = r#"
Inputs: {}
StartAt: Pause
TimeoutSeconds: 600
States:
  Pause:
    Type: Wait
    Seconds: 30
    Next: Done
  Done:
    Type: Succeed
"#;
    let mut h = started(yaml, json!({}));
    assert_eq!(h.run().status, SkillRunStatus::Waiting);
    assert_eq!(h.entry("Pause")[0].status, StateEntryStatus::Waiting);
    let sequence = h.entry("Pause")[0].sequence;
    h.advance_clock(Duration::seconds(10));
    assert_eq!(
        h.send(Event::TimerDue {
            timer: Timer::WaitElapsed { sequence }
        }),
        Disposition::Ignored(IgnoreReason::TimerNotDue)
    );
    let (timer, disposition) = h.fire_next_timer().unwrap();
    assert_eq!(timer, Timer::WaitElapsed { sequence });
    assert_eq!(disposition, Disposition::Applied);
    assert_eq!(h.now, epoch() + Duration::seconds(30));
    let run = h.run();
    assert_eq!(run.status, SkillRunStatus::Succeeded);
    assert_eq!(run.output, Some(json!({})));
    assert_eq!(h.path(), vec!["Pause", "Done"]);
}

#[test]
fn wait_with_a_past_timestamp_continues_inline() {
    let yaml = r#"
Inputs: {}
StartAt: Pause
TimeoutSeconds: 600
States:
  Pause:
    Type: Wait
    Timestamp: "2020-01-01T00:00:00Z"
    End: true
"#;
    let h = started(yaml, json!({}));
    assert_eq!(h.run().status, SkillRunStatus::Succeeded);
}

#[test]
fn skill_run_timeout_interrupts_a_wait() {
    let yaml = r#"
Inputs: {}
StartAt: Pause
TimeoutSeconds: 50
States:
  Pause:
    Type: Wait
    Seconds: 100
    End: true
"#;
    let mut h = started(yaml, json!({}));
    let (timer, disposition) = h.fire_next_timer().unwrap();
    assert_eq!(timer, Timer::SkillRunTimeout);
    assert_eq!(disposition, Disposition::Applied);
    let run = h.run();
    assert_eq!(run.error.as_deref(), Some("States.Timeout"));
    assert_eq!(
        run.cause.as_deref(),
        Some("The skill run did not finish within 50 seconds")
    );
    assert_eq!(h.entry("Pause")[0].status, StateEntryStatus::TimedOut);
    assert!(h.next_timer().is_none());
}

const CATCHING: &str = r#"
Inputs: { Repo: string }
StartAt: Merge
TimeoutSeconds: 600
States:
  Merge:
    Type: Task
    Resource: sanchoris:::github:mergePullRequest
    Parameters: { Repo.$: $.Repo, Number: 1, Execution.$: $$.Execution.Name, State.$: $$.State.Name }
    TimeoutSeconds: 30
    Catch:
      - ErrorEquals: [GitHub.Conflict]
        ResultPath: $.error
        Next: Recover
      - ErrorEquals: [States.ALL]
        ResultPath: $.other
        Next: Recover
    Next: Done
  Recover:
    Type: Pass
    End: true
  Done:
    Type: Succeed
"#;

#[test]
fn catch_places_the_error_at_its_result_path() {
    let mut h = started(CATCHING, json!({"Repo": "acme/shop-web"}));
    let merge = h.pending.pop_front().unwrap();
    assert_eq!(
        merge.parameters,
        json!({"Repo": "acme/shop-web", "Number": 1, "Execution": "open-pr-green-ci-1", "State": "Merge"})
    );
    assert_eq!(merge.task_token, None);
    h.send(Event::ResourceResult {
        key: merge.idempotency_key,
        outcome: ResourceOutcome::Failed {
            error: Some("GitHub.Conflict".to_string()),
            cause: Some("merge conflict in cart.rb".to_string()),
        },
    });
    let run = h.run();
    assert_eq!(run.status, SkillRunStatus::Succeeded);
    assert_eq!(
        run.output,
        Some(json!({
            "Repo": "acme/shop-web",
            "error": {"Error": "GitHub.Conflict", "Cause": "merge conflict in cart.rb"}
        }))
    );
    let entry = &h.entry("Merge")[0];
    assert_eq!(entry.status, StateEntryStatus::Caught);
    assert_eq!(entry.next_state.as_deref(), Some("Recover"));
    assert_eq!(entry.error.as_deref(), Some("GitHub.Conflict"));
}

#[test]
fn states_all_catches_timeouts() {
    let mut h = started(CATCHING, json!({"Repo": "acme/shop-web"}));
    h.pending.clear();
    let (timer, _) = h.fire_next_timer().unwrap();
    assert!(matches!(timer, Timer::TaskTimeout { .. }));
    let run = h.run();
    assert_eq!(run.status, SkillRunStatus::Succeeded);
    assert_eq!(run.output.unwrap()["other"]["Error"], "States.Timeout");
}

#[test]
fn runtime_errors_are_not_caught() {
    let yaml = r#"
Inputs: {}
StartAt: Merge
TimeoutSeconds: 600
States:
  Merge:
    Type: Task
    Resource: sanchoris:::github:mergePullRequest
    Parameters: { Repo.$: $.Missing }
    TimeoutSeconds: 30
    Catch:
      - ErrorEquals: [States.ALL]
        Next: Done
    Next: Done
  Done:
    Type: Succeed
"#;
    let h = started(yaml, json!({}));
    let run = h.run();
    assert_eq!(run.status, SkillRunStatus::Failed);
    assert_eq!(run.error.as_deref(), Some("States.Runtime"));
    assert!(h.pending.is_empty());
    assert_eq!(h.entry("Merge")[0].status, StateEntryStatus::Failed);
}

#[test]
fn choice_without_a_match_fails_with_no_choice_matched() {
    let yaml = r#"
Inputs: { Flag: boolean }
StartAt: Pick
TimeoutSeconds: 600
States:
  Pick:
    Type: Choice
    Choices:
      - Variable: $.Flag
        BooleanEquals: true
        Next: Done
  Done:
    Type: Succeed
"#;
    let h = started(yaml, json!({"Flag": false}));
    let run = h.run();
    assert_eq!(run.error.as_deref(), Some("States.NoChoiceMatched"));
    assert_eq!(h.entry("Pick")[0].status, StateEntryStatus::Failed);
    let h = started(yaml, json!({"Flag": true}));
    assert_eq!(h.run().status, SkillRunStatus::Succeeded);
}

#[test]
fn endless_inline_loops_fail_the_run() {
    let yaml = r#"
Inputs: {}
StartAt: Spin
TimeoutSeconds: 600
States:
  Spin:
    Type: Pass
    Next: Spin
"#;
    let h = started(yaml, json!({}));
    let run = h.run();
    assert_eq!(run.status, SkillRunStatus::Failed);
    assert_eq!(run.error.as_deref(), Some("States.Runtime"));
    assert_eq!(
        run.next_sequence as usize,
        sanchoris_engine::MAX_INLINE_TRANSITIONS + 1
    );
}

#[test]
fn approval_waits_for_its_task_token() {
    let yaml = r#"
Inputs: {}
StartAt: Ask
TimeoutSeconds: 600
States:
  Ask:
    Type: Task
    Resource: sanchoris:::approval:request.waitForTaskToken
    Parameters: { Message: Merge?, TaskToken.$: $$.Task.Token }
    TimeoutSeconds: 300
    ResultPath: $.approval
    End: true
"#;
    let mut h = started(yaml, json!({}));
    assert_eq!(h.run().status, SkillRunStatus::Waiting);
    let ask = h.pending.pop_front().unwrap();
    let token = ask.task_token.clone().unwrap();
    assert_eq!(token, "sanchoris-task:1:1:1");
    assert_eq!(h.token(), token);
    h.send(Event::TaskFailure {
        task_token: token,
        error: Some("Approval.Rejected".to_string()),
        cause: None,
    });
    let run = h.run();
    assert_eq!(run.error.as_deref(), Some("Approval.Rejected"));
    assert_eq!(
        h.notifications.last().unwrap().text,
        "[sanchoris] Skill open-pr-green-ci (run 1) failed: Approval.Rejected."
    );
}

#[test]
fn start_rejects_invalid_input() {
    let definition = parse_skill_yaml(CATCHING).unwrap();
    let request = |input| StartRequest {
        skill_run_id: 1,
        skill_name: "s".to_string(),
        input,
    };
    for input in [
        json!({}),
        json!({"Repo": 1}),
        json!({"Repo": "a/b", "Extra": 1}),
        json!([]),
    ] {
        assert!(matches!(
            start(&definition, request(input), epoch()),
            Err(EngineError::InvalidInput(_))
        ));
    }
}
