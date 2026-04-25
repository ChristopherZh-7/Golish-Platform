//! Filesystem-backed wiki page CRUD.
//!
//! Every page lives at `<app-data>/wiki/<path>` on disk and is mirrored
//! into the `wiki_pages` Postgres table on write so the [`super::search`]
//! and [`super::dashboard`] modules can query it via FTS without touching
//! the filesystem.  The DB write is best-effort: if it fails we log and
//! keep going so the user's edit isn't lost.
//!
//! The on-disk layout is documented in [`SCHEMA_MD`], which `wiki_init`
//! seeds into `<wiki>/SCHEMA.md` on first use.

use std::path::Path;

use golish_db::models::NewWikiPage;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::state::AppState;

use super::{is_wiki_file, wiki_base_dir};

/// Top-level wiki categories — directory names under `<wiki>/`.
///
/// `infer_category_from_path` snaps any unrecognised path back to
/// `"uncategorized"` so the dashboard always has a valid bucket.
pub(super) const WIKI_CATEGORIES: &[&str] = &[
    "products",
    "techniques",
    "pocs",
    "experience",
    "analysis",
];

const SCHEMA_MD: &str = r#"# Vulnerability Knowledge Base Schema

This wiki is structured to help the AI agent find exploit methods, PoCs, and research findings during penetration testing.

## Directory Structure

- **products/** — Per-product/component knowledge (e.g., `products/apache-log4j/`)
- **techniques/** — Attack techniques and methodology (e.g., `techniques/ssrf/`)
- **pocs/** — Proof-of-concept code and exploit scripts
- **experience/** — Past engagement notes, lessons learned
- **analysis/** — Deep-dive vulnerability analysis, root cause write-ups

## Page Conventions

Each page should include YAML-style frontmatter:

```
---
title: <descriptive title>
category: products|techniques|pocs|experience|analysis
tags: [tag1, tag2, ...]
cves: [CVE-XXXX-XXXXX, ...]
status: draft|partial|complete|needs-poc|verified
---
```

### Status Values

| Status | Meaning |
|--------|---------|
| `draft` | Just created, basic skeleton with CVE data only |
| `partial` | Some research done, missing exploit details or PoC |
| `complete` | Comprehensive: exploitation method + PoC + detection |
| `needs-poc` | Analysis complete but no working PoC available |
| `verified` | Tested and confirmed in actual engagement |

Followed by markdown content with actionable knowledge the agent can use.
"#;

const INDEX_MD_HEADER: &str = r#"# Vulnerability Knowledge Base

> Auto-generated dashboard. Edited by the system — do not modify manually.

"#;

const LOG_MD_HEADER: &str = "# Knowledge Base Change Log\n\n";

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

async fn build_tree(dir: &Path, prefix: &str) -> std::io::Result<Vec<WikiEntry>> {
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
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries)
}

/// Parse YAML-style frontmatter, returning `(title, category, tags, status)`.
///
/// The parser is intentionally tolerant: missing frontmatter falls back to
/// the first `# Title` heading, missing fields default to safe values
/// (`uncategorized`, empty tag list, `draft`).
pub(super) fn extract_frontmatter(content: &str) -> (String, String, Vec<String>, String) {
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

/// Look up the leading directory segment of a wiki-relative path against
/// [`WIKI_CATEGORIES`].  Used as a fallback when the page's frontmatter
/// doesn't declare a category.
pub(super) fn infer_category_from_path(path: &str) -> String {
    let first_segment = path.split('/').next().unwrap_or("");
    if WIKI_CATEGORIES.contains(&first_segment) {
        first_segment.to_string()
    } else {
        "uncategorized".to_string()
    }
}

#[tauri::command]
pub async fn wiki_init() -> Result<(), String> {
    let base = wiki_base_dir();
    fs::create_dir_all(&base)
        .await
        .map_err(|e| format!("mkdir failed: {e}"))?;

    for cat in WIKI_CATEGORIES {
        fs::create_dir_all(base.join(cat))
            .await
            .map_err(|e| format!("mkdir {cat} failed: {e}"))?;
    }

    let schema_path = base.join("SCHEMA.md");
    if !schema_path.exists() {
        fs::write(&schema_path, SCHEMA_MD)
            .await
            .map_err(|e| format!("write SCHEMA.md failed: {e}"))?;
    }

    let index_path = base.join("index.md");
    if !index_path.exists() {
        fs::write(&index_path, INDEX_MD_HEADER)
            .await
            .map_err(|e| format!("write index.md failed: {e}"))?;
    }

    let log_path = base.join("log.md");
    if !log_path.exists() {
        fs::write(&log_path, LOG_MD_HEADER)
            .await
            .map_err(|e| format!("write log.md failed: {e}"))?;
    }

    Ok(())
}

/// Re-index all wiki pages: scan filesystem, re-extract frontmatter,
/// infer category from path, and upsert into PostgreSQL.
/// Fixes "uncategorized" pages from before the category system.
#[tauri::command]
pub async fn wiki_reindex(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    let base = wiki_base_dir();
    if !base.exists() {
        return Ok(serde_json::json!({ "reindexed": 0 }));
    }

    let mut count = 0u32;
    let mut stack = vec![base.clone()];

    while let Some(dir) = stack.pop() {
        let Ok(mut rd) = fs::read_dir(&dir).await else { continue };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') { continue; }
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !is_wiki_file(&name) { continue; }
            let rel = path.strip_prefix(&base).unwrap_or(&path).to_string_lossy().to_string();
            if rel == "index.md" || rel == "log.md" || rel == "SCHEMA.md" { continue; }

            if let Ok(content) = fs::read_to_string(&path).await {
                let (title, fm_category, tags, status) = extract_frontmatter(&content);
                let category = if fm_category == "uncategorized" {
                    infer_category_from_path(&rel)
                } else {
                    fm_category
                };
                let page = NewWikiPage {
                    path: rel.clone(),
                    title,
                    category,
                    tags,
                    status,
                    content,
                };
                if let Err(e) = golish_db::repo::wiki_kb::upsert_page(pool, &page).await {
                    tracing::warn!("[wiki] reindex failed for {}: {}", rel, e);
                } else {
                    count += 1;
                }
            }
        }
    }

    Ok(serde_json::json!({ "reindexed": count }))
}

#[tauri::command]
pub async fn wiki_list() -> Result<Vec<WikiEntry>, String> {
    let base = wiki_base_dir();
    if !base.exists() {
        fs::create_dir_all(&base)
            .await
            .map_err(|e| format!("cannot create wiki dir: {e}"))?;
        return Ok(Vec::new());
    }
    build_tree(&base, "").await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn wiki_read(path: String) -> Result<String, String> {
    let full = wiki_base_dir().join(&path);
    if !full.exists() {
        return Err(format!("file not found: {path}"));
    }
    fs::read_to_string(&full)
        .await
        .map_err(|e| format!("read failed: {e}"))
}

#[tauri::command]
pub async fn wiki_write(
    state: tauri::State<'_, AppState>,
    path: String,
    content: String,
) -> Result<(), String> {
    let full = wiki_base_dir().join(&path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir failed: {e}"))?;
    }
    fs::write(&full, &content)
        .await
        .map_err(|e| format!("write failed: {e}"))?;

    if is_wiki_file(&path) {
        if let Ok(pool) = state.db_pool_ready().await {
            let (title, fm_category, tags, status) = extract_frontmatter(&content);
            let category = if fm_category == "uncategorized" {
                infer_category_from_path(&path)
            } else {
                fm_category
            };
            let page = NewWikiPage {
                path: path.clone(),
                title,
                category,
                tags,
                status,
                content: content.clone(),
            };
            if let Err(e) = golish_db::repo::wiki_kb::upsert_page(pool, &page).await {
                tracing::warn!("[wiki] DB sync failed for {path}: {e}");
            }
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn wiki_delete(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let full = wiki_base_dir().join(&path);
    if !full.exists() {
        return Ok(());
    }
    let meta = fs::metadata(&full)
        .await
        .map_err(|e| format!("stat failed: {e}"))?;
    if meta.is_dir() {
        fs::remove_dir_all(&full)
            .await
            .map_err(|e| format!("rmdir failed: {e}"))?;
        if let Ok(pool) = state.db_pool_ready().await {
            let prefix = if path.ends_with('/') {
                path.clone()
            } else {
                format!("{}/", path)
            };
            if let Err(e) = golish_db::repo::wiki_kb::delete_pages_by_prefix(pool, &prefix).await {
                tracing::warn!("[wiki] DB delete_pages_by_prefix failed for {path}: {e}");
            }
        }
    } else {
        fs::remove_file(&full)
            .await
            .map_err(|e| format!("rm failed: {e}"))?;
        if let Ok(pool) = state.db_pool_ready().await {
            if let Err(e) = golish_db::repo::wiki_kb::delete_page(pool, &path).await {
                tracing::warn!("[wiki] DB delete_page failed for {path}: {e}");
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn wiki_rename(old_path: String, new_path: String) -> Result<(), String> {
    let base = wiki_base_dir();
    let from = base.join(&old_path);
    let to = base.join(&new_path);
    if !from.exists() {
        return Err(format!("source not found: {old_path}"));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir failed: {e}"))?;
    }
    fs::rename(&from, &to)
        .await
        .map_err(|e| format!("rename failed: {e}"))
}

#[tauri::command]
pub async fn wiki_create_dir(path: String) -> Result<(), String> {
    let full = wiki_base_dir().join(&path);
    fs::create_dir_all(&full)
        .await
        .map_err(|e| format!("mkdir failed: {e}"))
}

#[tauri::command]
pub async fn wiki_create_cve(
    cve_id: String,
    title: String,
    poc_lang: Option<String>,
) -> Result<String, String> {
    let base = wiki_base_dir();
    let folder = base.join(&cve_id);
    if folder.exists() {
        return Err(format!("folder already exists: {cve_id}"));
    }
    fs::create_dir_all(&folder)
        .await
        .map_err(|e| format!("mkdir failed: {e}"))?;

    let readme = format!(
        "# {cve_id}: {title}\n\n\
         ## 概述\n\n\
         <!-- 漏洞描述 -->\n\n\
         ## 影响范围\n\n\
         - 产品/版本:\n\
         - CVSS:\n\
         - 类型:\n\n\
         ## 复现步骤\n\n\
         1. \n\n\
         ## POC\n\n\
         参见 `poc` 文件。\n\n\
         ## 修复建议\n\n\
         <!-- 修复方案 -->\n\n\
         ## 参考\n\n\
         - https://nvd.nist.gov/vuln/detail/{cve_id}\n"
    );
    fs::write(folder.join("README.md"), &readme)
        .await
        .map_err(|e| format!("write README failed: {e}"))?;

    let lang = poc_lang.as_deref().unwrap_or("py");
    let ext = lang;
    let poc_name = format!("poc.{ext}");
    let poc_content = match lang {
        "py" => format!(
            "#!/usr/bin/env python3\n\
             \"\"\"POC for {cve_id}: {title}\"\"\"\n\n\
             import requests\nimport sys\n\n\
             def exploit(target: str):\n\
             \x20   # TODO: implement\n\
             \x20   pass\n\n\
             if __name__ == \"__main__\":\n\
             \x20   if len(sys.argv) < 2:\n\
             \x20       print(f\"Usage: {{sys.argv[0]}} <target>\")\n\
             \x20       sys.exit(1)\n\
             \x20   exploit(sys.argv[1])\n"
        ),
        "go" => format!(
            "package main\n\n\
             // POC for {cve_id}: {title}\n\n\
             import (\n\t\"fmt\"\n\t\"net/http\"\n\t\"os\"\n)\n\n\
             func exploit(target string) error {{\n\
             \t// TODO: implement\n\
             \treturn nil\n\
             }}\n\n\
             func main() {{\n\
             \tif len(os.Args) < 2 {{\n\
             \t\tfmt.Fprintf(os.Stderr, \"Usage: %s <target>\\n\", os.Args[0])\n\
             \t\tos.Exit(1)\n\
             \t}}\n\
             \tif err := exploit(os.Args[1]); err != nil {{\n\
             \t\tfmt.Fprintln(os.Stderr, err)\n\
             \t\tos.Exit(1)\n\
             \t}}\n\
             }}\n"
        ),
        "sh" | "bash" => format!(
            "#!/usr/bin/env bash\n\
             # POC for {cve_id}: {title}\n\n\
             set -euo pipefail\n\n\
             TARGET=\"${{1:?Usage: $0 <target>}}\"\n\n\
             # TODO: implement\n\
             echo \"[*] Target: $TARGET\"\n"
        ),
        _ => format!("// POC for {cve_id}: {title}\n// TODO: implement\n"),
    };
    fs::write(folder.join(&poc_name), &poc_content)
        .await
        .map_err(|e| format!("write POC failed: {e}"))?;

    Ok(format!("{cve_id}/README.md"))
}
