//! Frontmatter parsing, category inference, and on-disk tree walking.
//!
//! These helpers are pure — no I/O for `extract_frontmatter` /
//! `infer_category_from_path`, and `build_tree` is the only async piece. They
//! live together because every command in the wiki module needs at least one
//! of them.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::fs;

use super::super::is_wiki_file;
use super::templates::WIKI_CATEGORIES;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<WikiEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<u64>,
}

/// Recursively walk `dir`, returning a sorted (dirs-first) tree of wiki
/// pages. Hidden files (`.foo`) are skipped.
pub(super) async fn build_tree(dir: &Path, prefix: &str) -> std::io::Result<Vec<WikiEntry>> {
    let mut entries = Vec::new();
    let mut rd = fs::read_dir(dir).await?;
    while let Some(entry) = rd.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let meta = entry.metadata().await?;
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };
        if meta.is_dir() {
            let children = Box::pin(build_tree(&entry.path(), &rel)).await?;
            entries.push(WikiEntry {
                path: rel,
                name,
                is_dir: true,
                children: Some(children),
                size: None,
                modified: None,
            });
        } else if is_wiki_file(&name) {
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            entries.push(WikiEntry {
                path: rel,
                name,
                is_dir: false,
                children: None,
                size: Some(meta.len()),
                modified,
            });
        }
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Ok(entries)
}

/// Parse YAML-style frontmatter, returning `(title, category, tags, status)`.
///
/// The parser is intentionally tolerant: missing frontmatter falls back to
/// the first `# Title` heading, missing fields default to safe values
/// (`uncategorized`, empty tag list, `draft`).
pub(in crate::tools::wiki) fn extract_frontmatter(
    content: &str,
) -> (String, String, Vec<String>, String) {
    if !content.starts_with("---") {
        let title = content
            .lines()
            .find(|l| l.starts_with('#'))
            .map(|l| l.trim_start_matches('#').trim().to_string())
            .unwrap_or_default();
        return (
            title,
            "uncategorized".to_string(),
            vec![],
            "draft".to_string(),
        );
    }
    let rest = &content[3..];
    let end = rest.find("\n---");
    let fm = match end {
        Some(i) => &rest[..i],
        None => {
            return (
                String::new(),
                "uncategorized".to_string(),
                vec![],
                "draft".to_string(),
            )
        }
    };

    let mut title = String::new();
    let mut category = "uncategorized".to_string();
    let mut tags = vec![];
    let mut status = "draft".to_string();

    for line in fm.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("title:") {
            title = v.trim().trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("category:") {
            category = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("status:") {
            status = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("tags:") {
            let raw = v.trim().trim_start_matches('[').trim_end_matches(']');
            tags = raw
                .split(',')
                .map(|t| t.trim().trim_matches('"').trim_matches('\'').to_string())
                .filter(|t| !t.is_empty())
                .collect();
        }
    }

    if title.is_empty() {
        title = content
            .lines()
            .skip_while(|l| l.starts_with("---") || l.trim().is_empty() || l.contains(':'))
            .find(|l| l.starts_with('#'))
            .map(|l| l.trim_start_matches('#').trim().to_string())
            .unwrap_or_default();
    }

    (title, category, tags, status)
}

/// Look up the leading directory segment of a wiki-relative path against
/// [`WIKI_CATEGORIES`]. Used as a fallback when the page's frontmatter
/// doesn't declare a category.
pub(in crate::tools::wiki) fn infer_category_from_path(path: &str) -> String {
    let first_segment = path.split('/').next().unwrap_or("");
    if WIKI_CATEGORIES.contains(&first_segment) {
        first_segment.to_string()
    } else {
        "uncategorized".to_string()
    }
}
