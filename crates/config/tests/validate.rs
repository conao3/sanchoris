use sanchoris_config::{
    Architecture, Compute, ConfigFiles, PlanLimits, SkillOrigin, TriggerSource, ValidationContext,
    ValidationErrors, builtin_skill, builtin_skills, load_repository, parse_repository,
    read_config_dir,
};
use std::collections::BTreeSet;

const EXAMPLE: &str = include_str!("fixtures/example.sanchoris.yaml");
const EXAMPLE_SKILL_YAML: &str = include_str!("fixtures/open-pr-green-ci.skill.yaml");
const EXAMPLE_SKILL_MD: &str = include_str!("../skills/open-pr-green-ci/SKILL.md");

fn plan() -> PlanLimits {
    PlanLimits {
        max_turns: 200,
        max_cost_usd: 100.0,
        max_timeout_seconds: 900_000,
        max_idle_seconds: 3600,
        max_children_depth: 3,
        max_children_count: 10,
        computes: BTreeSet::from([Compute::Standard, Compute::Docker]),
    }
}

fn context() -> ValidationContext {
    ValidationContext {
        credentials: BTreeSet::from(["acme-claude-team".to_string()]),
        plan: plan(),
    }
}

fn example_files(config: &str) -> ConfigFiles {
    ConfigFiles::from([
        ("sanchoris.yaml".to_string(), config.to_string()),
        (
            "skills/open-pr-green-ci/SKILL.md".to_string(),
            EXAMPLE_SKILL_MD.to_string(),
        ),
        (
            "skills/open-pr-green-ci/skill.yaml".to_string(),
            EXAMPLE_SKILL_YAML.to_string(),
        ),
    ])
}

fn replaced(from: &str, to: &str) -> String {
    assert!(EXAMPLE.contains(from), "fixture does not contain {from:?}");
    EXAMPLE.replacen(from, to, 1)
}

fn errors_with(files: &ConfigFiles, context: &ValidationContext) -> ValidationErrors {
    load_repository(files, context).expect_err("validation should fail")
}

fn assert_error(errors: &ValidationErrors, location: &str) {
    assert!(
        errors.has(location),
        "no error at {location}; got:\n{errors}"
    );
}

fn config_error(config: &str, location: &str) -> ValidationErrors {
    let errors = errors_with(&example_files(config), &context());
    assert_error(&errors, location);
    errors
}

fn skill_files(skill_yaml: &str) -> ConfigFiles {
    let mut files = example_files(EXAMPLE);
    files.insert(
        "skills/custom/SKILL.md".to_string(),
        "---\nname: custom\ndescription: A custom skill.\n---\nRun it.\n".to_string(),
    );
    files.insert(
        "skills/custom/skill.yaml".to_string(),
        skill_yaml.to_string(),
    );
    files
}

fn skill_error(skill_yaml: &str, location: &str) -> ValidationErrors {
    let errors = errors_with(&skill_files(skill_yaml), &context());
    assert_error(&errors, location);
    errors
}

const MINIMAL_SKILL: &str = "Inputs: {}\nStartAt: A\nTimeoutSeconds: 60\nStates:\n  A:\n    Type: Pass\n    Next: B\n  B:\n    Type: Succeed\n";

#[test]
fn example_config_with_local_skill_is_valid() {
    let repo = load_repository(&example_files(EXAMPLE), &context()).unwrap();
    assert_eq!(repo.config.triggers.len(), 3);
    assert_eq!(repo.config.sessions.len(), 2);
    let skill = repo.skill("open-pr-green-ci").unwrap();
    assert_eq!(skill.origin, SkillOrigin::Repository);
    assert!(skill.is_reliable());
    let TriggerSource::EventPattern(pattern) = &repo.config.triggers["labeled-issue"].source else {
        panic!()
    };
    assert_eq!(pattern["detail"]["sender"]["login"][2], "sanchoris[bot]");
}

#[test]
fn example_config_with_builtin_skill_is_valid() {
    let files = ConfigFiles::from([("sanchoris.yaml".to_string(), EXAMPLE.to_string())]);
    let repo = load_repository(&files, &context()).unwrap();
    assert_eq!(
        repo.skill("open-pr-green-ci").unwrap().origin,
        SkillOrigin::BuiltIn
    );
}

#[test]
fn config_round_trips_through_json() {
    let repo = load_repository(&example_files(EXAMPLE), &context()).unwrap();
    let json = serde_json::to_value(&repo.config).unwrap();
    assert_eq!(json["Sessions"]["shop"]["Project"], "shop");
    assert_eq!(json["Sessions"]["all"]["Repos"][0], "acme/idea");
    assert_eq!(
        json["Sessions"]["shop"]["GitHub"]["Permissions"]["checks"],
        "read"
    );
    let back: sanchoris_config::Config = serde_json::from_value(json).unwrap();
    assert_eq!(back, repo.config);
}

#[test]
fn builtin_skill_is_reliable_and_valid() {
    let skills = builtin_skills();
    assert_eq!(skills.len(), 1);
    let skill = builtin_skill("open-pr-green-ci").unwrap();
    assert!(skill.is_reliable());
    assert!(skill.body.contains("sanchoris skill run open-pr-green-ci"));
    let definition = skill.definition.unwrap();
    assert!(definition.inputs.contains_key("Repo"));
    let errors =
        sanchoris_config::validate_skill_definition(&definition, "Skills.open-pr-green-ci", 14_400);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(!skill.skill_yaml.unwrap().contains("acme/shop-web"));
}

#[test]
fn reads_config_from_directory() {
    let root = std::env::temp_dir().join(format!("sanchoris-config-{}", std::process::id()));
    let skill_dir = root.join("skills/open-pr-green-ci");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(root.join("sanchoris.yaml"), EXAMPLE).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), EXAMPLE_SKILL_MD).unwrap();
    std::fs::write(skill_dir.join("skill.yaml"), EXAMPLE_SKILL_YAML).unwrap();
    std::fs::write(skill_dir.join("notes.txt"), "ignored").unwrap();
    let files = read_config_dir(&root).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(files.len(), 3);
    let repo = load_repository(&files, &context()).unwrap();
    assert_eq!(
        repo.skill("open-pr-green-ci").unwrap().origin,
        SkillOrigin::Repository
    );
}

#[test]
fn missing_config_file_fails() {
    let errors = parse_repository(&ConfigFiles::new()).unwrap_err();
    assert_error(&errors, "sanchoris.yaml");
}

#[test]
fn missing_trigger_field_fails() {
    config_error(
        &replaced("      MaxNudges: 2\n", ""),
        "Triggers.labeled-issue.Start.MaxNudges",
    );
}

#[test]
fn missing_children_limit_fails() {
    config_error(
        &replaced(
            "HeartbeatSeconds: 900, Children: { MaxDepth: 1, MaxCount: 5 } }\n  chat-all",
            "HeartbeatSeconds: 900 }\n  chat-all",
        ),
        "Triggers.labeled-issue.Start.Limits.Children",
    );
}

#[test]
fn missing_session_fields_fail() {
    let config = replaced(
        "    Network: { Allow: [github.com, registry.npmjs.org] }\n    UsageLimit: { Resume: auto }\n",
        "",
    );
    let errors = config_error(&config, "Sessions.shop.Network");
    assert_error(&errors, "Sessions.shop.UsageLimit");
}

#[test]
fn missing_worker_credential_fails() {
    config_error(
        &replaced(
            "Model: claude-sonnet-5-5, Credential: acme-claude-team }\n    Project: shop",
            "Model: claude-sonnet-5-5 }\n    Project: shop",
        ),
        "Sessions.shop.Worker.Credential",
    );
}

#[test]
fn missing_project_and_repos_fails() {
    config_error(
        &replaced("    Project: shop\n", ""),
        "Sessions.shop.Project",
    );
}

#[test]
fn unknown_field_fails() {
    config_error(
        &replaced(
            "    UsageLimit: { Resume: auto }\n",
            "    UsageLimit: { Resume: auto, Retry: 3 }\n",
        ),
        "Sessions.shop.UsageLimit.Retry",
    );
}

#[test]
fn reports_every_error_at_once() {
    let config = replaced("      MaxNudges: 2\n", "").replacen(
        "Credential: acme-claude-team",
        "Credential: missing-credential",
        1,
    );
    let errors = errors_with(&example_files(&config), &context());
    assert_error(&errors, "Triggers.labeled-issue.Start.MaxNudges");
    let config = replaced(
        "      Session: shop\n      Instruction: 'Resolve",
        "      Session: nope\n      Instruction: 'Resolve",
    )
    .replacen(
        "Credential: acme-claude-team",
        "Credential: missing-credential",
        1,
    );
    let errors = errors_with(&example_files(&config), &context());
    assert_error(&errors, "Triggers.labeled-issue.Start.Session");
    assert_error(&errors, "Sessions.shop.Worker.Credential");
}

#[test]
fn unknown_session_fails() {
    config_error(
        &replaced(
            "      Session: shop\n      Instruction: 'Resolve",
            "      Session: shopx\n      Instruction: 'Resolve",
        ),
        "Triggers.labeled-issue.Start.Session",
    );
}

#[test]
fn unknown_skill_fails() {
    config_error(
        &replaced(
            "      Skills: [open-pr-green-ci]\n      RequiredSkills: []",
            "      Skills: [no-such-skill]\n      RequiredSkills: []",
        ),
        "Triggers.chat-shop.Start.Skills[0]",
    );
}

#[test]
fn unknown_required_skill_fails() {
    config_error(
        &replaced(
            "      RequiredSkills: [open-pr-green-ci]",
            "      RequiredSkills: [no-such-skill]",
        ),
        "Triggers.labeled-issue.Start.RequiredSkills[0]",
    );
}

#[test]
fn unknown_credential_fails() {
    let errors = errors_with(
        &example_files(EXAMPLE),
        &ValidationContext {
            credentials: BTreeSet::new(),
            plan: plan(),
        },
    );
    assert_error(&errors, "Sessions.shop.Worker.Credential");
    assert_error(&errors, "Sessions.all.Worker.Credential");
}

#[test]
fn requester_credential_is_accepted() {
    let config = EXAMPLE.replace("Credential: acme-claude-team", "Credential: requester");
    let context = ValidationContext {
        credentials: BTreeSet::new(),
        plan: plan(),
    };
    load_repository(&example_files(&config), &context).unwrap();
}

#[test]
fn unknown_project_fails() {
    config_error(
        &replaced("    Project: shop\n", "    Project: store\n"),
        "Sessions.shop.Project",
    );
}

#[test]
fn missing_actor_condition_fails() {
    config_error(
        &replaced(
            "        sender: { login: [mika-dev, sota-dev, \"sanchoris[bot]\"] }\n",
            "",
        ),
        "Triggers.labeled-issue.EventPattern.detail",
    );
}

#[test]
fn author_association_satisfies_actor_condition() {
    let config = replaced(
        "        sender: { login: [mika-dev, sota-dev, \"sanchoris[bot]\"] }\n",
        "        issue: { author_association: [OWNER, MEMBER] }\n",
    );
    load_repository(&example_files(&config), &context()).unwrap();
}

const SENDER_LINE: &str = "        sender: { login: [mika-dev, sota-dev, \"sanchoris[bot]\"] }\n";

fn comment_trigger(actor: &str) -> String {
    replaced(
        &format!(
            "      detail-type: [Issue Labeled]\n      detail:\n        repository: [frontend, backend]\n        label: [sanchoris]\n{SENDER_LINE}"
        ),
        &format!(
            "      detail-type: [Issue Comment Created]\n      detail:\n        repository: [frontend, backend]\n{actor}"
        ),
    )
}

#[test]
fn comment_triggers_need_the_commenter_as_actor() {
    let errors = config_error(
        &comment_trigger("        issue: { author_association: [OWNER, MEMBER] }\n"),
        "Triggers.labeled-issue.EventPattern.detail",
    );
    assert!(
        errors.to_string().contains(
            "Issue Comment Created triggers must constrain the actor of the event with a list of exact values in detail.sender.login or detail.comment.author_association"
        ),
        "{errors}"
    );
    for actor in [
        "        comment: { author_association: [OWNER, MEMBER] }\n",
        "        sender: { login: [mika-dev] }\n",
    ] {
        load_repository(&example_files(&comment_trigger(actor)), &context()).unwrap();
    }
}

#[test]
fn actor_conditions_accept_only_exact_values() {
    for (actor, location) in [
        (
            "        sender: { login: [{ exists: true }] }\n",
            "Triggers.labeled-issue.EventPattern.detail.sender.login",
        ),
        (
            "        sender: { login: [{ prefix: \"\" }] }\n",
            "Triggers.labeled-issue.EventPattern.detail.sender.login",
        ),
        (
            "        sender: { login: [{ anything-but: x }] }\n",
            "Triggers.labeled-issue.EventPattern.detail.sender.login",
        ),
        (
            "        sender: { login: [mika-dev, { prefix: m }] }\n",
            "Triggers.labeled-issue.EventPattern.detail.sender.login",
        ),
        (
            "        issue: { author_association: [{ anything-but: NONE }] }\n",
            "Triggers.labeled-issue.EventPattern.detail.issue.author_association",
        ),
        (
            "        sender: { login: [] }\n",
            "Triggers.labeled-issue.EventPattern.detail.sender.login",
        ),
    ] {
        let errors = config_error(&replaced(SENDER_LINE, actor), location);
        assert_error(&errors, "Triggers.labeled-issue.EventPattern.detail");
    }
    let errors = config_error(
        &comment_trigger("        comment: { author_association: [{ numeric: [\">\", 0] }] }\n"),
        "Triggers.labeled-issue.EventPattern.detail.comment.author_association",
    );
    assert_error(&errors, "Triggers.labeled-issue.EventPattern.detail");
}

#[test]
fn missing_github_repository_fails() {
    config_error(
        &replaced("        repository: [frontend, backend]\n", ""),
        "Triggers.labeled-issue.EventPattern.detail.repository",
    );
}

#[test]
fn web_trigger_needs_scope() {
    config_error(
        &replaced(
            "      detail:\n        scope: [all]\n",
            "      detail: {}\n",
        ),
        "Triggers.chat-all.EventPattern.detail.scope",
    );
}

#[test]
fn insufficient_permissions_fail() {
    let errors = config_error(
        &replaced(
            "Permissions: { contents: write, pull_requests: write, issues: write, checks: read }",
            "Permissions: { contents: write, pull_requests: read, issues: write }",
        ),
        "Triggers.labeled-issue.Start.Skills[0]",
    );
    let messages: Vec<String> = errors
        .at("Triggers.labeled-issue.Start.Skills[0]")
        .iter()
        .map(|e| e.message.clone())
        .collect();
    assert!(
        messages.iter().any(|m| m.contains("pull_requests: write")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("checks: read")),
        "{messages:?}"
    );
    assert_error(&errors, "Triggers.chat-shop.Start.Skills[0]");
}

#[test]
fn skill_repository_outside_github_repos_fails() {
    config_error(
        &replaced(
            "      Repos: [acme/shop-web, acme/shop-api]\n",
            "      Repos: [acme/shop-api]\n",
        ),
        "Triggers.labeled-issue.Start.Skills[0]",
    );
}

#[test]
fn cloned_repository_outside_github_repos_fails() {
    config_error(
        &replaced(
            "      Repos: [\"acme/*\"]\n",
            "      Repos: [\"other/*\"]\n",
        ),
        "Sessions.all.GitHub.Repos",
    );
}

#[test]
fn required_skill_without_skill_yaml_fails() {
    let mut files = example_files(EXAMPLE);
    files.insert(
        "skills/notes/SKILL.md".to_string(),
        "---\nname: notes\ndescription: Take notes.\n---\nWrite notes.\n".to_string(),
    );
    let config = replaced(
        "      Skills: [open-pr-green-ci]\n      RequiredSkills: [open-pr-green-ci]",
        "      Skills: [open-pr-green-ci, notes]\n      RequiredSkills: [notes]",
    );
    files.insert("sanchoris.yaml".to_string(), config);
    let errors = errors_with(&files, &context());
    assert_error(&errors, "Triggers.labeled-issue.Start.RequiredSkills[0]");
    assert!(!errors.has("Triggers.labeled-issue.Start.Skills[1]"));
}

#[test]
fn required_skill_must_be_in_skills() {
    config_error(
        &replaced(
            "      Skills: [open-pr-green-ci]\n      RequiredSkills: [open-pr-green-ci]",
            "      Skills: []\n      RequiredSkills: [open-pr-green-ci]",
        ),
        "Triggers.labeled-issue.Start.RequiredSkills[0]",
    );
}

#[test]
fn plan_limits_are_enforced() {
    let context = ValidationContext {
        credentials: BTreeSet::from(["acme-claude-team".to_string()]),
        plan: PlanLimits {
            max_turns: 50,
            max_cost_usd: 15.0,
            max_timeout_seconds: 36_000,
            max_idle_seconds: 600,
            max_children_depth: 0,
            max_children_count: 2,
            computes: BTreeSet::from([Compute::Standard]),
        },
    };
    let config = replaced(
        "    Workspace: { Compute: standard, Architecture: arm64, IdleSeconds: 900 }\n  all:",
        "    Workspace: { Compute: docker, Architecture: arm64, IdleSeconds: 900 }\n  all:",
    );
    let errors = errors_with(&example_files(&config), &context);
    for location in [
        "Triggers.labeled-issue.Start.Limits.Turns",
        "Triggers.labeled-issue.Start.Limits.CostUsd",
        "Triggers.labeled-issue.Start.Limits.TimeoutSeconds",
        "Triggers.labeled-issue.Start.Limits.Children.MaxDepth",
        "Triggers.labeled-issue.Start.Limits.Children.MaxCount",
        "Triggers.chat-all.Start.Limits.TimeoutSeconds",
        "Sessions.shop.Workspace.Compute",
        "Sessions.shop.Workspace.IdleSeconds",
        "Sessions.all.Workspace.IdleSeconds",
    ] {
        assert_error(&errors, location);
    }
    assert!(!errors.has("Triggers.chat-all.Start.Limits.CostUsd"));
}

#[test]
fn the_workspace_architecture_is_required_and_docker_runs_on_arm64() {
    config_error(
        &replaced(
            "    Workspace: { Compute: standard, Architecture: arm64, IdleSeconds: 900 }\n  all:",
            "    Workspace: { Compute: standard, IdleSeconds: 900 }\n  all:",
        ),
        "Sessions.shop.Workspace.Architecture",
    );
    config_error(
        &replaced(
            "    Workspace: { Compute: standard, Architecture: arm64, IdleSeconds: 900 }\n  all:",
            "    Workspace: { Compute: standard, Architecture: i386, IdleSeconds: 900 }\n  all:",
        ),
        "Sessions.shop.Workspace.Architecture",
    );
    let x86 = replaced(
        "    Workspace: { Compute: standard, Architecture: arm64, IdleSeconds: 900 }\n  all:",
        "    Workspace: { Compute: standard, Architecture: x86_64, IdleSeconds: 900 }\n  all:",
    );
    let repository = load_repository(&example_files(&x86), &context()).unwrap();
    assert_eq!(
        repository.config.sessions["shop"].workspace.architecture,
        Architecture::X86_64
    );
    let mut context = context();
    context.plan.computes.insert(Compute::Docker);
    let docker_x86 = replaced(
        "    Workspace: { Compute: standard, Architecture: arm64, IdleSeconds: 900 }\n  all:",
        "    Workspace: { Compute: docker, Architecture: x86_64, IdleSeconds: 900 }\n  all:",
    );
    let errors = errors_with(&example_files(&docker_x86), &context);
    assert_error(&errors, "Sessions.shop.Workspace.Architecture");
    let docker_arm = replaced(
        "    Workspace: { Compute: standard, Architecture: arm64, IdleSeconds: 900 }\n  all:",
        "    Workspace: { Compute: docker, Architecture: arm64, IdleSeconds: 900 }\n  all:",
    );
    load_repository(&example_files(&docker_arm), &context).unwrap();
}

#[test]
fn docker_compute_counts_three_times() {
    let config = replaced(
        "    Workspace: { Compute: standard, Architecture: arm64, IdleSeconds: 900 }\n  all:",
        "    Workspace: { Compute: docker, Architecture: arm64, IdleSeconds: 900 }\n  all:",
    );
    let mut context = context();
    context.plan.max_timeout_seconds = 86_400;
    let errors = errors_with(&example_files(&config), &context);
    assert_error(&errors, "Triggers.chat-shop.Start.Limits.TimeoutSeconds");
    assert!(!errors.has("Triggers.labeled-issue.Start.Limits.TimeoutSeconds"));
    assert!(!errors.has("Triggers.chat-all.Start.Limits.TimeoutSeconds"));
}

#[test]
fn schedule_trigger_is_valid() {
    let config = format!(
        "{EXAMPLE}  nightly:\n    State: ENABLED\n    ScheduleExpression: cron(0 18 ? * MON-FRI *)\n    Start:\n      Session: all\n      Instruction: Triage new issues.\n      Skills: []\n      RequiredSkills: []\n      MaxNudges: 0\n      Limits: {{ Turns: 10, CostUsd: 2, TimeoutSeconds: 3600, HeartbeatSeconds: 600, Children: {{ MaxDepth: 1, MaxCount: 5 }} }}\n"
    );
    load_repository(&example_files(&config), &context()).unwrap();
    let bad = config.replace("cron(0 18 ? * MON-FRI *)", "cron(0 18 * * MON-FRI *)");
    config_error(&bad, "Triggers.nightly.ScheduleExpression");
}

#[test]
fn event_pattern_and_schedule_are_exclusive() {
    config_error(
        &replaced(
            "    State: ENABLED\n    EventPattern:\n      source: [github]",
            "    State: ENABLED\n    ScheduleExpression: rate(1 hour)\n    EventPattern:\n      source: [github]",
        ),
        "Triggers.labeled-issue",
    );
}

#[test]
fn undefined_placeholder_fails() {
    config_error(
        &replaced(
            "Instruction: 'Resolve issue <number> in <repository>: <title>.",
            "Instruction: 'Resolve issue <number> in <repo>: <title>.",
        ),
        "Triggers.labeled-issue.Start.Instruction",
    );
}

#[test]
fn skill_directory_must_match_name() {
    let mut files = example_files(EXAMPLE);
    files.insert(
        "skills/other/SKILL.md".to_string(),
        "---\nname: different\ndescription: Mismatch.\n---\n".to_string(),
    );
    let errors = parse_repository(&files).unwrap_err();
    assert_error(&errors, "Skills.other.SKILL.md");
}

#[test]
fn skill_md_requires_description() {
    let mut files = example_files(EXAMPLE);
    files.insert(
        "skills/other/SKILL.md".to_string(),
        "---\nname: other\n---\nBody\n".to_string(),
    );
    let errors = parse_repository(&files).unwrap_err();
    assert_error(&errors, "Skills.other.SKILL.md");
}

#[test]
fn skill_yaml_missing_timeout_fails() {
    skill_error(
        &MINIMAL_SKILL.replace("TimeoutSeconds: 60\n", ""),
        "Skills.custom.skill.yaml",
    );
}

#[test]
fn minimal_skill_is_valid() {
    load_repository(&skill_files(MINIMAL_SKILL), &context()).unwrap();
}

#[test]
fn unreachable_state_fails() {
    skill_error(
        &format!("{MINIMAL_SKILL}  Orphan:\n    Type: Succeed\n"),
        "Skills.custom.States.Orphan",
    );
}

#[test]
fn unknown_next_fails() {
    skill_error(
        &MINIMAL_SKILL.replace("Next: B", "Next: C"),
        "Skills.custom.States.A.Next",
    );
}

#[test]
fn unknown_start_at_fails() {
    skill_error(
        &MINIMAL_SKILL.replace("StartAt: A", "StartAt: Z"),
        "Skills.custom.StartAt",
    );
}

#[test]
fn missing_terminal_fails() {
    skill_error(
        "Inputs: {}\nStartAt: A\nTimeoutSeconds: 60\nStates:\n  A:\n    Type: Pass\n    Next: B\n  B:\n    Type: Pass\n    Next: A\n",
        "Skills.custom.States",
    );
}

#[test]
fn next_and_end_together_fail() {
    skill_error(
        &MINIMAL_SKILL.replace("    Next: B\n", "    Next: B\n    End: true\n"),
        "Skills.custom.States.A",
    );
}

#[test]
fn wait_needs_one_of_seconds_or_timestamp() {
    let yaml = "Inputs: {}\nStartAt: W\nTimeoutSeconds: 60\nStates:\n  W:\n    Type: Wait\n    Seconds: 5\n    Timestamp: '2026-01-01T00:00:00Z'\n    End: true\n";
    skill_error(yaml, "Skills.custom.States.W");
}

fn task_skill(task: &str) -> String {
    format!(
        "Inputs: {{}}\nStartAt: T\nTimeoutSeconds: 600\nStates:\n  T:\n    Type: Task\n{task}    End: true\n"
    )
}

#[test]
fn malformed_resource_fails() {
    skill_error(
        &task_skill("    Resource: arn:aws:states:::lambda:invoke\n    TimeoutSeconds: 60\n"),
        "Skills.custom.States.T.Resource",
    );
}

#[test]
fn unknown_resource_fails() {
    skill_error(
        &task_skill(
            "    Resource: sanchoris:::github:deleteRepository\n    Parameters: { Repo: acme/x }\n    TimeoutSeconds: 60\n",
        ),
        "Skills.custom.States.T.Resource",
    );
}

#[test]
fn github_resource_needs_repo() {
    skill_error(
        &task_skill(
            "    Resource: sanchoris:::github:mergePullRequest\n    Parameters: { Number: 1 }\n    TimeoutSeconds: 60\n",
        ),
        "Skills.custom.States.T.Parameters.Repo",
    );
}

#[test]
fn git_push_needs_repos() {
    skill_error(
        &task_skill(
            "    Resource: sanchoris:::git:push\n    Parameters: {}\n    TimeoutSeconds: 60\n",
        ),
        "Skills.custom.States.T.Parameters.Repos",
    );
}

#[test]
fn handoff_needs_heartbeat() {
    skill_error(
        &task_skill(
            "    Resource: sanchoris:::agent:handoff.waitForTaskToken\n    Parameters: { Instruction: Fix it, TaskToken.$: $$.Task.Token }\n    TimeoutSeconds: 600\n",
        ),
        "Skills.custom.States.T.HeartbeatSeconds",
    );
}

#[test]
fn task_token_must_come_from_context() {
    skill_error(
        &task_skill(
            "    Resource: sanchoris:::approval:request.waitForTaskToken\n    Parameters: { Message: Approve, TaskToken: abc }\n    TimeoutSeconds: 600\n",
        ),
        "Skills.custom.States.T.Parameters.TaskToken.$",
    );
}

#[test]
fn states_all_must_be_last_and_alone() {
    let retry = "    Resource: sanchoris:::chat:post\n    Parameters: { Text: hi }\n    TimeoutSeconds: 60\n    Retry:\n      - ErrorEquals: [States.ALL]\n        IntervalSeconds: 1\n        MaxAttempts: 2\n        BackoffRate: 2\n      - ErrorEquals: [States.Timeout]\n        IntervalSeconds: 1\n        MaxAttempts: 2\n        BackoffRate: 2\n";
    skill_error(
        &task_skill(retry),
        "Skills.custom.States.T.Retry[0].ErrorEquals",
    );
    let alone = "    Resource: sanchoris:::chat:post\n    Parameters: { Text: hi }\n    TimeoutSeconds: 60\n    Retry:\n      - ErrorEquals: [States.Timeout, States.ALL]\n        IntervalSeconds: 1\n        MaxAttempts: 2\n        BackoffRate: 2\n";
    skill_error(
        &task_skill(alone),
        "Skills.custom.States.T.Retry[0].ErrorEquals",
    );
}

#[test]
fn custom_errors_must_not_use_states_prefix() {
    let yaml = "Inputs: {}\nStartAt: F\nTimeoutSeconds: 60\nStates:\n  F:\n    Type: Fail\n    Error: States.Custom\n    Cause: nope\n";
    skill_error(yaml, "Skills.custom.States.F.Error");
    let catch = "    Resource: sanchoris:::chat:post\n    Parameters: { Text: hi }\n    TimeoutSeconds: 60\n    Catch:\n      - ErrorEquals: [States.Oops]\n        Next: T\n";
    skill_error(
        &task_skill(catch),
        "Skills.custom.States.T.Catch[0].ErrorEquals[0]",
    );
}

#[test]
fn heartbeat_must_be_below_timeout() {
    skill_error(
        &task_skill(
            "    Resource: sanchoris:::agent:handoff.waitForTaskToken\n    Parameters: { Instruction: Fix it, TaskToken.$: $$.Task.Token }\n    TimeoutSeconds: 600\n    HeartbeatSeconds: 600\n",
        ),
        "Skills.custom.States.T.HeartbeatSeconds",
    );
}

#[test]
fn time_values_are_bounded_by_the_plan() {
    let huge = "18446744073709551615";
    skill_error(
        &format!(
            "Inputs: {{}}\nStartAt: A\nTimeoutSeconds: {huge}\nStates:\n  A:\n    Type: Pass\n    End: true\n"
        ),
        "Skills.custom.TimeoutSeconds",
    );
    skill_error(
        &format!(
            "Inputs: {{}}\nStartAt: W\nTimeoutSeconds: 60\nStates:\n  W:\n    Type: Wait\n    Seconds: {huge}\n    End: true\n"
        ),
        "Skills.custom.States.W.Seconds",
    );
    skill_error(
        &task_skill(&format!(
            "    Resource: sanchoris:::agent:handoff.waitForTaskToken\n    Parameters: {{ Instruction: Fix it, TaskToken.$: $$.Task.Token }}\n    TimeoutSeconds: {huge}\n    HeartbeatSeconds: 900001\n"
        )),
        "Skills.custom.States.T.TimeoutSeconds",
    );
    let retry = |field: &str| {
        task_skill(&format!(
            "    Resource: sanchoris:::chat:post\n    Parameters: {{ Text: hi }}\n    TimeoutSeconds: 60\n    Retry:\n      - ErrorEquals: [States.ALL]\n        MaxAttempts: 2\n{field}"
        ))
    };
    skill_error(
        &retry(&format!(
            "        IntervalSeconds: {huge}\n        BackoffRate: 2\n"
        )),
        "Skills.custom.States.T.Retry[0].IntervalSeconds",
    );
    skill_error(
        &retry(&format!(
            "        IntervalSeconds: 1\n        BackoffRate: 2\n        MaxDelaySeconds: {huge}\n"
        )),
        "Skills.custom.States.T.Retry[0].MaxDelaySeconds",
    );
    skill_error(
        &retry("        IntervalSeconds: 1\n        BackoffRate: 1.0e300\n"),
        "Skills.custom.States.T.Retry[0].BackoffRate",
    );
    load_repository(
        &skill_files(&retry(
            "        IntervalSeconds: 900000\n        BackoffRate: 10\n        MaxDelaySeconds: 900000\n",
        )),
        &context(),
    )
    .unwrap();
}

fn choice_skill(rules: &str) -> String {
    format!(
        "Inputs: {{}}\nStartAt: C\nTimeoutSeconds: 60\nStates:\n  C:\n    Type: Choice\n    Choices:\n{rules}    Default: D\n  D:\n    Type: Succeed\n"
    )
}

#[test]
fn choice_rules_are_checked() {
    load_repository(
        &skill_files(&choice_skill(
            "      - And:\n          - Variable: $.a\n            IsPresent: true\n          - Not:\n              Variable: $.b\n              StringEquals: x\n        Next: D\n",
        )),
        &context(),
    )
    .unwrap();
    skill_error(
        &choice_skill("      - Variable: $.a\n        StringEquals: x\n"),
        "Skills.custom.States.C.Choices[0].Next",
    );
    skill_error(
        &choice_skill(
            "      - And:\n          - Variable: $.a\n            IsPresent: true\n            Next: D\n        Next: D\n",
        ),
        "Skills.custom.States.C.Choices[0].And[0].Next",
    );
    skill_error(
        &choice_skill("      - StringEquals: x\n        Next: D\n"),
        "Skills.custom.States.C.Choices[0].Variable",
    );
    skill_error(
        &choice_skill(
            "      - Variable: $.a\n        StringEquals: x\n        IsPresent: true\n        Next: D\n",
        ),
        "Skills.custom.States.C.Choices[0]",
    );
    skill_error(
        &choice_skill("      - Variable: $.a\n        StringEquals: x\n        Next: E\n"),
        "Skills.custom.States.C.Choices[0].Next",
    );
}
