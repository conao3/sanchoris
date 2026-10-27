use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ValidationError {
    pub location: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(location: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            location: location.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.location, self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ValidationErrors(pub Vec<ValidationError>);

impl ValidationErrors {
    pub fn iter(&self) -> std::slice::Iter<'_, ValidationError> {
        self.0.iter()
    }

    pub fn at(&self, location: &str) -> Vec<&ValidationError> {
        self.0.iter().filter(|e| e.location == location).collect()
    }

    pub fn has(&self, location: &str) -> bool {
        self.0.iter().any(|e| e.location == location)
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

pub(crate) fn at(location: &str, key: &str) -> String {
    if location.is_empty() {
        key.to_string()
    } else {
        format!("{location}.{key}")
    }
}

pub(crate) fn index(location: &str, i: usize) -> String {
    format!("{location}[{i}]")
}
