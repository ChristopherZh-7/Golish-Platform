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

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

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

// ============================================================================
// Directory structure constants
// ============================================================================

const GOLISH_DIR: &str = ".golish";
const CAPTURES_DIR: &str = "captures";
const TOOL_OUTPUT_DIR: &str = "tool-output";
const SCRIPTS_DIR: &str = "scripts";
const EVIDENCE_DIR: &str = "evidence";
const ANALYSIS_DIR: &str = "analysis";
const TEMP_DIR: &str = "temp";
const HOST_INFO_DIR: &str = "_info";

const SCRIPT_CATEGORIES: &[&str] = &["recon", "exploit", "utils"];

// ============================================================================
// Directory initialization
// ============================================================================

/// Initialize the full `.golish/` directory structure for a project.
/// Called when creating a new project or when the structure is missing.
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
pub async fn save_project_json(
    project_root: &Path,
    config: &PentestProjectConfig,
) -> Result<()> {
    let path = project_root.join(GOLISH_DIR).join("project.json");
    tokio::fs::create_dir_all(path.parent().unwrap()).await?;
    let json = serde_json::to_string_pretty(config)?;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, &json).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
}

// ============================================================================
// Host resolution & path helpers
// ============================================================================

/// Slugify a hostname or IP for use as a directory name.
/// `admin.example.com` → `admin.example.com` (dots kept for readability)
/// `10.0.0.1` → `10.0.0.1`
/// `192.168.1.0/24` → `192.168.1.0_24`
fn host_slug(host: &str) -> String {
    host.replace('/', "_").replace('\\', "_")
}

/// Check if a string looks like an IP address.
fn is_ip(s: &str) -> bool {
    s.parse::<IpAddr>().is_ok()
}

/// Resolve a host identifier to a canonical directory name.
/// Prefers hostname over IP when a mapping is known.
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

/// Build the captures base path: `{root}/.golish/captures/{host}/{port}/`
pub fn captures_dir(project_root: &Path, host: &str, port: u16) -> PathBuf {
    project_root
        .join(GOLISH_DIR)
        .join(CAPTURES_DIR)
        .join(host_slug(host))
        .join(port.to_string())
}

/// Build the host info path: `{root}/.golish/captures/{host}/_info/`
pub fn host_info_dir(project_root: &Path, host: &str) -> PathBuf {
    project_root
        .join(GOLISH_DIR)
        .join(CAPTURES_DIR)
        .join(host_slug(host))
        .join(HOST_INFO_DIR)
}

/// Build the tool output path: `{root}/.golish/tool-output/{tool}/`
pub fn tool_output_dir(project_root: &Path, tool_name: &str) -> PathBuf {
    project_root
        .join(GOLISH_DIR)
        .join(TOOL_OUTPUT_DIR)
        .join(tool_name)
}

/// Build the evidence path: `{root}/.golish/evidence/{finding_id}/`
pub fn evidence_dir(project_root: &Path, finding_id: &str) -> PathBuf {
    project_root
        .join(GOLISH_DIR)
        .join(EVIDENCE_DIR)
        .join(finding_id)
}

/// Build the analysis path: `{root}/.golish/analysis/{host}/`
pub fn analysis_dir(project_root: &Path, host: &str) -> PathBuf {
    project_root
        .join(GOLISH_DIR)
        .join(ANALYSIS_DIR)
        .join(host_slug(host))
}

/// Build the scripts path: `{root}/.golish/scripts/{category}/`
pub fn scripts_dir(project_root: &Path, category: &str) -> PathBuf {
    let cat = if SCRIPT_CATEGORIES.contains(&category) {
        category
    } else {
        "utils"
    };
    project_root
        .join(GOLISH_DIR)
        .join(SCRIPTS_DIR)
        .join(cat)
}

/// Build the temp path: `{root}/.golish/temp/`
pub fn temp_dir(project_root: &Path) -> PathBuf {
    project_root.join(GOLISH_DIR).join(TEMP_DIR)
}

// ============================================================================
// File save operations
// ============================================================================

/// Compute SHA-256 hash prefix (first 8 hex chars) for a file's content.
fn sha256_prefix(content: &[u8]) -> String {
    let hash = Sha256::digest(content);
    format!("{:x}", hash)[..8].to_string()
}

/// Generate a filename with hash prefix: `{sha256_8}_{original_name}`
fn hashed_filename(original_name: &str, content: &[u8]) -> String {
    let prefix = sha256_prefix(content);
    format!("{}_{}", prefix, sanitize_filename(original_name))
}

/// Sanitize a filename to be filesystem-safe.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c => c,
        })
        .collect()
}

/// Sanitize a URL path segment for use as a filename component.
fn url_path_slug(url_path: &str) -> String {
    url_path
        .trim_start_matches('/')
        .replace('/', "-")
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_'
            ,
            c => c,
        })
        .collect::<String>()
        .chars()
        .take(100)
        .collect()
}

/// Save a captured JS file. Preserves original URL path structure.
/// Returns the relative path from project root.
pub async fn save_js_capture(
    project_root: &Path,
    host: &str,
    port: u16,
    filename: &str,
    content: &[u8],
    url_path: Option<&str>,
) -> Result<String> {
    let base = captures_dir(project_root, host, port).join("js");

    let dir = if let Some(url_p) = url_path {
        let trimmed = url_p.trim_start_matches('/');
        if let Some(parent) = std::path::Path::new(trimmed).parent() {
            if !parent.as_os_str().is_empty() {
                let safe_parent = parent
                    .to_string_lossy()
                    .replace("..", "_")
                    .replace(':', "_");
                base.join(safe_parent)
            } else {
                base
            }
        } else {
            base
        }
    } else {
        base
    };

    tokio::fs::create_dir_all(&dir).await?;

    let safe_name = hashed_filename(filename, content);
    let full_path = dir.join(&safe_name);
    tokio::fs::write(&full_path, content).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();

    tracing::debug!("[file-storage] Saved JS capture: {}", rel);
    Ok(rel)
}

/// Save a captured HTML file. Returns the relative path from project root.
pub async fn save_html_capture(
    project_root: &Path,
    host: &str,
    port: u16,
    url_path: &str,
    content: &[u8],
) -> Result<String> {
    let dir = captures_dir(project_root, host, port).join("html");
    tokio::fs::create_dir_all(&dir).await?;

    let slug = url_path_slug(url_path);
    let safe_name = format!("{}_{}.html", sha256_prefix(content), slug);
    let full_path = dir.join(&safe_name);
    tokio::fs::write(&full_path, content).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();
    Ok(rel)
}

/// Save an HTTP request/response pair as JSON. Returns the relative path.
pub async fn save_http_capture(
    project_root: &Path,
    host: &str,
    port: u16,
    method: &str,
    url_path: &str,
    content: &[u8],
) -> Result<String> {
    let dir = captures_dir(project_root, host, port).join("http");
    tokio::fs::create_dir_all(&dir).await?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let slug = url_path_slug(url_path);
    let filename = format!("{}_{}{}.json", timestamp, method, if slug.is_empty() { "root".to_string() } else { format!("_{}", slug) });
    let full_path = dir.join(&filename);
    tokio::fs::write(&full_path, content).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();
    Ok(rel)
}

/// Save tool output. Returns the relative path.
pub async fn save_tool_output(
    project_root: &Path,
    tool_name: &str,
    target_slug: &str,
    extension: &str,
    content: &[u8],
) -> Result<String> {
    let dir = tool_output_dir(project_root, tool_name);
    tokio::fs::create_dir_all(&dir).await?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let safe_target = sanitize_filename(target_slug);
    let filename = format!("{}_{}.{}", timestamp, safe_target, extension);
    let full_path = dir.join(&filename);
    tokio::fs::write(&full_path, content).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();
    tracing::debug!("[file-storage] Saved tool output: {}", rel);
    Ok(rel)
}

/// Save an evidence file for a finding. Returns the relative path.
pub async fn save_evidence(
    project_root: &Path,
    finding_id: &str,
    filename: &str,
    content: &[u8],
) -> Result<String> {
    let dir = evidence_dir(project_root, finding_id);
    tokio::fs::create_dir_all(&dir).await?;

    let safe_name = sanitize_filename(filename);
    let full_path = dir.join(&safe_name);
    tokio::fs::write(&full_path, content).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();
    Ok(rel)
}

/// Save an AI analysis report. Returns the relative path.
pub async fn save_analysis_report(
    project_root: &Path,
    host: &str,
    analysis_type: &str,
    content: &str,
) -> Result<String> {
    let dir = analysis_dir(project_root, host);
    tokio::fs::create_dir_all(&dir).await?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let filename = format!("{}_{}.md", sanitize_filename(analysis_type), timestamp);
    let full_path = dir.join(&filename);
    tokio::fs::write(&full_path, content.as_bytes()).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();
    Ok(rel)
}

/// Save a script to the appropriate category directory. Returns the relative path.
pub async fn save_script(
    project_root: &Path,
    category: &str,
    filename: &str,
    content: &str,
) -> Result<String> {
    let dir = scripts_dir(project_root, category);
    tokio::fs::create_dir_all(&dir).await?;

    let safe_name = sanitize_filename(filename);
    let full_path = dir.join(&safe_name);
    tokio::fs::write(&full_path, content.as_bytes()).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();
    Ok(rel)
}

/// Save host-level info (DNS, WHOIS, etc.). Returns the relative path.
pub async fn save_host_info(
    project_root: &Path,
    host: &str,
    filename: &str,
    content: &[u8],
) -> Result<String> {
    let dir = host_info_dir(project_root, host);
    tokio::fs::create_dir_all(&dir).await?;

    let safe_name = sanitize_filename(filename);
    let full_path = dir.join(&safe_name);
    tokio::fs::write(&full_path, content).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();
    Ok(rel)
}

// ============================================================================
// File read & listing operations
// ============================================================================

/// Read a file by its relative path from the project root.
pub async fn read_file(project_root: &Path, rel_path: &str) -> Result<Vec<u8>> {
    let full_path = project_root.join(rel_path);
    let content = tokio::fs::read(&full_path)
        .await
        .with_context(|| format!("Failed to read file: {:?}", full_path))?;
    Ok(content)
}

/// List all capture hosts (top-level directories under captures/).
pub async fn list_capture_hosts(project_root: &Path) -> Result<Vec<String>> {
    let dir = project_root.join(GOLISH_DIR).join(CAPTURES_DIR);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut hosts = Vec::new();
    let mut entries = tokio::fs::read_dir(&dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                hosts.push(name.to_string());
            }
        }
    }
    hosts.sort();
    Ok(hosts)
}

/// List all ports for a given host under captures/.
pub async fn list_capture_ports(project_root: &Path, host: &str) -> Result<Vec<u16>> {
    let dir = project_root
        .join(GOLISH_DIR)
        .join(CAPTURES_DIR)
        .join(host_slug(host));
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut ports = Vec::new();
    let mut entries = tokio::fs::read_dir(&dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                if name != HOST_INFO_DIR {
                    if let Ok(port) = name.parse::<u16>() {
                        ports.push(port);
                    }
                }
            }
        }
    }
    ports.sort();
    Ok(ports)
}

/// List files in a specific capture type directory (js, html, css, http, sourcemaps).
pub async fn list_capture_files(
    project_root: &Path,
    host: &str,
    port: u16,
    file_type: &str,
) -> Result<Vec<String>> {
    let dir = captures_dir(project_root, host, port).join(file_type);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    let mut entries = tokio::fs::read_dir(&dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_file() {
            if let Some(name) = entry.file_name().to_str() {
                files.push(name.to_string());
            }
        }
    }
    files.sort();
    Ok(files)
}

/// List all tool output directories.
pub async fn list_tool_outputs(project_root: &Path) -> Result<Vec<String>> {
    let dir = project_root.join(GOLISH_DIR).join(TOOL_OUTPUT_DIR);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut tools = Vec::new();
    let mut entries = tokio::fs::read_dir(&dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                tools.push(name.to_string());
            }
        }
    }
    tools.sort();
    Ok(tools)
}

// ============================================================================
// Cleanup
// ============================================================================

/// Clean the temp directory.
pub async fn clean_temp(project_root: &Path) -> Result<u64> {
    let dir = temp_dir(project_root);
    if !dir.exists() {
        return Ok(0);
    }

    let mut count = 0u64;
    let mut entries = tokio::fs::read_dir(&dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_file() {
            tokio::fs::remove_file(entry.path()).await?;
            count += 1;
        }
    }
    tracing::info!("[file-storage] Cleaned {} temp files", count);
    Ok(count)
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
    fn test_sha256_prefix() {
        let content = b"hello world";
        let prefix = sha256_prefix(content);
        assert_eq!(prefix.len(), 8);
        assert_eq!(prefix, "b94d27b9");
    }

    #[test]
    fn test_hashed_filename() {
        let name = hashed_filename("app.js", b"console.log('hi')");
        assert!(name.ends_with("_app.js"));
        assert_eq!(name.len(), 8 + 1 + "app.js".len());
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("file.txt"), "file.txt");
        assert_eq!(sanitize_filename("path/to/file"), "path_to_file");
        assert_eq!(sanitize_filename("file<>:\""), "file____");
    }

    #[test]
    fn test_url_path_slug() {
        assert_eq!(url_path_slug("/api/v1/users"), "api-v1-users");
        assert_eq!(url_path_slug("/"), "");
        assert_eq!(url_path_slug("/login"), "login");
    }

    #[test]
    fn test_resolve_host_dir_prefers_hostname() {
        let mut map = std::collections::HashMap::new();
        map.insert(
            "93.184.216.34".to_string(),
            vec!["example.com".to_string()],
        );

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
