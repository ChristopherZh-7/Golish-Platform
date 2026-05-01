//! App-layer implementation of `DbTrackingBackend` and `SubAgentChainPersistence`
//! backed by raw `sqlx` queries against the `PgPool`.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use golish_ai::db_traits::*;

// ============================================================================
// PgTrackingBackend
// ============================================================================

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
    async fn record_tool_call_start(&self, call_id: &str, session_id: Uuid, tool_name: &str, args: &serde_json::Value) {
        let res = sqlx::query(
            r#"INSERT INTO tool_calls (call_id, session_id, agent, name, args, status, source)
               VALUES ($1, $2, 'primary'::agent_type, $3, $4, 'running'::toolcall_status, 'ai')
               ON CONFLICT DO NOTHING"#,
        )
        .bind(call_id).bind(session_id).bind(tool_name).bind(args)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res { tracing::warn!("[db-track] tool_call_start: {e}"); }
    }

    async fn record_tool_call_finish(&self, call_id: &str, session_id: Uuid, status: &str, result: &str, duration_ms: i32) {
        let res = sqlx::query(
            r#"UPDATE tool_calls SET status = $1::toolcall_status, result = $2, duration_ms = $3, updated_at = NOW()
               WHERE call_id = $4 AND session_id = $5"#,
        )
        .bind(status).bind(result).bind(duration_ms).bind(call_id).bind(session_id)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res { tracing::warn!("[db-track] tool_call_finish: {e}"); }
    }

    async fn record_token_usage(&self, session_id: Uuid, model: &str, provider: &str, tokens_in: i32, tokens_out: i32, duration_ms: i32) {
        let res = sqlx::query(
            r#"INSERT INTO message_chains (session_id, agent, model, provider, tokens_in, tokens_out, duration_ms)
               VALUES ($1, 'primary'::agent_type, $2, $3, $4, $5, $6)"#,
        )
        .bind(session_id).bind(model).bind(provider).bind(tokens_in).bind(tokens_out).bind(duration_ms)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res { tracing::warn!("[db-track] token_usage: {e}"); }
    }

    async fn record_terminal_output(&self, session_id: Uuid, task_id: Option<Uuid>, subtask_id: Option<Uuid>, stream: &str, content: &str, project_path: &str) {
        let res = sqlx::query(
            r#"INSERT INTO terminal_logs (session_id, task_id, subtask_id, stream, content, project_path)
               VALUES ($1, $2, $3, $4::stream_type, $5, $6)"#,
        )
        .bind(session_id).bind(task_id).bind(subtask_id).bind(stream).bind(content).bind(project_path)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res { tracing::warn!("[db-track] terminal_output: {e}"); }
    }

    async fn record_search_log(&self, session_id: Uuid, task_id: Option<Uuid>, subtask_id: Option<Uuid>, engine: &str, query: &str, result: Option<&str>, project_path: &str) {
        let res = sqlx::query(
            r#"INSERT INTO search_logs (session_id, task_id, subtask_id, initiator, engine, query, result, project_path)
               VALUES ($1, $2, $3, 'primary'::agent_type, $4, $5, $6, $7)"#,
        )
        .bind(session_id).bind(task_id).bind(subtask_id).bind(engine).bind(query).bind(result).bind(project_path)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res { tracing::warn!("[db-track] search_log: {e}"); }
    }

    async fn record_audit(&self, action: &str, category: &str, details: &str, source: &str, session_id_str: &str, project_path: Option<&str>) {
        let res = sqlx::query(
            r#"INSERT INTO audit_log (action, category, details, source, session_id, project_path)
               VALUES ($1, $2, $3, $4, $5, $6)"#,
        )
        .bind(action).bind(category).bind(details).bind(source).bind(session_id_str).bind(project_path)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res { tracing::warn!("[db-track] audit: {e}"); }
    }

    async fn record_agent_call(&self, session_id: Uuid, initiator: &str, executor: &str, task: &str, result: Option<&str>, duration_ms: i32, project_path: &str) {
        let res = sqlx::query(
            r#"INSERT INTO agent_logs (session_id, initiator, executor, task, result, duration_ms, project_path)
               VALUES ($1, $2::agent_type, $3::agent_type, $4, $5, $6, $7)"#,
        )
        .bind(session_id).bind(initiator).bind(executor).bind(task).bind(result).bind(duration_ms).bind(project_path)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res { tracing::warn!("[db-track] agent_call: {e}"); }
    }

    async fn record_msg_log(&self, session_id: Uuid, task_id: Option<Uuid>, subtask_id: Option<Uuid>, agent: &str, msg_type: &str, message: &str, thinking: Option<&str>, project_path: Option<&str>) {
        let res = sqlx::query(
            r#"INSERT INTO msg_logs (session_id, task_id, subtask_id, agent, msg_type, message, thinking, project_path)
               VALUES ($1, $2, $3, $4::agent_type, $5::msglog_type, $6, $7, $8)"#,
        )
        .bind(session_id).bind(task_id).bind(subtask_id).bind(agent).bind(msg_type).bind(message).bind(thinking).bind(project_path)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res { tracing::warn!("[db-track] msg_log: {e}"); }
    }

    async fn record_vecstore_op(&self, session_id: Uuid, task_id: Option<Uuid>, subtask_id: Option<Uuid>, action: &str, query: &str, result_preview: &str, result_count: i32, project_path: Option<&str>) {
        let res = sqlx::query(
            r#"INSERT INTO vector_store_logs (session_id, task_id, subtask_id, action, query, result, result_count, project_path)
               VALUES ($1, $2, $3, $4::vecstore_action, $5, $6, $7, $8)"#,
        )
        .bind(session_id).bind(task_id).bind(subtask_id).bind(action).bind(query).bind(result_preview).bind(result_count).bind(project_path)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res { tracing::warn!("[db-track] vecstore_op: {e}"); }
    }

    async fn store_memory(&self, session_id: Uuid, content: &str, mem_type: &str, doc_type: &str, project_path: Option<&str>, metadata: Option<&serde_json::Value>, embedding_pgvector: Option<&str>) {
        let res = if let Some(emb) = embedding_pgvector {
            sqlx::query(
                r#"INSERT INTO memories (session_id, content, mem_type, doc_type, project_path, metadata, embedding)
                   VALUES ($1, $2, $3::memory_type, $4, $5, $6, $7::vector)"#,
            )
            .bind(session_id).bind(content).bind(mem_type).bind(doc_type).bind(project_path).bind(metadata).bind(emb)
            .execute(self.pool.as_ref()).await
        } else {
            sqlx::query(
                r#"INSERT INTO memories (session_id, content, mem_type, doc_type, project_path, metadata)
                   VALUES ($1, $2, $3::memory_type, $4, $5, $6)"#,
            )
            .bind(session_id).bind(content).bind(mem_type).bind(doc_type).bind(project_path).bind(metadata)
            .execute(self.pool.as_ref()).await
        };
        if let Err(e) = res { tracing::warn!("[db-track] store_memory: {e}"); }
    }

    async fn store_memory_with_tool(&self, session_id: Uuid, content: &str, mem_type: &str, tool_name: Option<&str>, project_path: Option<&str>, metadata: Option<&serde_json::Value>, embedding_pgvector: &str) {
        let res = sqlx::query(
            r#"INSERT INTO memories (session_id, content, mem_type, doc_type, tool_name, embedding, project_path, metadata)
               VALUES ($1, $2, $3::memory_type, 'tool_result', $4, $5::vector, $6, $7)"#,
        )
        .bind(session_id).bind(content).bind(mem_type).bind(tool_name).bind(embedding_pgvector).bind(project_path).bind(metadata)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res { tracing::warn!("[db-track] store_memory_with_tool: {e}"); }
    }

    async fn search_memories_text(&self, query: &str, project_path: Option<&str>, limit: i64) -> Vec<MemoryHit> {
        let pattern = format!("%{}%", query);
        sqlx::query_as::<_, PgMemoryHitRow>(
            r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
               FROM memories WHERE content ILIKE $1
               AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
               ORDER BY created_at DESC LIMIT $3"#,
        )
        .bind(&pattern).bind(project_path).bind(limit)
        .fetch_all(self.pool.as_ref()).await
        .unwrap_or_default()
        .into_iter().map(Into::into).collect()
    }

    async fn search_memories_semantic(&self, embedding_pgvector: &str, project_path: Option<&str>, limit: i64) -> Vec<ScoredMemoryHit> {
        let rows: Vec<PgScoredRow> = sqlx::query_as(
            r#"SELECT id, content, mem_type::TEXT as mem_type, tool_name, metadata, created_at,
                      1.0 - (embedding <=> $1::vector) AS score
               FROM memories WHERE embedding IS NOT NULL
               AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
               ORDER BY embedding <=> $1::vector ASC LIMIT $3"#,
        )
        .bind(embedding_pgvector).bind(project_path).bind(limit)
        .fetch_all(self.pool.as_ref()).await.unwrap_or_default();

        rows.into_iter().map(|r| ScoredMemoryHit {
            hit: MemoryHit { id: r.id, content: r.content, mem_type: r.mem_type, metadata: r.metadata, created_at: r.created_at },
            tool_name: r.tool_name,
            score: r.score,
        }).collect()
    }

    async fn search_memories_by_doc_type(&self, query: &str, doc_type: &str, sub_filter: Option<&str>, project_path: Option<&str>, limit: i64) -> Vec<MemoryHit> {
        let pattern = format!("%{}%", query);
        if let Some(sf) = sub_filter {
            let sf_pattern = format!("%{}%", sf);
            sqlx::query_as::<_, PgMemoryHitRow>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories WHERE doc_type = $1 AND content ILIKE $2 AND content ILIKE $3
                   AND ($4::text IS NULL OR project_path = $4 OR project_path IS NULL)
                   ORDER BY created_at DESC LIMIT $5"#,
            )
            .bind(doc_type).bind(&pattern).bind(&sf_pattern).bind(project_path).bind(limit)
            .fetch_all(self.pool.as_ref()).await.unwrap_or_default()
            .into_iter().map(Into::into).collect()
        } else {
            sqlx::query_as::<_, PgMemoryHitRow>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories WHERE doc_type = $1 AND content ILIKE $2
                   AND ($3::text IS NULL OR project_path = $3 OR project_path IS NULL)
                   ORDER BY created_at DESC LIMIT $4"#,
            )
            .bind(doc_type).bind(&pattern).bind(project_path).bind(limit)
            .fetch_all(self.pool.as_ref()).await.unwrap_or_default()
            .into_iter().map(Into::into).collect()
        }
    }

    async fn search_memories_text_with_category(&self, query: &str, category: Option<&str>, project_path: Option<&str>, limit: i64) -> Vec<MemoryHit> {
        let pattern = format!("%{}%", query);
        if let Some(cat) = category {
            let cat_pattern = format!("[{}]%", cat);
            sqlx::query_as::<_, PgMemoryHitRow>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories WHERE content ILIKE $1 AND content ILIKE $2
                   AND ($3::text IS NULL OR project_path = $3 OR project_path IS NULL)
                   ORDER BY created_at DESC LIMIT $4"#,
            )
            .bind(&pattern).bind(&cat_pattern).bind(project_path).bind(limit)
            .fetch_all(self.pool.as_ref()).await.unwrap_or_default()
            .into_iter().map(Into::into).collect()
        } else {
            self.search_memories_text(query, project_path, limit).await
        }
    }

    async fn search_memories_semantic_with_category(&self, category: Option<&str>, project_path: Option<&str>, embedding_pgvector: &str, limit: i64) -> Vec<MemoryHit> {
        if let Some(cat) = category {
            let cat_pattern = format!("[{}]%", cat);
            sqlx::query_as::<_, PgMemoryHitRow>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories WHERE embedding IS NOT NULL AND content ILIKE $1
                   AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
                   ORDER BY embedding <=> $3::vector ASC LIMIT $4"#,
            )
            .bind(&cat_pattern).bind(project_path).bind(embedding_pgvector).bind(limit)
            .fetch_all(self.pool.as_ref()).await.unwrap_or_default()
            .into_iter().map(Into::into).collect()
        } else {
            sqlx::query_as::<_, PgMemoryHitRow>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories WHERE embedding IS NOT NULL
                   AND ($1::text IS NULL OR project_path = $1 OR project_path IS NULL)
                   ORDER BY embedding <=> $2::vector ASC LIMIT $3"#,
            )
            .bind(project_path).bind(embedding_pgvector).bind(limit)
            .fetch_all(self.pool.as_ref()).await.unwrap_or_default()
            .into_iter().map(Into::into).collect()
        }
    }

    async fn fetch_memories_by_keyword(&self, keyword: &str, project_path: Option<&str>, limit: i64) -> Vec<MemoryHit> {
        let pattern = format!("%{}%", keyword);
        sqlx::query_as::<_, PgMemoryHitRow>(
            r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
               FROM memories WHERE content ILIKE $1
               AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
               ORDER BY created_at DESC LIMIT $3"#,
        )
        .bind(&pattern).bind(project_path).bind(limit)
        .fetch_all(self.pool.as_ref()).await.unwrap_or_default()
        .into_iter().map(Into::into).collect()
    }

    async fn fetch_active_plans(&self, project_path: &str) -> Vec<BriefingPlan> {
        sqlx::query_as::<_, PgBriefingPlanRow>(
            r#"SELECT title, description, steps, current_step, status::TEXT as status
               FROM execution_plans
               WHERE project_path = $1 AND status IN ('planning', 'in_progress', 'paused')
               ORDER BY updated_at DESC LIMIT 3"#,
        )
        .bind(project_path)
        .fetch_all(self.pool.as_ref()).await.unwrap_or_default()
        .into_iter().map(Into::into).collect()
    }

    async fn list_recent_memories(&self, category: Option<&str>, project_path: Option<&str>, limit: i64) -> Vec<MemoryHit> {
        if let Some(cat) = category {
            let cat_pattern = format!("[{}]%", cat);
            sqlx::query_as::<_, PgMemoryHitRow>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories WHERE content ILIKE $1
                   AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
                   ORDER BY created_at DESC LIMIT $3"#,
            )
            .bind(&cat_pattern).bind(project_path).bind(limit)
            .fetch_all(self.pool.as_ref()).await.unwrap_or_default()
            .into_iter().map(Into::into).collect()
        } else {
            sqlx::query_as::<_, PgMemoryHitRow>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories WHERE ($1::text IS NULL OR project_path = $1 OR project_path IS NULL)
                   ORDER BY created_at DESC LIMIT $2"#,
            )
            .bind(project_path).bind(limit)
            .fetch_all(self.pool.as_ref()).await.unwrap_or_default()
            .into_iter().map(Into::into).collect()
        }
    }

    async fn ensure_session(&self, session_id: Uuid) {
        let _ = sqlx::query("INSERT INTO sessions (id) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(session_id)
            .execute(self.pool.as_ref()).await;
    }

    async fn load_prompt_template_overrides(&self) -> Vec<(String, String)> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT template_name, content FROM prompt_templates WHERE is_active = true",
        )
        .fetch_all(self.pool.as_ref()).await.unwrap_or_default()
    }
}

// ============================================================================
// PgChainPersistence
// ============================================================================

pub struct PgChainPersistence {
    pool: Arc<PgPool>,
}

impl PgChainPersistence {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl golish_sub_agents::SubAgentChainPersistence for PgChainPersistence {
    async fn chain_create(
        &self, session_id: Uuid, task_id: Option<Uuid>, subtask_id: Option<Uuid>,
        agent_type: &str, _parent_chain_id: Option<Uuid>, _model: Option<&str>,
    ) -> anyhow::Result<Uuid> {
        let (id,): (Uuid,) = sqlx::query_as(
            r#"INSERT INTO message_chains (session_id, task_id, subtask_id, agent)
               VALUES ($1, $2, $3, $4::agent_type) RETURNING id"#,
        )
        .bind(session_id).bind(task_id).bind(subtask_id).bind(agent_type)
        .fetch_one(self.pool.as_ref()).await?;
        Ok(id)
    }

    async fn chain_update(&self, id: Uuid, chain_json: &serde_json::Value) -> anyhow::Result<()> {
        sqlx::query("UPDATE message_chains SET chain = $1, updated_at = NOW() WHERE id = $2")
            .bind(chain_json).bind(id)
            .execute(self.pool.as_ref()).await?;
        Ok(())
    }

    async fn chain_update_usage(
        &self, id: Uuid, input_tokens: i32, output_tokens: i32,
        _cache_read_tokens: i32, _input_cost: f64, _output_cost: f64, duration_ms: i32,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"UPDATE message_chains
               SET tokens_in = COALESCE(tokens_in, 0) + $1,
                   tokens_out = COALESCE(tokens_out, 0) + $2,
                   duration_ms = COALESCE(duration_ms, 0) + $3,
                   updated_at = NOW()
               WHERE id = $4"#,
        )
        .bind(input_tokens).bind(output_tokens).bind(duration_ms).bind(id)
        .execute(self.pool.as_ref()).await?;
        Ok(())
    }

    async fn load_prompt_template_overrides(&self) -> Vec<(String, String)> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT template_name, content FROM prompt_templates WHERE is_active = true",
        )
        .fetch_all(self.pool.as_ref()).await.unwrap_or_default()
    }
}

// ============================================================================
// Internal sqlx row types (kept here to avoid leaking sqlx into golish-ai)
// ============================================================================

#[derive(sqlx::FromRow)]
struct PgMemoryHitRow {
    id: Uuid,
    content: String,
    mem_type: String,
    metadata: Option<serde_json::Value>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<PgMemoryHitRow> for MemoryHit {
    fn from(r: PgMemoryHitRow) -> Self {
        Self { id: r.id, content: r.content, mem_type: r.mem_type, metadata: r.metadata, created_at: r.created_at }
    }
}

#[derive(sqlx::FromRow)]
struct PgScoredRow {
    id: Uuid,
    content: String,
    mem_type: String,
    tool_name: Option<String>,
    metadata: Option<serde_json::Value>,
    created_at: chrono::DateTime<chrono::Utc>,
    score: f32,
}

#[derive(sqlx::FromRow)]
struct PgBriefingPlanRow {
    title: String,
    description: Option<String>,
    steps: serde_json::Value,
    current_step: i32,
    status: String,
}

impl From<PgBriefingPlanRow> for BriefingPlan {
    fn from(r: PgBriefingPlanRow) -> Self {
        Self { title: r.title, description: r.description, steps: r.steps, current_step: r.current_step, status: r.status }
    }
}

// ============================================================================
// DbReadinessGate newtype wrapper for golish_core::DbReadyGate
// ============================================================================

#[derive(Clone)]
pub struct CoreDbReadyGate(pub golish_core::DbReadyGate);

#[async_trait]
impl golish_ai::db_traits::DbReadinessGate for CoreDbReadyGate {
    fn is_ready(&self) -> bool {
        self.0.is_ready()
    }
    fn is_failed(&self) -> bool {
        self.0.is_failed()
    }
    async fn wait(&mut self) -> bool {
        self.0.wait().await
    }
    fn clone_box(&self) -> Box<dyn golish_ai::db_traits::DbReadinessGate> {
        Box::new(self.clone())
    }
}
