use std::time::Instant;

pub use crate::db_traits::{BriefingPlan, MemoryHit, ScoredMemoryHit};

/// Guard returned by `start_tool_call` to track timing.
pub struct ToolCallGuard {
    pub(super) call_id: String,
    pub(super) session_uuid: uuid::Uuid,
    pub(super) started_at: Instant,
}
