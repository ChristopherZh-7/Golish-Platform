#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{McpConfigFile, McpServerConfig};

const TRUSTED_CONFIGS_FILENAME: &str = "trusted-mcp-configs.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrustedMcpConfigs {
    #[serde(default)]
    pub trusted_paths: HashSet<String>,
}

/// Return the set of built-in MCP server names.
pub fn builtin_server_names() -> HashSet<String> {
    builtin_configs().into_keys().collect()
}

/// Built-in MCP servers that ship with Golish.
///
/// These are always available and cannot be removed by the user.
/// User/project configs with the same name will override them.
fn builtin_configs() -> HashMap<String, McpServerConfig> {
    let mut servers = HashMap::new();

    let js_reverse_path = [
        "js-reverse-mcp/build/src/index.js",
        "js-reverse-mcp/src/index.js",
    ]
    .into_iter()
    .filter_map(resolve_builtin_tool_path)
    .find(|entry_point| {
        js_reverse_entry_point_has_generated_devtools_runtime(Path::new(entry_point))
    });
    if let Some(entry_point) = js_reverse_path {
        servers.insert(
            "js-reverse".to_string(),
            McpServerConfig {
                transport: None,
                command: Some("node".to_string()),
                args: vec![entry_point],
                env: Default::default(),
                url: None,
                headers: Default::default(),
                enabled: true,
                timeout: 60,
                oauth: None,
            },
        );
    }

    servers
}

/// Return whether the js-reverse entry point has the generated DevTools runtime
/// entry that its source and packaged build import at startup.
fn js_reverse_entry_point_has_generated_devtools_runtime(entry_point: &Path) -> bool {
    if !entry_point.is_file() {
        return false;
    }

    let Some(runtime_root) = entry_point.parent().and_then(Path::parent) else {
        return false;
    };
    let devtools_root = runtime_root.join("node_modules/chrome-devtools-frontend");
    devtools_root.join("mcp/mcp.js").is_file()
        && devtools_root
            .join("front_end/core/common/common.js")
            .is_file()
}

/// Resolve the canonical directory for a known built-in server setup action.
///
/// This registry is intentionally independent from merged user/project config
/// so an override can never redirect `npm install` or `npm run build`.
pub fn builtin_setup_directory(server_name: &str) -> Option<PathBuf> {
    let manifest_path = match server_name {
        "js-reverse" => "js-reverse-mcp/package.json",
        _ => return None,
    };
    let manifest_path = PathBuf::from(resolve_builtin_tool_path(manifest_path)?);
    manifest_path.parent().map(Path::to_path_buf)
}

/// Resolve the absolute path to a built-in tool's entry point.
///
/// Searches candidate directories for `tools/{rel_path}`. Candidates (in order):
/// 1. Relative to the executable (production bundles and local target dirs)
/// 2. The compile-time repository root (development builds)
///
/// Runtime workspace/cwd paths are deliberately excluded: a project directory
/// is untrusted input and must never be able to impersonate a built-in tool.
fn resolve_builtin_tool_path(rel_path: &str) -> Option<String> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.to_path_buf());
            candidates.push(exe_dir.join("../Resources"));
            candidates.push(exe_dir.join(".."));
            candidates.push(exe_dir.join("../.."));
            candidates.push(exe_dir.join("../../.."));
            candidates.push(exe_dir.join("../../../.."));
        }
    }

    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."));

    for base in candidates {
        let full = base.join("tools").join(rel_path);
        if full.exists() {
            return full
                .canonicalize()
                .ok()
                .map(|p| p.to_string_lossy().to_string());
        }
    }

    None
}

/// Load and merge MCP configs from builtin, user-global, and project locations.
pub fn load_mcp_config(project_dir: &Path) -> Result<McpConfigFile> {
    load_mcp_config_inner(
        user_config_path(),
        project_dir,
        is_project_config_trusted(project_dir),
    )
}

fn load_mcp_config_inner(
    user_config: Option<PathBuf>,
    project_dir: &Path,
    project_config_trusted: bool,
) -> Result<McpConfigFile> {
    let mut merged = McpConfigFile::default();

    // 0. Load built-in configs (lowest priority)
    merged.mcp_servers.extend(builtin_configs());

    // 1. Load user-global config (~/.golish/mcp.json)
    if let Some(path) = user_config {
        if path.exists() {
            let user_config: McpConfigFile = load_json_file(&path)
                .with_context(|| format!("Failed to load MCP config at {}", path.display()))?;
            merged.mcp_servers.extend(user_config.mcp_servers);
        }
    }

    // 2. Load project config (<project>/.golish/mcp.json)
    let project_config_path = project_dir.join(".golish/mcp.json");
    if project_config_trusted && project_config_path.exists() {
        let project_config: McpConfigFile =
            load_json_file(&project_config_path).with_context(|| {
                format!(
                    "Failed to load MCP config at {}",
                    project_config_path.display()
                )
            })?;
        merged.mcp_servers.extend(project_config.mcp_servers);
    }

    Ok(merged)
}

/// Interpolate environment variables in config values.
/// Supports both $VAR and ${VAR} syntax.
pub fn interpolate_env_vars(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '$' {
            out.push(ch);
            continue;
        }

        match chars.peek() {
            Some('{') => {
                chars.next(); // consume '{'
                let mut var_name = String::new();
                let mut found_close = false;
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next == '}' {
                        found_close = true;
                        break;
                    }
                    var_name.push(next);
                }
                if var_name.is_empty() {
                    out.push('$');
                    out.push('{');
                    if found_close {
                        out.push('}');
                    }
                    continue;
                }
                if let Ok(value) = std::env::var(&var_name) {
                    out.push_str(&value);
                }
            }
            Some(next) if is_var_start(*next) => {
                let mut var_name = String::new();
                while let Some(&next) = chars.peek() {
                    if !is_var_char(next) {
                        break;
                    }
                    chars.next();
                    var_name.push(next);
                }
                if let Ok(value) = std::env::var(&var_name) {
                    out.push_str(&value);
                }
            }
            _ => {
                out.push('$');
            }
        }
    }

    out
}

/// Check if a project's MCP config has been approved.
pub fn is_project_config_trusted(project_dir: &Path) -> bool {
    let Some(path) = trusted_configs_path() else {
        return false;
    };
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(trusted) = serde_json::from_str::<TrustedMcpConfigs>(&contents) else {
        return false;
    };
    let Ok(project_path) = project_dir.canonicalize() else {
        return false;
    };
    trusted
        .trusted_paths
        .contains(&project_path.to_string_lossy().to_string())
}

/// Mark a project's MCP config as trusted (after user approval).
pub fn trust_project_config(project_dir: &Path) -> Result<()> {
    let Some(path) = trusted_configs_path() else {
        return Ok(());
    };
    let mut trusted = if path.exists() {
        let contents = fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str::<TrustedMcpConfigs>(&contents).unwrap_or_default()
    } else {
        TrustedMcpConfigs::default()
    };
    let project_path = project_dir
        .canonicalize()
        .unwrap_or_else(|_| project_dir.to_path_buf());
    trusted
        .trusted_paths
        .insert(project_path.to_string_lossy().to_string());

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create MCP trust directory at {}",
                parent.display()
            )
        })?;
    }
    let serialized = serde_json::to_string_pretty(&trusted)?;
    fs::write(&path, serialized)?;
    Ok(())
}

fn load_json_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let contents = fs::read_to_string(path)?;
    let config = serde_json::from_str(&contents)?;
    Ok(config)
}

fn user_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".golish/mcp.json"))
}

fn trusted_configs_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".golish").join(TRUSTED_CONFIGS_FILENAME))
}

fn is_var_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_var_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}
