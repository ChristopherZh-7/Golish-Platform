//! [`PtyManager::create_session_internal`] — generic session creation.
//!
//! Spawns the shell, wires up shell integration (ZDOTDIR / `--rcfile`),
//! resolves the working directory, opens a PTY pair, and starts the
//! reader/emitter thread pair.

use parking_lot::Mutex;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use std::fmt::Write as _;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

/// Lower-case hex encode for `QBIT_PTY_DUMP=1` raw-byte trace logging.
/// Inlined to avoid pulling in the `hex` crate just for a debug helper.
fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let _ = write!(out, "{:02x}", b);
    }
    out
}

use uuid::Uuid;

use crate::error::{PtyError, Result};
use crate::grid::{GridDims, GridManager};
use crate::parser::{OscEvent, TerminalParser};
use crate::shell::{detect_shell, ShellIntegration};

use super::core::{ActiveSession, PtyManager, PtySession};
use super::emitter::PtyEventEmitter;
use super::stdin_wait_detector::{
    append_to_tail, detect_stdin_wait, STDIN_WAIT_IDLE_THRESHOLD,
};
use super::utf8::{process_utf8_with_buffer, OutputMessage, Utf8IncompleteBuffer};

/// Cap how often the emitter thread ships a `terminal_grid_update`
/// event to the frontend. 60 ms ≈ 16 fps which is enough for vim /
/// htop to feel snappy without saturating the IPC bridge.
const GRID_EMIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(60);

/// Dispatch a batch of OSC events through the supplied emitter, applying
/// the session-local side effects (e.g. updating
/// [`ActiveSession::working_directory`]). Called from both the reader
/// thread and [`PtyManager::write`] so behaviour is consistent for
/// real-PTY events and synthesized events alike.
pub(super) fn dispatch_parsed_events(
    events: Vec<OscEvent>,
    session_id: &str,
    session: &Arc<ActiveSession>,
    emitter: &Arc<dyn PtyEventEmitter>,
    grid_manager: &Arc<GridManager>,
) {
    for event in events {
        match &event {
            OscEvent::DirectoryChanged { path } => {
                let new_path = PathBuf::from(path);
                let mut current = session.working_directory.lock();
                if *current != new_path {
                    tracing::warn!(
                        session_id = %session_id,
                        old_dir = %current.display(),
                        new_dir = %new_path.display(),
                        "[cwd-debug] PTY manager emitting directory_changed event"
                    );
                    *current = new_path;
                    drop(current);
                    emitter.emit_directory_changed(session_id, path);
                }
            }
            OscEvent::VirtualEnvChanged { name } => {
                emitter.emit_virtual_env_changed(session_id, name.as_deref());
            }
            OscEvent::AlternateScreenEnabled => {
                session.alt_screen.store(true, Ordering::Release);
                // Eagerly create the GridTerminal at the current PTY
                // dimensions so the emitter thread doesn't see a stale
                // smaller grid when the first frame lands. Subsequent
                // resize events from the frontend re-shape it through
                // `PtyManager::resize_grid`.
                let cols = *session.cols.lock();
                let rows = *session.rows.lock();
                let _ = grid_manager.get_or_create(session_id, GridDims { cols, rows });
                emitter.emit_alternate_screen(session_id, true);
            }
            OscEvent::AlternateScreenDisabled => {
                session.alt_screen.store(false, Ordering::Release);
                grid_manager.dispose(session_id);
                emitter.emit_alternate_screen(session_id, false);
            }
            OscEvent::SynchronizedOutputEnabled => {
                emitter.emit_synchronized_output(session_id, true);
            }
            OscEvent::SynchronizedOutputDisabled => {
                emitter.emit_synchronized_output(session_id, false);
            }
            _ => {
                if let Some((event_name, payload)) = event.to_command_block_event(session_id) {
                    emitter.emit_command_block(event_name, payload);
                }
            }
        }
    }
}

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

        // Erase the emitter's static type so it can be stored in
        // ActiveSession (which can't carry a generic parameter without
        // poisoning every call site). The reader thread + write-side
        // injection logic both work through this type-erased Arc.
        let emitter: Arc<dyn PtyEventEmitter> = emitter;

        // Shared parser so [`PtyManager::write`] can synthesize OSC
        // events (e.g. CommandStart for PowerShell on Windows) without
        // racing the reader thread's view of the parser state.
        let parser = Arc::new(Mutex::new(TerminalParser::new()));

        let alt_screen = Arc::new(AtomicBool::new(false));

        let session = Arc::new(ActiveSession {
            child: Mutex::new(child),
            master: master.clone(),
            writer: Mutex::new(writer),
            working_directory: Mutex::new(work_dir.clone()),
            rows: Mutex::new(rows),
            cols: Mutex::new(cols),
            shell_type: shell_info.shell_type(),
            parser: parser.clone(),
            emitter: emitter.clone(),
            alt_screen: alt_screen.clone(),
        });

        // Store session.
        {
            let mut sessions = self.sessions.lock();
            sessions.insert(session_id.clone(), session.clone());
        }

        // Start read thread with the generic emitter.
        let reader_session_id = session_id.clone();
        let reader_session = session.clone();
        let reader_emitter = emitter.clone();
        let reader_parser = parser.clone();
        let reader_grid_manager = self.grid_manager.clone();

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
        let emitter_grid_manager = self.grid_manager.clone();
        let emitter_alt_screen = alt_screen.clone();

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

            let mut buf = [0u8; 4096];
            let mut total_bytes_read: u64 = 0;
            // Bounded counter for the default-on `[pty-dump]` raw-byte
            // trace below; first N reads get dumped, then we go quiet.
            let mut pty_dump_reads_counter: u64 = 0;
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
                        // regions are suppressed. Parser is shared with
                        // PtyManager::write so synthesized OSC events
                        // (e.g. PowerShell CommandStart) stay coherent
                        // with reader-thread region tracking.
                        let parse_result = {
                            let mut parser = reader_parser.lock();
                            parser.parse_filtered(data)
                        };

                        // Raw-byte dump for diagnosing the
                        // "PowerShell `dir` first-N commands collapse
                        // Mode/Directory rows" bug. Default-on for the
                        // first N reads of each session so we don't need
                        // to chase env-var plumbing across npm → cargo →
                        // tauri; set `QBIT_PTY_DUMP_MAX` env to override
                        // the read cap (default 80 reads ~= one `dir`
                        // worth of bytes), or `QBIT_PTY_DUMP=0` to
                        // disable entirely.
                        let dump_cap: u64 = std::env::var("QBIT_PTY_DUMP_MAX")
                            .ok()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(80);
                        let dump_enabled = std::env::var("QBIT_PTY_DUMP")
                            .map(|v| v != "0")
                            .unwrap_or(true);
                        if dump_enabled && pty_dump_reads_counter < dump_cap {
                            pty_dump_reads_counter += 1;
                            const MAX_DUMP: usize = 512;
                            let raw_preview = &data[..data.len().min(MAX_DUMP)];
                            let filtered_preview =
                                &parse_result.output[..parse_result.output.len().min(MAX_DUMP)];
                            tracing::info!(
                                session_id = %reader_session_id,
                                read_seq = pty_dump_reads_counter,
                                raw_len = data.len(),
                                raw_hex = %hex_encode(raw_preview),
                                raw_utf8 = %String::from_utf8_lossy(raw_preview),
                                filtered_len = parse_result.output.len(),
                                filtered_hex = %hex_encode(filtered_preview),
                                filtered_utf8 = %String::from_utf8_lossy(filtered_preview),
                                events = ?parse_result.events,
                                "[pty-dump] raw read"
                            );
                        }

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
                        dispatch_parsed_events(
                            parse_result.events,
                            &reader_session_id,
                            &reader_session,
                            &reader_emitter,
                            &reader_grid_manager,
                        );

                        // Send raw output bytes to the emitter thread
                        // for coalescing. UTF-8 boundary handling
                        // happens in the emitter thread. We forward
                        // `prompt_visible` alongside `output` so the
                        // `stdin_wait` detector can probe PS1/PS2/PS3
                        // prompts that are intentionally absent from
                        // the region-filtered `output`.
                        if !parse_result.output.is_empty()
                            || !parse_result.prompt_visible.is_empty()
                        {
                            let _ = output_tx.send(OutputMessage::Data {
                                output: parse_result.output,
                                prompt_visible: parse_result.prompt_visible,
                            });
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
            // Bounded counter for the default-on `[pty-dump]` emit trace
            // below — matches the reader-thread counter so both halves of
            // the pipeline produce comparable evidence.
            let mut pty_dump_emits_counter: u64 = 0;
            let pty_dump_emit_cap: u64 = std::env::var("QBIT_PTY_DUMP_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(80);
            let pty_dump_emit_enabled = std::env::var("QBIT_PTY_DUMP")
                .map(|v| v != "0")
                .unwrap_or(true);

            // State for the Warp-style `stdin_wait` heuristic. We keep
            // the most recent ~256 bytes of emitted output plus a
            // timestamp of the last emit so the Timeout branch can probe
            // for prompt patterns once the PTY has gone quiet for at
            // least `STDIN_WAIT_IDLE_THRESHOLD`. Tracking happens here
            // (rather than in the reader thread) because (a) the tail
            // buffer must reflect what the frontend actually sees after
            // coalescing, and (b) the existing Timeout branch already
            // wakes up every 16 ms, so we get the idle scheduling for
            // free.
            let mut stdin_wait_tail: Vec<u8> = Vec::with_capacity(256);
            let mut stdin_wait_last_emit_at = std::time::Instant::now();
            let mut stdin_wait_emitted_for_idle_window: bool = false;

            // State for the Phase B GridTerminal stream. When the
            // session is on alt-screen, bytes flowing through here are
            // also fed to the per-session `GridTerminal`; the resulting
            // grid diff is shipped to the frontend at most once per
            // [`GRID_EMIT_INTERVAL`] to keep IPC load reasonable. We
            // also fire an immediate diff right after `alt_screen`
            // flips on so the frontend gets a baseline frame
            // immediately instead of waiting for the next coalesce
            // tick.
            let mut grid_last_emit_at = std::time::Instant::now();
            let mut grid_pending_emit = false;
            let mut last_seen_alt_screen = false;

            // Parallel coalesce buffer for prompt-visible bytes (PS1 /
            // PS2 / PS3 / interactive prompt text). Separate from
            // `coalesce_buf` because we may have prompt bytes without
            // user-visible Output region bytes (zsh `select> ` is the
            // archetypal case) and vice versa during pure stdout.
            let mut prompt_visible_buf: Vec<u8> = Vec::with_capacity(16 * 1024);

            loop {
                match output_rx.recv_timeout(timeout) {
                    Ok(OutputMessage::Data { output, prompt_visible }) => {
                        coalesce_buf.extend_from_slice(&output);
                        prompt_visible_buf.extend_from_slice(&prompt_visible);

                        // Drain all immediately-queued messages without
                        // blocking, coalescing them into a single emit
                        // call.
                        loop {
                            match output_rx.try_recv() {
                                Ok(OutputMessage::Data { output, prompt_visible }) => {
                                    coalesce_buf.extend_from_slice(&output);
                                    prompt_visible_buf.extend_from_slice(&prompt_visible);
                                }
                                Ok(OutputMessage::Eof) => {
                                    // Flush coalesced bytes, then emit
                                    // session_ended.
                                    let output =
                                        process_utf8_with_buffer(&mut utf8_buffer, &coalesce_buf);
                                    if !output.is_empty() {
                                        emitter_for_output.emit_output(&output_session_id, &output);
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

                        // Refresh the stdin_wait tail from `prompt_visible`
                        // first — it's a superset of `output` plus PS1/
                        // PS2/PS3 prompts. We do this even when `output`
                        // is empty (zsh PS2 `select> ` lives only in the
                        // Input region and never makes it to `output`).
                        if !prompt_visible_buf.is_empty() {
                            append_to_tail(&mut stdin_wait_tail, &prompt_visible_buf);
                            stdin_wait_last_emit_at = std::time::Instant::now();
                            stdin_wait_emitted_for_idle_window = false;
                        }

                        // Emit the coalesced batch.
                        let output = process_utf8_with_buffer(&mut utf8_buffer, &coalesce_buf);
                        if !output.is_empty() {
                            if pty_dump_emit_enabled
                                && pty_dump_emits_counter < pty_dump_emit_cap
                            {
                                pty_dump_emits_counter += 1;
                                const MAX_DUMP: usize = 512;
                                let preview = &output.as_bytes()
                                    [..output.as_bytes().len().min(MAX_DUMP)];
                                tracing::info!(
                                    session_id = %output_session_id,
                                    emit_seq = pty_dump_emits_counter,
                                    coalesced_len = output.len(),
                                    coalesced_hex = %hex_encode(preview),
                                    coalesced_utf8 = %String::from_utf8_lossy(preview),
                                    "[pty-dump] emit to frontend"
                                );
                            }
                            emitter_for_output.emit_output(&output_session_id, &output);

                            // Phase B · GridTerminal: when the session
                            // is on alt-screen, feed the coalesced
                            // bytes into the per-session GridTerminal.
                            // The actual `terminal_grid_update` emit
                            // happens below (either as part of this
                            // tick if the 60 ms quota has elapsed, or
                            // on the next Timeout poll).
                            if emitter_alt_screen.load(Ordering::Acquire) {
                                if let Some(grid) =
                                    emitter_grid_manager.get(&output_session_id)
                                {
                                    grid.lock().write(output.as_bytes());
                                    grid_pending_emit = true;
                                }
                            }
                        }
                        coalesce_buf.clear();
                        prompt_visible_buf.clear();
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
                        // Idle — nothing to flush. Probe for a stdin
                        // prompt at the tail of recently emitted bytes
                        // so the frontend can switch its bottom input
                        // box into "respond to the running command"
                        // mode. We only fire once per idle window: a
                        // subsequent burst of output resets
                        // `stdin_wait_emitted_for_idle_window` back to
                        // false, so the next quiet period can fire
                        // again.
                        if !stdin_wait_emitted_for_idle_window
                            && stdin_wait_last_emit_at.elapsed() >= STDIN_WAIT_IDLE_THRESHOLD
                            && !stdin_wait_tail.is_empty()
                        {
                            if let Some(kind) = detect_stdin_wait(&stdin_wait_tail) {
                                emitter_for_output.emit_stdin_wait(
                                    &output_session_id,
                                    kind.as_event_str(),
                                );
                                stdin_wait_emitted_for_idle_window = true;
                            }
                        }

                        // Phase B · GridTerminal: on the rising edge of
                        // `alt_screen` (false → true) ship a full
                        // baseline frame immediately so the frontend
                        // doesn't render an empty grid while waiting
                        // for the first coalesced tick.
                        let alt_now = emitter_alt_screen.load(Ordering::Acquire);
                        if alt_now && !last_seen_alt_screen {
                            if let Some(grid) = emitter_grid_manager.get(&output_session_id) {
                                let snapshot = grid.lock().snapshot_full();
                                emitter_for_output
                                    .emit_grid_update(&output_session_id, &snapshot);
                                grid_last_emit_at = std::time::Instant::now();
                                grid_pending_emit = false;
                            }
                        }
                        last_seen_alt_screen = alt_now;

                        // Coalesced grid diff: fire at most once per
                        // GRID_EMIT_INTERVAL while bytes are arriving.
                        if alt_now
                            && grid_pending_emit
                            && grid_last_emit_at.elapsed() >= GRID_EMIT_INTERVAL
                        {
                            if let Some(grid) = emitter_grid_manager.get(&output_session_id) {
                                let snapshot = grid.lock().snapshot_diff();
                                if !snapshot.is_noop() {
                                    emitter_for_output
                                        .emit_grid_update(&output_session_id, &snapshot);
                                }
                                grid_last_emit_at = std::time::Instant::now();
                                grid_pending_emit = false;
                            }
                        }
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
