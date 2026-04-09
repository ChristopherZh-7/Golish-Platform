//! Background database tracking for AI agent activity.
//!
//! Records tool calls, token usage, terminal output, web searches, and audit
//! entries to PostgreSQL without blocking the agent loop. All writes are spawned
//! as background tasks that log warnings on failure but never panic.

use std::sync::Arc;
use std::time::Instant;

use sqlx::PgPool;
use uuid::Uuid;

/// Lightweight handle passed through the agent loop for background DB recording.
/// All methods spawn fire-and-forget tasks so the agentic loop is never blocked.
#[derive(Clone)]
pub struct DbTracker {
    pool: Arc<PgPool>,
    session_uuid: Uuid,
}

impl DbTracker {
    pub fn new(pool: Arc<PgPool>, session_uuid: Uuid) -> Self {
        Self { pool, session_uuid }
    }

    pub fn session_uuid(&self) -> Uuid {
        self.session_uuid
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
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

        let call_id_for_guard = call_id_owned.clone();
        tokio::spawn(async move {
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

        tokio::spawn(async move {
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

        tokio::spawn(async move {
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

        tokio::spawn(async move {
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

        tokio::spawn(async move {
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

        tokio::spawn(async move {
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

        tokio::spawn(async move {
            let res = sqlx::query(
                r#"INSERT INTO memories (session_id, content, mem_type, doc_type, metadata)
                   VALUES ($1, $2, $3::memory_type, 'memory', $4)"#,
            )
            .bind(session_uuid)
            .bind(&content)
            .bind(&mem_type)
            .bind(&metadata)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to store memory: {e}");
            }
        });
    }

    /// Store a memory with an embedding vector for semantic search.
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
        let emb_dim = embedding.len() as i32;
        let emb_bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();

        tokio::spawn(async move {
            let res = sqlx::query(
                r#"INSERT INTO memories
                   (session_id, content, mem_type, doc_type, tool_name,
                    embedding, embedding_dim, metadata)
                   VALUES ($1, $2, $3::memory_type, 'memory', $4, $5, $6, $7)"#,
            )
            .bind(session_uuid)
            .bind(&content)
            .bind(&mem_type)
            .bind(&tool_name)
            .bind(&emb_bytes)
            .bind(emb_dim)
            .bind(&metadata)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to store memory with embedding: {e}");
            }
        });
    }

    /// Semantic similarity search over memories.
    /// Loads candidate embeddings from DB and computes cosine similarity in Rust.
    pub async fn search_memories_semantic(
        &self,
        query_embedding: &[f32],
        limit: usize,
        threshold: f32,
    ) -> Vec<ScoredMemoryHit> {
        #[derive(Debug, sqlx::FromRow)]
        struct CandRow {
            id: Uuid,
            content: String,
            mem_type: String,
            tool_name: Option<String>,
            metadata: Option<serde_json::Value>,
            created_at: chrono::DateTime<chrono::Utc>,
            embedding: Option<Vec<u8>>,
            embedding_dim: Option<i32>,
        }

        let rows: Vec<CandRow> = sqlx::query_as(
            r#"SELECT id, content, mem_type::TEXT as mem_type, tool_name,
                      metadata, created_at, embedding, embedding_dim
               FROM memories
               WHERE embedding IS NOT NULL AND session_id = $1
               ORDER BY created_at DESC
               LIMIT 500"#,
        )
        .bind(self.session_uuid)
        .fetch_all(self.pool.as_ref())
        .await
        .unwrap_or_default();

        let mut scored: Vec<ScoredMemoryHit> = rows
            .into_iter()
            .filter_map(|r| {
                let emb = decode_emb(&r.embedding?, r.embedding_dim?)?;
                let score = cosine_sim(query_embedding, &emb);
                if score < threshold {
                    return None;
                }
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
            })
            .collect();

        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        scored
    }

    /// Search memories by text content (ILIKE).
    pub async fn search_memories_text(&self, query: &str, limit: i64) -> Vec<MemoryHit> {
        let pattern = format!("%{}%", query);
        sqlx::query_as::<_, MemoryHit>(
            r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
               FROM memories
               WHERE content ILIKE $1
               ORDER BY created_at DESC
               LIMIT $2"#,
        )
        .bind(&pattern)
        .bind(limit)
        .fetch_all(self.pool.as_ref())
        .await
        .unwrap_or_default()
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScoredMemoryHit {
    pub hit: MemoryHit,
    pub tool_name: Option<String>,
    pub score: f32,
}

fn decode_emb(bytes: &[u8], dim: i32) -> Option<Vec<f32>> {
    let dim = dim as usize;
    if bytes.len() != dim * 4 {
        return None;
    }
    let mut vec = Vec::with_capacity(dim);
    for chunk in bytes.chunks_exact(4) {
        vec.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Some(vec)
}

fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0_f32, 0.0_f32, 0.0_f32);
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let d = na.sqrt() * nb.sqrt();
    if d == 0.0 { 0.0 } else { dot / d }
}

fn truncate_for_db(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_bytes).collect();
        format!("{}... [truncated]", truncated)
    }
}
