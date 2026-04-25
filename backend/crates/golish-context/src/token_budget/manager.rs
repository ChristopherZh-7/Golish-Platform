//! TokenBudgetManager: runtime usage tracking and alert thresholds.

use std::sync::Arc;
use tokio::sync::RwLock;

use super::config::TokenBudgetConfig;
use super::stats::{TokenAlertLevel, TokenUsageStats};


/// Manages token budget for a conversation
#[derive(Debug)]
pub struct TokenBudgetManager {
    config: TokenBudgetConfig,
    stats: Arc<RwLock<TokenUsageStats>>,
}

impl TokenBudgetManager {
    /// Create a new token budget manager
    pub fn new(config: TokenBudgetConfig) -> Self {
        Self {
            config,
            stats: Arc::new(RwLock::new(TokenUsageStats::new())),
        }
    }

    /// Create with default config
    pub fn default_for_model(model: &str) -> Self {
        Self::new(TokenBudgetConfig::for_model(model))
    }

    /// Get the current configuration
    pub fn config(&self) -> &TokenBudgetConfig {
        &self.config
    }

    /// Update configuration
    pub fn set_config(&mut self, config: TokenBudgetConfig) {
        self.config = config;
    }

    /// Get current token usage stats
    pub async fn stats(&self) -> TokenUsageStats {
        self.stats.read().await.clone()
    }

    /// Reset token usage stats
    pub async fn reset(&self) {
        let mut stats = self.stats.write().await;
        stats.reset();
    }

    /// Estimate tokens for text content using tokenx-rs heuristic estimator.
    ///
    /// Uses segment-based analysis (96% accuracy vs tiktoken cl100k_base)
    /// instead of the naive chars/4 approximation.
    pub fn estimate_tokens(text: &str) -> usize {
        tokenx_rs::estimate_token_count(text)
    }

    /// Calculate usage percentage (0.0 - 1.0+)
    pub async fn usage_percentage(&self) -> f64 {
        let stats = self.stats.read().await;
        stats.total_tokens as f64 / self.config.max_context_tokens as f64
    }

    /// Check if usage exceeds warning threshold
    pub async fn exceeds_warning(&self) -> bool {
        self.usage_percentage().await > self.config.warning_threshold
    }

    /// Check if usage exceeds alert threshold
    pub async fn exceeds_alert(&self) -> bool {
        self.usage_percentage().await > self.config.alert_threshold
    }

    /// Get current alert level
    pub async fn alert_level(&self) -> TokenAlertLevel {
        let usage = self.usage_percentage().await;
        if usage >= 1.0 {
            TokenAlertLevel::Critical
        } else if usage > self.config.alert_threshold {
            TokenAlertLevel::Alert
        } else if usage > self.config.warning_threshold {
            TokenAlertLevel::Warning
        } else {
            TokenAlertLevel::Normal
        }
    }

    /// Calculate remaining available tokens
    pub async fn remaining_tokens(&self) -> usize {
        let stats = self.stats.read().await;
        self.config
            .available_tokens()
            .saturating_sub(stats.total_tokens)
    }

    /// Update system prompt tokens
    pub async fn set_system_prompt_tokens(&self, tokens: usize) {
        let mut stats = self.stats.write().await;
        stats.system_prompt_tokens = tokens;
        self.update_total(&mut stats);
    }

    /// Add tokens for a user message
    pub async fn add_user_message(&self, tokens: usize) {
        let mut stats = self.stats.write().await;
        stats.user_messages_tokens += tokens;
        self.update_total(&mut stats);
    }

    /// Add tokens for an assistant message
    pub async fn add_assistant_message(&self, tokens: usize) {
        let mut stats = self.stats.write().await;
        stats.assistant_messages_tokens += tokens;
        self.update_total(&mut stats);
    }

    /// Add tokens for a tool result
    pub async fn add_tool_result(&self, tokens: usize) {
        let mut stats = self.stats.write().await;
        stats.tool_results_tokens += tokens;
        self.update_total(&mut stats);
    }

    /// Update total token count
    fn update_total(&self, stats: &mut TokenUsageStats) {
        stats.total_tokens = stats.system_prompt_tokens
            + stats.user_messages_tokens
            + stats.assistant_messages_tokens
            + stats.tool_results_tokens
            + stats.decision_ledger_tokens;
        stats.timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }

    /// Check if adding tokens would exceed budget
    pub async fn would_exceed_budget(&self, additional_tokens: usize) -> bool {
        let stats = self.stats.read().await;
        stats.total_tokens + additional_tokens > self.config.available_tokens()
    }

    /// Calculate how many tokens need to be pruned to fit new content
    pub async fn tokens_to_prune(&self, new_tokens: usize) -> usize {
        let stats = self.stats.read().await;
        let needed = stats.total_tokens + new_tokens;
        let available = self.config.available_tokens();
        needed.saturating_sub(available)
    }

    /// Set stats directly (useful for initialization from message history)
    pub async fn set_stats(&self, new_stats: TokenUsageStats) {
        let mut stats = self.stats.write().await;
        *stats = new_stats;
    }
}

