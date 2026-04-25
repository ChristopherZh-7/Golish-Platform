//! TokenUsage DTO + constants.

use serde::{Deserialize, Serialize};


/// Token usage for a single completion request
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl TokenUsage {
    /// Create a new TokenUsage with specified input and output tokens
    pub fn new(input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            input_tokens,
            output_tokens,
        }
    }

    /// Calculate total tokens (input + output)
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// Maximum tokens allowed for tool responses before truncation
pub const MAX_TOOL_RESPONSE_TOKENS: usize = 25_000;

/// Default context window size (Claude 3.5 Sonnet)
pub const DEFAULT_MAX_CONTEXT_TOKENS: usize = 128_000;
