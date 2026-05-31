//! Per-domain Tauri command crate for the **platform** service.
//!
//! Holds the `#[tauri::command]` wrappers for the cross-cutting platform
//! services that back the workspace UI:
//! - [`vault`] — credential vault (R7-aligned secrets store, encrypted at rest).
//! - [`audit`] — audit log + cross-service activity timeline.
//! - [`notes`] — quick project notes.
//! - [`recordings`] — terminal session recordings.
//!
//! This is the final leaf of the crate-per-service split (M5). Every command
//! takes the narrow [`golish_app_core::DbState`], never the monolithic
//! `golish::AppState`, so the crate sits below the main `golish` binary. The
//! `golish` aggregate `generate_handler!` reaches these commands via the
//! `commands_facade::{vault, workspace}` re-exports, which glob the `__cmd__$name`
//! macros re-exported from the modules below.
//!
//! ## Boundary
//!
//! Cross-service reads in `audit.rs` (`passive_scans` = recon, `agent_logs` /
//! `search_logs` = agent) go through the shared `golish_db::repo::*` layer (L2),
//! not sibling app crates, so platform-app has **zero sibling dependency** and
//! stays a clean leaf. Those reads remain guarded by `check_repo_ownership.py`'s
//! ALLOWLIST (layer A); cutting them to ports (`AgentLogReadPort` + recon
//! `passive_scans` port) is deferred to a later port milestone (layer B).

// `too_many_arguments` is intentionally allowed crate-wide: the platform
// `#[tauri::command]` handlers (e.g. `vault_add`) thread `State` + many optional
// request fields straight from the frontend. Mirrors the same crate-level allow
// in `golish`, `golish-pentest-app` and `golish-agent-app`.
#![allow(clippy::too_many_arguments)]

pub mod audit;
pub mod notes;
pub mod recordings;
pub mod vault;
