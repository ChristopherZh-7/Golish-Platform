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
/// Checks both system PATH (via `which`) and the app's managed tools directory.
#[tauri::command]
pub async fn check_recon_tools_cmd() -> Result<serde_json::Value, String> {
    let tools = [
        "nmap", "subfinder", "httpx", "nuclei", "whatweb", "katana",
        "masscan", "rustscan", "nikto", "ffuf", "gobuster", "dirsearch",
        "feroxbuster", "dig",
    ];

    // Build the app's tools directory path
    let app_tools_dir = golish_core::paths::tools_dir();

    let mut results = Vec::new();
    let mut missing = Vec::new();

    for tool in &tools {
        // Check system PATH
        let in_path = std::process::Command::new("which")
            .arg(tool)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        // Check app's tools directory (case-insensitive directory name match)
        let in_app = app_tools_dir.as_ref().map_or(false, |dir| {
            if !dir.exists() { return false; }
            std::fs::read_dir(dir)
                .ok()
                .map(|entries| {
                    entries.filter_map(|e| e.ok()).any(|e| {
                        let name = e.file_name().to_string_lossy().to_lowercase();
                        name == *tool && e.path().is_dir()
                    })
                })
                .unwrap_or(false)
        });

        let installed = in_path || in_app;
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
