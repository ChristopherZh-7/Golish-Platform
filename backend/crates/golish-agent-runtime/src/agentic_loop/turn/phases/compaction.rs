//! Compaction phase — decides pre-turn vs inter-turn compaction
//! based on the current iteration.
//!
//! Delegates to the existing `compaction_loop::{pre,inter}_turn_compaction`
//! helpers; the phase wrapper exists to keep the main loop body free
//! of conditional compaction logic.
//!
//! Behavior mapping:
//! - iteration == 1  → `pre_turn_compaction` (errors are swallowed,
//!   the turn proceeds regardless)
//! - iteration  > 1  → `inter_turn_compaction` (on a failed
//!   compaction that leaves context still over budget the phase
//!   returns `Fail`, carrying the `TerminalErrorEmitted` upstream)

use rig::completion::Message;

use super::super::super::compaction_loop::{inter_turn_compaction, pre_turn_compaction};
use super::super::super::context::AgenticLoopContext;
use super::super::state::TurnState;
use super::PhaseOutcome;

/// Run the appropriate compaction helper for this iteration.
pub async fn run(
    state: &TurnState,
    ctx: &AgenticLoopContext<'_>,
    chat_history: &mut Vec<Message>,
    accumulated_response: &str,
) -> PhaseOutcome {
    let iteration = state.iteration as usize;

    if iteration == 1 {
        pre_turn_compaction(ctx, chat_history).await;
        return PhaseOutcome::Continue;
    }

    // iteration > 1
    match inter_turn_compaction(ctx, chat_history, iteration, accumulated_response).await {
        Ok(()) => PhaseOutcome::Continue,
        Err(e) => PhaseOutcome::Fail(e),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use golish_llm_providers::LlmClient;
    use tokio::sync::RwLock;

    use crate::test_utils::TestContextBuilder;

    use super::*;

    #[tokio::test]
    async fn iteration_one_runs_pre_turn_path_and_continues() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let state = TurnState {
            iteration: 1,
            ..TurnState::default()
        };
        let mut history: Vec<Message> = vec![];

        let outcome = run(&state, &ctx, &mut history, "").await;

        assert!(matches!(outcome, PhaseOutcome::Continue));
        assert!(
            history.is_empty(),
            "no session_id => pre_turn_compaction is a no-op for empty history"
        );
    }

    #[tokio::test]
    async fn iteration_greater_than_one_runs_inter_turn_path_and_continues() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let state = TurnState {
            iteration: 5,
            ..TurnState::default()
        };
        let mut history: Vec<Message> = vec![];

        let outcome = run(&state, &ctx, &mut history, "partial").await;

        assert!(matches!(outcome, PhaseOutcome::Continue));
        assert!(
            history.is_empty(),
            "no session_id => inter_turn_compaction returns Ok(()) without touching history"
        );
    }
}
