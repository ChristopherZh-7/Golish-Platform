// =============================================================================
// Helper functions for serde defaults
// =============================================================================

pub(super) fn default_true() -> bool {
    true
}

pub(super) fn default_context_enabled() -> bool {
    true
}

pub(super) fn default_compaction_threshold() -> f64 {
    0.80
}

pub(super) fn default_protected_turns() -> usize {
    2
}

pub(super) fn default_cooldown_seconds() -> u64 {
    60
}

pub(super) fn default_web_search_context_size() -> String {
    "medium".to_string()
}
