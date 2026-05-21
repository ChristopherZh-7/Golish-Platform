//! Per-session WebView storage isolation.
//!
//! Phase 0 spike (see plan §"Phase 0 实际发现汇总") found that
//! `WebviewWindowBuilder::data_directory` is **not supported on macOS
//! WKWebView**. Tauri 2 exposes `data_store_identifier([u8; 16])` on
//! macOS ≥ 14 / iOS ≥ 17 instead. Linux / Windows still use the
//! `data_directory` PathBuf API.
//!
//! This module hides that branch behind a single
//! [`apply_isolation`] entry point so the rest of the engine doesn't
//! need to repeat the `cfg` dance.
//!
//! For non-UUID session ids we derive a stable `[u8; 16]` via
//! `uuid::Uuid::new_v5(NAMESPACE_OID, session_id)` rather than pulling
//! a fresh hashing crate (`blake3`) — the security property required
//! here is "stable per session_id, unique across sessions", which
//! UUID v5 satisfies trivially.

use std::path::Path;

use tauri::{webview::WebviewWindowBuilder, Wry};

/// Applies per-session storage isolation in a platform-aware way.
///
/// The returned builder is otherwise unchanged. Other platforms (e.g.
/// Android / iOS pre-17) become a no-op; the capture engine is expected
/// to refuse to start there from a higher layer.
pub(crate) fn apply_isolation<'a>(
    builder: WebviewWindowBuilder<'a, Wry, tauri::AppHandle<Wry>>,
    _session_id: &str,
    _per_session_dir: &Path,
) -> WebviewWindowBuilder<'a, Wry, tauri::AppHandle<Wry>> {
    #[cfg(target_os = "macos")]
    {
        let bytes = derive_macos_data_store_id(_session_id);
        return builder.data_store_identifier(bytes);
    }
    #[cfg(all(not(target_os = "macos"), not(any(target_os = "android", target_os = "ios"))))]
    {
        return builder.data_directory(_per_session_dir.to_path_buf());
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        // No isolation API available on this target. Capture should be
        // gated upstream; we just return the unchanged builder.
        builder
    }
}

/// Stable `[u8; 16]` derived from a session id. Direct UUID v4 parse
/// shortcut for normal session ids; v5(NAMESPACE_OID, session_id) for
/// anything that doesn't parse as a UUID (e.g. test fixtures).
#[cfg(target_os = "macos")]
fn derive_macos_data_store_id(session_id: &str) -> [u8; 16] {
    if let Ok(u) = uuid::Uuid::parse_str(session_id) {
        return *u.as_bytes();
    }
    let derived = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, session_id.as_bytes());
    *derived.as_bytes()
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_data_store_id_is_stable_per_session_id() {
        use super::derive_macos_data_store_id;
        let sid = "abc-123-test";
        let a = derive_macos_data_store_id(sid);
        let b = derive_macos_data_store_id(sid);
        assert_eq!(a, b, "must be deterministic");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_data_store_id_differs_per_session_id() {
        use super::derive_macos_data_store_id;
        let a = derive_macos_data_store_id("session-a");
        let b = derive_macos_data_store_id("session-b");
        assert_ne!(a, b, "different inputs → different outputs");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_data_store_id_for_uuid_input_round_trips() {
        use super::derive_macos_data_store_id;
        let u = uuid::Uuid::new_v4();
        let bytes = derive_macos_data_store_id(&u.to_string());
        assert_eq!(bytes, *u.as_bytes(), "uuid input is identity-mapped");
    }
}
