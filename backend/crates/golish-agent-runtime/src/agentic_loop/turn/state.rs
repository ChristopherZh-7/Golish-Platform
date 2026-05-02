//! Mutable state owned by a single agent turn.
//!
//! This struct is intentionally grown incrementally:
//! - C1-1 (PoC): only `iteration` — prove the module boundary works.
//! - C1-2+    : migrate more local variables from the main loop
//!              (`accumulated_response`, `accumulated_thinking`,
//!              `chat_history`, `total_usage`, counters, etc.)
//!
//! Every migration step must keep `cargo check --workspace --tests`
//! green; adding a field here without a consumer is fine.

/// Mutable state threaded through every phase of one agent turn.
///
/// Despite the name, this struct's lifetime is the **whole agent loop**
/// (from `run_agentic_loop_unified` entry to its final `break`), not a
/// single iteration. Counters that accumulate across iterations
/// (`consecutive_no_tool_turns`, `total_reflector_nudges`) live here for
/// the same reason: they are loop-wide and must be readable by every
/// phase. The `iteration` counter is the per-iteration index.
#[derive(Debug)]
pub struct TurnState {
    /// Iteration counter, incremented by `pre_flight` at the start of
    /// every loop body entry. Starts at 0 so the first body sees 1.
    pub iteration: u32,
    /// Whether the reflector nudge is still in effect. Starts `true`;
    /// the `first_iter_hooks` phase may disable it on iteration 1 if
    /// the registered message hooks indicate so.
    pub reflector_active: bool,
    /// Number of consecutive iterations that produced text without a
    /// tool call. Reset to 0 by `reflector_or_break` when a tool call
    /// shows up; incremented otherwise. Used to bound reflector budget.
    pub consecutive_no_tool_turns: u32,
    /// Total number of reflector corrections injected into the chat
    /// history during this agent run. Capped at 3 inside
    /// `maybe_run_reflector`.
    pub total_reflector_nudges: u32,
}

impl Default for TurnState {
    fn default() -> Self {
        Self {
            iteration: 0,
            reflector_active: true,
            consecutive_no_tool_turns: 0,
            total_reflector_nudges: 0,
        }
    }
}

impl TurnState {
    pub fn new() -> Self {
        Self::default()
    }
}
