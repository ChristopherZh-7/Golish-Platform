//! Unit tests for the context manager. Combines the original `tests` and
//! `compaction_tests` modules from the monolithic file into one place.

use rig::message::{Message, Text};
use rig::one_or_many::OneOrMany;

use crate::token_budget::{TokenAlertLevel, TokenBudgetConfig, TokenBudgetManager};
use crate::token_trunc::DEFAULT_MAX_TOOL_RESPONSE_TOKENS;

use super::config::{ContextManagerConfig, ContextTrimConfig};
use super::manager::ContextManager;
use super::state::CompactionState;

fn create_user_message(text: &str) -> Message {
    Message::User {
        content: OneOrMany::one(rig::message::UserContent::Text(Text {
            text: text.to_string(),
        })),
    }
}

#[allow(dead_code)]
fn create_assistant_message(text: &str) -> Message {
    Message::Assistant {
        id: None,
        content: OneOrMany::one(rig::message::AssistantContent::Text(Text {
            text: text.to_string(),
        })),
    }
}

// ──────────────── Manager creation / basic API ──────────────────────────

#[tokio::test]
async fn test_context_manager_creation() {
    let manager = ContextManager::for_model("claude-3-5-sonnet");
    // Context management is disabled by default
    assert!(!manager.is_enabled());
    assert_eq!(manager.alert_level().await, TokenAlertLevel::Normal);
}

#[tokio::test]
async fn test_update_from_messages() {
    let manager = ContextManager::for_model("claude-3-5-sonnet");
    let messages = vec![
        create_user_message("Hello, how are you?"),
        create_user_message("I need help with something."),
    ];

    manager.update_from_messages(&messages).await;
    let stats = manager.stats().await;
    assert!(stats.user_messages_tokens > 0);
}

#[tokio::test]
async fn test_tool_response_truncation() {
    let manager = ContextManager::new(
        TokenBudgetConfig::default(),
        ContextTrimConfig {
            max_tool_response_tokens: 10, // Very small for testing
            ..Default::default()
        },
    );

    let long_content = "This is a very long tool response that contains a lot of text. \
        We need to ensure that it exceeds the minimum truncation length of 100 characters. \
        This additional text should push us well over that threshold and trigger actual truncation.";
    let result = manager
        .truncate_tool_response(long_content, "test_tool")
        .await;

    assert!(result.truncated);
    assert!(result.result_chars < long_content.len());
}

#[tokio::test]
async fn test_context_summary() {
    let manager = ContextManager::for_model("claude-3-5-sonnet");
    let summary = manager.get_summary().await;

    assert!(summary.max_tokens > 0);
    assert_eq!(summary.utilization, 0.0);
    assert_eq!(summary.alert_level, TokenAlertLevel::Normal);
}

// ──────────────── ContextManagerConfig ──────────────────────────────────

#[test]
fn test_context_manager_config_default() {
    let config = ContextManagerConfig::default();
    assert!(config.enabled);
    assert!((config.compaction_threshold - 0.80).abs() < f64::EPSILON);
    assert_eq!(config.protected_turns, 2);
    assert_eq!(config.cooldown_seconds, 60);
}

#[tokio::test]
async fn test_with_config_enables_context_management() {
    let config = ContextManagerConfig::default();
    let manager = ContextManager::with_config("claude-3-5-sonnet", config);

    assert!(manager.is_enabled());
    assert!(manager.trim_config().enabled);
}

#[tokio::test]
async fn test_with_config_disabled_results_in_noop() {
    let config = ContextManagerConfig {
        enabled: false,
        ..Default::default()
    };
    let manager = ContextManager::with_config("claude-3-5-sonnet", config);

    assert!(!manager.is_enabled());
    assert!(!manager.trim_config().enabled);
}

#[tokio::test]
async fn test_with_config_sets_thresholds() {
    let config = ContextManagerConfig {
        enabled: true,
        compaction_threshold: 0.75,
        protected_turns: 3,
        cooldown_seconds: 120,
    };
    let manager = ContextManager::with_config("claude-3-5-sonnet", config);

    let summary = manager.get_summary().await;
    assert!((summary.alert_threshold - 0.75).abs() < f64::EPSILON);
    // Warning threshold is 0.10 below compaction_threshold.
    assert!((summary.warning_threshold - 0.65).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_for_model_enabled_enables_context_management() {
    let manager = ContextManager::for_model_enabled("claude-3-5-sonnet");

    assert!(manager.is_enabled());
    assert!(manager.trim_config().enabled);
}

#[tokio::test]
async fn test_for_model_disabled_by_default() {
    let manager = ContextManager::for_model("claude-3-5-sonnet");

    assert!(!manager.is_enabled());
    assert!(!manager.trim_config().enabled);
}

// ──────────────── Compaction decision logic ─────────────────────────────

/// Helper to create a context manager with specific settings for testing.
///
/// Uses internal field access via the `pub(super)` visibility on
/// [`ContextManager`]'s fields. Pure-Rust code only — no public API.
fn create_test_manager(enabled: bool, alert_threshold: f64) -> ContextManager {
    let budget_config = TokenBudgetConfig {
        max_context_tokens: 200_000,
        reserved_system_tokens: 0,
        reserved_response_tokens: 0,
        warning_threshold: alert_threshold - 0.10,
        alert_threshold,
        model: "claude-3-5-sonnet".to_string(),
        tokenizer_id: None,
        detailed_tracking: false,
    };

    let trim_config = ContextTrimConfig {
        enabled,
        target_utilization: alert_threshold - 0.10,
        aggressive_on_critical: true,
        max_tool_response_tokens: DEFAULT_MAX_TOOL_RESPONSE_TOKENS,
    };

    ContextManager {
        token_budget: std::sync::Arc::new(TokenBudgetManager::new(budget_config)),
        trim_config,
        token_budget_enabled: enabled,
        last_efficiency: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        event_tx: None,
    }
}

#[test]
fn test_should_compact_below_threshold() {
    let manager = create_test_manager(true, 0.80);
    let mut state = CompactionState::new();
    state.update_tokens(100_000); // 50%

    let check = manager.should_compact(&state, "claude-3-5-sonnet");

    assert!(!check.should_compact, "50% usage should not trigger compaction");
    assert_eq!(check.current_tokens, 100_000);
    assert_eq!(check.max_tokens, 200_000);
    assert!((check.threshold - 0.80).abs() < f64::EPSILON);
    assert!(check.reason.contains("below threshold"));
}

#[test]
fn test_should_compact_above_threshold() {
    let manager = create_test_manager(true, 0.80);
    let mut state = CompactionState::new();
    state.update_tokens(170_000); // 85%

    let check = manager.should_compact(&state, "claude-3-5-sonnet");

    assert!(check.should_compact, "85% usage should trigger compaction");
    assert_eq!(check.current_tokens, 170_000);
    assert_eq!(check.max_tokens, 200_000);
    assert!((check.threshold - 0.80).abs() < f64::EPSILON);
    assert!(check.reason.contains("exceeds threshold"));
}

#[test]
fn test_should_compact_already_attempted() {
    let manager = create_test_manager(true, 0.80);
    let mut state = CompactionState::new();
    state.update_tokens(170_000);
    state.mark_attempted();

    let check = manager.should_compact(&state, "claude-3-5-sonnet");

    assert!(!check.should_compact, "Should not compact if already attempted");
    assert_eq!(check.reason, "Already attempted this turn");
}

#[test]
fn test_should_compact_disabled() {
    let manager = create_test_manager(false, 0.80);
    let mut state = CompactionState::new();
    state.update_tokens(170_000);

    let check = manager.should_compact(&state, "claude-3-5-sonnet");

    assert!(!check.should_compact, "Should not compact when disabled");
    assert_eq!(check.reason, "Context management disabled");
}

#[test]
fn test_compaction_state_reset_turn() {
    let mut state = CompactionState::new();
    state.update_tokens(150_000);
    state.mark_attempted();
    state.increment_count();

    assert!(state.attempted_this_turn);
    assert_eq!(state.last_input_tokens, Some(150_000));
    assert_eq!(state.compaction_count, 1);

    state.reset_turn();

    assert!(!state.attempted_this_turn);
    assert_eq!(state.last_input_tokens, Some(150_000));
    assert_eq!(state.compaction_count, 1);
}

#[test]
fn test_compaction_state_heuristic() {
    let mut state = CompactionState::new();

    state.update_tokens_heuristic(40_000); // 40k chars / 4 = 10k tokens

    assert_eq!(state.last_input_tokens, Some(10_000));
    assert!(state.using_heuristic);

    state.update_tokens(12_000);

    assert_eq!(state.last_input_tokens, Some(12_000));
    assert!(!state.using_heuristic);
}

#[test]
fn test_is_context_exceeded() {
    let manager = create_test_manager(true, 0.80);

    let mut state = CompactionState::new();
    state.update_tokens(199_999);
    assert!(
        !manager.is_context_exceeded(&state, "claude-3-5-sonnet"),
        "199,999 tokens should not exceed 200,000 limit"
    );

    state.update_tokens(200_000);
    assert!(
        manager.is_context_exceeded(&state, "claude-3-5-sonnet"),
        "200,000 tokens should equal/exceed 200,000 limit"
    );

    state.update_tokens(200_001);
    assert!(
        manager.is_context_exceeded(&state, "claude-3-5-sonnet"),
        "200,001 tokens should exceed 200,000 limit"
    );
}

#[test]
fn test_is_context_exceeded_different_models() {
    let manager = create_test_manager(true, 0.80);

    let mut state = CompactionState::new();
    state.update_tokens(127_999);
    assert!(
        !manager.is_context_exceeded(&state, "gpt-4o"),
        "127,999 should not exceed 128,000"
    );

    state.update_tokens(128_000);
    assert!(
        manager.is_context_exceeded(&state, "gpt-4o"),
        "128,000 should exceed 128,000 limit"
    );

    state.update_tokens(999_999);
    assert!(
        !manager.is_context_exceeded(&state, "gemini-pro"),
        "999,999 should not exceed 1,000,000"
    );

    state.update_tokens(1_000_000);
    assert!(
        manager.is_context_exceeded(&state, "gemini-pro"),
        "1,000,000 should exceed 1,000,000 limit"
    );
}

#[test]
fn test_should_compact_at_exact_threshold() {
    let manager = create_test_manager(true, 0.80);
    let mut state = CompactionState::new();
    state.update_tokens(160_000); // exactly 80%

    let check = manager.should_compact(&state, "claude-3-5-sonnet");

    assert!(
        check.should_compact,
        "Exactly at threshold should trigger compaction"
    );
}

#[test]
fn test_should_compact_with_heuristic() {
    let manager = create_test_manager(true, 0.80);
    let mut state = CompactionState::new();
    state.update_tokens_heuristic(680_000); // 680k / 4 = 170k tokens (85%)

    let check = manager.should_compact(&state, "claude-3-5-sonnet");

    assert!(check.should_compact);
    assert!(check.using_heuristic, "Should indicate heuristic is being used");
}

#[test]
fn test_compaction_state_default() {
    let state = CompactionState::default();

    assert!(!state.attempted_this_turn);
    assert_eq!(state.compaction_count, 0);
    assert!(state.last_input_tokens.is_none());
    assert!(!state.using_heuristic);
}

#[test]
fn test_compaction_check_fields() {
    let manager = create_test_manager(true, 0.75);
    let mut state = CompactionState::new();
    state.update_tokens(150_000); // 75% of 200k

    let check = manager.should_compact(&state, "claude-3-5-sonnet");

    assert!(check.should_compact);
    assert_eq!(check.current_tokens, 150_000);
    assert_eq!(check.max_tokens, 200_000);
    assert!((check.threshold - 0.75).abs() < f64::EPSILON);
    assert!(!check.using_heuristic);
    assert!(!check.reason.is_empty());
}

#[test]
fn test_compaction_state_estimated() {
    let mut state = CompactionState::new();

    state.update_tokens_estimated(150_000);
    assert_eq!(state.last_input_tokens, Some(150_000));
    assert!(state.using_heuristic);

    state.update_tokens(155_000);
    assert_eq!(state.last_input_tokens, Some(155_000));
    assert!(!state.using_heuristic);
}

#[test]
fn test_proactive_estimate_triggers_compaction() {
    let manager = create_test_manager(true, 0.80);
    let mut state = CompactionState::new();

    state.update_tokens(100_000);
    let check = manager.should_compact(&state, "claude-3-5-sonnet");
    assert!(!check.should_compact, "100k/200k (50%) should not trigger compaction");

    state.update_tokens_estimated(180_000);
    let check = manager.should_compact(&state, "claude-3-5-sonnet");
    assert!(
        check.should_compact,
        "180k/200k (90%) via estimate should trigger compaction"
    );
    assert!(
        check.using_heuristic,
        "Should indicate estimate (not provider) is being used"
    );
}

#[test]
fn test_estimated_tokens_below_threshold_no_compaction() {
    let manager = create_test_manager(true, 0.80);
    let mut state = CompactionState::new();

    state.update_tokens_estimated(140_000); // 70%
    let check = manager.should_compact(&state, "claude-3-5-sonnet");
    assert!(!check.should_compact, "140k/200k (70%) should not trigger compaction");
}

#[test]
fn test_multiple_tool_results_accumulation() {
    let manager = create_test_manager(true, 0.80);
    let mut state = CompactionState::new();

    state.update_tokens(80_000);
    assert!(!manager.should_compact(&state, "claude-3-5-sonnet").should_compact);

    state.update_tokens_estimated(100_000);
    assert!(!manager.should_compact(&state, "claude-3-5-sonnet").should_compact);

    state.update_tokens_estimated(130_000);
    assert!(!manager.should_compact(&state, "claude-3-5-sonnet").should_compact);

    state.update_tokens_estimated(165_000);
    assert!(
        manager.should_compact(&state, "claude-3-5-sonnet").should_compact,
        "165k/200k should trigger compaction"
    );
}
