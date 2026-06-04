//! DB → in-memory restore for [`PlanManager`].

use super::super::{PlanStep, PlanSummary, StepStatus, TaskPlan};
use super::PlanManager;

impl PlanManager {
    /// Load the active plans from DB for the current project, one bucket per
    /// stage. Returns true if at least one plan was loaded.
    ///
    /// `list_active` is project-scoped and ordered by `updated_at DESC`, so we
    /// keep the **most recent** row per stage key (`NULL` stage_id → `""`
    /// chat-mode bucket). `current_stage` is set to the most recently updated
    /// stage so the no-arg readers return a sensible plan right after restore.
    pub async fn load_from_db(&self) -> bool {
        let Some(project_path) = &self.project_path else {
            return false;
        };

        let Some(repo) = &self.db_repo else {
            return false;
        };

        let rows =
            match crate::db_shim::execution_plans::list_active(repo.as_ref(), project_path).await {
                Ok(rows) if !rows.is_empty() => rows,
                _ => return false,
            };

        // rows[0] is the most recently updated active plan → becomes current.
        let current_key = rows[0].stage_id.clone().unwrap_or_default();

        // Build each stage bucket, keeping the newest row per key (DESC order
        // means the first row seen for a key wins; older duplicates skipped).
        let mut restored: Vec<(String, TaskPlan)> = Vec::with_capacity(rows.len());
        {
            let mut plans = self.plans.write().await;
            let mut ids = self.db_plan_ids.write().await;
            for row in &rows {
                let key = row.stage_id.clone().unwrap_or_default();
                if plans.contains_key(&key) {
                    // Older duplicate for this stage; the newer one already won.
                    continue;
                }

                let steps: Vec<crate::db_traits::PlanStep> =
                    serde_json::from_value(row.steps.clone()).unwrap_or_default();
                let plan_steps: Vec<PlanStep> = steps
                    .iter()
                    .map(|s| PlanStep {
                        id: Some(s.id.clone()),
                        step: s.title.clone(),
                        status: match s.status.as_str() {
                            "completed" => StepStatus::Completed,
                            "in_progress" => StepStatus::InProgress,
                            _ => StepStatus::Pending,
                        },
                        failure_kind: None,
                    })
                    .collect();

                let summary = PlanSummary::from_steps(&plan_steps);

                let tp = TaskPlan {
                    explanation: Some(row.description.clone()),
                    steps: plan_steps,
                    summary,
                    version: 1,
                    ..Default::default()
                };

                ids.insert(key.clone(), row.id);
                restored.push((key.clone(), tp.clone()));
                plans.insert(key, tp);
            }
        }

        tracing::info!(
            stages = restored.len(),
            current = %current_key,
            "Loaded active plan(s) from DB"
        );

        // Broadcast each restored stage so the frontend rehydrates without
        // waiting for the next LLM-driven `update_plan` (emitter is optional).
        if let Some(ref emitter) = self.event_emitter {
            for (key, tp) in &restored {
                let stage_id = if key.is_empty() {
                    None
                } else {
                    Some(key.clone())
                };
                emitter.emit_plan_updated(
                    tp.version,
                    tp.summary.clone(),
                    tp.steps.clone(),
                    tp.explanation.clone(),
                    stage_id,
                );
            }
            tracing::debug!("[PlanManager] Emitted PlanUpdated after load_from_db restore");
        }

        *self.current_stage.write().await = current_key;

        true
    }
}
