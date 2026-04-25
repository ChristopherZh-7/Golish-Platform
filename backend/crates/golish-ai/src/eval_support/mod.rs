//! Evaluation support for running the unified agentic loop in test/eval contexts.
//!
//! This module provides simplified entry points for evaluations to use the
//! same agentic loop as the main application, ensuring evals test actual
//! behavior.
//!
//! # Submodules
//!
//! - [`types`]: small DTOs ([`EvalToolCall`], [`EvalAgentOutput`],
//!   [`EvalConfig`]).
//! - [`extractors`]: post-process the captured event stream into structured
//!   tool calls + file lists, plus a verbose human-readable printer.
//! - [`single_turn`]: [`run_eval_agentic_loop`] and the tool-augmented
//!   variant [`run_eval_agentic_loop_with_tools`].
//! - [`multi_turn`]: [`run_multi_turn_eval`] which threads a conversation
//!   history across multiple user prompts.

mod extractors;
mod multi_turn;
mod single_turn;
mod types;

#[cfg(test)]
mod tests;

pub use multi_turn::{run_multi_turn_eval, MultiTurnEvalOutput};
pub use single_turn::{
    run_eval_agentic_loop, run_eval_agentic_loop_with_tools, CustomToolExecutor,
};
pub use types::{EvalAgentOutput, EvalConfig, EvalToolCall};
