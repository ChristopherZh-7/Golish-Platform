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
}
