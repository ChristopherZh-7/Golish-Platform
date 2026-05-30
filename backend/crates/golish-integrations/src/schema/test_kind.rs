//! [`TestKind`] — how to verify the configured credentials actually work
//! (builtin provider test, exec a command, or issue an HTTP request).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// How to verify the configured credentials actually work.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TestKind {
    /// Delegate to a provider-defined test. Used by `IntelProvider`
    /// implementors that already have their own `test_connection`.
    Builtin,

    /// Spawn a command. `{{exec}}` is substituted with the tool's
    /// resolved executable path. Stdout is matched against
    /// `ok_regex` / `fail_regex`.
    Exec {
        /// Shell-style command template.
        cmd: String,
        /// On match → [`crate::types::HealthStatus::Healthy`].
        ok_regex: String,
        /// On match (before checking ok_regex) →
        /// [`crate::types::HealthStatus::Invalid`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fail_regex: Option<String>,
        #[serde(default = "default_timeout_30")]
        timeout_secs: u32,
    },

    /// Issue an HTTP request and check the status code.
    Http {
        method: String,
        /// URL template, supports `{{value:field_key}}` substitution.
        url: String,
        /// Header templates, supports the same substitution.
        #[serde(default)]
        headers: HashMap<String, String>,
        /// Inclusive `[lo, hi]`. Default 200..=299.
        #[serde(default = "default_ok_range")]
        ok_status_range: (u16, u16),
        #[serde(default = "default_timeout_30")]
        timeout_secs: u32,
    },
}

fn default_timeout_30() -> u32 {
    30
}

fn default_ok_range() -> (u16, u16) {
    (200, 299)
}
