use crate::model::{Notification, NotificationKind};

pub fn render(skill_run_id: i64, skill_name: &str, kind: &NotificationKind) -> String {
    let head = format!("[sanchoris] Skill {skill_name} (run {skill_run_id})");
    match kind {
        NotificationKind::Succeeded { output } => format!("{head} succeeded.\nOutput: {output}"),
        NotificationKind::Failed { error, cause } => match cause {
            Some(cause) => format!("{head} failed: {error}.\nCause: {cause}"),
            None => format!("{head} failed: {error}."),
        },
        NotificationKind::NeedsAgent { instruction } => format!(
            "{head} needs you: {instruction}\nWhen you are done, run `sanchoris skill continue {skill_run_id}`."
        ),
    }
}

pub fn notification(skill_run_id: i64, skill_name: &str, kind: NotificationKind) -> Notification {
    Notification {
        skill_run_id,
        skill_name: skill_name.to_string(),
        text: render(skill_run_id, skill_name, &kind),
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_each_kind() {
        assert_eq!(
            render(
                1,
                "open-pr-green-ci",
                &NotificationKind::NeedsAgent {
                    instruction: "CI failed on acme/shop-web#52 (checkout_spec::flaky).".into()
                }
            ),
            "[sanchoris] Skill open-pr-green-ci (run 1) needs you: CI failed on acme/shop-web#52 (checkout_spec::flaky).\nWhen you are done, run `sanchoris skill continue 1`."
        );
        assert_eq!(
            render(
                2,
                "s",
                &NotificationKind::Succeeded {
                    output: json!({"a": 1})
                }
            ),
            "[sanchoris] Skill s (run 2) succeeded.\nOutput: {\"a\":1}"
        );
        assert_eq!(
            render(
                3,
                "s",
                &NotificationKind::Failed {
                    error: "E".into(),
                    cause: Some("why".into())
                }
            ),
            "[sanchoris] Skill s (run 3) failed: E.\nCause: why"
        );
        assert_eq!(
            render(
                3,
                "s",
                &NotificationKind::Failed {
                    error: "E".into(),
                    cause: None
                }
            ),
            "[sanchoris] Skill s (run 3) failed: E."
        );
    }
}
