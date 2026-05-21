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

    // ────────────────────────────────────────────────────────────────────
    // Capture engine errors (Phase 1 T1.3)
    //
    // Each carries a `[CAPTURE_*]` / `[WEBVIEW_*]` / `[STORAGE_*]`
    // prefix in its Display so the frontend `mapErr()` can dispatch
    // off the prefix without needing a separate `code` field.
    // ────────────────────────────────────────────────────────────────────
    #[error("[CAPTURE_NO_RECIPE] integration group has no capture recipe declared")]
    CaptureNoRecipe,

    #[error("[CAPTURE_ALREADY_RUNNING] session already in-flight for {tool_id}/{group_id}")]
    CaptureAlreadyRunning { tool_id: String, group_id: String },

    #[error("[CAPTURE_SESSION_NOT_FOUND] session_id={0} not found or already expired")]
    CaptureSessionNotFound(String),

    #[error("[WEBVIEW_CREATE_FAILED] failed to create capture webview: {0}")]
    WebviewCreateFailed(String),

    #[error("[CAPTURE_TIMEOUT] session expired after {timeout_secs}s without completion")]
    CaptureTimeout { timeout_secs: u32 },

    #[error("[CAPTURE_RULE_FAILED] rule #{rule_index} ({rule_kind}) failed: {reason}")]
    CaptureRuleFailed {
        rule_index: usize,
        rule_kind: &'static str,
        reason: String,
    },

    #[error("[CAPTURE_INVALID_URL] login_url is not a valid http(s) URL: {0}")]
    CaptureInvalidUrl(String),

    #[error("[CAPTURE_INVALID_TARGET_FIELD] rule #{rule_index} references unknown field {field}")]
    CaptureInvalidTargetField { rule_index: usize, field: String },
}

pub type IntegrationResult<T> = Result<T, IntegrationError>;

#[cfg(test)]
mod capture_error_tests {
    use super::*;

    #[test]
    fn capture_no_recipe_message() {
        let e = IntegrationError::CaptureNoRecipe;
        let s = e.to_string();
        assert!(s.contains("CAPTURE_NO_RECIPE"));
        assert!(s.contains("no capture recipe"));
    }

    #[test]
    fn capture_already_running_message() {
        let e = IntegrationError::CaptureAlreadyRunning {
            tool_id: "enscan-go".into(),
            group_id: "aqc".into(),
        };
        let s = e.to_string();
        assert!(s.contains("CAPTURE_ALREADY_RUNNING"));
        assert!(s.contains("enscan-go/aqc"));
    }

    #[test]
    fn capture_rule_failed_includes_kind_and_reason() {
        let e = IntegrationError::CaptureRuleFailed {
            rule_index: 2,
            rule_kind: "cookie",
            reason: "BDUSS not found".into(),
        };
        let s = e.to_string();
        assert!(s.contains("CAPTURE_RULE_FAILED"));
        assert!(s.contains("#2"));
        assert!(s.contains("cookie"));
        assert!(s.contains("BDUSS not found"));
    }

    #[test]
    fn capture_invalid_target_field_message() {
        let e = IntegrationError::CaptureInvalidTargetField {
            rule_index: 0,
            field: "ghost_field".into(),
        };
        let s = e.to_string();
        assert!(s.contains("CAPTURE_INVALID_TARGET_FIELD"));
        assert!(s.contains("ghost_field"));
    }

    #[test]
    fn capture_timeout_message() {
        let e = IntegrationError::CaptureTimeout { timeout_secs: 300 };
        let s = e.to_string();
        assert!(s.contains("CAPTURE_TIMEOUT"));
        assert!(s.contains("300"));
    }
}
