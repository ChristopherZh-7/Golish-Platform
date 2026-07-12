//! Turn executor — the phase scheduler.
//!
//! `run_turn_loop` owns the full life-cycle of one agentic run from
//! span creation through phase scheduling to final-output recording.
//! `agentic_loop::run_agentic_loop_unified` is now a thin wrapper that
//! delegates here.
//!
//! ## Why a single function (not yet a `TurnExecutor` struct)?
//!
//! A future `TurnExecutor` struct with `phase_order` and
//! `Vec<Box<dyn TurnInterceptor>>` is sketched but not implemented.
//! We deliberately keep the milestone scope minimal: the phases all
//! have heterogeneous signatures, so unifying them behind a single
//! trait would be a large follow-on effort. The current contract is just:
//!
//! - move the body out of `mod.rs` so it shrinks to ≤150 LOC, and
//! - make the phase order *visibly* the body of one function so the
//!   scheduler is one screen tall and can be read top-to-bottom.
//!
//! C1-7 introduces `TurnInterceptor` and lifts span / HITL plumbing
//! out of the phases. C1-8 adds per-phase unit tests.

use anyhow::Result;
use rig::completion::Message;
use rig::message::{Text, UserContent};
use rig::one_or_many::OneOrMany;
use tracing::Instrument;

use golish_agent_kit::system_hooks::HookRegistry;
use golish_context::token_budget::TokenUsage;
use golish_sub_agents::SubAgentContext;

use super::super::config::AgenticLoopConfig;
use super::super::context::{AgenticLoopContext, LoopCaptureContext};
use super::super::stream_processor::StreamProcessOutcome;
use super::super::tool_list::build_tool_list;
use super::super::unified_helpers::{
    record_agent_turn_start, record_turn_completion, trace_input_for_span,
};
use super::interceptor::{LangfuseInterceptor, TurnInterceptor, TurnSpans};
use super::{
    assistant_push_phase, compaction as compaction_phase,
    completion::{self as completion_phase, CompletionOutcome},
    first_iter_hooks_phase, pre_flight, reflector_or_break_phase, token_estimate_phase,
    tool_dispatch_phase, PhaseOutcome, ReflectorPhaseOutcome, TurnState,
};

/// E1 · max "stop repeating" recovery re-prompts per run before we give up and
/// accept the partial output (avoids an unbounded retry loop on a model that
/// keeps degenerating).
const MAX_REPETITION_RECOVERIES: u32 = 2;

/// E2 · max retries per run after a retriable mid-stream error truncated the
/// output.
const MAX_MID_STREAM_RETRIES: u32 = 2;

fn accepted_stage_submission_ends_loop(
    stage_submission_accepted: bool,
    harness_stage_active: bool,
) -> bool {
    stage_submission_accepted && harness_stage_active
}

/// E1 · corrective prompt injected when the model degenerated into repetition.
fn repetition_recovery_message() -> Message {
    Message::User {
        content: OneOrMany::one(UserContent::Text(Text {
            text: "[System: Your previous response was repeating itself and was cut off. \
                   Stop repeating. Do not restate earlier text. Give your conclusion or the \
                   next concrete action directly and concisely — if a tool is needed, call it.]"
                .to_string(),
        })),
    }
}

/// E2 · corrective prompt injected when a transient error truncated the stream.
fn mid_stream_recovery_message() -> Message {
    Message::User {
        content: OneOrMany::one(UserContent::Text(Text {
            text: "[System: Your previous response was cut off by a transient connection error. \
                   Continue from where you left off and finish the step — do not repeat what you \
                   already wrote; if a tool is needed, call it.]"
                .to_string(),
        })),
    }
}

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
    // `tool_call` spans (created inside their respective phases). The
    // pair is wrapped in a `TurnSpans` so interceptors can write
    // trailing fields onto them after the loop terminates.
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

    let spans = TurnSpans {
        chat_message_span,
        agent_span,
    };

    // Cross-cutting hooks (Langfuse trailing fields, future HITL
    // recording, eval logging). Today there is exactly one — the trait
    // is in place so adding more does not require touching phase code.
    let interceptors: Vec<Box<dyn TurnInterceptor>> = vec![Box::new(LangfuseInterceptor)];

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
        // Loop-wide state lives in `TurnState`:
        //   - `iteration` (set by `pre_flight`)
        //   - `reflector_active` (set by `first_iter_hooks`)
        //   - `consecutive_no_tool_turns`, `total_reflector_nudges`
        //     (managed by `reflector_or_break`).
        let mut turn_state = TurnState::new();

        loop {
            // Phase 1: PreFlight — iteration counter + cancel + budget.
            // Runs BEFORE pre_turn_compaction so cancellation is observed
            // one step earlier than the legacy code path.
            match pre_flight::run(&mut turn_state, ctx, &spans.agent_span).await {
                PhaseOutcome::Continue => {}
                PhaseOutcome::Break(reason) => {
                    tracing::debug!(?reason, "pre-flight phase requested loop break");
                    break;
                }
                PhaseOutcome::Fail(e) => return Err(e),
            }

            // Phase 2: Compaction — pre-turn (iter 1) or inter-turn (>1).
            // `inter_turn_compaction` may surface a terminal error via Fail.
            match compaction_phase::run(&turn_state, ctx, &mut chat_history, &accumulated_response)
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
                &spans.agent_span,
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
                repetition_detected,
                mid_stream_error,
            } = outcome;

            // 设计 2026-06-12 (submit-only-lock-hardening 防御 B) · dispatch 层闭锁
            // 取值。必须在下面「批次含 submit 则置 stage_deliverable_submitted」之
            // 前算：这样同一批次里的 submit 仍放过、同批的其它工具（如 textual-adapter
            // 恢复出来的 update_plan）被拒。API 层 tool_choice 锁不住忽略它的 provider
            // 走文本通道恢复的调用，这道闸在真正执行前堵死侧门。
            let submit_only_lock =
                ctx.harness_submit_only && !turn_state.stage_deliverable_submitted;
            let forced_tool_lock = ctx
                .harness_forced_tool
                .as_deref()
                .filter(|_| !turn_state.forced_tool_dispatched);

            // Harness stage barrier: remember when the agent submits a
            // StageDeliverable. A later idle turn (in ReflectorOrBreak) then ends
            // the stage loop so the orchestrator runs the authoritative gate and
            // advances — instead of the reflector stranding the agent in-stage
            // (root cause of the "stuck in scoping" hang). `submit_stage_deliverable`
            // only exists in harness stages, so this is inert elsewhere.
            if tool_calls_to_execute
                .iter()
                .any(|tc| tc.function.name == "submit_stage_deliverable")
            {
                turn_state.stage_deliverable_submitted = true;
            }
            if forced_tool_lock
                .map(|tool| {
                    tool_calls_to_execute
                        .iter()
                        .any(|tc| tc.function.name == tool)
                })
                .unwrap_or(false)
            {
                turn_state.forced_tool_dispatched = true;
            }

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

            // E1/E2 · provider-resilience recovery (between AssistantPush and
            // Reflector). The partial assistant turn is already in history; we
            // append a corrective user turn and re-run a fresh iteration, each
            // bounded by its own budget so a persistently-failing model can't
            // spin forever. Only fires on text-only turns — if the model still
            // managed a tool call, let ToolDispatch proceed normally.
            if !has_tool_calls && repetition_detected {
                if turn_state.repetition_recoveries < MAX_REPETITION_RECOVERIES {
                    turn_state.repetition_recoveries += 1;
                    tracing::warn!(
                        recovery = turn_state.repetition_recoveries,
                        max = MAX_REPETITION_RECOVERIES,
                        "[resilience] repetitive output detected; injecting recovery re-prompt and retrying"
                    );
                    chat_history.push(repetition_recovery_message());
                    continue;
                }
                tracing::warn!(
                    "[resilience] repetitive output persisted after {} recoveries; accepting partial output",
                    MAX_REPETITION_RECOVERIES
                );
            } else if !has_tool_calls {
                if let Some(err) = mid_stream_error.as_deref() {
                    if turn_state.mid_stream_retries < MAX_MID_STREAM_RETRIES {
                        turn_state.mid_stream_retries += 1;
                        tracing::warn!(
                            retry = turn_state.mid_stream_retries,
                            max = MAX_MID_STREAM_RETRIES,
                            error = %err,
                            "[resilience] transient mid-stream error truncated output; injecting continuation re-prompt and retrying"
                        );
                        chat_history.push(mid_stream_recovery_message());
                        continue;
                    }
                    tracing::warn!(
                        error = %err,
                        "[resilience] mid-stream error persisted after {} retries; accepting partial output",
                        MAX_MID_STREAM_RETRIES
                    );
                }
            }

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
            let dispatch = tool_dispatch_phase::run(
                tool_calls_to_execute,
                &tools,
                ctx,
                &capture_ctx,
                model,
                &sub_agent_context,
                &hook_registry,
                &llm_span,
                &mut chat_history,
                submit_only_lock,
                forced_tool_lock,
            )
            .await;
            if accepted_stage_submission_ends_loop(
                dispatch.stage_submission_accepted,
                ctx.harness_stage.is_some(),
            ) {
                tracing::info!(
                    target: "harness::submit_tool",
                    "accepted stage deliverable ended the primary stage loop"
                );
                break;
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

        Ok::<_, anyhow::Error>((
            accumulated_response,
            accumulated_thinking,
            chat_history,
            total_usage,
        ))
    }
    .instrument(spans.agent_span.clone())
    .instrument(spans.chat_message_span.clone())
    .await?;

    // Run all registered interceptors' `after_turn` hooks. Today this
    // is just `LangfuseInterceptor` writing trailing trace fields, but
    // the loop is in place for future cross-cutting concerns.
    for interceptor in &interceptors {
        interceptor
            .after_turn(ctx, &spans, &accumulated_response, &total_usage)
            .await;
    }

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

#[cfg(test)]
mod tests {
    use super::accepted_stage_submission_ends_loop;

    #[test]
    fn accepted_submission_ends_only_an_active_harness_stage_loop() {
        assert!(accepted_stage_submission_ends_loop(true, true));
        assert!(!accepted_stage_submission_ends_loop(true, false));
        assert!(!accepted_stage_submission_ends_loop(false, true));
        assert!(!accepted_stage_submission_ends_loop(false, false));
    }
}
