#![allow(dead_code)] // Artifact system implemented but not yet integrated
//! L3: Project Artifacts
//!
//! Auto-maintained project documentation (README.md, CLAUDE.md) based on session activity.
//! Proposes updates that users review and apply.

pub mod prompts;
pub mod synthesis;
pub mod manager;
pub mod generators;

pub use prompts::*;
pub use synthesis::*;
pub use manager::*;
pub use generators::*;

#[cfg(test)]
mod tests;
