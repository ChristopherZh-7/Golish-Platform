//! Agent mode commands for controlling tool approval behavior.
//!
//! These commands allow the frontend to get and set the agent mode for
//! a specific session, controlling how tool approvals are handled.

use crate::error::GolishError;
use golish_settings::ProjectSettingsManager;
use std::path::PathBuf;
use tauri::State;

use crate::ai::agent_mode::AgentMode;
use crate::state::AgentState;

use super::ai_session_not_initialized_error;

/// Set the agent mode for a session.
///
/// # Arguments
/// * `session_id` - The session ID to set the mode for
/// * `mode` - The agent mode ("default", "auto-approve", or "planning")
/// * `workspace` - Optional workspace path to persist the mode to project settings
#[tauri::command]
pub async fn set_agent_mode(
    session_id: String,
    mode: AgentMode,
    workspace: Option<PathBuf>,
    state: State<'_, AgentState>,
) -> Result<(), GolishError> {
    let bridges = state.ai_state.bridges.read().await;
    let bridge = bridges
        .get(&session_id)
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;

    bridge.set_agent_mode(mode).await;

    // If workspace is provided, also persist to project settings
    if let Some(workspace_path) = workspace {
        let project_settings = ProjectSettingsManager::new(&workspace_path).await;
        project_settings.set_agent_mode(mode.to_string()).await?;
    }

    Ok(())
}

/// Save the agent mode to project settings explicitly.
///
/// # Arguments
/// * `workspace` - The workspace path to save settings to
/// * `mode` - The agent mode to save
#[tauri::command]
pub async fn save_project_agent_mode(
    workspace: PathBuf,
    mode: AgentMode,
) -> Result<(), GolishError> {
    let project_settings = ProjectSettingsManager::new(&workspace).await;
    project_settings
        .set_agent_mode(mode.to_string())
        .await
        .map_err(GolishError::from)
}

/// Get the current agent mode for a session.
///
/// # Arguments
/// * `session_id` - The session ID to get the mode for
///
/// # Returns
/// The current agent mode ("default", "auto-approve", or "planning")
#[tauri::command]
pub async fn get_agent_mode(
    session_id: String,
    state: State<'_, AgentState>,
) -> Result<AgentMode, GolishError> {
    let bridges = state.ai_state.bridges.read().await;
    let bridge = bridges
        .get(&session_id)
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;

    Ok(bridge.get_agent_mode().await)
}

/// Set the execution mode for a session (Chat vs Task).
///
/// - **Chat**: conversational assistant with tools and optional sub-agent delegation
/// - **Task**: PentAGI-style automated orchestration (Generator → Subtasks → Refiner → Reporter)
#[tauri::command]
pub async fn set_execution_mode(
    session_id: String,
    mode: String,
    state: State<'_, AgentState>,
) -> Result<(), GolishError> {
    let bridges = state.ai_state.bridges.read().await;
    let bridge = bridges
        .get(&session_id)
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;

    let parsed: golish_agent_kit::execution_mode::ExecutionMode = mode
        .parse()
        .map_err(|_| format!("Invalid execution mode: '{}'. Use 'chat' or 'task'.", mode))?;

    bridge.set_execution_mode(parsed).await;
    Ok(())
}

/// Get the current execution mode for a session.
#[tauri::command]
pub async fn get_execution_mode(
    session_id: String,
    state: State<'_, AgentState>,
) -> Result<String, GolishError> {
    let bridges = state.ai_state.bridges.read().await;
    let bridge = bridges
        .get(&session_id)
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;

    Ok(bridge.get_execution_mode().await.to_string())
}

/// Descriptor for a registered execution mode policy. Returned by
/// [`list_execution_modes`] so the frontend can render the mode
/// picker without hard-coding `chat` / `task`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionModeDescriptor {
    pub id: String,
    pub display_name: String,
    pub icon: String,
    pub badge_color: String,
    pub description: String,
    pub allows_sub_agents: bool,
}

/// List every execution mode registered in the runtime's
/// `ExecutionModeRegistry`. Today this is `chat` and `task`; future
/// modes (`plan`, `debug`, …) automatically appear once their
/// `ExecutionModePolicy` is added to `ExecutionModeRegistry::default`.
#[tauri::command]
pub async fn list_execution_modes() -> Result<Vec<ExecutionModeDescriptor>, GolishError> {
    let registry = golish_agent_runtime::execution_mode::ExecutionModeRegistry::default();
    let descriptors = registry
        .list_all()
        .into_iter()
        .map(|policy| {
            let label = policy.label();
            ExecutionModeDescriptor {
                id: policy.id().to_string(),
                display_name: label.display_name.to_string(),
                icon: label.icon.to_string(),
                badge_color: label.badge_color.to_string(),
                description: policy.description().to_string(),
                allows_sub_agents: policy.allows_sub_agents(),
            }
        })
        .collect();
    Ok(descriptors)
}
