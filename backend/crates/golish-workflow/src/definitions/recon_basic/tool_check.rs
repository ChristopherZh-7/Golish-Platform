use serde::{Deserialize, Serialize};

use super::state::AvailableTools;

fn which_check(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStatus {
    pub name: String,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconToolCheck {
    pub tools: Vec<ToolStatus>,
    pub all_ready: bool,
    pub missing: Vec<String>,
}

/// Check which recon tools are installed. Returns a status report
/// that the frontend can use to decide whether to allow pipeline execution.
pub fn check_recon_tools() -> ReconToolCheck {
    let config = golish_pentest::PentestConfig::default();
    let scan = golish_pentest::scan_toolsconfig_with_status(
        config.toolsconfig_dir(),
        config.tools_dir(),
    );

    let is_installed = |id: &str| -> bool {
        scan.tools
            .iter()
            .find(|t| t.id == id || t.executable.split('/').last().unwrap_or(&t.executable) == id)
            .map(|t| t.installed)
            .unwrap_or_else(|| which_check(id))
    };

    let checks = vec![
        ("dig", which_check("dig")),
        ("curl", which_check("curl")),
        ("nc", which_check("nc")),
        ("nmap", is_installed("nmap")),
        ("subfinder", is_installed("subfinder")),
        ("httpx", is_installed("httpx")),
        ("whatweb", is_installed("whatweb")),
    ];

    let tools: Vec<ToolStatus> = checks
        .iter()
        .map(|(name, installed)| ToolStatus {
            name: name.to_string(),
            installed: *installed,
        })
        .collect();

    let missing: Vec<String> = checks
        .iter()
        .filter(|(_, installed)| !installed)
        .map(|(name, _)| name.to_string())
        .collect();

    let all_ready = missing.is_empty();

    ReconToolCheck {
        tools,
        all_ready,
        missing,
    }
}

/// Get the AvailableTools struct from the check results.
/// Used by the pipeline's InitializeTask to populate the state.
pub fn get_available_tools() -> AvailableTools {
    let check = check_recon_tools();
    let find = |name: &str| check.tools.iter().find(|t| t.name == name).map(|t| t.installed).unwrap_or(false);

    AvailableTools {
        dig: find("dig"),
        curl: find("curl"),
        nc: find("nc"),
        nmap: find("nmap"),
        subfinder: find("subfinder"),
        httpx: find("httpx"),
        whatweb: find("whatweb"),
    }
}
