//! Per-iteration token usage extraction + propagation.
//!
//! Updates `total_usage`, `compaction_state`, the LLM span, and emits a
//! `ContextWarning` event for the frontend. Falls back to a `tokenx-rs`
//! estimate when the provider didn't report usage in its `Final` chunk.

use rig::completion::{GetTokenUsage, Message};

use golish_context::token_budget::TokenUsage;
use golish_core::events::AiEvent;

use super::super::context::AgenticLoopContext;
use super::super::helpers::estimate_message_tokens;


/// Update `total_usage`, `compaction_state`, and the LLM span with token usage
/// extracted from the provider's `Final` chunk. Falls back to a `tokenx-rs`
/// estimate when the provider didn't report usage.
pub(super) async fn record_token_usage<R: GetTokenUsage>(
    ctx: &AgenticLoopContext<'_>,
    chat_history: &[Message],
    llm_span: &tracing::Span,
    iteration: usize,
    total_usage: &mut TokenUsage,
    resp: &R,
) {
    if let Some(usage) = resp.token_usage() {
        total_usage.input_tokens += usage.input_tokens;
        total_usage.output_tokens += usage.output_tokens;

        // Token usage as span attributes for Langfuse (prompt/completion_tokens
        // per GenAI semantic conventions).
        llm_span.record("gen_ai.usage.prompt_tokens", usage.input_tokens as i64);
        llm_span.record("gen_ai.usage.completion_tokens", usage.output_tokens as i64);
        tracing::info!(
            "[compaction] Token usage iter {}: input={}, output={}, cumulative={}",
            iteration,
            usage.input_tokens,
            usage.output_tokens,
            total_usage.total()
        );

        {
            let mut compaction_state = ctx.compaction_state.write().await;
            compaction_state.update_tokens(usage.input_tokens);
            tracing::info!(
                "[compaction] State updated: {} input tokens from provider",
                usage.input_tokens
            );
        }

        let model_config = golish_context::TokenBudgetConfig::for_model(ctx.llm.model_name);
        let max_tokens = model_config.max_context_tokens;
        let utilization = usage.input_tokens as f64 / max_tokens as f64;
        let _ = ctx.events.event_tx.send(AiEvent::ContextWarning {
            utilization,
            total_tokens: usage.input_tokens as usize,
            max_tokens,
        });
    } else {
        // Fallback: estimate from the chat history via tokenx-rs.
        // Roughly split 80/20 input/output as a reasonable approximation.
        let estimated_tokens: usize = chat_history.iter().map(estimate_message_tokens).sum();
        let estimated_input = (estimated_tokens as f64 * 0.8) as u64;
        let estimated_output = (estimated_tokens as f64 * 0.2) as u64;
        total_usage.input_tokens += estimated_input;
        total_usage.output_tokens += estimated_output;

        {
            let mut compaction_state = ctx.compaction_state.write().await;
            compaction_state.update_tokens_estimated(estimated_tokens as u64);
            tracing::info!(
                "[compaction] State updated (tokenx-rs estimate): ~{} estimated tokens",
                estimated_tokens,
            );
        }

        let model_config = golish_context::TokenBudgetConfig::for_model(ctx.llm.model_name);
        let max_tokens = model_config.max_context_tokens;
        let utilization = estimated_tokens as f64 / max_tokens as f64;
        let _ = ctx.events.event_tx.send(AiEvent::ContextWarning {
            utilization,
            total_tokens: estimated_tokens,
            max_tokens,
        });
    }
}
