use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::pty::PtyManager;
use crate::tools::pty_interactive::PtyOutputTap;

/// Terminal / PTY managed state.
///
/// Reserved for Tauri `manage<PtyState>()` migration (P2-2).
#[allow(dead_code)]
pub struct PtyState {
    pub manager: Arc<PtyManager>,
    pub output_tap: Arc<PtyOutputTap>,
    pub active_session: Arc<Mutex<Option<String>>>,
    pub busy_sessions: Arc<Mutex<HashSet<String>>>,
}

#[allow(dead_code)]
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
