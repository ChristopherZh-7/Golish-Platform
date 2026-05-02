//! Phase handlers for the turn state machine.
//!
//! Each phase is a small async function that mutates `TurnState` and
//! returns a `PhaseOutcome` telling the scheduler whether to continue,
//! skip to the next iteration, or break the loop.
//!
//! First extracted phase is `pre_flight` (PoC for ADR-0010). Additional
//! phases land in subsequent PRs — see the migration plan.

pub mod pre_flight;

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
}
