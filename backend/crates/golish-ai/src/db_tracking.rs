//! Background database tracking for AI agent activity.
//!
//! Records tool calls, token usage, terminal output, web searches, and audit
//! entries to PostgreSQL without blocking the agent loop. All writes are spawned
//! as background tasks that log warnings on failure but never panic.

use std::sync::Arc;
use std::time::Instant;

use golish_db::DbReadyGate;
use sqlx::PgPool;
use uuid::Uuid;

/// Lightweight handle passed through the agent loop for background DB recording.
/// All methods spawn fire-and-forget tasks so the agentic loop is never blocked.
/// Queries are gated on `DbReadyGate` — if PG isn't ready yet, fire-and-forget
/// writes silently wait (up to a short timeout) rather than timing out against
/// the pool's acquire_timeout.
#[derive(Clone)]
pub struct DbTracker {
    pool: Arc<PgPool>,
    session_uuid: Uuid,
    ready_gate: DbReadyGate,
    project_path: Option<String>,
}

impl DbTracker {
    pub fn new(pool: Arc<PgPool>, session_uuid: Uuid, ready_gate: DbReadyGate) -> Self {
        Self {
            pool,
            session_uuid,
            ready_gate,
            project_path: None,
        }
    }

    pub fn with_project_path(mut self, path: Option<String>) -> Self {
        self.project_path = path;
        self
    }

    pub fn session_uuid(&self) -> Uuid {
        self.session_uuid
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn ready_gate(&self) -> &DbReadyGate {
        &self.ready_gate
    }

    // -- Tool calls --------------------------------------------------------

    /// Record a tool call start. Returns a guard with a start timestamp so
    /// `finish_tool_call` can compute duration.
    pub fn start_tool_call(
        &self,
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> ToolCallGuard {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let call_id_owned = call_id.to_string();
        let tool_name = tool_name.to_string();
        let args = args.clone();
        let mut gate = self.ready_gate.clone();

        let call_id_for_guard = call_id_owned.clone();
        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO tool_calls (call_id, session_id, agent, name, args, status, source)
                   VALUES ($1, $2, 'primary'::agent_type, $3, $4, 'running'::toolcall_status, 'ai')
                   ON CONFLICT DO NOTHING"#,
            )
            .bind(&call_id_owned)
            .bind(session_uuid)
            .bind(&tool_name)
            .bind(&args)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record tool call start: {e}");
            }
        });

        ToolCallGuard {
            pool: self.pool.clone(),
            session_uuid: self.session_uuid,
            call_id: call_id_for_guard,
            started_at: Instant::now(),
        }
    }

    /// Record a completed tool call result.
    pub fn finish_tool_call(&self, guard: ToolCallGuard, success: bool, result_text: &str) {
        let pool = guard.pool;
        let session_uuid = guard.session_uuid;
        let call_id = guard.call_id;
        let duration = guard.started_at.elapsed().as_millis() as i32;
        let status = if success { "finished" } else { "failed" };
        let result_text = truncate_for_db(result_text, 50_000);
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"UPDATE tool_calls
                   SET status = $1::toolcall_status, result = $2,
                       duration_ms = $3, updated_at = NOW()
                   WHERE call_id = $4 AND session_id = $5"#,
            )
            .bind(status)
            .bind(&result_text)
            .bind(duration)
            .bind(&call_id)
            .bind(session_uuid)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record tool call finish: {e}");
            }
        });
    }

    // -- Token usage / message chains --------------------------------------

    /// Record token usage for a single LLM turn.
    pub fn record_token_usage(
        &self,
        tokens_in: u64,
        tokens_out: u64,
        model: &str,
        provider: &str,
        duration_ms: u64,
    ) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let model = model.to_string();
        let provider = provider.to_string();
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO message_chains
                   (session_id, agent, model, provider, tokens_in, tokens_out, duration_ms)
                   VALUES ($1, 'primary'::agent_type, $2, $3, $4, $5, $6)"#,
            )
            .bind(session_uuid)
            .bind(&model)
            .bind(&provider)
            .bind(tokens_in as i32)
            .bind(tokens_out as i32)
            .bind(duration_ms as i32)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record token usage: {e}");
            }
        });
    }

    // -- Terminal logs -----------------------------------------------------

    /// Record terminal output (stdout/stderr) from a command execution.
    pub fn record_terminal_output(&self, stream: &str, content: &str) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let stream = stream.to_string();
        let content = truncate_for_db(content, 100_000);
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO terminal_logs (session_id, stream, content)
                   VALUES ($1, $2::stream_type, $3)"#,
            )
            .bind(session_uuid)
            .bind(&stream)
            .bind(&content)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record terminal output: {e}");
            }
        });
    }

    // -- Search logs -------------------------------------------------------

    /// Record a web search query and its result.
    pub fn record_search(&self, engine: &str, query: &str, result: Option<&str>) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let engine = engine.to_string();
        let query = query.to_string();
        let result = result.map(|r| truncate_for_db(r, 50_000));
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO search_logs (session_id, initiator, engine, query, result)
                   VALUES ($1, 'primary'::agent_type, $2, $3, $4)"#,
            )
            .bind(session_uuid)
            .bind(&engine)
            .bind(&query)
            .bind(&result)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record search log: {e}");
            }
        });
    }

    // -- Audit log ---------------------------------------------------------

    /// Record an audit log entry.
    pub fn audit(&self, action: &str, category: &str, details: &str) {
        self.audit_with_source(action, category, details, "ai");
    }

    /// Record an audit log entry with explicit source.
    pub fn audit_with_source(&self, action: &str, category: &str, details: &str, source: &str) {
        let pool = self.pool.clone();
        let action = action.to_string();
        let category = category.to_string();
        let details = details.to_string();
        let source = source.to_string();
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO audit_log (action, category, details, source)
                   VALUES ($1, $2, $3, $4)"#,
            )
            .bind(&action)
            .bind(&category)
            .bind(&details)
            .bind(&source)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record audit entry: {e}");
            }
        });
    }

    // -- Memory storage ----------------------------------------------------

    /// Store a memory observation (without embedding — text search only).
    pub fn store_memory(&self, content: &str, mem_type: &str, metadata: Option<serde_json::Value>) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let content = content.to_string();
        let mem_type = mem_type.to_string();
        let project_path = self.project_path.clone();
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO memories (session_id, content, mem_type, doc_type, project_path, metadata)
                   VALUES ($1, $2, $3::memory_type, 'memory', $4, $5)"#,
            )
            .bind(session_uuid)
            .bind(&content)
            .bind(&mem_type)
            .bind(&project_path)
            .bind(&metadata)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to store memory: {e}");
            }
        });
    }

    /// Store a global memory (project_path = NULL), visible across all projects.
    /// Use for general techniques, tool usage patterns, and reusable knowledge.
    pub fn store_memory_global(
        &self,
        content: &str,
        mem_type: &str,
        metadata: Option<serde_json::Value>,
    ) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let content = content.to_string();
        let mem_type = mem_type.to_string();
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO memories (session_id, content, mem_type, doc_type, project_path, metadata)
                   VALUES ($1, $2, $3::memory_type, 'memory', NULL, $4)"#,
            )
            .bind(session_uuid)
            .bind(&content)
            .bind(&mem_type)
            .bind(&metadata)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to store global memory: {e}");
            }
        });
    }

    /// Store a memory with an embedding vector for semantic search.
    /// Adapts to pgvector (native vector type) or BYTEA fallback automatically.
    pub fn store_memory_with_embedding(
        &self,
        content: &str,
        mem_type: &str,
        embedding: Vec<f32>,
        tool_name: Option<&str>,
        metadata: Option<serde_json::Value>,
    ) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let content = content.to_string();
        let mem_type = mem_type.to_string();
        let tool_name = tool_name.map(str::to_string);
        let project_path = self.project_path.clone();
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }

            let res = if gate.has_pgvector() {
                let emb_str = vec_to_pgvector(&embedding);
                sqlx::query(
                    r#"INSERT INTO memories
                       (session_id, content, mem_type, doc_type, tool_name,
                        embedding, project_path, metadata)
                       VALUES ($1, $2, $3::memory_type, 'tool_result', $4, $5::vector, $6, $7)"#,
                )
                .bind(session_uuid)
                .bind(&content)
                .bind(&mem_type)
                .bind(&tool_name)
                .bind(&emb_str)
                .bind(&project_path)
                .bind(&metadata)
                .execute(pool.as_ref())
                .await
            } else {
                let emb_bytes = embedding_to_bytes(&embedding);
                sqlx::query(
                    r#"INSERT INTO memories
                       (session_id, content, mem_type, doc_type, tool_name,
                        embedding, project_path, metadata)
                       VALUES ($1, $2, $3::memory_type, 'tool_result', $4, $5, $6, $7)"#,
                )
                .bind(session_uuid)
                .bind(&content)
                .bind(&mem_type)
                .bind(&tool_name)
                .bind(&emb_bytes)
                .bind(&project_path)
                .bind(&metadata)
                .execute(pool.as_ref())
                .await
            };

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to store memory with embedding: {e}");
            }
        });
    }

    /// Run the gatekeeper pipeline: decide whether to store a tool result as
    /// a long-term memory, build the structured content, and persist it.
    /// This is a fire-and-forget background task.
    pub fn maybe_store_tool_memory(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        result_value: &serde_json::Value,
        success: bool,
    ) {
        use golish_db::gatekeeper::{self, StoreDecision};
        use golish_db::models::ToolcallStatus;

        let status = if success {
            ToolcallStatus::Finished
        } else {
            ToolcallStatus::Failed
        };

        let decision = gatekeeper::should_store(tool_name, status);
        let mem_type = match decision {
            StoreDecision::Skip => return,
            StoreDecision::Store(t) | StoreDecision::StoreSummary(t) => t,
        };

        let result_text = match result_value {
            serde_json::Value::String(s) => s.clone(),
            _ => serde_json::to_string(result_value).unwrap_or_default(),
        };

        let filtered = match gatekeeper::filter_content(&result_text) {
            Some(c) => c,
            None => return,
        };

        let memory_content = gatekeeper::build_memory_content(tool_name, args, &filtered);

        let mem_type_str = format!("{:?}", mem_type).to_lowercase();
        let metadata = serde_json::json!({
            "tool_name": tool_name,
            "success": success,
        });

        self.store_memory(&memory_content, &mem_type_str, Some(metadata));
    }

    /// Semantic similarity search over memories.
    /// Uses pgvector's native `<=>` operator when available, falls back to
    /// Rust-side cosine similarity with BYTEA embeddings otherwise.
    pub async fn search_memories_semantic(
        &mut self,
        query_embedding: &[f32],
        limit: usize,
        threshold: f32,
    ) -> Vec<ScoredMemoryHit> {
        if !self.ready_gate.is_ready() {
            self.ready_gate.wait().await;
        }

        if self.ready_gate.has_pgvector() {
            self.search_pgvector(query_embedding, limit, threshold).await
        } else {
            self.search_bytea_fallback(query_embedding, limit, threshold)
                .await
        }
    }

    async fn search_pgvector(
        &self,
        query_embedding: &[f32],
        limit: usize,
        threshold: f32,
    ) -> Vec<ScoredMemoryHit> {
        let emb_str = vec_to_pgvector(query_embedding);
        let rows: Vec<PgvectorScoredRow> = sqlx::query_as(
            r#"SELECT id, content, mem_type::TEXT as mem_type, tool_name,
                      metadata, created_at,
                      1.0 - (embedding <=> $1::vector) AS score
               FROM memories
               WHERE embedding IS NOT NULL
                 AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
               ORDER BY embedding <=> $1::vector ASC
               LIMIT $3"#,
        )
        .bind(&emb_str)
        .bind(&self.project_path)
        .bind(limit as i64)
        .fetch_all(self.pool.as_ref())
        .await
        .unwrap_or_default();

        rows.into_iter()
            .filter(|r| r.score >= threshold)
            .map(|r| ScoredMemoryHit {
                hit: MemoryHit {
                    id: r.id,
                    content: r.content,
                    mem_type: r.mem_type,
                    metadata: r.metadata,
                    created_at: r.created_at,
                },
                tool_name: r.tool_name,
                score: r.score,
            })
            .collect()
    }

    async fn search_bytea_fallback(
        &self,
        query_embedding: &[f32],
        limit: usize,
        threshold: f32,
    ) -> Vec<ScoredMemoryHit> {
        let rows: Vec<ByteaEmbeddingRow> = sqlx::query_as(
            r#"SELECT id, content, mem_type::TEXT as mem_type, tool_name,
                      metadata, created_at, embedding
               FROM memories
               WHERE embedding IS NOT NULL
                 AND ($1::text IS NULL OR project_path = $1 OR project_path IS NULL)
               ORDER BY created_at DESC
               LIMIT $2"#,
        )
        .bind(&self.project_path)
        .bind((limit * 5) as i64) // fetch more, filter by score in Rust
        .fetch_all(self.pool.as_ref())
        .await
        .unwrap_or_default();

        let mut scored: Vec<ScoredMemoryHit> = rows
            .into_iter()
            .filter_map(|r| {
                let stored = bytes_to_embedding(&r.embedding?)?;
                let score = cosine_similarity(query_embedding, &stored);
                if score >= threshold {
                    Some(ScoredMemoryHit {
                        hit: MemoryHit {
                            id: r.id,
                            content: r.content,
                            mem_type: r.mem_type,
                            metadata: r.metadata,
                            created_at: r.created_at,
                        },
                        tool_name: r.tool_name,
                        score,
                    })
                } else {
                    None
                }
            })
            .collect();

        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        scored
    }

    /// Search memories by text content (ILIKE), scoped to current project + global.
    pub async fn search_memories_text(&mut self, query: &str, limit: i64) -> Vec<MemoryHit> {
        if !self.ready_gate.is_ready() {
            self.ready_gate.wait().await;
        }
        let pattern = format!("%{}%", query);
        sqlx::query_as::<_, MemoryHit>(
            r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
               FROM memories
               WHERE content ILIKE $1
                 AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
               ORDER BY created_at DESC
               LIMIT $3"#,
        )
        .bind(&pattern)
        .bind(&self.project_path)
        .bind(limit)
        .fetch_all(self.pool.as_ref())
        .await
        .unwrap_or_default()
    }

    /// Record a sub-agent execution in the agent_logs table.
    /// Fire-and-forget — errors are logged but don't propagate.
    pub fn record_agent_call(
        &self,
        initiator: &str,
        executor: &str,
        task: &str,
        result: Option<&str>,
        duration_ms: u64,
    ) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let initiator = initiator.to_string();
        let executor = executor.to_string();
        let task = task.to_string();
        let result = result.map(|r| r.to_string());
        let duration_ms = duration_ms as i32;
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO agent_logs (session_id, initiator, executor, task, result, duration_ms)
                   VALUES ($1, $2::agent_type, $3::agent_type, $4, $5, $6)"#,
            )
            .bind(session_uuid)
            .bind(&initiator)
            .bind(&executor)
            .bind(&task)
            .bind(&result)
            .bind(duration_ms)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record agent call: {e}");
            }
        });
    }

    /// Search memories by text content with optional category filter.
    /// Used by the `search_memories` AI tool. Scoped to current project + global.
    pub async fn search_memories_by_text(
        &self,
        query: &str,
        category: Option<&str>,
        limit: i64,
    ) -> Result<Vec<MemoryHit>, sqlx::Error> {
        let mut gate = self.ready_gate.clone();
        if !gate.is_ready() {
            gate.wait().await;
        }
        let pattern = format!("%{}%", query);

        if let Some(cat) = category {
            let cat_pattern = format!("[{}]%", cat);
            sqlx::query_as::<_, MemoryHit>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories
                   WHERE content ILIKE $1 AND content ILIKE $2
                     AND ($3::text IS NULL OR project_path = $3 OR project_path IS NULL)
                   ORDER BY created_at DESC
                   LIMIT $4"#,
            )
            .bind(&pattern)
            .bind(&cat_pattern)
            .bind(&self.project_path)
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await
        } else {
            sqlx::query_as::<_, MemoryHit>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories
                   WHERE content ILIKE $1
                     AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
                   ORDER BY created_at DESC
                   LIMIT $3"#,
            )
            .bind(&pattern)
            .bind(&self.project_path)
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await
        }
    }

    /// Fetch recent memories relevant to a sub-agent briefing.
    /// Searches by keyword and returns the most recent matches, scoped to current project.
    pub async fn fetch_memories_for_briefing(
        &self,
        keywords: &[&str],
        limit: i64,
    ) -> Vec<MemoryHit> {
        let mut gate = self.ready_gate.clone();
        if !gate.is_ready() {
            gate.wait().await;
        }

        let mut results: Vec<MemoryHit> = Vec::new();
        let per_keyword_limit = (limit / keywords.len().max(1) as i64).max(2);

        for keyword in keywords {
            if keyword.is_empty() {
                continue;
            }
            let pattern = format!("%{}%", keyword);
            match sqlx::query_as::<_, MemoryHit>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories
                   WHERE content ILIKE $1
                     AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
                   ORDER BY created_at DESC
                   LIMIT $3"#,
            )
            .bind(&pattern)
            .bind(&self.project_path)
            .bind(per_keyword_limit)
            .fetch_all(self.pool.as_ref())
            .await
            {
                Ok(rows) => {
                    for row in rows {
                        if !results.iter().any(|r| r.id == row.id) {
                            results.push(row);
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("[db-track] Briefing memory search for '{}' failed: {}", keyword, e);
                }
            }
        }

        results.truncate(limit as usize);
        results
    }

    /// Fetch active execution plans for the current project.
    pub async fn fetch_active_plans(&self) -> Vec<BriefingPlan> {
        let mut gate = self.ready_gate.clone();
        if !gate.is_ready() {
            gate.wait().await;
        }

        let project_path = match &self.project_path {
            Some(p) => p.clone(),
            None => return Vec::new(),
        };

        match sqlx::query_as::<_, BriefingPlan>(
            r#"SELECT title, description, steps, current_step, status::TEXT as status
               FROM execution_plans
               WHERE project_path = $1 AND status IN ('planning', 'in_progress', 'paused')
               ORDER BY updated_at DESC
               LIMIT 3"#,
        )
        .bind(&project_path)
        .fetch_all(self.pool.as_ref())
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::debug!("[db-track] Briefing plan fetch failed: {}", e);
                Vec::new()
            }
        }
    }

    /// List recent memories, optionally filtered by category.
    /// Used by the `list_memories` AI tool. Scoped to current project + global.
    pub async fn list_recent_memories(
        &self,
        category: Option<&str>,
        limit: i64,
    ) -> Result<Vec<MemoryHit>, sqlx::Error> {
        let mut gate = self.ready_gate.clone();
        if !gate.is_ready() {
            gate.wait().await;
        }

        if let Some(cat) = category {
            let cat_pattern = format!("[{}]%", cat);
            sqlx::query_as::<_, MemoryHit>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories
                   WHERE content ILIKE $1
                     AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
                   ORDER BY created_at DESC
                   LIMIT $3"#,
            )
            .bind(&cat_pattern)
            .bind(&self.project_path)
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await
        } else {
            sqlx::query_as::<_, MemoryHit>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories
                   WHERE ($1::text IS NULL OR project_path = $1 OR project_path IS NULL)
                   ORDER BY created_at DESC
                   LIMIT $2"#,
            )
            .bind(&self.project_path)
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await
        }
    }
}

/// Guard returned by `start_tool_call` to track timing.
pub struct ToolCallGuard {
    pool: Arc<PgPool>,
    session_uuid: Uuid,
    call_id: String,
    started_at: Instant,
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
struct PgvectorScoredRow {
    id: Uuid,
    content: String,
    mem_type: String,
    tool_name: Option<String>,
    metadata: Option<serde_json::Value>,
    created_at: chrono::DateTime<chrono::Utc>,
    score: f32,
}

#[derive(Debug, sqlx::FromRow)]
struct ByteaEmbeddingRow {
    id: Uuid,
    content: String,
    mem_type: String,
    tool_name: Option<String>,
    metadata: Option<serde_json::Value>,
    created_at: chrono::DateTime<chrono::Utc>,
    embedding: Option<Vec<u8>>,
}

/// Convert a Vec<f32> into pgvector's text format: `[0.1,0.2,...]`
fn vec_to_pgvector(v: &[f32]) -> String {
    let parts: Vec<String> = v.iter().map(|f| f.to_string()).collect();
    format!("[{}]", parts.join(","))
}

/// Serialize f32 embedding to bytes (little-endian) for BYTEA storage.
fn embedding_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Deserialize f32 embedding from BYTEA (little-endian).
fn bytes_to_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut norm_a, mut norm_b) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b.iter()) {
        let (x, y) = (*x as f64, *y as f64);
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = (norm_a * norm_b).sqrt();
    if denom < 1e-12 {
        0.0
    } else {
        (dot / denom) as f32
    }
}

fn truncate_for_db(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_bytes).collect();
        format!("{}... [truncated]", truncated)
    }
}

/// Wait for PG to become ready with a 60-second timeout.
/// Returns `true` if PG is ready, `false` if timed out (caller should skip the write).
async fn await_db_ready(gate: &mut DbReadyGate) -> bool {
    if gate.is_ready() {
        return true;
    }
    match tokio::time::timeout(std::time::Duration::from_secs(60), gate.wait()).await {
        Ok(()) => true,
        Err(_) => {
            tracing::warn!("[db-track] Timed out waiting for PostgreSQL readiness, skipping write");
            false
        }
    }
}
