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
#[derive(Debug, Default)]
pub struct TurnState {
    /// Iteration counter, incremented by `pre_flight` at the start of
    /// every loop body entry. Starts at 0 so the first body sees 1.
    pub iteration: u32,
}

impl TurnState {
    pub fn new() -> Self {
        Self::default()
    }
}
