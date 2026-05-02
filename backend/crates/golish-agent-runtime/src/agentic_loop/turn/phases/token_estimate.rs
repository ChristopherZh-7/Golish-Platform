//! TokenEstimate phase — proactively estimate input tokens before the
//! LLM call so compaction can fire one turn earlier than the provider's
//! own post-hoc token count would allow.
//!
//! The estimate sums `system_prompt` tokens with per-message estimates
//! from `helpers::estimate_message_tokens` and writes the result into
//! the shared `compaction_state` snapshot.

use rig::completion::Message;

use super::super::super::context::AgenticLoopContext;
use super::super::super::helpers::estimate_message_tokens;
use super::PhaseOutcome;

/// Estimate input tokens and record them into `compaction_state`.
pub async fn run(
    ctx: &AgenticLoopContext<'_>,
    system_prompt: &str,
    chat_history: &[Message],
) -> PhaseOutcome {
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

    PhaseOutcome::Continue
}
