//! [`PlanManager`] runtime: thread-safe access to a [`TaskPlan`] with
//! validation, optional PostgreSQL persistence, and prompt-injection
//! formatting.

use std::sync::Arc;

use chrono::Utc;
use tokio::sync::RwLock;

use super::{
    PlanError, PlanStep, PlanSummary, StepStatus, TaskPlan, UpdatePlanArgs, MAX_PLAN_STEPS,
    MIN_PLAN_STEPS,
};

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
                    tracing::debug!(
                        "[PlanManager] Emitted PlanUpdated after load_from_db restore"
                    );
                }

                true
            }
            _ => false,
        }
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

    /// Update the plan with new steps.
    ///
    /// Validates the input and updates the plan atomically.
    /// If DB persistence is enabled, also saves to PostgreSQL.
    pub async fn update_plan(&self, args: UpdatePlanArgs) -> Result<TaskPlan, PlanError> {
        // Validate step count
        let step_count = args.plan.len();
        if !(MIN_PLAN_STEPS..=MAX_PLAN_STEPS).contains(&step_count) {
            return Err(PlanError::InvalidStepCount(step_count));
        }

        // Validate steps and count in_progress
        let mut in_progress_count = 0;
        for (i, step) in args.plan.iter().enumerate() {
            // Check for empty descriptions
            let trimmed = step.step.trim();
            if trimmed.is_empty() {
                return Err(PlanError::EmptyStepDescription(i + 1));
            }

            // Count in_progress steps
            if step.status == StepStatus::InProgress {
                in_progress_count += 1;
            }
        }

        // Ensure at most one in_progress
        if in_progress_count > 1 {
            return Err(PlanError::MultipleInProgress(in_progress_count));
        }

        // Build a lookup of existing step descriptions → IDs for stable matching
        let existing_plan = self.plan.read().await;
        let existing_id_map: std::collections::HashMap<String, String> = existing_plan
            .steps
            .iter()
            .filter_map(|s| s.id.as_ref().map(|id| (s.step.clone(), id.clone())))
            .collect();
        // Collect completed/failed steps that must be preserved (PentAGI-style:
        // refine only replaces pending work, never removes finished work).
        let preserved_steps: Vec<PlanStep> = existing_plan
            .steps
            .iter()
            .filter(|s| matches!(s.status, StepStatus::Completed | StepStatus::Failed))
            .cloned()
            .collect();
        drop(existing_plan);

        // Convert incoming steps, reusing IDs for matching descriptions.
        // Truncate step text to prevent bloated plan entries (defense-in-depth).
        const MAX_STEP_LEN: usize = 200;
        let incoming_steps: Vec<PlanStep> = args
            .plan
            .into_iter()
            .map(|input| {
                let mut trimmed = input.step.trim().to_string();
                if trimmed.len() > MAX_STEP_LEN {
                    // Truncate at a char boundary
                    let mut end = MAX_STEP_LEN;
                    while !trimmed.is_char_boundary(end) && end > 0 {
                        end -= 1;
                    }
                    trimmed.truncate(end);
                    trimmed.push('…');
                    tracing::warn!(
                        original_len = input.step.len(),
                        "[PlanManager] Step text too long, truncated to {}",
                        MAX_STEP_LEN,
                    );
                }
                let id = existing_id_map
                    .get(&trimmed)
                    .cloned()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                PlanStep {
                    id: Some(id),
                    step: trimmed,
                    status: input.status,
                }
            })
            .collect();

        // Track which preserved steps the AI already included
        let incoming_ids: std::collections::HashSet<String> =
            incoming_steps.iter().filter_map(|s| s.id.clone()).collect();

        // Re-inject completed/failed steps that the AI omitted (plan refine
        // dropped them, but we must keep finished work visible).
        let mut steps = Vec::with_capacity(preserved_steps.len() + incoming_steps.len());
        for ps in &preserved_steps {
            if let Some(ref id) = ps.id {
                if !incoming_ids.contains(id) {
                    steps.push(ps.clone());
                }
            }
        }
        steps.extend(incoming_steps);

        // Calculate summary
        let summary = PlanSummary::from_steps(&steps);

        // Update the plan
        let mut plan = self.plan.write().await;
        plan.explanation = args.explanation.map(|s| s.trim().to_string());
        plan.steps = steps;
        plan.summary = summary;
        plan.version += 1;
        plan.updated_at = Utc::now();

        tracing::info!(
            version = plan.version,
            total = plan.summary.total,
            completed = plan.summary.completed,
            "Plan updated"
        );

        let result = plan.clone();
        drop(plan);

        // Persist to DB in the background (fire-and-forget)
        if let Some(repo) = &self.db_repo {
            let repo = repo.clone();
            let db_plan_id = self.db_plan_id.clone();
            let session_id = self.session_id;
            let project_path = self.project_path.clone();
            let explanation = result.explanation.clone().unwrap_or_default();
            let db_steps: Vec<serde_json::Value> = result
                .steps
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id.as_deref().unwrap_or("unknown"),
                        "title": s.step,
                        "description": "",
                        "status": format!("{}", s.status),
                    })
                })
                .collect();
            let steps_json = serde_json::Value::Array(db_steps);
            let current_step = result
                .steps
                .iter()
                .position(|s| s.status == StepStatus::InProgress)
                .unwrap_or(0) as i32;

            let plan_status = if result.summary.completed == result.summary.total {
                crate::db_traits::PlanStatus::Completed
            } else if result.summary.in_progress > 0 {
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

        Ok(result)
    }

    /// Clear the plan.
    pub async fn clear(&self) {
        let mut plan = self.plan.write().await;
        *plan = TaskPlan::default();
        tracing::info!("Plan cleared");
    }

    /// Apply a sequence of patch operations to the current plan
    /// (P0-2 stage 2). Each op lands in order; positions are computed
    /// against the **mutating** in-memory list so dependent ops
    /// (`Add` followed by `Modify` on the new id) work the way an LLM
    /// would expect.
    ///
    /// Currently does **not** emit `PlanUpdated` nor persist to DB —
    /// those side-effects come once the wrapper tool lands; until then
    /// callers are responsible for whatever publishing they need
    /// (typically: none, since this is exercised by tests only).
    pub async fn apply_patch_ops(
        &self,
        ops: Vec<super::PlanPatchOp>,
    ) -> Result<TaskPlan, super::PlanError> {
        let mut plan = self.plan.write().await;
        let mut steps = plan.steps.clone();

        for op in ops {
            match op {
                super::PlanPatchOp::Add {
                    after_id,
                    title,
                    status,
                } => {
                    let trimmed = title.trim();
                    if trimmed.is_empty() {
                        // Treat empty inserts as a no-op rather than fail,
                        // mirroring update_plan's "no whitespace-only steps"
                        // intent without aborting the whole batch.
                        continue;
                    }
                    let new_step = PlanStep {
                        id: Some(uuid::Uuid::new_v4().to_string()),
                        step: trimmed.to_string(),
                        status: status.unwrap_or(StepStatus::Pending),
                        failure_kind: None,
                    };
                    let pos = match after_id.as_deref() {
                        Some(aid) => steps
                            .iter()
                            .position(|s| s.id.as_deref() == Some(aid))
                            .map(|i| i + 1)
                            .unwrap_or_else(|| steps.len()),
                        None => 0,
                    };
                    steps.insert(pos.min(steps.len()), new_step);
                }
                super::PlanPatchOp::Remove { id } => {
                    steps.retain(|s| s.id.as_deref() != Some(id.as_str()));
                }
                super::PlanPatchOp::Modify {
                    id,
                    title,
                    status,
                    failure_kind,
                } => {
                    if let Some(step) =
                        steps.iter_mut().find(|s| s.id.as_deref() == Some(id.as_str()))
                    {
                        if let Some(t) = title {
                            let trimmed = t.trim();
                            if !trimmed.is_empty() {
                                step.step = trimmed.to_string();
                            }
                        }
                        if let Some(s) = status {
                            step.status = s;
                        }
                        if failure_kind.is_some() {
                            step.failure_kind = failure_kind;
                        }
                    }
                }
                super::PlanPatchOp::Reorder { id, after_id } => {
                    if let Some(idx) =
                        steps.iter().position(|s| s.id.as_deref() == Some(id.as_str()))
                    {
                        let step = steps.remove(idx);
                        let new_pos = match after_id.as_deref() {
                            Some(aid) => steps
                                .iter()
                                .position(|s| s.id.as_deref() == Some(aid))
                                .map(|i| i + 1)
                                .unwrap_or_else(|| steps.len()),
                            None => 0,
                        };
                        steps.insert(new_pos.min(steps.len()), step);
                    }
                }
            }
        }

        // Validation: same caps as update_plan.
        if steps.len() > MAX_PLAN_STEPS {
            return Err(super::PlanError::InvalidStepCount(steps.len()));
        }
        let in_progress = steps
            .iter()
            .filter(|s| s.status == StepStatus::InProgress)
            .count();
        if in_progress > 1 {
            return Err(super::PlanError::MultipleInProgress(in_progress));
        }

        plan.steps = steps;
        plan.summary = PlanSummary::from_steps(&plan.steps);
        plan.version += 1;
        plan.updated_at = Utc::now();

        tracing::info!(
            version = plan.version,
            total = plan.summary.total,
            "Plan updated via patch ops"
        );

        Ok(plan.clone())
    }
}
