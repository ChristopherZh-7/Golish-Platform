use std::sync::Arc;
use std::time::Instant;

use sqlx::PgPool;
use uuid::Uuid;

/// Guard returned by `start_tool_call` to track timing.
pub struct ToolCallGuard {
    pub(super) pool: Arc<PgPool>,
    pub(super) session_uuid: Uuid,
    pub(super) call_id: String,
    pub(super) started_at: Instant,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct MemoryHit {
    pub id: Uuid,
    pub content: String,
    pub mem_type: String,
    pub metadata: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct BriefingPlan {
    pub title: String,
    pub description: Option<String>,
    pub steps: serde_json::Value,
    pub current_step: i32,
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScoredMemoryHit {
    pub hit: MemoryHit,
    pub tool_name: Option<String>,
    pub score: f32,
}

#[derive(Debug, sqlx::FromRow)]
pub(super) struct PgvectorScoredRow {
    pub id: Uuid,
    pub content: String,
    pub mem_type: String,
    pub tool_name: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub score: f32,
}
