//! File read, listing, and cleanup operations.

use anyhow::{Context, Result};
use std::path::Path;

use super::{host_slug, CAPTURES_DIR, GOLISH_DIR, HOST_INFO_DIR, TOOL_OUTPUT_DIR};

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
    let dir = super::captures_dir(project_root, host, port).join(file_type);
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

/// Clean the temp directory.
pub async fn clean_temp(project_root: &Path) -> Result<u64> {
    let dir = super::temp_dir(project_root);
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
