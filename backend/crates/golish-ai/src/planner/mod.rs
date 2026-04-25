//! Plan management for agent task tracking.
//!
//! This module provides a simple planning system that allows the AI agent to
//! create and update multi-step plans. Based on vtcode-core's implementation.
//!
//! # Submodules
//!
//! - [`manager`]: [`PlanManager`] runtime — thread-safe access to a
//!   [`TaskPlan`] with validation, optional PostgreSQL persistence, and
//!   prompt-injection formatting.
//!
//! Core plan types ([`PlanStep`], [`StepStatus`], etc.) live in
//! `golish-core::plan` and are re-exported here for convenience.

use serde::Deserialize;

// Re-export core plan types from golish-core.
pub use golish_core::plan::{
    PlanStep, PlanSummary, StepStatus, TaskPlan, MAX_PLAN_STEPS, MIN_PLAN_STEPS,
};

mod manager;

#[cfg(test)]
mod tests;

pub use manager::PlanManager;

/// Arguments for the update_plan tool.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdatePlanArgs {
    /// Optional explanation/summary of the plan.
    pub explanation: Option<String>,
    /// The plan steps.
    pub plan: Vec<PlanStepInput>,
}

/// Input format for a plan step (from tool arguments).
#[derive(Debug, Clone, Deserialize)]
pub struct PlanStepInput {
    /// Description of the step.
    pub step: String,
    /// Status of the step.
    #[serde(default)]
    pub status: StepStatus,
}

/// Error type for plan validation.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("Plan must have between {MIN_PLAN_STEPS} and {MAX_PLAN_STEPS} steps, got {0}")]
    InvalidStepCount(usize),

    #[error("Step {0} has empty description")]
    EmptyStepDescription(usize),

    #[error("Only one step can be in_progress at a time, found {0}")]
    MultipleInProgress(usize),
}
