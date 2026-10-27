use crate::error::at;
use crate::repository::{Skill, SkillOrigin};

pub struct BuiltinSkillSource {
    pub name: &'static str,
    pub skill_md: &'static str,
    pub skill_yaml: Option<&'static str>,
}

pub const BUILTIN_SKILL_SOURCES: &[BuiltinSkillSource] = &[BuiltinSkillSource {
    name: "open-pr-green-ci",
    skill_md: include_str!("../skills/open-pr-green-ci/SKILL.md"),
    skill_yaml: Some(include_str!("../skills/open-pr-green-ci/skill.yaml")),
}];

pub fn builtin_skills() -> Vec<Skill> {
    BUILTIN_SKILL_SOURCES
        .iter()
        .map(|source| {
            Skill::from_sources(
                source.skill_md,
                source.skill_yaml,
                SkillOrigin::BuiltIn,
                &at("BuiltinSkills", source.name),
            )
            .unwrap_or_else(|errors| {
                panic!("built-in skill {} is invalid: {errors:?}", source.name)
            })
        })
        .collect()
}

pub fn builtin_skill(name: &str) -> Option<Skill> {
    builtin_skills().into_iter().find(|s| s.name == name)
}
