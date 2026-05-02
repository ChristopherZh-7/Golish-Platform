//! Phase handlers for the turn state machine.
//!
//! Each phase is a small async function that mutates `TurnState` and
//! returns a `PhaseOutcome` telling the scheduler whether to continue,
//! skip to the next iteration, or break the loop.
//!
//! Extracted phases are added one PR at a time — see the migration
//! plan in `docs/adr/0010-turn-executor-state-machine.md`.

pub mod compaction;
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
    /// An unrecoverable error occurred. The scheduler must propagate
    /// this up the call stack (bubbling through `?` after matching).
    Fail(anyhow::Error),
}
