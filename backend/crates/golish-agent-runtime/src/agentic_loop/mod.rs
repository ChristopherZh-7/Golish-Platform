//! Agentic tool loop for LLM execution.
//!
//! This module contains the main agentic loop that handles:
//! - Tool execution with HITL approval
//! - Loop detection and prevention
//! - Context window management
//! - Message history management
//! - Extended thinking (streaming reasoning content)

use anyhow::Result;
use rig::completion::Message;
use tracing::Instrument;

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

use golish_agent_kit::system_hooks::HookRegistry;
use golish_agent_kit::tool_definitions::ToolConfig;
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
pub mod sub_agent_dispatch;
mod stream_processor;
mod tool_dispatch;
mod tool_execution;
mod tool_list;
pub mod toolcall_fixer;
mod turn;
mod unified_helpers;

use stream_processor::StreamProcessOutcome;
use tool_list::build_tool_list;
// Phase aliases (`*_phase`) come from `turn::{...}` re-exports and avoid
// clashing with sibling `agentic_loop::{compaction, tool_dispatch}`
// modules whose internals these phases reach into.
use turn::{
    assistant_push_phase, compaction as compaction_phase,
    completion::{self as completion_phase, CompletionOutcome},
    first_iter_hooks_phase, pre_flight, reflector_or_break_phase, token_estimate_phase,
    tool_dispatch_phase, PhaseOutcome, ReflectorPhaseOutcome, TurnState,
};
use unified_helpers::{record_agent_turn_start, record_turn_completion, trace_input_for_span};

pub use tool_execution::{execute_tool_direct_generic, execute_with_hitl_generic};

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

mod stream_retry;

pub mod compaction;
pub use compaction::{
    apply_compaction, get_artifacts_dir, get_artifacts_dir_for, get_summaries_dir,
    get_summaries_dir_for, get_transcript_dir, get_transcript_dir_for, maybe_compact,
    CompactionResult,
};

pub use context::{
    AgenticLoopContext, LoopAccessControl, LoopCaptureContext, LoopEventRefs, LoopLlmRefs,
    McpToolExecutor, OutputClassifier, PostShellHook, TerminalErrorEmitted, ToolExecutionResult,
};
use context::{emit_event, emit_to_frontend};


pub use entry::{run_agentic_loop, run_agentic_loop_generic};
pub use config::AgenticLoopConfig;

/// Unified agentic loop that handles all model types.
///
/// This function replaces both `run_agentic_loop` (Anthropic) and
/// `run_agentic_loop_generic` by using configuration to control behavior.
///
/// # Key Differences from Separate Loops
///
/// 1. **Thinking History**: When `config.capabilities.supports_thinking_history` is true,
///    reasoning content from the model is preserved in the message history
///    (required by Anthropic API when extended thinking is enabled).
///
/// 2. **HITL Approval**: When `config.require_hitl` is true, tool execution
///    requires human-in-the-loop approval (unless auto-approved by policy).
///
/// 3. **Sub-Agent Restrictions**: When `config.is_sub_agent` is true,
///    certain tool restrictions may apply.
///
/// # Arguments
/// * `model` - The completion model to use
/// * `system_prompt` - System prompt for the agent
/// * `initial_history` - Starting conversation history
/// * `sub_agent_context` - Sub-agent execution context (includes depth tracking)
/// * `ctx` - Agent loop context with dependencies
/// * `config` - Configuration controlling behavior
///
/// # Returns
/// Tuple of (response_text, updated_history, token_usage)
///
/// # Example
/// ```ignore
/// use golish_agent_runtime::agentic_loop::{run_agentic_loop_unified, AgenticLoopConfig};
///
/// // For Anthropic models (with thinking support)
/// let config = AgenticLoopConfig::main_agent_anthropic();
/// let (response, history, usage) = run_agentic_loop_unified(
///     &model, system_prompt, history, context, &ctx, config
/// ).await?;
///
/// // For generic models (without thinking support)
/// let config = AgenticLoopConfig::main_agent_generic();
/// let (response, history, usage) = run_agentic_loop_unified(
///     &model, system_prompt, history, context, &ctx, config
/// ).await?;
/// ```
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
    let supports_thinking = config.capabilities.supports_thinking_history;

    let agent_label = if config.is_sub_agent {
        format!("sub-agent (depth={})", sub_agent_context.depth)
    } else {
        "main-agent".to_string()
    };

    tracing::info!(
        "[{}] Starting agentic loop: provider={}, model={}, thinking={}, temperature={}",
        agent_label,
        ctx.llm.provider_name,
        ctx.llm.model_name,
        supports_thinking,
        config.capabilities.supports_temperature
    );

    // Create root span for the entire agent turn (this becomes the Langfuse trace).
    // All child spans (llm_completion, tool_call) will be nested under this.
    let trace_input_truncated = trace_input_for_span(&initial_history);

    // Create outer trace span (this becomes the Langfuse trace)
    let chat_message_span = tracing::info_span!(
        "chat_message",
        "langfuse.session.id" = ctx.events.session_id.unwrap_or(""),
        "langfuse.observation.input" = %trace_input_truncated,
        "langfuse.observation.output" = tracing::field::Empty,
    );

    // Create agent span as child of trace (this is the main agent observation)
    let agent_span = tracing::info_span!(
        parent: &chat_message_span,
        "agent",
        "langfuse.observation.type" = "agent",
        "langfuse.session.id" = ctx.events.session_id.unwrap_or(""),
        "langfuse.observation.input" = %trace_input_truncated,
        "langfuse.observation.output" = tracing::field::Empty,
        agent_type = %agent_label,
        model = %ctx.llm.model_name,
        provider = %ctx.llm.provider_name,
    );
    // Instrument the main loop body with both spans so they're properly exported to OpenTelemetry.
    // Using nested .instrument() ensures both spans are entered for the duration of the loop.
    let (accumulated_response, accumulated_thinking, chat_history, total_usage) = async {
        // Reset loop detector for new turn
        {
        let mut detector = ctx.access.loop_detector.write().await;
        detector.reset();
    }

    // Create persistent capture context for file event correlation
    let capture_ctx = LoopCaptureContext::new(ctx.sidecar_state);

    // Create hook registry for system hooks
    let hook_registry = HookRegistry::new();

    let tools = build_tool_list(ctx, &sub_agent_context).await;

    let mut chat_history = initial_history;

    // Update context manager with current history
    ctx.context_manager
        .update_from_messages(&chat_history)
        .await;

    // Note: Context compaction is now handled by the summarizer agent
    // which is triggered via should_compact() in the agentic loop

    record_agent_turn_start(ctx, &chat_history);

    let mut accumulated_response = String::new();
    // Thinking history tracking - only used when supports_thinking is true
    let mut accumulated_thinking = String::new();
    let mut total_usage = TokenUsage::default();
    // Loop-wide state lives in `TurnState` (see ADR-0010):
    // - `iteration` (set by `pre_flight`)
    // - `reflector_active` (set by `first_iter_hooks`)
    // - `consecutive_no_tool_turns`, `total_reflector_nudges`
    //   (managed by `reflector_or_break`).
    let mut turn_state = TurnState::new();

    loop {
        // Phase: PreFlight — iteration bookkeeping + cancel + budget
        // (see ADR-0010). Side-effects on `turn_state.iteration` and
        // the shared compaction snapshot.
        //
        // Note: pre-flight now runs BEFORE `pre_turn_compaction`
        // (historically they were interleaved). The observable behavior
        // change is that cancellation and the max-iteration budget are
        // checked one extra step earlier, which is desirable.
        match pre_flight::run(&mut turn_state, ctx, &agent_span).await {
            PhaseOutcome::Continue => {}
            PhaseOutcome::Break(reason) => {
                tracing::debug!(?reason, "pre-flight phase requested loop break");
                break;
            }
            PhaseOutcome::Fail(e) => return Err(e),
        }

        // Phase: Compaction — pre-turn (iteration 1) or inter-turn (>1).
        // `inter_turn_compaction` may surface a terminal error via Fail.
        match compaction_phase::run(
            &turn_state,
            ctx,
            &mut chat_history,
            &accumulated_response,
        )
        .await
        {
            PhaseOutcome::Continue => {}
            PhaseOutcome::Break(_) => {
                // Compaction phase does not request break today; keep
                // the arm for exhaustiveness as scheduler evolves.
                break;
            }
            PhaseOutcome::Fail(e) => return Err(e),
        }

        // Phase: FirstIterHooks — no-op for sub-agents, on iteration 1
        // for main agent runs the message hooks + memory gatekeeper.
        match first_iter_hooks_phase::run(
            &mut turn_state,
            ctx,
            &config,
            &hook_registry,
            &mut chat_history,
        )
        .await
        {
            PhaseOutcome::Continue => {}
            PhaseOutcome::Break(_) => break,
            PhaseOutcome::Fail(e) => return Err(e),
        }

        // Phase: TokenEstimate — proactively update compaction_state with
        // an estimated input-token count before the LLM call.
        match token_estimate_phase::run(ctx, system_prompt, &chat_history).await {
            PhaseOutcome::Continue => {}
            PhaseOutcome::Break(_) => break,
            PhaseOutcome::Fail(e) => return Err(e),
        }

        // Phase: Completion — build llm_span, start stream, consume it
        // to a StreamProcessOutcome. Carries `llm_span` forward so
        // downstream phases can continue to emit nested observations
        // (C1-7 will lift this out to a TurnInterceptor).
        let (outcome, llm_span) = match completion_phase::run(
            &turn_state,
            ctx,
            &config,
            model,
            system_prompt,
            &chat_history,
            &tools,
            &agent_span,
            supports_thinking,
            &mut accumulated_response,
            &mut accumulated_thinking,
            &mut total_usage,
        )
        .await?
        {
            CompletionOutcome::Continue { outcome, llm_span } => (outcome, llm_span),
            CompletionOutcome::BreakAgentLoop => break,
        };

        let StreamProcessOutcome {
            has_tool_calls,
            tool_calls_to_execute,
            text_content,
            thinking_content,
            thinking_signature,
            thinking_id,
        } = outcome;

        // Phase: AssistantPush — append the assistant message
        // (text + reasoning + tool calls) to the chat history.
        assistant_push_phase::run(
            &mut chat_history,
            &text_content,
            &thinking_content,
            &thinking_signature,
            &thinking_id,
            &tool_calls_to_execute,
            has_tool_calls,
            supports_thinking,
            ctx,
        );

        // Phase: ReflectorOrBreak — when no tool calls were produced,
        // optionally invoke the reflector and tell the scheduler whether
        // to repeat the iteration or break the loop. Tool-call branch
        // resets `consecutive_no_tool_turns` and falls through.
        match reflector_or_break_phase::run(
            &mut turn_state,
            ctx,
            &sub_agent_context,
            &config,
            &mut chat_history,
            has_tool_calls,
            &text_content,
            &tools,
        )
        .await
        {
            ReflectorPhaseOutcome::Continue => {}
            ReflectorPhaseOutcome::Repeat => continue,
            ReflectorPhaseOutcome::Break => break,
        }

        // Phase: ToolDispatch — allow-list filter, push synthetic errors
        // for blocked calls, and dispatch the permitted batch.
        tool_dispatch_phase::run(
            tool_calls_to_execute,
            &tools,
            ctx,
            &capture_ctx,
            model,
            &sub_agent_context,
            &hook_registry,
            &llm_span,
            &mut chat_history,
        )
        .await;
    }

    record_turn_completion(
        ctx,
        &config,
        &sub_agent_context,
        supports_thinking,
        &accumulated_thinking,
        &total_usage,
    );

        Ok::<_, anyhow::Error>((accumulated_response, accumulated_thinking, chat_history, total_usage))
    }
    .instrument(agent_span.clone())
    .instrument(chat_message_span.clone())
    .await?;

    unified_helpers::record_final_output_and_usage(
        ctx,
        &accumulated_response,
        &total_usage,
        &chat_message_span,
        &agent_span,
    );

    // Convert accumulated_thinking to Option (None if empty)
    let reasoning = if accumulated_thinking.is_empty() {
        None
    } else {
        Some(accumulated_thinking)
    };

    Ok((
        accumulated_response,
        reasoning,
        chat_history,
        Some(total_usage),
    ))
}

// =============================================================================
// CONTEXT COMPACTION ORCHESTRATION
// =============================================================================

#[cfg(test)]
mod tests;
