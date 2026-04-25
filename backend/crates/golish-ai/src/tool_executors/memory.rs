use serde_json::json;
use super::common::{error_result, extract_string_param, ToolResult};

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
                "search_memories" | "store_memory" | "list_memories"
                | "search_code" | "save_code" | "search_guide" | "save_guide" => {
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

        // --- Code Vector Store (PentAGI multi-store pattern) ---
        "search_code" => {
            let query = match extract_string_param(args, &["query", "search_query", "q"]) {
                Some(q) if !q.is_empty() => q,
                _ => return Some(error_result(
                    "search_code requires a non-empty 'query' parameter."
                )),
            };
            let lang = args.get("language").and_then(|v| v.as_str()).map(String::from);
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5).min(20) as i64;

            match tracker.search_memories_by_doc_type(&query, "code", lang.as_deref(), limit).await {
                Ok(memories) => {
                    let results: Vec<serde_json::Value> = memories.iter().map(|m| {
                        json!({
                            "content": m.content,
                            "language": m.metadata.as_ref().and_then(|md| md.get("language")),
                            "metadata": m.metadata,
                            "created_at": m.created_at.to_rfc3339(),
                        })
                    }).collect();
                    Some((json!({ "code_samples": results, "count": results.len(), "query": query }), true))
                }
                Err(e) => Some(error_result(format!("Code search failed: {}", e))),
            }
        }
        "save_code" => {
            let content = match args.get("content").and_then(|v| v.as_str()) {
                Some(c) => c.to_string(),
                None => return Some(error_result("save_code requires a 'content' parameter")),
            };
            let language = args.get("language").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
            let description = args.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();

            let tagged = format!("[code:{}] {}{}", language, if description.is_empty() { String::new() } else { format!("{} — ", description) }, content);
            let metadata = Some(json!({ "language": language, "description": description, "doc_type": "code" }));

            tracker.store_memory_with_doc_type(&tagged, "technique", "code", metadata);

            Some((json!({ "success": true, "message": "Code sample stored", "language": language }), true))
        }

        // --- Guide Vector Store (PentAGI multi-store pattern) ---
        "search_guide" => {
            let query = match extract_string_param(args, &["query", "search_query", "q"]) {
                Some(q) if !q.is_empty() => q,
                _ => return Some(error_result(
                    "search_guide requires a non-empty 'query' parameter."
                )),
            };
            let guide_type = args.get("type").and_then(|v| v.as_str()).map(String::from);
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5).min(20) as i64;

            match tracker.search_memories_by_doc_type(&query, "guide", guide_type.as_deref(), limit).await {
                Ok(memories) => {
                    let results: Vec<serde_json::Value> = memories.iter().map(|m| {
                        json!({
                            "content": m.content,
                            "guide_type": m.metadata.as_ref().and_then(|md| md.get("guide_type")),
                            "metadata": m.metadata,
                            "created_at": m.created_at.to_rfc3339(),
                        })
                    }).collect();
                    Some((json!({ "guides": results, "count": results.len(), "query": query }), true))
                }
                Err(e) => Some(error_result(format!("Guide search failed: {}", e))),
            }
        }
        "save_guide" => {
            let content = match args.get("content").and_then(|v| v.as_str()) {
                Some(c) => c.to_string(),
                None => return Some(error_result("save_guide requires a 'content' parameter")),
            };
            let guide_type = args.get("type").and_then(|v| v.as_str()).unwrap_or("procedure").to_string();

            let tagged = format!("[guide:{}] {}", guide_type, content);
            let metadata = Some(json!({ "guide_type": guide_type, "doc_type": "guide" }));

            tracker.store_memory_with_doc_type(&tagged, "technique", "guide", metadata);

            Some((json!({ "success": true, "message": "Guide stored", "type": guide_type }), true))
        }

        _ => None,
    }
}
