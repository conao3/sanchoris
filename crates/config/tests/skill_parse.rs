use sanchoris_config::skill::{ResultPath, State, parse_skill_yaml};

#[test]
fn parses_open_pr_green_ci() {
    let def = parse_skill_yaml(include_str!("fixtures/open-pr-green-ci.skill.yaml")).unwrap();
    assert_eq!(def.start_at, "Init");
    assert_eq!(def.states.len(), 9);
    let State::Task(hand_off) = &def.states["HandOff"] else {
        panic!()
    };
    assert_eq!(hand_off.result_path, ResultPath::Discard);
    assert_eq!(hand_off.heartbeat_seconds, Some(900));
    let State::Task(push) = &def.states["Push"] else {
        panic!()
    };
    assert_eq!(push.result_path, ResultPath::Root);
}

#[test]
fn rejects_task_without_timeout() {
    let text = "Inputs: {}\nStartAt: A\nTimeoutSeconds: 10\nStates:\n  A:\n    Type: Task\n    Resource: sanchoris:::git:push\n    End: true\n";
    let err = parse_skill_yaml(text).unwrap_err().to_string();
    assert!(err.contains("TimeoutSeconds"), "{err}");
}
