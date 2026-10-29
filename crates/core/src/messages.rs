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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
            trigger_name: "chat-all",
            task_title: "Thread",
            instruction: "hello",
            skills: &[],
        });
        assert_eq!(
            text,
            "[sanchoris] Trigger chat-all started this task: Thread.\nInstruction: hello"
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
}
