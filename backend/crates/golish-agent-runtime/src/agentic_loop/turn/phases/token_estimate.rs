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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use golish_llm_providers::LlmClient;
    use rig::message::{Text, UserContent};
    use rig::one_or_many::OneOrMany;
    use tokio::sync::RwLock;

    use crate::test_utils::TestContextBuilder;

    use super::*;

    fn user_message(text: &str) -> Message {
        Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: text.to_string(),
            })),
        }
    }

    #[tokio::test]
    async fn estimates_match_system_plus_history_and_write_compaction_state() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let history = vec![user_message("hello world this is a test prompt")];
        let system_prompt = "You are a helpful assistant.";

        let outcome = run(&ctx, system_prompt, &history).await;
        assert!(matches!(outcome, PhaseOutcome::Continue));

        let snapshot = ctx.compaction_state.read().await;
        let recorded = snapshot
            .last_input_tokens
            .expect("token estimate must be recorded into compaction_state");
        assert!(
            recorded > 0,
            "non-trivial system prompt + history should yield > 0 tokens, got {}",
            recorded
        );
    }

    #[tokio::test]
    async fn empty_inputs_record_zero_tokens() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);

        let outcome = run(&ctx, "", &[]).await;
        assert!(matches!(outcome, PhaseOutcome::Continue));

        let snapshot = ctx.compaction_state.read().await;
        assert_eq!(snapshot.last_input_tokens, Some(0));
    }
}
