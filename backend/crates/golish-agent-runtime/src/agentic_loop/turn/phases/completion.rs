//! Completion phase — create the per-turn LLM span, drive the streaming
//! LLM call, and consume the stream into a `StreamProcessOutcome`.
//!
//! Combines what was previously three separate code blocks in the main
//! loop:
//! 1. Langfuse `llm_completion` span creation
//! 2. `start_completion_stream` + `process_stream`
//! 3. `StreamOutcome` match (continue with outcome vs break)
//!
//! Returns a phase-specific `CompletionOutcome` because we need to
//! carry the `StreamProcessOutcome` through to downstream phases
//! (AssistantPush / Reflector / ToolDispatch in C1-5).
//!
//! Note: the Langfuse span lives inside this phase today; in C1-7 it
//! moves to a `TurnInterceptor` so spans can be managed declaratively.

use anyhow::Result;
use rig::completion::Message;
use tracing::Span;

use golish_context::token_budget::TokenUsage;

use super::super::super::config::AgenticLoopConfig;
use super::super::super::context::AgenticLoopContext;
use super::super::super::llm_stream_start::start_completion_stream;
use super::super::super::stream_processor::{process_stream, StreamOutcome, StreamProcessOutcome};
use super::super::super::unified_helpers::{
    log_image_and_reasoning_diagnostics, record_last_user_text_for_span,
};
use super::super::super::MAX_COMPLETION_TOKENS;
use super::super::state::TurnState;

/// Outcome of the Completion phase.
pub enum CompletionOutcome {
    /// Stream produced usable content; carry the accumulators to the
    /// next phases.
    Continue {
        outcome: StreamProcessOutcome,
        llm_span: Span,
    },
    /// Stream produced nothing and a terminal error was emitted —
    /// scheduler must break the outer loop.
    BreakAgentLoop,
}

/// Drive one LLM turn: create span, start stream, process to completion.
#[allow(clippy::too_many_arguments)]
pub async fn run<M>(
    state: &TurnState,
    ctx: &AgenticLoopContext<'_>,
    config: &AgenticLoopConfig,
    model: &M,
    system_prompt: &str,
    chat_history: &[Message],
    tools: &[rig::completion::ToolDefinition],
    agent_span: &Span,
    supports_thinking: bool,
    accumulated_response: &mut String,
    accumulated_thinking: &mut String,
    total_usage: &mut TokenUsage,
) -> Result<CompletionOutcome>
where
    M: rig::completion::CompletionModel + Sync,
{
    let iteration = state.iteration as usize;

    // Create span for Langfuse observability (child of agent_span). Token
    // usage fields are Empty and will be recorded when available by
    // process_stream. Langfuse expects gen_ai.* / langfuse.observation.*
    // for maximum compatibility.
    let llm_span = tracing::info_span!(
        parent: agent_span,
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
        "gen_ai.reasoning" = tracing::field::Empty,
        "gen_ai.prompt" = tracing::field::Empty,
        "gen_ai.completion" = tracing::field::Empty,
        "langfuse.observation.input" = tracing::field::Empty,
        "langfuse.observation.output" = tracing::field::Empty,
    );

    record_last_user_text_for_span(&llm_span, chat_history);

    log_image_and_reasoning_diagnostics(
        chat_history,
        iteration,
        ctx.llm.provider_name,
        supports_thinking,
    );

    let stream = start_completion_stream(
        ctx,
        config,
        model,
        system_prompt,
        chat_history,
        tools,
        &llm_span,
        accumulated_response,
    )
    .await?;

    match process_stream::<M>(
        stream,
        ctx,
        chat_history,
        &llm_span,
        iteration,
        supports_thinking,
        accumulated_response,
        accumulated_thinking,
        total_usage,
    )
    .await?
    {
        StreamOutcome::Continue(outcome) => Ok(CompletionOutcome::Continue { outcome, llm_span }),
        StreamOutcome::BreakAgentLoop => Ok(CompletionOutcome::BreakAgentLoop),
    }
}
