//! PTY module - re-exports from golish-pty crate.
//!
//! This module provides a thin wrapper around the golish-pty infrastructure crate.
//!
//! # Architecture
//!
//! - **golish-pty**: Infrastructure crate with PTY management system
//! - **golish/pty/mod.rs**: Re-exports for compatibility

// Re-export everything from golish-pty
pub use golish_pty::*;
