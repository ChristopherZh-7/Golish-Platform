//! Code indexer module - re-exports from golish-indexer crate.
//!
//! This module provides a thin wrapper around the golish-indexer infrastructure crate.
//!
//! # Architecture
//!
//! - **golish-indexer**: Infrastructure crate with indexer state management
//! - **golish/indexer/mod.rs**: Re-exports + Tauri commands

// Tauri commands (stay in main crate due to AppState dependency).
// Consumers access them via `crate::indexer::commands::*`; we don't re-export
// at this level so command names don't shadow `golish_ai::indexer::*` types
// in `crate::indexer::*` lookups.
pub mod commands;

// Re-export everything from golish-ai::indexer
pub use golish_ai::indexer::*;
