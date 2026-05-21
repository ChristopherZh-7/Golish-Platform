//! Runtime value types: what the UI reads back from storage, plus the
//! result of running a [`crate::schema::TestKind`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What the UI sees when calling `integrations_get`.
///
/// For secret fields, [`Self::value`] is intentionally [`None`] —
/// reveal-on-demand goes through a separate `integrations_get_cleartext`
/// path (not exposed in `Phase 1` IPC yet) so a stale UI never holds
/// secrets in memory longer than needed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FieldValue {
    /// True when the field has been written before.
    pub has_value: bool,

    /// Plain value for non-secret fields. Always `None` for secret
    /// fields, regardless of whether they've been set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    /// Short obfuscated preview (e.g. `"AKIA****WXYZ"`) shown so the
    /// user can confirm which credential is stored without revealing
    /// it. Computed by the storage backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_hint: Option<String>,

    /// When this field was last updated. `None` for fields that were
    /// never written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

impl FieldValue {
    /// A blank slot (nothing configured).
    pub fn empty() -> Self {
        Self {
            has_value: false,
            value: None,
            display_hint: None,
            updated_at: None,
        }
    }

    /// A configured plain (non-secret) value.
    pub fn plain(value: impl Into<String>, updated_at: DateTime<Utc>) -> Self {
        Self {
            has_value: true,
            value: Some(value.into()),
            display_hint: None,
            updated_at: Some(updated_at),
        }
    }

    /// A configured secret — value is intentionally not surfaced.
    pub fn secret_set(display_hint: Option<String>, updated_at: DateTime<Utc>) -> Self {
        Self {
            has_value: true,
            value: None,
            display_hint,
            updated_at: Some(updated_at),
        }
    }
}

/// High-level outcome of testing a configured credential group.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// Test passed — credential is currently valid.
    Healthy,

    /// Test indicated the credential is unusable (auth rejected,
    /// signature invalid, ...).
    Invalid,

    /// Credential structure looks right but the server says it's
    /// expired (refresh / re-issue needed).
    Expired,

    /// Auth succeeded but the user is currently throttled or out of
    /// quota.
    RateLimited,

    /// Status couldn't be determined (network failure, timeout,
    /// schema has no test recipe). Not a hard failure — the user
    /// should retry.
    Unknown,
}

/// Returned by `integrations_test`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntegrationHealth {
    pub status: HealthStatus,
    /// Human-readable message (truncated stdout / response body,
    /// or descriptive error). Backend MUST NOT include the actual
    /// secret value here.
    pub message: String,
    pub tested_at: DateTime<Utc>,
}

impl IntegrationHealth {
    pub fn healthy(message: impl Into<String>) -> Self {
        Self {
            status: HealthStatus::Healthy,
            message: message.into(),
            tested_at: Utc::now(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            status: HealthStatus::Invalid,
            message: message.into(),
            tested_at: Utc::now(),
        }
    }

    pub fn unknown(message: impl Into<String>) -> Self {
        Self {
            status: HealthStatus::Unknown,
            message: message.into(),
            tested_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_value_empty_round_trip() {
        let v = FieldValue::empty();
        let json = serde_json::to_string(&v).unwrap();
        // empty should not include None fields
        assert_eq!(json, r#"{"has_value":false}"#);
        let back: FieldValue = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn field_value_secret_set_hides_value() {
        let v = FieldValue::secret_set(Some("AKIA****WXYZ".into()), Utc::now());
        assert!(v.has_value);
        assert_eq!(v.value, None);
        assert_eq!(v.display_hint.as_deref(), Some("AKIA****WXYZ"));
    }

    #[test]
    fn health_status_serde() {
        let raw = r#""rate_limited""#;
        let s: HealthStatus = serde_json::from_str(raw).unwrap();
        assert_eq!(s, HealthStatus::RateLimited);
        let back = serde_json::to_string(&s).unwrap();
        assert_eq!(back, raw);
    }

    #[test]
    fn integration_health_constructors() {
        let h = IntegrationHealth::healthy("OK");
        assert_eq!(h.status, HealthStatus::Healthy);
        assert_eq!(h.message, "OK");

        let h = IntegrationHealth::invalid("auth failed");
        assert_eq!(h.status, HealthStatus::Invalid);

        let h = IntegrationHealth::unknown("timeout");
        assert_eq!(h.status, HealthStatus::Unknown);
    }

    // ────────────────────────────────────────────────────────────────
    // Capture runtime type tests (Phase 1 T1.2)
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn capture_state_is_terminal() {
        assert!(!CaptureState::WaitingLogin.is_terminal());
        assert!(!CaptureState::Navigating.is_terminal());
        assert!(!CaptureState::Extracting.is_terminal());
        assert!(CaptureState::Captured.is_terminal());
        assert!(CaptureState::Partial.is_terminal());
        assert!(CaptureState::Failed.is_terminal());
        assert!(CaptureState::Timeout.is_terminal());
        assert!(CaptureState::Cancelled.is_terminal());
    }

    #[test]
    fn capture_state_serde_round_trip() {
        let s = CaptureState::WaitingLogin;
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(j, "\"waiting_login\"");
        let back: CaptureState = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);

        let s = CaptureState::Captured;
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(j, "\"captured\"");
    }

    #[test]
    fn capture_session_info_minimal_round_trip() {
        let info = CaptureSessionInfo {
            session_id: "abc-123".into(),
            tool_id: "enscan-go".into(),
            group_id: "aqc".into(),
            state: CaptureState::WaitingLogin,
            login_url: "https://aiqicha.baidu.com".into(),
            expected_fields: vec!["cookies.aqc".into()],
            captured_fields: vec![],
            failed_rules: vec![],
            error_message: None,
            expires_at: Some(1716291660000),
            updated_at: 1716291660000,
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: CaptureSessionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, back);
    }

    #[test]
    fn failed_rule_serde() {
        let f = FailedRule {
            rule_index: 2,
            reason: "cookie not found".into(),
        };
        let j = serde_json::to_string(&f).unwrap();
        let back: FailedRule = serde_json::from_str(&j).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn capture_event_payload_minimal() {
        let ev = CaptureEventPayload {
            session_id: "sid".into(),
            tool_id: "t".into(),
            group_id: "g".into(),
            state: CaptureState::Captured,
            captured_fields: vec!["cookies.aqc".into()],
            failed_rules: vec![],
            error_message: None,
        };
        let j = serde_json::to_string(&ev).unwrap();
        // skip_serializing_if = Vec::is_empty + Option::is_none should
        // omit failed_rules & error_message but keep captured_fields.
        assert!(j.contains("\"captured_fields\""));
        assert!(!j.contains("\"failed_rules\""));
        assert!(!j.contains("\"error_message\""));
    }
}

// ────────────────────────────────────────────────────────────────────────
// Capture runtime types (Phase 1 T1.2)
//
// Architecture: docs/design/2026-05-21-credential-capture-engine.md
// ────────────────────────────────────────────────────────────────────────

/// State of an in-flight capture session.
///
/// State machine (see design doc §5.1):
///
/// ```text
///   waiting_login → navigating → extracting → captured / partial
///                              ↓
///                        failed / timeout
///                              ↓
///   (any non-terminal) → cancelled (on user action)
/// ```
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CaptureState {
    /// Webview opened, user has not yet logged in.
    WaitingLogin,
    /// `success_url_pattern` matched; engine is currently navigating
    /// to `visit_url` (when set) before extraction.
    Navigating,
    /// Rules are executing.
    Extracting,
    /// All `required` rules succeeded → fields written to vault.
    Captured,
    /// Some optional rules failed but partial credentials written.
    Partial,
    /// At least one `required` rule failed.
    Failed,
    /// Hit `timeout_secs` without completing.
    Timeout,
    /// User clicked cancel or closed the window manually.
    Cancelled,
}

impl CaptureState {
    /// Terminal states never transition further. The engine drops
    /// the webview + per-session data_dir on transition into any of
    /// these.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Captured
                | Self::Partial
                | Self::Failed
                | Self::Timeout
                | Self::Cancelled
        )
    }
}

/// Per-rule failure detail. `rule_index` is 0-based and references
/// [`crate::schema::CaptureRecipe::rules`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailedRule {
    pub rule_index: usize,
    pub reason: String,
}

/// Snapshot of a session, returned by `integrations_capture_start` /
/// `_status`.
///
/// Timestamps are Unix milliseconds (NOT chrono) because the frontend
/// `Date.now()` works in the same unit — keeps the UI countdown logic
/// trivial.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureSessionInfo {
    pub session_id: String,
    pub tool_id: String,
    pub group_id: String,
    pub state: CaptureState,
    pub login_url: String,
    /// `target_field` values declared in the recipe rules, in order.
    /// UI uses this to render "we will try to harvest: X, Y, Z".
    pub expected_fields: Vec<String>,
    /// Fields actually written (subset of `expected_fields`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captured_fields: Vec<String>,
    /// Per-rule failure detail.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_rules: Vec<FailedRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Unix milliseconds. `None` for already-terminal sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// Unix milliseconds when state last transitioned.
    pub updated_at: i64,
}

/// Event payload emitted on the `"integration-capture"` channel.
/// Subset of [`CaptureSessionInfo`] — the frontend already has
/// `expires_at` from the `_start` response and doesn't need it again
/// for every transition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureEventPayload {
    pub session_id: String,
    pub tool_id: String,
    pub group_id: String,
    pub state: CaptureState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captured_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_rules: Vec<FailedRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}
