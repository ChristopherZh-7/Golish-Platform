//! Tauri-backed implementation of [`golish_core::EventEmitter`].
//!
//! This is the glue between the `tauri::AppHandle`-based event system used
//! by the desktop shell and the `EventEmitter` trait consumed by business
//! logic crates (scan_runner, vuln_intel, indexer, projects).
//!
//! It lives in `golish-app-core` (the application-boundary shared crate) so
//! every per-domain app crate (`golish-vuln-app`, …) can construct it inside
//! `#[tauri::command]` handlers without depending on the monolithic `golish`
//! crate. Keeping the `AppHandle` here (and only here) means the business
//! crates can still be compiled + tested without Tauri in scope.

use golish_core::{EventEmitter, EventEmitterHandle};
use tauri::Emitter;

/// Thin adapter that emits events through a `tauri::AppHandle`.
///
/// Constructed once per request inside `tauri::command` handlers and passed
/// to the underlying business logic as `&EventEmitterHandle`.
#[derive(Debug, Clone)]
pub struct TauriEventEmitter {
    app: tauri::AppHandle,
}

impl TauriEventEmitter {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }

    /// Build a ready-to-use [`EventEmitterHandle`] backed by a `tauri::AppHandle`.
    pub fn handle(app: tauri::AppHandle) -> EventEmitterHandle {
        EventEmitterHandle::new(Self::new(app))
    }
}

impl EventEmitter for TauriEventEmitter {
    fn emit_json(&self, event: &str, payload: serde_json::Value) {
        if let Err(e) = self.app.emit(event, payload) {
            tracing::warn!(event = %event, error = %e, "[tauri-emitter] emit failed");
        }
    }
}
