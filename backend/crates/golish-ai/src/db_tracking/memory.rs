//! Memory storage and retrieval: storing observations, semantic search,
//! text search, hybrid search, briefing fetch, and plan fetch.

use super::helpers::{await_db_ready, vec_to_pgvector};
use super::types::{BriefingPlan, MemoryHit, PgvectorScoredRow, ScoredMemoryHit};
use super::DbTracker;

impl DbTracker {
    /// Store a memory observation. When an embedder is configured, generates and
    /// stores an embedding vector alongside the text for semantic search.
    pub fn store_memory(&self, content: &str, mem_type: &str, metadata: Option<serde_json::Value>) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let content = content.to_string();
        let mem_type = mem_type.to_string();
        let project_path = self.project_path.clone();
        let mut gate = self.ready_gate.clone();
        let embedder = self.embedder.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }

            let embedding = if let Some(ref emb) = embedder {
                match emb.embed(&content).await {
                    Ok(v) => Some(v),
                    Err(e) => {
                        tracing::warn!("[db-track] Embedding generation failed, storing text-only: {e}");
                        None
                    }
                }
            } else {
                None
            };

            let res = if let Some(ref emb_vec) = embedding {
                let emb_str = vec_to_pgvector(emb_vec);
                sqlx::query(
                    r#"INSERT INTO memories
                       (session_id, content, mem_type, doc_type, project_path, metadata, embedding)
                       VALUES ($1, $2, $3::memory_type, 'memory', $4, $5, $6::vector)"#,
                )
                .bind(session_uuid)
                .bind(&content)
                .bind(&mem_type)
                .bind(&project_path)
                .bind(&metadata)
                .bind(&emb_str)
                .execute(pool.as_ref())
                .await
            } else {
                sqlx::query(
                    r#"INSERT INTO memories (session_id, content, mem_type, doc_type, project_path, metadata)
                       VALUES ($1, $2, $3::memory_type, 'memory', $4, $5)"#,
                )
                .bind(session_uuid)
                .bind(&content)
                .bind(&mem_type)
                .bind(&project_path)
                .bind(&metadata)
                .execute(pool.as_ref())
                .await
            };

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
        let embedder = self.embedder.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }

            let embedding = if let Some(ref emb) = embedder {
                emb.embed(&content).await.ok()
            } else {
                None
            };

            let res = if let Some(ref emb_vec) = embedding {
                let emb_str = vec_to_pgvector(emb_vec);
                sqlx::query(
                    r#"INSERT INTO memories
                       (session_id, content, mem_type, doc_type, project_path, metadata, embedding)
                       VALUES ($1, $2, $3::memory_type, 'memory', NULL, $4, $5::vector)"#,
                )
                .bind(session_uuid)
                .bind(&content)
                .bind(&mem_type)
                .bind(&metadata)
                .bind(&emb_str)
                .execute(pool.as_ref())
                .await
            } else {
                sqlx::query(
                    r#"INSERT INTO memories (session_id, content, mem_type, doc_type, project_path, metadata)
                       VALUES ($1, $2, $3::memory_type, 'memory', NULL, $4)"#,
                )
                .bind(session_uuid)
                .bind(&content)
                .bind(&mem_type)
                .bind(&metadata)
                .execute(pool.as_ref())
                .await
            };

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to store global memory: {e}");
            }
        });
    }

    /// Store a memory with a specific `doc_type` (for multi-vector store: "code", "guide").
    pub fn store_memory_with_doc_type(
        &self,
        content: &str,
        mem_type: &str,
        doc_type: &str,
        metadata: Option<serde_json::Value>,
    ) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let content = content.to_string();
        let mem_type = mem_type.to_string();
        let doc_type = doc_type.to_string();
        let project_path = self.project_path.clone();
        let mut gate = self.ready_gate.clone();
        let embedder = self.embedder.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }

            let embedding = if let Some(ref emb) = embedder {
                emb.embed(&content).await.ok()
            } else {
                None
            };

            let res = if let Some(ref emb_vec) = embedding {
                let emb_str = vec_to_pgvector(emb_vec);
                sqlx::query(
                    r#"INSERT INTO memories
                       (session_id, content, mem_type, doc_type, project_path, metadata, embedding)
                       VALUES ($1, $2, $3::memory_type, $4, $5, $6, $7::vector)"#,
                )
                .bind(session_uuid)
                .bind(&content)
                .bind(&mem_type)
                .bind(&doc_type)
                .bind(&project_path)
                .bind(&metadata)
                .bind(&emb_str)
                .execute(pool.as_ref())
                .await
            } else {
                sqlx::query(
                    r#"INSERT INTO memories (session_id, content, mem_type, doc_type, project_path, metadata)
                       VALUES ($1, $2, $3::memory_type, $4, $5, $6)"#,
                )
                .bind(session_uuid)
                .bind(&content)
                .bind(&mem_type)
                .bind(&doc_type)
                .bind(&project_path)
                .bind(&metadata)
                .execute(pool.as_ref())
                .await
            };

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to store {} memory: {e}", doc_type);
            }
        });
    }

    /// Search memories filtered by `doc_type` (for multi-vector store).
    /// Optional `sub_filter` is matched against metadata or content tags.
    pub async fn search_memories_by_doc_type(
        &self,
        query: &str,
        doc_type: &str,
        sub_filter: Option<&str>,
        limit: i64,
    ) -> Result<Vec<MemoryHit>, sqlx::Error> {
        let mut gate = self.ready_gate.clone();
        if !gate.is_ready() && !gate.wait().await {
            return Ok(Vec::new());
        }
        let pattern = format!("%{}%", query);

        if let Some(sf) = sub_filter {
            let sf_pattern = format!("%{}%", sf);
            sqlx::query_as::<_, MemoryHit>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories
                   WHERE doc_type = $1
                     AND content ILIKE $2
                     AND content ILIKE $3
                     AND ($4::text IS NULL OR project_path = $4 OR project_path IS NULL)
                   ORDER BY created_at DESC
                   LIMIT $5"#,
            )
            .bind(doc_type)
            .bind(&pattern)
            .bind(&sf_pattern)
            .bind(&self.project_path)
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await
        } else {
            sqlx::query_as::<_, MemoryHit>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories
                   WHERE doc_type = $1
                     AND content ILIKE $2
                     AND ($3::text IS NULL OR project_path = $3 OR project_path IS NULL)
                   ORDER BY created_at DESC
                   LIMIT $4"#,
            )
            .bind(doc_type)
            .bind(&pattern)
            .bind(&self.project_path)
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await
        }
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
        self.record_vecstore_op(
            "store",
            &format!("{}:{}", mem_type, tool_name.unwrap_or("unknown")),
            1,
            &content[..content.len().min(200)],
        );

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

            let emb_str = vec_to_pgvector(&embedding);
            let res = sqlx::query(
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
            .await;

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

    /// Semantic similarity search over memories using pgvector's `<=>` operator.
    pub async fn search_memories_semantic(
        &mut self,
        query_embedding: &[f32],
        limit: usize,
        threshold: f32,
    ) -> Vec<ScoredMemoryHit> {
        if !self.ready_gate.is_ready() {
            if !self.ready_gate.wait().await {
                return Vec::new();
            }
        }

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

    /// Search memories by text content (ILIKE), scoped to current project + global.
    pub async fn search_memories_text(&mut self, query: &str, limit: i64) -> Vec<MemoryHit> {
        if !self.ready_gate.is_ready() {
            if !self.ready_gate.wait().await {
                return Vec::new();
            }
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

    /// Search memories by text content with optional category filter.
    /// Used by the `search_memories` AI tool. Scoped to current project + global.
    ///
    /// When an embedder is available, performs hybrid search: semantic + text with
    /// deduplicated, interleaved results. Falls back to pure text (ILIKE) otherwise.
    pub async fn search_memories_by_text(
        &self,
        query: &str,
        category: Option<&str>,
        limit: i64,
    ) -> Result<Vec<MemoryHit>, sqlx::Error> {
        self.record_vecstore_op(
            "search",
            query,
            0,
            &format!("category={}", category.unwrap_or("all")),
        );

        let mut gate = self.ready_gate.clone();
        if !gate.is_ready() && !gate.wait().await {
            return Ok(Vec::new());
        }

        if let Some(ref embedder) = self.embedder {
            match embedder.embed(query).await {
                Ok(embedding) => {
                    tracing::debug!(
                        "[memory-search] Using hybrid (semantic + text) search, dim={}",
                        embedding.len()
                    );
                    return self.hybrid_search(query, &embedding, category, limit).await;
                }
                Err(e) => {
                    tracing::warn!(
                        "[memory-search] Embedding generation failed, falling back to text: {e}"
                    );
                }
            }
        }

        self.text_only_search(query, category, limit).await
    }

    async fn hybrid_search(
        &self,
        query: &str,
        embedding: &[f32],
        category: Option<&str>,
        limit: i64,
    ) -> Result<Vec<MemoryHit>, sqlx::Error> {
        let emb_str = vec_to_pgvector(embedding);
        let half = (limit / 2).max(1);

        let semantic_results: Vec<MemoryHit> = if let Some(cat) = category {
            let cat_pattern = format!("[{}]%", cat);
            sqlx::query_as::<_, MemoryHit>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories
                   WHERE embedding IS NOT NULL
                     AND content ILIKE $1
                     AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
                   ORDER BY embedding <=> $3::vector ASC
                   LIMIT $4"#,
            )
            .bind(&cat_pattern)
            .bind(&self.project_path)
            .bind(&emb_str)
            .bind(half)
            .fetch_all(self.pool.as_ref())
            .await
            .unwrap_or_default()
        } else {
            sqlx::query_as::<_, MemoryHit>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories
                   WHERE embedding IS NOT NULL
                     AND ($1::text IS NULL OR project_path = $1 OR project_path IS NULL)
                   ORDER BY embedding <=> $2::vector ASC
                   LIMIT $3"#,
            )
            .bind(&self.project_path)
            .bind(&emb_str)
            .bind(half)
            .fetch_all(self.pool.as_ref())
            .await
            .unwrap_or_default()
        };

        let text_results = self.text_only_search(query, category, half).await.unwrap_or_default();

        let mut seen = std::collections::HashSet::new();
        let mut merged = Vec::with_capacity(limit as usize);
        for hit in semantic_results.into_iter().chain(text_results.into_iter()) {
            if seen.insert(hit.id) && (merged.len() as i64) < limit {
                merged.push(hit);
            }
        }
        Ok(merged)
    }

    async fn text_only_search(
        &self,
        query: &str,
        category: Option<&str>,
        limit: i64,
    ) -> Result<Vec<MemoryHit>, sqlx::Error> {
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
        if !gate.is_ready() && !gate.wait().await {
            return Vec::new();
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
        if !gate.is_ready() && !gate.wait().await {
            return Vec::new();
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
        if !gate.is_ready() && !gate.wait().await {
            return Ok(Vec::new());
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
