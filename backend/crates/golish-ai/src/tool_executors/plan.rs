use std::sync::Arc;
use serde_json::json;
use golish_core::events::AiEvent;
use super::common::{error_result, ToolResult};

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
