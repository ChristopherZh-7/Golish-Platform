//! DB → in-memory restore for [`PlanManager`].

use super::super::{PlanStep, PlanSummary, StepStatus};
use super::PlanManager;

impl PlanManager {
    /// Load the most recent active plan from DB for the current project.
    /// Returns true if a plan was loaded.
    pub async fn load_from_db(&self) -> bool {
        let Some(project_path) = &self.project_path else {
            return false;
        };

        let Some(repo) = &self.db_repo else {
            return false;
        };
        match crate::db_shim::execution_plans::list_active(repo.as_ref(), project_path).await {
            Ok(plans) if !plans.is_empty() => {
                let db_plan = &plans[0];
                let steps: Vec<crate::db_traits::PlanStep> =
                    serde_json::from_value(db_plan.steps.clone()).unwrap_or_default();

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

                let snapshot_version: u32;
                let snapshot_summary: PlanSummary;
                let snapshot_steps: Vec<PlanStep>;
                let snapshot_explanation: Option<String>;
                {
                    let mut plan = self.plan.write().await;
                    plan.explanation = Some(db_plan.description.clone());
                    plan.steps = plan_steps;
                    plan.summary = summary;
                    plan.version = 1;
                    snapshot_version = plan.version;
                    snapshot_summary = plan.summary.clone();
                    snapshot_steps = plan.steps.clone();
                    snapshot_explanation = plan.explanation.clone();
                }

                {
                    let mut db_id = self.db_plan_id.write().await;
                    *db_id = Some(db_plan.id);
                }

                tracing::info!(
                    plan_id = %db_plan.id,
                    title = %db_plan.title,
                    steps = db_plan.steps.as_array().map(|a| a.len()).unwrap_or(0),
                    "Loaded active plan from DB"
                );

                if let Some(ref emitter) = self.event_emitter {
                    emitter.emit_plan_updated(
                        snapshot_version,
                        snapshot_summary,
                        snapshot_steps,
                        snapshot_explanation,
                    );
                    tracing::debug!("[PlanManager] Emitted PlanUpdated after load_from_db restore");
                }

                true
            }
            _ => false,
        }
    }
}
