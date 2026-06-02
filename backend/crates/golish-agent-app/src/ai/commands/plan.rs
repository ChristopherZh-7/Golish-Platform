//! Plan management commands for retrieving the current task plan.
//!
//! These commands allow the frontend to query the current plan state.

use crate::error::GolishError;
use tauri::State;

use crate::state::AgentState;
use golish_agent_kit::planner::TaskPlan;

/// Get the current task plan for a session.
///
/// # Arguments
/// * `session_id` - The session ID to get the plan for
///
/// # Returns
/// The current `TaskPlan` (version, summary, steps). An uninitialized session
/// returns an empty plan (version 0) instead of an error — see below.
#[tauri::command]
pub async fn get_plan(
    session_id: String,
    state: State<'_, AgentState>,
) -> Result<TaskPlan, GolishError> {
    let bridges = state.ai_state.bridges.read().await;
    // P2 · an uninitialized session simply has no plan yet. The frontend restore
    // fallback (useTaskPlanState) calls this early — before `init_ai_session`
    // registers the bridge — so return an empty plan (version 0, which the
    // frontend treats as "no plan") instead of erroring. Erroring here only
    // produced a recurring `console.warn` on every early restore.
    match bridges.get(&session_id) {
        Some(bridge) => Ok(bridge.plan_manager().snapshot().await),
        None => Ok(TaskPlan::default()),
    }
}
