//! Bulk fetch helpers used by briefings and the recent-memories UI.

use super::super::types::{BriefingPlan, MemoryHit};
use super::super::DbTracker;

impl DbTracker {

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
