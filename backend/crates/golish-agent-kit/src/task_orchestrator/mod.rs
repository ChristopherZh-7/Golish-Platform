//! Task Orchestrator — PentAGI-style automated task execution.
//!
//! Implements the full Task mode state machine:
//! 1. **Generator**: Decomposes user input into ordered subtasks
//! 2. **Primary Agent Loop**: Executes each subtask with delegation
//! 3. **Refiner**: After each subtask, adjusts remaining plan
//! 4. **Reporter**: Generates a final task report
//!
//! This module operates at a level above the `AgentBridge`, calling into it
//! for each agent invocation while managing the overall task lifecycle and DB
//! persistence.
//!
//! # Submodules
//!
//! - [`prompts`]: prompt templates used by the orchestrator phases.
//! - [`types`]: planning DTOs, cost tracking, execution context, and the
//!   [`AgentExecutor`] trait.
//! - [`orchestrator`]: [`TaskOrchestrator`] struct + entry points (`run`,
//!   `resume`) + event emission helpers.
//! - [`subtask_phases`]: the heavy execution methods (`execute_subtask_loop`,
//!   `execute_single_subtask`, `refine_remaining`) on a separate `impl`
//!   block.
//! - [`helpers`]: small free functions shared across the phases.
//!
//! `bridge_executor` (the `AgentBridge`-backed implementation of
//! [`AgentExecutor`]) lives in the `golish-ai` umbrella crate because it
//! depends on `agent_bridge`. After A1-3 it will move into
//! `golish-agent-bridge` and remain re-exported as
//! `golish_agent_bridge::bridge_executor` for backward
//! compatibility.

pub mod prompts;

pub mod harness_backfill;
pub mod harness_resume;
mod helpers;
mod orchestrator;
pub mod stage_execution;
mod subtask_phases;
mod types;

pub use harness_backfill::{backfill_harness_stage, infer_harness_stage};
pub use orchestrator::TaskOrchestrator;
pub use types::{
    AgentExecutor, AgentResult, AgentTokenUsage, ExecutionContext, GeneratorOutput, PlannedSubtask,
    RefinerOutput, SubtaskModification, SubtaskResult, TaskCostTracker,
};
