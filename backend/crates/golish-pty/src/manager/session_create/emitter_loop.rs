//! PTY output emitter thread body: coalesces bursts of small reads into
//! batched IPC emits (~60 fps), drives the alt-screen GridTerminal stream,
//! and runs the Warp-style `stdin_wait` idle heuristic.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use crate::grid::GridManager;

use super::super::emitter::PtyEventEmitter;
use super::super::stdin_wait_detector::{
    append_to_tail, detect_stdin_wait, STDIN_WAIT_IDLE_THRESHOLD,
};
use super::super::utf8::{process_utf8_with_buffer, OutputMessage, Utf8IncompleteBuffer};
use super::util::{hex_encode, GRID_EMIT_INTERVAL};

/// Output emitter loop. Receives raw output bytes from the reader thread
/// and coalesces bursts into batched emit calls. TUI apps doing
/// full-screen redraws produce many small reads per frame; without
/// coalescing these become a flood of Tauri IPC events that saturate the
/// bridge. The 16 ms timeout targets ~60 fps.
pub(super) fn run_emitter_loop(
    output_rx: Receiver<OutputMessage>,
    emitter_for_output: Arc<dyn PtyEventEmitter>,
    output_session_id: String,
    emitter_grid_manager: Arc<GridManager>,
    emitter_alt_screen: Arc<AtomicBool>,
) {
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
            Ok(OutputMessage::Data {
                output,
                prompt_visible,
            }) => {
                coalesce_buf.extend_from_slice(&output);
                prompt_visible_buf.extend_from_slice(&prompt_visible);

                // Drain all immediately-queued messages without
                // blocking, coalescing them into a single emit
                // call.
                loop {
                    match output_rx.try_recv() {
                        Ok(OutputMessage::Data {
                            output,
                            prompt_visible,
                        }) => {
                            coalesce_buf.extend_from_slice(&output);
                            prompt_visible_buf.extend_from_slice(&prompt_visible);
                        }
                        Ok(OutputMessage::Eof) => {
                            // Flush coalesced bytes, then emit
                            // session_ended.
                            let output = process_utf8_with_buffer(&mut utf8_buffer, &coalesce_buf);
                            if !output.is_empty() {
                                emitter_for_output.emit_output(&output_session_id, &output);
                            }
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
                    if pty_dump_emit_enabled && pty_dump_emits_counter < pty_dump_emit_cap {
                        pty_dump_emits_counter += 1;
                        const MAX_DUMP: usize = 512;
                        let preview = &output.as_bytes()[..output.len().min(MAX_DUMP)];
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
                        if let Some(grid) = emitter_grid_manager.get(&output_session_id) {
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
                    let remaining = String::from_utf8_lossy(utf8_buffer.as_slice()).to_string();
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
                        emitter_for_output.emit_stdin_wait(&output_session_id, kind.as_event_str());
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
                        emitter_for_output.emit_grid_update(&output_session_id, &snapshot);
                        grid_last_emit_at = std::time::Instant::now();
                        grid_pending_emit = false;
                    }
                }
                last_seen_alt_screen = alt_now;

                // Coalesced grid diff: fire at most once per
                // GRID_EMIT_INTERVAL while bytes are arriving.
                if alt_now && grid_pending_emit && grid_last_emit_at.elapsed() >= GRID_EMIT_INTERVAL
                {
                    if let Some(grid) = emitter_grid_manager.get(&output_session_id) {
                        let snapshot = grid.lock().snapshot_diff();
                        if !snapshot.is_noop() {
                            emitter_for_output.emit_grid_update(&output_session_id, &snapshot);
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
}
