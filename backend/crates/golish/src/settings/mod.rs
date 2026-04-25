//! Settings module - re-exports from golish-settings crate.
//!
//! This module provides a thin wrapper around the golish-settings infrastructure crate,
//! adding Tauri-specific commands for the GUI application.
//!
//! # Architecture
//!
//! - **golish-settings**: Infrastructure crate with core settings logic
//! - **golish/settings/commands.rs**: Tauri commands (stays in main crate to avoid AppState circular dependency)
//! - **golish/settings/mod.rs**: Re-exports and command registration

// Tauri commands (stay in main crate due to AppState dependency).
// Consumers access them via `crate::settings::commands::*`.
pub mod commands;

// Re-export everything from golish-settings
pub use golish_settings::*;
