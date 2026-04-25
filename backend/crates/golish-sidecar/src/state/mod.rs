//! Sidecar state subsystem.
//!
//! - [`SidecarState`] — owns the runtime state of the sidecar processor +
//!   active session tracking + Tauri event emission.
//! - [`SidecarStatus`] — public snapshot for the UI.
//!
//! Methods are split across [`lifecycle`] (new/init/status/shutdown/config
//! getter/setter, app handle, event emission) and [`sessions`] (start/resume/
//! end/list/find/capture/context retrieval).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::RwLock;

use tauri::AppHandle;

use super::config::SidecarConfig;
use super::processor::Processor;

/// Status of the sidecar system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarStatus {
    /// Whether a session is currently active
    pub active_session: bool,
    /// Current session ID if any
    pub session_id: Option<String>,
    /// Whether the sidecar is enabled
    pub enabled: bool,
    /// Sessions directory path
    pub sessions_dir: PathBuf,
    /// Workspace path (cwd of current session)
    pub workspace_path: Option<PathBuf>,
}

/// Internal state for active session tracking
#[derive(Default)]
pub(super) struct InternalState {
    /// Current session ID
    pub(super) current_session_id: Option<String>,
    /// Current workspace path
    pub(super) workspace_path: Option<PathBuf>,
    /// Whether initialized
    pub(super) initialized: bool,
}

/// Main sidecar state manager
pub struct SidecarState {
    /// Configuration
    pub(super) config: RwLock<SidecarConfig>,
    /// Internal state
    pub(super) state: RwLock<InternalState>,
    /// Event processor
    pub(super) processor: RwLock<Option<Processor>>,
    /// Tauri app handle for emitting events
    pub(super) app_handle: RwLock<Option<AppHandle>>,
}

impl Default for SidecarState {
    fn default() -> Self {
        Self::new()
    }
}

mod lifecycle;
mod sessions;

#[cfg(test)]
mod tests;
