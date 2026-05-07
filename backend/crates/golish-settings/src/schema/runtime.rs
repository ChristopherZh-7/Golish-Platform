//! Runtime/observability settings: indexer, context, telemetry, notifications,
//! and indexed-codebase metadata.
//!
//! Lives in one module because these structs share a lifecycle theme: they
//! configure the *agent runtime* (long-running indexers, context-window
//! compaction, native OS notifications) plus the OpenTelemetry/Langfuse
//! tracing pipeline used to observe it.

use super::defaults::{
    default_compaction_threshold, default_context_enabled, default_cooldown_seconds,
    default_protected_turns,
};
use super::enums::IndexLocation;
use serde::{Deserialize, Serialize};

/// Code indexer settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexerSettings {
    /// Where to store index files: `"global"` or `"local"`.
    pub index_location: IndexLocation,
}

impl Default for IndexerSettings {
    fn default() -> Self {
        Self {
            index_location: IndexLocation::Global,
        }
    }
}

/// Context window management settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextSettings {
    /// Enable context window management.
    #[serde(default = "default_context_enabled")]
    pub enabled: bool,

    /// Context utilization threshold (0.0-1.0) at which compaction is triggered.
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: f64,

    /// DEPRECATED: No longer used. Compaction replaces pruning.
    /// Kept for backwards compatibility with existing config files.
    #[serde(default = "default_protected_turns")]
    pub protected_turns: usize,

    /// DEPRECATED: No longer used. Compaction replaces pruning.
    /// Kept for backwards compatibility with existing config files.
    #[serde(default = "default_cooldown_seconds")]
    pub cooldown_seconds: u64,
}

impl Default for ContextSettings {
    fn default() -> Self {
        Self {
            enabled: default_context_enabled(),
            compaction_threshold: default_compaction_threshold(),
            protected_turns: default_protected_turns(),
            cooldown_seconds: default_cooldown_seconds(),
        }
    }
}

/// Telemetry and observability settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TelemetrySettings {
    /// Langfuse integration settings.
    pub langfuse: LangfuseSettings,
}

/// Langfuse tracing configuration.
///
/// Langfuse provides LLM observability via OpenTelemetry.
/// See: <https://langfuse.com/docs/integrations/opentelemetry>.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LangfuseSettings {
    /// Enable Langfuse tracing.
    pub enabled: bool,

    /// Langfuse host URL (defaults to `https://cloud.langfuse.com`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,

    /// Langfuse public key (supports `$ENV_VAR` syntax, or set `LANGFUSE_PUBLIC_KEY` env var).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,

    /// Langfuse secret key (supports `$ENV_VAR` syntax, or set `LANGFUSE_SECRET_KEY` env var).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_key: Option<String>,

    /// Sampling ratio (0.0 to 1.0, default 1.0 = sample everything).
    /// Use lower values for high-traffic production deployments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling_ratio: Option<f64>,
}

/// Native OS notification settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationsSettings {
    /// Enable native OS notifications for agent/command completion.
    pub native_enabled: bool,

    /// Enable in-app notification sounds (independent of OS notifications).
    /// Defaults to true.
    pub sound_enabled: bool,

    /// Notification sound (macOS system sound name like `"Blow"` or `"Ping"`).
    /// If `None`, defaults to `"Blow"` on macOS and no sound on other platforms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
}

impl Default for NotificationsSettings {
    fn default() -> Self {
        Self {
            native_enabled: false,
            sound_enabled: true,
            sound: None,
        }
    }
}

/// Configuration for an indexed codebase.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodebaseConfig {
    /// Path to the codebase (supports `~` for home directory).
    pub path: String,

    /// Memory file associated with this codebase: `"AGENTS.md"`, `"CLAUDE.md"`, or `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_file: Option<String>,
}
