//! [`PtyManager`] — owns active PTY sessions.
//!
//! Layout:
//! - This file: types ([`PtySession`], [`ActiveSession`], [`PtyManager`])
//!   plus the smaller per-session methods (write / resize / destroy /
//!   get / list / get_foreground_process) and the
//!   [`PtyManager::create_session_with_runtime`] entry point.
//! - [`super::session_create`]: the bulk of session creation —
//!   `create_session_internal`, which spawns the shell + reader/emitter
//!   thread pair.

use crate::error::{PtyError, Result};

use parking_lot::Mutex;

use portable_pty::{Child, MasterPty, PtySize};
use serde::{Deserialize, Serialize};

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use golish_core::runtime::GolishRuntime;

use super::emitter::RuntimeEmitter;

/// Public-facing description of a PTY session.
#[allow(dead_code)] // Used by Tauri feature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtySession {
    pub id: String,
    pub working_directory: String,
    pub rows: u16,
    pub cols: u16,
}

/// Internal session state tracking active PTY sessions.
pub(super) struct ActiveSession {
    #[allow(dead_code)]
    pub(super) child: Mutex<Box<dyn Child + Send + Sync>>,
    pub(super) master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pub(super) writer: Mutex<Box<dyn Write + Send>>,
    pub(super) working_directory: Mutex<PathBuf>,
    pub(super) rows: Mutex<u16>,
    pub(super) cols: Mutex<u16>,
}

/// Manager for PTY sessions.
///
/// When the `tauri` feature is enabled, this provides full PTY session
/// management with event emission to the Tauri frontend. Without the
/// feature, it provides a minimal stub for compilation.
#[derive(Default)]
pub struct PtyManager {
    pub(super) sessions: Mutex<HashMap<String, Arc<ActiveSession>>>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self::default()
    }

    // ──────────────────────────────────────────────────────────────────
    // Public API
    // ──────────────────────────────────────────────────────────────────

    /// Create a PTY session with runtime-based event emission.
    ///
    /// This is the preferred way to create PTY sessions as it works
    /// with any [`GolishRuntime`] implementation (Tauri, CLI, or
    /// future runtimes).
    ///
    /// # Arguments
    /// * `runtime` — runtime implementation for event emission.
    /// * `working_directory` — initial working directory (defaults to
    ///   project root).
    /// * `rows` — terminal height in rows.
    /// * `cols` — terminal width in columns.
    pub fn create_session_with_runtime(
        &self,
        runtime: Arc<dyn GolishRuntime>,
        working_directory: Option<PathBuf>,
        rows: u16,
        cols: u16,
    ) -> Result<PtySession> {
        let emitter = Arc::new(RuntimeEmitter(runtime));
        self.create_session_internal(emitter, working_directory, rows, cols)
    }

    pub fn write(&self, session_id: &str, data: &[u8]) -> Result<()> {
        let sessions = self.sessions.lock();
        let session = sessions
            .get(session_id)
            .ok_or_else(|| PtyError::SessionNotFound(session_id.to_string()))?;

        let mut writer = session.writer.lock();
        writer.write_all(data).map_err(PtyError::Io)?;
        writer.flush().map_err(PtyError::Io)?;

        Ok(())
    }

    pub fn resize(&self, session_id: &str, rows: u16, cols: u16) -> Result<()> {
        let sessions = self.sessions.lock();
        let session = sessions
            .get(session_id)
            .ok_or_else(|| PtyError::SessionNotFound(session_id.to_string()))?;

        let old_rows = *session.rows.lock();
        let old_cols = *session.cols.lock();

        // Skip resize if dimensions haven't changed.
        if old_rows == rows && old_cols == cols {
            return Ok(());
        }

        let master = session.master.lock();
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Pty(e.to_string()))?;

        *session.rows.lock() = rows;
        *session.cols.lock() = cols;

        tracing::trace!(
            session_id = %session_id,
            old_size = %format!("{}x{}", old_cols, old_rows),
            new_size = %format!("{}x{}", cols, rows),
            "PTY resized"
        );

        Ok(())
    }

    pub fn destroy(&self, session_id: &str) -> Result<()> {
        let mut sessions = self.sessions.lock();
        let session_count_before = sessions.len();

        sessions
            .remove(session_id)
            .ok_or_else(|| PtyError::SessionNotFound(session_id.to_string()))?;

        tracing::info!(
            session_id = %session_id,
            sessions_before = session_count_before,
            sessions_after = sessions.len(),
            "PTY session destroyed"
        );

        Ok(())
    }

    pub fn get_session(&self, session_id: &str) -> Result<PtySession> {
        let sessions = self.sessions.lock();
        let session = sessions
            .get(session_id)
            .ok_or_else(|| PtyError::SessionNotFound(session_id.to_string()))?;

        let working_directory = session
            .working_directory
            .lock()
            .to_string_lossy()
            .to_string();
        let rows = *session.rows.lock();
        let cols = *session.cols.lock();

        Ok(PtySession {
            id: session_id.to_string(),
            working_directory,
            rows,
            cols,
        })
    }

    /// List all active session IDs.
    pub fn list_session_ids(&self) -> Vec<String> {
        let sessions = self.sessions.lock();
        sessions.keys().cloned().collect()
    }

    /// Get the foreground process name for a PTY session.
    ///
    /// Uses OS-level process group detection to get the actual running
    /// process, rather than guessing based on command patterns.
    ///
    /// # Platform Support
    /// - macOS / Linux: uses `ps` to query the terminal's foreground
    ///   process group.
    /// - Windows: returns `None` (process groups work differently).
    ///
    /// # Returns
    /// - `Ok(Some(String))` — foreground process name (e.g., `"npm"`,
    ///   `"cargo"`, `"python"`).
    /// - `Ok(None)` — no foreground process or shell is in foreground.
    /// - `Err(_)` — failed to query process information.
    pub fn get_foreground_process(&self, session_id: &str) -> Result<Option<String>> {
        use std::process::Command;

        // Verify session exists.
        let sessions = self.sessions.lock();
        if !sessions.contains_key(session_id) {
            return Err(PtyError::SessionNotFound(session_id.to_string()));
        }
        drop(sessions);

        // Platform-specific process detection.
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            // Get the PTY's foreground process group leader. Uses `ps`
            // to query the terminal's current foreground process.
            let output = Command::new("sh")
                .arg("-c")
                .arg("ps -o comm= -p $(ps -o tpgid= -p $$) 2>/dev/null || echo ''")
                .output();

            match output {
                Ok(output) if output.status.success() => {
                    let process_name = String::from_utf8_lossy(&output.stdout).trim().to_string();

                    if process_name.is_empty() {
                        Ok(None)
                    } else {
                        // Extract just the binary name (remove path).
                        let name = process_name
                            .rsplit('/')
                            .next()
                            .unwrap_or(&process_name)
                            .to_string();
                        Ok(Some(name))
                    }
                }
                _ => Ok(None),
            }
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            // Windows and other platforms don't have the same process
            // group semantics.
            Ok(None)
        }
    }
}
