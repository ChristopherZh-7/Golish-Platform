//! TokenBudgetConfig + per-model factory + Default.

use serde::{Deserialize, Serialize};

use super::limits::ModelContextLimits;
use super::usage::DEFAULT_MAX_CONTEXT_TOKENS;


/// Configuration for token budget management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBudgetConfig {
    /// Maximum tokens allowed in context window
    pub max_context_tokens: usize,
    /// Threshold (0.0-1.0) at which to warn about token usage
    pub warning_threshold: f64,
    /// Threshold (0.0-1.0) at which to alert about token usage
    pub alert_threshold: f64,
    /// Model identifier for context-specific limits
    pub model: String,
    /// Optional custom tokenizer ID
    pub tokenizer_id: Option<String>,
    /// Enable detailed per-component token tracking
    pub detailed_tracking: bool,
    /// Reserved tokens for system prompt (subtracted from available budget)
    pub reserved_system_tokens: usize,
    /// Reserved tokens for assistant response
    pub reserved_response_tokens: usize,
}

impl Default for TokenBudgetConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: DEFAULT_MAX_CONTEXT_TOKENS,
            warning_threshold: 0.75,
            alert_threshold: 0.85,
            model: "claude-3-5-sonnet".to_string(),
            tokenizer_id: None,
            detailed_tracking: false,
            reserved_system_tokens: 4_000,
            reserved_response_tokens: 8_192,
        }
    }
}

impl TokenBudgetConfig {
    /// Create config for a specific model
    pub fn for_model(model: &str) -> Self {
        let limits = ModelContextLimits::default();
        let model_lower = model.to_lowercase();
        let max_context = match model_lower.as_str() {
            // Claude models (check 4.5 before 4 to avoid false matches)
            m if m.contains("claude-3-5-sonnet") => limits.claude_3_5_sonnet,
            m if m.contains("claude-3-opus") => limits.claude_3_opus,
            m if m.contains("claude-3-haiku") => limits.claude_3_haiku,
            m if m.contains("claude-4-5-opus")
                || m.contains("claude-4.5-opus")
                || m.contains("claude-opus-4-5")
                || m.contains("claude-opus-4.5") =>
            {
                limits.claude_4_5_opus
            }
            m if m.contains("claude-4-5-sonnet")
                || m.contains("claude-4.5-sonnet")
                || m.contains("claude-sonnet-4-5")
                || m.contains("claude-sonnet-4.5") =>
            {
                limits.claude_4_5_sonnet
            }
            m if m.contains("claude-4-5-haiku")
                || m.contains("claude-4.5-haiku")
                || m.contains("claude-haiku-4-5")
                || m.contains("claude-haiku-4.5") =>
            {
                limits.claude_4_5_haiku
            }
            m if m.contains("claude-4-6-sonnet")
                || m.contains("claude-4.6-sonnet")
                || m.contains("claude-sonnet-4-6")
                || m.contains("claude-sonnet-4.6") =>
            {
                limits.claude_4_6_sonnet
            }
            m if m.contains("claude-4-sonnet") || m.contains("claude-sonnet-4") => {
                limits.claude_4_sonnet
            }
            m if m.contains("claude-4-6-opus")
                || m.contains("claude-4.6-opus")
                || m.contains("claude-opus-4-6")
                || m.contains("claude-opus-4.6") =>
            {
                limits.claude_4_6_opus
            }
            m if m.contains("claude-4-opus") || m.contains("claude-opus-4") => limits.claude_4_opus,
            // OpenAI Codex models (check before gpt-5 since codex contains gpt-5)
            m if m.contains("codex") => limits.codex,
            // OpenAI GPT-5.x (check before gpt-4 to avoid false matches)
            m if m.contains("gpt-5.2") || m.contains("gpt-5-2") => limits.gpt_5_2,
            m if m.contains("gpt-5.1") || m.contains("gpt-5-1") => limits.gpt_5_1,
            // OpenAI GPT-4.1 (check before gpt-4 to avoid false matches)
            m if m.contains("gpt-4.1") || m.contains("gpt-4-1") => limits.gpt_4_1,
            // OpenAI GPT-4 variants
            m if m.contains("gpt-4o") => limits.gpt_4o,
            m if m.contains("gpt-4-turbo") => limits.gpt_4_turbo,
            // OpenAI o-series reasoning models
            m if m.contains("o3") => limits.o3,
            m if m.contains("o1") => limits.o1,
            // Google Gemini models (check specific variants before generic)
            m if m.contains("gemini") && m.contains("flash") => limits.gemini_flash,
            m if m.contains("gemini") && m.contains("pro") => limits.gemini_pro,
            m if m.contains("gemini") => limits.gemini_pro, // Default Gemini to Pro
            // Default fallback
            _ => DEFAULT_MAX_CONTEXT_TOKENS,
        };

        Self {
            max_context_tokens: max_context,
            model: model.to_string(),
            ..Default::default()
        }
    }

    /// Calculate available tokens after reservations
    pub fn available_tokens(&self) -> usize {
        self.max_context_tokens
            .saturating_sub(self.reserved_system_tokens)
            .saturating_sub(self.reserved_response_tokens)
    }
}
