//! Database tracking for AI agent activity.
//!
//! Tool-call start/finish writes are awaited because harness gates immediately
//! consume them as deterministic evidence. Other telemetry remains background
//! best-effort and never blocks the agent loop.

mod helpers;
mod memory;
mod recording;
mod types;

pub use types::{BriefingPlan, MemoryHit, ScoredMemoryHit, ToolCallGuard};

use std::sync::Arc;

use crate::db_traits::{DbReadinessGate, DbTrackingBackend, TextEmbedder};
use parking_lot::RwLock;
use uuid::Uuid;

/// Lightweight handle passed through the agent loop for DB recording.
/// Harness-critical tool lifecycle methods are ordered/awaited; bulk telemetry
/// methods remain fire-and-forget.
/// Queries are gated on `DbReadinessGate` — if PG isn't ready yet, fire-and-forget
/// writes silently wait (up to a short timeout) rather than timing out against
/// the pool's acquire_timeout.
#[derive(Clone)]
pub struct DbTracker {
    pub(crate) backend: Arc<dyn DbTrackingBackend>,
    pub(crate) session_uuid: Arc<RwLock<Uuid>>,
    pub(crate) ready_gate: Box<dyn DbReadinessGate>,
    pub(crate) project_path: Option<String>,
    pub(crate) task_id: Option<Uuid>,
    pub(crate) subtask_id: Option<Uuid>,
    pub(crate) embedder: Option<Arc<dyn TextEmbedder>>,
    pub(crate) repo: Option<Arc<dyn crate::db_traits::DbRepoProvider>>,
}

impl DbTracker {
    pub fn new(
        backend: Arc<dyn DbTrackingBackend>,
        session_uuid: Uuid,
        ready_gate: impl DbReadinessGate + 'static,
    ) -> Self {
        Self {
            backend,
            session_uuid: Arc::new(RwLock::new(session_uuid)),
            ready_gate: Box::new(ready_gate),
            project_path: None,
            task_id: None,
            subtask_id: None,
            embedder: None,
            repo: None,
        }
    }

    pub fn set_embedder(&mut self, embedder: Arc<dyn TextEmbedder>) {
        self.embedder = Some(embedder);
    }

    pub fn embedder(&self) -> Option<&Arc<dyn TextEmbedder>> {
        self.embedder.as_ref()
    }

    pub fn set_repo(&mut self, repo: Arc<dyn crate::db_traits::DbRepoProvider>) {
        self.repo = Some(repo);
    }

    pub fn repo(&self) -> Option<&dyn crate::db_traits::DbRepoProvider> {
        self.repo.as_deref()
    }

    /// Override the session UUID this tracker stamps on recorded rows
    /// (`tool_calls`, etc.). Used by the headless `--stage-run` path to unify the
    /// tracker's session with the orchestrator's `session_id` (resolved from the
    /// chat-session key) so session-scoped gate cross-checks can read this run's
    /// tool calls — otherwise the tracker keeps the random uuid it was built with.
    pub fn set_session_uuid(&self, session_uuid: Uuid) {
        *self.session_uuid.write() = session_uuid;
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
        *self.session_uuid.read()
    }

    /// Current operation/task id (= `audit_log.run_id` grouping key for the
    /// evidence ledger hash chain). `None` outside a task scope.
    pub fn task_id(&self) -> Option<Uuid> {
        self.task_id
    }

    /// Project path scope for evidence rows, if set.
    pub fn project_path(&self) -> Option<&str> {
        self.project_path.as_deref()
    }

    pub fn backend(&self) -> &Arc<dyn DbTrackingBackend> {
        &self.backend
    }

    /// stage_run resume ledger: record that `org_id` passed `stage_kind` (its own
    /// gate) now. Awaited (low-frequency, correctness-sensitive) rather than
    /// fire-and-forget so the row is durable before the run reports the org passed.
    pub async fn record_org_stage_completion(
        &self,
        org_id: Uuid,
        stage_kind: &str,
        stage_run_id: Option<&str>,
    ) {
        self.backend
            .record_org_stage_completion(org_id, stage_kind, stage_run_id)
            .await;
    }

    /// stage_run resume ledger: most recent pass timestamp for `(org_id,
    /// stage_kind)`, or `None` if never completed. TTL is the caller's policy.
    pub async fn recent_org_stage_completion(
        &self,
        org_id: Uuid,
        stage_kind: &str,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        self.backend
            .recent_org_stage_completion(org_id, stage_kind)
            .await
    }

    pub fn ready_gate(&self) -> &dyn DbReadinessGate {
        &*self.ready_gate
    }
}
