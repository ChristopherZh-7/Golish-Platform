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

mod executor;
mod interceptor;
mod phases;
mod state;

pub use executor::run_turn_loop;
pub use phases::{
    assistant_push as assistant_push_phase, compaction, completion,
    first_iter_hooks as first_iter_hooks_phase, pre_flight,
    reflector_or_break::{self as reflector_or_break_phase, ReflectorPhaseOutcome},
    token_estimate as token_estimate_phase, tool_dispatch as tool_dispatch_phase, PhaseOutcome,
};
pub use state::TurnState;
