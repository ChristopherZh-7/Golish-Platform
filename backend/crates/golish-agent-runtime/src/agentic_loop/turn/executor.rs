//! Turn executor — the phase scheduler.
//!
//! `run_turn_loop` owns the full life-cycle of one agentic run from
//! span creation through phase scheduling to final-output recording.
//! `agentic_loop::run_agentic_loop_unified` is now a thin wrapper that
//! delegates here.
//!
//! ## Why a single function (not yet a `TurnExecutor` struct)?
//!
//! ADR-0010 sketches a future `TurnExecutor` struct with `phase_order`
//! and `Vec<Box<dyn TurnInterceptor>>`. We deliberately keep the
//! milestone scope minimal: the phases all have heterogeneous
//! signatures, so unifying them behind a single trait would be a large
//! follow-on effort. The C1-6 contract is just:
//!
//! - move the body out of `mod.rs` so it shrinks to ≤150 LOC, and
//! - make the phase order *visibly* the body of one function so the
//!   scheduler is one screen tall and can be read top-to-bottom.
//!
//! C1-7 introduces `TurnInterceptor` and lifts span / HITL plumbing
//! out of the phases. C1-8 adds per-phase unit tests.

use anyhow::Result;
use rig::completion::Message;
use tracing::Instrument;

use golish_agent_kit::system_hooks::HookRegistry;
use golish_context::token_budget::TokenUsage;
use golish_sub_agents::SubAgentContext;

use super::super::config::AgenticLoopConfig;
use super::super::context::{AgenticLoopContext, LoopCaptureContext};
use super::super::stream_processor::StreamProcessOutcome;
use super::super::tool_list::build_tool_list;
use super::super::unified_helpers::{
    record_agent_turn_start, record_final_output_and_usage, record_turn_completion,
    trace_input_for_span,
};
use super::{
    assistant_push_phase, compaction as compaction_phase,
    completion::{self as completion_phase, CompletionOutcome},
    first_iter_hooks_phase, pre_flight, reflector_or_break_phase, token_estimate_phase,
    tool_dispatch_phase, PhaseOutcome, ReflectorPhaseOutcome, TurnState,
};

/// Drive one full agentic run end-to-end: build observability spans,
/// initialise per-loop state, schedule the 8 turn phases, then record
/// final output / token usage.
///
/// Returns `(response_text, optional_reasoning, updated_history,
/// total_token_usage)`. Reasoning is `Some` only when the model emitted
/// thinking content; the chat history reflects the conversation as
/// observed *after* the loop terminates.
pub async fn run_turn_loop<M>(
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

    // Build the Langfuse span tree: `chat_message` (trace) ⊃ `agent`
    // (root observation) ⊃ each iteration's `llm_completion` /
    // `tool_call` spans (created inside their respective phases).
    let trace_input_truncated = trace_input_for_span(&initial_history);

    let chat_message_span = tracing::info_span!(
        "chat_message",
        "langfuse.session.id" = ctx.events.session_id.unwrap_or(""),
        "langfuse.observation.input" = %trace_input_truncated,
        "langfuse.observation.output" = tracing::field::Empty,
    );

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

    // Nested `.instrument()` ensures both spans are entered for the
    // whole loop body, so OpenTelemetry exports the right parent chain.
    let (accumulated_response, accumulated_thinking, chat_history, total_usage) = async {
        // Reset loop detector for new turn.
        {
            let mut detector = ctx.access.loop_detector.write().await;
            detector.reset();
        }

        let capture_ctx = LoopCaptureContext::new(ctx.sidecar_state);
        let hook_registry = HookRegistry::new();
        let tools = build_tool_list(ctx, &sub_agent_context).await;

        let mut chat_history = initial_history;
        ctx.context_manager
            .update_from_messages(&chat_history)
            .await;

        record_agent_turn_start(ctx, &chat_history);

        let mut accumulated_response = String::new();
        let mut accumulated_thinking = String::new();
        let mut total_usage = TokenUsage::default();
        // Loop-wide state lives in `TurnState` (see ADR-0010):
        //   - `iteration` (set by `pre_flight`)
        //   - `reflector_active` (set by `first_iter_hooks`)
        //   - `consecutive_no_tool_turns`, `total_reflector_nudges`
        //     (managed by `reflector_or_break`).
        let mut turn_state = TurnState::new();

        loop {
            // Phase 1: PreFlight — iteration counter + cancel + budget.
            // Runs BEFORE pre_turn_compaction so cancellation is observed
            // one step earlier than the legacy code path.
            match pre_flight::run(&mut turn_state, ctx, &agent_span).await {
                PhaseOutcome::Continue => {}
                PhaseOutcome::Break(reason) => {
                    tracing::debug!(?reason, "pre-flight phase requested loop break");
                    break;
                }
                PhaseOutcome::Fail(e) => return Err(e),
            }

            // Phase 2: Compaction — pre-turn (iter 1) or inter-turn (>1).
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
                PhaseOutcome::Break(_) => break,
                PhaseOutcome::Fail(e) => return Err(e),
            }

            // Phase 3: FirstIterHooks — message hooks + memory gatekeeper
            // on iteration 1 of a non-sub-agent run; otherwise no-op.
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

            // Phase 4: TokenEstimate — proactive input-token count.
            match token_estimate_phase::run(ctx, system_prompt, &chat_history).await {
                PhaseOutcome::Continue => {}
                PhaseOutcome::Break(_) => break,
                PhaseOutcome::Fail(e) => return Err(e),
            }

            // Phase 5: Completion — span + LLM stream + accumulators.
            // Carries `llm_span` to ToolDispatch so tool_call spans nest
            // under the same Langfuse generation. (C1-7 lifts span
            // ownership to a `TurnInterceptor`.)
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

            // Phase 6: AssistantPush — append assistant content to history.
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

            // Phase 7: ReflectorOrBreak — no-tool-call branch:
            // optionally inject a corrective prompt and repeat, or break.
            // Tool-call branch resets the no-tool counter and falls through.
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

            // Phase 8: ToolDispatch — allow-list filter + dispatch.
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

        Ok::<_, anyhow::Error>((
            accumulated_response,
            accumulated_thinking,
            chat_history,
            total_usage,
        ))
    }
    .instrument(agent_span.clone())
    .instrument(chat_message_span.clone())
    .await?;

    record_final_output_and_usage(
        ctx,
        &accumulated_response,
        &total_usage,
        &chat_message_span,
        &agent_span,
    );

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
