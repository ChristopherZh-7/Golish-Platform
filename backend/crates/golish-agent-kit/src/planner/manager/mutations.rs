//! Plan mutation paths for [`PlanManager`]: full rewrite (`update_plan`)
//! and incremental patch ops (`apply_patch_ops`).

use chrono::Utc;

use super::super::{
    PlanError, PlanStep, PlanSummary, StepStatus, TaskPlan, UpdatePlanArgs, MAX_PLAN_STEPS,
    MIN_PLAN_STEPS,
};
use super::PlanManager;

impl PlanManager {
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
                    failure_kind: None,
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

        self.persist_async(&result);

        Ok(result)
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
        ops: Vec<super::super::PlanPatchOp>,
    ) -> Result<TaskPlan, super::super::PlanError> {
        let mut plan = self.plan.write().await;
        let mut steps = plan.steps.clone();

        for op in ops {
            match op {
                super::super::PlanPatchOp::Add {
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
                super::super::PlanPatchOp::Remove { id } => {
                    steps.retain(|s| s.id.as_deref() != Some(id.as_str()));
                }
                super::super::PlanPatchOp::Modify {
                    id,
                    title,
                    status,
                    failure_kind,
                } => {
                    if let Some(step) = steps
                        .iter_mut()
                        .find(|s| s.id.as_deref() == Some(id.as_str()))
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
                super::super::PlanPatchOp::Reorder { id, after_id } => {
                    if let Some(idx) = steps
                        .iter()
                        .position(|s| s.id.as_deref() == Some(id.as_str()))
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
            return Err(super::super::PlanError::InvalidStepCount(steps.len()));
        }
        let in_progress = steps
            .iter()
            .filter(|s| s.status == StepStatus::InProgress)
            .count();
        if in_progress > 1 {
            return Err(super::super::PlanError::MultipleInProgress(in_progress));
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

        let result = plan.clone();
        drop(plan);

        self.persist_async(&result);

        Ok(result)
    }
}
