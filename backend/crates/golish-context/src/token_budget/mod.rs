//! Token budget tracking: usage stats, per-model context limits, alert
//! thresholds, and a runtime [\].
//!
//! Submodules:
//! - [\]: [\] DTO + tool/context constants.
//! - [\]: [\] (max context tokens per model).
//! - [\]: [\] (rolled-up budget for a (provider,model)).
//! - [\]: [\] + [\] enum.
//! - [\]: [\] runtime.

mod config;
mod limits;
mod manager;
mod stats;
mod usage;

#[cfg(test)]
mod tests;

pub use config::TokenBudgetConfig;
pub use limits::ModelContextLimits;
pub use manager::TokenBudgetManager;
pub use stats::{TokenAlertLevel, TokenUsageStats};
pub use usage::{TokenUsage, DEFAULT_MAX_CONTEXT_TOKENS, MAX_TOOL_RESPONSE_TOKENS};
