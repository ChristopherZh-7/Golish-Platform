use crate::state::AppState;
use golish_db::models::NewWikiPage;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;

const WIKI_EXTENSIONS: &[&str] = &[
    ".md", ".txt", ".py", ".sh", ".bash", ".zsh", ".go", ".rs", ".rb", ".pl",
    ".js", ".ts", ".jsx", ".tsx", ".c", ".cpp", ".h", ".hpp", ".java", ".cs",
    ".swift", ".kt", ".lua", ".r", ".ps1", ".bat", ".cmd", ".php", ".html",
    ".css", ".xml", ".json", ".yaml", ".yml", ".toml", ".ini", ".conf", ".cfg",
    ".sql", ".graphql", ".proto", ".dockerfile", ".nse",
];

fn is_wiki_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    WIKI_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
        || lower == "dockerfile"
        || lower == "makefile"
        || lower == "rakefile"
}

fn is_text_searchable(name: &str) -> bool {
    is_wiki_file(name)
}

fn wiki_base_dir() -> PathBuf {
    golish_core::paths::app_data_base()
        .expect("cannot resolve home directory")
        .join("wiki")
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiSearchResult {
    pub path: String,
    pub name: String,
    pub line: usize,
    pub content: String,
}

async fn build_tree(dir: &std::path::Path, prefix: &str) -> std::io::Result<Vec<WikiEntry>> {
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

const WIKI_CATEGORIES: &[&str] = &[
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

/// Returns (title, category, tags, status)
fn extract_frontmatter(content: &str) -> (String, String, Vec<String>, String) {
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

fn infer_category_from_path(path: &str) -> String {
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
pub async fn wiki_search(query: String) -> Result<Vec<WikiSearchResult>, String> {
    let base = wiki_base_dir();
    if !base.exists() {
        return Ok(Vec::new());
    }
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();
    let mut stack = vec![base.clone()];

    while let Some(dir) = stack.pop() {
        let Ok(mut rd) = fs::read_dir(&dir).await else {
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
            if !is_text_searchable(&name) {
                continue;
            }
            let rel = path
                .strip_prefix(&base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            if name.to_lowercase().contains(&query_lower) {
                results.push(WikiSearchResult {
                    path: rel.clone(),
                    name: name.clone(),
                    line: 0,
                    content: name.clone(),
                });
            }

            if let Ok(content) = fs::read_to_string(&path).await {
                for (i, line) in content.lines().enumerate() {
                    if line.to_lowercase().contains(&query_lower) {
                        results.push(WikiSearchResult {
                            path: rel.clone(),
                            name: name.clone(),
                            line: i + 1,
                            content: line.chars().take(200).collect(),
                        });
                        if results.len() >= 100 {
                            return Ok(results);
                        }
                    }
                }
            }
        }
    }

    Ok(results)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiSearchResultDb {
    pub path: String,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub snippet: String,
    pub word_count: i32,
}

#[tauri::command]
pub async fn wiki_search_db(
    state: tauri::State<'_, AppState>,
    query: String,
    category: Option<String>,
    tag: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<WikiSearchResultDb>, String> {
    let pool = state.db_pool_ready().await?;
    let max = limit.unwrap_or(20);

    let pages = if let Some(cat) = category {
        golish_db::repo::wiki_kb::search_by_category(pool, &cat, max)
            .await
            .map_err(|e| e.to_string())?
    } else if let Some(t) = tag {
        golish_db::repo::wiki_kb::search_by_tag(pool, &t, max)
            .await
            .map_err(|e| e.to_string())?
    } else {
        golish_db::repo::wiki_kb::search_fts(pool, &query, max)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(pages
        .into_iter()
        .map(|p| {
            let snippet = p.content.chars().take(300).collect();
            WikiSearchResultDb {
                path: p.path,
                title: p.title,
                category: p.category,
                tags: p.tags,
                snippet,
                word_count: p.word_count,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn wiki_stats(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    let count = golish_db::repo::wiki_kb::count_pages(pool)
        .await
        .map_err(|e| e.to_string())?;
    let recent = golish_db::repo::wiki_kb::list_recent(pool, 5)
        .await
        .map_err(|e| e.to_string())?;
    let recent_paths: Vec<String> = recent.into_iter().map(|p| p.path).collect();
    Ok(serde_json::json!({
        "total_pages": count,
        "recent_updated": recent_paths,
    }))
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

// ============================================================================
// KB Research Log commands (separate from main conversation system)
// ============================================================================

#[tauri::command]
pub async fn kb_research_load(
    state: tauri::State<'_, AppState>,
    cve_id: String,
) -> Result<Option<serde_json::Value>, String> {
    let pool = state.db_pool_ready().await?;
    let log = golish_db::repo::kb_research::get_log(pool, &cve_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(log.map(|l| {
        serde_json::json!({
            "cve_id": l.cve_id,
            "session_id": l.session_id,
            "turns": l.turns,
            "status": l.status,
            "updated_at": l.updated_at.to_rfc3339(),
        })
    }))
}

#[tauri::command]
pub async fn kb_research_save_turn(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    session_id: String,
    turn: serde_json::Value,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;

    let existing = golish_db::repo::kb_research::get_log(pool, &cve_id)
        .await
        .map_err(|e| e.to_string())?;

    if existing.is_some() {
        golish_db::repo::kb_research::append_turn(pool, &cve_id, &turn)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        let turns = serde_json::json!([turn]);
        golish_db::repo::kb_research::upsert_log(pool, &cve_id, &session_id, &turns, "in_progress")
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn kb_research_set_status(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    status: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    golish_db::repo::kb_research::set_status(pool, &cve_id, &status)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn kb_research_clear(
    state: tauri::State<'_, AppState>,
    cve_id: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    golish_db::repo::kb_research::delete_log(pool, &cve_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ============================================================================
// VulnLink CRUD — replaces localStorage with PostgreSQL
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnLinkFull {
    pub wiki_paths: Vec<String>,
    pub poc_templates: Vec<VulnPocEntry>,
    pub scan_history: Vec<VulnScanEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnPocEntry {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub poc_type: String,
    pub language: String,
    pub content: String,
    pub source: String,
    pub source_url: String,
    pub severity: String,
    pub verified: bool,
    pub description: String,
    pub tags: Vec<String>,
    pub created: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnScanEntry {
    pub id: String,
    pub target: String,
    pub date: i64,
    pub result: String,
    pub details: Option<String>,
}

#[tauri::command]
pub async fn vuln_link_get_all(
    state: tauri::State<'_, AppState>,
) -> Result<std::collections::HashMap<String, VulnLinkFull>, String> {
    let pool = state.db_pool_ready().await?;
    let mut result: std::collections::HashMap<String, VulnLinkFull> = std::collections::HashMap::new();

    // Load all wiki links
    let all_wiki: Vec<golish_db::models::VulnKbLink> =
        sqlx::query_as("SELECT * FROM vuln_kb_links ORDER BY created_at DESC")
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
    for l in all_wiki {
        result
            .entry(l.cve_id.clone())
            .or_insert_with(|| VulnLinkFull {
                wiki_paths: vec![],
                poc_templates: vec![],
                scan_history: vec![],
            })
            .wiki_paths
            .push(l.wiki_path);
    }

    // Load all PoCs
    let all_pocs: Vec<golish_db::models::VulnKbPoc> =
        sqlx::query_as("SELECT * FROM vuln_kb_pocs ORDER BY created_at DESC")
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
    for p in all_pocs {
        result
            .entry(p.cve_id.clone())
            .or_insert_with(|| VulnLinkFull {
                wiki_paths: vec![],
                poc_templates: vec![],
                scan_history: vec![],
            })
            .poc_templates
            .push(poc_to_entry(p));
    }

    // Load all scans
    let all_scans: Vec<golish_db::models::VulnScanHistory> =
        sqlx::query_as("SELECT * FROM vuln_scan_history ORDER BY scanned_at DESC")
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
    for s in all_scans {
        result
            .entry(s.cve_id.clone())
            .or_insert_with(|| VulnLinkFull {
                wiki_paths: vec![],
                poc_templates: vec![],
                scan_history: vec![],
            })
            .scan_history
            .push(VulnScanEntry {
                id: s.id.to_string(),
                target: s.target,
                date: s.scanned_at.timestamp_millis(),
                result: s.result,
                details: s.details,
            });
    }

    Ok(result)
}

#[tauri::command]
pub async fn vuln_link_get(
    state: tauri::State<'_, AppState>,
    cve_id: String,
) -> Result<VulnLinkFull, String> {
    let pool = state.db_pool_ready().await?;

    let links = golish_db::repo::wiki_kb::get_links_for_cve(pool, &cve_id)
        .await
        .map_err(|e| e.to_string())?;
    let wiki_paths: Vec<String> = links.into_iter().map(|l| l.wiki_path).collect();

    let pocs = golish_db::repo::wiki_kb::get_pocs_for_cve(pool, &cve_id)
        .await
        .map_err(|e| e.to_string())?;
    let poc_templates: Vec<VulnPocEntry> = pocs.into_iter().map(poc_to_entry).collect();

    let scans = golish_db::repo::vuln_scan::get_scans_for_cve(pool, &cve_id)
        .await
        .map_err(|e| e.to_string())?;
    let scan_history: Vec<VulnScanEntry> = scans
        .into_iter()
        .map(|s| VulnScanEntry {
            id: s.id.to_string(),
            target: s.target,
            date: s.scanned_at.timestamp_millis(),
            result: s.result,
            details: s.details,
        })
        .collect();

    Ok(VulnLinkFull {
        wiki_paths,
        poc_templates,
        scan_history,
    })
}

#[tauri::command]
pub async fn vuln_link_add_wiki(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    wiki_path: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    golish_db::repo::wiki_kb::link_cve_to_wiki(pool, &cve_id, &wiki_path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn vuln_link_remove_wiki(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    wiki_path: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    sqlx::query("DELETE FROM vuln_kb_links WHERE cve_id = $1 AND wiki_path = $2")
        .bind(&cve_id)
        .bind(&wiki_path)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn vuln_link_add_poc(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    name: String,
    poc_type: String,
    language: String,
    content: String,
) -> Result<VulnPocEntry, String> {
    let pool = state.db_pool_ready().await?;
    let poc = golish_db::repo::wiki_kb::upsert_poc(pool, &cve_id, &name, &poc_type, &language, &content)
        .await
        .map_err(|e| e.to_string())?;
    Ok(poc_to_entry(poc))
}

#[tauri::command]
pub async fn vuln_link_update_poc(
    state: tauri::State<'_, AppState>,
    poc_id: String,
    name: String,
    content: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    let id: uuid::Uuid = poc_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    sqlx::query("UPDATE vuln_kb_pocs SET name = $2, content = $3, updated_at = NOW() WHERE id = $1")
        .bind(id)
        .bind(&name)
        .bind(&content)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn vuln_link_remove_poc(
    state: tauri::State<'_, AppState>,
    poc_id: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    let id: uuid::Uuid = poc_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    golish_db::repo::wiki_kb::delete_poc(pool, id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn vuln_link_add_scan(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    target: String,
    result: String,
    details: Option<String>,
) -> Result<VulnScanEntry, String> {
    let pool = state.db_pool_ready().await?;
    let scan = golish_db::repo::vuln_scan::add_scan(
        pool, &cve_id, &target, &result, details.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(VulnScanEntry {
        id: scan.id.to_string(),
        target: scan.target,
        date: scan.scanned_at.timestamp_millis(),
        result: scan.result,
        details: scan.details,
    })
}

#[tauri::command]
pub async fn vuln_link_remove_scan(
    state: tauri::State<'_, AppState>,
    scan_id: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    let id: uuid::Uuid = scan_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    golish_db::repo::vuln_scan::delete_scan(pool, id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ============================================================================
// PoC-first workflow commands
// ============================================================================

fn poc_to_entry(p: golish_db::models::VulnKbPoc) -> VulnPocEntry {
    VulnPocEntry {
        id: p.id.to_string(),
        name: p.name,
        poc_type: p.poc_type,
        language: p.language,
        content: p.content,
        source: p.source,
        source_url: p.source_url,
        severity: p.severity,
        verified: p.verified,
        description: p.description,
        tags: p.tags,
        created: p.created_at.timestamp_millis(),
    }
}

#[tauri::command]
pub async fn vuln_link_add_poc_full(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    name: String,
    poc_type: String,
    language: String,
    content: String,
    source: String,
    source_url: String,
    severity: String,
    description: String,
    tags: Vec<String>,
) -> Result<VulnPocEntry, String> {
    let pool = state.db_pool_ready().await?;
    let poc = golish_db::repo::wiki_kb::upsert_poc_full(
        pool, &cve_id, &name, &poc_type, &language, &content,
        &source, &source_url, &severity, &description, &tags,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(poc_to_entry(poc))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CvePocSummaryResponse {
    pub cve_id: String,
    pub poc_count: i64,
    pub max_severity: String,
    pub any_verified: bool,
    pub has_research: bool,
    pub has_wiki: bool,
}

#[tauri::command]
pub async fn vuln_poc_list_cves(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<CvePocSummaryResponse>, String> {
    let pool = state.db_pool_ready().await?;
    let rows = golish_db::repo::wiki_kb::list_cves_with_pocs(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| CvePocSummaryResponse {
            cve_id: r.cve_id,
            poc_count: r.poc_count,
            max_severity: r.max_severity.unwrap_or_else(|| "unknown".to_string()),
            any_verified: r.any_verified.unwrap_or(false),
            has_research: r.has_research.unwrap_or(false),
            has_wiki: r.has_wiki.unwrap_or(false),
        })
        .collect())
}

#[tauri::command]
pub async fn vuln_poc_list_unresearched(
    state: tauri::State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<CvePocSummaryResponse>, String> {
    let pool = state.db_pool_ready().await?;
    let rows = golish_db::repo::wiki_kb::list_unresearched_cves(pool, limit.unwrap_or(20))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| CvePocSummaryResponse {
            cve_id: r.cve_id,
            poc_count: r.poc_count,
            max_severity: r.max_severity.unwrap_or_else(|| "unknown".to_string()),
            any_verified: r.any_verified.unwrap_or(false),
            has_research: false,
            has_wiki: r.has_wiki.unwrap_or(false),
        })
        .collect())
}

#[tauri::command]
pub async fn vuln_poc_stats(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    golish_db::repo::wiki_kb::poc_stats(pool)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn vuln_poc_set_verified(
    state: tauri::State<'_, AppState>,
    poc_id: String,
    verified: bool,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    let id: uuid::Uuid = poc_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    sqlx::query("UPDATE vuln_kb_pocs SET verified = $2, updated_at = NOW() WHERE id = $1")
        .bind(id)
        .bind(verified)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ============================================================================
// Karpathy-style wiki dashboard commands
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiPageInfo {
    pub path: String,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub status: String,
    pub word_count: i32,
    pub updated_at: String,
}

fn summary_to_info(s: golish_db::models::WikiPageSummary) -> WikiPageInfo {
    WikiPageInfo {
        path: s.path,
        title: s.title,
        category: s.category,
        tags: s.tags,
        status: s.status,
        word_count: s.word_count,
        updated_at: s.updated_at.to_rfc3339(),
    }
}

#[tauri::command]
pub async fn wiki_pages_grouped(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<WikiPageInfo>, String> {
    let pool = state.db_pool_ready().await?;
    let pages = golish_db::repo::wiki_kb::list_pages_grouped_by_category(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(pages.into_iter().map(summary_to_info).collect())
}

#[tauri::command]
pub async fn wiki_pages_for_paths(
    state: tauri::State<'_, AppState>,
    paths: Vec<String>,
) -> Result<Vec<WikiPageInfo>, String> {
    let pool = state.db_pool_ready().await?;
    let pages = golish_db::repo::wiki_kb::list_pages_for_paths(pool, &paths)
        .await
        .map_err(|e| e.to_string())?;
    Ok(pages.into_iter().map(summary_to_info).collect())
}

#[tauri::command]
pub async fn wiki_suggest_for_cve(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    limit: Option<i64>,
) -> Result<Vec<WikiPageInfo>, String> {
    let pool = state.db_pool_ready().await?;
    let pages = golish_db::repo::wiki_kb::suggest_pages_for_cve(pool, &cve_id, limit.unwrap_or(10))
        .await
        .map_err(|e| e.to_string())?;
    Ok(pages.into_iter().map(summary_to_info).collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiChangelogEntry {
    pub id: i64,
    pub page_path: String,
    pub action: String,
    pub title: String,
    pub category: String,
    pub actor: String,
    pub summary: String,
    pub created_at: String,
}

#[tauri::command]
pub async fn wiki_changelog_list(
    state: tauri::State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<WikiChangelogEntry>, String> {
    let pool = state.db_pool_ready().await?;
    let entries = golish_db::repo::wiki_kb::list_changelog(pool, limit.unwrap_or(50))
        .await
        .map_err(|e| e.to_string())?;
    Ok(entries
        .into_iter()
        .map(|e| WikiChangelogEntry {
            id: e.id,
            page_path: e.page_path,
            action: e.action,
            title: e.title,
            category: e.category,
            actor: e.actor,
            summary: e.summary,
            created_at: e.created_at.to_rfc3339(),
        })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiBacklink {
    pub source_path: String,
    pub context: String,
}

#[tauri::command]
pub async fn wiki_backlinks(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<Vec<WikiBacklink>, String> {
    let pool = state.db_pool_ready().await?;
    let refs = golish_db::repo::wiki_kb::get_backlinks(pool, &path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(refs
        .into_iter()
        .map(|r| WikiBacklink {
            source_path: r.source_path,
            context: r.context,
        })
        .collect())
}

#[tauri::command]
pub async fn wiki_stats_full(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    golish_db::repo::wiki_kb::wiki_stats_full(pool)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn wiki_orphan_pages(
    state: tauri::State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<WikiPageInfo>, String> {
    let pool = state.db_pool_ready().await?;
    let pages = golish_db::repo::wiki_kb::list_orphan_pages(pool, limit.unwrap_or(20))
        .await
        .map_err(|e| e.to_string())?;
    Ok(pages.into_iter().map(summary_to_info).collect())
}
