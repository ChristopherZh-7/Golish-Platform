//! Internal sqlx row types (kept here to avoid leaking sqlx into golish-agent
//! crates). Moved verbatim from `tracking_bridge.rs`; used by the `memory`
//! submodule's query helpers.

use uuid::Uuid;

use golish_agent_kit::db_traits::*;

#[derive(sqlx::FromRow)]
pub(super) struct PgMemoryHitRow {
    id: Uuid,
    content: String,
    mem_type: String,
    metadata: Option<serde_json::Value>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<PgMemoryHitRow> for MemoryHit {
    fn from(r: PgMemoryHitRow) -> Self {
        Self {
            id: r.id,
            content: r.content,
            mem_type: r.mem_type,
            metadata: r.metadata,
            created_at: r.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
pub(super) struct PgScoredRow {
    pub(super) id: Uuid,
    pub(super) content: String,
    pub(super) mem_type: String,
    pub(super) tool_name: Option<String>,
    pub(super) metadata: Option<serde_json::Value>,
    pub(super) created_at: chrono::DateTime<chrono::Utc>,
    pub(super) score: f32,
}

#[derive(sqlx::FromRow)]
pub(super) struct PgBriefingPlanRow {
    title: String,
    description: Option<String>,
    steps: serde_json::Value,
    current_step: i32,
    status: String,
}

impl From<PgBriefingPlanRow> for BriefingPlan {
    fn from(r: PgBriefingPlanRow) -> Self {
        Self {
            title: r.title,
            description: r.description,
            steps: r.steps,
            current_step: r.current_step,
            status: r.status,
        }
    }
}
