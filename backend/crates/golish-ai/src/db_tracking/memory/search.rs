//! Search helpers: keyword/text, semantic (pgvector), and hybrid search
//! variants, plus document-type filtered lookups.

use super::super::helpers::vec_to_pgvector;
use super::super::types::{MemoryHit, PgvectorScoredRow, ScoredMemoryHit};
use super::super::DbTracker;

impl DbTracker {

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
}
