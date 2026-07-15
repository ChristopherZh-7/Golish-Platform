//! Task Orchestrator — harness-driven automated task execution.
//!
//! A Task = one operation driven by the metalcraft Executor over the profile-
//! projected Operation DAG: each stage self-plans + dispatches specialists,
//! submits a StageDeliverable, and passes a deterministic evidence gate before
//! the graph advances (with human approval at 大阶段 boundaries). A final
//! reporter summarizes the run.
//!
//! This module operates at a level above the `AgentBridge`, calling into it
//! for each agent invocation while managing the overall task lifecycle and DB
//! persistence.
//!
//! # Submodules
//!
//! - [`prompts`]: prompt templates used by the orchestrator phases.
//! - [`types`]: planning DTO, token usage, execution context, and the
//!   [`AgentExecutor`] trait.
//! - [`orchestrator`]: [`TaskOrchestrator`] struct + entry point (`run`) +
//!   event emission helpers.
//! - [`subtask_phases`]: the Executor-driven operation loop +
//!   `execute_single_subtask` (per-stage agentic loop + gate) on a separate
//!   `impl` block.
//! - [`helpers`]: small free functions shared across the phases.
//!
//! `bridge_executor` (the `AgentBridge`-backed implementation of
//! [`AgentExecutor`]) lives in the `golish-ai` umbrella crate because it
//! depends on `agent_bridge`. After A1-3 it will move into
//! `golish-agent-bridge` and remain re-exported as
//! `golish_agent_bridge::bridge_executor` for backward
//! compatibility.

pub mod prompts;

mod active_recon_scope;
pub mod agent_run_checkpoint;
pub mod continuity;
pub mod harness_backfill;
pub mod harness_resume;
mod helpers;
mod orchestrator;
pub(crate) mod refiner;
pub mod runtime_supervisor;
pub mod stage_execution;
pub mod stage_refiner;
mod subtask_phases;
mod types;

pub use continuity::build_existing_db_continuity_plan;
pub use harness_backfill::{backfill_harness_stage, infer_harness_stage};
pub use orchestrator::TaskOrchestrator;
pub use types::{
    AgentExecutor, AgentResult, AgentTokenUsage, ExecutionContext, PlannedSubtask, SubtaskResult,
};
