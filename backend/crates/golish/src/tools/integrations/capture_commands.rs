//! Tauri IPC commands for the credential capture engine.
//!
//! Three commands form the UI ↔ engine contract:
//!
//! | Command                          | UI button                          |
//! |----------------------------------|------------------------------------|
//! | [`integrations_capture_start`]   | ⚡ "Auto Capture" → confirm → start |
//! | [`integrations_capture_status`]  | (fallback poll for reconnect)       |
//! | [`integrations_capture_cancel`]  | Cancel button on the status toast   |
//!
//! All three accept a single `args:` object payload so the wire format
//! stays uniform with `integrations_set` / `integrations_clear` and
//! the frontend invoke wrappers can pass `{ args: { ... } }` to match
//! Tauri 2's serde defaults.
//!
//! Error mapping: every error path goes through
//! [`super::state::map_err`] (which is `pub` exported here as
//! [`map_err`]) so the 8 capture-specific `IntegrationError` variants
//! land in the right `GolishError` bucket (`Validation` / `NotFound` /
//! `Internal`). The frontend's `mapErr()` then dispatches off the
//! `[CAPTURE_*]` / `[WEBVIEW_*]` prefix preserved in the Display
//! string.

use std::sync::Arc;

use golish_integrations::error::IntegrationError;
use golish_integrations::types::{CaptureSessionInfo, CaptureState};
use golish_integrations::SchemaResolver;
use serde::Deserialize;

use crate::error::GolishError;

use super::capture::CaptureEngine;
use super::state::{map_err, IntegrationsState};

#[derive(Debug, Deserialize)]
pub struct CaptureStartArgs {
    pub tool_id: String,
    pub group_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CaptureSessionArgs {
    pub session_id: String,
}

/// Open a credential-capture session.
///
/// Resolves the schema → finds the group → reads its `capture` recipe;
/// rejects with `CAPTURE_NO_RECIPE` when the group has no recipe set.
/// On success, registers the session in the engine, builds the
/// isolated Tauri webview, and returns the initial
/// [`CaptureSessionInfo`] (the frontend uses `expires_at` for the
/// countdown without needing to poll).
///
/// IDOR note: there is no per-user resource here — credentials live
/// in the local vault keyed by `(tool_id, group_id)`. Anyone with
/// Golish desktop access can already invoke the existing
/// `integrations_set` / `_get` commands; capture has the same
/// privilege boundary by design.
#[tauri::command]
pub async fn integrations_capture_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, IntegrationsState>,
    engine: tauri::State<'_, Arc<CaptureEngine>>,
    args: CaptureStartArgs,
) -> Result<CaptureSessionInfo, GolishError> {
    let CaptureStartArgs { tool_id, group_id } = args;

    // 1. Resolve schema → group → recipe.
    let resolved = state.resolver().get(&tool_id).await.map_err(map_err)?;
    let group = resolved
        .schema
        .groups
        .iter()
        .find(|g| g.id == group_id)
        .ok_or_else(|| {
            map_err(IntegrationError::SchemaNotFound(format!(
                "{tool_id}/{group_id}"
            )))
        })?;
    let recipe = group
        .capture
        .clone()
        .ok_or_else(|| map_err(IntegrationError::CaptureNoRecipe))?;

    // 2. Register the session (rejects duplicates here, not at
    //    webview-creation time).
    let handle = engine
        .register(tool_id.clone(), group_id.clone(), recipe)
        .await
        .map_err(map_err)?;

    // 3. Build the isolated webview. Failure → roll the session into
    //    Failed (emitted via `transition_and_emit`) so the UI doesn't
    //    leave a "WaitingLogin" session orphaned in the registry.
    if let Err(e) = engine.start_webview(&app, &handle).await {
        let sid = handle.session_id.clone();
        let reason = e.to_string();
        let app_clone = app.clone();
        let engine_clone = engine.inner().clone();
        // Fire-and-forget the rollback; we don't want to mask the
        // root error by awaiting it.
        tauri::async_runtime::spawn(async move {
            if let Err(roll_err) = engine_clone
                .transition_and_emit(
                    &app_clone,
                    &sid,
                    CaptureState::Failed,
                    Some(format!("[WEBVIEW_CREATE_FAILED] {reason}")),
                )
                .await
            {
                tracing::error!(
                    session_id = %sid,
                    error = %roll_err,
                    "capture: failed to roll back session after webview-create failure"
                );
            }
        });
        return Err(map_err(e));
    }

    // 4. Return the initial snapshot.
    let s = handle.inner.read().await;
    Ok(s.info())
}

/// Poll one session's state. Prefer subscribing to the
/// `"integration-capture"` Tauri event for push-based updates; this
/// is a fallback for reconnect / page-refresh scenarios.
///
/// Returns `[CAPTURE_SESSION_NOT_FOUND]` (mapped to `NotFound` →
/// HTTP-404-style) when the id was already GC'd or never existed.
#[tauri::command]
pub async fn integrations_capture_status(
    engine: tauri::State<'_, Arc<CaptureEngine>>,
    args: CaptureSessionArgs,
) -> Result<CaptureSessionInfo, GolishError> {
    let handle = engine.get(&args.session_id).await.map_err(map_err)?;
    let s = handle.inner.read().await;
    Ok(s.info())
}

/// Cancel an in-flight session.
///
/// Idempotent: cancelling an already-terminal session is a no-op
/// (returns `Ok(())`) — the engine's `transition_and_emit` ignores
/// transitions after a terminal state, see
/// `CaptureEngine::transition`.
///
/// Also closes the lingering webview window if one is still open.
#[tauri::command]
pub async fn integrations_capture_cancel(
    app: tauri::AppHandle,
    engine: tauri::State<'_, Arc<CaptureEngine>>,
    args: CaptureSessionArgs,
) -> Result<(), GolishError> {
    use tauri::Manager;
    engine
        .transition_and_emit(&app, &args.session_id, CaptureState::Cancelled, None)
        .await
        .map_err(map_err)?;
    // Best-effort: close the capture window if the user didn't close
    // it themselves. Window may already be gone — that's fine.
    let label = format!("capture-{}", args.session_id);
    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.close();
    }
    Ok(())
}
