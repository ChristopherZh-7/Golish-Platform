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
//! - [`bridge_executor`] / [`prompts`]: pre-existing companion modules wiring
//!   the orchestrator to a real `AgentBridge` and the prompt templates.
//! - [`types`]: planning DTOs, cost tracking, execution context, and the
//!   [`AgentExecutor`] trait.
//! - [`orchestrator`]: [`TaskOrchestrator`] struct + entry points (`run`,
//!   `resume`) + event emission helpers.
//! - [`subtask_phases`]: the heavy execution methods (`execute_subtask_loop`,
//!   `execute_single_subtask`, `refine_remaining`) on a separate `impl`
//!   block.
//! - [`helpers`]: small free functions shared across the phases.

pub mod bridge_executor;
pub mod prompts;

mod helpers;
mod orchestrator;
mod subtask_phases;
mod types;

pub use orchestrator::TaskOrchestrator;
pub use types::{
    AgentExecutor, AgentResult, AgentTokenUsage, ExecutionContext, GeneratorOutput,
    PlannedSubtask, RefinerOutput, SubtaskModification, SubtaskResult, TaskCostTracker,
};
