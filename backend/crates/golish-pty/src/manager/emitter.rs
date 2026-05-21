//! Event emission abstraction for PTY sessions.
//!
//! The PTY reader thread emits a handful of event types (terminal output,
//! session ended, directory / virtual-env changes, OSC 133 command-block
//! events, alternate-screen toggles, synchronized-output toggles). The
//! [`PtyEventEmitter`] trait abstracts over how those events reach
//! consumers; the concrete [`RuntimeEmitter`] implementation forwards
//! them through [`GolishRuntime`].

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use golish_core::runtime::{GolishRuntime, RuntimeEvent};

/// Internal trait for emitting PTY events.
///
/// Abstracts over how PTY events (output, exit, directory changes, etc.)
/// are delivered to consumers. The primary implementation is
/// [`RuntimeEmitter`], which emits events via [`GolishRuntime`] (used by
/// Tauri, CLI, and other runtimes).
///
/// Implementors must be `Send + Sync + 'static` to work with `std::thread`
/// spawning in the PTY read loop.
pub(super) trait PtyEventEmitter: Send + Sync + 'static {
    /// Emit terminal output data.
    fn emit_output(&self, session_id: &str, data: &str);

    /// Emit session ended event.
    fn emit_session_ended(&self, session_id: &str);

    /// Emit directory changed event.
    fn emit_directory_changed(&self, session_id: &str, path: &str);

    /// Emit virtual environment changed event.
    fn emit_virtual_env_changed(&self, session_id: &str, name: Option<&str>);

    /// Emit a command-block event (prompt start/end, command start/end).
    fn emit_command_block(&self, event_name: &str, event: CommandBlockEvent);

    /// Emit alternate screen buffer state change. Used to trigger fullterm
    /// mode for TUI applications.
    fn emit_alternate_screen(&self, session_id: &str, enabled: bool);

    /// Emit synchronized output mode change (DEC 2026). Used to batch
    /// terminal updates atomically to prevent flickering.
    fn emit_synchronized_output(&self, session_id: &str, enabled: bool);

    /// Emit a "running command is blocking on stdin" hint. Drives the
    /// Warp-style interactive input mode in the frontend (see
    /// `docs/design/2026-05-15-warp-style-interaction.md`).
    ///
    /// `detector` is a stable identifier for the heuristic that fired
    /// (e.g. `"yn_choice"`, `"password"`) so the frontend can tweak the
    /// hint text per pattern and the integration tests can assert exact
    /// values.
    fn emit_stdin_wait(&self, session_id: &str, detector: &str);

    /// Emit a virtual terminal grid snapshot. Drives the Phase B
    /// GridTerminal frontend (see
    /// `docs/design/2026-05-15-grid-terminal-phase-b.md`). The payload
    /// is the `GridUpdate` JSON described in §3 of that document.
    fn emit_grid_update(&self, session_id: &str, update: &crate::grid::GridUpdate);
}

/// Payload for command-block lifecycle events.
#[allow(dead_code)] // Used by Tauri feature
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandBlockEvent {
    pub session_id: String,
    pub command: Option<String>,
    pub exit_code: Option<i32>,
    pub event_type: String,
}

/// Event emitter that forwards through [`GolishRuntime`] (used by Tauri,
/// CLI, and any future runtime implementations).
pub(super) struct RuntimeEmitter(pub(super) Arc<dyn GolishRuntime>);

impl PtyEventEmitter for RuntimeEmitter {
    fn emit_output(&self, session_id: &str, data: &str) {
        let bytes = data.as_bytes().to_vec();
        if let Err(e) = self.0.emit(RuntimeEvent::TerminalOutput {
            session_id: session_id.to_string(),
            data: bytes,
        }) {
            tracing::warn!(
                session_id = %session_id,
                bytes = data.len(),
                error = %e,
                "Failed to emit terminal output"
            );
        }
    }

    fn emit_session_ended(&self, session_id: &str) {
        tracing::info!(
            session_id = %session_id,
            "PTY session ended (EOF)"
        );
        // Use TerminalExit with no exit code (EOF/closed).
        if let Err(e) = self.0.emit(RuntimeEvent::TerminalExit {
            session_id: session_id.to_string(),
            code: None,
        }) {
            tracing::error!(
                session_id = %session_id,
                error = %e,
                "Failed to emit session ended event"
            );
        }
    }

    fn emit_directory_changed(&self, session_id: &str, path: &str) {
        tracing::debug!(
            session_id = %session_id,
            path = %path,
            "Emitting directory_changed"
        );
        // Use Custom event for directory changes (not yet in RuntimeEvent enum).
        if let Err(e) = self.0.emit(RuntimeEvent::Custom {
            name: "directory_changed".to_string(),
            payload: serde_json::json!({
                "session_id": session_id,
                "path": path
            }),
        }) {
            tracing::warn!(
                session_id = %session_id,
                path = %path,
                error = %e,
                "Failed to emit directory_changed event"
            );
        }
    }

    fn emit_virtual_env_changed(&self, session_id: &str, name: Option<&str>) {
        tracing::debug!(
            session_id = %session_id,
            name = ?name,
            "Emitting virtual_env_changed"
        );
        if let Err(e) = self.0.emit(RuntimeEvent::Custom {
            name: "virtual_env_changed".to_string(),
            payload: serde_json::json!({
                "session_id": session_id,
                "name": name
            }),
        }) {
            tracing::warn!(
                session_id = %session_id,
                name = ?name,
                error = %e,
                "Failed to emit virtual_env_changed event"
            );
        }
    }

    fn emit_command_block(&self, event_name: &str, event: CommandBlockEvent) {
        let payload = match serde_json::to_value(&event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    event_name = %event_name,
                    error = %e,
                    "Failed to serialize command block event"
                );
                return;
            }
        };

        if let Err(e) = self.0.emit(RuntimeEvent::Custom {
            name: event_name.to_string(),
            payload,
        }) {
            tracing::warn!(
                event_name = %event_name,
                session_id = %event.session_id,
                error = %e,
                "Failed to emit command block event"
            );
        }
    }

    fn emit_alternate_screen(&self, session_id: &str, enabled: bool) {
        tracing::trace!(
            session_id = %session_id,
            enabled = enabled,
            "Emitting alternate_screen"
        );
        if let Err(e) = self.0.emit(RuntimeEvent::Custom {
            name: "alternate_screen".to_string(),
            payload: serde_json::json!({
                "session_id": session_id,
                "enabled": enabled
            }),
        }) {
            tracing::warn!(
                session_id = %session_id,
                enabled = enabled,
                error = %e,
                "Failed to emit alternate_screen event"
            );
        }
    }

    fn emit_synchronized_output(&self, session_id: &str, enabled: bool) {
        tracing::debug!(
            session_id = %session_id,
            enabled = enabled,
            "Emitting synchronized_output"
        );
        if let Err(e) = self.0.emit(RuntimeEvent::Custom {
            name: "synchronized_output".to_string(),
            payload: serde_json::json!({
                "session_id": session_id,
                "enabled": enabled
            }),
        }) {
            tracing::warn!(
                session_id = %session_id,
                enabled = enabled,
                error = %e,
                "Failed to emit synchronized_output event"
            );
        }
    }

    fn emit_stdin_wait(&self, session_id: &str, detector: &str) {
        tracing::debug!(
            session_id = %session_id,
            detector = %detector,
            "Emitting stdin_wait"
        );
        if let Err(e) = self.0.emit(RuntimeEvent::Custom {
            name: "stdin_wait".to_string(),
            payload: serde_json::json!({
                "session_id": session_id,
                "detector": detector,
            }),
        }) {
            tracing::warn!(
                session_id = %session_id,
                detector = %detector,
                error = %e,
                "Failed to emit stdin_wait event"
            );
        }
    }

    fn emit_grid_update(&self, session_id: &str, update: &crate::grid::GridUpdate) {
        // Build the payload manually so we can flatten `update` onto
        // the same object as `session_id` — the frontend listener
        // expects `{ session_id, rev, cols, ... }` as one flat object.
        let payload = match serde_json::to_value(update) {
            Ok(serde_json::Value::Object(mut map)) => {
                map.insert(
                    "session_id".to_string(),
                    serde_json::Value::String(session_id.to_string()),
                );
                serde_json::Value::Object(map)
            }
            Ok(other) => {
                tracing::warn!(
                    session_id = %session_id,
                    "GridUpdate serialised to non-object JSON, skipping emit: {:?}",
                    other
                );
                return;
            }
            Err(e) => {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "Failed to serialize GridUpdate"
                );
                return;
            }
        };

        if let Err(e) = self.0.emit(RuntimeEvent::Custom {
            name: "terminal_grid_update".to_string(),
            payload,
        }) {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "Failed to emit terminal_grid_update event"
            );
        }
    }
}
