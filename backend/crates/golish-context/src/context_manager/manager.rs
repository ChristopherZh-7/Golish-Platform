//! [`ContextManager`] — the orchestrator that ties together token budgeting,
//! compaction decisions, tool-output truncation, and threshold-driven events.

use rig::message::Message;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::token_budget::{TokenAlertLevel, TokenBudgetConfig, TokenBudgetManager, TokenUsageStats};
use crate::token_trunc::{aggregate_tool_output, TruncationResult, DEFAULT_MAX_TOOL_RESPONSE_TOKENS};

use super::config::{ContextManagerConfig, ContextTrimConfig};
use super::events::{ContextEfficiency, ContextEvent, ContextSummary};
use super::state::{CompactionCheck, CompactionState};

/// Central manager for context window management.
#[derive(Debug)]
pub struct ContextManager {
    /// Token budget manager.
    pub(super) token_budget: Arc<TokenBudgetManager>,
    /// Trim configuration.
    pub(super) trim_config: ContextTrimConfig,
    /// Whether token budgeting is enabled.
    pub(super) token_budget_enabled: bool,
    /// Last recorded efficiency metrics.
    pub(super) last_efficiency: Arc<RwLock<Option<ContextEfficiency>>>,
    /// Event channel for notifications.
    pub(super) event_tx: Option<tokio::sync::mpsc::Sender<ContextEvent>>,
}

impl ContextManager {
    /// Create a new context manager.
    pub fn new(budget_config: TokenBudgetConfig, trim_config: ContextTrimConfig) -> Self {
        Self {
            token_budget: Arc::new(TokenBudgetManager::new(budget_config)),
            trim_config,
            token_budget_enabled: false, // Disabled by default
            last_efficiency: Arc::new(RwLock::new(None)),
            event_tx: None,
        }
    }

    /// Create with high-level configuration.
    ///
    /// This constructor accepts a [`ContextManagerConfig`] which mirrors
    /// the settings from application configuration (e.g.,
    /// `ContextSettings`). It properly enables both `token_budget_enabled`
    /// and `trim_config.enabled` based on the `config.enabled` setting.
    ///
    /// # Example
    /// ```
    /// use golish_context::context_manager::{ContextManager, ContextManagerConfig};
    ///
    /// let manager = ContextManager::with_config("claude-3-5-sonnet", ContextManagerConfig::default());
    /// assert!(manager.is_enabled());
    /// ```
    pub fn with_config(model: &str, config: ContextManagerConfig) -> Self {
        let budget_config = TokenBudgetConfig::for_model(model);

        // Configure token budget thresholds based on compaction_threshold.
        // Alert threshold sits at compaction_threshold; warning is slightly below.
        let mut budget_config = budget_config;
        budget_config.alert_threshold = config.compaction_threshold;
        budget_config.warning_threshold = (config.compaction_threshold - 0.10).max(0.50);

        let trim_config = ContextTrimConfig {
            enabled: config.enabled,
            target_utilization: config.compaction_threshold - 0.10,
            aggressive_on_critical: true,
            max_tool_response_tokens: DEFAULT_MAX_TOOL_RESPONSE_TOKENS,
        };

        Self {
            token_budget: Arc::new(TokenBudgetManager::new(budget_config)),
            trim_config,
            token_budget_enabled: config.enabled,
            last_efficiency: Arc::new(RwLock::new(None)),
            event_tx: None,
        }
    }

    /// Create with default configuration for a model.
    pub fn for_model(model: &str) -> Self {
        Self::new(
            TokenBudgetConfig::for_model(model),
            ContextTrimConfig::default(),
        )
    }

    /// Create with default configuration for a model, with context
    /// management enabled.
    ///
    /// Equivalent to `with_config(model, ContextManagerConfig::default())`.
    pub fn for_model_enabled(model: &str) -> Self {
        Self::with_config(model, ContextManagerConfig::default())
    }

    /// Set event channel for notifications.
    pub fn set_event_channel(&mut self, tx: tokio::sync::mpsc::Sender<ContextEvent>) {
        self.event_tx = Some(tx);
    }

    /// Get reference to token budget manager.
    pub fn token_budget(&self) -> Arc<TokenBudgetManager> {
        Arc::clone(&self.token_budget)
    }

    /// Get current trim configuration.
    pub fn trim_config(&self) -> &ContextTrimConfig {
        &self.trim_config
    }

    /// Update trim configuration.
    pub fn set_trim_config(&mut self, config: ContextTrimConfig) {
        self.trim_config = config;
    }

    /// Check if token budgeting is enabled.
    pub fn is_enabled(&self) -> bool {
        self.token_budget_enabled
    }

    /// Enable/disable token budgeting.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.token_budget_enabled = enabled;
    }

    /// Get current token usage stats.
    pub async fn stats(&self) -> TokenUsageStats {
        self.token_budget.stats().await
    }

    /// Get current alert level.
    pub async fn alert_level(&self) -> TokenAlertLevel {
        self.token_budget.alert_level().await
    }

    /// Get utilization percentage.
    pub async fn utilization(&self) -> f64 {
        self.token_budget.usage_percentage().await
    }

    /// Get remaining tokens.
    pub async fn remaining_tokens(&self) -> usize {
        self.token_budget.remaining_tokens().await
    }

    /// Get last efficiency metrics.
    pub async fn last_efficiency(&self) -> Option<ContextEfficiency> {
        self.last_efficiency.read().await.clone()
    }

    /// Reset token budget.
    pub async fn reset(&self) {
        self.token_budget.reset().await;
        *self.last_efficiency.write().await = None;
    }

    /// Update budget from message history.
    pub async fn update_from_messages(&self, messages: &[Message]) {
        let mut stats = TokenUsageStats::new();

        for message in messages {
            let chars = estimate_message_chars(message);
            let tokens = chars / 4; // chars/4 heuristic, same as TokenBudgetManager::estimate_tokens
            match message {
                Message::User { content } => {
                    let has_tool_result = content
                        .iter()
                        .any(|c| matches!(c, rig::message::UserContent::ToolResult(_)));
                    if has_tool_result {
                        stats.tool_results_tokens += tokens;
                    } else {
                        stats.user_messages_tokens += tokens;
                    }
                }
                Message::Assistant { .. } => stats.assistant_messages_tokens += tokens,
            }
        }

        stats.total_tokens = stats.system_prompt_tokens
            + stats.user_messages_tokens
            + stats.assistant_messages_tokens
            + stats.tool_results_tokens;

        self.token_budget.set_stats(stats).await;

        // Check thresholds and emit events.
        self.check_and_emit_alerts().await;
    }

    /// Check thresholds and emit alert events.
    async fn check_and_emit_alerts(&self) {
        if let Some(ref tx) = self.event_tx {
            let alert_level = self.token_budget.alert_level().await;
            let stats = self.token_budget.stats().await;
            let utilization = self.token_budget.usage_percentage().await;
            let max_tokens = self.token_budget.config().max_context_tokens;

            let event = match alert_level {
                TokenAlertLevel::Critical => Some(ContextEvent::ContextExceeded {
                    total_tokens: stats.total_tokens,
                    max_tokens,
                }),
                TokenAlertLevel::Alert => Some(ContextEvent::AlertThreshold {
                    utilization,
                    total_tokens: stats.total_tokens,
                    max_tokens,
                }),
                TokenAlertLevel::Warning => Some(ContextEvent::WarningThreshold {
                    utilization,
                    total_tokens: stats.total_tokens,
                    max_tokens,
                }),
                TokenAlertLevel::Normal => None,
            };

            if let Some(event) = event {
                let _ = tx.send(event).await;
            }
        }
    }

    /// Truncate tool response if it exceeds limits.
    pub async fn truncate_tool_response(&self, content: &str, tool_name: &str) -> TruncationResult {
        let result = aggregate_tool_output(content, self.trim_config.max_tool_response_tokens);

        if result.truncated {
            if let Some(ref tx) = self.event_tx {
                let _ = tx
                    .send(ContextEvent::ToolResponseTruncated {
                        original_tokens: TokenBudgetManager::estimate_tokens(content),
                        truncated_tokens: TokenBudgetManager::estimate_tokens(&result.content),
                        tool_name: tool_name.to_string(),
                    })
                    .await;
            }

            tracing::debug!(
                "Tool response '{}' truncated: {} -> {} tokens",
                tool_name,
                TokenBudgetManager::estimate_tokens(content),
                TokenBudgetManager::estimate_tokens(&result.content)
            );
        }

        result
    }

    /// Check if there's room for a new message.
    pub async fn can_add_message(&self, estimated_tokens: usize) -> bool {
        !self
            .token_budget
            .would_exceed_budget(estimated_tokens)
            .await
    }

    /// Get context summary for diagnostics.
    pub async fn get_summary(&self) -> ContextSummary {
        let stats = self.token_budget.stats().await;
        let config = self.token_budget.config();

        ContextSummary {
            total_tokens: stats.total_tokens,
            max_tokens: config.max_context_tokens,
            available_tokens: config.available_tokens(),
            utilization: self.token_budget.usage_percentage().await,
            alert_level: self.token_budget.alert_level().await,
            system_prompt_tokens: stats.system_prompt_tokens,
            user_messages_tokens: stats.user_messages_tokens,
            assistant_messages_tokens: stats.assistant_messages_tokens,
            tool_results_tokens: stats.tool_results_tokens,
            warning_threshold: config.warning_threshold,
            alert_threshold: config.alert_threshold,
        }
    }

    /// Check if compaction should be triggered.
    ///
    /// This should be called between turns, before starting a new agent loop.
    ///
    /// # Arguments
    /// * `compaction_state` — the current compaction state.
    /// * `model` — the model name (for looking up context limits).
    ///
    /// # Returns
    /// A [`CompactionCheck`] with the decision and context.
    pub fn should_compact(
        &self,
        compaction_state: &CompactionState,
        model: &str,
    ) -> CompactionCheck {
        // Already attempted this turn?
        if compaction_state.attempted_this_turn {
            return CompactionCheck {
                should_compact: false,
                current_tokens: compaction_state.last_input_tokens.unwrap_or(0),
                max_tokens: TokenBudgetConfig::for_model(model).max_context_tokens,
                threshold: self.token_budget.config().alert_threshold,
                using_heuristic: compaction_state.using_heuristic,
                reason: "Already attempted this turn".to_string(),
            };
        }

        // Context management disabled?
        if !self.token_budget_enabled {
            return CompactionCheck {
                should_compact: false,
                current_tokens: compaction_state.last_input_tokens.unwrap_or(0),
                max_tokens: TokenBudgetConfig::for_model(model).max_context_tokens,
                threshold: self.token_budget.config().alert_threshold,
                using_heuristic: compaction_state.using_heuristic,
                reason: "Context management disabled".to_string(),
            };
        }

        let current_tokens = compaction_state.last_input_tokens.unwrap_or(0);
        let model_config = TokenBudgetConfig::for_model(model);
        let max_tokens = model_config.max_context_tokens;
        let threshold = self.token_budget.config().alert_threshold;
        let threshold_tokens = (max_tokens as f64 * threshold) as u64;

        let should_compact = current_tokens >= threshold_tokens;

        let reason = if should_compact {
            format!(
                "Token usage {}% ({}/{}) exceeds threshold {}%",
                (current_tokens as f64 / max_tokens as f64 * 100.0) as u32,
                current_tokens,
                max_tokens,
                (threshold * 100.0) as u32
            )
        } else {
            format!(
                "Token usage {}% ({}/{}) below threshold {}%",
                (current_tokens as f64 / max_tokens as f64 * 100.0) as u32,
                current_tokens,
                max_tokens,
                (threshold * 100.0) as u32
            )
        };

        CompactionCheck {
            should_compact,
            current_tokens,
            max_tokens,
            threshold,
            using_heuristic: compaction_state.using_heuristic,
            reason,
        }
    }

    /// Check if context has exceeded the absolute limit (session is dead).
    pub fn is_context_exceeded(&self, compaction_state: &CompactionState, model: &str) -> bool {
        let current_tokens = compaction_state.last_input_tokens.unwrap_or(0);
        let model_config = TokenBudgetConfig::for_model(model);
        let max_context_tokens = model_config.max_context_tokens;

        current_tokens >= max_context_tokens as u64
    }
}

/// Estimate character length of a message without allocating strings.
/// Used for token estimation (chars / 4 heuristic).
pub(super) fn estimate_message_chars(message: &Message) -> usize {
    use rig::completion::AssistantContent;
    use rig::message::UserContent;

    match message {
        Message::User { content } => content
            .iter()
            .map(|c| match c {
                UserContent::Text(t) => t.text.len(),
                UserContent::Image(_) => 7,     // "[image]"
                UserContent::Document(_) => 10, // "[document]"
                UserContent::ToolResult(result) => result
                    .content
                    .iter()
                    .map(|tc| format!("{:?}", tc).len())
                    .sum(),
                _ => 7, // "[media]"
            })
            .sum(),
        Message::Assistant { content, .. } => content
            .iter()
            .map(|c| match c {
                AssistantContent::Text(t) => t.text.len(),
                AssistantContent::ToolCall(call) => 8 + call.function.name.len(),
                _ => 11, // "[reasoning]"
            })
            .sum(),
    }
}
