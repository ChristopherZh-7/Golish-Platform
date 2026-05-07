//! Bulk fetch helpers used by briefings and the recent-memories UI.

use super::super::types::{BriefingPlan, MemoryHit};
use super::super::DbTracker;

impl DbTracker {
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
            let rows = self
                .backend
                .fetch_memories_by_keyword(keyword, self.project_path.as_deref(), per_keyword_limit)
                .await;
            for row in rows {
                if !results.iter().any(|r| r.id == row.id) {
                    results.push(row);
                }
            }
        }

        results.truncate(limit as usize);
        results
    }

    pub async fn fetch_active_plans(&self) -> Vec<BriefingPlan> {
        let mut gate = self.ready_gate.clone();
        if !gate.is_ready() && !gate.wait().await {
            return Vec::new();
        }

        let project_path = match &self.project_path {
            Some(p) => p.clone(),
            None => return Vec::new(),
        };

        self.backend.fetch_active_plans(&project_path).await
    }

    pub async fn list_recent_memories(&self, category: Option<&str>, limit: i64) -> Vec<MemoryHit> {
        let mut gate = self.ready_gate.clone();
        if !gate.is_ready() && !gate.wait().await {
            return Vec::new();
        }

        self.backend
            .list_recent_memories(category, self.project_path.as_deref(), limit)
            .await
    }
}
