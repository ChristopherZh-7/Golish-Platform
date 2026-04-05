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

// Penetration testing tool management (ported from Golish)
pub mod pentest;

// Pentest AI tools (expose installed pentest tools to the AI agent)
pub mod pentest_ai;
