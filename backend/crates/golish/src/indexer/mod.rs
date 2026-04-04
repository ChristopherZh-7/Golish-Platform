//! Code indexer module - re-exports from golish-indexer crate.
//!
//! This module provides a thin wrapper around the golish-indexer infrastructure crate.
//!
//! # Architecture
//!
//! - **golish-indexer**: Infrastructure crate with indexer state management
//! - **golish/indexer/mod.rs**: Re-exports + Tauri commands

// Tauri commands (stay in main crate due to AppState dependency)
pub mod commands;

// Re-export everything from golish-ai::indexer
pub use golish_ai::indexer::*;

// Re-export commands for Tauri
pub use commands::*;
