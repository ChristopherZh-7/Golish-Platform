//! TokenUsageStats + TokenAlertLevel.

use serde::{Deserialize, Serialize};



/// Statistics tracking token usage across different components
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsageStats {
    /// Total tokens currently in context
    pub total_tokens: usize,
    /// Tokens used by system prompt
    pub system_prompt_tokens: usize,
    /// Tokens used by user messages
    pub user_messages_tokens: usize,
    /// Tokens used by assistant messages
    pub assistant_messages_tokens: usize,
    /// Tokens used by tool results
    pub tool_results_tokens: usize,
    /// Tokens used by decision ledger/history
    pub decision_ledger_tokens: usize,
    /// Unix timestamp of last update
    pub timestamp: u64,
}

impl TokenUsageStats {
    /// Create new stats with current timestamp
    pub fn new() -> Self {
        Self {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            ..Default::default()
        }
    }

    /// Reset all counters
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Calculate total excluding system prompt
    pub fn conversation_tokens(&self) -> usize {
        self.user_messages_tokens
            + self.assistant_messages_tokens
            + self.tool_results_tokens
            + self.decision_ledger_tokens
    }
}

/// Alert level for token usage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenAlertLevel {
    /// Below warning threshold
    Normal,
    /// Above warning threshold but below alert
    Warning,
    /// Above alert threshold
    Alert,
    /// Context window exceeded
    Critical,
}
