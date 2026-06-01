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

/// Set the execution mode for a session.
///
/// `mode` is the unified picker id:
/// - **`"chat"`** → conversational single-agent engine, no harness profile.
/// - **`"<profile_id>"`** (e.g. `"assessment"` / `"red_team"`) → Task engine
///   running the named harness operation profile.
/// - **`"task"`** (legacy) → Task engine with the env-default profile.
#[tauri::command]
pub async fn set_execution_mode(
    session_id: String,
    mode: String,
    state: State<'_, AgentState>,
) -> Result<(), GolishError> {
    use golish_agent_kit::execution_mode::ExecutionMode;

    let bridges = state.ai_state.bridges.read().await;
    let bridge = bridges
        .get(&session_id)
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;

    match mode.as_str() {
        "chat" => {
            bridge.set_execution_mode(ExecutionMode::Chat).await;
            bridge.set_harness_profile(None).await;
        }
        // Legacy bare "task": orchestrate with the env-default harness profile.
        "task" => {
            bridge.set_execution_mode(ExecutionMode::Task).await;
            bridge.set_harness_profile(None).await;
        }
        id if golish_agent_kit::harness::EMBEDDED_PROFILE_IDS.contains(&id) => {
            bridge.set_execution_mode(ExecutionMode::Task).await;
            bridge.set_harness_profile(Some(id.to_string())).await;
        }
        other => {
            return Err(format!(
                "Invalid execution mode: '{}'. Use 'chat' or a harness profile id.",
                other
            )
            .into());
        }
    }
    Ok(())
}

/// Get the current execution mode for a session, as the unified picker id
/// (`"chat"` or the active harness profile id).
#[tauri::command]
pub async fn get_execution_mode(
    session_id: String,
    state: State<'_, AgentState>,
) -> Result<String, GolishError> {
    let bridges = state.ai_state.bridges.read().await;
    let bridge = bridges
        .get(&session_id)
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;

    let mode = bridge.get_execution_mode().await;
    if mode.is_task() {
        if let Some(profile) = bridge.get_harness_profile().await {
            return Ok(profile);
        }
    }
    Ok(mode.to_string())
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

/// List the modes shown in the chat-panel picker: the `chat` engine plus one
/// entry per embedded harness profile. Adding a profile JSON (and its id to
/// `EMBEDDED_PROFILE_IDS`) automatically surfaces a new mode — no frontend
/// change required. The legacy generic `task` entry is intentionally omitted:
/// every orchestrated run is now chosen via a profile.
#[tauri::command]
pub async fn list_execution_modes() -> Result<Vec<ExecutionModeDescriptor>, GolishError> {
    let mut descriptors: Vec<ExecutionModeDescriptor> = Vec::new();

    let registry = golish_agent_runtime::execution_mode::ExecutionModeRegistry::default();
    if let Some(chat) = registry.get("chat") {
        let label = chat.label();
        descriptors.push(ExecutionModeDescriptor {
            id: chat.id().to_string(),
            display_name: label.display_name.to_string(),
            icon: label.icon.to_string(),
            badge_color: label.badge_color.to_string(),
            description: chat.description().to_string(),
            allows_sub_agents: chat.allows_sub_agents(),
        });
    }

    for id in golish_agent_kit::harness::EMBEDDED_PROFILE_IDS {
        let Ok(Some(profile)) = golish_agent_kit::harness::load_embedded_profile(id) else {
            continue;
        };
        descriptors.push(ExecutionModeDescriptor {
            id: (*id).to_string(),
            display_name: profile.display_name.clone(),
            icon: "Zap".to_string(),
            badge_color: profile_badge_color(id).to_string(),
            description: profile_mode_description(&profile),
            allows_sub_agents: true,
        });
    }

    Ok(descriptors)
}

/// Stable badge color per profile (mirrors the picker color palette).
fn profile_badge_color(id: &str) -> &'static str {
    match id {
        "assessment" => "green",
        "pentest" => "blue",
        "red_team" => "magenta",
        _ => "muted",
    }
}

/// Short, human one-line picker description, keyed off the profile's
/// authorization ceiling (the display name already conveys the profile kind).
fn profile_mode_description(profile: &golish_agent_kit::harness::Profile) -> String {
    use golish_agent_kit::harness::AuthorizationLevel as L;
    match profile.max_authorization {
        L::ObserveOnly => "Read-only, no probing",
        L::PassiveIntel => "Passive intel only",
        L::ActiveRecon => "Passive + active recon",
        L::VulnValidation => "Recon + vuln scanning",
        L::ControlledExploit => "Incl. controlled exploit validation",
        L::PostExploitRedTeam => "Full red team, incl. post-exploitation",
    }
    .to_string()
}
