//! Configuration knobs for the context manager: trim policy and high-level
//! settings facade consumed by application configuration layers.

use serde::{Deserialize, Serialize};

use crate::token_trunc::DEFAULT_MAX_TOOL_RESPONSE_TOKENS;

/// Configuration for context trimming behaviour.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextTrimConfig {
    /// Enable automatic context trimming.
    pub enabled: bool,
    /// Target utilization ratio (0.0–1.0) when trimming.
    pub target_utilization: f64,
    /// Enable aggressive trimming when critically low on space.
    pub aggressive_on_critical: bool,
    /// Maximum tool response tokens before truncation.
    pub max_tool_response_tokens: usize,
}

impl Default for ContextTrimConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default
            target_utilization: 0.7,
            aggressive_on_critical: true,
            max_tool_response_tokens: DEFAULT_MAX_TOOL_RESPONSE_TOKENS,
        }
    }
}

/// High-level configuration for context management behaviour.
///
/// This struct is designed to be easily constructed from application
/// settings (like `ContextSettings` from `golish-settings`) without creating
/// a dependency between `golish-context` and `golish-settings`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextManagerConfig {
    /// Enable context window management (pruning, truncation, token budgeting).
    pub enabled: bool,
    /// Context utilization threshold (0.0–1.0) at which pruning is triggered.
    pub compaction_threshold: f64,
    /// Number of recent turns to protect from pruning.
    pub protected_turns: usize,
    /// Minimum seconds between pruning operations (cooldown).
    pub cooldown_seconds: u64,
}

impl Default for ContextManagerConfig {
    fn default() -> Self {
        Self {
            enabled: true,              // Enabled by default
            compaction_threshold: 0.80, // Trigger at 80% utilization
            protected_turns: 2,         // Protect last 2 turns
            cooldown_seconds: 60,       // 1 minute cooldown
        }
    }
}
