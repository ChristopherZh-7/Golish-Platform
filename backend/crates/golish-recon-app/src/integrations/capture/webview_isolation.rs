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
//! We derive a stable `[u8; 16]` via
//! `uuid::Uuid::new_v5(NAMESPACE_OID, profile_key)` rather than pulling
//! a fresh hashing crate (`blake3`) — the security property required
//! here is "stable per capture profile, unique across profiles", which
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
    _profile_key: &str,
    _profile_dir: &Path,
) -> WebviewWindowBuilder<'a, Wry, tauri::AppHandle<Wry>> {
    #[cfg(target_os = "macos")]
    {
        let bytes = derive_macos_data_store_id(_profile_key);
        builder.data_store_identifier(bytes)
    }
    #[cfg(all(
        not(target_os = "macos"),
        not(any(target_os = "android", target_os = "ios"))
    ))]
    {
        builder.data_directory(_profile_dir.to_path_buf())
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        // No isolation API available on this target. Capture should be
        // gated upstream; we just return the unchanged builder.
        builder
    }
}

/// Stable `[u8; 16]` derived from the capture profile key.
#[cfg(target_os = "macos")]
fn derive_macos_data_store_id(profile_key: &str) -> [u8; 16] {
    let derived = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, profile_key.as_bytes());
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
    fn macos_data_store_id_uses_profile_key_not_session_uuid_identity() {
        use super::derive_macos_data_store_id;
        let u = uuid::Uuid::new_v4();
        let bytes = derive_macos_data_store_id(&u.to_string());
        assert_ne!(
            bytes,
            *u.as_bytes(),
            "profile storage must derive from the stable profile key, not reuse per-session UUID bytes"
        );
    }
}
