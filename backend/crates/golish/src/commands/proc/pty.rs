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
    tracing::info!(
        "[active-terminal] Frontend reports active session: {}",
        session_id
    );
    let mut active = state.active_session.lock();
    *active = Some(session_id);
    Ok(())
}

/// Frontend → backend nudge for the Phase B GridTerminal. The grid is
/// a fire-and-forget event stream (`terminal_grid_update`) so the
/// frontend doesn't normally need to ask for anything, but on
/// reconnect or when it detects a non-contiguous `rev` it can call
/// this to receive a full baseline snapshot.
///
/// Returns the latest [`golish_pty::GridUpdate`] (always `full = true`)
/// if the session is currently rendering through a GridTerminal, or
/// `None` if no grid is allocated (i.e. the session is not on
/// alt-screen).
#[tauri::command]
pub async fn pty_request_grid_snapshot(
    state: State<'_, PtyState>,
    session_id: String,
) -> Result<Option<golish_pty::GridUpdate>> {
    let Some(grid) = state.manager.grid_terminal(&session_id) else {
        return Ok(None);
    };
    // Hoist the snapshot out before returning so the `MutexGuard` is
    // dropped at the end of this statement instead of being held
    // across the final `Ok(...)` expression (rustc complains about the
    // borrow lifetime otherwise).
    let snapshot = grid.lock().snapshot_full();
    Ok(Some(snapshot))
}

/// Frontend → backend grid resize. Mirrors `pty_resize` but targets the
/// GridTerminal layer rather than the underlying PTY (which keeps its
/// own dimensions to drive the shell). No-ops when no grid is allocated
/// for the session.
#[tauri::command]
pub async fn pty_resize_grid(
    state: State<'_, PtyState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<()> {
    state.manager.resize_grid(&session_id, cols, rows);
    Ok(())
}
