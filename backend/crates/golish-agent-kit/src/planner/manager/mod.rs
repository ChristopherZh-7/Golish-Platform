//! [`PlanManager`] runtime: thread-safe access to a [`TaskPlan`] with
//! validation, optional PostgreSQL persistence, and prompt-injection
//! formatting.
//!
//! Split into sibling modules:
//! - [`persistence`] — `load_from_db` (DB → in-memory restore)
//! - [`mutations`] — `update_plan` / `apply_patch_ops` (the big mutators)
//!
//! `mod.rs` keeps the struct, constructors, read-only accessors, prompt
//! formatting, and the shared `persist_async` helper.

use std::sync::Arc;

use tokio::sync::RwLock;

use super::{StepStatus, TaskPlan};

mod mutations;
mod persistence;

/// Manager for task plans.
///
/// Provides thread-safe access to the current plan with validation.
/// Optionally persists plans to PostgreSQL for cross-session continuation.
pub struct PlanManager {
    plan: Arc<RwLock<TaskPlan>>,
    db_repo: Option<Arc<dyn crate::db_traits::DbRepoProvider>>,
    session_id: Option<uuid::Uuid>,
    project_path: Option<String>,
    db_plan_id: Arc<RwLock<Option<uuid::Uuid>>>,
    event_emitter: Option<super::SharedPlanEventEmitter>,
}

impl Default for PlanManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PlanManager {
    /// Create a new PlanManager with an empty plan.
    pub fn new() -> Self {
        Self {
            plan: Arc::new(RwLock::new(TaskPlan::default())),
            db_repo: None,
            session_id: None,
            project_path: None,
            db_plan_id: Arc::new(RwLock::new(None)),
            event_emitter: None,
        }
    }

    /// Enable DB persistence for this PlanManager (trait-based).
    pub fn with_db_repo(
        mut self,
        session_id: Option<uuid::Uuid>,
        project_path: Option<String>,
    ) -> Self {
        self.session_id = session_id;
        self.project_path = project_path;
        self
    }

    /// Set the DB repository provider.
    pub fn set_repo(&mut self, repo: Arc<dyn crate::db_traits::DbRepoProvider>) {
        self.db_repo = Some(repo);
    }

    /// Set the event emitter used to broadcast plan changes from background
    /// load operations (e.g. `load_from_db`). The emitter is optional;
    /// without it, the loaded plan is kept in memory only.
    pub fn set_event_emitter(&mut self, emitter: super::SharedPlanEventEmitter) {
        self.event_emitter = Some(emitter);
    }

    /// Get a snapshot of the current plan.
    pub async fn snapshot(&self) -> TaskPlan {
        self.plan.read().await.clone()
    }

    /// Check if the plan is empty.
    pub async fn is_empty(&self) -> bool {
        self.plan.read().await.is_empty()
    }

    /// Format the current plan as a status string for system prompt injection.
    pub async fn format_for_prompt(&self) -> Option<String> {
        let plan = self.plan.read().await;
        if plan.is_empty() {
            return None;
        }

        let mut lines = Vec::new();
        lines.push("## Active Execution Plan".to_string());
        if let Some(ref explanation) = plan.explanation {
            lines.push(format!("**Goal**: {}", explanation));
        }
        lines.push(format!(
            "**Progress**: {}/{} steps completed",
            plan.summary.completed, plan.summary.total
        ));
        lines.push(String::new());

        for (i, step) in plan.steps.iter().enumerate() {
            let icon = match step.status {
                StepStatus::Completed => "✓",
                StepStatus::InProgress => "→",
                StepStatus::Pending => "○",
                StepStatus::Cancelled => "✗",
                StepStatus::Failed => "✗",
            };
            lines.push(format!("{} {}. {}", icon, i + 1, step.step));
        }

        Some(lines.join("\n"))
    }

    /// Spawn a fire-and-forget DB persist task for the given plan
    /// snapshot. Used by both `update_plan` and `apply_patch_ops`
    /// so the patch tool's writes survive an app restart just like
    /// rewrites do.
    ///
    /// No-op when no repo is wired (e.g. tests / headless eval).
    fn persist_async(&self, snapshot: &TaskPlan) {
        let Some(repo) = &self.db_repo else { return };
        let repo = repo.clone();
        let db_plan_id = self.db_plan_id.clone();
        let session_id = self.session_id;
        let project_path = self.project_path.clone();
        let explanation = snapshot.explanation.clone().unwrap_or_default();
        let db_steps: Vec<serde_json::Value> = snapshot
            .steps
            .iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id.as_deref().unwrap_or("unknown"),
                    "title": s.step,
                    "description": "",
                    "status": format!("{}", s.status),
                    "failure_kind": s.failure_kind.as_ref().map(|fk| format!("{}", fk)),
                })
            })
            .collect();
        let steps_json = serde_json::Value::Array(db_steps);
        let current_step = snapshot
            .steps
            .iter()
            .position(|s| s.status == StepStatus::InProgress)
            .unwrap_or(0) as i32;

        let plan_status = if snapshot.summary.completed == snapshot.summary.total {
            crate::db_traits::PlanStatus::Completed
        } else if snapshot.summary.in_progress > 0 {
            crate::db_traits::PlanStatus::InProgress
        } else {
            crate::db_traits::PlanStatus::Planning
        };

        tokio::spawn(async move {
            let existing_id = *db_plan_id.read().await;
            if let Some(id) = existing_id {
                if let Err(e) = crate::db_shim::execution_plans::update_steps(
                    repo.as_ref(),
                    id,
                    &steps_json,
                    current_step,
                    plan_status,
                )
                .await
                {
                    tracing::warn!("Failed to update plan in DB: {}", e);
                }
            } else {
                let title = explanation.chars().take(100).collect::<String>();
                let title = if title.is_empty() {
                    "Untitled Plan".to_string()
                } else {
                    title
                };
                match crate::db_shim::execution_plans::create(
                    repo.as_ref(),
                    crate::db_traits::NewExecutionPlan {
                        session_id,
                        project_path,
                        title,
                        description: explanation,
                        steps: steps_json,
                    },
                )
                .await
                {
                    Ok(created) => {
                        let mut db_id = db_plan_id.write().await;
                        *db_id = Some(created.id);
                        tracing::info!(plan_id = %created.id, "Created plan in DB");
                    }
                    Err(e) => {
                        tracing::warn!("Failed to create plan in DB: {}", e);
                    }
                }
            }
        });
    }

    /// Clear the plan.
    pub async fn clear(&self) {
        let mut plan = self.plan.write().await;
        *plan = TaskPlan::default();
        tracing::info!("Plan cleared");
    }
}
