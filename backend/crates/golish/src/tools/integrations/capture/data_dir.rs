//! Per-session data directory management.
//!
//! Each capture session gets an isolated WebKit/WebView data dir so
//! cookies, localStorage, IndexedDB etc. never leak between sessions
//! or into the main Golish window.
//!
//! Lifetime:
//!   - [`session_dir`] creates the dir on demand at webview-build time.
//!   - [`cleanup_session_dir`] is called on terminal-state transition.
//!     Failures are logged but never bubble (cleanup mustn't mask the
//!     real outcome of a capture session).
//!
//! Layout:
//!   macOS:   `~/Library/Application Support/com.golish.platform/capture-sessions/<session_id>/`
//!   Linux:   `~/.local/share/com.golish.platform/capture-sessions/<session_id>/`
//!   Windows: `%APPDATA%/com.golish.platform/capture-sessions/<session_id>/`
//!
//! On macOS the dir is only used for non-WKWebView state (we use
//! `data_store_identifier` for the actual cookie store — see
//! [`super::webview_isolation`]); it still gets created so cleanup
//! semantics are uniform across platforms.

use std::path::PathBuf;

use golish_integrations::error::{IntegrationError, IntegrationResult};

const APP_DIR: &str = "com.golish.platform";
const CAPTURE_SUB: &str = "capture-sessions";

/// Returns (and lazily creates) the parent dir under which every
/// per-session capture data dir lives.
pub(crate) fn capture_root() -> IntegrationResult<PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| IntegrationError::WebviewCreateFailed("no platform data_dir".into()))?
        .join(APP_DIR)
        .join(CAPTURE_SUB);
    std::fs::create_dir_all(&base).map_err(|e| {
        IntegrationError::WebviewCreateFailed(format!(
            "mkdir capture root {}: {e}",
            base.display()
        ))
    })?;
    Ok(base)
}

/// Returns the per-session data dir, creating it if missing.
pub(crate) fn session_dir(session_id: &str) -> IntegrationResult<PathBuf> {
    let dir = capture_root()?.join(session_id);
    std::fs::create_dir_all(&dir).map_err(|e| {
        IntegrationError::WebviewCreateFailed(format!("mkdir session dir {}: {e}", dir.display()))
    })?;
    Ok(dir)
}

/// Best-effort recursive delete. Logs warnings on failure but never
/// panics — we run this from terminal-state transitions where a
/// transient FS error must not mask the capture outcome.
pub(crate) fn cleanup_session_dir(session_id: &str) {
    let Ok(root) = capture_root() else {
        return;
    };
    let dir = root.join(session_id);
    if !dir.exists() {
        return;
    }
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        tracing::warn!(
            session_id = %session_id,
            dir = %dir.display(),
            error = %e,
            "capture: failed to cleanup session data dir"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_missing_dir_is_noop() {
        cleanup_session_dir("nonexistent-session-uuid-aaaa-bbbb-cccc-dddd-eeee-ffff");
    }

    #[test]
    fn session_dir_creates_then_cleans() {
        let sid = format!("test-{}", uuid::Uuid::new_v4());
        let dir = session_dir(&sid).expect("session_dir should create");
        assert!(dir.exists(), "session dir should exist after create");
        cleanup_session_dir(&sid);
        assert!(!dir.exists(), "session dir should be gone after cleanup");
    }

    #[test]
    fn session_dir_is_idempotent() {
        let sid = format!("test-{}", uuid::Uuid::new_v4());
        let d1 = session_dir(&sid).expect("first call");
        let d2 = session_dir(&sid).expect("second call");
        assert_eq!(d1, d2);
        cleanup_session_dir(&sid);
    }
}
