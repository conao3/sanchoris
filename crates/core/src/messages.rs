use sanchoris_config::validate::covers;
use sanchoris_config::{FILE_ISSUE_SKILL, OPEN_PR_SKILL};
use serde_json::{Map, Value};

pub const PREFIX: &str = "[sanchoris]";
pub const REDACTED: &str = "<redacted>";

#[derive(Debug, Clone, PartialEq)]
pub struct OfferedSkill {
    pub name: String,
    pub required: bool,
    pub inputs: Vec<OfferedInput>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OfferedInput {
    pub name: String,
    pub kind: String,
    pub value: Option<Value>,
}

const SECRET_NAMES: [&str; 6] = ["token", "secret", "password", "credential", "apikey", "key"];
const SECRET_PREFIXES: [&str; 9] = [
    "sk-",
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "github_pat_",
    "xox",
    "AKIA",
    "-----BEGIN",
];

const TOKEN_PREFIXES: [&str; 6] = ["github_pat_", "ghp_", "gho_", "ghs_", "ghu_", "sk-"];
const TOKEN_MIN_BODY: usize = 20;
const AWS_KEY_PREFIXES: [&str; 2] = ["AKIA", "ASIA"];
const AWS_KEY_BODY: usize = 16;

fn token_end(rest: &[u8]) -> Option<usize> {
    let body = |start: usize, allowed: fn(&u8) -> bool| {
        rest[start..]
            .iter()
            .take_while(|byte| allowed(byte))
            .count()
    };
    for prefix in TOKEN_PREFIXES {
        if rest.starts_with(prefix.as_bytes()) {
            let length = body(prefix.len(), |byte| {
                byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-'
            });
            if length >= TOKEN_MIN_BODY {
                return Some(prefix.len() + length);
            }
        }
    }
    for prefix in AWS_KEY_PREFIXES {
        if rest.starts_with(prefix.as_bytes())
            && body(prefix.len(), |byte| {
                byte.is_ascii_uppercase() || byte.is_ascii_digit()
            }) == AWS_KEY_BODY
            && rest
                .get(prefix.len() + AWS_KEY_BODY)
                .is_none_or(|byte| !byte.is_ascii_alphanumeric())
        {
            return Some(prefix.len() + AWS_KEY_BODY);
        }
    }
    None
}

pub fn redact_tokens(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut redacted = String::with_capacity(text.len());
    let mut copied = 0;
    let mut index = 0;
    while index < bytes.len() {
        let at_boundary = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        match token_end(&bytes[index..]).filter(|_| at_boundary) {
            Some(length) => {
                redacted.push_str(&text[copied..index]);
                redacted.push_str(REDACTED);
                index += length;
                copied = index;
            }
            None => index += 1,
        }
    }
    redacted.push_str(&text[copied..]);
    redacted
}

pub fn redact_tokens_in(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact_tokens(text)),
        Value::Array(items) => Value::Array(items.iter().map(redact_tokens_in).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), redact_tokens_in(item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

pub fn is_secret_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase().replace(['_', '-'], "");
    SECRET_NAMES.iter().any(|word| lower.contains(word))
}

pub fn redact(name: &str, value: &Value) -> Value {
    if is_secret_name(name) {
        return Value::String(REDACTED.into());
    }
    match value {
        Value::String(text) if SECRET_PREFIXES.iter().any(|p| text.starts_with(p)) => {
            Value::String(REDACTED.into())
        }
        Value::Array(items) => Value::Array(items.iter().map(|item| redact("", item)).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), redact(key, item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn inputs_line(skill: &OfferedSkill) -> Option<String> {
    if skill.inputs.is_empty() {
        return None;
    }
    let mut resolved = Map::new();
    let listed = skill
        .inputs
        .iter()
        .map(|input| match &input.value {
            Some(value) => {
                let shown = redact(&input.name, value);
                if shown == *value {
                    resolved.insert(input.name.clone(), value.clone());
                }
                format!("{} ({}) = {shown}", input.name, input.kind)
            }
            None => format!("{} ({}) not resolved", input.name, input.kind),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut line = format!("Inputs of {}: {listed}.", skill.name);
    if !resolved.is_empty() {
        line.push_str(&format!(
            " When your changes are committed, run it with `sanchoris skill run {} --input '{}'`.",
            skill.name,
            Value::Object(resolved).to_string().replace('\'', "'\\''")
        ));
    }
    Some(line)
}

#[derive(Debug, Clone, PartialEq)]
pub struct StartMessage<'a> {
    pub trigger_name: &'a str,
    pub task_title: &'a str,
    pub instruction: &'a str,
    pub skills: &'a [OfferedSkill],
}

pub fn start_message(message: &StartMessage<'_>) -> String {
    let mut text = format!(
        "{PREFIX} Trigger {} started this task: {}.\nInstruction: {}",
        message.trigger_name, message.task_title, message.instruction
    );
    if !message.skills.is_empty() {
        let listed = message
            .skills
            .iter()
            .map(|skill| {
                if skill.required {
                    format!("{} (required)", skill.name)
                } else {
                    skill.name.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let how = match message.skills {
            [only] => format!("Run it with `sanchoris skill run {}`.", only.name),
            _ => "Run one with `sanchoris skill run <name>`.".to_string(),
        };
        text.push_str(&format!("\nSkills: {listed}. {how}"));
        for line in message.skills.iter().filter_map(inputs_line) {
            text.push('\n');
            text.push_str(&line);
        }
    }
    text
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatStartMessage<'a> {
    pub session_name: &'a str,
    pub member: &'a str,
    pub skills: &'a [OfferedSkill],
    pub text: &'a str,
}

pub fn chat_start_message(message: &ChatStartMessage<'_>) -> String {
    let mut text = format!(
        "{PREFIX} {} started this chat in session {}. The last reply of each of your turns is posted to the chat, and the member's next message comes to this session.",
        message.member, message.session_name
    );
    if !message.skills.is_empty() {
        let listed = message
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&format!(
            "\nSkills: {listed}. Run one with `sanchoris skill run <name> --input '<JSON of its inputs>'`."
        ));
        for line in message.skills.iter().filter_map(inputs_line) {
            text.push('\n');
            text.push_str(&line);
        }
    }
    if message
        .skills
        .iter()
        .any(|skill| skill.name == FILE_ISSUE_SKILL)
    {
        text.push_str(&format!(
            "\nWork that belongs in its own task, for example in another repository, goes into an issue: `sanchoris skill run {FILE_ISSUE_SKILL} --input '{{\"Repo\": \"owner/name\", \"Title\": \"...\", \"Body\": \"...\", \"Labels\": [\"...\"]}}'`. Write the body so that an agent who has not seen this chat can do the work, and add the labels that the repository's trigger reacts to; that trigger starts the task and the chat shows it."
        ));
    }
    text.push_str("\n\n");
    text.push_str(&reply_message(message.member, message.text));
    text
}

pub fn nudge_message(pending_skills: &[&str], reminder: u32, max_nudges: u32) -> String {
    let subject = match pending_skills {
        [only] => format!("The required skill {only} has not succeeded yet. Run it"),
        many => format!(
            "The required skills {} have not succeeded yet. Run them",
            many.join(", ")
        ),
    };
    format!("{PREFIX} {subject} before you finish. (reminder {reminder} of {max_nudges})")
}

pub fn reply_message(member: &str, text: &str) -> String {
    format!("{PREFIX} Message from {member}:\n{text}")
}

pub fn usage_limit_resume_message() -> String {
    format!("{PREFIX} Continue where you left off.")
}

pub fn skill_succeeded_message(skill_name: &str, run_number: i32, output: &Value) -> String {
    format!("{PREFIX} Skill {skill_name} (run {run_number}) succeeded.\nOutput: {output}")
}

pub fn skill_failed_message(skill_name: &str, run_number: i32, error: &str, cause: &str) -> String {
    format!("{PREFIX} Skill {skill_name} (run {run_number}) failed: {error}.\nCause: {cause}")
}

pub fn skill_needs_agent_message(skill_name: &str, run_number: i32, instruction: &str) -> String {
    format!(
        "{PREFIX} Skill {skill_name} (run {run_number}) needs you: {instruction}\nWhen you are done, run `sanchoris skill continue {run_number}`."
    )
}

pub fn restored_message() -> String {
    format!(
        "{PREFIX} Your workspace was interrupted by the platform and restored in a new workspace from the saved session and the task branch. The platform stopped the tool call that was running, nobody refused it: run it again if you still need it. Changes that were not pushed to the task branch are gone. Continue where you left off."
    )
}

/// The settings a task runs with, from the config revision active when it started.
#[derive(Debug, Clone, PartialEq)]
pub struct Workflow<'a> {
    pub config_repository: &'a str,
    pub config_root: &'a str,
    pub config_branch: &'a str,
    /// The branch the task pushes in every repository it changes.
    pub task_branch: &'a str,
    /// The trigger that started the task and its definition, or None for a chat.
    pub trigger: Option<(&'a str, &'a Value)>,
    pub session_name: &'a str,
    pub session: &'a Value,
    pub start: &'a Value,
    /// The GitHub accounts the tenant's GitHub App is installed on.
    pub accounts: &'a [String],
}

/// Whether the task can open a pull request on the config repository: `GitHub.Repos` covers it, `GitHub.Permissions` writes contents and pull requests, and `Start.Skills` has the skill that opens pull requests.
fn opens_config_pull_requests(workflow: &Workflow<'_>) -> bool {
    let github = &workflow.session["GitHub"];
    let repos = github["Repos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    covers(&repos, workflow.config_repository)
        && github["Permissions"]["contents"] == "write"
        && github["Permissions"]["pull_requests"] == "write"
        && workflow.start["Skills"]
            .as_array()
            .is_some_and(|skills| skills.iter().any(|skill| skill == OPEN_PR_SKILL))
}

fn yaml(value: &Value) -> String {
    serde_yaml_ng::to_string(value).expect("a JSON value serializes as YAML")
}

/// The worker's second system prompt: the task's workflow settings and how to propose a change to them, then the Project's `SystemPrompt`. The first system prompt is the worker's own and stays the same for every task, so it is cached across tasks.
pub fn workflow_prompt(workflow: &Workflow<'_>, project_prompt: Option<&str>) -> String {
    let origin = match workflow.trigger {
        Some((name, _)) => format!("Trigger `{name}` started this task."),
        None => format!(
            "A member started this task from a chat in session `{}`.",
            workflow.session_name
        ),
    };
    let accounts = if workflow.accounts.is_empty() {
        "none".to_string()
    } else {
        workflow.accounts.join(", ")
    };
    let mut text = format!(
        "# sanchoris workflow settings

You work in a sanchoris task. These settings decide what this task can reach and do. They come from `sanchoris.yaml` under `{root}` on branch `{branch}` of the config repository `{config}`, as they were when the task started; a change to them applies to tasks that start after it, not to this one.

{origin}

Session `{session_name}`:
```yaml
{session}```

Start (instruction, skills, required skills, limits):
```yaml
{start}```
",
        root = workflow.config_root,
        branch = workflow.config_branch,
        config = workflow.config_repository,
        session_name = workflow.session_name,
        session = yaml(workflow.session),
        start = yaml(workflow.start),
    );
    if let Some((name, definition)) = workflow.trigger {
        text.push_str(&format!(
            "
Trigger `{name}`:
```yaml
{}```
",
            yaml(definition)
        ));
    }
    text.push_str(&format!(
        "
What they mean:
- GitHub: `git` and the GitHub API reach only the repositories in the session's `GitHub.Repos` (`owner/*` is every repository the GitHub App is installed on for that account), with `GitHub.Permissions`. The GitHub App is installed on these accounts of the tenant: {accounts}. `git` gets a token for each repository's account by itself; for an API call, take the token of an account with `printf 'protocol=https\\nhost=github.com\\npath=<owner>/<repo>\\n' | git credential fill`. The token is a GitHub App installation token, so endpoints about a user, such as `/user`, refuse it.
- Network: only the hosts in `Network.Allow` are reachable.
- Limits: the task stops for a person when it reaches the turns, the cost, or the time in `Limits`.

Changing them:
When a request needs something these settings do not allow, such as another repository or account, a permission, a network host, a skill, or a longer limit, do not work around it. Tell the member which setting stops it and propose the change to `sanchoris.yaml` as a diff. {how} A GitHub account the App is not installed on needs the App first: an owner or admin of the tenant adds it in Settings, Integrations.
",
        how = if opens_config_pull_requests(workflow) {
            format!(
                "You can open the change as a pull request: offer it with the diff, and when the member agrees, commit the change in the directory `{dir}` under the workspace (the clone of `{config}`; clone it there when it is not there), then run `{OPEN_PR_SKILL}` with `Repos` and `Repo` set to `{config}` and `Issue` set to the number of the issue the change is for (file one with `{FILE_ISSUE_SKILL}`, without a label that starts a task, when the task has none). The skill pushes that clone's `HEAD` to the task branch `{branch}` and opens the pull request, so the pull request also carries any other commits of this task in that clone. The member reviews and merges it on GitHub.",
                config = workflow.config_repository,
                dir = workflow
                    .config_repository
                    .rsplit('/')
                    .next()
                    .expect("owner/name"),
                branch = workflow.task_branch,
            )
        } else {
            format!(
                "This task cannot open a pull request on `{}`, so ask the member to make the change there.",
                workflow.config_repository,
            )
        },
    ));
    if let Some(prompt) = project_prompt.filter(|prompt| !prompt.trim().is_empty()) {
        text.push_str("\n# Project\n\n");
        text.push_str(prompt);
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn known_token_formats_are_redacted_inside_text() {
        let github = format!("ghs_{}", "a1B2".repeat(9));
        let pat = format!("github_pat_{}", "x_Y9".repeat(20));
        assert_eq!(
            redact_tokens(&format!(
                "{github} then https://x-access-token:{pat}@github.com, gho_{0} ghu_{0} ghp_{0}",
                "Z".repeat(36)
            )),
            "<redacted> then https://x-access-token:<redacted>@github.com, <redacted> <redacted> <redacted>"
        );
        assert_eq!(
            redact_tokens(&format!(
                "key=sk-ant-oat01-{} openai sk-proj-{}",
                "aB_-3".repeat(5),
                "c".repeat(20)
            )),
            "key=<redacted> openai <redacted>"
        );
        let aws = "Y34FZKBOKMUTVV7A";
        assert_eq!(
            redact_tokens(&format!("AKIA{aws} ASIA{aws}, AKIA{aws}X")),
            format!("<redacted> <redacted>, AKIA{aws}X")
        );
        let plain = "task-0123456789abcdefghijklmn, ghs_short, sk-learn, 東京 ghp_";
        assert_eq!(redact_tokens(plain), plain);
        assert_eq!(
            redact_tokens_in(&json!({ "a": [format!("x {github}"), 3], "b": null })),
            json!({ "a": ["x <redacted>", 3], "b": null })
        );
    }

    #[test]
    fn start_message_matches_the_documented_example() {
        let skills = [OfferedSkill {
            name: "open-pr-green-ci".into(),
            required: true,
            inputs: Vec::new(),
        }];
        let text = start_message(&StartMessage {
            trigger_name: "labeled-issue",
            task_title: "Fix flaky checkout test (acme/shop-web#35)",
            instruction: "Resolve issue 35 in acme/shop-web.",
            skills: &skills,
        });
        assert_eq!(
            text,
            "[sanchoris] Trigger labeled-issue started this task: Fix flaky checkout test (acme/shop-web#35).\n\
             Instruction: Resolve issue 35 in acme/shop-web.\n\
             Skills: open-pr-green-ci (required). Run it with `sanchoris skill run open-pr-green-ci`."
        );
    }

    #[test]
    fn start_message_lists_several_skills() {
        let skills = [
            OfferedSkill {
                name: "open-pr-green-ci".into(),
                required: true,
                inputs: Vec::new(),
            },
            OfferedSkill {
                name: "lint".into(),
                required: false,
                inputs: Vec::new(),
            },
        ];
        let text = start_message(&StartMessage {
            trigger_name: "t",
            task_title: "T",
            instruction: "Do it.",
            skills: &skills,
        });
        assert!(text.ends_with(
            "Skills: open-pr-green-ci (required), lint. Run one with `sanchoris skill run <name>`."
        ));
    }

    #[test]
    fn start_message_shows_the_resolved_inputs_of_each_skill() {
        let skills = [OfferedSkill {
            name: "open-pr-green-ci".into(),
            required: true,
            inputs: vec![
                OfferedInput {
                    name: "Issue".into(),
                    kind: "number".into(),
                    value: Some(json!(35)),
                },
                OfferedInput {
                    name: "MaxFixes".into(),
                    kind: "number".into(),
                    value: None,
                },
                OfferedInput {
                    name: "Repo".into(),
                    kind: "string".into(),
                    value: Some(json!("acme/shop-web")),
                },
                OfferedInput {
                    name: "Repos".into(),
                    kind: "array".into(),
                    value: Some(json!(["acme/shop-web"])),
                },
            ],
        }];
        let text = start_message(&StartMessage {
            trigger_name: "labeled-issue",
            task_title: "Fix flaky checkout test (acme/shop-web#35)",
            instruction: "Resolve issue 35 in acme/shop-web.",
            skills: &skills,
        });
        assert_eq!(
            text.lines().last().unwrap(),
            "Inputs of open-pr-green-ci: Issue (number) = 35, MaxFixes (number) not resolved, Repo (string) = \"acme/shop-web\", Repos (array) = [\"acme/shop-web\"]. When your changes are committed, run it with `sanchoris skill run open-pr-green-ci --input '{\"Issue\":35,\"Repo\":\"acme/shop-web\",\"Repos\":[\"acme/shop-web\"]}'`."
        );
    }

    #[test]
    fn secret_like_inputs_are_redacted_and_left_out_of_the_command() {
        let skills = [OfferedSkill {
            name: "deploy".into(),
            required: false,
            inputs: vec![
                OfferedInput {
                    name: "ApiToken".into(),
                    kind: "string".into(),
                    value: Some(json!("abc")),
                },
                OfferedInput {
                    name: "Env".into(),
                    kind: "object".into(),
                    value: Some(json!({ "GITHUB": "ghp_0123", "Region": "ap-northeast-1" })),
                },
            ],
        }];
        let text = start_message(&StartMessage {
            trigger_name: "t",
            task_title: "T",
            instruction: "Do it.",
            skills: &skills,
        });
        assert_eq!(
            text.lines().last().unwrap(),
            "Inputs of deploy: ApiToken (string) = \"<redacted>\", Env (object) = {\"GITHUB\":\"<redacted>\",\"Region\":\"ap-northeast-1\"}."
        );
        assert!(!text.contains("abc") && !text.contains("ghp_0123"));
    }

    #[test]
    fn start_message_without_skills_has_no_skills_line() {
        let text = start_message(&StartMessage {
            trigger_name: "nightly",
            task_title: "Thread",
            instruction: "hello",
            skills: &[],
        });
        assert_eq!(
            text,
            "[sanchoris] Trigger nightly started this task: Thread.\nInstruction: hello"
        );
    }

    #[test]
    fn chat_start_message_names_the_session_and_how_to_file_issues() {
        let skills = [OfferedSkill {
            name: "file-issue".into(),
            required: false,
            inputs: vec![OfferedInput {
                name: "Repo".into(),
                kind: "string".into(),
                value: None,
            }],
        }];
        let text = chat_start_message(&ChatStartMessage {
            session_name: "all",
            member: "Mika",
            skills: &skills,
            text: "Split the checkout work.",
        });
        let lines = text.lines().collect::<Vec<_>>();
        assert_eq!(
            lines[0],
            "[sanchoris] Mika started this chat in session all. The last reply of each of your turns is posted to the chat, and the member's next message comes to this session."
        );
        assert_eq!(
            lines[1],
            "Skills: file-issue. Run one with `sanchoris skill run <name> --input '<JSON of its inputs>'`."
        );
        assert_eq!(
            lines[2],
            "Inputs of file-issue: Repo (string) not resolved."
        );
        assert!(lines[3].contains("sanchoris skill run file-issue --input"));
        assert_eq!(
            lines[5..],
            ["[sanchoris] Message from Mika:", "Split the checkout work."]
        );
    }

    #[test]
    fn nudge_message_matches_the_documented_example() {
        assert_eq!(
            nudge_message(&["open-pr-green-ci"], 1, 2),
            "[sanchoris] The required skill open-pr-green-ci has not succeeded yet. Run it before you finish. (reminder 1 of 2)"
        );
        assert_eq!(
            nudge_message(&["a", "b"], 2, 3),
            "[sanchoris] The required skills a, b have not succeeded yet. Run them before you finish. (reminder 2 of 3)"
        );
    }

    #[test]
    fn reply_and_usage_messages() {
        assert_eq!(
            reply_message("Mika", "Use the staging database."),
            "[sanchoris] Message from Mika:\nUse the staging database."
        );
        assert_eq!(
            usage_limit_resume_message(),
            "[sanchoris] Continue where you left off."
        );
    }

    #[test]
    fn skill_notifications() {
        assert_eq!(
            skill_needs_agent_message(
                "open-pr-green-ci",
                1,
                "CI failed on acme/shop-web#52 (checkout_spec::flaky)."
            ),
            "[sanchoris] Skill open-pr-green-ci (run 1) needs you: CI failed on acme/shop-web#52 (checkout_spec::flaky).\n\
             When you are done, run `sanchoris skill continue 1`."
        );
        assert_eq!(
            skill_succeeded_message("open-pr-green-ci", 1, &json!({ "ok": true })),
            "[sanchoris] Skill open-pr-green-ci (run 1) succeeded.\nOutput: {\"ok\":true}"
        );
        assert_eq!(
            skill_failed_message(
                "open-pr-green-ci",
                2,
                "Skill.FixesExhausted",
                "The checks still fail after 3 fixes"
            ),
            "[sanchoris] Skill open-pr-green-ci (run 2) failed: Skill.FixesExhausted.\nCause: The checks still fail after 3 fixes"
        );
    }

    #[test]
    fn the_workflow_prompt_says_whether_the_task_can_open_a_config_pull_request() {
        let session = json!({
            "GitHub": {
                "Repos": ["acme/*"],
                "Permissions": { "contents": "write", "pull_requests": "write" },
            },
        });
        let workflow = |start: &Value| {
            workflow_prompt(
                &Workflow {
                    config_repository: "acme/sanchoris-config",
                    config_root: "/",
                    config_branch: "main",
                    task_branch: "sanchoris/1",
                    trigger: None,
                    session_name: "all",
                    session: &session,
                    start,
                    accounts: &["acme".to_string()],
                },
                Some("Write tests."),
            )
        };
        let opens = workflow(&json!({ "Skills": ["open-pr-green-ci"] }));
        assert!(
            opens.contains("You can open the change as a pull request"),
            "{opens}"
        );
        assert!(opens.contains("  Repos:\n  - acme/*\n"), "{opens}");
        assert!(opens.contains("directory `sanchoris-config` under the workspace"));
        assert!(opens.contains("the task branch `sanchoris/1`"));
        assert!(opens.ends_with("# Project\n\nWrite tests.\n"), "{opens}");
        let asks = workflow(&json!({ "Skills": ["file-issue"] }));
        assert!(asks.contains("This task cannot open a pull request on `acme/sanchoris-config`"));
    }
}
