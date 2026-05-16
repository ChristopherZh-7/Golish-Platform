use super::common::{error_result, ToolResult};
use golish_core::events::AiEvent;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// Execute the update_plan tool.
///
/// Updates the task plan with new steps and their statuses.
/// Emits a PlanUpdated event when the plan is successfully updated.
pub async fn execute_plan_tool(
    plan_manager: &Arc<crate::planner::PlanManager>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<AiEvent>,
    args: &serde_json::Value,
) -> ToolResult {
    let update_args: crate::planner::UpdatePlanArgs = match serde_json::from_value(args.clone()) {
        Ok(a) => a,
        Err(e) => return error_result(format!("Invalid update_plan arguments: {}", e)),
    };

    match plan_manager.update_plan(update_args).await {
        Ok(plan) => {
            let _ = event_tx.send(AiEvent::PlanUpdated {
                version: plan.version,
                summary: plan.summary.clone(),
                steps: plan.steps.clone(),
                explanation: None,
            });

            (
                json!({
                    "success": true,
                    "version": plan.version,
                    "summary": {
                        "total": plan.summary.total,
                        "completed": plan.summary.completed,
                        "in_progress": plan.summary.in_progress,
                        "pending": plan.summary.pending
                    }
                }),
                true,
            )
        }
        Err(e) => error_result(format!("Failed to update plan: {}", e)),
    }
}

/// Arguments for the `update_plan_patch` tool.
///
/// Mirrors PentAGI's `subtask_patch_tool`: instead of rewriting the
/// plan from scratch on each refine cycle, the LLM emits a list of
/// `add` / `remove` / `modify` / `reorder` operations that the
/// PlanManager applies on top of the current plan.
#[derive(Debug, Deserialize)]
struct UpdatePlanPatchArgs {
    /// Sequence of patch operations to apply, in order.
    pub ops: Vec<crate::planner::PlanPatchOp>,
    /// Optional explanation summarising the refinement intent.
    /// Reserved for future use; the current PlanManager keeps the
    /// existing plan-level explanation.
    #[serde(default)]
    #[allow(dead_code)]
    pub explanation: Option<String>,
}

/// Execute the `update_plan_patch` tool (P0-2 stage 3).
///
/// Lets the refiner LLM modify the plan incrementally rather than
/// emitting a full rewrite. Emits the same `PlanUpdated` event as
/// `update_plan` so frontend handlers stay unchanged.
pub async fn execute_plan_patch_tool(
    plan_manager: &Arc<crate::planner::PlanManager>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<AiEvent>,
    args: &serde_json::Value,
) -> ToolResult {
    let parsed: UpdatePlanPatchArgs = match serde_json::from_value(args.clone()) {
        Ok(a) => a,
        Err(e) => {
            return error_result(format!("Invalid update_plan_patch arguments: {}", e));
        }
    };

    if parsed.ops.is_empty() {
        return error_result("update_plan_patch requires at least one op".to_string());
    }

    let op_count = parsed.ops.len();

    match plan_manager.apply_patch_ops(parsed.ops).await {
        Ok(plan) => {
            let _ = event_tx.send(AiEvent::PlanUpdated {
                version: plan.version,
                summary: plan.summary.clone(),
                steps: plan.steps.clone(),
                explanation: plan.explanation.clone(),
            });

            (
                json!({
                    "success": true,
                    "version": plan.version,
                    "ops_applied": op_count,
                    "summary": {
                        "total": plan.summary.total,
                        "completed": plan.summary.completed,
                        "in_progress": plan.summary.in_progress,
                        "pending": plan.summary.pending
                    }
                }),
                true,
            )
        }
        Err(e) => error_result(format!("Failed to apply plan patch: {}", e)),
    }
}
