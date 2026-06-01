//! Per-domain crate for the **agent** service (crate-per-service M4).
//!
//! M4-A (this step) holds the agent runtime state only:
//! - [`AiState`] — per-session agent bridges, moved out of the god-crate so the
//!   agent command surface can later move here without a dependency cycle.
//! - [`AgentState`] — the narrow managed state agent Tauri commands take instead
//!   of the monolithic `golish::AppState`.
//!
//! M4-proper (this step) additionally moved the `ai/` subtree — the
//! `ai/commands/*` Tauri handlers + the AppState-free agent bridges
//! (`db_bridge` / `tracking_bridge` / `session_bridge` / `graph_bridge` /
//! `embedder_bridge` / `sidecar_bridge`) + the `ai/mod.rs` facade — plus the
//! agent-owned `conversation_store` into this crate. The main `golish` crate
//! keeps thin `crate::ai` / `crate::tools::conversation_store` shims that
//! re-export from here, and routes the command surface through
//! `commands_facade::{ai, workspace}` like the other app crates.

// `too_many_arguments` is intentionally allowed crate-wide: the moved agent
// `#[tauri::command]` handlers (e.g. `agents.rs`) thread `AppHandle`, multiple
// `State` handles and many optional request fields straight from the frontend.
// Mirrors the same crate-level allow in `golish` and `golish-pentest-app`.
#![allow(clippy::too_many_arguments)]

pub mod ai;
pub mod conversation_store;
pub mod state;

// Re-export app-core's `error` module + `runtime` adapters at the crate root so
// the moved `ai/` files keep resolving `crate::error::*` / `crate::runtime::*`
// (GolishError + TauriRuntime live in golish-app-core) without editing each file.
pub use golish_app_core::{error, runtime};

pub use state::{ai_session_not_initialized_error, AgentState, AiState};
