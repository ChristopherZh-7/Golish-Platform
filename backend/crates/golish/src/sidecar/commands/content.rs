//! Read-only commands exposing per-session content (state.md, log.md, metadata).

use crate::error::GolishError;
use crate::state::SidecarManaged;
use tauri::State;

use super::super::session::{Session, SessionMeta};

/// Get the state.md content for a session (body only)
#[tauri::command]
pub async fn sidecar_get_session_state(
    state: State<'_, SidecarManaged>,
    session_id: String,
) -> Result<String, GolishError> {
    state
        .sidecar_state
        .get_session_state(&session_id)
        .await
        .map_err(GolishError::from)
}

/// Get the injectable context for the current session
#[tauri::command]
pub async fn sidecar_get_injectable_context(
    state: State<'_, SidecarManaged>,
) -> Result<Option<String>, GolishError> {
    state
        .sidecar_state
        .get_injectable_context()
        .await
        .map_err(GolishError::from)
}

/// Get the metadata for a session
#[tauri::command]
pub async fn sidecar_get_session_meta(
    state: State<'_, SidecarManaged>,
    session_id: String,
) -> Result<SessionMeta, GolishError> {
    state
        .sidecar_state
        .get_session_meta(&session_id)
        .await
        .map_err(GolishError::from)
}

/// List all sessions
#[tauri::command]
pub async fn sidecar_list_sessions(state: State<'_, SidecarManaged>) -> Result<Vec<SessionMeta>, GolishError> {
    state
        .sidecar_state
        .list_sessions()
        .await
        .map_err(GolishError::from)
}

/// Get the session log (append-only event log)
#[tauri::command]
pub async fn sidecar_get_session_log(
    state: State<'_, SidecarManaged>,
    session_id: String,
) -> Result<String, GolishError> {
    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
?;

    session.read_log().await.map_err(GolishError::from)
}
