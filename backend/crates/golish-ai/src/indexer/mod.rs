//! Code indexer state management for Qbit.
//!
//! This crate provides state management for code indexing functionality, including:
//! - Index path resolution (global vs local storage)
//! - IndexerState management with vtcode-indexer integration
//! - Tree-sitter based code analysis via vtcode-core
//!
//! # Architecture
//!
//! This is a **Layer 2 (Infrastructure)** crate:
//! - Depends on: golish-settings (for IndexLocation config)
//! - Used by: golish (main application via Tauri commands)
//!
//! # Usage
//!
//! ```rust,ignore
//! use golish_indexer::IndexerState;
//! use golish_settings::schema::IndexLocation;
//!
//! // Create indexer state with workspace path
//! let indexer = IndexerState::new(
//!     workspace_path,
//!     IndexLocation::Global
//! );
//!
//! // Access the indexer
//! let guard = indexer.indexer.read();
//! ```

pub mod paths;
pub mod state;

pub use paths::{compute_index_dir, find_existing_index_dir, migrate_index};
pub use state::IndexerState;
