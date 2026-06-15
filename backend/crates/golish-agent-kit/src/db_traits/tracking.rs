//! Fire-and-forget recording and memory operations trait.
//!
//! [`DbTrackingBackend`] abstracts all direct recording (tool calls, tokens,
//! terminal output, search logs, audit, agent calls, messages, vecstore) and
//! memory storage/search operations used by `golish-ai`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::types::{BriefingPlan, MemoryHit, ScoredMemoryHit};

/// Backend for all fire-and-forget recording and memory operations.
///
/// The application layer provides the concrete implementation.
/// `DbTracker` holds an `Arc<dyn DbTrackingBackend>` and delegates
/// every recording / memory method to it.
#[async_trait]
pub trait DbTrackingBackend: Send + Sync {
    // ── Recording (fire-and-forget) ─────────────────────────────────────

    async fn record_tool_call_start(
        &self,
        call_id: &str,
        session_id: Uuid,
        tool_name: &str,
        args: &serde_json::Value,
    );

    async fn record_tool_call_finish(
        &self,
        call_id: &str,
        session_id: Uuid,
        status: &str,
        result: &str,
        duration_ms: i32,
    );

    async fn record_token_usage(
        &self,
        session_id: Uuid,
        model: &str,
        provider: &str,
        tokens_in: i32,
        tokens_out: i32,
        duration_ms: i32,
    );

    async fn record_terminal_output(
        &self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        stream: &str,
        content: &str,
        project_path: &str,
    );

    async fn record_search_log(
        &self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        engine: &str,
        query: &str,
        result: Option<&str>,
        project_path: &str,
    );

    async fn record_audit(
        &self,
        action: &str,
        category: &str,
        details: &str,
        source: &str,
        session_id_str: &str,
        project_path: Option<&str>,
    );

    async fn record_agent_call(
        &self,
        session_id: Uuid,
        initiator: &str,
        executor: &str,
        task: &str,
        result: Option<&str>,
        duration_ms: i32,
        project_path: &str,
    );

    async fn record_msg_log(
        &self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        agent: &str,
        msg_type: &str,
        message: &str,
        thinking: Option<&str>,
        project_path: Option<&str>,
    );

    async fn record_vecstore_op(
        &self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        action: &str,
        query: &str,
        result_preview: &str,
        result_count: i32,
        project_path: Option<&str>,
    );

    // ── Memory storage ──────────────────────────────────────────────────

    async fn store_memory(
        &self,
        session_id: Uuid,
        content: &str,
        mem_type: &str,
        doc_type: &str,
        project_path: Option<&str>,
        metadata: Option<&serde_json::Value>,
        embedding_pgvector: Option<&str>,
    );

    async fn store_memory_with_tool(
        &self,
        session_id: Uuid,
        content: &str,
        mem_type: &str,
        tool_name: Option<&str>,
        project_path: Option<&str>,
        metadata: Option<&serde_json::Value>,
        embedding_pgvector: &str,
    );

    // ── Memory search ───────────────────────────────────────────────────

    async fn search_memories_text(
        &self,
        query: &str,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit>;

    async fn search_memories_semantic(
        &self,
        embedding_pgvector: &str,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<ScoredMemoryHit>;

    async fn search_memories_by_doc_type(
        &self,
        query: &str,
        doc_type: &str,
        sub_filter: Option<&str>,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit>;

    async fn search_memories_text_with_category(
        &self,
        query: &str,
        category: Option<&str>,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit>;

    async fn search_memories_semantic_with_category(
        &self,
        category: Option<&str>,
        project_path: Option<&str>,
        embedding_pgvector: &str,
        limit: i64,
    ) -> Vec<MemoryHit>;

    // ── Memory fetch ────────────────────────────────────────────────────

    async fn fetch_memories_by_keyword(
        &self,
        keyword: &str,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit>;

    async fn fetch_active_plans(&self, project_path: &str) -> Vec<BriefingPlan>;

    async fn list_recent_memories(
        &self,
        category: Option<&str>,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit>;

    // ── Session & prompt templates ──────────────────────────────────────

    async fn ensure_session(&self, session_id: Uuid);

    async fn load_prompt_template_overrides(&self) -> Vec<(String, String)>;

    // ── Per-(org, stage) completion ledger (stage_run resume-skip) ───────
    //
    // Default impls are no-op / "never completed" so backends that don't care
    // (tests, headless mocks) compile unchanged; the real `PgTrackingBackend`
    // overrides them against `org_stage_completions`.

    /// Record that `organization_id` passed `stage_kind` (its own gate) now.
    async fn record_org_stage_completion(
        &self,
        organization_id: Uuid,
        stage_kind: &str,
        stage_run_id: Option<&str>,
    ) {
        let _ = (organization_id, stage_kind, stage_run_id);
    }

    /// Most recent pass timestamp for `(organization_id, stage_kind)`, or `None`
    /// if the org has never completed this stage. TTL/freshness is the caller's
    /// policy (compare `passed_at` against now).
    async fn recent_org_stage_completion(
        &self,
        organization_id: Uuid,
        stage_kind: &str,
    ) -> Option<DateTime<Utc>> {
        let _ = (organization_id, stage_kind);
        None
    }
}
