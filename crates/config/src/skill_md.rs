use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillDocument {
    pub name: String,
    pub description: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SkillDocumentError {
    #[error("SKILL.md must start with a YAML frontmatter block delimited by --- lines")]
    MissingFrontmatter,
    #[error("SKILL.md frontmatter is not valid YAML: {0}")]
    Yaml(String),
    #[error("SKILL.md frontmatter must be a mapping")]
    NotMapping,
    #[error("SKILL.md frontmatter is missing {0}")]
    MissingField(&'static str),
    #[error("SKILL.md frontmatter field {0} must be a non-empty string")]
    InvalidField(&'static str),
    #[error(
        "skill name {0:?} must be 1 to 64 characters of lowercase letters, digits, and hyphens"
    )]
    InvalidName(String),
}

pub fn is_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn parse_skill_md(text: &str) -> Result<SkillDocument, SkillDocumentError> {
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().ok_or(SkillDocumentError::MissingFrontmatter)?;
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return Err(SkillDocumentError::MissingFrontmatter);
    }
    let mut frontmatter = String::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            closed = true;
            break;
        }
        frontmatter.push_str(line);
    }
    if !closed {
        return Err(SkillDocumentError::MissingFrontmatter);
    }
    let body: String = lines.collect();
    let value: Value = serde_yaml_ng::from_str(&frontmatter)
        .map_err(|e| SkillDocumentError::Yaml(e.to_string()))?;
    let Value::Object(map) = value else {
        return Err(SkillDocumentError::NotMapping);
    };
    let field = |key: &'static str| match map.get(key) {
        None => Err(SkillDocumentError::MissingField(key)),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        Some(_) => Err(SkillDocumentError::InvalidField(key)),
    };
    let name = field("name")?;
    let description = field("description")?;
    if !is_skill_name(&name) {
        return Err(SkillDocumentError::InvalidName(name));
    }
    Ok(SkillDocument {
        name,
        description,
        body,
    })
}
