use crate::error::Result;
use crate::pty::PtySession;
use crate::runtime::TauriRuntime;
use crate::state::PtyState;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn pty_create(
    state: State<'_, PtyState>,
    app_handle: tauri::AppHandle,
    working_directory: Option<String>,
    rows: Option<u16>,
    cols: Option<u16>,
) -> Result<PtySession> {
    let working_dir = working_directory.map(PathBuf::from);
    let rows = rows.unwrap_or(24);
    let cols = cols.unwrap_or(80);

    let runtime = Arc::new(TauriRuntime::new(app_handle));

    Ok(state
        .manager
        .create_session_with_runtime(runtime, working_dir, rows, cols)?)
}

#[tauri::command]
pub async fn pty_write(state: State<'_, PtyState>, session_id: String, data: String) -> Result<()> {
    Ok(state.manager.write(&session_id, data.as_bytes())?)
}

#[tauri::command]
pub async fn pty_resize(
    state: State<'_, PtyState>,
    session_id: String,
    rows: u16,
    cols: u16,
) -> Result<()> {
    Ok(state.manager.resize(&session_id, rows, cols)?)
}

#[tauri::command]
pub async fn pty_destroy(state: State<'_, PtyState>, session_id: String) -> Result<()> {
    Ok(state.manager.destroy(&session_id)?)
}

#[tauri::command]
pub async fn pty_get_session(state: State<'_, PtyState>, session_id: String) -> Result<PtySession> {
    Ok(state.manager.get_session(&session_id)?)
}

#[tauri::command]
pub async fn pty_get_foreground_process(
    state: State<'_, PtyState>,
    session_id: String,
) -> Result<Option<String>> {
    Ok(state.manager.get_foreground_process(&session_id)?)
}

#[tauri::command]
pub async fn set_active_terminal_session(
    state: State<'_, PtyState>,
    session_id: String,
) -> Result<()> {
    tracing::info!("[active-terminal] Frontend reports active session: {}", session_id);
    let mut active = state.active_session.lock();
    *active = Some(session_id);
    Ok(())
}
