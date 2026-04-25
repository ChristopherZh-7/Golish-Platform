//! Output / observation types emitted by the context manager: alert events,
//! efficiency metrics, the diagnostics summary, warning info, and the
//! enforcement-step result envelope.

use rig::message::Message;
use serde::{Deserialize, Serialize};

use crate::token_budget::TokenAlertLevel;

/// Efficiency metrics after context operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextEfficiency {
    /// Utilization before operation.
    pub utilization_before: f64,
    /// Utilization after operation.
    pub utilization_after: f64,
    /// Tokens freed.
    pub tokens_freed: usize,
    /// Messages pruned.
    pub messages_pruned: usize,
    /// Tool responses truncated.
    pub tool_responses_truncated: usize,
    /// Timestamp of operation.
    pub timestamp: u64,
}

/// Events emitted during context management.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContextEvent {
    /// Warning threshold exceeded.
    WarningThreshold {
        utilization: f64,
        total_tokens: usize,
        max_tokens: usize,
    },
    /// Alert threshold exceeded.
    AlertThreshold {
        utilization: f64,
        total_tokens: usize,
        max_tokens: usize,
    },
    /// Tool response was truncated.
    ToolResponseTruncated {
        original_tokens: usize,
        truncated_tokens: usize,
        tool_name: String,
    },
    /// Context window exceeded (critical).
    ContextExceeded {
        total_tokens: usize,
        max_tokens: usize,
    },
}

/// Summary of current context state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSummary {
    pub total_tokens: usize,
    pub max_tokens: usize,
    pub available_tokens: usize,
    pub utilization: f64,
    pub alert_level: TokenAlertLevel,
    pub system_prompt_tokens: usize,
    pub user_messages_tokens: usize,
    pub assistant_messages_tokens: usize,
    pub tool_results_tokens: usize,
    pub warning_threshold: f64,
    pub alert_threshold: f64,
}

/// Information about a context warning threshold being exceeded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextWarningInfo {
    /// Current utilization ratio (0.0–1.0).
    pub utilization: f64,
    /// Total tokens currently in use.
    pub total_tokens: usize,
    /// Maximum tokens available.
    pub max_tokens: usize,
}

/// Result of enforcing context window limits.
///
/// Contains the messages and information about any warnings that occurred.
/// The caller can use this to emit appropriate `AiEvent` types to the
/// frontend. Note: pruning has been replaced by compaction via the
/// summarizer agent.
#[derive(Debug, Clone)]
pub struct ContextEnforcementResult {
    /// The messages (unchanged — pruning is no longer performed).
    pub messages: Vec<Message>,
    /// Warning info if utilization exceeded warning threshold.
    pub warning_info: Option<ContextWarningInfo>,
}
