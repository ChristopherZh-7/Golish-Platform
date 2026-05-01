use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::pty::PtyManager;
use crate::tools::pty_interactive::PtyOutputTap;

/// Terminal / PTY managed state.
pub struct PtyState {
    pub manager: Arc<PtyManager>,
    pub output_tap: Arc<PtyOutputTap>,
    pub active_session: Arc<Mutex<Option<String>>>,
    /// Terminal sessions currently in use by pentest tool executions.
    pub busy_sessions: Arc<Mutex<HashSet<String>>>,
}

impl PtyState {
    pub fn new() -> Self {
        Self {
            manager: Arc::new(PtyManager::new()),
            output_tap: Arc::new(PtyOutputTap::new()),
            active_session: Arc::new(Mutex::new(None)),
            busy_sessions: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}
