//! Project file storage operations for the hybrid DB + filesystem architecture.
//!
//! Raw captured files (JS, HTML, HTTP dumps, tool output, evidence) are stored
//! on the filesystem under `{project_root}/.golish/`, while structured metadata
//! lives in PostgreSQL with `file_path` references.
//!
//! Directory layout:
//! ```text
//! {project_root}/.golish/
//! ├── project.json          # Pentest project configuration
//! ├── captures/{host}/{port}/{type}/{file}
//! ├── tool-output/{tool}/{timestamp}_{target}.{ext}
//! ├── scripts/{category}/{file}
//! ├── evidence/{finding_id}/{file}
//! ├── analysis/{host}/{type}_{timestamp}.md
//! └── temp/
//! ```

#![allow(dead_code)]

mod crud_ops;
mod import_export;

pub use crud_ops::*;
pub use import_export::*;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

// ── Config types ────────────────────────────────────────────────────────

/// Pentest project configuration stored in `{project_root}/.golish/project.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PentestProjectConfig {
    pub name: String,
    pub created_at: String,
    #[serde(default)]
    pub scope: ScopeConfig,
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default)]
    pub capture: CaptureConfig,
    #[serde(default)]
    pub host_map: std::collections::HashMap<String, Vec<String>>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScopeConfig {
    #[serde(default)]
    pub in_scope: Vec<String>,
    #[serde(default)]
    pub out_of_scope: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxyConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zap_api_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zap_api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    pub auto_save_js: bool,
    pub auto_save_html: bool,
    pub auto_save_tool_output: bool,
    pub max_file_size_mb: u64,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            auto_save_js: true,
            auto_save_html: false,
            auto_save_tool_output: true,
            max_file_size_mb: 50,
        }
    }
}

// ── Directory structure constants ───────────────────────────────────────

const GOLISH_DIR: &str = ".golish";
const CAPTURES_DIR: &str = "captures";
const TOOL_OUTPUT_DIR: &str = "tool-output";
const SCRIPTS_DIR: &str = "scripts";
const EVIDENCE_DIR: &str = "evidence";
const ANALYSIS_DIR: &str = "analysis";
const TEMP_DIR: &str = "temp";
const HOST_INFO_DIR: &str = "_info";

const SCRIPT_CATEGORIES: &[&str] = &["recon", "exploit", "utils"];

// ── Directory initialization ────────────────────────────────────────────

/// Initialize the full `.golish/` directory structure for a project.
pub async fn init_project_dirs(project_root: &Path) -> Result<()> {
    let base = project_root.join(GOLISH_DIR);

    let dirs = [
        base.join(CAPTURES_DIR),
        base.join(TOOL_OUTPUT_DIR),
        base.join(SCRIPTS_DIR).join("recon"),
        base.join(SCRIPTS_DIR).join("exploit"),
        base.join(SCRIPTS_DIR).join("utils"),
        base.join(EVIDENCE_DIR),
        base.join(ANALYSIS_DIR),
        base.join(TEMP_DIR),
    ];

    for dir in &dirs {
        tokio::fs::create_dir_all(dir)
            .await
            .with_context(|| format!("Failed to create directory: {:?}", dir))?;
    }

    tracing::info!("[file-storage] Initialized project dirs at {:?}", base);
    Ok(())
}

/// Initialize a `project.json` file if it doesn't exist.
pub async fn init_project_json(project_root: &Path, name: &str) -> Result<PathBuf> {
    let path = project_root.join(GOLISH_DIR).join("project.json");

    if !path.exists() {
        let config = PentestProjectConfig {
            name: name.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            scope: ScopeConfig::default(),
            proxy: ProxyConfig::default(),
            capture: CaptureConfig::default(),
            host_map: std::collections::HashMap::new(),
            notes: String::new(),
        };
        let json = serde_json::to_string_pretty(&config)?;
        tokio::fs::write(&path, &json).await?;
        tracing::info!("[file-storage] Created project.json at {:?}", path);
    }

    Ok(path)
}

/// Load the pentest project config from `project.json`.
pub async fn load_project_json(project_root: &Path) -> Result<Option<PentestProjectConfig>> {
    let path = project_root.join(GOLISH_DIR).join("project.json");
    if !path.exists() {
        return Ok(None);
    }
    let contents = tokio::fs::read_to_string(&path).await?;
    let config: PentestProjectConfig = serde_json::from_str(&contents)?;
    Ok(Some(config))
}

/// Save the pentest project config to `project.json`.
pub async fn save_project_json(project_root: &Path, config: &PentestProjectConfig) -> Result<()> {
    let path = project_root.join(GOLISH_DIR).join("project.json");
    tokio::fs::create_dir_all(path.parent().unwrap()).await?;
    let json = serde_json::to_string_pretty(config)?;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, &json).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
}

// ── Host resolution & path helpers ──────────────────────────────────────

fn host_slug(host: &str) -> String {
    host.replace(['/', '\\'], "_")
}

fn is_ip(s: &str) -> bool {
    s.parse::<IpAddr>().is_ok()
}

/// Resolve a host identifier to a canonical directory name.
pub fn resolve_host_dir(
    host: &str,
    host_map: &std::collections::HashMap<String, Vec<String>>,
) -> String {
    if is_ip(host) {
        for (ip, hostnames) in host_map {
            if ip == host {
                if let Some(hostname) = hostnames.first() {
                    return host_slug(hostname);
                }
            }
        }
    }
    host_slug(host)
}

pub fn captures_dir(project_root: &Path, host: &str, port: u16) -> PathBuf {
    project_root
        .join(GOLISH_DIR)
        .join(CAPTURES_DIR)
        .join(host_slug(host))
        .join(port.to_string())
}

pub fn host_info_dir(project_root: &Path, host: &str) -> PathBuf {
    project_root
        .join(GOLISH_DIR)
        .join(CAPTURES_DIR)
        .join(host_slug(host))
        .join(HOST_INFO_DIR)
}

pub fn tool_output_dir(project_root: &Path, tool_name: &str) -> PathBuf {
    project_root
        .join(GOLISH_DIR)
        .join(TOOL_OUTPUT_DIR)
        .join(tool_name)
}

pub fn evidence_dir(project_root: &Path, finding_id: &str) -> PathBuf {
    project_root
        .join(GOLISH_DIR)
        .join(EVIDENCE_DIR)
        .join(finding_id)
}

pub fn analysis_dir(project_root: &Path, host: &str) -> PathBuf {
    project_root
        .join(GOLISH_DIR)
        .join(ANALYSIS_DIR)
        .join(host_slug(host))
}

pub fn scripts_dir(project_root: &Path, category: &str) -> PathBuf {
    let cat = if SCRIPT_CATEGORIES.contains(&category) {
        category
    } else {
        "utils"
    };
    project_root.join(GOLISH_DIR).join(SCRIPTS_DIR).join(cat)
}

pub fn temp_dir(project_root: &Path) -> PathBuf {
    project_root.join(GOLISH_DIR).join(TEMP_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_slug() {
        assert_eq!(host_slug("example.com"), "example.com");
        assert_eq!(host_slug("10.0.0.1"), "10.0.0.1");
        assert_eq!(host_slug("192.168.1.0/24"), "192.168.1.0_24");
    }

    #[test]
    fn test_is_ip() {
        assert!(is_ip("10.0.0.1"));
        assert!(is_ip("192.168.1.1"));
        assert!(is_ip("::1"));
        assert!(!is_ip("example.com"));
        assert!(!is_ip("admin.example.com"));
    }

    #[test]
    fn test_resolve_host_dir_prefers_hostname() {
        let mut map = std::collections::HashMap::new();
        map.insert("93.184.216.34".to_string(), vec!["example.com".to_string()]);

        assert_eq!(resolve_host_dir("93.184.216.34", &map), "example.com");
        assert_eq!(resolve_host_dir("example.com", &map), "example.com");
        assert_eq!(resolve_host_dir("10.0.0.1", &map), "10.0.0.1");
    }

    #[test]
    fn test_captures_dir() {
        let root = PathBuf::from("/projects/test");
        let dir = captures_dir(&root, "example.com", 443);
        assert_eq!(
            dir,
            PathBuf::from("/projects/test/.golish/captures/example.com/443")
        );
    }

    #[test]
    fn test_tool_output_dir() {
        let root = PathBuf::from("/projects/test");
        let dir = tool_output_dir(&root, "nmap");
        assert_eq!(
            dir,
            PathBuf::from("/projects/test/.golish/tool-output/nmap")
        );
    }

    #[test]
    fn test_scripts_dir_valid_category() {
        let root = PathBuf::from("/projects/test");
        assert_eq!(
            scripts_dir(&root, "exploit"),
            PathBuf::from("/projects/test/.golish/scripts/exploit")
        );
    }

    #[test]
    fn test_scripts_dir_unknown_category_defaults_to_utils() {
        let root = PathBuf::from("/projects/test");
        assert_eq!(
            scripts_dir(&root, "unknown"),
            PathBuf::from("/projects/test/.golish/scripts/utils")
        );
    }
}
