//! Filesystem-level helpers for the markdown wiki backing the KB.
//!
//! These utilities are shared across the search/read/save/query verb
//! modules — they own the on-disk schema (`<app_data>/wiki/`), parse
//! frontmatter, infer categories, extract cross-references, and rebuild the
//! auto-generated `index.md`.

use serde_json::json;

use crate::tool_executors::common::ToolResult;

pub(super) const WIKI_CATEGORIES: &[&str] =
    &["products", "techniques", "pocs", "experience", "analysis"];

/// Resolve `~/.golish/wiki` (or platform equivalent).
pub(super) fn wiki_base_dir() -> std::path::PathBuf {
    golish_core::paths::app_data_base()
        .expect("cannot resolve home directory")
        .join("wiki")
}

/// Parse the `---` YAML-ish frontmatter block. Returns
/// `(title, category, tags, status)`. Falls back to "uncategorized" /
/// "draft" / first markdown heading when fields are missing or malformed.
pub(super) fn extract_wiki_frontmatter(content: &str) -> (String, String, Vec<String>, String) {
    if !content.starts_with("---") {
        let title = content
            .lines()
            .find(|l| l.starts_with('#'))
            .map(|l| l.trim_start_matches('#').trim().to_string())
            .unwrap_or_default();
        return (title, "uncategorized".to_string(), vec![], "draft".to_string());
    }
    let rest = &content[3..];
    let end = rest.find("\n---");
    let fm = match end {
        Some(i) => &rest[..i],
        None => return (String::new(), "uncategorized".to_string(), vec![], "draft".to_string()),
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

/// Infer a category from the first path segment, falling back to
/// "uncategorized" when it isn't one of the well-known buckets.
pub(super) fn infer_wiki_category(path: &str) -> String {
    let first_segment = path.split('/').next().unwrap_or("");
    if WIKI_CATEGORIES.contains(&first_segment) {
        first_segment.to_string()
    } else {
        "uncategorized".to_string()
    }
}

/// Extract markdown links from wiki content and resolve them to wiki paths.
/// Returns `Vec<(target_path, context_snippet)>`.
pub(super) fn extract_wiki_links(content: &str, source_path: &str) -> Vec<(String, String)> {
    let re = regex::Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap();
    let source_dir = source_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");

    let mut refs = Vec::new();
    for cap in re.captures_iter(content) {
        let link_text = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let href = cap.get(2).map(|m| m.as_str()).unwrap_or("");

        if href.starts_with("http://") || href.starts_with("https://") || href.starts_with('#') {
            continue;
        }

        let resolved = if href.starts_with('/') {
            href.trim_start_matches('/').to_string()
        } else if source_dir.is_empty() {
            href.to_string()
        } else {
            format!("{}/{}", source_dir, href)
        };

        // Normalise ../ and ./ segments.
        let parts: Vec<&str> = resolved.split('/').collect();
        let mut normalized: Vec<&str> = Vec::new();
        for p in parts {
            match p {
                "." | "" => {}
                ".." => {
                    normalized.pop();
                }
                _ => normalized.push(p),
            }
        }
        let target = normalized.join("/");
        if !target.is_empty() && target != source_path {
            refs.push((target, link_text.to_string()));
        }
    }
    refs.dedup_by(|a, b| a.0 == b.0);
    refs
}

/// Rebuild `index.md` from every page on disk. Best-effort: failures are
/// logged via `tracing::warn` and the rest of the write proceeds.
pub(super) async fn update_wiki_index(base: &std::path::Path) {
    let index_path = base.join("index.md");

    let mut entries: Vec<(String, String, String, String)> = Vec::new();
    let mut stack = vec![base.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(mut rd) = tokio::fs::read_dir(&dir).await else { continue };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if !name.ends_with(".md") {
                continue;
            }
            let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().to_string();
            if rel == "index.md" || rel == "log.md" || rel == "SCHEMA.md" {
                continue;
            }

            if let Ok(content) = tokio::fs::read_to_string(&p).await {
                let (title, category, _tags, status) = extract_wiki_frontmatter(&content);
                let display_title = if title.is_empty() {
                    name.trim_end_matches(".md").to_string()
                } else {
                    title
                };
                entries.push((category, rel, display_title, status));
            }
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.2.cmp(&b.2)));

    let mut index = String::from(
        "# Vulnerability Knowledge Base\n\n> Auto-generated index. Updated on every write.\n\n",
    );

    let mut current_cat = String::new();
    for (cat, path, title, status) in &entries {
        if *cat != current_cat {
            current_cat = cat.clone();
            let icon = match cat.as_str() {
                "products" => "📦",
                "techniques" => "⚔️",
                "pocs" => "🔧",
                "experience" => "📝",
                "analysis" => "🔬",
                _ => "📄",
            };
            index.push_str(&format!("\n## {icon} {}\n\n", cat));
        }
        index.push_str(&format!("- [{title}]({path}) `{status}`\n"));
    }

    index.push_str(&format!(
        "\n---\n*{} pages total. Last updated: {}*\n",
        entries.len(),
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));

    if let Err(e) = tokio::fs::write(&index_path, &index).await {
        tracing::warn!("[kb] Failed to update index.md: {}", e);
    }
}

/// Filesystem-only fallback search used when the database is unavailable.
pub(super) async fn kb_search_filesystem_fallback(query: &str, limit: usize) -> ToolResult {
    let base = wiki_base_dir();
    if !base.exists() {
        return (
            json!({ "results": [], "count": 0, "query": query, "note": "KB not initialized" }),
            true,
        );
    }

    let query_lower = query.to_lowercase();
    let mut results = Vec::new();
    let mut stack = vec![base.clone()];

    while let Some(dir) = stack.pop() {
        let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !name.ends_with(".md") {
                continue;
            }
            let rel = path
                .strip_prefix(&base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                if content.to_lowercase().contains(&query_lower) {
                    let snippet: String = content.chars().take(500).collect();
                    results.push(json!({
                        "path": rel,
                        "title": name,
                        "snippet": snippet,
                    }));
                    if results.len() >= limit {
                        break;
                    }
                }
            }
        }
        if results.len() >= limit {
            break;
        }
    }

    let count = results.len();
    (
        json!({ "results": results, "count": count, "query": query, "source": "filesystem" }),
        true,
    )
}
