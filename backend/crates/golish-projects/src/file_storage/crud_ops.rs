//! File save operations for captured JS, HTML, HTTP, tool output,
//! evidence, analysis reports, scripts, and host info.

use anyhow::Result;
use sha2::{Digest, Sha256};
use std::path::Path;

use super::{
    analysis_dir, captures_dir, evidence_dir, host_info_dir, scripts_dir, tool_output_dir,
};

fn sha256_prefix(content: &[u8]) -> String {
    let hash = Sha256::digest(content);
    let hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
    hex[..8].to_string()
}

fn hashed_filename(original_name: &str, content: &[u8]) -> String {
    let prefix = sha256_prefix(content);
    format!("{}_{}", prefix, sanitize_filename(original_name))
}

pub(super) fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c => c,
        })
        .collect()
}

fn url_path_slug(url_path: &str) -> String {
    url_path
        .trim_start_matches('/')
        .replace('/', "-")
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c => c,
        })
        .collect::<String>()
        .chars()
        .take(100)
        .collect()
}

/// Save a captured JS file. Returns the relative path from project root.
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
