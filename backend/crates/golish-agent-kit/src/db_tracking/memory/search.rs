//! Search helpers: keyword/text, semantic (pgvector), and hybrid search
//! variants, plus document-type filtered lookups.

use super::super::helpers::vec_to_pgvector;
use super::super::types::{MemoryHit, ScoredMemoryHit};
use super::super::DbTracker;

impl DbTracker {
    pub async fn search_memories_by_doc_type(
        &self,
        query: &str,
        doc_type: &str,
        sub_filter: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit> {
        let mut gate = self.ready_gate.clone();
        if !gate.is_ready() && !gate.wait().await {
            return Vec::new();
        }

        self.backend
            .search_memories_by_doc_type(
                query,
                doc_type,
                sub_filter,
                self.project_path.as_deref(),
                limit,
            )
            .await
    }

    pub async fn search_memories_semantic(
        &mut self,
        query_embedding: &[f32],
        limit: usize,
        threshold: f32,
    ) -> Vec<ScoredMemoryHit> {
        if !self.ready_gate.is_ready() && !self.ready_gate.wait().await {
            return Vec::new();
        }

        let emb_str = vec_to_pgvector(query_embedding);
        let results = self
            .backend
            .search_memories_semantic(&emb_str, self.project_path.as_deref(), limit as i64)
            .await;

        results
            .into_iter()
            .filter(|r| r.score >= threshold)
            .collect()
    }

    pub async fn search_memories_text(&mut self, query: &str, limit: i64) -> Vec<MemoryHit> {
        if !self.ready_gate.is_ready() && !self.ready_gate.wait().await {
            return Vec::new();
        }

        self.backend
            .search_memories_text(query, self.project_path.as_deref(), limit)
            .await
    }

    pub async fn search_memories_by_text(
        &self,
        query: &str,
        category: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit> {
        self.record_vecstore_op(
            "search",
            query,
            0,
            &format!("category={}", category.unwrap_or("all")),
        );

        let mut gate = self.ready_gate.clone();
        if !gate.is_ready() && !gate.wait().await {
            return Vec::new();
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

        self.backend
            .search_memories_text_with_category(
                query,
                category,
                self.project_path.as_deref(),
                limit,
            )
            .await
    }

    async fn hybrid_search(
        &self,
        query: &str,
        embedding: &[f32],
        category: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit> {
        let emb_str = vec_to_pgvector(embedding);
        let half = (limit / 2).max(1);

        let semantic_results = self
            .backend
            .search_memories_semantic_with_category(
                category,
                self.project_path.as_deref(),
                &emb_str,
                half,
            )
            .await;

        let text_results = self
            .backend
            .search_memories_text_with_category(query, category, self.project_path.as_deref(), half)
            .await;

        let mut seen = std::collections::HashSet::new();
        let mut merged = Vec::with_capacity(limit as usize);
        for hit in semantic_results.into_iter().chain(text_results) {
            if seen.insert(hit.id) && (merged.len() as i64) < limit {
                merged.push(hit);
            }
        }
        merged
    }
}
