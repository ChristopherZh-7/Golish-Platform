//! Sidecar module - re-exports from golish-sidecar crate.
//!
//! This module provides a thin wrapper around the golish-sidecar infrastructure crate.
//!
//! # Architecture
//!
//! - **golish-sidecar**: Infrastructure crate with session management and context capture
//! - **golish/sidecar/mod.rs**: Re-exports + Tauri commands

// Tauri commands (stay in main crate due to AppState dependency).
// Consumers access them via `crate::sidecar::commands::*`.
pub mod commands;

// Re-export everything from golish-sidecar
pub use golish_sidecar::*;
