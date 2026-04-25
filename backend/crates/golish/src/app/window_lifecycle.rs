//! Window state persistence / restoration helpers.
//!
//! These functions were previously nested `async fn` definitions inside
//! `run_gui`. They are lifted here verbatim (behaviour preserved) so the
//! Tauri builder only has to call them.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tauri::{Emitter, Manager};

use crate::state::AppState;
use crate::tools;
use crate::window_state;

pub(crate) async fn persist_window_state_from_window(window: &tauri::Window) {
    let scale_factor = window.scale_factor().unwrap_or(1.0);

    let size = window
        .inner_size()
        .map(|size| size.to_logical::<f64>(scale_factor))
        .unwrap_or_else(|_| tauri::LogicalSize::new(0.0, 0.0));

    let position = window
        .outer_position()
        .ok()
        .map(|p| p.to_logical::<f64>(scale_factor));

    let is_maximized = window.is_maximized().unwrap_or(false);

    let normalized = window_state::normalize_persisted_window_state(
        size.width,
        size.height,
        position.map(|p| p.x),
        position.map(|p| p.y),
        is_maximized,
    );

    static LOGGED: AtomicBool = AtomicBool::new(false);

    let state = window.app_handle().state::<AppState>();
    let mut settings = state.settings_manager.get().await;

    settings.ui.window.width = normalized.width;
    settings.ui.window.height = normalized.height;
    settings.ui.window.x = normalized.x;
    settings.ui.window.y = normalized.y;
    settings.ui.window.maximized = normalized.maximized;

    if !LOGGED.swap(true, Ordering::SeqCst) {
        tracing::debug!(
            settings_path = %state.settings_manager.path().display(),
            width = settings.ui.window.width,
            height = settings.ui.window.height,
            x = ?settings.ui.window.x,
            y = ?settings.ui.window.y,
            maximized = settings.ui.window.maximized,
            "Persisting window state"
        );
    }

    if let Err(e) = state.settings_manager.update(settings).await {
        tracing::debug!(error = %e, "Failed to persist window state");
    }
}

pub(crate) async fn persist_window_state_from_webview_window(window: &tauri::WebviewWindow) {
    let scale_factor = window.scale_factor().unwrap_or(1.0);

    let size = window
        .inner_size()
        .map(|size| size.to_logical::<f64>(scale_factor))
        .unwrap_or_else(|_| tauri::LogicalSize::new(0.0, 0.0));

    let position = window
        .outer_position()
        .ok()
        .map(|p| p.to_logical::<f64>(scale_factor));

    let is_maximized = window.is_maximized().unwrap_or(false);

    let normalized = window_state::normalize_persisted_window_state(
        size.width,
        size.height,
        position.map(|p| p.x),
        position.map(|p| p.y),
        is_maximized,
    );

    static LOGGED: AtomicBool = AtomicBool::new(false);

    let state = window.app_handle().state::<AppState>();
    let mut settings = state.settings_manager.get().await;

    settings.ui.window.width = normalized.width;
    settings.ui.window.height = normalized.height;
    settings.ui.window.x = normalized.x;
    settings.ui.window.y = normalized.y;
    settings.ui.window.maximized = normalized.maximized;

    if !LOGGED.swap(true, Ordering::SeqCst) {
        tracing::debug!(
            settings_path = %state.settings_manager.path().display(),
            width = settings.ui.window.width,
            height = settings.ui.window.height,
            x = ?settings.ui.window.x,
            y = ?settings.ui.window.y,
            maximized = settings.ui.window.maximized,
            "Persisting window state (exit)"
        );
    }

    if let Err(e) = state.settings_manager.update(settings).await {
        tracing::debug!(error = %e, "Failed to persist window state");
    }
}

pub(crate) async fn restore_window_state_on_startup(app_handle: &tauri::AppHandle) {
    let window = app_handle
        .get_webview_window("main")
        .or_else(|| app_handle.webview_windows().values().next().cloned());

    let Some(window) = window else {
        return;
    };

    let state = app_handle.state::<AppState>();
    let settings = state.settings_manager.get().await;
    let ws = settings.ui.window;

    // Clamp to current monitor to avoid off-screen/oversized restores.
    let scale_factor = window.scale_factor().unwrap_or(1.0);
    let monitor_rect = match window.current_monitor() {
        Ok(Some(monitor)) => {
            let monitor_pos = monitor.position().to_logical::<f64>(scale_factor);
            let monitor_size = monitor.size().to_logical::<f64>(scale_factor);
            Some(window_state::MonitorRect {
                x: monitor_pos.x,
                y: monitor_pos.y,
                width: monitor_size.width,
                height: monitor_size.height,
            })
        }
        _ => None,
    };

    let Some(action) = window_state::compute_restore_action(&ws, monitor_rect) else {
        return;
    };

    match action {
        window_state::RestoreAction::Maximize => {
            let _ = window.maximize();
        }
        window_state::RestoreAction::Bounds {
            width,
            height,
            x,
            y,
        } => {
            let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(width, height)));
            if let (Some(x), Some(y)) = (x, y) {
                let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
                    x, y,
                )));
            }
        }
    }
}

pub(crate) async fn persist_window_state_on_exit(app_handle: &tauri::AppHandle) {
    let window = app_handle
        .get_webview_window("main")
        .or_else(|| app_handle.webview_windows().values().next().cloned());

    let Some(window) = window else {
        return;
    };

    persist_window_state_from_webview_window(&window).await;
}

/// `tauri::Builder::on_window_event` handler. Persists window bounds on
/// move/resize and gracefully handles `CloseRequested` (flush frontend state,
/// stop ZAP, persist bounds, then destroy the window).
pub(crate) fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    static SAVE_SEQ: AtomicU64 = AtomicU64::new(0);

    match event {
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
            let seq = SAVE_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
            let window = window.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                if SAVE_SEQ.load(Ordering::SeqCst) != seq {
                    return;
                }
                persist_window_state_from_window(&window).await;
            });
        }
        tauri::WindowEvent::CloseRequested { api, .. } => {
            static CLOSING: AtomicBool = AtomicBool::new(false);
            if CLOSING.swap(true, Ordering::SeqCst) {
                return;
            }
            api.prevent_close();
            let w = window.clone();
            let app_handle = window.app_handle().clone();
            let _ = window.emit("flush-state", ());
            tauri::async_runtime::spawn(async move {
                if let Some(pentest) = app_handle.try_state::<tools::pentest::PentestState>() {
                    tracing::info!("[AppClose] Stopping ZAP before exit");
                    let _ = pentest.zap_manager.stop().await;
                }
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                persist_window_state_from_window(&w).await;
                w.destroy().ok();
            });
        }
        _ => {}
    }
}

/// `tauri::App::run` handler. Ensures we save window bounds on Cmd+Q / app
/// quit, even if the frontend doesn't get a chance to flush its debounced
/// state.
pub(crate) fn handle_run_event(app_handle: &tauri::AppHandle, event: tauri::RunEvent) {
    static EXITING: AtomicBool = AtomicBool::new(false);

    if let tauri::RunEvent::ExitRequested { api, .. } = event {
        if EXITING.swap(true, Ordering::SeqCst) {
            return;
        }

        api.prevent_exit();
        let handle = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            persist_window_state_on_exit(&handle).await;
            handle.exit(0);
        });
    }
}
