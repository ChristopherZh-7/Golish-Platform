use std::path::PathBuf;
use std::sync::OnceLock;

/// Platform-specific base directory for Golish application data.
///
/// - macOS: `~/Library/Application Support/golish-platform`
/// - Windows: `~/AppData/Local/golish-platform`
/// - Linux: `~/.golish-platform`
pub fn app_data_base() -> Option<PathBuf> {
    golish_platform::paths::app_data_base("golish-platform")
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

/// Directory containing standalone Asset Intel provider descriptors
/// (0.zone / 360 Quake / FOFA / Hunter / Shodan).
///
/// These are JSON `ToolConfig`s with an `asset_intel` block and an empty
/// `executable` — they're queried through the Asset Intel pipeline, never
/// installed or launched as CLI tools, so they live outside `toolsconfig`
/// (which the Tool Manager scans) to keep the two concerns separate.
/// Resolves from `<project_root>/resources/intel-providers` first,
/// then falls back to the per-machine app data directory.
pub fn intel_providers_dir() -> Option<PathBuf> {
    resolve_shared_dir("intel-providers")
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

/// Path to the bundled `core.json` describing built-in integrations
/// (currently just GitHub Token). Returns `None` when the file is
/// not present (e.g. during unit tests).
///
/// Format: `{ "integrations": [{ "tool_id": "...", "schema": ... }] }`
/// — see `resources/integrations/core.json`.
pub fn integrations_core_file() -> Option<PathBuf> {
    if let Some(res) = project_resources_dir() {
        let p = res.join("integrations").join("core.json");
        if p.is_file() {
            return Some(p);
        }
    }
    app_data_base().map(|b| b.join("integrations").join("core.json"))
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

/// Expand a leading `~` or `~/` into the current user's home directory.
///
/// Returns the original path unchanged when no expansion applies or the
/// home directory cannot be resolved. Single source of truth for the
/// `expand_home_dir` / `expand_tilde` helpers that were copy-pasted across
/// `golish`, `golish-indexer` and `golish-agent-kit`.
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Same as [`expand_tilde`] but returns an owned [`String`]. Useful for
/// `&str`-boundary code such as path completion.
pub fn expand_tilde_string(path: &str) -> String {
    expand_tilde(path).to_string_lossy().into_owned()
}

/// Inverse of [`expand_tilde`]: replace a leading home-directory prefix
/// with `~/` for display. Returns the path unchanged when it is not under
/// the home directory.
pub fn contract_home_dir(path: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(stripped) = path.strip_prefix(&home) {
            return format!("~/{}", stripped.display());
        }
    }
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_data_base_matches_platform_helper() {
        assert_eq!(
            super::app_data_base(),
            golish_platform::paths::app_data_base("golish-platform")
        );
    }

    #[test]
    fn expand_tilde_leaves_absolute_path_untouched() {
        assert_eq!(
            super::expand_tilde("/etc/hosts"),
            std::path::PathBuf::from("/etc/hosts")
        );
    }

    #[test]
    fn expand_tilde_resolves_tilde_prefix() {
        if let Some(home) = dirs::home_dir() {
            assert_eq!(super::expand_tilde("~/foo"), home.join("foo"));
            assert_eq!(super::expand_tilde("~"), home);
        }
    }

    #[test]
    fn contract_home_dir_roundtrips_with_expand() {
        if let Some(home) = dirs::home_dir() {
            let p = home.join("proj").join("x");
            assert_eq!(super::contract_home_dir(&p), "~/proj/x");
        }
    }
}
