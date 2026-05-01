use std::path::PathBuf;
use std::sync::OnceLock;

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

static PROJECT_ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Detect the project root by walking up from the current exe or CARGO_MANIFEST_DIR.
/// Returns `<project_root>/resources` if found.
fn project_resources_dir() -> Option<&'static PathBuf> {
    PROJECT_ROOT
        .get_or_init(|| {
            if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
                let mut p = PathBuf::from(dir);
                while p.parent().is_some() {
                    let res = p.join("resources");
                    if res.is_dir() {
                        return Some(res);
                    }
                    if !p.pop() {
                        break;
                    }
                }
            }
            if let Ok(exe) = std::env::current_exe() {
                let mut p = exe;
                for _ in 0..10 {
                    if !p.pop() {
                        break;
                    }
                    let res = p.join("resources");
                    if res.is_dir() && p.join("backend").is_dir() {
                        return Some(res);
                    }
                }
            }
            None
        })
        .as_ref()
}

/// Resolve a shared resource path: project `resources/<name>` first,
/// then fall back to `app_data_base/<name>`.
fn resolve_shared_dir(name: &str) -> Option<PathBuf> {
    if let Some(res) = project_resources_dir() {
        let dir = res.join(name);
        if dir.is_dir() {
            return Some(dir);
        }
    }
    app_data_base().map(|b| b.join(name))
}

/// Directory containing tool binaries / installations.
pub fn tools_dir() -> Option<PathBuf> {
    app_data_base().map(|b| b.join("tools"))
}

/// Directory containing per-tool JSON configuration files.
/// Resolves from `<project_root>/resources/toolsconfig` first,
/// then falls back to the per-machine app data directory.
pub fn toolsconfig_dir() -> Option<PathBuf> {
    resolve_shared_dir("toolsconfig")
}

/// Wiki / vulnerability knowledge-base root.
/// Resolves from `<project_root>/resources/wiki` first,
/// then falls back to the per-machine app data directory.
pub fn wiki_dir() -> Option<PathBuf> {
    resolve_shared_dir("wiki")
}

/// Directory containing flow/pipeline template files.
/// Resolves from `<project_root>/resources/flow-templates` first,
/// then falls back to the per-machine app data directory.
pub fn flow_templates_dir() -> Option<PathBuf> {
    resolve_shared_dir("flow-templates")
}

/// Per-tool skill/usage-guide Markdown files.
/// Resolves from `<project_root>/resources/skills` first,
/// then falls back to the per-machine app data directory.
pub fn skills_dir() -> Option<PathBuf> {
    resolve_shared_dir("skills")
}

/// Embedded PostgreSQL data directory.
pub fn pg_data_dir() -> Option<PathBuf> {
    app_data_base().map(|b| b.join("pgdata"))
}

/// Wordlists directory.
/// Resolves from `<project_root>/resources/wordlists` first,
/// then falls back to the per-machine app data directory.
pub fn wordlists_dir() -> Option<PathBuf> {
    resolve_shared_dir("wordlists")
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
