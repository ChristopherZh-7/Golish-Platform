//! Memory / plan domain methods for `PgTrackingBackend` (inherent `_impl`
//! layer). Bodies moved verbatim from the original `tracking_bridge.rs` trait
//! impl; the trait methods in `mod.rs` delegate here.

use uuid::Uuid;

use super::rows::{PgBriefingPlanRow, PgMemoryHitRow, PgScoredRow};
use super::PgTrackingBackend;
use golish_agent_kit::db_traits::*;

impl PgTrackingBackend {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn store_memory_impl(
        &self,
        session_id: Uuid,
        content: &str,
        mem_type: &str,
        doc_type: &str,
        project_path: Option<&str>,
        metadata: Option<&serde_json::Value>,
        embedding_pgvector: Option<&str>,
    ) {
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
        if let Err(e) = res {
            tracing::warn!("[db-track] store_memory: {e}");
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn store_memory_with_tool_impl(
        &self,
        session_id: Uuid,
        content: &str,
        mem_type: &str,
        tool_name: Option<&str>,
        project_path: Option<&str>,
        metadata: Option<&serde_json::Value>,
        embedding_pgvector: &str,
    ) {
        let res = sqlx::query(
            r#"INSERT INTO memories (session_id, content, mem_type, doc_type, tool_name, embedding, project_path, metadata)
               VALUES ($1, $2, $3::memory_type, 'tool_result', $4, $5::vector, $6, $7)"#,
        )
        .bind(session_id).bind(content).bind(mem_type).bind(tool_name).bind(embedding_pgvector).bind(project_path).bind(metadata)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res {
            tracing::warn!("[db-track] store_memory_with_tool: {e}");
        }
    }

    pub(super) async fn search_memories_text_impl(
        &self,
        query: &str,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit> {
        let pattern = format!("%{}%", query);
        sqlx::query_as::<_, PgMemoryHitRow>(
            r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
               FROM memories WHERE content ILIKE $1
               AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
               ORDER BY created_at DESC LIMIT $3"#,
        )
        .bind(&pattern)
        .bind(project_path)
        .bind(limit)
        .fetch_all(self.pool.as_ref())
        .await
        .unwrap_or_default()
        .into_iter()
        .map(Into::into)
        .collect()
    }

    pub(super) async fn search_memories_semantic_impl(
        &self,
        embedding_pgvector: &str,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<ScoredMemoryHit> {
        let rows: Vec<PgScoredRow> = sqlx::query_as(
            r#"SELECT id, content, mem_type::TEXT as mem_type, tool_name, metadata, created_at,
                      1.0 - (embedding <=> $1::vector) AS score
               FROM memories WHERE embedding IS NOT NULL
               AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
               ORDER BY embedding <=> $1::vector ASC LIMIT $3"#,
        )
        .bind(embedding_pgvector)
        .bind(project_path)
        .bind(limit)
        .fetch_all(self.pool.as_ref())
        .await
        .unwrap_or_default();

        rows.into_iter()
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

    pub(super) async fn search_memories_by_doc_type_impl(
        &self,
        query: &str,
        doc_type: &str,
        sub_filter: Option<&str>,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit> {
        let pattern = format!("%{}%", query);
        if let Some(sf) = sub_filter {
            let sf_pattern = format!("%{}%", sf);
            sqlx::query_as::<_, PgMemoryHitRow>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories WHERE doc_type = $1 AND content ILIKE $2 AND content ILIKE $3
                   AND ($4::text IS NULL OR project_path = $4 OR project_path IS NULL)
                   ORDER BY created_at DESC LIMIT $5"#,
            )
            .bind(doc_type)
            .bind(&pattern)
            .bind(&sf_pattern)
            .bind(project_path)
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect()
        } else {
            sqlx::query_as::<_, PgMemoryHitRow>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories WHERE doc_type = $1 AND content ILIKE $2
                   AND ($3::text IS NULL OR project_path = $3 OR project_path IS NULL)
                   ORDER BY created_at DESC LIMIT $4"#,
            )
            .bind(doc_type)
            .bind(&pattern)
            .bind(project_path)
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect()
        }
    }

    pub(super) async fn search_memories_text_with_category_impl(
        &self,
        query: &str,
        category: Option<&str>,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit> {
        let pattern = format!("%{}%", query);
        if let Some(cat) = category {
            let cat_pattern = format!("[{}]%", cat);
            sqlx::query_as::<_, PgMemoryHitRow>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories WHERE content ILIKE $1 AND content ILIKE $2
                   AND ($3::text IS NULL OR project_path = $3 OR project_path IS NULL)
                   ORDER BY created_at DESC LIMIT $4"#,
            )
            .bind(&pattern)
            .bind(&cat_pattern)
            .bind(project_path)
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect()
        } else {
            self.search_memories_text_impl(query, project_path, limit)
                .await
        }
    }

    pub(super) async fn search_memories_semantic_with_category_impl(
        &self,
        category: Option<&str>,
        project_path: Option<&str>,
        embedding_pgvector: &str,
        limit: i64,
    ) -> Vec<MemoryHit> {
        if let Some(cat) = category {
            let cat_pattern = format!("[{}]%", cat);
            sqlx::query_as::<_, PgMemoryHitRow>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories WHERE embedding IS NOT NULL AND content ILIKE $1
                   AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
                   ORDER BY embedding <=> $3::vector ASC LIMIT $4"#,
            )
            .bind(&cat_pattern)
            .bind(project_path)
            .bind(embedding_pgvector)
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect()
        } else {
            sqlx::query_as::<_, PgMemoryHitRow>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories WHERE embedding IS NOT NULL
                   AND ($1::text IS NULL OR project_path = $1 OR project_path IS NULL)
                   ORDER BY embedding <=> $2::vector ASC LIMIT $3"#,
            )
            .bind(project_path)
            .bind(embedding_pgvector)
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect()
        }
    }

    pub(super) async fn fetch_memories_by_keyword_impl(
        &self,
        keyword: &str,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit> {
        let pattern = format!("%{}%", keyword);
        sqlx::query_as::<_, PgMemoryHitRow>(
            r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
               FROM memories WHERE content ILIKE $1
               AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
               ORDER BY created_at DESC LIMIT $3"#,
        )
        .bind(&pattern)
        .bind(project_path)
        .bind(limit)
        .fetch_all(self.pool.as_ref())
        .await
        .unwrap_or_default()
        .into_iter()
        .map(Into::into)
        .collect()
    }

    pub(super) async fn fetch_active_plans_impl(&self, project_path: &str) -> Vec<BriefingPlan> {
        sqlx::query_as::<_, PgBriefingPlanRow>(
            r#"SELECT title, description, steps, current_step, status::TEXT as status
               FROM execution_plans
               WHERE project_path = $1 AND status IN ('planning', 'in_progress', 'paused')
               ORDER BY updated_at DESC LIMIT 3"#,
        )
        .bind(project_path)
        .fetch_all(self.pool.as_ref())
        .await
        .unwrap_or_default()
        .into_iter()
        .map(Into::into)
        .collect()
    }

    pub(super) async fn list_recent_memories_impl(
        &self,
        category: Option<&str>,
        project_path: Option<&str>,
        limit: i64,
    ) -> Vec<MemoryHit> {
        if let Some(cat) = category {
            let cat_pattern = format!("[{}]%", cat);
            sqlx::query_as::<_, PgMemoryHitRow>(
                r#"SELECT id, content, mem_type::TEXT as mem_type, metadata, created_at
                   FROM memories WHERE content ILIKE $1
                   AND ($2::text IS NULL OR project_path = $2 OR project_path IS NULL)
                   ORDER BY created_at DESC LIMIT $3"#,
            )
            .bind(&cat_pattern)
            .bind(project_path)
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect()
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

    pub(super) async fn ensure_session_impl(&self, session_id: Uuid) {
        let _ = sqlx::query("INSERT INTO sessions (id) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(session_id)
            .execute(self.pool.as_ref())
            .await;
    }

    pub(super) async fn load_prompt_template_overrides_impl(&self) -> Vec<(String, String)> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT template_name, content FROM prompt_templates WHERE is_active = true",
        )
        .fetch_all(self.pool.as_ref())
        .await
        .unwrap_or_default()
    }
}
