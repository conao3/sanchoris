use crate::common;

use common::{REPO, SHOP_WEB_ID, World, skill_input};
use sanchoris_core::items::{ApprovalWait, Output, StateEntryItem, Task, TimerItem};
use sanchoris_core::keys;
use serde_json::json;
use uuid::Uuid;

async fn skill_run(world: &World, task_id: Uuid, number: i32) -> (String, Option<String>) {
    common::skill_run(&world.store, world.tenant(), task_id, number).await
}

async fn entries(world: &World, skill_run_id: Uuid) -> Vec<StateEntryItem> {
    let mut entries = world
        .dynamo
        .all::<StateEntryItem>()
        .await
        .into_iter()
        .filter(|e| e.skill_run_id == skill_run_id)
        .collect::<Vec<_>>();
    entries.sort_by_key(|e| e.sequence);
    entries
}

#[tokio::test]
async fn skill_success_completes_the_task_after_the_turn_end() {
    let world = World::new("pro").await;
    world.github.script_checks(&["success"]);
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    let accepted = world.request_skill(run_id, skill_input(35)).await;
    assert_eq!(
        accepted.response(),
        json!({ "RunId": 1, "Status": "RUNNING" })
    );
    world.drive().await;

    assert_eq!(
        skill_run(&world, task.id, 1).await,
        ("SUCCEEDED".into(), None)
    );
    let messages = world.messages(run_id).await;
    assert_eq!(messages.len(), 1);
    assert!(
        messages[0].starts_with("[sanchoris] Skill open-pr-green-ci (run 1) succeeded.\nOutput: "),
        "{}",
        messages[0]
    );
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "RUNNING");

    world.turn_end(run_id, 1).await;
    world.worker.settle().await.unwrap();
    let task = world.task(&format!("{REPO}#35")).await;
    assert_eq!(task.status, "SUCCEEDED");
    assert_eq!(world.outbox(run_id, "shutdown").await.len(), 1);

    {
        let state = world.github.state.lock().unwrap();
        assert_eq!(state.created_pulls.len(), 1);
        assert_eq!(state.created_comments, vec![(REPO.to_string(), 35)]);
        let comment = &state.comments[&(REPO.to_string(), 35)][0];
        assert!(
            comment
                .body
                .starts_with("Pull requests are open and the checks pass: ")
        );
        assert!(
            comment
                .body
                .contains(&format!("<!-- sanchoris:{}:Report -->", accepted.id))
        );
    }

    let path = entries(&world, accepted.id)
        .await
        .into_iter()
        .map(|e| e.state_name)
        .collect::<Vec<_>>();
    assert_eq!(
        path,
        ["Init", "Push", "OpenPRs", "WaitChecks", "Green", "Report"]
    );
    let mut outputs = world
        .dynamo
        .all::<Output>()
        .await
        .into_iter()
        .filter(|o| o.skill_run_id == accepted.id)
        .map(|o| (o.kind, o.number))
        .collect::<Vec<_>>();
    outputs.sort();
    assert_eq!(
        outputs,
        vec![("comment".into(), 35), ("pull_request".into(), 52)]
    );
    assert!(world.dynamo.all::<TimerItem>().await.is_empty());
    world.worker.settle().await.unwrap();
    assert_eq!(*world.runner.stopped.lock().unwrap(), vec![run_id]);
}

#[tokio::test]
async fn ci_failure_hands_off_to_the_agent_and_continues_to_success() {
    let world = World::new("pro").await;
    world.github.script_checks(&["failure", "success"]);
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world.request_skill(run_id, skill_input(35)).await;
    world.turn_end(run_id, 1).await;
    world.drive().await;

    assert_eq!(skill_run(&world, task.id, 1).await.0, "WAITING_AGENT");
    let messages = world.messages(run_id).await;
    assert_eq!(
        messages,
        vec![format!(
            "[sanchoris] Skill open-pr-green-ci (run 1) needs you: CI failed on {REPO}#52 (ci/test). Fix it, commit, and continue this skill. Log: ci/test: failure\ncheckout_spec::flaky failed\nWhen you are done, run `sanchoris skill continue 1`."
        )]
    );
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "RUNNING");

    let status = world.continue_skill(run_id, 1).await;
    assert_eq!(status["Status"], "RUNNING");
    world.turn_end(run_id, 2).await;
    world.worker.settle().await.unwrap();
    let waiting = world.task(&format!("{REPO}#35")).await;
    assert_eq!(
        (waiting.status.as_str(), waiting.wait_reason.as_deref()),
        ("WAITING", Some("skill"))
    );
    world.drive().await;
    assert_eq!(skill_run(&world, task.id, 1).await.0, "SUCCEEDED");
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "RUNNING");
    world.turn_end(run_id, 3).await;
    world.worker.settle().await.unwrap();
    assert_eq!(world.task(&format!("{REPO}#35")).await.status, "SUCCEEDED");
    let state = world.github.state.lock().unwrap();
    assert_eq!(state.created_pulls.len(), 1);
    assert_eq!(
        state.pushes[&(REPO.to_string(), format!("sanchoris/{}", task.id))],
        2
    );
}

#[tokio::test]
async fn exhausted_fixes_put_the_task_in_needs_you() {
    let world = World::new("pro").await;
    world
        .github
        .script_checks(&["failure", "failure", "failure", "failure"]);
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world.request_skill(run_id, skill_input(35)).await;
    world.drive().await;
    for _ in 0..3 {
        assert_eq!(skill_run(&world, task.id, 1).await.0, "WAITING_AGENT");
        world.continue_skill(run_id, 1).await;
        world.drive().await;
    }
    assert_eq!(
        skill_run(&world, task.id, 1).await,
        ("FAILED".into(), Some("Skill.FixesExhausted".into()))
    );
    let task = world.task(&format!("{REPO}#35")).await;
    assert_eq!(task.status, "NEEDS_YOU");
    assert_eq!(task.error.as_deref(), Some("Skill.FixesExhausted"));
    assert_eq!(
        task.cause.as_deref(),
        Some("The required skill open-pr-green-ci failed: The checks still fail after 3 fixes")
    );
    let last = world.messages(run_id).await.pop().unwrap();
    assert_eq!(
        last,
        "[sanchoris] Skill open-pr-green-ci (run 1) failed: Skill.FixesExhausted.\nCause: The checks still fail after 3 fixes"
    );
}

#[tokio::test]
async fn skill_requests_are_validated_against_the_task() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    let unknown = world
        .try_request_skill(run_id, "deploy", json!({}))
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), "skill_not_offered");
    let bad_input = world
        .try_request_skill(run_id, "open-pr-green-ci", json!({ "Repos": REPO }))
        .await
        .unwrap_err();
    assert_eq!(bad_input.code(), "invalid_input");
    assert!(world.skill_runs().await.is_empty());
}

#[tokio::test]
async fn check_run_webhooks_wake_the_wait_before_the_poll_interval() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world.request_skill(run_id, skill_input(35)).await;
    world.drive().await;
    assert_eq!(skill_run(&world, task.id, 1).await.0, "WAITING");
    world.github.state.lock().unwrap().check_results.clear();
    world.github.script_checks(&["success"]);
    world.worker.settle().await.unwrap();
    assert_eq!(skill_run(&world, task.id, 1).await.0, "WAITING");

    let sha = world
        .github
        .state
        .lock()
        .unwrap()
        .pulls
        .values()
        .next()
        .unwrap()
        .head_sha
        .clone();
    world
        .deliver(
            "delivery-check",
            "check_run",
            &json!({
                "action": "completed",
                "repository": { "id": SHOP_WEB_ID, "full_name": REPO },
                "installation": { "id": common::INSTALLATION },
                "check_run": {
                    "id": 7, "name": "ci/test", "status": "completed", "conclusion": "success",
                    "head_sha": sha, "html_url": "https://ci.example/7", "app": { "slug": "actions" },
                    "pull_requests": [{ "number": 52 }],
                },
                "sender": { "login": "github-actions[bot]", "id": 41898282, "type": "Bot" },
            }),
        )
        .await;
    world.drive().await;
    assert_eq!(skill_run(&world, task.id, 1).await.0, "SUCCEEDED");
}

#[tokio::test]
async fn polling_completes_the_checks_wait_without_a_webhook() {
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    world.request_skill(run_id, skill_input(35)).await;
    world.drive().await;
    assert_eq!(skill_run(&world, task.id, 1).await.0, "WAITING");
    world.github.state.lock().unwrap().check_results.clear();
    world.github.script_checks(&["success"]);
    world.clock.advance(chrono::Duration::minutes(5));
    world.drive().await;
    assert_eq!(skill_run(&world, task.id, 1).await.0, "SUCCEEDED");
}

#[tokio::test]
async fn failed_pushes_are_retried_by_the_engine() {
    let world = World::new("pro").await;
    world.github.script_checks(&["success"]);
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    let accepted = world.request_skill(run_id, skill_input(35)).await;
    world.worker.settle().await.unwrap();
    assert_eq!(world.answer_execs(false).await, 1);
    world.worker.settle().await.unwrap();
    let entry = entries(&world, accepted.id)
        .await
        .into_iter()
        .find(|e| e.state_name == "Push")
        .unwrap();
    assert_eq!(entry.status, "RUNNING");
    assert_eq!(entry.error.as_deref(), Some("States.TaskFailed"));
    world.clock.advance(chrono::Duration::seconds(10));
    world.drive().await;
    assert_eq!(skill_run(&world, task.id, 1).await.0, "SUCCEEDED");
    let execs = world.outbox(run_id, "exec").await;
    assert_eq!(execs.len(), 2);
    assert_eq!(
        execs[0]["commands"],
        json!([format!(
            "git -C 'shop-web' push origin 'HEAD:refs/heads/sanchoris/{}'",
            task.id
        )])
    );
}

#[tokio::test]
async fn approval_requests_wait_for_a_signed_decision() {
    use sanchoris_core::tokens::{Signer, TaskTokenSigner};
    let world = World::new("pro").await;
    let task = world.started_task(35).await;
    let run_id = task.run_id.unwrap();
    let definition = sanchoris_config::skill::parse_skill_yaml(
        "Inputs: {}\nStartAt: Ask\nTimeoutSeconds: 600\nStates:\n  Ask:\n    Type: Task\n    Resource: sanchoris:::approval:request.waitForTaskToken\n    Parameters: { Message: Ship it?, TaskToken.$: $$.Task.Token }\n    TimeoutSeconds: 300\n    End: true\n",
    )
    .unwrap();
    let skill = sanchoris_config::Skill {
        name: "ship".into(),
        description: "Ship".into(),
        body: String::new(),
        skill_md: String::new(),
        skill_yaml: Some(String::new()),
        definition: Some(definition),
        origin: sanchoris_config::SkillOrigin::Repository,
    };
    world
        .dynamo
        .modify::<Task>(&keys::task_key(world.tenant(), task.id), |stored| {
            stored.definition["Skills"]
                .as_array_mut()
                .unwrap()
                .push(json!({ "Name": "ship", "Definition": skill }));
        })
        .await;
    let accepted = world
        .try_request_skill(run_id, "ship", json!({}))
        .await
        .unwrap();
    world.worker.settle().await.unwrap();
    let waiting = world.task(&format!("{REPO}#35")).await;
    assert_eq!(waiting.status, "NEEDS_YOU");
    assert_eq!(waiting.error.as_deref(), Some("Approval.Requested"));
    let token = world
        .dynamo
        .all::<ApprovalWait>()
        .await
        .into_iter()
        .find(|w| w.skill_run_id == accepted.id)
        .unwrap()
        .task_token;
    let signer = TaskTokenSigner::new(Signer::new(&[9u8; 32]).unwrap());
    let signed = signer.sign(accepted.id, &token);
    let mut tx = world.store.tx(world.tenant());
    let last = if signed.ends_with('0') { '1' } else { '0' };
    let forged = format!("{}{last}", &signed[..signed.len() - 1]);
    assert!(
        sanchoris_core::skillrun::resolve_approval(
            &mut tx,
            &signer,
            &forged,
            true,
            "Mika",
            world.clock_now()
        )
        .await
        .is_err()
    );
    sanchoris_core::skillrun::resolve_approval(
        &mut tx,
        &signer,
        &signed,
        true,
        "Mika",
        world.clock_now(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(skill_run(&world, task.id, 1).await.0, "SUCCEEDED");
    let task = world.task(&format!("{REPO}#35")).await;
    assert_eq!(task.status, "RUNNING");
}
