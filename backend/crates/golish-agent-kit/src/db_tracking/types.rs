use std::time::Instant;

pub use crate::db_traits::{BriefingPlan, MemoryHit, ScoredMemoryHit};

/// Guard returned by `start_tool_call` to track timing.
pub struct ToolCallGuard {
    pub record_id: Option<uuid::Uuid>,
    pub call_id: String,
    pub session_uuid: uuid::Uuid,
    pub started_at: Instant,
}

#[cfg(test)]
mod tests {
    use super::ToolCallGuard;
    use std::time::Instant;
    use uuid::Uuid;

    #[test]
    fn runtime_tool_tracking_guard_keeps_the_persisted_record_and_start_session_identity() {
        let record_id = Uuid::new_v4();
        let session_uuid = Uuid::new_v4();
        let guard = ToolCallGuard {
            record_id: Some(record_id),
            call_id: "request-call-id".to_string(),
            session_uuid,
            started_at: Instant::now(),
        };
        assert_eq!(guard.record_id, Some(record_id));
        assert_eq!(guard.session_uuid, session_uuid);
        assert_eq!(guard.call_id, "request-call-id");
    }
}
