//! Domain error types for the scan-runner crate.

#[derive(Debug, thiserror::Error)]
pub enum ScanRunnerError {
    #[error("nuclei: {0}")]
    Nuclei(String),

    #[error("whatweb: {0}")]
    WhatWeb(String),

    #[error("feroxbuster: {0}")]
    Feroxbuster(String),

    #[error("storage: {0}")]
    Storage(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("db: {0}")]
    Db(#[from] sqlx::Error),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

pub type ScanRunnerResult<T> = Result<T, ScanRunnerError>;
