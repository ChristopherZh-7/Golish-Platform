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

use assistant_message::push_assistant_message;
use first_iter_hooks::run_first_iteration_hooks;
use llm_stream_start::start_completion_stream;
use reflector::{maybe_run_reflector, ReflectorOutcome};
use stream_processor::{process_stream, StreamOutcome};
use tool_dispatch::dispatch_tool_calls;
use tool_list::build_tool_list;
// `compaction` alias avoids clashing with the sibling `agentic_loop::compaction` module.
use turn::{compaction as compaction_phase, pre_flight, PhaseOutcome, TurnState};
#[allow(unused_imports)] // BreakReason is used for log-fields in the Break arm below
use turn::BreakReason;
use unified_helpers::{
    log_image_and_reasoning_diagnostics, push_unavailable_tool_results,
    record_agent_turn_start, record_last_user_text_for_span, record_turn_completion,
    trace_input_for_span,
};

use helpers::estimate_message_tokens;
pub use tool_execution::{execute_tool_direct_generic, execute_with_hitl_generic};

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
    // Iteration counter now lives in `turn::TurnState`; ADR-0010 C1-1 PoC.
    // Other per-turn locals will migrate into `TurnState` in subsequent PRs.
    let mut turn_state = TurnState::new();
    let mut consecutive_no_tool_turns: u32 = 0;
    let mut total_reflector_nudges: u32 = 0;
    // Mutated by `run_first_iteration_hooks` once at iteration 1; see
    // [`first_iter_hooks::FirstIterationOutcome`].
    let mut reflector_active = true;

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
        let iteration = turn_state.iteration as usize;

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

        // First-iteration hooks: synchronous message hooks + memory gatekeeper.
        if iteration == 1 && !config.is_sub_agent {
            let outcome =
                run_first_iteration_hooks(ctx, &hook_registry, &mut chat_history).await;
            reflector_active = outcome.reflector_active;
        }

        // Create span for Langfuse observability (child of agent_span)
        // Token usage fields are Empty and will be recorded when available
        // Note: Langfuse expects prompt_tokens/completion_tokens per GenAI semantic conventions
        // Using both gen_ai.* and langfuse.observation.* for maximum compatibility
        let llm_span = tracing::info_span!(
            parent: &agent_span,
            "llm_completion",
            "gen_ai.operation.name" = "chat_completion",
            "gen_ai.request.model" = %ctx.llm.model_name,
            "gen_ai.system" = %ctx.llm.provider_name,
            "gen_ai.request.temperature" = 0.3_f64,
            "gen_ai.request.max_tokens" = MAX_COMPLETION_TOKENS as i64,
            "langfuse.observation.type" = "generation",
            "langfuse.session.id" = ctx.events.session_id.unwrap_or(""),
            iteration = iteration,
            "gen_ai.usage.prompt_tokens" = tracing::field::Empty,
            "gen_ai.usage.completion_tokens" = tracing::field::Empty,
            // Use both gen_ai.* and langfuse.observation.* for input/output mapping
            "gen_ai.reasoning" = tracing::field::Empty,
            "gen_ai.prompt" = tracing::field::Empty,
            "gen_ai.completion" = tracing::field::Empty,
            "langfuse.observation.input" = tracing::field::Empty,
            "langfuse.observation.output" = tracing::field::Empty,
        );
        // Note: We use explicit parent instead of span.enter() for async compatibility

        record_last_user_text_for_span(&llm_span, &chat_history);

        log_image_and_reasoning_diagnostics(
            &chat_history,
            iteration,
            ctx.llm.provider_name,
            supports_thinking,
        );

        // Proactive token count: estimate tokens BEFORE sending to detect
        // compaction need early. This is a leading indicator vs the lagging
        // provider-reported count after the response.
        {
            let system_prompt_tokens = tokenx_rs::estimate_token_count(system_prompt);
            let history_tokens: usize = chat_history.iter().map(estimate_message_tokens).sum();
            let estimated_input_tokens = (system_prompt_tokens + history_tokens) as u64;

            let mut compaction_state = ctx.compaction_state.write().await;
            compaction_state.update_tokens_estimated(estimated_input_tokens);
            tracing::debug!(
                "[compaction] Pre-call estimate: ~{} tokens (system={}, history={})",
                estimated_input_tokens,
                system_prompt_tokens,
                history_tokens,
            );
        }

        let stream = start_completion_stream(
            ctx,
            &config,
            model,
            system_prompt,
            &chat_history,
            &tools,
            &llm_span,
            &accumulated_response,
        )
        .await?;

        let outcome = match process_stream::<M>(
            stream,
            ctx,
            &chat_history,
            &llm_span,
            iteration,
            supports_thinking,
            &mut accumulated_response,
            &mut accumulated_thinking,
            &mut total_usage,
        )
        .await?
        {
            StreamOutcome::Continue(outcome) => outcome,
            StreamOutcome::BreakAgentLoop => break,
        };

        let stream_processor::StreamProcessOutcome {
            has_tool_calls,
            tool_calls_to_execute,
            text_content,
            thinking_content,
            thinking_signature,
            thinking_id,
        } = outcome;

        push_assistant_message(
            &mut chat_history,
            &text_content,
            &thinking_content,
            &thinking_signature,
            &thinking_id,
            &tool_calls_to_execute,
            has_tool_calls,
            supports_thinking,
            ctx.llm.provider_name,
        );

        // If no tool calls, either invoke the reflector or finish.
        if !has_tool_calls {
            consecutive_no_tool_turns += 1;

            match maybe_run_reflector(
                ctx,
                &sub_agent_context,
                &config,
                &mut chat_history,
                &text_content,
                consecutive_no_tool_turns,
                &mut total_reflector_nudges,
                reflector_active,
                &tools,
            )
            .await
            {
                ReflectorOutcome::Injected => continue,
                ReflectorOutcome::Skipped => break,
            }
        } else {
            consecutive_no_tool_turns = 0;
        }

        // Filter out tool calls not in the allowed tool list.
        // In Task mode the primary only has orchestration tools; the model may
        // hallucinate direct-tool calls from the system prompt or restored history.
        let allowed_names: std::collections::HashSet<&str> =
            tools.iter().map(|t| t.name.as_str()).collect();
        let (permitted, rejected): (Vec<_>, Vec<_>) = tool_calls_to_execute
            .into_iter()
            .partition(|tc| allowed_names.contains(tc.function.name.as_str()));

        if !rejected.is_empty() {
            let rejected_names: Vec<&str> = rejected.iter().map(|tc| tc.function.name.as_str()).collect();
            tracing::warn!(
                "[tool-guard] Blocked {} tool call(s) not in allowed list: {:?}",
                rejected.len(),
                rejected_names,
            );
            push_unavailable_tool_results(&mut chat_history, &rejected);
        }

        if !permitted.is_empty() {
            dispatch_tool_calls(
                permitted,
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
