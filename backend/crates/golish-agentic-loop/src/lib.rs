//! High-level agentic loop runtime, split out from `golish-agent-loop` to
//! finish P3-1 (compile-time budget).
//!
//! # Why a separate crate?
//!
//! `golish-agent-loop` originally hosted both:
//! 1. **Low-level building blocks** — tool executors, tool definitions,
//!    HITL recorder, loop detector, tool policy, sidecar trait, system
//!    hooks, planner, in-memory db tracking, llm-client wiring, …
//! 2. **The high-level streaming loop** — `agentic_loop/` (~6.5 KLOC) plus
//!    its eval harness (`eval_support/`) and mocks (`test_utils*`).
//!
//! Touching anything in (2) used to recompile the whole package
//! (~13 KLOC + heavy generic instantiations from rig-core), missing the
//! 8s `cargo check` budget recorded in the architecture roadmap. Splitting
//! (2) into this crate restores incremental editing of the loop body
//! without re-checking the lower layer.
//!
//! # Layering
//!
//! ```text
//!                     ┌────────────────────────────┐
//!                     │  golish-agentic-loop (this)│
//!                     │  agentic_loop / eval_support│
//!                     │  test_utils                 │
//!                     └────────────┬───────────────┘
//!                                  │ depends on
//!                                  ▼
//!                     ┌────────────────────────────┐
//!                     │  golish-agent-loop          │
//!                     │  tool_executors / tool_*    │
//!                     │  hitl / loop_detection / …  │
//!                     └────────────────────────────┘
//! ```
//!
//! Down-stream consumers (`golish-agent-bridge`, `golish-ai`, evals) keep
//! the original import paths via re-exports in their facades.

pub mod agentic_loop;
pub mod eval_support;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

// Re-export the most frequently used loop entry points so existing
// `crate::agentic_loop::run_agentic_loop_unified` / `_generic` style imports
// keep working when the umbrella crates re-export `agentic_loop` from here.
pub use agentic_loop::{
    apply_compaction, get_artifacts_dir, get_artifacts_dir_for, get_summaries_dir,
    get_summaries_dir_for, get_transcript_dir, get_transcript_dir_for, maybe_compact,
    run_agentic_loop, run_agentic_loop_generic, run_agentic_loop_unified, AgenticLoopConfig,
    AgenticLoopContext, CompactionResult, LoopAccessControl, LoopCaptureContext, LoopEventRefs,
    LoopLlmRefs, McpToolExecutor, OutputClassifier, PostShellHook, TerminalErrorEmitted,
    ToolExecutionResult,
};
