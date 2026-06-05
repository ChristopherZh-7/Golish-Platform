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
//!
//! # Crate-local conversions
//!
//! Conversions from `golish`-internal error types (e.g. `HistoryError`) live in
//! the `golish` crate's `error.rs` next to the type they convert, because the
//! orphan rule lets a downstream crate `impl From<LocalType> for GolishError`.

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

    /// Stable, machine-readable error code for the IPC boundary.
    /// MIRROR of `frontend/lib/api/error-codes.ts`; keep both in sync.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "IO",
            Self::Database(_) => "DATABASE",
            Self::Json(_) => "JSON",
            Self::Http(_) => "HTTP",
            Self::Pty(_) => "PTY",
            Self::Tool(_) => "TOOL",
            Self::Skills(_) => "SKILLS",
            Self::Pentest(_) => "PENTEST",
            Self::VulnIntel(_) => "VULN_INTEL",
            Self::ScanRunner(_) => "SCAN_RUNNER",
            Self::SessionNotFound(_) => "SESSION_NOT_FOUND",
            Self::NotFound(_) => "NOT_FOUND",
            Self::Validation(_) => "VALIDATION",
            Self::Config(_) => "CONFIG",
            Self::Internal(_) => "INTERNAL",
        }
    }
}

impl From<anyhow::Error> for GolishError {
    fn from(err: anyhow::Error) -> Self {
        Self::from_anyhow(err)
    }
}

impl From<golish_db::DbError> for GolishError {
    fn from(err: golish_db::DbError) -> Self {
        // Preserve the stable IPC code where golish-db's typed error maps onto an
        // existing GolishError variant; fall back to Internal for the rest.
        match err {
            golish_db::DbError::Sqlx(e) => Self::Database(e),
            golish_db::DbError::Json(e) => Self::Json(e),
            golish_db::DbError::Io(e) => Self::Io(e),
            golish_db::DbError::NotFound(m) => Self::NotFound(m),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<String> for GolishError {
    fn from(s: String) -> Self {
        Self::Internal(s)
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

impl From<zip::result::ZipError> for GolishError {
    fn from(err: zip::result::ZipError) -> Self {
        Self::Internal(format!("zip error: {err}"))
    }
}

impl From<base64::DecodeError> for GolishError {
    fn from(err: base64::DecodeError) -> Self {
        Self::Validation(format!("base64 decode error: {err}"))
    }
}

impl From<std::path::StripPrefixError> for GolishError {
    fn from(err: std::path::StripPrefixError) -> Self {
        Self::Internal(format!("path strip prefix error: {err}"))
    }
}

impl Serialize for GolishError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        // I1: emit a stable { code, message } envelope. `message` is retained
        // (still human-readable) so older string consumers degrade gracefully;
        // `code` lets the frontend branch via lib/api/error-codes.ts.
        let mut state = serializer.serialize_struct("GolishError", 2)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

/// Alias kept for the IPC boundary; downstream callers may use either name.
#[allow(dead_code)]
pub type IpcError = GolishError;

pub type Result<T> = std::result::Result<T, GolishError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_stable_per_variant() {
        assert_eq!(GolishError::NotFound("x".into()).code(), "NOT_FOUND");
        assert_eq!(GolishError::Validation("x".into()).code(), "VALIDATION");
        assert_eq!(GolishError::Config("x".into()).code(), "CONFIG");
        assert_eq!(GolishError::Internal("x".into()).code(), "INTERNAL");
        assert_eq!(
            GolishError::SessionNotFound("s".into()).code(),
            "SESSION_NOT_FOUND"
        );
    }

    #[test]
    fn serializes_with_code_and_message() {
        let err = GolishError::NotFound("widget 42".to_string());
        let v = serde_json::to_value(&err).expect("serialize");
        assert_eq!(v["code"], "NOT_FOUND");
        assert_eq!(v["message"], "Not found: widget 42");
    }

    #[test]
    fn validation_serializes_with_validation_code() {
        let err = GolishError::Validation("bad input".to_string());
        let v = serde_json::to_value(&err).expect("serialize");
        assert_eq!(v["code"], "VALIDATION");
        assert_eq!(v["message"], "Validation error: bad input");
    }
}
