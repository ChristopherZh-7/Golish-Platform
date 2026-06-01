//! Application-boundary shared types for Golish app crates.
//!
//! This crate holds the types that every per-domain *app* crate
//! (`golish-vuln-app`, `golish-recon-app`, …) needs in order to define
//! `#[tauri::command]` functions without depending on the monolithic
//! `golish` application crate:
//!
//! - [`GolishError`] — the unified error type returned at the Tauri/CLI
//!   boundary (wraps every domain crate's error type).
//! - [`DbState`] — the narrow managed DB state handle commands receive via
//!   `tauri::State<'_, DbState>`.
//! - [`pty_interactive`] — shared PTY output tap + AI `run_pty_cmd` tool, used
//!   by both the main app's runtime wiring and the pentest app's AI tools.
//! - [`ports`] — provider-side service ports (S1-2): the `VaultReadPort` trait +
//!   `PgVaultAdapter`, shared by platform-staying code and the pentest app.
//! - [`runtime`] — `GolishRuntime` adapters (`TauriRuntime` / `CliRuntime`),
//!   shared by the main app's terminal/CLI wiring and the agent app's
//!   `init_ai_agent` (which constructs a `TauriRuntime`).
//!
//! ## Architecture Principle
//!
//! golish-app-core sits at **L5 (application-shared)** in the crate DAG:
//! above the domain services (L2/L3) whose errors it aggregates, below the
//! per-domain app crates (L5+) and the main `golish` binary (L6). It must
//! **not** depend on `golish` (that would be an upward edge / cycle).
//!
//! Note: the monolithic `AppState` intentionally stays in the `golish` crate
//! because it aggregates golish-internal subsystems (AI / indexer / settings
//! / sidecar / …); app crates take the narrow `DbState` instead.

pub mod domain;
pub mod error;
pub mod event_emitter;
pub mod ports;
pub mod pty_interactive;
pub mod runtime;
pub mod scoping;
pub mod state;

pub use error::{GolishError, IpcError, Result};
pub use event_emitter::TauriEventEmitter;
pub use state::DbState;
