//! Phase handlers for the turn state machine.
//!
//! Each phase is a small async function that mutates `TurnState` and
//! returns a `PhaseOutcome` telling the scheduler whether to continue,
//! skip to the next iteration, or break the loop.
//!
//! Extracted phases are added one PR at a time.

pub mod assistant_push;
pub mod compaction;
pub mod completion;
pub mod first_iter_hooks;
pub mod pre_flight;
pub mod reflector_or_break;
pub mod token_estimate;
pub mod tool_dispatch;

/// Why a phase asked the loop to break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakReason {
    /// User cancelled via the cancellation flag on the loop context.
    Cancelled,
    /// `iteration` exceeded `MAX_TOOL_ITERATIONS`.
    MaxIterations,
}

/// How a phase asks the scheduler to proceed.
#[derive(Debug)]
pub enum PhaseOutcome {
    /// Move on to the next phase (or the next iteration, if this was
    /// the last phase of the iteration).
    Continue,
    /// Terminate the loop. The scheduler is responsible for any
    /// finalization work; this variant just carries the reason for
    /// observability.
    Break(BreakReason),
    /// An unrecoverable error occurred. The scheduler must propagate
    /// this up the call stack (bubbling through `?` after matching).
    Fail(anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn break_reason_variants_are_distinct_and_copy() {
        assert_ne!(BreakReason::Cancelled, BreakReason::MaxIterations);
        let reason = BreakReason::Cancelled;
        let copied = reason;
        assert_eq!(reason, copied);
    }

    #[test]
    fn phase_outcome_variants_match_correctly() {
        assert!(matches!(PhaseOutcome::Continue, PhaseOutcome::Continue));
        assert!(matches!(
            PhaseOutcome::Break(BreakReason::MaxIterations),
            PhaseOutcome::Break(BreakReason::MaxIterations)
        ));
        let err = anyhow::anyhow!("boom");
        match PhaseOutcome::Fail(err) {
            PhaseOutcome::Fail(e) => assert_eq!(e.to_string(), "boom"),
            _ => panic!("expected Fail variant"),
        }
    }
}
