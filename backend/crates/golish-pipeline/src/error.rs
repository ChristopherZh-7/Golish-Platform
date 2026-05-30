//! Domain error types for the pipeline crate.

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("step failed: {0}")]
    StepFailed(String),

    #[error("template: {0}")]
    Template(String),

    #[error("storage: {0}")]
    Storage(String),

    #[error("tool resolve: {0}")]
    ToolResolve(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("db: {0}")]
    Db(#[from] sqlx::Error),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

pub type PipelineResult<T> = Result<T, PipelineError>;

impl From<golish_db::DbError> for PipelineError {
    fn from(err: golish_db::DbError) -> Self {
        match err {
            golish_db::DbError::Sqlx(e) => Self::Db(e),
            other => Self::Other(other.into()),
        }
    }
}
