//! Turn executor state machine primitives (see ADR-0010).
//!
//! This module hosts the incremental extraction of the 300-line
//! `run_agentic_loop_unified` function into a phase-based state
//! machine. Start small: the first PoC (`phases::pre_flight`) handles
//! iteration counting, cancellation, and max-iteration budget. Other
//! phases migrate in follow-up PRs (see the migration plan in the ADR).
//!
//! Design goal: each phase becomes an independently-testable unit,
//! `mod.rs` of `agentic_loop` eventually shrinks to ~150 LOC of
//! phase scheduling.

mod phases;
mod state;

pub use phases::{
    compaction, first_iter_hooks as first_iter_hooks_phase, pre_flight,
    token_estimate as token_estimate_phase, BreakReason, PhaseOutcome,
};
pub use state::TurnState;
