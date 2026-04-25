//! Session lifecycle commands: start / end / current / resume.

use crate::state::AppState;
use tauri::State;

use super::super::session::SessionMeta;

/// Start a new session
#[tauri::command]
pub async fn sidecar_start_session(
    state: State<'_, AppState>,
    initial_request: String,
) -> Result<String, String> {
    state
        .sidecar_state
        .start_session(&initial_request)
        .map_err(|e| e.to_string())
}

/// End the current session
#[tauri::command]
pub async fn sidecar_end_session(
    state: State<'_, AppState>,
) -> Result<Option<SessionMeta>, String> {
    state.sidecar_state.end_session().map_err(|e| e.to_string())
}

/// Get the current session ID
#[tauri::command]
pub async fn sidecar_current_session(state: State<'_, AppState>) -> Result<Option<String>, String> {
    Ok(state.sidecar_state.current_session_id())
}

/// Resume a previous sidecar session by session ID
///
/// This reactivates an existing session, preserving all context (state.md, log.md, patches, artifacts).
/// Updates the session status to "Active" and sets it as the current session.
#[tauri::command]
pub async fn sidecar_resume_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SessionMeta, String> {
    state
        .sidecar_state
        .resume_session(&session_id)
        .map_err(|e| e.to_string())
}
