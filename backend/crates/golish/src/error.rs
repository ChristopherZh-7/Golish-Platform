//! Unified error type for the Golish application boundary.
//!
//! `GolishError` is the top-level error type used by Tauri commands and the CLI.
//! Domain crates define their own fine-grained error types (`PtyError`,
//! `ToolError`, etc.); this enum wraps them at the application boundary so that
//! Tauri commands can return a single error type that serializes to a
//! user-friendly string.
//!
//! # Migration guide
//!
//! Tauri commands that currently return `Result<T, String>` should migrate to
//! `Result<T, GolishError>` and use `?` with the `From` impls defined here.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum GolishError {
    // -- Infrastructure -------------------------------------------------------

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    // -- Domain crate errors --------------------------------------------------

    #[error("{0}")]
    Pty(#[from] golish_pty::PtyError),

    #[error("{0}")]
    Tool(#[from] golish_tools::ToolError),

    #[error("{0}")]
    Skills(#[from] golish_skills::SkillsError),

    #[error("{0}")]
    Pentest(#[from] golish_pentest::PentestError),

    #[error("{0}")]
    VulnIntel(#[from] golish_vuln_intel::VulnIntelError),

    #[error("{0}")]
    Pipeline(#[from] golish_pipeline::PipelineError),

    #[error("{0}")]
    ScanRunner(#[from] golish_scan_runner::ScanRunnerError),

    // -- Application-level errors ---------------------------------------------

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("{0}")]
    Internal(String),
}

impl GolishError {
    /// Wrap an `anyhow::Error` as an internal error.
    pub fn from_anyhow(err: anyhow::Error) -> Self {
        Self::Internal(err.to_string())
    }
}

impl From<anyhow::Error> for GolishError {
    fn from(err: anyhow::Error) -> Self {
        Self::from_anyhow(err)
    }
}

impl From<String> for GolishError {
    fn from(s: String) -> Self {
        Self::Internal(s)
    }
}

impl From<crate::history::HistoryError> for GolishError {
    fn from(err: crate::history::HistoryError) -> Self {
        Self::Internal(err.to_string())
    }
}

impl From<std::string::FromUtf8Error> for GolishError {
    fn from(err: std::string::FromUtf8Error) -> Self {
        Self::Internal(err.to_string())
    }
}

impl From<uuid::Error> for GolishError {
    fn from(err: uuid::Error) -> Self {
        Self::Internal(err.to_string())
    }
}

impl From<&str> for GolishError {
    fn from(s: &str) -> Self {
        Self::Internal(s.to_string())
    }
}

impl Serialize for GolishError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type IpcError = GolishError;

pub type Result<T> = std::result::Result<T, GolishError>;
