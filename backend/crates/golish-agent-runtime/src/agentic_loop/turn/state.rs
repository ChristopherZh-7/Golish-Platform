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
#[derive(Debug)]
pub struct TurnState {
    /// Iteration counter, incremented by `pre_flight` at the start of
    /// every loop body entry. Starts at 0 so the first body sees 1.
    pub iteration: u32,
    /// Whether the reflector nudge is still in effect. Starts `true`;
    /// the `first_iter_hooks` phase may disable it on iteration 1 if
    /// the registered message hooks indicate so.
    pub reflector_active: bool,
}

impl Default for TurnState {
    fn default() -> Self {
        Self {
            iteration: 0,
            reflector_active: true,
        }
    }
}

impl TurnState {
    pub fn new() -> Self {
        Self::default()
    }
}
