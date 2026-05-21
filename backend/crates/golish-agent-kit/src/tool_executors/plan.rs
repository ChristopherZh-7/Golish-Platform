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

#[cfg(test)]
mod patch_tool_tests {
    use super::*;
    use crate::planner::{PlanManager, PlanStepInput, StepStatus, UpdatePlanArgs};
    use tokio::sync::mpsc;

    async fn seeded_manager(steps: &[&str]) -> Arc<PlanManager> {
        let manager = Arc::new(PlanManager::new());
        let args = UpdatePlanArgs {
            explanation: Some("seed".into()),
            plan: steps
                .iter()
                .map(|t| PlanStepInput {
                    step: (*t).into(),
                    status: StepStatus::Pending,
                })
                .collect(),
        };
        manager.update_plan(args).await.unwrap();
        manager
    }

    fn channel() -> (
        mpsc::UnboundedSender<AiEvent>,
        mpsc::UnboundedReceiver<AiEvent>,
    ) {
        mpsc::unbounded_channel()
    }

    #[tokio::test]
    async fn rejects_empty_ops_with_error() {
        let manager = seeded_manager(&["A"]).await;
        let (tx, mut rx) = channel();
        let args = json!({ "ops": [] });
        let (value, success) = execute_plan_patch_tool(&manager, &tx, &args).await;
        assert!(!success);
        let err = value.get("error").and_then(|v| v.as_str()).unwrap_or("");
        assert!(err.contains("at least one op"));
        assert!(rx.try_recv().is_err(), "no event on empty ops");
    }

    #[tokio::test]
    async fn applies_add_op_and_emits_plan_updated_event() {
        let manager = seeded_manager(&["A"]).await;
        let (tx, mut rx) = channel();
        let args = json!({
            "ops": [
                { "op": "add", "after_id": null, "title": "Z", "status": "pending" }
            ]
        });
        let (value, success) = execute_plan_patch_tool(&manager, &tx, &args).await;
        assert!(success, "{value}");
        assert_eq!(value.get("ops_applied").and_then(|v| v.as_u64()), Some(1));
        let snapshot = manager.snapshot().await;
        assert_eq!(snapshot.steps.len(), 2);
        assert_eq!(snapshot.steps[0].step, "Z");

        let event = rx.try_recv().expect("must emit PlanUpdated");
        match event {
            AiEvent::PlanUpdated {
                version,
                summary,
                steps,
                ..
            } => {
                assert_eq!(version, snapshot.version);
                assert_eq!(summary.total, 2);
                assert_eq!(steps.len(), 2);
            }
            other => panic!("expected PlanUpdated, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn rejects_invalid_op_schema_with_error() {
        let manager = seeded_manager(&["A"]).await;
        let (tx, mut rx) = channel();
        // Missing required `title` on Add → serde rejects
        let args = json!({
            "ops": [
                { "op": "add" }
            ]
        });
        let (value, success) = execute_plan_patch_tool(&manager, &tx, &args).await;
        assert!(!success);
        assert!(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("Invalid update_plan_patch arguments"));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn composite_ops_applied_in_order() {
        let manager = seeded_manager(&["A", "B", "C"]).await;
        let snapshot = manager.snapshot().await;
        let b_id = snapshot.steps[1].id.clone().unwrap();
        let (tx, _rx) = channel();
        let args = json!({
            "ops": [
                { "op": "remove", "id": b_id },
                { "op": "modify", "id": snapshot.steps[0].id.clone().unwrap(),
                  "status": "completed" }
            ]
        });
        let (value, success) = execute_plan_patch_tool(&manager, &tx, &args).await;
        assert!(success, "{value}");
        let snapshot = manager.snapshot().await;
        assert_eq!(snapshot.steps.len(), 2);
        assert_eq!(snapshot.steps[0].step, "A");
        assert_eq!(snapshot.steps[0].status, StepStatus::Completed);
        assert_eq!(snapshot.steps[1].step, "C");
    }

    #[tokio::test]
    async fn surface_planner_validation_error() {
        let manager = seeded_manager(&["A", "B"]).await;
        let snapshot = manager.snapshot().await;
        let a = snapshot.steps[0].id.clone().unwrap();
        let b = snapshot.steps[1].id.clone().unwrap();
        let (tx, _rx) = channel();
        // Force MultipleInProgress
        let args = json!({
            "ops": [
                { "op": "modify", "id": a, "status": "in_progress" },
                { "op": "modify", "id": b, "status": "in_progress" }
            ]
        });
        let (value, success) = execute_plan_patch_tool(&manager, &tx, &args).await;
        assert!(!success);
        assert!(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("Failed to apply plan patch"));
    }
}
