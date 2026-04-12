//! Recon tool commands for Tauri.
//!
//! Pipeline execution is now AI-driven via the agent system.
//! These commands provide tool availability checks.

use tauri::State;

use crate::state::AppState;

/// Run the recon_basic pipeline without requiring AI initialization.
///
/// Deprecated: Pipeline execution is now AI-driven.
/// Kept for API compatibility; returns an error directing callers to use the AI agent.
#[tauri::command]
pub async fn run_recon_pipeline(
    _state: State<'_, AppState>,
    _app: tauri::AppHandle,
    _targets: Vec<String>,
    _project_name: String,
    _project_path: String,
    _session_id: Option<String>,
) -> Result<String, String> {
    Err("Pipeline execution is now AI-driven. Use the AI agent to execute pipelines.".to_string())
}

/// Check if common recon tools are installed.
#[tauri::command]
pub async fn check_recon_tools_cmd() -> Result<serde_json::Value, String> {
    let tools = [
        "nmap", "subfinder", "httpx", "nuclei", "whatweb", "katana",
        "masscan", "rustscan", "nikto", "ffuf", "gobuster", "dirsearch",
        "feroxbuster", "dig",
    ];

    let mut results = Vec::new();
    let mut missing = Vec::new();

    for tool in &tools {
        let installed = std::process::Command::new("which")
            .arg(tool)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !installed {
            missing.push(tool.to_string());
        }
        results.push(serde_json::json!({
            "name": tool,
            "installed": installed,
        }));
    }

    Ok(serde_json::json!({
        "tools": results,
        "all_ready": missing.is_empty(),
        "missing": missing,
    }))
}
