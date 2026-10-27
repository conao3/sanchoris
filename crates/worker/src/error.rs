use sanchoris_core::db::DbError;
use sanchoris_core::ports::PortError;
use sanchoris_core::skillrun::SkillRunError;
use thiserror::Error;

use crate::ports::{GitHubError, RunnerError};

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    SkillRun(#[from] SkillRunError),
    #[error(transparent)]
    GitHub(#[from] GitHubError),
    #[error(transparent)]
    Runner(#[from] RunnerError),
    #[error(transparent)]
    Port(#[from] PortError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("injected fault at {0}")]
    Injected(&'static str),
}

impl WorkerError {
    pub fn is_conflict(&self) -> bool {
        match self {
            Self::Db(error) => error.is_conflict(),
            Self::SkillRun(error) => error.is_conflict(),
            _ => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, WorkerError>;
