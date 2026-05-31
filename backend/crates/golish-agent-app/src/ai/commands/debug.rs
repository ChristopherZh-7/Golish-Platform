// Debug commands for AI sessions.

use crate::error::GolishError;
use tauri::State;

use crate::ai::commands::ai_session_not_initialized_error;
use crate::state::AgentState;
use golish_core::ApiRequestStatsSnapshot;

/// Get API request statistics for a session.
#[tauri::command]
pub async fn get_api_request_stats(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<ApiRequestStatsSnapshot, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;

    Ok(bridge.get_api_request_stats_snapshot().await)
}
