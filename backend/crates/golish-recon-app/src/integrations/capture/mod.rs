//! Credential Capture Engine.
//!
//! Owns per-session isolated WebView windows, drives the capture state
//! machine ([`golish_integrations::types::CaptureState`]), runs each
//! recipe rule against the live page, and persists harvested values
//! through the existing storage backend chain
//! (`IntegrationsState::pick_backend → StorageBackend::write`).
//!
//! Design doc: `docs/design/2026-05-21-credential-capture-engine.md`.
//! Implementation plan: `docs/superpowers/plans/2026-05-21-credential-capture-engine.md`.
//!
//! Phase 2 module layout:
//!   - [`engine`]: the long-lived `CaptureEngine` (Tauri-managed
//!     singleton holding the in-flight session registry).
//!   - [`session`]: in-memory [`CaptureSession`] / [`CaptureSessionHandle`].
//!   - [`data_dir`]: per-session WebView data directory (cookies, local
//!     storage, IndexedDB) creation + cleanup. Per-session isolation
//!     keeps a capture run from poisoning the user's main Golish window.
//!   - [`webview_isolation`]: platform-aware `data_directory` /
//!     `data_store_identifier` selection.

mod data_dir;
mod engine;
mod session;
mod webview_isolation;

pub use engine::CaptureEngine;
pub use session::{CaptureSession, CaptureSessionHandle};
