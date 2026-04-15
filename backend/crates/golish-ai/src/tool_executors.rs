//! Tool execution logic for the agent bridge.
//!
//! This module contains the logic for executing various types of tools:
//! - Indexer tools (code search, file analysis)
//! - Plan tools (task planning and tracking)
//!
//! Note: Workflow tool execution is handled in the golish crate to avoid
//! circular dependencies with WorkflowState and BridgeLlmExecutor types.

use std::sync::Arc;

use serde_json::json;

use golish_core::events::AiEvent;

use golish_web::web_fetch::WebFetcher;

/// Result type for tool execution: (json_result, success_flag)
type ToolResult = (serde_json::Value, bool);

/// Helper to create an error result
fn error_result(msg: impl Into<String>) -> ToolResult {
    (json!({"error": msg.into()}), false)
}

/// Try to extract a string parameter from args, checking multiple possible key names.
/// Handles models that pass null, numbers, or alternative key names.
fn extract_string_param(args: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(val) = args.get(*key) {
            if let Some(s) = val.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            } else if !val.is_null() {
                let s = val.to_string();
                let s = s.trim().trim_matches('"');
                if !s.is_empty() && s != "null" {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

/// Execute a web fetch tool using readability-based content extraction.
pub async fn execute_web_fetch_tool(tool_name: &str, args: &serde_json::Value) -> ToolResult {
    if tool_name != "web_fetch" {
        return error_result(format!("Unknown web fetch tool: {}", tool_name));
    }

    // web_fetch expects a single "url" parameter (not "urls" array)
    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u.to_string(),
        None => {
            return error_result(
                "web_fetch requires a 'url' parameter (string). Example: {\"url\": \"https://example.com\"}"
            )
        }
    };

    let fetcher = WebFetcher::new();

    match fetcher.fetch(&url).await {
        Ok(result) => (
            json!({
                "url": result.url,
                "content": result.content
            }),
            true,
        ),
        Err(e) => error_result(format!("Failed to fetch {}: {}", url, e)),
    }
}

/// Execute the update_plan tool.
///
/// Updates the task plan with new steps and their statuses.
/// Emits a PlanUpdated event when the plan is successfully updated.
pub async fn execute_plan_tool(
    plan_manager: &Arc<crate::planner::PlanManager>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<AiEvent>,
    args: &serde_json::Value,
) -> ToolResult {
    // Parse the arguments into UpdatePlanArgs
    let update_args: crate::planner::UpdatePlanArgs = match serde_json::from_value(args.clone()) {
        Ok(a) => a,
        Err(e) => return error_result(format!("Invalid update_plan arguments: {}", e)),
    };

    // Update the plan
    match plan_manager.update_plan(update_args).await {
        Ok(plan) => {
            // Emit PlanUpdated event
            let _ = event_tx.send(AiEvent::PlanUpdated {
                version: plan.version,
                summary: plan.summary.clone(),
                steps: plan.steps.clone(),
                explanation: None,
            });

            (
                json!({
                    "success": true,
                    "version": plan.version,
                    "summary": {
                        "total": plan.summary.total,
                        "completed": plan.summary.completed,
                        "in_progress": plan.summary.in_progress,
                        "pending": plan.summary.pending
                    }
                }),
                true,
            )
        }
        Err(e) => error_result(format!("Failed to update plan: {}", e)),
    }
}

/// Execute the ask_human barrier tool.
///
/// Emits an AskHumanRequest event to the frontend, pauses the agentic loop,
/// and waits for the user to respond. Uses the same coordinator/oneshot
/// pattern as HITL tool approval.
pub async fn execute_ask_human_tool(
    args: &serde_json::Value,
    event_tx: &tokio::sync::mpsc::UnboundedSender<AiEvent>,
    coordinator: Option<&crate::event_coordinator::CoordinatorHandle>,
    pending_approvals: &tokio::sync::RwLock<
        std::collections::HashMap<String, tokio::sync::oneshot::Sender<golish_core::hitl::ApprovalDecision>>,
    >,
) -> (serde_json::Value, bool) {
    let question = args.get("question").and_then(|v| v.as_str()).unwrap_or("I need your input.");
    let input_type = args.get("input_type").and_then(|v| v.as_str()).unwrap_or("freetext");
    let options: Vec<String> = args.get("options")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let context = args.get("context").and_then(|v| v.as_str()).unwrap_or("").to_string();

    let request_id = uuid::Uuid::new_v4().to_string();

    // Register a oneshot channel to wait for the user's response.
    // We reuse the approval mechanism: the frontend will send an ApprovalDecision
    // where `approved=true` means the user responded (reason contains the text),
    // and `approved=false` means the user skipped.
    let rx = if let Some(coord) = coordinator {
        coord.register_approval(request_id.clone())
    } else {
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut pending = pending_approvals.write().await;
            pending.insert(request_id.clone(), tx);
        }
        rx
    };

    let _ = event_tx.send(AiEvent::AskHumanRequest {
        request_id: request_id.clone(),
        question: question.to_string(),
        input_type: input_type.to_string(),
        options,
        context,
    });

    tracing::info!("[ask_human] Waiting for user response: id={}, type={}", request_id, input_type);

    const ASK_HUMAN_TIMEOUT_SECS: u64 = 600;

    match tokio::time::timeout(
        std::time::Duration::from_secs(ASK_HUMAN_TIMEOUT_SECS),
        rx,
    ).await {
        Ok(Ok(decision)) => {
            let _ = event_tx.send(AiEvent::AskHumanResponse {
                request_id,
                response: decision.reason.clone().unwrap_or_default(),
                skipped: !decision.approved,
            });

            if decision.approved {
                (json!({
                    "response": decision.reason.unwrap_or_default(),
                    "skipped": false,
                }), true)
            } else {
                (json!({
                    "skipped": true,
                    "message": "User chose to skip this request. Adapt your approach accordingly.",
                }), true)
            }
        }
        Ok(Err(_)) => {
            (json!({
                "error": "Request was cancelled",
                "skipped": true,
            }), false)
        }
        Err(_) => {
            tracing::warn!("[ask_human] Timed out after {}s", ASK_HUMAN_TIMEOUT_SECS);
            if coordinator.is_none() {
                let mut pending = pending_approvals.write().await;
                pending.remove(&request_id);
            }
            (json!({
                "error": format!("No response within {} seconds", ASK_HUMAN_TIMEOUT_SECS),
                "timeout": true,
                "skipped": true,
            }), false)
        }
    }
}

/// Execute memory tool calls (search_memories, store_memory, list_memories).
///
/// Delegates to the `DbTracker` for database operations. Returns graceful errors
/// if the database is not ready.
pub async fn execute_memory_tool(
    tool_name: &str,
    args: &serde_json::Value,
    db_tracker: Option<&crate::db_tracking::DbTracker>,
) -> Option<ToolResult> {
    let tracker = match db_tracker {
        Some(t) => t,
        None => {
            return match tool_name {
                "search_memories" | "store_memory" | "list_memories" => {
                    Some(error_result("Memory tools are not available (database not configured)"))
                }
                _ => None,
            };
        }
    };

    match tool_name {
        "search_memories" => {
            let query = match extract_string_param(args, &["query", "search_query", "q"]) {
                Some(q) if !q.is_empty() => q,
                _ => return Some(error_result(
                    "search_memories requires a non-empty 'query' string parameter. \
                     Example: {\"query\": \"nmap scan results for 10.0.0.1\"}"
                )),
            };
            let category = args.get("category").and_then(|v| v.as_str()).map(String::from);
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(10)
                .min(50) as i64;

            match tracker.search_memories_by_text(&query, category.as_deref(), limit).await {
                Ok(memories) => {
                    let results: Vec<serde_json::Value> = memories
                        .iter()
                        .map(|m| {
                            json!({
                                "content": m.content,
                                "mem_type": m.mem_type,
                                "metadata": m.metadata,
                                "created_at": m.created_at.to_rfc3339(),
                            })
                        })
                        .collect();
                    Some((
                        json!({
                            "memories": results,
                            "count": results.len(),
                            "query": query,
                        }),
                        true,
                    ))
                }
                Err(e) => Some(error_result(format!("Memory search failed: {}", e))),
            }
        }
        "store_memory" => {
            let content = match args.get("content").and_then(|v| v.as_str()) {
                Some(c) => c.to_string(),
                None => return Some(error_result("store_memory requires a 'content' parameter")),
            };
            let category = args
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("recon")
                .to_string();
            let tags = args
                .get("tags")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let scope = args
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("project");

            let memory_content = if tags.is_empty() {
                format!("[{}] {}", category, content)
            } else {
                format!("[{}] [tags: {}] {}", category, tags, content)
            };

            let metadata = if tags.is_empty() {
                Some(json!({ "category": category }))
            } else {
                Some(json!({ "category": category, "tags": tags }))
            };

            if scope == "global" {
                tracker.store_memory_global(&memory_content, "observation", metadata);
            } else {
                tracker.store_memory(&memory_content, "observation", metadata);
            }

            Some((
                json!({
                    "success": true,
                    "message": format!("Memory stored successfully (scope: {})", scope),
                    "category": category,
                    "scope": scope,
                }),
                true,
            ))
        }
        "list_memories" => {
            let category = args.get("category").and_then(|v| v.as_str()).map(String::from);
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(20)
                .min(100) as i64;

            match tracker.list_recent_memories(category.as_deref(), limit).await {
                Ok(memories) => {
                    let results: Vec<serde_json::Value> = memories
                        .iter()
                        .map(|m| {
                            json!({
                                "content": m.content,
                                "mem_type": m.mem_type,
                                "metadata": m.metadata,
                                "created_at": m.created_at.to_rfc3339(),
                            })
                        })
                        .collect();
                    Some((
                        json!({
                            "memories": results,
                            "count": results.len(),
                        }),
                        true,
                    ))
                }
                Err(e) => Some(error_result(format!("Failed to list memories: {}", e))),
            }
        }
        _ => None,
    }
}

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
        "search_knowledge_base" | "write_knowledge" | "read_knowledge" | "ingest_cve" | "save_poc"
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
            if let Some(parent) = full.parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return Some(error_result(format!("mkdir failed: {}", e)));
                }
            }
            if let Err(e) = tokio::fs::write(&full, &content).await {
                return Some(error_result(format!("write failed: {}", e)));
            }

            let cve_id_opt = extract_string_param(args, &["cve_id"]);

            if let Some(tracker) = db_tracker {
                let (title, fm_category, tags, status) = extract_wiki_frontmatter(&content);
                let category = if fm_category == "uncategorized" {
                    infer_wiki_category(&path)
                } else {
                    fm_category
                };
                let page = golish_db::models::NewWikiPage {
                    path: path.clone(),
                    title: title.clone(),
                    category: category.clone(),
                    tags: tags.clone(),
                    status,
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
            }

            let mut msg = format!("Knowledge page written to {}", path);
            if let Some(ref cve) = cve_id_opt {
                msg.push_str(&format!(" (linked to {})", cve));
            }

            Some((
                json!({
                    "success": true,
                    "path": path,
                    "linked_cve": cve_id_opt,
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

            if let Some(tracker) = db_tracker {
                match golish_db::repo::wiki_kb::upsert_poc(
                    tracker.pool(),
                    &cve_id,
                    &name,
                    &poc_type,
                    &language,
                    &content,
                ).await {
                    Ok(poc) => {
                        Some((
                            json!({
                                "success": true,
                                "poc_id": poc.id.to_string(),
                                "message": format!("PoC '{}' saved for {}. It will appear in the PoC tab.", name, cve_id),
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

        _ => None,
    }
}

fn wiki_base_dir() -> std::path::PathBuf {
    let home = dirs::home_dir().expect("cannot resolve home directory");
    #[cfg(target_os = "macos")]
    let base = home
        .join("Library")
        .join("Application Support")
        .join("golish-platform");
    #[cfg(target_os = "windows")]
    let base = home.join("AppData").join("Local").join("golish-platform");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = home.join(".golish-platform");
    base.join("wiki")
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

/// Normalize tool arguments for run_pty_cmd.
/// If the command is passed as an array, convert it to a space-joined string.
/// This prevents shell_words::join() from quoting metacharacters like &&, ||, |, etc.
pub fn normalize_run_pty_cmd_args(mut args: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = args.as_object_mut() {
        if let Some(command) = obj.get_mut("command") {
            if let Some(arr) = command.as_array() {
                // Convert array to space-joined string
                let cmd_str: String = arr
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                *command = serde_json::Value::String(cmd_str);
            }
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_normalize_run_pty_cmd_array_to_string() {
        // Command as array with shell operators
        let args = json!({
            "command": ["cd", "/path", "&&", "pwd"],
            "cwd": "."
        });

        let normalized = normalize_run_pty_cmd_args(args);

        assert_eq!(normalized["command"].as_str().unwrap(), "cd /path && pwd");
        // Other fields should be preserved
        assert_eq!(normalized["cwd"].as_str().unwrap(), ".");
    }

    #[test]
    fn test_normalize_run_pty_cmd_string_unchanged() {
        // Command already as string - should be unchanged
        let args = json!({
            "command": "cd /path && pwd",
            "cwd": "."
        });

        let normalized = normalize_run_pty_cmd_args(args);

        assert_eq!(normalized["command"].as_str().unwrap(), "cd /path && pwd");
    }

    #[test]
    fn test_normalize_run_pty_cmd_pipe_operator() {
        let args = json!({
            "command": ["ls", "-la", "|", "grep", "foo"]
        });

        let normalized = normalize_run_pty_cmd_args(args);

        assert_eq!(normalized["command"].as_str().unwrap(), "ls -la | grep foo");
    }

    #[test]
    fn test_normalize_run_pty_cmd_redirect() {
        let args = json!({
            "command": ["echo", "hello", ">", "output.txt"]
        });

        let normalized = normalize_run_pty_cmd_args(args);

        assert_eq!(
            normalized["command"].as_str().unwrap(),
            "echo hello > output.txt"
        );
    }

    #[test]
    fn test_normalize_run_pty_cmd_empty_array() {
        let args = json!({
            "command": []
        });

        let normalized = normalize_run_pty_cmd_args(args);

        assert_eq!(normalized["command"].as_str().unwrap(), "");
    }

    #[test]
    fn test_normalize_run_pty_cmd_no_command_field() {
        // Args without command field should pass through unchanged
        let args = json!({
            "cwd": "/some/path"
        });

        let normalized = normalize_run_pty_cmd_args(args.clone());

        assert_eq!(normalized, args);
    }

    #[test]
    fn test_extract_string_param_normal() {
        let args = json!({"query": "nmap results"});
        assert_eq!(
            extract_string_param(&args, &["query"]),
            Some("nmap results".to_string())
        );
    }

    #[test]
    fn test_extract_string_param_null() {
        let args = json!({"query": null});
        assert_eq!(extract_string_param(&args, &["query"]), None);
    }

    #[test]
    fn test_extract_string_param_empty() {
        let args = json!({"query": ""});
        assert_eq!(extract_string_param(&args, &["query"]), None);
    }

    #[test]
    fn test_extract_string_param_alternate_key() {
        let args = json!({"search_query": "test"});
        assert_eq!(
            extract_string_param(&args, &["query", "search_query"]),
            Some("test".to_string())
        );
    }

    #[test]
    fn test_extract_string_param_number() {
        let args = json!({"query": 42});
        assert_eq!(
            extract_string_param(&args, &["query"]),
            Some("42".to_string())
        );
    }

    #[test]
    fn test_extract_string_param_missing() {
        let args = json!({"other": "value"});
        assert_eq!(extract_string_param(&args, &["query"]), None);
    }
}
