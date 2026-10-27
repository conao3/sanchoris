use sanchoris_core::db::DbError;
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
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("injected fault at {0}")]
    Injected(&'static str),
}

impl From<sqlx::Error> for WorkerError {
    fn from(error: sqlx::Error) -> Self {
        Self::Db(DbError::Sqlx(error))
    }
}

pub type Result<T> = std::result::Result<T, WorkerError>;
