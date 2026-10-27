use crate::builtin::builtin_skills;
use crate::config::{CONFIG_FILE, Config, parse_config_yaml};
use crate::error::{ValidationError, ValidationErrors, at};
use crate::skill::{SkillDefinition, parse_skill_yaml};
use crate::skill_md::parse_skill_md;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

pub const SKILLS_DIR: &str = "skills";
pub const SKILL_MD: &str = "SKILL.md";
pub const SKILL_YAML: &str = "skill.yaml";

pub type ConfigFiles = BTreeMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillOrigin {
    Repository,
    BuiltIn,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
    pub skill_md: String,
    pub skill_yaml: Option<String>,
    pub definition: Option<SkillDefinition>,
    pub origin: SkillOrigin,
}

impl Skill {
    pub fn is_reliable(&self) -> bool {
        self.definition.is_some()
    }

    pub fn from_sources(
        skill_md: &str,
        skill_yaml: Option<&str>,
        origin: SkillOrigin,
        location: &str,
    ) -> Result<Skill, Vec<ValidationError>> {
        let mut errors = Vec::new();
        let document = parse_skill_md(skill_md)
            .map_err(|e| errors.push(ValidationError::new(at(location, SKILL_MD), e.to_string())))
            .ok();
        let definition = match skill_yaml {
            Some(text) => parse_skill_yaml(text)
                .map_err(|e| {
                    errors.push(ValidationError::new(
                        at(location, SKILL_YAML),
                        format!("invalid skill.yaml: {e}"),
                    ))
                })
                .ok()
                .map(Some),
            None => Some(None),
        };
        match (document, definition) {
            (Some(document), Some(definition)) if errors.is_empty() => Ok(Skill {
                name: document.name,
                description: document.description,
                body: document.body,
                skill_md: skill_md.to_string(),
                skill_yaml: skill_yaml.map(str::to_string),
                definition,
                origin,
            }),
            _ => Err(errors),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ConfigRepository {
    pub config: Config,
    pub skills: IndexMap<String, Skill>,
}

impl ConfigRepository {
    pub fn skill(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }
}

pub fn read_config_dir(root: &Path) -> io::Result<ConfigFiles> {
    let mut files = ConfigFiles::new();
    files.insert(
        CONFIG_FILE.to_string(),
        std::fs::read_to_string(root.join(CONFIG_FILE))?,
    );
    let skills = root.join(SKILLS_DIR);
    if !skills.exists() {
        return Ok(files);
    }
    for entry in std::fs::read_dir(&skills)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir = entry.file_name().into_string().map_err(|name| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("skill directory name {name:?} is not UTF-8"),
            )
        })?;
        for file in [SKILL_MD, SKILL_YAML] {
            let path = entry.path().join(file);
            if path.exists() {
                files.insert(
                    format!("{SKILLS_DIR}/{dir}/{file}"),
                    std::fs::read_to_string(path)?,
                );
            }
        }
    }
    Ok(files)
}

struct SkillFiles<'a> {
    skill_md: Option<&'a str>,
    skill_yaml: Option<&'a str>,
}

pub fn parse_repository(files: &ConfigFiles) -> Result<ConfigRepository, ValidationErrors> {
    let mut errors = Vec::new();
    let config = match files.get(CONFIG_FILE) {
        Some(text) => parse_config_yaml(text).map_err(|e| errors.extend(e.0)).ok(),
        None => {
            errors.push(ValidationError::new(
                CONFIG_FILE,
                "the configuration repository has no sanchoris.yaml at its root",
            ));
            None
        }
    };

    let mut dirs: BTreeMap<&str, SkillFiles> = BTreeMap::new();
    for (path, text) in files {
        let Some(rest) = path.strip_prefix("skills/") else {
            continue;
        };
        let Some((dir, file)) = rest.split_once('/') else {
            continue;
        };
        let entry = dirs.entry(dir).or_insert(SkillFiles {
            skill_md: None,
            skill_yaml: None,
        });
        match file {
            SKILL_MD => entry.skill_md = Some(text),
            SKILL_YAML => entry.skill_yaml = Some(text),
            _ => {}
        }
    }

    let mut skills = IndexMap::new();
    for (dir, sources) in dirs {
        let location = at("Skills", dir);
        let Some(skill_md) = sources.skill_md else {
            if sources.skill_yaml.is_some() {
                errors.push(ValidationError::new(
                    at(&location, SKILL_MD),
                    "a skill directory with skill.yaml must also have SKILL.md",
                ));
            }
            continue;
        };
        match Skill::from_sources(
            skill_md,
            sources.skill_yaml,
            SkillOrigin::Repository,
            &location,
        ) {
            Ok(skill) if skill.name != dir => errors.push(ValidationError::new(
                at(&location, SKILL_MD),
                format!(
                    "frontmatter name {:?} must equal the directory name {dir:?}",
                    skill.name
                ),
            )),
            Ok(skill) => {
                skills.insert(skill.name.clone(), skill);
            }
            Err(e) => errors.extend(e),
        }
    }
    for skill in builtin_skills() {
        if !skills.contains_key(&skill.name) {
            skills.insert(skill.name.clone(), skill);
        }
    }

    match config {
        Some(config) if errors.is_empty() => Ok(ConfigRepository { config, skills }),
        _ => Err(ValidationErrors(errors)),
    }
}
