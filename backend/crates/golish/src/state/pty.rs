use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::pty::PtyManager;
use crate::tools::pty_interactive::PtyOutputTap;

/// Terminal / PTY managed state.
///
/// Managed independently of `AppState` as of A4: PTY-related commands
/// take `State<'_, PtyState>` directly instead of the monolithic
/// `AppState`.
pub struct PtyState {
    pub manager: Arc<PtyManager>,
    pub output_tap: Arc<PtyOutputTap>,
    pub active_session: Arc<Mutex<Option<String>>>,
    pub busy_sessions: Arc<Mutex<HashSet<String>>>,
}

impl PtyState {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            manager: Arc::new(PtyManager::new()),
            output_tap: Arc::new(PtyOutputTap::new()),
            active_session: Arc::new(Mutex::new(None)),
            busy_sessions: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Build from AppState-owned `Arc`s so the two views share the same
    /// underlying data (used by `AppState::extract_pty_state`).
    pub fn from_shared(
        manager: Arc<PtyManager>,
        output_tap: Arc<PtyOutputTap>,
        active_session: Arc<Mutex<Option<String>>>,
        busy_sessions: Arc<Mutex<HashSet<String>>>,
    ) -> Self {
        Self {
            manager,
            output_tap,
            active_session,
            busy_sessions,
        }
    }
}
