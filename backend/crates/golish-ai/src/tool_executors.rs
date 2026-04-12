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
