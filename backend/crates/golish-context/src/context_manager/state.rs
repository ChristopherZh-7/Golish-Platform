//! Compaction state machine: turn-scoped flags + token estimates that drive
//! the decision to compact, and the [`CompactionCheck`] result describing
//! that decision.

/// State tracking for context compaction.
#[derive(Debug, Clone, Default)]
pub struct CompactionState {
    /// Whether compaction has been attempted this turn.
    pub attempted_this_turn: bool,
    /// Number of compactions performed this session.
    pub compaction_count: u32,
    /// Last known input token count from provider.
    pub last_input_tokens: Option<u64>,
    /// Whether we're using heuristic (no provider tokens available).
    pub using_heuristic: bool,
}

impl CompactionState {
    /// Create a new [`CompactionState`] with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the turn-specific state (called at the start of each turn).
    pub fn reset_turn(&mut self) {
        self.attempted_this_turn = false;
    }

    /// Mark that compaction has been attempted this turn.
    pub fn mark_attempted(&mut self) {
        self.attempted_this_turn = true;
    }

    /// Increment the compaction count (called after successful compaction).
    pub fn increment_count(&mut self) {
        self.compaction_count += 1;
    }

    /// Update token count from provider response.
    pub fn update_tokens(&mut self, input_tokens: u64) {
        self.last_input_tokens = Some(input_tokens);
        self.using_heuristic = false;
    }

    /// Update token count using local estimation (tokenx-rs).
    ///
    /// Called before LLM requests with a locally-computed token estimate to
    /// provide a leading indicator for compaction decisions.
    pub fn update_tokens_estimated(&mut self, estimated_tokens: u64) {
        self.last_input_tokens = Some(estimated_tokens);
        self.using_heuristic = true;
    }

    /// Update token count using heuristic estimation (`char_count / 4`).
    pub fn update_tokens_heuristic(&mut self, char_count: usize) {
        self.last_input_tokens = Some((char_count / 4) as u64);
        self.using_heuristic = true;
    }
}

/// Result of checking whether compaction should occur.
#[derive(Debug, Clone)]
pub struct CompactionCheck {
    /// Whether compaction should be triggered.
    pub should_compact: bool,
    /// Current token usage.
    pub current_tokens: u64,
    /// Maximum tokens for the model.
    pub max_tokens: usize,
    /// Threshold that was used (e.g., 0.80).
    pub threshold: f64,
    /// Whether tokens came from provider or heuristic.
    pub using_heuristic: bool,
    /// Reason for the decision.
    pub reason: String,
}
