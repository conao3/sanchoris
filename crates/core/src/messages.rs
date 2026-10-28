use serde_json::Value;

pub const PREFIX: &str = "[sanchoris]";

#[derive(Debug, Clone, PartialEq)]
pub struct OfferedSkill {
    pub name: String,
    pub required: bool,
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
        "{PREFIX} Your workspace was interrupted by the platform and restored in a new workspace from the saved session and the task branch. Changes that were not pushed to the task branch are gone. Continue where you left off."
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
            },
            OfferedSkill {
                name: "lint".into(),
                required: false,
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
