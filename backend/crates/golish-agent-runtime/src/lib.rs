//! High-level agent runtime (**Layer 4b**).
//!
//! Renamed from `golish-agentic-loop` in A2. Split out from
//! `golish-agent-kit` (Layer 4a) to keep the compile-time budget honest:
//! the streaming loop body is ~6.5 KLOC with heavy generic
//! instantiations from rig-core, so editing it used to re-check the
//! whole infrastructure layer (~13 KLOC). Separating the two crates
//! restores incremental editing.
//!
//! # What lives here
//!
//! - [`agentic_loop`]   — streaming tool-call loop (`run_agentic_loop*`)
//! - [`eval_support`]   — evals harness built on top of the loop
//! - [`test_utils`]     — shared mocks (feature `test-utils` or `cfg(test)`)
//!
//! # Layering
//!
//! ```text
//!   golish-agent-runtime (this, L4b)   ← run_agentic_loop_unified, eval harness
//!              │
//!              ▼ depends on
//!   golish-agent-kit            (L4a)  ← tool_executors, hitl, planner, tool_policy
//! ```
//!
//! Down-stream consumers (`golish-agent-bridge`, `golish-ai` umbrella,
//! evals) import from here directly.

pub mod agentic_loop;
pub mod eval_support;
pub mod execution_mode;

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
