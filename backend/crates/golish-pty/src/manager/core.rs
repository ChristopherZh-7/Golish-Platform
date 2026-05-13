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
use crate::parser::TerminalParser;
use crate::shell::ShellType;

use parking_lot::Mutex;

use portable_pty::{Child, MasterPty, PtySize};
use serde::{Deserialize, Serialize};

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use golish_core::runtime::GolishRuntime;

use super::emitter::{CommandBlockEvent, PtyEventEmitter, RuntimeEmitter};

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
    /// Shell type the PTY is running. Drives a couple of small per-shell
    /// behaviours such as the synthetic OSC 133;C injection for
    /// PowerShell on Windows (see [`PtyManager::write`]).
    pub(super) shell_type: ShellType,
    /// Parser instance shared with the reader thread. Writes that need to
    /// synthesize OSC events (e.g. CommandStart for PowerShell) take this
    /// lock so reader-thread state stays coherent.
    pub(super) parser: Arc<Mutex<TerminalParser>>,
    /// Event emitter shared with the reader thread. Used by writes to
    /// dispatch synthesized command-block events.
    pub(super) emitter: Arc<dyn PtyEventEmitter>,
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
            .ok_or_else(|| PtyError::SessionNotFound(session_id.to_string()))?
            .clone();
        drop(sessions);

        // For PowerShell sessions we synthesize the OSC 133;C
        // (CommandStart) sequence when the caller injects a complete
        // command via stdin. PowerShell has no preexec hook (unlike zsh
        // preexec / bash DEBUG trap) so the integration script can't
        // emit CommandStart by itself. Without this hook, the Timeline
        // never sees a CommandStart for stdin-injected commands and so
        // it never produces a command block.
        //
        // We feed a synthetic byte sequence into the shared parser so
        // its region state stays coherent with what the reader thread
        // sees, then dispatch the resulting events through the same
        // emitter the reader thread uses.
        if Self::needs_synthetic_command_start(session.shell_type) {
            if let Some(cmd) = Self::extract_injected_command(data) {
                Self::inject_command_start(&session, session_id, &cmd);
            }
        }

        let mut writer = session.writer.lock();
        writer.write_all(data).map_err(PtyError::Io)?;
        writer.flush().map_err(PtyError::Io)?;

        Ok(())
    }

    /// PowerShell (and `cmd.exe`) on Windows have no preexec / DEBUG
    /// trap. We synthesize OSC 133;C for those shells so the Timeline
    /// sees CommandStart even when a command is injected via stdin from
    /// the in-app input box (rather than typed interactively).
    fn needs_synthetic_command_start(shell_type: ShellType) -> bool {
        matches!(shell_type, ShellType::PowerShell)
    }

    /// Extract a single complete command line from a stdin write, or
    /// `None` if the write does not look like a complete command.
    ///
    /// A "complete command" is the substring before the first `\n` (or
    /// `\r\n`), with leading whitespace trimmed. We deliberately ignore
    /// writes that don't end with a newline (typing one character at a
    /// time) and writes that are pure whitespace.
    fn extract_injected_command(data: &[u8]) -> Option<String> {
        let text = std::str::from_utf8(data).ok()?;
        let newline_idx = text.find('\n')?;
        let line = &text[..newline_idx];
        let line = line.trim_end_matches('\r').trim();
        if line.is_empty() {
            return None;
        }
        // Skip control-only writes such as the Ctrl-C `\x03` payload we
        // already emit elsewhere — they never carry a command.
        if line.bytes().all(|b| b < 0x20) {
            return None;
        }
        Some(line.to_string())
    }

    /// Synthesize an OSC 133;C event for the supplied command.
    ///
    /// Feeds the bytes through the shared parser so the parser's
    /// internal region tracking advances (it now treats the next read
    /// as Output region, which is exactly what we want for the
    /// command's stdout/stderr), then forwards the resulting
    /// CommandStart event through the same emitter the reader thread
    /// uses. No bytes are written to the PTY — these synthetic events
    /// only travel through Golish's own event bus.
    fn inject_command_start(session: &Arc<ActiveSession>, session_id: &str, command: &str) {
        let mut payload = Vec::with_capacity(command.len() + 16);
        payload.extend_from_slice(b"\x1b]133;C;");
        payload.extend_from_slice(command.as_bytes());
        payload.push(0x07);

        let mut parser = session.parser.lock();
        let result = parser.parse_filtered(&payload);
        drop(parser);

        for event in result.events {
            if let Some((event_name, mut block_event)) =
                event.to_command_block_event(session_id)
            {
                // Backfill the command text — to_command_block_event
                // only sets `command` when the event was constructed
                // with it, but parser.parse_filtered may yield
                // CommandStart whose command field is None when the
                // marker is just "C" without trailing ";<cmd>". Make
                // sure the synthetic event carries the command name.
                if matches!(event, crate::parser::OscEvent::CommandStart { .. })
                    && block_event.command.is_none()
                {
                    block_event = CommandBlockEvent {
                        command: Some(command.to_string()),
                        ..block_event
                    };
                }
                session.emitter.emit_command_block(event_name, block_event);
            }
        }
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
        // Verify session exists.
        let sessions = self.sessions.lock();
        if !sessions.contains_key(session_id) {
            return Err(PtyError::SessionNotFound(session_id.to_string()));
        }
        drop(sessions);

        Ok(golish_platform::process::foreground_process_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_command_start_only_for_powershell() {
        assert!(PtyManager::needs_synthetic_command_start(ShellType::PowerShell));
        assert!(!PtyManager::needs_synthetic_command_start(ShellType::Zsh));
        assert!(!PtyManager::needs_synthetic_command_start(ShellType::Bash));
        assert!(!PtyManager::needs_synthetic_command_start(ShellType::Fish));
        assert!(!PtyManager::needs_synthetic_command_start(ShellType::Sh));
        assert!(!PtyManager::needs_synthetic_command_start(ShellType::Cmd));
        assert!(!PtyManager::needs_synthetic_command_start(ShellType::Unknown));
    }

    #[test]
    fn extracts_simple_command() {
        assert_eq!(
            PtyManager::extract_injected_command(b"ls\n"),
            Some("ls".to_string())
        );
    }

    #[test]
    fn extracts_command_with_args() {
        assert_eq!(
            PtyManager::extract_injected_command(b"git status --short\n"),
            Some("git status --short".to_string())
        );
    }

    #[test]
    fn extracts_command_with_crlf() {
        assert_eq!(
            PtyManager::extract_injected_command(b"pwd\r\n"),
            Some("pwd".to_string())
        );
    }

    #[test]
    fn trims_leading_and_trailing_whitespace() {
        assert_eq!(
            PtyManager::extract_injected_command(b"  echo hi  \n"),
            Some("echo hi".to_string())
        );
    }

    #[test]
    fn rejects_writes_without_newline() {
        // Partial typing should not synthesize a CommandStart — wait for
        // the user (or the input box) to actually submit the command.
        assert_eq!(PtyManager::extract_injected_command(b"ls"), None);
    }

    #[test]
    fn rejects_blank_line() {
        assert_eq!(PtyManager::extract_injected_command(b"\n"), None);
        assert_eq!(PtyManager::extract_injected_command(b"   \n"), None);
    }

    #[test]
    fn rejects_control_only_payload() {
        // Ctrl-C (0x03) and similar control bytes are written by the
        // shortcut handlers, never accompanied by a real command.
        assert_eq!(PtyManager::extract_injected_command(b"\x03\n"), None);
        assert_eq!(PtyManager::extract_injected_command(b"\x04\n"), None);
    }

    #[test]
    fn rejects_non_utf8_payload() {
        assert_eq!(
            PtyManager::extract_injected_command(&[0xff, 0xfe, b'\n']),
            None
        );
    }

    #[test]
    fn extracts_first_line_only_for_multiline_paste() {
        // If the input box ever pastes multiple lines we still want a
        // single CommandStart for the first line.
        assert_eq!(
            PtyManager::extract_injected_command(b"first\nsecond\n"),
            Some("first".to_string())
        );
    }
}
