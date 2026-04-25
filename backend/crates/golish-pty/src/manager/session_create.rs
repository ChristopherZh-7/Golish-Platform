//! [`PtyManager::create_session_internal`] — generic session creation.
//!
//! Spawns the shell, wires up shell integration (ZDOTDIR / `--rcfile`),
//! resolves the working directory, opens a PTY pair, and starts the
//! reader/emitter thread pair.

use parking_lot::Mutex;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use uuid::Uuid;

use crate::error::{PtyError, Result};
use crate::parser::{OscEvent, TerminalParser};
use crate::shell::{detect_shell, ShellIntegration};

use super::core::{ActiveSession, PtyManager, PtySession};
use super::emitter::PtyEventEmitter;
use super::utf8::{process_utf8_with_buffer, OutputMessage, Utf8IncompleteBuffer};

impl PtyManager {
    /// Internal implementation that takes a generic emitter.
    ///
    /// Core session creation logic, abstracted over the event emission
    /// mechanism.
    pub(super) fn create_session_internal<E: PtyEventEmitter>(
        &self,
        emitter: Arc<E>,
        working_directory: Option<PathBuf>,
        rows: u16,
        cols: u16,
    ) -> Result<PtySession> {
        let session_id = Uuid::new_v4().to_string();

        tracing::info!(
            session_id = %session_id,
            rows = rows,
            cols = cols,
            requested_dir = ?working_directory,
            "Creating PTY session"
        );

        let pty_system = native_pty_system();

        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system
            .openpty(size)
            .map_err(|e| PtyError::Pty(e.to_string()))?;

        // Detect shell from environment (settings integration can be
        // added later).
        let shell_env = std::env::var("SHELL").ok();
        let shell_info = detect_shell(None, shell_env.as_deref());

        tracing::info!(
            "Spawning shell: {} (detected type: {:?})",
            shell_info.path.display(),
            shell_info.shell_type()
        );

        let mut cmd = CommandBuilder::new(shell_info.path.to_str().unwrap_or("/bin/sh"));

        // Set up shell integration (ZDOTDIR for zsh, --rcfile for bash,
        // etc.). This injects OSC 133 sequences automatically without
        // requiring config-file edits.
        let integration = ShellIntegration::setup(shell_info.shell_type());

        // For shells with integration that provides custom args (like
        // bash --rcfile), use those instead of the default login args.
        let shell_args = integration.as_ref().map(|i| i.shell_args());
        if let Some(ref args) = shell_args {
            if !args.is_empty() {
                tracing::debug!(
                    session_id = %session_id,
                    args = ?args,
                    "Using integration shell args"
                );
                for arg in args {
                    cmd.arg(arg);
                }
            } else {
                cmd.args(shell_info.login_args());
            }
        } else {
            cmd.args(shell_info.login_args());
        }

        cmd.env("QBIT", "1");
        cmd.env("QBIT_VERSION", env!("CARGO_PKG_VERSION"));
        cmd.env("TERM", "xterm-256color");
        if std::env::var("LANG").is_err() {
            cmd.env("LANG", "en_US.UTF-8");
        }
        if std::env::var("LC_ALL").is_err() {
            cmd.env("LC_ALL", "en_US.UTF-8");
        }
        // Note: set QBIT_DEBUG=1 to enable shell integration debug output.

        // Set integration environment variables.
        if let Some(integration) = integration {
            for (key, value) in integration.env_vars() {
                tracing::debug!(
                    session_id = %session_id,
                    key = %key,
                    value = %value,
                    "Setting shell integration env var"
                );
                cmd.env(key, value);
            }
        }

        let (work_dir, dir_source) = if let Some(dir) = working_directory {
            (dir, "explicit")
        } else if let Ok(workspace) = std::env::var("QBIT_WORKSPACE") {
            // Expand ~ to home directory.
            let path = if let Some(stripped) = workspace.strip_prefix("~/") {
                if let Some(home) = dirs::home_dir() {
                    home.join(stripped)
                } else {
                    PathBuf::from(&workspace)
                }
            } else {
                PathBuf::from(&workspace)
            };
            (path, "QBIT_WORKSPACE")
        } else if let Ok(init_cwd) = std::env::var("INIT_CWD") {
            (PathBuf::from(init_cwd), "INIT_CWD")
        } else if let Ok(cwd) = std::env::current_dir() {
            // If cwd is root "/", fall through to home_dir — this
            // happens when launched from Finder.
            if cwd.as_os_str() == "/" {
                (
                    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
                    "home_dir (cwd was root)",
                )
            // If we're in src-tauri, go up to project root.
            } else if cwd.ends_with("src-tauri") {
                if let Some(parent) = cwd.parent() {
                    (parent.to_path_buf(), "current_dir (adjusted)")
                } else {
                    (cwd, "current_dir")
                }
            } else {
                (cwd, "current_dir")
            }
        } else {
            (
                dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
                "home_dir fallback",
            )
        };

        tracing::debug!(
            session_id = %session_id,
            work_dir = %work_dir.display(),
            source = dir_source,
            "Working directory resolved"
        );

        cmd.cwd(&work_dir);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::Pty(e.to_string()))?;

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Pty(e.to_string()))?;

        let master = Arc::new(Mutex::new(pair.master));

        let session = Arc::new(ActiveSession {
            child: Mutex::new(child),
            master: master.clone(),
            writer: Mutex::new(writer),
            working_directory: Mutex::new(work_dir.clone()),
            rows: Mutex::new(rows),
            cols: Mutex::new(cols),
        });

        // Store session.
        {
            let mut sessions = self.sessions.lock();
            sessions.insert(session_id.clone(), session.clone());
        }

        // Start read thread with the generic emitter.
        let reader_session_id = session_id.clone();
        let reader_session = session.clone();

        // Get a reader from the master.
        let mut reader = {
            let master = master.lock();
            master
                .try_clone_reader()
                .map_err(|e| PtyError::Pty(e.to_string()))?
        };

        // Channel for passing raw output bytes from the reader thread to
        // the emitter thread. Allows the emitter to coalesce bursts of
        // small reads into batched IPC events (~60 fps / 16 ms window).
        let (output_tx, output_rx) = std::sync::mpsc::channel::<OutputMessage>();

        // Clone emitter for the output emitter thread (reader keeps the
        // original).
        let emitter_for_output = emitter.clone();
        let output_session_id = session_id.clone();

        // Spawn reader thread.
        let reader_session_id_for_log = reader_session_id.clone();
        tracing::trace!(
            session_id = %reader_session_id_for_log,
            "Spawning PTY reader thread"
        );

        thread::spawn(move || {
            tracing::trace!(
                session_id = %reader_session_id,
                "PTY reader thread started"
            );

            let mut parser = TerminalParser::new();
            let mut buf = [0u8; 4096];
            let mut total_bytes_read: u64 = 0;
            // Note: utf8_buffer moved to emitter thread — UTF-8
            // boundary handling happens there.

            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        tracing::debug!(
                            session_id = %reader_session_id,
                            total_bytes = total_bytes_read,
                            "PTY reader received EOF"
                        );
                        // Signal EOF to emitter thread; it will flush
                        // any pending UTF-8 bytes and emit
                        // session_ended.
                        let _ = output_tx.send(OutputMessage::Eof);
                        break;
                    }
                    Ok(n) => {
                        total_bytes_read += n as u64;
                        let data = &buf[..n];

                        // Parse and filter: only Output region bytes are
                        // returned. Prompt (A→B) and Input (B→C)
                        // regions are suppressed.
                        let parse_result = parser.parse_filtered(data);

                        if !parse_result.events.is_empty() {
                            tracing::trace!(
                                session_id = %reader_session_id,
                                event_count = parse_result.events.len(),
                                events = ?parse_result.events,
                                "Parsed OSC events"
                            );
                        }

                        // Semantic events are emitted directly from the
                        // reader thread. The corresponding output bytes
                        // for the same reads are queued in the channel.
                        // Delivery ordering of semantic vs. output
                        // events via Tauri IPC was never strictly
                        // guaranteed, so this is acceptable.
                        for event in parse_result.events {
                            match &event {
                                OscEvent::DirectoryChanged { path } => {
                                    // Update the session's working
                                    // directory so path completion uses
                                    // the current directory, not the
                                    // initial one.
                                    let new_path = PathBuf::from(path);
                                    let mut current = reader_session.working_directory.lock();
                                    if *current != new_path {
                                        tracing::warn!(
                                            session_id = %reader_session_id,
                                            old_dir = %current.display(),
                                            new_dir = %new_path.display(),
                                            "[cwd-debug] PTY manager emitting directory_changed event"
                                        );
                                        tracing::trace!(
                                            session_id = %reader_session_id,
                                            old_dir = %current.display(),
                                            new_dir = %new_path.display(),
                                            "Working directory changed"
                                        );
                                        *current = new_path;
                                        drop(current); // Release lock before emitting.
                                        emitter.emit_directory_changed(&reader_session_id, path);
                                    }
                                }
                                OscEvent::VirtualEnvChanged { name } => {
                                    emitter.emit_virtual_env_changed(
                                        &reader_session_id,
                                        name.as_deref(),
                                    );
                                }
                                OscEvent::AlternateScreenEnabled => {
                                    emitter.emit_alternate_screen(&reader_session_id, true);
                                }
                                OscEvent::AlternateScreenDisabled => {
                                    emitter.emit_alternate_screen(&reader_session_id, false);
                                }
                                OscEvent::SynchronizedOutputEnabled => {
                                    emitter.emit_synchronized_output(&reader_session_id, true);
                                }
                                OscEvent::SynchronizedOutputDisabled => {
                                    emitter.emit_synchronized_output(&reader_session_id, false);
                                }
                                _ => {
                                    if let Some((event_name, payload)) =
                                        event.to_command_block_event(&reader_session_id)
                                    {
                                        emitter.emit_command_block(event_name, payload);
                                    }
                                }
                            }
                        }

                        // Send raw output bytes to the emitter thread
                        // for coalescing. UTF-8 boundary handling
                        // happens in the emitter thread.
                        if !parse_result.output.is_empty() {
                            let _ = output_tx.send(OutputMessage::Data(parse_result.output));
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            session_id = %reader_session_id,
                            error = %e,
                            error_kind = ?e.kind(),
                            total_bytes = total_bytes_read,
                            "PTY read error"
                        );
                        let _ = output_tx.send(OutputMessage::Eof);
                        break;
                    }
                }
            }

            tracing::trace!(
                session_id = %reader_session_id,
                total_bytes = total_bytes_read,
                "PTY reader thread exiting"
            );
        });

        // Spawn output emitter thread.
        //
        // Receives raw output bytes from the reader thread and
        // coalesces bursts into batched emit calls. TUI apps doing
        // full-screen redraws produce many small reads per frame;
        // without coalescing these become a flood of Tauri IPC events
        // that saturate the bridge. The 16 ms timeout targets ~60 fps.
        thread::spawn(move || {
            let mut utf8_buffer = Utf8IncompleteBuffer::new();
            let mut coalesce_buf: Vec<u8> = Vec::with_capacity(16 * 1024);
            let timeout = std::time::Duration::from_millis(16);

            loop {
                match output_rx.recv_timeout(timeout) {
                    Ok(OutputMessage::Data(bytes)) => {
                        coalesce_buf.extend_from_slice(&bytes);

                        // Drain all immediately-queued messages without
                        // blocking, coalescing them into a single emit
                        // call.
                        loop {
                            match output_rx.try_recv() {
                                Ok(OutputMessage::Data(more)) => {
                                    coalesce_buf.extend_from_slice(&more);
                                }
                                Ok(OutputMessage::Eof) => {
                                    // Flush coalesced bytes, then emit
                                    // session_ended.
                                    let output = process_utf8_with_buffer(
                                        &mut utf8_buffer,
                                        &coalesce_buf,
                                    );
                                    if !output.is_empty() {
                                        emitter_for_output
                                            .emit_output(&output_session_id, &output);
                                    }
                                    if utf8_buffer.has_pending() {
                                        let remaining =
                                            String::from_utf8_lossy(utf8_buffer.as_slice())
                                                .to_string();
                                        if !remaining.is_empty() {
                                            emitter_for_output
                                                .emit_output(&output_session_id, &remaining);
                                        }
                                    }
                                    emitter_for_output.emit_session_ended(&output_session_id);
                                    return;
                                }
                                Err(_) => break,
                            }
                        }

                        // Emit the coalesced batch.
                        let output = process_utf8_with_buffer(&mut utf8_buffer, &coalesce_buf);
                        if !output.is_empty() {
                            emitter_for_output.emit_output(&output_session_id, &output);
                        }
                        coalesce_buf.clear();
                    }
                    Ok(OutputMessage::Eof) => {
                        // Flush any incomplete UTF-8 sequence, then
                        // signal session end.
                        if utf8_buffer.has_pending() {
                            let remaining =
                                String::from_utf8_lossy(utf8_buffer.as_slice()).to_string();
                            if !remaining.is_empty() {
                                emitter_for_output.emit_output(&output_session_id, &remaining);
                            }
                        }
                        emitter_for_output.emit_session_ended(&output_session_id);
                        return;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        // Idle — nothing to flush.
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        // Reader thread exited without sending an
                        // explicit Eof message.
                        emitter_for_output.emit_session_ended(&output_session_id);
                        return;
                    }
                }
            }
        });

        Ok(PtySession {
            id: session_id,
            working_directory: work_dir.to_string_lossy().to_string(),
            rows,
            cols,
        })
    }
}
