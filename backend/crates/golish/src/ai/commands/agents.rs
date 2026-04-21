//! Tauri commands for managing sub-agent definitions.
//!
//! These commands expose the file-based agent system to the frontend,
//! allowing listing, reading, saving, and deleting agent definitions.

use golish_sub_agents::file_loader::{serialize_agent_to_md, AgentFileInfo};
use golish_sub_agents::discovery::{discover_agents, seed_default_agent_files};
use golish_sub_agents::definition::AgentSource;
use std::path::PathBuf;

/// List all discovered agents (built-in + file-based).
#[tauri::command]
pub async fn list_agent_definitions(
    working_directory: Option<String>,
) -> Result<Vec<AgentFileInfo>, String> {
    let workspace = working_directory.map(PathBuf::from);
    let agents = discover_agents(workspace.as_deref());
    Ok(agents.iter().map(AgentFileInfo::from).collect())
}

/// Read the full body (system prompt) of an agent file.
#[tauri::command]
pub async fn read_agent_prompt(agent_id: String, working_directory: Option<String>) -> Result<String, String> {
    let workspace = working_directory.map(PathBuf::from);
    let agents = discover_agents(workspace.as_deref());
    let agent = agents
        .iter()
        .find(|a| a.id == agent_id)
        .ok_or_else(|| format!("Agent '{}' not found", agent_id))?;
    Ok(agent.system_prompt.clone())
}

/// Save (create or update) an agent definition to a `.md` file.
///
/// When `scope` is `"project"`, saves to `<workspace>/.golish/agents/`.
/// Otherwise saves to `~/.golish/agents/` (global).
/// Existing file-based agents keep their current location unless `scope` is explicitly provided.
#[tauri::command]
pub async fn save_agent_definition(
    agent_id: String,
    name: String,
    description: String,
    system_prompt: String,
    allowed_tools: Vec<String>,
    max_iterations: Option<usize>,
    timeout_secs: Option<u64>,
    idle_timeout_secs: Option<u64>,
    readonly: Option<bool>,
    is_background: Option<bool>,
    model: Option<String>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    top_p: Option<f32>,
    scope: Option<String>,
    working_directory: Option<String>,
) -> Result<String, String> {
    let workspace = working_directory.map(PathBuf::from);

    // Find if this agent already exists (to get its file path)
    let agents = discover_agents(workspace.as_deref());
    let existing = agents.iter().find(|a| a.id == agent_id);

    let file_path = if let Some(scope_str) = &scope {
        // Explicit scope provided — force save location
        match scope_str.as_str() {
            "project" => {
                let ws = workspace.as_ref().ok_or("Working directory required for project agents")?;
                let agents_dir = ws.join(".golish").join("agents");
                std::fs::create_dir_all(&agents_dir).map_err(|e| e.to_string())?;
                agents_dir.join(format!("{agent_id}.md"))
            }
            _ => {
                let home = dirs::home_dir().ok_or("No home directory")?;
                let agents_dir = home.join(".golish").join("agents");
                std::fs::create_dir_all(&agents_dir).map_err(|e| e.to_string())?;
                agents_dir.join(format!("{agent_id}.md"))
            }
        }
    } else {
        match existing {
            Some(agent) => match &agent.source {
                AgentSource::File(path) => path.clone(),
                AgentSource::BuiltIn => {
                    let home = dirs::home_dir().ok_or("No home directory")?;
                    let agents_dir = home.join(".golish").join("agents");
                    std::fs::create_dir_all(&agents_dir).map_err(|e| e.to_string())?;
                    agents_dir.join(format!("{agent_id}.md"))
                }
            },
            None => {
                let home = dirs::home_dir().ok_or("No home directory")?;
                let agents_dir = home.join(".golish").join("agents");
                std::fs::create_dir_all(&agents_dir).map_err(|e| e.to_string())?;
                agents_dir.join(format!("{agent_id}.md"))
            }
        }
    };

    let mut def = golish_sub_agents::SubAgentDefinition::new(
        &agent_id,
        name,
        description,
        system_prompt,
    );
    def.allowed_tools = allowed_tools;
    if let Some(max) = max_iterations {
        def.max_iterations = max;
    }
    if let Some(t) = timeout_secs {
        def.timeout_secs = Some(t);
    }
    if let Some(t) = idle_timeout_secs {
        def.idle_timeout_secs = Some(t);
    }
    if let Some(r) = readonly {
        def.readonly = r;
    }
    if let Some(b) = is_background {
        def.is_background = b;
    }
    if let Some(m) = model {
        if m != "inherit" && !m.is_empty() {
            def.model_override = Some(("auto".to_string(), m));
        }
    }
    def.temperature = temperature;
    def.max_tokens = max_tokens;
    def.top_p = top_p;

    let content = serialize_agent_to_md(&def);
    std::fs::write(&file_path, content).map_err(|e| e.to_string())?;

    Ok(file_path.to_string_lossy().to_string())
}

/// Delete an agent definition file.
/// System agents (worker, memorist, reflector) cannot be deleted.
#[tauri::command]
pub async fn delete_agent_definition(
    agent_id: String,
    working_directory: Option<String>,
) -> Result<(), String> {
    let workspace = working_directory.map(PathBuf::from);
    let agents = discover_agents(workspace.as_deref());
    let agent = agents
        .iter()
        .find(|a| a.id == agent_id)
        .ok_or_else(|| format!("Agent '{}' not found", agent_id))?;

    if agent.is_system {
        return Err(format!(
            "Cannot delete system agent '{}'. System agents are required for runtime operation.",
            agent_id
        ));
    }

    match &agent.source {
        AgentSource::File(path) => {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
            Ok(())
        }
        AgentSource::BuiltIn => Err(format!(
            "Cannot delete built-in agent '{}'. Create a file override first.",
            agent_id
        )),
    }
}

/// Seed default agent files if they don't exist.
/// Called during app initialization.
#[tauri::command]
pub async fn seed_agents() -> Result<usize, String> {
    seed_default_agent_files().map_err(|e| e.to_string())
}
