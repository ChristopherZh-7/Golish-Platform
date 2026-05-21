//! Error type shared across the integrations crate.
//!
//! Maps cleanly to the IPC error codes documented in
//! `docs/design/2026-05-21-integrations.md §4`:
//!
//! | Code  | Variant                       |
//! |-------|-------------------------------|
//! | 40001 | [`IntegrationError::Validation`] |
//! | 40401 | [`IntegrationError::SchemaNotFound`] |
//! | 40901 | [`IntegrationError::ExternalFileCorrupt`] |
//! | 50001 | [`IntegrationError::Internal`] |

use thiserror::Error;

#[derive(Debug, Error)]
pub enum IntegrationError {
    /// 40001 — field-level validation failure (required missing, wrong
    /// type, regex mismatch, ...).
    #[error("validation error: {0}")]
    Validation(String),

    /// 40401 — referenced schema does not exist.
    #[error("integration not found: {0}")]
    SchemaNotFound(String),

    /// 40901 — external file exists but couldn't be parsed (probably
    /// edited by the external process into a state we can't merge into).
    #[error("external file corrupt at {path}: {reason}")]
    ExternalFileCorrupt { path: String, reason: String },

    /// I/O failure (read / write / fsync).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// YAML (de)serialization failure.
    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    /// JSON (de)serialization failure.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// 50001 — anything else.
    #[error("internal error: {0}")]
    Internal(String),
}

pub type IntegrationResult<T> = Result<T, IntegrationError>;
