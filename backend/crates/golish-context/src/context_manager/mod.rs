//! Context-window management orchestration.
//!
//! Coordinates token budgeting, context compaction, and truncation
//! strategies. Layout:
//!
//! - [`config`]: trim policy + high-level settings facade.
//! - [`state`]: compaction state machine + decision result type.
//! - [`events`]: alert events, efficiency metrics, summary, warning info,
//!   enforcement-step result envelope.
//! - [`manager`]: the [`ContextManager`] type plus the chars-estimation
//!   helper.
//!
//! Public API for future use — not all methods are currently called.
#![allow(dead_code)]

mod config;
mod events;
mod manager;
mod state;

#[cfg(test)]
mod tests;

pub use config::{ContextManagerConfig, ContextTrimConfig};
pub use events::{
    ContextEfficiency, ContextEnforcementResult, ContextEvent, ContextSummary, ContextWarningInfo,
};
pub use manager::ContextManager;
pub use state::{CompactionCheck, CompactionState};
