//! Tauri-backed implementation of [`golish_core::EventEmitter`].
//!
//! This is the glue between the `tauri::AppHandle`-based event system used
//! by the desktop shell and the `EventEmitter` trait consumed by business
//! logic crates (pipeline, scan_runner, vuln_intel, indexer, projects).
//!
//! Keeping the `AppHandle` here (and only here) means the business crates
//! can be compiled + tested without Tauri in scope.

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
