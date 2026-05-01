//! Code indexer state management for Golish.
//!
//! This module provides state management for code indexing functionality:
//! - Index path resolution (global vs local storage)
//! - `IndexerState` with a pluggable `IndexerBackend` trait
//!
//! The concrete indexer backend (e.g. vtcode-indexer's `SimpleIndexer`) is
//! injected at the application layer via [`IndexerState::set_backend`].

pub mod paths;
pub mod state;

pub use paths::{compute_index_dir, find_existing_index_dir, migrate_index};
pub use state::{CodeSearchResult, IndexerBackend, IndexerState};
