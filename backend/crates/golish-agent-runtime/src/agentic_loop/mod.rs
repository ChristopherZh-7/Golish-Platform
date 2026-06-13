//! Agentic tool loop for LLM execution.
//!
//! This module is the public entry to the per-turn state machine. The
//! actual phase scheduler lives in [`turn::run_turn_loop`]; the
//! sub-modules here host the helpers and tool integrations that the
//! phases call into.
//!
//! High-level flow:
//! - Tool execution with HITL approval
//! - Loop detection and prevention
//! - Context-window management
//! - Message-history management
//! - Extended thinking (streaming reasoning content)
//!
//! This file is the thin public surface (≤ 150 LOC) of the agentic loop;
//! the phase-by-phase implementation lives in `turn::*`.

use anyhow::Result;
use rig::completion::Message;

#[cfg(test)]
#[allow(unused_imports)]
use {
    crate::agentic_loop::sub_agent_dispatch::{detect_repetitive_text, partition_tool_calls},
    rig::completion::AssistantContent,
    rig::message::{Text, ToolCall, ToolResult, ToolResultContent, UserContent},
    rig::one_or_many::OneOrMany,
    serde_json::json,
    std::sync::Arc,
};

// Re-imported here so sibling sub-modules (`context`, `single_tool_call`)
// can reach them via `super::...`. Production code paths.
use golish_agent_kit::tool_definitions::ToolSelectionConfig;
use golish_agent_kit::tool_executors::normalize_run_pty_cmd_args;
use golish_context::token_budget::TokenUsage;
use golish_sub_agents::SubAgentContext;

// `AiEvent` is no longer emitted directly from this function (the
// pre-flight phase handles its cases); it is still needed by the
// `agentic_loop::tests` submodule which inherits via `use super::*;`.
#[cfg(test)]
use golish_core::events::AiEvent;

mod assistant_message;
mod compaction_loop;
mod config;
mod context;
mod entry;
mod first_iter_hooks;
mod helpers;
mod llm_helpers;
mod llm_stream_start;
mod reflector;
mod single_tool_call;
mod stream_processor;
mod stream_retry;
pub mod sub_agent_dispatch;
pub mod tool_classifier;
mod tool_dispatch;
pub(crate) mod tool_execution;
mod tool_gate;
mod tool_intent;
mod tool_list;
pub mod toolcall_fixer;
mod turn;
mod unified_helpers;

pub mod compaction;

pub use compaction::{
    apply_compaction, get_artifacts_dir, get_artifacts_dir_for, get_summaries_dir,
    get_summaries_dir_for, get_transcript_dir, get_transcript_dir_for, maybe_compact,
    CompactionResult,
};
pub use config::AgenticLoopConfig;
pub use context::{
    AgenticLoopContext, LoopAccessControl, LoopCaptureContext, LoopEventRefs, LoopLlmRefs,
    McpToolExecutor, OutputClassifier, PostShellHook, TerminalErrorEmitted, ToolExecutionResult,
};
pub use entry::{run_agentic_loop, run_agentic_loop_generic};
pub use tool_execution::{execute_tool_direct_generic, execute_with_hitl_generic};

use context::{emit_event, emit_to_frontend};

// Keep `estimate_message_tokens` in scope for the `tests` submodule that
// inherits via `use super::*;`. Production callers moved to the
// `turn::phases::token_estimate` phase.
#[cfg(test)]
#[allow(unused_imports)]
use helpers::estimate_message_tokens;

/// Maximum number of tool call iterations before stopping
pub const MAX_TOOL_ITERATIONS: usize = 100;

/// Timeout for approval requests in seconds (30 minutes)
pub const APPROVAL_TIMEOUT_SECS: u64 = 1800;

/// Maximum tokens for a single completion request
pub const MAX_COMPLETION_TOKENS: u32 = 10_000;

/// Token threshold above which truncated tool output is further summarized by the LLM.
/// Outputs shorter than this after truncation are passed through as-is.
const SUMMARIZE_THRESHOLD_TOKENS: usize = 2000;

/// Unified agentic loop that handles all model types.
///
/// Configuration-driven entry point used by both Anthropic-with-thinking
/// runs and generic-model runs. The actual scheduler lives in
/// [`turn::run_turn_loop`]; this function is the thin public wrapper
/// callers across the workspace import.
///
/// # Behaviour notes
/// - `config.capabilities.supports_thinking_history`: preserve reasoning
///   content in the chat history (Anthropic extended thinking).
/// - `config.require_hitl`: route tool execution through HITL approval.
/// - `config.is_sub_agent`: tighten allow-list to orchestration tools.
///
/// Returns `(response_text, optional_reasoning, updated_history,
/// total_usage)`.
pub async fn run_agentic_loop_unified<M>(
    model: &M,
    system_prompt: &str,
    initial_history: Vec<Message>,
    sub_agent_context: SubAgentContext,
    ctx: &AgenticLoopContext<'_>,
    config: AgenticLoopConfig,
) -> Result<(String, Option<String>, Vec<Message>, Option<TokenUsage>)>
where
    M: rig::completion::CompletionModel + Sync,
{
    turn::run_turn_loop(
        model,
        system_prompt,
        initial_history,
        sub_agent_context,
        ctx,
        config,
    )
    .await
}

#[cfg(test)]
mod tests;
