use serde_json::json;
use super::common::{error_result, extract_string_param, ToolResult};

/// Execute a vulnerability knowledge base tool.
///
/// Handles: search_knowledge_base, write_knowledge, read_knowledge, ingest_cve.
/// Uses the wiki filesystem (markdown) as primary storage and PostgreSQL for full-text search.
pub async fn execute_knowledge_base_tool(
    tool_name: &str,
    args: &serde_json::Value,
    db_tracker: Option<&crate::db_tracking::DbTracker>,
) -> Option<ToolResult> {
    let is_kb_tool = matches!(
        tool_name,
        "search_knowledge_base"
            | "write_knowledge"
            | "read_knowledge"
            | "ingest_cve"
            | "save_poc"
            | "list_cves_with_pocs"
            | "list_unresearched_cves"
            | "poc_stats"
    );
    if !is_kb_tool {
        return None;
    }

    match tool_name {
        "search_knowledge_base" => {
            let query = match extract_string_param(args, &["query", "q", "search"]) {
                Some(q) if !q.is_empty() => q,
                _ => return Some(error_result(
                    "search_knowledge_base requires a non-empty 'query' parameter"
                )),
            };
            let category = args.get("category").and_then(|v| v.as_str()).map(String::from);
            let tag = args.get("tag").and_then(|v| v.as_str()).map(String::from);
            let limit = args
                .get("limit")
                .and_then(|v| v.as_i64())
                .unwrap_or(10)
                .min(50);

            let tracker = match db_tracker {
                Some(t) => t,
                None => {
                    return Some(kb_search_filesystem_fallback(&query, limit as usize).await);
                }
            };

            let pages = if let Some(cat) = category {
                golish_db::repo::wiki_kb::search_by_category(tracker.pool(), &cat, limit).await
            } else if let Some(t) = tag {
                golish_db::repo::wiki_kb::search_by_tag(tracker.pool(), &t, limit).await
            } else {
                golish_db::repo::wiki_kb::search_fts(tracker.pool(), &query, limit).await
            };

            match pages {
                Ok(results) => {
                    let items: Vec<serde_json::Value> = results
                        .iter()
                        .map(|p| {
                            json!({
                                "path": p.path,
                                "title": p.title,
                                "category": p.category,
                                "tags": p.tags,
                                "status": p.status,
                                "snippet": p.content.chars().take(500).collect::<String>(),
                                "word_count": p.word_count,
                            })
                        })
                        .collect();
                    let count = items.len();
                    Some((json!({ "results": items, "count": count, "query": query }), true))
                }
                Err(e) => Some(error_result(format!("KB search failed: {}", e))),
            }
        }

        "write_knowledge" => {
            let path = match extract_string_param(args, &["path"]) {
                Some(p) if !p.is_empty() => p,
                _ => return Some(error_result("write_knowledge requires a 'path' parameter")),
            };
            let content = match args.get("content").and_then(|v| v.as_str()) {
                Some(c) => c.to_string(),
                None => return Some(error_result("write_knowledge requires a 'content' parameter")),
            };

            let base = wiki_base_dir();
            let full = base.join(&path);
            let is_new = !full.exists();
            if let Some(parent) = full.parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return Some(error_result(format!("mkdir failed: {}", e)));
                }
            }
            if let Err(e) = tokio::fs::write(&full, &content).await {
                return Some(error_result(format!("write failed: {}", e)));
            }

            let cve_id_opt = extract_string_param(args, &["cve_id"]);
            let (title, fm_category, tags, status) = extract_wiki_frontmatter(&content);
            let category = if fm_category == "uncategorized" {
                infer_wiki_category(&path)
            } else {
                fm_category
            };

            // --- Cross-reference extraction ---
            let cross_refs = extract_wiki_links(&content, &path);
            let refs_saved = cross_refs.len();

            if let Some(tracker) = db_tracker {
                let page = golish_db::models::NewWikiPage {
                    path: path.clone(),
                    title: title.clone(),
                    category: category.clone(),
                    tags: tags.clone(),
                    status: status.clone(),
                    content: content.clone(),
                };
                if let Err(e) = golish_db::repo::wiki_kb::upsert_page(tracker.pool(), &page).await
                {
                    tracing::warn!("[kb] DB sync failed for {}: {}", path, e);
                }

                if let Some(ref cve) = cve_id_opt {
                    if let Err(e) = golish_db::repo::wiki_kb::link_cve_to_wiki(tracker.pool(), cve, &path).await {
                        tracing::warn!("[kb] CVE link failed for {} -> {}: {}", cve, path, e);
                    }
                }

                // Persist cross-references (replace all from this page)
                let _ = golish_db::repo::wiki_kb::delete_refs_from(tracker.pool(), &path).await;
                for (target, ctx) in &cross_refs {
                    let _ = golish_db::repo::wiki_kb::upsert_page_ref(
                        tracker.pool(), &path, target, ctx,
                    ).await;
                }

                // Append changelog entry
                let action = if is_new { "create" } else { "update" };
                let summary = if is_new {
                    format!("Created page: {}", title)
                } else {
                    format!("Updated page: {}", title)
                };
                let log_entry = golish_db::models::NewWikiChangelog {
                    page_path: path.clone(),
                    action: action.to_string(),
                    title: title.clone(),
                    category: category.clone(),
                    actor: "agent".to_string(),
                    summary,
                };
                if let Err(e) = golish_db::repo::wiki_kb::add_changelog(tracker.pool(), &log_entry).await {
                    tracing::warn!("[kb] changelog write failed: {}", e);
                }
            }

            // --- Append to log.md ---
            let log_path = base.join("log.md");
            let action_label = if is_new { "create" } else { "update" };
            let now = chrono::Utc::now().format("%Y-%m-%d %H:%M");
            let log_line = format!(
                "\n## [{now}] {action_label} | {title}\n\n- Path: `{path}`\n- Category: {category}\n- Tags: {tags}\n- Status: {status}\n{cve_line}\n",
                title = title,
                path = path,
                category = category,
                tags = if tags.is_empty() { "none".to_string() } else { tags.join(", ") },
                status = status,
                cve_line = cve_id_opt.as_ref().map(|c| format!("- CVE: {c}\n")).unwrap_or_default(),
            );
            if let Err(e) = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .await
            {
                tracing::warn!("[kb] Failed to open log.md for append: {}", e);
            } else {
                use tokio::io::AsyncWriteExt;
                if let Ok(mut f) = tokio::fs::OpenOptions::new().append(true).open(&log_path).await {
                    let _ = f.write_all(log_line.as_bytes()).await;
                }
            }

            // --- Update index.md ---
            update_wiki_index(&base).await;

            let mut msg = format!("Knowledge page written to {}", path);
            if let Some(ref cve) = cve_id_opt {
                msg.push_str(&format!(" (linked to {})", cve));
            }
            if refs_saved > 0 {
                msg.push_str(&format!(", {} cross-references indexed", refs_saved));
            }

            Some((
                json!({
                    "success": true,
                    "path": path,
                    "linked_cve": cve_id_opt,
                    "cross_references": refs_saved,
                    "message": msg,
                }),
                true,
            ))
        }

        "read_knowledge" => {
            let path = match extract_string_param(args, &["path"]) {
                Some(p) if !p.is_empty() => p,
                _ => return Some(error_result("read_knowledge requires a 'path' parameter")),
            };

            let full = wiki_base_dir().join(&path);
            match tokio::fs::read_to_string(&full).await {
                Ok(content) => Some((
                    json!({
                        "path": path,
                        "content": content,
                    }),
                    true,
                )),
                Err(e) => Some(error_result(format!("File not found or unreadable: {} ({})", path, e))),
            }
        }

        "ingest_cve" => {
            let cve_id = match extract_string_param(args, &["cve_id", "cve"]) {
                Some(c) if !c.is_empty() => c,
                _ => return Some(error_result("ingest_cve requires a 'cve_id' parameter")),
            };
            let product = match extract_string_param(args, &["product", "component"]) {
                Some(p) if !p.is_empty() => p,
                _ => return Some(error_result("ingest_cve requires a 'product' parameter")),
            };
            let additional = extract_string_param(args, &["additional_context", "notes"]);

            let mut cve_info = String::new();
            if let Some(tracker) = db_tracker {
                match golish_db::repo::vuln_intel::search_entries(
                    tracker.pool(),
                    &cve_id,
                    1,
                )
                .await
                {
                    Ok(entries) if !entries.is_empty() => {
                        let e = &entries[0];
                        cve_info = format!(
                            "Title: {}\nSeverity: {}\nCVSS: {}\nPublished: {}\nDescription: {}\nAffected: {}\nReferences: {}",
                            e.title,
                            e.sev,
                            e.cvss_score.map_or("N/A".to_string(), |s| s.to_string()),
                            e.published,
                            e.description,
                            e.affected_products,
                            e.refs,
                        );
                    }
                    _ => {}
                }
            }

            let slug = product.to_lowercase().replace(' ', "-");
            let path = format!("products/{}/{}.md", slug, cve_id);
            let base = wiki_base_dir();
            let full = base.join(&path);

            if full.exists() {
                let existing = tokio::fs::read_to_string(&full).await.unwrap_or_default();
                if let Some(tracker) = db_tracker {
                    let _ = golish_db::repo::wiki_kb::link_cve_to_wiki(
                        tracker.pool(), &cve_id, &path,
                    ).await;
                }
                return Some((
                    json!({
                        "exists": true,
                        "path": path,
                        "content": existing,
                        "message": format!("Wiki page already exists at {}. Read and update it with write_knowledge if needed.", path),
                    }),
                    true,
                ));
            }

            let mut page_content = format!(
                "---\ntitle: \"{} — {}\"\ncategory: products\ntags: [{}]\ncves: [{}]\nstatus: draft\n---\n\n# {} — {}\n\n",
                cve_id, product, slug, cve_id, cve_id, product
            );

            if !cve_info.is_empty() {
                page_content.push_str("## Vulnerability Details\n\n");
                page_content.push_str(&cve_info);
                page_content.push_str("\n\n");
            } else {
                page_content.push_str("## Vulnerability Details\n\n> No data in vuln intel DB. Research needed.\n\n");
            }

            page_content.push_str("## Exploitation\n\n<!-- Add exploit method, PoC, and attack chain details here -->\n\n");
            page_content.push_str("## Detection & Mitigation\n\n<!-- How to detect and fix -->\n\n");
            page_content.push_str("## Notes\n\n");

            if let Some(ctx) = additional {
                page_content.push_str(&ctx);
                page_content.push('\n');
            }

            if let Some(parent) = full.parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return Some(error_result(format!("mkdir failed: {}", e)));
                }
            }
            if let Err(e) = tokio::fs::write(&full, &page_content).await {
                return Some(error_result(format!("write failed: {}", e)));
            }

            if let Some(tracker) = db_tracker {
                let page = golish_db::models::NewWikiPage {
                    path: path.clone(),
                    title: format!("{} — {}", cve_id, product),
                    category: "products".to_string(),
                    tags: vec![slug.clone(), cve_id.clone()],
                    status: "draft".to_string(),
                    content: page_content.clone(),
                };
                let _ = golish_db::repo::wiki_kb::upsert_page(tracker.pool(), &page).await;
                let _ =
                    golish_db::repo::wiki_kb::link_cve_to_wiki(tracker.pool(), &cve_id, &path)
                        .await;
            }

            Some((
                json!({
                    "success": true,
                    "path": path,
                    "message": format!("Created wiki page for {} at {}. Edit with write_knowledge to add exploit details.", cve_id, path),
                }),
                true,
            ))
        }

        "save_poc" => {
            let cve_id = match extract_string_param(args, &["cve_id"]) {
                Some(c) if !c.is_empty() => c,
                _ => return Some(error_result("save_poc requires a 'cve_id' parameter")),
            };
            let name = match extract_string_param(args, &["name"]) {
                Some(n) if !n.is_empty() => n,
                _ => return Some(error_result("save_poc requires a 'name' parameter")),
            };
            let poc_type = extract_string_param(args, &["poc_type"]).unwrap_or_else(|| "script".to_string());
            let language = extract_string_param(args, &["language"]).unwrap_or_else(|| "python".to_string());
            let content = match args.get("content").and_then(|v| v.as_str()) {
                Some(c) => c.to_string(),
                None => return Some(error_result("save_poc requires a 'content' parameter")),
            };

            let source = extract_string_param(args, &["source"]).unwrap_or_else(|| "manual".to_string());
            let source_url = extract_string_param(args, &["source_url"]).unwrap_or_default();
            let severity = extract_string_param(args, &["severity"]).unwrap_or_else(|| "unknown".to_string());
            let description = extract_string_param(args, &["description"]).unwrap_or_default();
            let tags: Vec<String> = args
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            if let Some(tracker) = db_tracker {
                match golish_db::repo::wiki_kb::upsert_poc_full(
                    tracker.pool(),
                    &cve_id,
                    &name,
                    &poc_type,
                    &language,
                    &content,
                    &source,
                    &source_url,
                    &severity,
                    &description,
                    &tags,
                ).await {
                    Ok(poc) => {
                        Some((
                            json!({
                                "success": true,
                                "poc_id": poc.id.to_string(),
                                "source": source,
                                "severity": severity,
                                "message": format!("PoC '{}' saved for {} (source: {}, severity: {})", name, cve_id, source, severity),
                            }),
                            true,
                        ))
                    }
                    Err(e) => Some(error_result(format!("Failed to save PoC: {}", e))),
                }
            } else {
                Some(error_result("Database not available"))
            }
        }

        "list_cves_with_pocs" => {
            if let Some(tracker) = db_tracker {
                match golish_db::repo::wiki_kb::list_cves_with_pocs(tracker.pool()).await {
                    Ok(rows) => {
                        let items: Vec<serde_json::Value> = rows
                            .iter()
                            .map(|r| {
                                json!({
                                    "cve_id": r.cve_id,
                                    "poc_count": r.poc_count,
                                    "max_severity": r.max_severity,
                                    "any_verified": r.any_verified,
                                    "has_research": r.has_research,
                                    "has_wiki": r.has_wiki,
                                })
                            })
                            .collect();
                        Some((
                            json!({
                                "total": items.len(),
                                "cves": items,
                            }),
                            true,
                        ))
                    }
                    Err(e) => Some(error_result(format!("Failed to list CVEs: {}", e))),
                }
            } else {
                Some(error_result("Database not available"))
            }
        }

        "list_unresearched_cves" => {
            let limit = args
                .get("limit")
                .and_then(|v| v.as_i64())
                .unwrap_or(20);

            if let Some(tracker) = db_tracker {
                match golish_db::repo::wiki_kb::list_unresearched_cves(tracker.pool(), limit).await {
                    Ok(rows) => {
                        let items: Vec<serde_json::Value> = rows
                            .iter()
                            .map(|r| {
                                json!({
                                    "cve_id": r.cve_id,
                                    "poc_count": r.poc_count,
                                    "max_severity": r.max_severity,
                                    "any_verified": r.any_verified,
                                })
                            })
                            .collect();
                        Some((
                            json!({
                                "total": items.len(),
                                "message": format!("{} CVEs have PoCs but no research yet — prioritize by severity", items.len()),
                                "cves": items,
                            }),
                            true,
                        ))
                    }
                    Err(e) => Some(error_result(format!("Failed to list unresearched CVEs: {}", e))),
                }
            } else {
                Some(error_result("Database not available"))
            }
        }

        "poc_stats" => {
            if let Some(tracker) = db_tracker {
                match golish_db::repo::wiki_kb::poc_stats(tracker.pool()).await {
                    Ok(stats) => Some((stats, true)),
                    Err(e) => Some(error_result(format!("Failed to get PoC stats: {}", e))),
                }
            } else {
                Some(error_result("Database not available"))
            }
        }

        _ => None,
    }
}

/// Execute a security analysis tool.
///
/// Handles: log_operation, discover_apis, analyze_js, fingerprint_target,
/// log_scan_result, query_target_data.

fn wiki_base_dir() -> std::path::PathBuf {
    golish_core::paths::app_data_base()
        .expect("cannot resolve home directory")
        .join("wiki")
}

/// Returns (title, category, tags, status)
fn extract_wiki_frontmatter(content: &str) -> (String, String, Vec<String>, String) {
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

const WIKI_CATEGORIES: &[&str] = &["products", "techniques", "pocs", "experience", "analysis"];

fn infer_wiki_category(path: &str) -> String {
    let first_segment = path.split('/').next().unwrap_or("");
    if WIKI_CATEGORIES.contains(&first_segment) {
        first_segment.to_string()
    } else {
        "uncategorized".to_string()
    }
}

async fn kb_search_filesystem_fallback(query: &str, limit: usize) -> ToolResult {
    let base = wiki_base_dir();
    if !base.exists() {
        return (json!({ "results": [], "count": 0, "query": query, "note": "KB not initialized" }), true);
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
    (json!({ "results": results, "count": count, "query": query, "source": "filesystem" }), true)
}

/// Extract markdown links from wiki content and resolve them to wiki paths.
/// Returns Vec<(target_path, context_snippet)>.
fn extract_wiki_links(content: &str, source_path: &str) -> Vec<(String, String)> {
    let re = regex::Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap();
    let source_dir = source_path
        .rsplit_once('/')
        .map(|(d, _)| d)
        .unwrap_or("");

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

        // Normalize ../ and ./ segments
        let parts: Vec<&str> = resolved.split('/').collect();
        let mut normalized: Vec<&str> = Vec::new();
        for p in parts {
            match p {
                "." | "" => {}
                ".." => { normalized.pop(); }
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

/// Rebuild index.md from all wiki pages on disk.
async fn update_wiki_index(base: &std::path::Path) {
    let index_path = base.join("index.md");

    let mut entries: Vec<(String, String, String, String)> = Vec::new();
    let mut stack = vec![base.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(mut rd) = tokio::fs::read_dir(&dir).await else { continue };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') { continue; }
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if !name.ends_with(".md") { continue; }
            let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().to_string();
            if rel == "index.md" || rel == "log.md" || rel == "SCHEMA.md" { continue; }

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

    let mut index = String::from("# Vulnerability Knowledge Base\n\n> Auto-generated index. Updated on every write.\n\n");

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
