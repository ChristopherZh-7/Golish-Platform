//! Application bootstrap & lifecycle helpers extracted from `lib.rs::run_gui`.
//!
//! These modules contain **no** new logic — they are a mechanical
//! decomposition of the previous monolithic `run_gui` function so that:
//!
//! * command registration in `lib.rs` is easier to maintain,
//! * individual startup phases (telemetry, DB, sidecar, MCP, window state,
//!   menu) can be read / tested independently,
//! * CLI/headless callers can reuse the bootstrap helpers without pulling in
//!   the Tauri builder.

pub(crate) mod bootstrap;
pub(crate) mod mcp_bootstrap;
pub(crate) mod menu;
pub(crate) mod sidecar_bootstrap;
pub(crate) mod tauri_app;
pub(crate) mod window_lifecycle;
pub(crate) mod workspace;
