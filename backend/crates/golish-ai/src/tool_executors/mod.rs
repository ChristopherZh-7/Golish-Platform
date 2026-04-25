//! Tool execution logic for the agent bridge.
//!
//! This module contains the logic for executing various types of tools:
//! - Web fetch tools (readability-based content extraction)
//! - Plan tools (task planning and tracking)
//! - Ask human (barrier tool for user interaction)
//! - Memory tools (search, store, list memories + code/guide stores)
//! - Knowledge base tools (wiki-based vulnerability knowledge)
//! - Security analysis tools (finding management and analysis)
//! - Shell helpers (PTY command normalization)
//!
//! Note: Workflow tool execution is handled in the golish crate to avoid
//! circular dependencies with WorkflowState and BridgeLlmExecutor types.

mod common;
mod web;
mod plan;
mod ask_human;
mod memory;
pub mod knowledge_base;
pub mod security;
mod shell;

pub use common::{ToolResult, error_result, extract_string_param};
pub use web::execute_web_fetch_tool;
pub use plan::execute_plan_tool;
pub use ask_human::execute_ask_human_tool;
pub use memory::execute_memory_tool;
pub use knowledge_base::execute_knowledge_base_tool;
pub use security::execute_security_analysis_tool;
pub use shell::normalize_run_pty_cmd_args;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_normalize_run_pty_cmd_array_to_string() {
        let args = json!({
            "command": ["cd", "/path", "&&", "pwd"],
            "cwd": "."
        });

        let normalized = normalize_run_pty_cmd_args(args);

        assert_eq!(normalized["command"].as_str().unwrap(), "cd /path && pwd");
        assert_eq!(normalized["cwd"].as_str().unwrap(), ".");
    }

    #[test]
    fn test_normalize_run_pty_cmd_string_unchanged() {
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
