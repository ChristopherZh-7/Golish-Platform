// Context and token management commands.

use crate::error::GolishError;
use tauri::State;

use super::ai_session_not_initialized_error;
use crate::state::AgentState;
use golish_context::token_budget::{TokenAlertLevel, TokenUsageStats};
use golish_context::{ContextSummary, ContextTrimConfig};

/// Get the current context summary including token usage and alert level.
#[tauri::command]
pub async fn get_context_summary(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<ContextSummary, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.get_context_summary().await)
}

/// Get detailed token usage statistics.
#[tauri::command]
pub async fn get_token_usage_stats(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<TokenUsageStats, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.get_token_usage_stats().await)
}

/// Get the current token alert level.
#[tauri::command]
pub async fn get_token_alert_level(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<TokenAlertLevel, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.get_token_alert_level().await)
}

/// Get the context utilization percentage (0.0 - 1.0+).
#[tauri::command]
pub async fn get_context_utilization(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<f64, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.get_context_utilization().await)
}

/// Get remaining available tokens in the context window.
#[tauri::command]
pub async fn get_remaining_tokens(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<usize, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.get_remaining_tokens().await)
}

/// Reset the context manager (clear all token tracking).
#[tauri::command]
pub async fn reset_context_manager(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    bridge.reset_context_manager().await;
    Ok(())
}

/// Get the context trim configuration.
#[tauri::command]
pub async fn get_context_trim_config(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<ContextTrimConfig, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.get_context_trim_config())
}

/// Check if context management is enabled.
#[tauri::command]
pub async fn is_context_management_enabled(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<bool, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.is_context_management_enabled())
}

/// Retry context compaction for a specific session.
#[tauri::command]
pub async fn retry_compaction(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;

    bridge
        .retry_compaction()
        .await
        .map_err(GolishError::Internal)
}
