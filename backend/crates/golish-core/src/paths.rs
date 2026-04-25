use std::path::PathBuf;

/// Platform-specific base directory for Golish application data.
///
/// - macOS: `~/Library/Application Support/golish-platform`
/// - Windows: `~/AppData/Local/golish-platform`
/// - Linux: `~/.golish-platform`
pub fn app_data_base() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    #[cfg(target_os = "macos")]
    let base = home
        .join("Library")
        .join("Application Support")
        .join("golish-platform");
    #[cfg(target_os = "windows")]
    let base = home
        .join("AppData")
        .join("Local")
        .join("golish-platform");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = home.join(".golish-platform");
    Some(base)
}

/// Directory containing tool binaries / installations.
pub fn tools_dir() -> Option<PathBuf> {
    app_data_base().map(|b| b.join("tools"))
}

/// Directory containing per-tool JSON configuration files.
pub fn toolsconfig_dir() -> Option<PathBuf> {
    app_data_base().map(|b| b.join("toolsconfig"))
}

/// Directory containing flow/pipeline template files.
pub fn flow_templates_dir() -> Option<PathBuf> {
    app_data_base().map(|b| b.join("flow-templates"))
}

/// Embedded PostgreSQL data directory.
pub fn pg_data_dir() -> Option<PathBuf> {
    app_data_base().map(|b| b.join("pgdata"))
}

/// Wordlists directory.
pub fn wordlists_dir() -> Option<PathBuf> {
    app_data_base().map(|b| b.join("wordlists"))
}

/// Per-workspace `.golish` directory.
///
/// For a real workspace path, returns `{workspace}/.golish`.
/// For empty or "." workspace, falls back to `~/.golish`.
pub fn golish_dir_for_workspace(workspace: &std::path::Path) -> PathBuf {
    let ws_str = workspace.to_string_lossy();
    if !ws_str.is_empty() && ws_str != "." {
        workspace.join(".golish")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".golish")
    }
}
