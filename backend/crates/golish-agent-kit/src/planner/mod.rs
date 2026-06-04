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

use serde::{Deserialize, Serialize};

// Re-export core plan types from golish-core.
pub use golish_core::plan::{
    FailureKind, PlanStep, PlanSummary, StepStatus, TaskPlan, MAX_PLAN_STEPS, MIN_PLAN_STEPS,
};

mod manager;

#[cfg(test)]
mod tests;

pub use manager::PlanManager;

use std::sync::Arc;

/// Trait for broadcasting plan changes from [`PlanManager`].
///
/// The planner crate intentionally does not depend on `golish-events` or
/// `golish-core::events`; higher layers wrap a concrete event channel into
/// this trait and inject it via [`PlanManager::set_event_emitter`].
///
/// Currently used to broadcast `PlanUpdated` events when a plan is restored
/// from the database on session start so the frontend sees the restored
/// plan without waiting for the next LLM-driven `update_plan` call.
pub trait PlanEventEmitter: Send + Sync + 'static {
    fn emit_plan_updated(
        &self,
        version: u32,
        summary: PlanSummary,
        steps: Vec<PlanStep>,
        explanation: Option<String>,
        stage_id: Option<String>,
    );
}

/// Shared alias for `Arc<dyn PlanEventEmitter>` to keep call sites concise.
pub type SharedPlanEventEmitter = Arc<dyn PlanEventEmitter>;

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

/// Patch operation applied to the current plan by `apply_patch_ops`.
///
/// Mirrors the PentAGI refiner `subtask_patch_tool` shape but kept
/// internal for now — no LLM tool consumes this directly yet (P0-2
/// stage 2 implementation; tool exposure deferred to P1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PlanPatchOp {
    /// Insert a new step. `after_id` = None means insert at the head.
    /// If `after_id` doesn't match any existing step the new step lands
    /// at the **end** to preserve the LLM's intent.
    Add {
        #[serde(default)]
        after_id: Option<String>,
        title: String,
        #[serde(default)]
        status: Option<StepStatus>,
    },
    /// Remove a step by stable id. Missing id is a no-op.
    Remove { id: String },
    /// Modify a step in place. Each optional field overrides only if
    /// present. Missing id is a no-op.
    Modify {
        id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        status: Option<StepStatus>,
        #[serde(default)]
        failure_kind: Option<FailureKind>,
    },
    /// Move a step to a new position. `after_id` = None means the head.
    Reorder {
        id: String,
        #[serde(default)]
        after_id: Option<String>,
    },
}
