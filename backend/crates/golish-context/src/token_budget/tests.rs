//! Token-budget tests.

use super::*;
use super::*;

#[tokio::test]
async fn test_token_estimation() {
    let text = "Hello, world!"; // 13 chars ≈ 4 tokens
    let estimate = TokenBudgetManager::estimate_tokens(text);
    assert!((3..=5).contains(&estimate));
}

#[tokio::test]
async fn test_usage_tracking() {
    let manager = TokenBudgetManager::new(TokenBudgetConfig {
        max_context_tokens: 1000,
        ..Default::default()
    });

    manager.add_user_message(100).await;
    manager.add_assistant_message(200).await;
    manager.add_tool_result(50).await;

    let stats = manager.stats().await;
    assert_eq!(stats.total_tokens, 350);
    assert_eq!(stats.user_messages_tokens, 100);
    assert_eq!(stats.assistant_messages_tokens, 200);
    assert_eq!(stats.tool_results_tokens, 50);
}

#[tokio::test]
async fn test_alert_levels() {
    let manager = TokenBudgetManager::new(TokenBudgetConfig {
        max_context_tokens: 1000,
        warning_threshold: 0.5,
        alert_threshold: 0.8,
        reserved_system_tokens: 0,
        reserved_response_tokens: 0,
        ..Default::default()
    });

    // Normal
    manager.add_user_message(400).await;
    assert_eq!(manager.alert_level().await, TokenAlertLevel::Normal);

    // Warning
    manager.add_user_message(200).await;
    assert_eq!(manager.alert_level().await, TokenAlertLevel::Warning);

    // Alert
    manager.add_user_message(300).await;
    assert_eq!(manager.alert_level().await, TokenAlertLevel::Alert);

    // Critical
    manager.add_user_message(200).await;
    assert_eq!(manager.alert_level().await, TokenAlertLevel::Critical);
}

#[tokio::test]
async fn test_model_config() {
    let config = TokenBudgetConfig::for_model("claude-3-5-sonnet");
    assert_eq!(config.max_context_tokens, 200_000);

    let config = TokenBudgetConfig::for_model("unknown-model");
    assert_eq!(config.max_context_tokens, DEFAULT_MAX_CONTEXT_TOKENS);
}

#[test]
fn test_model_context_limits_claude_4_5() {
    // Claude 4.5 Opus
    let config = TokenBudgetConfig::for_model("claude-opus-4-5-20251101");
    assert_eq!(config.max_context_tokens, 200_000);

    let config = TokenBudgetConfig::for_model("claude-4-5-opus");
    assert_eq!(config.max_context_tokens, 200_000);

    let config = TokenBudgetConfig::for_model("claude-4.5-opus");
    assert_eq!(config.max_context_tokens, 200_000);

    let config = TokenBudgetConfig::for_model("claude-opus-4.5");
    assert_eq!(config.max_context_tokens, 200_000);

    // Claude 4.5 Sonnet
    let config = TokenBudgetConfig::for_model("claude-sonnet-4-5-20251101");
    assert_eq!(config.max_context_tokens, 200_000);

    let config = TokenBudgetConfig::for_model("claude-4-5-sonnet");
    assert_eq!(config.max_context_tokens, 200_000);

    let config = TokenBudgetConfig::for_model("claude-4.5-sonnet");
    assert_eq!(config.max_context_tokens, 200_000);

    let config = TokenBudgetConfig::for_model("claude-sonnet-4.5");
    assert_eq!(config.max_context_tokens, 200_000);

    // Claude 4.5 Haiku
    let config = TokenBudgetConfig::for_model("claude-haiku-4-5-20251101");
    assert_eq!(config.max_context_tokens, 200_000);

    let config = TokenBudgetConfig::for_model("claude-4-5-haiku");
    assert_eq!(config.max_context_tokens, 200_000);

    let config = TokenBudgetConfig::for_model("claude-4.5-haiku");
    assert_eq!(config.max_context_tokens, 200_000);

    let config = TokenBudgetConfig::for_model("claude-haiku-4.5");
    assert_eq!(config.max_context_tokens, 200_000);
}

#[test]
fn test_model_context_limits_gpt_5() {
    // GPT-5.1 (400k context)
    let config = TokenBudgetConfig::for_model("gpt-5.1");
    assert_eq!(config.max_context_tokens, 400_000);

    let config = TokenBudgetConfig::for_model("gpt-5-1");
    assert_eq!(config.max_context_tokens, 400_000);

    let config = TokenBudgetConfig::for_model("gpt-5.1-preview");
    assert_eq!(config.max_context_tokens, 400_000);

    // GPT-5.2 (400k context)
    let config = TokenBudgetConfig::for_model("gpt-5.2");
    assert_eq!(config.max_context_tokens, 400_000);

    let config = TokenBudgetConfig::for_model("gpt-5-2");
    assert_eq!(config.max_context_tokens, 400_000);

    let config = TokenBudgetConfig::for_model("gpt-5.2-preview");
    assert_eq!(config.max_context_tokens, 400_000);
}

// ==================== TokenUsage Tests ====================

#[test]
fn test_token_usage_new() {
    let usage = TokenUsage::new(1000, 500);
    assert_eq!(usage.input_tokens, 1000);
    assert_eq!(usage.output_tokens, 500);
}

#[test]
fn test_token_usage_total() {
    let usage = TokenUsage::new(1500, 300);
    assert_eq!(usage.total(), 1800);
}

#[test]
fn test_token_usage_default() {
    let usage = TokenUsage::default();
    assert_eq!(usage.input_tokens, 0);
    assert_eq!(usage.output_tokens, 0);
    assert_eq!(usage.total(), 0);
}

#[test]
fn test_token_usage_accumulation() {
    // Simulates accumulating tokens across multiple LLM calls in a single agent turn
    let mut total = TokenUsage::default();

    // First LLM call
    let call1 = TokenUsage::new(5000, 200);
    total.input_tokens += call1.input_tokens;
    total.output_tokens += call1.output_tokens;

    // Second LLM call (after tool execution)
    let call2 = TokenUsage::new(5500, 150);
    total.input_tokens += call2.input_tokens;
    total.output_tokens += call2.output_tokens;

    // Third LLM call
    let call3 = TokenUsage::new(6000, 300);
    total.input_tokens += call3.input_tokens;
    total.output_tokens += call3.output_tokens;

    assert_eq!(total.input_tokens, 16500);
    assert_eq!(total.output_tokens, 650);
    assert_eq!(total.total(), 17150);
}

#[test]
fn test_token_usage_serialization() {
    let usage = TokenUsage::new(12345, 6789);
    let json = serde_json::to_string(&usage).unwrap();
    assert!(json.contains("\"input_tokens\":12345"));
    assert!(json.contains("\"output_tokens\":6789"));

    // Deserialize back
    let parsed: TokenUsage = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.input_tokens, 12345);
    assert_eq!(parsed.output_tokens, 6789);
}

#[test]
fn test_token_usage_large_values() {
    // Test with realistic large token counts (200k context window)
    let usage = TokenUsage::new(180_000, 4_000);
    assert_eq!(usage.total(), 184_000);
}

// ==================== Model Context Limits Tests ====================

#[test]
fn test_model_context_limits_gpt() {
    // GPT-4o
    let config = TokenBudgetConfig::for_model("gpt-4o");
    assert_eq!(config.max_context_tokens, 128_000);

    let config = TokenBudgetConfig::for_model("gpt-4o-2024-08-06");
    assert_eq!(config.max_context_tokens, 128_000);

    // GPT-4 Turbo
    let config = TokenBudgetConfig::for_model("gpt-4-turbo");
    assert_eq!(config.max_context_tokens, 128_000);

    let config = TokenBudgetConfig::for_model("gpt-4-turbo-preview");
    assert_eq!(config.max_context_tokens, 128_000);

    // GPT-4.1 (1M context)
    let config = TokenBudgetConfig::for_model("gpt-4.1");
    assert_eq!(config.max_context_tokens, 1_047_576);

    let config = TokenBudgetConfig::for_model("gpt-4-1");
    assert_eq!(config.max_context_tokens, 1_047_576);

    let config = TokenBudgetConfig::for_model("gpt-4.1-preview");
    assert_eq!(config.max_context_tokens, 1_047_576);
}

#[test]
fn test_model_context_limits_gemini() {
    // Gemini Pro
    let config = TokenBudgetConfig::for_model("gemini-pro");
    assert_eq!(config.max_context_tokens, 1_000_000);

    let config = TokenBudgetConfig::for_model("gemini-1.5-pro");
    assert_eq!(config.max_context_tokens, 1_000_000);

    let config = TokenBudgetConfig::for_model("gemini-2.0-pro");
    assert_eq!(config.max_context_tokens, 1_000_000);

    // Gemini Flash
    let config = TokenBudgetConfig::for_model("gemini-flash");
    assert_eq!(config.max_context_tokens, 1_000_000);

    let config = TokenBudgetConfig::for_model("gemini-1.5-flash");
    assert_eq!(config.max_context_tokens, 1_000_000);

    let config = TokenBudgetConfig::for_model("gemini-2.0-flash");
    assert_eq!(config.max_context_tokens, 1_000_000);

    // Generic Gemini defaults to Pro
    let config = TokenBudgetConfig::for_model("gemini");
    assert_eq!(config.max_context_tokens, 1_000_000);

    let config = TokenBudgetConfig::for_model("gemini-1.5");
    assert_eq!(config.max_context_tokens, 1_000_000);
}

#[test]
fn test_model_context_limits_o_series() {
    // o1 model
    let config = TokenBudgetConfig::for_model("o1");
    assert_eq!(config.max_context_tokens, 200_000);

    let config = TokenBudgetConfig::for_model("o1-preview");
    assert_eq!(config.max_context_tokens, 200_000);

    let config = TokenBudgetConfig::for_model("o1-mini");
    assert_eq!(config.max_context_tokens, 200_000);

    // o3 model
    let config = TokenBudgetConfig::for_model("o3");
    assert_eq!(config.max_context_tokens, 200_000);

    let config = TokenBudgetConfig::for_model("o3-mini");
    assert_eq!(config.max_context_tokens, 200_000);
}

#[test]
fn test_model_context_limits_codex() {
    // Codex models (192k context)
    let config = TokenBudgetConfig::for_model("gpt-5.2-codex");
    assert_eq!(config.max_context_tokens, 192_000);

    let config = TokenBudgetConfig::for_model("gpt-5.1-codex");
    assert_eq!(config.max_context_tokens, 192_000);

    let config = TokenBudgetConfig::for_model("gpt-5.1-codex-max");
    assert_eq!(config.max_context_tokens, 192_000);

    let config = TokenBudgetConfig::for_model("gpt-5.1-codex-mini");
    assert_eq!(config.max_context_tokens, 192_000);

    let config = TokenBudgetConfig::for_model("codex-1");
    assert_eq!(config.max_context_tokens, 192_000);
}
