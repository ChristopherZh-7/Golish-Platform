//! Virtual terminal grid for Phase B (GridTerminal).
//!
//! Wraps `alacritty_terminal` so the backend can maintain a complete
//! grid state machine — cursor, alt-screen, scrollback, SGR attributes —
//! and ship per-frame diffs to the frontend. This is what allows the
//! Phase B frontend (`GridTerminal.tsx`) to render TUI applications
//! (vim, htop, less, …) without xterm.js, which fixes the Windows
//! WebView2 / WebGL black-screen bug we hit repeatedly.
//!
//! Public entry points:
//!
//! * [`GridManager`] owns one [`GridTerminal`] per active PTY session.
//! * [`GridTerminal::write`] feeds raw PTY bytes through the vte parser
//!   into the alacritty `Term` state machine.
//! * [`GridTerminal::snapshot_full`] / [`GridTerminal::snapshot_diff`]
//!   produce serialisable structures destined for the frontend.
//!
//! The actual wire format and frontend protocol live in
//! [`super::grid::snapshot::GridUpdate`]; see also
//! `docs/design/2026-05-15-grid-terminal-phase-b.md`.

mod cell;
mod snapshot;
mod terminal;

#[cfg(test)]
mod tests;

pub use cell::{Cell, CellAttrs, Color, Cursor, CursorStyle};
pub use snapshot::{GridUpdate, RowUpdate};
pub use terminal::{GridDims, GridTerminal};

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

/// Owns one [`GridTerminal`] per PTY session id.
///
/// Cheap to construct (a `Default` impl is provided). Cloning is
/// intentionally not implemented — the manager is meant to live behind
/// an `Arc` shared with the PTY emitter thread.
#[derive(Default)]
pub struct GridManager {
    sessions: Mutex<HashMap<String, Arc<Mutex<GridTerminal>>>>,
}

impl GridManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create a [`GridTerminal`] for `session_id`. Initialised at
    /// `dims.cols × dims.rows`; callers should call
    /// [`GridTerminal::resize`] later when the frontend reports a new
    /// viewport size.
    pub fn get_or_create(
        &self,
        session_id: &str,
        dims: GridDims,
    ) -> Arc<Mutex<GridTerminal>> {
        let mut sessions = self.sessions.lock();
        sessions
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(GridTerminal::new(dims))))
            .clone()
    }

    /// Look up an existing terminal without creating one.
    pub fn get(&self, session_id: &str) -> Option<Arc<Mutex<GridTerminal>>> {
        self.sessions.lock().get(session_id).cloned()
    }

    /// Drop the terminal for `session_id`. Called when the PTY session
    /// exits or leaves alt-screen.
    pub fn dispose(&self, session_id: &str) {
        self.sessions.lock().remove(session_id);
    }

    /// Number of sessions currently tracked. Test helper.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.sessions.lock().len()
    }

    /// True when no sessions are tracked. Test helper; paired with
    /// [`Self::len`] so `clippy::len_without_is_empty` stays quiet.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.sessions.lock().is_empty()
    }
}
