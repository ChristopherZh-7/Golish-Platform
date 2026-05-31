//! Tools module - re-exports from golish-tools crate.
//!
//! This module provides a thin wrapper around the golish-tools infrastructure crate.
//!
//! # Architecture
//!
//! - **golish-tools**: Infrastructure crate with tool execution system
//! - **golish/tools/mod.rs**: Re-exports for compatibility

// Re-export everything from golish-tools
pub use golish_tools::*;

// NB: the project-scoping (IDOR / ownership) guards live in golish-app-core (L5);
// the last golish-staying consumers (vault / notes) moved to golish-platform-app
// (crate-per-service M5), so the historical `crate::tools::scoping` re-export was
// dropped. The per-domain app crates import `golish_app_core::scoping` directly.

// Penetration testing service (tool mgmt / AI bridge / pipelines / findings /
// methodology / evidence / security analysis) extracted to the
// `golish-pentest-app` crate (crate-per-service split M3). Commands reach the
// aggregate `generate_handler!` via `commands_facade::{pentest,workspace,
// findings,evidence,pipeline}`. golish-staying `ai/` + startup wiring still call
// the pentest module at compile time (layer A), so it is re-exported here.
pub(crate) use golish_pentest_app::pentest;

// Wiki / knowledge-base storage — extracted to the `golish-vuln-app` crate
// (crate-per-service split M1b). Commands reach the aggregate
// `generate_handler!` via `commands_facade::wiki`.

// Recon commands live in the `golish-recon-app` crate (M2); pentest commands in
// `golish-pentest-app` (M3). Both reach the aggregate `generate_handler!` via the
// `commands_facade` re-exports. After M3 moved pipeline/storage out, no
// golish-staying module reaches recon's `targets` at compile time anymore.

// Platform commands (vault / audit / notes / recordings) live in the
// `golish-platform-app` crate (M5); they reach the aggregate `generate_handler!`
// via the `commands_facade::{vault, workspace}` re-exports.

// Project export/import
pub mod project_io;

// Interactive PTY tool (allows AI to control visible terminal sessions).
// Sunk into golish-app-core (L5) so the pentest app crate's AI tools and the
// main app's runtime wiring share one copy; re-exported here so golish-staying
// callers keep using `crate::tools::pty_interactive::*`.
pub(crate) use golish_app_core::pty_interactive;

// Vulnerability intelligence — extracted to the `golish-vuln-app` crate
// (crate-per-service split M1). Commands are re-exported to the aggregate
// `generate_handler!` via `commands_facade::vuln_intel`.

// Frontend conversation & timeline persistence (replaces workspace.json) —
// extracted to golish-agent-app (agent-owned `conversation_store` table,
// crate-per-service M4-proper); re-exported so `commands_facade::workspace`
// keeps feeding its commands to `generate_handler!`.
pub(crate) use golish_agent_app::conversation_store;
