//! Recon tool commands for Tauri.
//!
//! Pipeline execution is now AI-driven via the agent system.
//! These commands provide tool availability checks.

use crate::error::GolishError;
use serde::Serialize;

/// Availability of a single recon tool (name + whether it is installed).
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct ToolStatus {
    pub name: String,
    pub installed: bool,
}

/// Result of checking all common recon tools for availability.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct ReconToolCheck {
    pub tools: Vec<ToolStatus>,
    pub all_ready: bool,
    pub missing: Vec<String>,
}

/// Check if common recon tools are installed.
/// Uses unified pentest preflight checks for consistency.
#[tauri::command]
pub async fn check_recon_tools_cmd() -> Result<ReconToolCheck, GolishError> {
    let tools = [
        "nmap",
        "subfinder",
        "httpx",
        "nuclei",
        "whatweb",
        "katana",
        "masscan",
        "rustscan",
        "nikto",
        "ffuf",
        "gobuster",
        "dirsearch",
        "feroxbuster",
        "dig",
    ];

    let mut results = Vec::new();
    let mut missing = Vec::new();
    let config_manager = golish_pentest::ConfigManager::with_defaults();

    for tool in &tools {
        let preflight = golish_pentest::preflight_tool(
            tool,
            &config_manager,
            golish_pentest::PreflightMode::AllowPathFallback,
        )
        .await;
        if !preflight.ready {
            missing.push(tool.to_string());
        }
        results.push(ToolStatus {
            name: tool.to_string(),
            installed: preflight.installed,
        });
    }

    Ok(ReconToolCheck {
        all_ready: missing.is_empty(),
        tools: results,
        missing,
    })
}
