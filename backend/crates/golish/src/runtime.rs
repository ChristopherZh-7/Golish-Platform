//! Runtime adapters — re-exported from `golish-app-core`.
//!
//! `TauriRuntime` / `CliRuntime` (impls of `golish_core::GolishRuntime`) were
//! sunk into `golish-app-core` (crate-per-service M4-proper) so the agent
//! command surface in `golish-agent-app` can construct a `TauriRuntime`
//! without depending on the monolithic `golish` crate. This shim keeps the
//! historical `crate::runtime::*` paths (terminal PTY wiring + CLI) working.
pub use golish_app_core::runtime::*;
