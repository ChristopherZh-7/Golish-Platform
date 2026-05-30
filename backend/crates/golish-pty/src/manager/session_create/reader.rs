//! PTY reader thread body: reads raw bytes off the master, runs them
//! through the shared parser, dispatches OSC events, and forwards the
//! region-filtered output to the emitter thread.

use std::io::Read;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::grid::GridManager;
use crate::parser::TerminalParser;

use super::super::core::ActiveSession;
use super::super::emitter::PtyEventEmitter;
use super::super::utf8::OutputMessage;
use super::util::{dispatch_parsed_events, hex_encode};

/// Reader loop. Runs on its own thread until the PTY hits EOF or a read
/// error; signals the emitter thread via `output_tx`.
pub(super) fn run_reader_loop(
    mut reader: Box<dyn Read + Send>,
    session_id: String,
    session: Arc<ActiveSession>,
    emitter: Arc<dyn PtyEventEmitter>,
    parser: Arc<Mutex<TerminalParser>>,
    grid_manager: Arc<GridManager>,
    output_tx: Sender<OutputMessage>,
) {
    tracing::trace!(
        session_id = %session_id,
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
                    session_id = %session_id,
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
                    let mut parser = parser.lock();
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
                        session_id = %session_id,
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
                        session_id = %session_id,
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
                    &session_id,
                    &session,
                    &emitter,
                    &grid_manager,
                );

                // Send raw output bytes to the emitter thread
                // for coalescing. UTF-8 boundary handling
                // happens in the emitter thread. We forward
                // `prompt_visible` alongside `output` so the
                // `stdin_wait` detector can probe PS1/PS2/PS3
                // prompts that are intentionally absent from
                // the region-filtered `output`.
                if !parse_result.output.is_empty() || !parse_result.prompt_visible.is_empty() {
                    let _ = output_tx.send(OutputMessage::Data {
                        output: parse_result.output,
                        prompt_visible: parse_result.prompt_visible,
                    });
                }
            }
            Err(e) => {
                tracing::error!(
                    session_id = %session_id,
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
        session_id = %session_id,
        total_bytes = total_bytes_read,
        "PTY reader thread exiting"
    );
}
