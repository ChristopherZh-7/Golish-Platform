//! Shared helpers for session creation: OSC event dispatch, hex tracing,
//! and the grid-emit cadence constant.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::grid::{GridDims, GridManager};
use crate::parser::OscEvent;

use super::super::core::ActiveSession;
use super::super::emitter::PtyEventEmitter;

/// Cap how often the emitter thread ships a `terminal_grid_update`
/// event to the frontend. 60 ms ≈ 16 fps which is enough for vim /
/// htop to feel snappy without saturating the IPC bridge.
pub(super) const GRID_EMIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(60);

/// Lower-case hex encode for `QBIT_PTY_DUMP=1` raw-byte trace logging.
/// Inlined to avoid pulling in the `hex` crate just for a debug helper.
pub(super) fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let _ = write!(out, "{:02x}", b);
    }
    out
}

/// Dispatch a batch of OSC events through the supplied emitter, applying
/// the session-local side effects (e.g. updating
/// [`ActiveSession::working_directory`]). Called from the reader thread so
/// behaviour is consistent for real-PTY events and synthesized events alike.
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
