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
use golish_llm_providers::resolve_stream_quirks;

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

    // 设计 2026-06-11 · targeted gate-repair pass: lock this turn's tool_choice
    // onto `submit_stage_deliverable`; released once the submit tool has been
    // dispatched (`stage_deliverable_submitted`) so the loop can wind down
    // normally instead of being forced into duplicate submissions.
    let submit_only = ctx.harness_submit_only && !state.stage_deliverable_submitted;

    let stream = start_completion_stream(
        ctx,
        config,
        model,
        system_prompt,
        chat_history,
        tools,
        &llm_span,
        accumulated_response,
        submit_only,
    )
    .await?;

    let quirks = resolve_stream_quirks(
        ctx.llm.provider_name,
        ctx.llm.model_name,
        ctx.llm.model_override,
    );
    tracing::debug!(
        "[Quirks] provider={} model={} reasoning_handling={:?} force_disable_thinking={} user_override={}",
        ctx.llm.provider_name,
        ctx.llm.model_name,
        quirks.reasoning_handling,
        quirks.force_disable_thinking_kwargs,
        ctx.llm.model_override.is_some(),
    );

    match process_stream::<M>(
        stream,
        ctx,
        chat_history,
        &llm_span,
        iteration,
        supports_thinking,
        &quirks,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use golish_llm_providers::LlmClient;
    use rig::message::{Text, UserContent};
    use rig::one_or_many::OneOrMany;
    use tokio::sync::RwLock;

    use crate::test_utils::{MockCompletionModel, MockResponse, TestContextBuilder};

    use super::*;

    fn user_message(text: &str) -> Message {
        Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: text.to_string(),
            })),
        }
    }

    #[test]
    fn break_outcome_variant_signals_loop_termination() {
        let outcome = CompletionOutcome::BreakAgentLoop;
        assert!(matches!(outcome, CompletionOutcome::BreakAgentLoop));
    }

    #[tokio::test]
    async fn smoke_test_drives_mock_stream_and_returns_continue_with_text() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let config = AgenticLoopConfig::main_agent_generic();
        let model = MockCompletionModel::new(vec![MockResponse::text(
            "Hello from completion phase test.",
        )]);
        let history = vec![user_message("Say hello.")];
        let state = TurnState {
            iteration: 1,
            ..TurnState::default()
        };

        let mut accumulated_response = String::new();
        let mut accumulated_thinking = String::new();
        let mut total_usage = TokenUsage::default();

        let outcome = run(
            &state,
            &ctx,
            &config,
            &model,
            "You are a test bot.",
            &history,
            &[],
            &Span::none(),
            false,
            &mut accumulated_response,
            &mut accumulated_thinking,
            &mut total_usage,
        )
        .await
        .expect("completion phase must succeed for a healthy mock model");

        match outcome {
            CompletionOutcome::Continue { outcome, .. } => {
                assert!(
                    !outcome.has_tool_calls,
                    "text-only response must not produce tool calls"
                );
                assert!(
                    outcome
                        .text_content
                        .contains("Hello from completion phase test."),
                    "text content must come from the mock model, got: {:?}",
                    outcome.text_content
                );
            }
            CompletionOutcome::BreakAgentLoop => {
                panic!("text-only mock response must not break the loop")
            }
        }
        assert!(
            accumulated_response.contains("Hello from completion phase test."),
            "accumulated_response must reflect the streamed text"
        );
    }
}
