//! Per-session data directory management.
//!
//! Each capture-enabled `(tool_id, group_id)` gets an isolated
//! WebKit/WebView profile so cookies, localStorage, IndexedDB etc.
//! persist across Golish restarts without leaking into the main Golish
//! window.
//!
//! Lifetime:
//!   - [`profile_dir`] creates the dir on demand at webview-build time.
//!   - [`cleanup_profile_dir`] is called only by the explicit
//!     "clear capture login state" IPC command.
//!
//! Layout:
//!   macOS:   `~/Library/Application Support/com.golish.platform/capture-profiles/<tool>__<group>/`
//!   Linux:   `~/.local/share/com.golish.platform/capture-profiles/<tool>__<group>/`
//!   Windows: `%APPDATA%/com.golish.platform/capture-profiles/<tool>__<group>/`
//!
//! On macOS the dir is only used for non-WKWebView state (we use
//! `data_store_identifier` for the actual cookie store — see
//! [`super::webview_isolation`]); it still gets created so cleanup
//! semantics are uniform across platforms.

use std::path::PathBuf;

use golish_integrations::error::{IntegrationError, IntegrationResult};

const APP_DIR: &str = "com.golish.platform";
const CAPTURE_SUB: &str = "capture-sessions";
const PROFILE_SUB: &str = "capture-profiles";

/// Returns (and lazily creates) the parent dir under which every
/// per-session capture data dir lives.
pub(crate) fn capture_root() -> IntegrationResult<PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| IntegrationError::WebviewCreateFailed("no platform data_dir".into()))?
        .join(APP_DIR)
        .join(CAPTURE_SUB);
    std::fs::create_dir_all(&base).map_err(|e| {
        IntegrationError::WebviewCreateFailed(format!("mkdir capture root {}: {e}", base.display()))
    })?;
    Ok(base)
}

pub(crate) fn profile_root() -> IntegrationResult<PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| IntegrationError::WebviewCreateFailed("no platform data_dir".into()))?
        .join(APP_DIR)
        .join(PROFILE_SUB);
    std::fs::create_dir_all(&base).map_err(|e| {
        IntegrationError::WebviewCreateFailed(format!(
            "mkdir capture profile root {}: {e}",
            base.display()
        ))
    })?;
    Ok(base)
}

/// Returns the per-session data dir, creating it if missing.
#[allow(dead_code)]
pub(crate) fn session_dir(session_id: &str) -> IntegrationResult<PathBuf> {
    let dir = capture_root()?.join(session_id);
    std::fs::create_dir_all(&dir).map_err(|e| {
        IntegrationError::WebviewCreateFailed(format!("mkdir session dir {}: {e}", dir.display()))
    })?;
    Ok(dir)
}

pub(crate) fn profile_key(tool_id: &str, group_id: &str) -> String {
    format!(
        "{}__{}",
        sanitize_segment(tool_id),
        sanitize_segment(group_id)
    )
}

pub(crate) fn profile_dir(tool_id: &str, group_id: &str) -> IntegrationResult<PathBuf> {
    let dir = profile_root()?.join(profile_key(tool_id, group_id));
    std::fs::create_dir_all(&dir).map_err(|e| {
        IntegrationError::WebviewCreateFailed(format!("mkdir profile dir {}: {e}", dir.display()))
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

pub(crate) fn cleanup_profile_dir(tool_id: &str, group_id: &str) {
    let Ok(root) = profile_root() else {
        return;
    };
    let dir = root.join(profile_key(tool_id, group_id));
    if !dir.exists() {
        return;
    }
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        tracing::warn!(
            tool_id = %tool_id,
            group_id = %group_id,
            dir = %dir.display(),
            error = %e,
            "capture: failed to cleanup profile dir"
        );
    }
}

fn sanitize_segment(input: &str) -> String {
    let mut out = String::new();
    let mut last_was_dash = false;
    for ch in input.chars() {
        let next = if ch.is_ascii_alphanumeric() {
            last_was_dash = false;
            Some(ch.to_ascii_lowercase())
        } else if !last_was_dash {
            last_was_dash = true;
            Some('-')
        } else {
            None
        };
        if let Some(ch) = next {
            out.push(ch);
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed
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

    #[test]
    fn profile_key_is_stable_and_path_safe() {
        let key = profile_key("ENScan_GO", "aqc/cookies");
        assert_eq!(key, "enscan-go__aqc-cookies");
        assert_eq!(key, profile_key("enscan go", "aqc cookies"));
    }

    #[test]
    fn profile_dir_is_stable_for_tool_group() {
        let d1 = profile_dir("enscan-go", "aqc").expect("first profile dir");
        let d2 = profile_dir("enscan-go", "aqc").expect("second profile dir");
        assert_eq!(d1, d2);
        assert!(d1.ends_with("enscan-go__aqc"));
        cleanup_profile_dir("enscan-go", "aqc");
    }
}
