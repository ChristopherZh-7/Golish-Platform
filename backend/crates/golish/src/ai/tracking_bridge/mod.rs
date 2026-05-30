//! App-layer implementation of `DbTrackingBackend` backed by raw `sqlx`
//! queries against the `PgPool`.
//!
//! The per-domain method bodies live in sibling inherent-impl modules
//! (`records` / `memory`) with a `_impl` suffix; this file holds the struct,
//! the `#[async_trait]` trait impl (thin delegation), and re-exports the
//! standalone `PgChainPersistence` / `CoreDbReadyGate` types (defined in the
//! `chain` / `ready_gate` submodules) plus the internal sqlx `rows`.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use golish_agent_kit::db_traits::*;

mod chain;
mod memory;
mod ready_gate;
mod records;
mod rows;

pub use chain::PgChainPersistence;
pub use ready_gate::CoreDbReadyGate;

pub struct PgTrackingBackend {
    pool: Arc<PgPool>,
}

impl PgTrackingBackend {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DbTrackingBackend for PgTrackingBackend {
    // ── Telemetry records ────────────────────────────────────
    async fn record_tool_call_start(
        &self,
        call_id: &str,
        session_id: Uuid,
        tool_name: &str,
        args: &serde_json::Value,
    ) {
        self.record_tool_call_start_impl(call_id, session_id, tool_name, args)
            .await
    }

    async fn record_tool_call_finish(
        &self,
        call_id: &str,
        session_id: Uuid,
        status: &str,
        result: &str,
        duration_ms: i32,
    ) {
        self.record_tool_call_finish_impl(call_id, session_id, status, result, duration_ms)
            .await
    }

    async fn record_token_usage(
        &self,
        session_id: Uuid,
        model: &str,
        provider: &str,
        tokens_in: i32,
        tokens_out: i32,
        duration_ms: i32,
    ) {
        self.record_token_usage_impl(
            session_id,
            model,
            provider,
            tokens_in,
            tokens_out,
            duration_ms,
        )
        .await
    }

    async fn record_terminal_output(
        &self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        stream: &str,
        content: &str,
        project_path: &str,
    ) {
        self.record_terminal_output_impl(
            session_id,
            task_id,
            subtask_id,
            stream,
            content,
            project_path,
        )
        .await
    }

    async fn record_search_log(
        &self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        engine: &str,
        query: &str,
        result: Option<&str>,
        project_path: &str,
    ) {
        self.record_search_log_impl(
            session_id,
            task_id,
            subtask_id,
            engine,
            query,
            result,
            project_path,
        )
        .await
    }

    async fn record_audit(
        &self,
        action: &str,
        category: &str,
        details: &str,
        source: &str,
        session_id_str: &str,
        project_path: Option<&str>,
    ) {
        self.record_audit_impl(
            action,
            category,
            details,
            source,
            session_id_str,
            project_path,
        )
        .await
    }

    async fn record_agent_call(
        &self,
        session_id: Uuid,
        initiator: &str,
        executor: &str,
        task: &str,
        result: Option<&str>,
        duration_ms: i32,
        project_path: &str,
    ) {
        self.record_agent_call_impl(
            session_id,
            initiator,
            executor,
            task,
            result,
            duration_ms,
            project_path,
        )
        .await
    }

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
    ) {
        self.record_msg_log_impl(
            session_id,
            task_id,
            subtask_id,
            agent,
            msg_type,
            message,
            thinking,
            project_path,
        )
        .await
    }

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
    ) {
        self.record_vecstore_op_impl(
            session_id,
            task_id,
            subtask_id,
            action,
            query,
            result_preview,
            result_count,
            project_path,
        )
        .await
    }

    // ── Memory / plans ───────────────────────────────────────
    async fn store_memory(
        &self,
        session_id: Uuid,
        content: &str,
        mem_type: &str,
        doc_type: &str,
        project_path: Option<&str>,
        metadata: Option<&serde_json::Value>,
        embedding_pgvector: Option<&str>,
    ) {
        self.store_memory_impl(
            session_id,
            content,
            mem_type,
            doc_type,
            project_path,
            metadata,
            embedding_pgvector,
        )
        .await
    }

    async fn store_memory_with_tool(
        &self,
        session_id: Uuid,
        content: &str,
        mem_type: &str,
        tool_name: Option<&str>,
        project_path: Option<&str>,
        metadata: Option<&serde_json::Value>,
        embedding_pgvector: &str,
    ) {
        self.store_memory_with_tool_impl(
            session_id,
            content,
            mem_type,
            tool_name,
            project_path,
            metadata,
            embedding_pgvector,
        )
        .await
    }

    async fn search_memories_text(
        &self,
        query: &str,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit> {
        self.search_memories_text_impl(query, project_path, limit)
            .await
    }

    async fn search_memories_semantic(
        &self,
        embedding_pgvector: &str,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<ScoredMemoryHit> {
        self.search_memories_semantic_impl(embedding_pgvector, project_path, limit)
            .await
    }

    async fn search_memories_by_doc_type(
        &self,
        query: &str,
        doc_type: &str,
        sub_filter: Option<&str>,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit> {
        self.search_memories_by_doc_type_impl(query, doc_type, sub_filter, project_path, limit)
            .await
    }

    async fn search_memories_text_with_category(
        &self,
        query: &str,
        category: Option<&str>,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit> {
        self.search_memories_text_with_category_impl(query, category, project_path, limit)
            .await
    }

    async fn search_memories_semantic_with_category(
        &self,
        category: Option<&str>,
        project_path: Option<&str>,
        embedding_pgvector: &str,
        limit: i64,
    ) -> Vec<MemoryHit> {
        self.search_memories_semantic_with_category_impl(
            category,
            project_path,
            embedding_pgvector,
            limit,
        )
        .await
    }

    async fn fetch_memories_by_keyword(
        &self,
        keyword: &str,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit> {
        self.fetch_memories_by_keyword_impl(keyword, project_path, limit)
            .await
    }

    async fn fetch_active_plans(&self, project_path: &str) -> Vec<BriefingPlan> {
        self.fetch_active_plans_impl(project_path).await
    }

    async fn list_recent_memories(
        &self,
        category: Option<&str>,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit> {
        self.list_recent_memories_impl(category, project_path, limit)
            .await
    }

    async fn ensure_session(&self, session_id: Uuid) {
        self.ensure_session_impl(session_id).await
    }

    async fn load_prompt_template_overrides(&self) -> Vec<(String, String)> {
        self.load_prompt_template_overrides_impl().await
    }
}
