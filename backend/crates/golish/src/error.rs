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
use serde_json::Value as JsonValue;
use thiserror::Error;

/// Sentinel prefix used to mark a serialized error as "i18n-coded" so the
/// frontend can route it through `localizeBackendError` (which calls
/// `i18next.t("backend.errors." + code, params)`). Keep this prefix in sync
/// with `frontend/lib/errors.ts`.
pub const I18N_ERROR_PREFIX: &str = "[i18n]";

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

    /// I18n-coded error. `code` resolves on the frontend to a translated
    /// string under the `backend.errors.` namespace; `params` is forwarded
    /// to i18next interpolation. Use this for any user-facing error you
    /// want translated. The `Display` impl renders the raw English code so
    /// CLI / log consumers still see something readable.
    #[error("[i18n:{code}] {params}")]
    I18n { code: String, params: JsonValue },
}

impl GolishError {
    /// Wrap an `anyhow::Error` as an internal error.
    pub fn from_anyhow(err: anyhow::Error) -> Self {
        Self::Internal(err.to_string())
    }

    /// Construct an i18n-coded error. `code` should be a dotted path under
    /// the `backend.errors.` namespace (e.g. `"java.install_failed"`).
    /// `params` are passed to i18next interpolation; pass `serde_json::json!({})`
    /// when there are none.
    pub fn i18n(code: impl Into<String>, params: JsonValue) -> Self {
        Self::I18n {
            code: code.into(),
            params,
        }
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
        // I18n errors get a sentinel-prefixed format the frontend can parse
        // back into `{ code, params }`; everything else stays as a plain
        // user-facing string for backwards compatibility with the existing
        // `catch (err) { toast(String(err)) }` pattern across the app.
        match self {
            Self::I18n { code, params } => {
                let payload = format!("{}{}|{}", I18N_ERROR_PREFIX, code, params);
                serializer.serialize_str(&payload)
            }
            _ => serializer.serialize_str(&self.to_string()),
        }
    }
}

/// Alias kept for the IPC boundary; downstream callers may use either name.
#[allow(dead_code)]
pub type IpcError = GolishError;

pub type Result<T> = std::result::Result<T, GolishError>;
