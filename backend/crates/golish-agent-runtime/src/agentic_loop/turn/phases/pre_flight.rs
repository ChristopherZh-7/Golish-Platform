//! `PreFlight` phase — iteration bookkeeping + cancellation/budget guards.
//!
//! Runs at the start of every loop iteration, before compaction or any
//! LLM work. Keeps three concerns together because they are all cheap
//! and logically "pre-flight checks":
//!
//! 1. Increment the iteration counter in `TurnState`.
//! 2. Reset the compaction state snapshot for the new turn.
//! 3. Emit an error + break if the cancellation flag is set.
//! 4. Emit an error + break if `iteration > MAX_TOOL_ITERATIONS`.
//!
//! This is the first phase extracted under ADR-0010. The old inline
//! equivalent lived in the middle of `run_agentic_loop_unified`; see
//! the ADR's "Phase inventory" table for the full migration plan.

use tracing::Span;

use golish_core::events::AiEvent;

use super::super::super::context::AgenticLoopContext;
use super::super::state::TurnState;
use super::{BreakReason, PhaseOutcome};
use crate::agentic_loop::MAX_TOOL_ITERATIONS;

/// Advance the iteration counter and gate the loop on user
/// cancellation + the max-iteration budget.
pub async fn run(
    state: &mut TurnState,
    ctx: &AgenticLoopContext<'_>,
    agent_span: &Span,
) -> PhaseOutcome {
    state.iteration += 1;

    // Reset compaction state for this turn (preserves last_input_tokens).
    {
        let mut compaction_state = ctx.compaction_state.write().await;
        compaction_state.reset_turn();
    }

    // Cancellation check.
    if let Some(flag) = &ctx.cancelled {
        if flag.load(std::sync::atomic::Ordering::SeqCst) {
            tracing::info!(
                "Agent loop cancelled by user (iteration {})",
                state.iteration
            );
            let _ = ctx.events.event_tx.send(AiEvent::Error {
                message: "Agent stopped by user".to_string(),
                error_type: "cancelled".to_string(),
            });
            return PhaseOutcome::Break(BreakReason::Cancelled);
        }
    }

    // Max-iteration budget.
    if state.iteration > MAX_TOOL_ITERATIONS as u32 {
        let _max_iter_event = tracing::info_span!(
            parent: agent_span,
            "max_iterations_reached",
            "langfuse.observation.type" = "event",
            "langfuse.session.id" = ctx.events.session_id.unwrap_or(""),
            max_iterations = MAX_TOOL_ITERATIONS,
        );
        let _ = ctx.events.event_tx.send(AiEvent::Error {
            message: "Maximum tool iterations reached".to_string(),
            error_type: "max_iterations".to_string(),
        });
        return PhaseOutcome::Break(BreakReason::MaxIterations);
    }

    PhaseOutcome::Continue
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use golish_llm_providers::LlmClient;
    use tokio::sync::RwLock;

    use crate::test_utils::TestContextBuilder;

    use super::*;

    #[tokio::test]
    async fn happy_path_increments_iteration_and_returns_continue() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let mut state = TurnState::new();

        let outcome = run(&mut state, &ctx, &Span::none()).await;

        assert!(matches!(outcome, PhaseOutcome::Continue));
        assert_eq!(state.iteration, 1, "first call must produce iteration=1");

        let outcome2 = run(&mut state, &ctx, &Span::none()).await;
        assert!(matches!(outcome2, PhaseOutcome::Continue));
        assert_eq!(state.iteration, 2, "second call advances counter");
    }

    #[tokio::test]
    async fn cancellation_flag_breaks_with_cancelled_reason_and_emits_error() {
        let mut test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let cancel_flag = Arc::new(AtomicBool::new(true));
        let mut ctx = test_ctx.as_agentic_context_with_client(&client);
        ctx.cancelled = Some(&cancel_flag);
        let mut state = TurnState::new();

        let outcome = run(&mut state, &ctx, &Span::none()).await;

        assert!(matches!(
            outcome,
            PhaseOutcome::Break(BreakReason::Cancelled)
        ));
        let cancel_events = test_ctx.find_events(|e| {
            matches!(e, AiEvent::Error { error_type, .. } if error_type == "cancelled")
        });
        assert_eq!(cancel_events.len(), 1, "exactly one cancelled error event");
    }

    #[tokio::test]
    async fn cancellation_flag_unset_does_not_break() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let mut ctx = test_ctx.as_agentic_context_with_client(&client);
        ctx.cancelled = Some(&cancel_flag);
        let mut state = TurnState::new();

        let outcome = run(&mut state, &ctx, &Span::none()).await;

        assert!(matches!(outcome, PhaseOutcome::Continue));
        assert!(!cancel_flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn iteration_over_budget_breaks_with_max_iterations() {
        let mut test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let mut state = TurnState {
            iteration: MAX_TOOL_ITERATIONS as u32,
            ..TurnState::default()
        };

        let outcome = run(&mut state, &ctx, &Span::none()).await;

        assert!(matches!(
            outcome,
            PhaseOutcome::Break(BreakReason::MaxIterations)
        ));
        assert_eq!(state.iteration, (MAX_TOOL_ITERATIONS as u32) + 1);
        let max_iter_events = test_ctx.find_events(|e| {
            matches!(e, AiEvent::Error { error_type, .. } if error_type == "max_iterations")
        });
        assert_eq!(max_iter_events.len(), 1);
    }
}
