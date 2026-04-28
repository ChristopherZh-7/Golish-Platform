// Context and token management commands.

use tauri::State;

use super::ai_session_not_initialized_error;
use crate::state::AppState;
use golish_context::token_budget::{TokenAlertLevel, TokenUsageStats};
use golish_context::{ContextSummary, ContextTrimConfig};

/// Get the current context summary including token usage and alert level.
#[tauri::command]
pub async fn get_context_summary(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<ContextSummary, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.get_context_summary().await);
    }
    let guard = state.ai_state.get_bridge().await?;
    Ok(guard.as_ref().unwrap().get_context_summary().await)
}

/// Get detailed token usage statistics.
#[tauri::command]
pub async fn get_token_usage_stats(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<TokenUsageStats, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.get_token_usage_stats().await);
    }
    let guard = state.ai_state.get_bridge().await?;
    Ok(guard.as_ref().unwrap().get_token_usage_stats().await)
}

/// Get the current token alert level.
#[tauri::command]
pub async fn get_token_alert_level(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<TokenAlertLevel, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.get_token_alert_level().await);
    }
    let guard = state.ai_state.get_bridge().await?;
    Ok(guard.as_ref().unwrap().get_token_alert_level().await)
}

/// Get the context utilization percentage (0.0 - 1.0+).
#[tauri::command]
pub async fn get_context_utilization(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<f64, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.get_context_utilization().await);
    }
    let guard = state.ai_state.get_bridge().await?;
    Ok(guard.as_ref().unwrap().get_context_utilization().await)
}

/// Get remaining available tokens in the context window.
#[tauri::command]
pub async fn get_remaining_tokens(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<usize, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.get_remaining_tokens().await);
    }
    let guard = state.ai_state.get_bridge().await?;
    Ok(guard.as_ref().unwrap().get_remaining_tokens().await)
}

/// Reset the context manager (clear all token tracking).
#[tauri::command]
pub async fn reset_context_manager(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        bridge.reset_context_manager().await;
        return Ok(());
    }
    let guard = state.ai_state.get_bridge().await?;
    guard.as_ref().unwrap().reset_context_manager().await;
    Ok(())
}

/// Get the context trim configuration.
#[tauri::command]
pub async fn get_context_trim_config(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<ContextTrimConfig, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.get_context_trim_config());
    }
    state.ai_state.with_bridge(|b| b.get_context_trim_config()).await
}

/// Check if context management is enabled.
#[tauri::command]
pub async fn is_context_management_enabled(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<bool, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.is_context_management_enabled());
    }
    state.ai_state.with_bridge(|b| b.is_context_management_enabled()).await
}

/// Retry context compaction for a specific session.
#[tauri::command]
pub async fn retry_compaction(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;

    bridge.retry_compaction().await
}
