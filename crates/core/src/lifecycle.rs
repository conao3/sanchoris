use chrono::{DateTime, Duration, Utc};

use crate::messages;
use crate::model::{SkillRunStatus, TaskStatus, UsageLimitResume, WaitReason};

pub const ERROR_NUDGES_EXHAUSTED: &str = "Task.NudgesExhausted";
pub const ERROR_USAGE_LIMIT: &str = "Worker.UsageLimit";
pub const ERROR_TURN_LIMIT: &str = "Task.TurnLimitExceeded";
pub const ERROR_COST_LIMIT: &str = "Task.CostLimitExceeded";
pub const ERROR_TIMEOUT: &str = "Task.Timeout";
pub const ERROR_HEARTBEAT_TIMEOUT: &str = "Task.HeartbeatTimeout";
pub const ERROR_WORKSPACE_STOPPED: &str = "Workspace.Stopped";

#[derive(Debug, Clone, PartialEq)]
pub enum TaskDecision {
    Succeed,
    Wait {
        reason: WaitReason,
        resume_at: Option<DateTime<Utc>>,
    },
    Nudge {
        nudges: u32,
        message: String,
    },
    NeedsYou {
        error: String,
        cause: String,
    },
    Resume {
        message: String,
        reset_nudges: bool,
    },
}

impl TaskDecision {
    pub fn status(&self) -> TaskStatus {
        match self {
            Self::Succeed => TaskStatus::Succeeded,
            Self::Wait { .. } => TaskStatus::Waiting,
            Self::Nudge { .. } | Self::Resume { .. } => TaskStatus::Running,
            Self::NeedsYou { .. } => TaskStatus::NeedsYou,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequiredSkill<'a> {
    pub name: &'a str,
    pub latest: Option<SkillRunStatus>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TurnEnd<'a> {
    pub required: &'a [RequiredSkill<'a>],
    pub nudges: u32,
    pub max_nudges: u32,
}

pub fn decide_turn_end(turn: &TurnEnd<'_>) -> TaskDecision {
    if turn.required.is_empty() {
        return TaskDecision::Wait {
            reason: WaitReason::Chat,
            resume_at: None,
        };
    }
    if turn
        .required
        .iter()
        .all(|skill| skill.latest == Some(SkillRunStatus::Succeeded))
    {
        return TaskDecision::Succeed;
    }
    if turn
        .required
        .iter()
        .any(|skill| skill.latest.is_some_and(SkillRunStatus::is_active))
    {
        return TaskDecision::Wait {
            reason: WaitReason::Skill,
            resume_at: None,
        };
    }
    let pending = turn
        .required
        .iter()
        .filter(|skill| skill.latest != Some(SkillRunStatus::Succeeded))
        .map(|skill| skill.name)
        .collect::<Vec<_>>();
    if turn.nudges < turn.max_nudges {
        let nudges = turn.nudges + 1;
        return TaskDecision::Nudge {
            nudges,
            message: messages::nudge_message(&pending, nudges, turn.max_nudges),
        };
    }
    TaskDecision::NeedsYou {
        error: ERROR_NUDGES_EXHAUSTED.to_string(),
        cause: format!(
            "The required skills have not succeeded after {} reminders: {}",
            turn.max_nudges,
            pending.join(", ")
        ),
    }
}

pub fn decide_skill_run_finished(
    required_skills: &[&str],
    skill_name: &str,
    status: SkillRunStatus,
    error: &str,
    cause: &str,
) -> Option<TaskDecision> {
    (status == SkillRunStatus::Failed && required_skills.contains(&skill_name)).then(|| {
        TaskDecision::NeedsYou {
            error: error.to_string(),
            cause: format!("The required skill {skill_name} failed: {cause}"),
        }
    })
}

pub fn decide_usage_limit(resume: UsageLimitResume, reset_at: DateTime<Utc>) -> TaskDecision {
    match resume {
        UsageLimitResume::Auto => TaskDecision::Wait {
            reason: WaitReason::UsageLimit,
            resume_at: Some(reset_at),
        },
        UsageLimitResume::Manual => TaskDecision::NeedsYou {
            error: ERROR_USAGE_LIMIT.to_string(),
            cause: format!(
                "The worker reached its usage limit, which resets at {}",
                reset_at.to_rfc3339()
            ),
        },
    }
}

pub fn decide_usage_limit_resume(
    resume_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<TaskDecision> {
    (now >= resume_at).then(|| TaskDecision::Resume {
        message: messages::usage_limit_resume_message(),
        reset_nudges: false,
    })
}

pub fn decide_human_reply(member: &str, text: &str) -> TaskDecision {
    TaskDecision::Resume {
        message: messages::reply_message(member, text),
        reset_nudges: true,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskLimits {
    pub turns: u32,
    pub cost_usd: f64,
    pub timeout_seconds: i64,
    pub heartbeat_seconds: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskProgress {
    pub turns: u32,
    pub cost_usd: f64,
    pub started_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
}

pub fn check_limits(
    limits: &TaskLimits,
    progress: &TaskProgress,
    now: DateTime<Utc>,
) -> Option<TaskDecision> {
    let exceeded = |error: &str, cause: String| {
        Some(TaskDecision::NeedsYou {
            error: error.to_string(),
            cause,
        })
    };
    if progress.turns >= limits.turns {
        return exceeded(
            ERROR_TURN_LIMIT,
            format!(
                "The session used {} of {} turns",
                progress.turns, limits.turns
            ),
        );
    }
    if progress.cost_usd >= limits.cost_usd {
        return exceeded(
            ERROR_COST_LIMIT,
            format!(
                "The session cost {:.2} USD of the {:.2} USD limit",
                progress.cost_usd, limits.cost_usd
            ),
        );
    }
    if now - progress.started_at >= Duration::seconds(limits.timeout_seconds) {
        return exceeded(
            ERROR_TIMEOUT,
            format!(
                "The task ran longer than {} seconds",
                limits.timeout_seconds
            ),
        );
    }
    if now - progress.last_activity_at >= Duration::seconds(limits.heartbeat_seconds) {
        return exceeded(
            ERROR_HEARTBEAT_TIMEOUT,
            format!(
                "The session had no activity for {} seconds",
                limits.heartbeat_seconds
            ),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn required(latest: Option<SkillRunStatus>) -> Vec<RequiredSkill<'static>> {
        vec![RequiredSkill {
            name: "open-pr-green-ci",
            latest,
        }]
    }

    fn turn_end(required: &[RequiredSkill<'_>], nudges: u32) -> TaskDecision {
        decide_turn_end(&TurnEnd {
            required,
            nudges,
            max_nudges: 2,
        })
    }

    #[test]
    fn running_required_skill_waits_without_nudging() {
        for status in [
            SkillRunStatus::Running,
            SkillRunStatus::WaitingAgent,
            SkillRunStatus::Waiting,
        ] {
            assert_eq!(
                turn_end(&required(Some(status)), 0),
                TaskDecision::Wait {
                    reason: WaitReason::Skill,
                    resume_at: None
                }
            );
        }
    }

    #[test]
    fn succeeded_required_skills_complete_the_task() {
        let decision = turn_end(&required(Some(SkillRunStatus::Succeeded)), 2);
        assert_eq!(decision, TaskDecision::Succeed);
        assert_eq!(decision.status(), TaskStatus::Succeeded);
    }

    #[test]
    fn every_required_skill_must_succeed() {
        let skills = [
            RequiredSkill {
                name: "a",
                latest: Some(SkillRunStatus::Succeeded),
            },
            RequiredSkill {
                name: "b",
                latest: None,
            },
        ];
        assert_eq!(
            turn_end(&skills, 0),
            TaskDecision::Nudge {
                nudges: 1,
                message: "[sanchoris] The required skill b has not succeeded yet. Run it before you finish. (reminder 1 of 2)".into()
            }
        );
    }

    #[test]
    fn missing_required_skill_is_nudged_until_the_limit() {
        assert_eq!(
            turn_end(&required(None), 0),
            TaskDecision::Nudge {
                nudges: 1,
                message: "[sanchoris] The required skill open-pr-green-ci has not succeeded yet. Run it before you finish. (reminder 1 of 2)".into()
            }
        );
        assert!(matches!(
            turn_end(&required(Some(SkillRunStatus::Failed)), 1),
            TaskDecision::Nudge { nudges: 2, .. }
        ));
        let decision = turn_end(&required(None), 2);
        assert_eq!(
            decision,
            TaskDecision::NeedsYou {
                error: "Task.NudgesExhausted".into(),
                cause: "The required skills have not succeeded after 2 reminders: open-pr-green-ci"
                    .into()
            }
        );
        assert_eq!(decision.status(), TaskStatus::NeedsYou);
    }

    #[test]
    fn zero_max_nudges_goes_straight_to_needs_you() {
        let decision = decide_turn_end(&TurnEnd {
            required: &required(None),
            nudges: 0,
            max_nudges: 0,
        });
        assert!(matches!(decision, TaskDecision::NeedsYou { .. }));
    }

    #[test]
    fn tasks_without_required_skills_wait_for_chat() {
        assert_eq!(
            turn_end(&[], 0),
            TaskDecision::Wait {
                reason: WaitReason::Chat,
                resume_at: None
            }
        );
    }

    #[test]
    fn failed_required_skill_needs_you() {
        assert_eq!(
            decide_skill_run_finished(
                &["open-pr-green-ci"],
                "open-pr-green-ci",
                SkillRunStatus::Failed,
                "Skill.FixesExhausted",
                "The checks still fail after 3 fixes"
            ),
            Some(TaskDecision::NeedsYou {
                error: "Skill.FixesExhausted".into(),
                cause: "The required skill open-pr-green-ci failed: The checks still fail after 3 fixes".into()
            })
        );
        assert_eq!(
            decide_skill_run_finished(
                &["open-pr-green-ci"],
                "lint",
                SkillRunStatus::Failed,
                "E",
                "C"
            ),
            None
        );
        assert_eq!(
            decide_skill_run_finished(
                &["open-pr-green-ci"],
                "open-pr-green-ci",
                SkillRunStatus::Succeeded,
                "",
                ""
            ),
            None
        );
    }

    #[test]
    fn usage_limit_waits_or_needs_you() {
        let reset_at = Utc.with_ymd_and_hms(2026, 10, 27, 12, 0, 0).unwrap();
        assert_eq!(
            decide_usage_limit(UsageLimitResume::Auto, reset_at),
            TaskDecision::Wait {
                reason: WaitReason::UsageLimit,
                resume_at: Some(reset_at)
            }
        );
        assert_eq!(
            decide_usage_limit(UsageLimitResume::Manual, reset_at),
            TaskDecision::NeedsYou {
                error: "Worker.UsageLimit".into(),
                cause:
                    "The worker reached its usage limit, which resets at 2026-10-27T12:00:00+00:00"
                        .into()
            }
        );
    }

    #[test]
    fn usage_limit_resumes_after_reset() {
        let reset_at = Utc.with_ymd_and_hms(2026, 10, 27, 12, 0, 0).unwrap();
        assert_eq!(
            decide_usage_limit_resume(reset_at, reset_at - Duration::seconds(1)),
            None
        );
        assert_eq!(
            decide_usage_limit_resume(reset_at, reset_at),
            Some(TaskDecision::Resume {
                message: "[sanchoris] Continue where you left off.".into(),
                reset_nudges: false
            })
        );
    }

    #[test]
    fn human_reply_resumes_the_session_and_resets_nudges() {
        let decision = decide_human_reply("Mika", "Skip the flaky test.");
        assert_eq!(
            decision,
            TaskDecision::Resume {
                message: "[sanchoris] Message from Mika:\nSkip the flaky test.".into(),
                reset_nudges: true
            }
        );
        assert_eq!(decision.status(), TaskStatus::Running);
    }

    #[test]
    fn limits_are_enforced() {
        let start = Utc.with_ymd_and_hms(2026, 10, 27, 0, 0, 0).unwrap();
        let limits = TaskLimits {
            turns: 60,
            cost_usd: 20.0,
            timeout_seconds: 14400,
            heartbeat_seconds: 900,
        };
        let progress = TaskProgress {
            turns: 10,
            cost_usd: 1.5,
            started_at: start,
            last_activity_at: start + Duration::seconds(600),
        };
        let now = start + Duration::seconds(1000);
        assert_eq!(check_limits(&limits, &progress, now), None);

        let error_of = |progress: &TaskProgress, now| match check_limits(&limits, progress, now) {
            Some(TaskDecision::NeedsYou { error, .. }) => error,
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(
            error_of(
                &TaskProgress {
                    turns: 60,
                    ..progress.clone()
                },
                now
            ),
            "Task.TurnLimitExceeded"
        );
        assert_eq!(
            error_of(
                &TaskProgress {
                    cost_usd: 20.0,
                    ..progress.clone()
                },
                now
            ),
            "Task.CostLimitExceeded"
        );
        assert_eq!(
            error_of(&progress, start + Duration::seconds(14400)),
            "Task.Timeout"
        );
        assert_eq!(
            error_of(&progress, start + Duration::seconds(1500)),
            "Task.HeartbeatTimeout"
        );
    }
}
