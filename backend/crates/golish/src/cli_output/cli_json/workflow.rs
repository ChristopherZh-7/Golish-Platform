//! Workflow lifecycle events plus the plan-management `PlanUpdated` event.

use golish_core::plan::{PlanStep, PlanSummary};

pub(super) fn workflow_started(
    workflow_id: &str,
    workflow_name: &str,
    session_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "workflow_id": workflow_id,
        "workflow_name": workflow_name,
        "session_id": session_id
    })
}

pub(super) fn workflow_step_started(
    workflow_id: &str,
    step_name: &str,
    step_index: usize,
    total_steps: usize,
) -> serde_json::Value {
    serde_json::json!({
        "workflow_id": workflow_id,
        "step_name": step_name,
        "step_index": step_index,
        "total_steps": total_steps
    })
}

pub(super) fn workflow_step_completed(
    workflow_id: &str,
    step_name: &str,
    output: &Option<String>,
    duration_ms: u64,
) -> serde_json::Value {
    serde_json::json!({
        "workflow_id": workflow_id,
        "step_name": step_name,
        "output": output,
        "duration_ms": duration_ms
    })
}

pub(super) fn workflow_completed(
    workflow_id: &str,
    final_output: &str,
    total_duration_ms: u64,
) -> serde_json::Value {
    serde_json::json!({
        "workflow_id": workflow_id,
        "final_output": final_output,
        "total_duration_ms": total_duration_ms
    })
}

pub(super) fn workflow_error(
    workflow_id: &str,
    step_name: &Option<String>,
    error: &str,
) -> serde_json::Value {
    serde_json::json!({
        "workflow_id": workflow_id,
        "step_name": step_name,
        "error": error
    })
}

pub(super) fn plan_updated(
    version: u32,
    summary: &PlanSummary,
    steps: &[PlanStep],
    explanation: &Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "version": version,
        "summary": summary,
        "steps": steps,
        "explanation": explanation
    })
}
