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

static PROJECT_RESOURCE_DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// Detect every usable resource root by walking up from `CARGO_MANIFEST_DIR`
/// and the current executable. A built binary may have a partial
/// `target/debug/resources` bundle (for example methodology only) while the
/// source checkout still owns `resources/toolsconfig`; callers therefore must
/// resolve each resource family independently instead of caching one global
/// winner.
fn project_resource_dirs() -> &'static [PathBuf] {
    PROJECT_RESOURCE_DIRS
        .get_or_init(|| {
            let mut roots = Vec::new();
            if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
                let mut p = PathBuf::from(dir);
                while p.parent().is_some() {
                    let res = p.join("resources");
                    if res.is_dir() && !roots.contains(&res) {
                        roots.push(res);
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
                    for res in [p.join("resources"), p.join("Resources").join("resources")] {
                        if res.is_dir() && !roots.contains(&res) {
                            roots.push(res);
                        }
                    }
                }
            }
            roots
        })
        .as_slice()
}

/// Resolve a shared resource path: project `resources/<name>` first,
/// then fall back to `app_data_base/<name>`.
fn resolve_shared_dir(name: &str) -> Option<PathBuf> {
    if let Some(dir) = project_resource_dirs()
        .iter()
        .map(|resources| resources.join(name))
        .find(|dir| dir.is_dir())
    {
        return Some(dir);
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

/// Checked-in or application-resource CyberStrike methodology corpus.
///
/// This is a read-only, content-addressed knowledge corpus. Authorization to
/// query it is enforced separately by the methodology manifest trust policy.
pub fn bundled_methodology_corpus_dir() -> Option<PathBuf> {
    project_resource_dirs()
        .iter()
        .map(|resources| {
            resources
                .join("methodology")
                .join("corpora")
                .join("cyberstrike")
        })
        .find(|corpus| corpus.join("manifest.json").is_file())
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
    if let Some(path) = project_resource_dirs()
        .iter()
        .map(|resources| resources.join("integrations").join("core.json"))
        .find(|path| path.is_file())
    {
        return Some(path);
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

    #[test]
    fn bundled_cyberstrike_methodology_corpus_resolves_from_project_resources() {
        let root = super::bundled_methodology_corpus_dir()
            .expect("checked-in CyberStrike methodology corpus should resolve");
        assert!(root.join("manifest.json").is_file());
        assert!(root.join("skills/recon-methodology/SKILL.md").is_file());
    }

    #[test]
    fn partial_binary_resource_bundle_does_not_shadow_checked_in_toolsconfig() {
        let root = super::toolsconfig_dir()
            .expect("checked-in toolsconfig should resolve independently of methodology");
        assert!(root.join("httpx.json").is_file());
        assert!(root.join("naabu.json").is_file());
    }
}
