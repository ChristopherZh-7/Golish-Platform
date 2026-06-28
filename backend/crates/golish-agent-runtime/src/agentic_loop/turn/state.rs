//! Mutable state owned by a single agent turn.
//!
//! This struct is intentionally grown incrementally:
//! - C1-1 (PoC): only `iteration` — prove the module boundary works.
//! - C1-2+: migrate more local variables from the main loop
//!   (`accumulated_response`, `accumulated_thinking`, `chat_history`,
//!   `total_usage`, counters, etc.)
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
    /// E1 · how many times this run injected a "stop repeating" recovery
    /// re-prompt after degenerate-repetition detection. Bounded by
    /// `MAX_REPETITION_RECOVERIES`.
    pub repetition_recoveries: u32,
    /// E2 · how many times this run retried after a retriable mid-stream
    /// error left truncated output. Bounded by `MAX_MID_STREAM_RETRIES`.
    pub mid_stream_retries: u32,
    /// Harness stage barrier: set once the agent dispatches
    /// `submit_stage_deliverable` during a harness stage. A subsequent idle turn
    /// then breaks the loop (stage attempt done → orchestrator runs the gate and
    /// advances the stage) instead of the reflector nudging the agent to keep
    /// working. Only meaningful while `ctx.harness_stage` is set; that tool only
    /// exists in harness stages, so this is inert in chat / non-harness runs.
    pub stage_deliverable_submitted: bool,
    /// Whether the optional one-shot `ctx.harness_forced_tool` has already been
    /// dispatched in this agent run. Once true, subsequent iterations fall back
    /// to normal tool-choice behavior so the stage can submit or repair.
    pub forced_tool_dispatched: bool,
}

impl Default for TurnState {
    fn default() -> Self {
        Self {
            iteration: 0,
            reflector_active: true,
            consecutive_no_tool_turns: 0,
            total_reflector_nudges: 0,
            repetition_recoveries: 0,
            mid_stream_retries: 0,
            stage_deliverable_submitted: false,
            forced_tool_dispatched: false,
        }
    }
}

impl TurnState {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_starts_at_iteration_zero_with_active_reflector() {
        let s = TurnState::default();
        assert_eq!(
            s.iteration, 0,
            "first pre_flight call must produce iteration=1"
        );
        assert!(
            s.reflector_active,
            "reflector starts active until first_iter_hooks decides otherwise"
        );
        assert_eq!(s.consecutive_no_tool_turns, 0);
        assert_eq!(s.total_reflector_nudges, 0);
    }

    #[test]
    fn new_matches_default() {
        let a = TurnState::new();
        let b = TurnState::default();
        assert_eq!(a.iteration, b.iteration);
        assert_eq!(a.reflector_active, b.reflector_active);
        assert_eq!(a.consecutive_no_tool_turns, b.consecutive_no_tool_turns);
        assert_eq!(a.total_reflector_nudges, b.total_reflector_nudges);
    }
}
