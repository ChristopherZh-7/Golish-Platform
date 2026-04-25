//! Background database tracking for AI agent activity.
//!
//! Records tool calls, token usage, terminal output, web searches, and audit
//! entries to PostgreSQL without blocking the agent loop. All writes are spawned
//! as background tasks that log warnings on failure but never panic.

mod types;
mod helpers;
mod recording;
mod memory;

pub use types::{BriefingPlan, MemoryHit, ScoredMemoryHit, ToolCallGuard};

use std::sync::Arc;

use golish_db::DbReadyGate;
use golish_db::embeddings::Embedder;
use sqlx::PgPool;
use uuid::Uuid;

/// Lightweight handle passed through the agent loop for background DB recording.
/// All methods spawn fire-and-forget tasks so the agentic loop is never blocked.
/// Queries are gated on `DbReadyGate` — if PG isn't ready yet, fire-and-forget
/// writes silently wait (up to a short timeout) rather than timing out against
/// the pool's acquire_timeout.
#[derive(Clone)]
pub struct DbTracker {
    pub(crate) pool: Arc<PgPool>,
    pub(crate) session_uuid: Uuid,
    pub(crate) ready_gate: DbReadyGate,
    pub(crate) project_path: Option<String>,
    pub(crate) task_id: Option<Uuid>,
    pub(crate) subtask_id: Option<Uuid>,
    pub(crate) embedder: Option<Arc<dyn Embedder>>,
}

impl DbTracker {
    pub fn new(pool: Arc<PgPool>, session_uuid: Uuid, ready_gate: DbReadyGate) -> Self {
        Self {
            pool,
            session_uuid,
            ready_gate,
            project_path: None,
            task_id: None,
            subtask_id: None,
            embedder: None,
        }
    }

    pub fn set_embedder(&mut self, embedder: Arc<dyn Embedder>) {
        self.embedder = Some(embedder);
    }

    pub fn embedder(&self) -> Option<&Arc<dyn Embedder>> {
        self.embedder.as_ref()
    }

    pub fn with_project_path(mut self, path: Option<String>) -> Self {
        self.project_path = path;
        self
    }

    /// Set the current task scope for subsequent log writes.
    pub fn set_task_context(&mut self, task_id: Option<Uuid>, subtask_id: Option<Uuid>) {
        self.task_id = task_id;
        self.subtask_id = subtask_id;
    }

    /// Create a scoped clone with task context set.
    pub fn with_task_context(mut self, task_id: Option<Uuid>, subtask_id: Option<Uuid>) -> Self {
        self.task_id = task_id;
        self.subtask_id = subtask_id;
        self
    }

    pub fn session_uuid(&self) -> Uuid {
        self.session_uuid
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn pool_arc(&self) -> &Arc<PgPool> {
        &self.pool
    }

    pub fn ready_gate(&self) -> &DbReadyGate {
        &self.ready_gate
    }
}
