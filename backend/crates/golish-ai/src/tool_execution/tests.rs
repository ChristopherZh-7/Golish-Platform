use serde_json::json;

use super::direct::{normalize_run_pty_cmd_args, ToolExecutionError, ToolExecutionResult};
use super::hitl::{ToolExecutionConfig, ToolSource};
use super::route::ToolRoutingCategory;

#[test]
fn test_tool_category_from_name() {
    assert_eq!(
        ToolRoutingCategory::from_tool_name("indexer_search_code"),
        ToolRoutingCategory::Registry
    );
    assert_eq!(
        ToolRoutingCategory::from_tool_name("indexer_analyze_file"),
        ToolRoutingCategory::Registry
    );
    assert_eq!(
        ToolRoutingCategory::from_tool_name("web_fetch"),
        ToolRoutingCategory::WebFetch
    );
    // web_search and other Tavily tools now go through Registry
    assert_eq!(
        ToolRoutingCategory::from_tool_name("web_search"),
        ToolRoutingCategory::Registry
    );
    assert_eq!(
        ToolRoutingCategory::from_tool_name("web_search_answer"),
        ToolRoutingCategory::Registry
    );
    assert_eq!(
        ToolRoutingCategory::from_tool_name("web_extract"),
        ToolRoutingCategory::Registry
    );
    assert_eq!(
        ToolRoutingCategory::from_tool_name("web_crawl"),
        ToolRoutingCategory::Registry
    );
    assert_eq!(
        ToolRoutingCategory::from_tool_name("web_map"),
        ToolRoutingCategory::Registry
    );
    assert_eq!(
        ToolRoutingCategory::from_tool_name("update_plan"),
        ToolRoutingCategory::UpdatePlan
    );
    assert_eq!(
        ToolRoutingCategory::from_tool_name("sub_agent_coder"),
        ToolRoutingCategory::SubAgent
    );
    assert_eq!(
        ToolRoutingCategory::from_tool_name("sub_agent_researcher"),
        ToolRoutingCategory::SubAgent
    );
    assert_eq!(
        ToolRoutingCategory::from_tool_name("read_file"),
        ToolRoutingCategory::Registry
    );
    assert_eq!(
        ToolRoutingCategory::from_tool_name("run_pty_cmd"),
        ToolRoutingCategory::Registry
    );
    assert_eq!(
        ToolRoutingCategory::from_tool_name("run_command"),
        ToolRoutingCategory::Registry
    );
}

#[test]
fn test_tool_source_is_main_agent() {
    assert!(ToolSource::MainAgent.is_main_agent());
    assert!(!ToolSource::sub_agent("coder", 1).is_main_agent());
}

#[test]
fn test_tool_execution_result_constructors() {
    let success = ToolExecutionResult::success(json!({"result": "ok"}));
    assert!(success.success);
    assert!(success.files_modified.is_empty());

    let with_files = ToolExecutionResult::success_with_files(
        json!({"result": "ok"}),
        vec!["file1.rs".to_string()],
    );
    assert!(with_files.success);
    assert_eq!(with_files.files_modified, vec!["file1.rs"]);

    let failure = ToolExecutionResult::failure(json!({"error": "failed"}));
    assert!(!failure.success);

    let error = ToolExecutionResult::error("Something went wrong");
    assert!(!error.success);
    assert_eq!(error.content["error"], "Something went wrong");
}

#[test]
fn test_tool_execution_config_default() {
    let config = ToolExecutionConfig::default();
    assert!(config.require_hitl);
    assert_eq!(config.source, ToolSource::MainAgent);
    assert!(config.allow_sub_agents);
}

#[test]
fn test_normalize_run_pty_cmd_args_array() {
    let args = json!({
        "command": ["cd", "/path", "&&", "pwd"],
        "cwd": "."
    });
    let normalized = normalize_run_pty_cmd_args(args);
    assert_eq!(normalized["command"].as_str().unwrap(), "cd /path && pwd");
    assert_eq!(normalized["cwd"].as_str().unwrap(), ".");
}

#[test]
fn test_normalize_run_pty_cmd_args_string() {
    let args = json!({
        "command": "cd /path && pwd",
        "cwd": "."
    });
    let normalized = normalize_run_pty_cmd_args(args);
    assert_eq!(normalized["command"].as_str().unwrap(), "cd /path && pwd");
}

#[test]
fn test_normalize_run_pty_cmd_args_pipe() {
    let args = json!({
        "command": ["ls", "-la", "|", "grep", "foo"]
    });
    let normalized = normalize_run_pty_cmd_args(args);
    assert_eq!(normalized["command"].as_str().unwrap(), "ls -la | grep foo");
}

#[test]
fn test_tool_execution_error_display() {
    let err = ToolExecutionError::ToolNotFound("unknown_tool".to_string());
    assert_eq!(err.to_string(), "Tool not found: unknown_tool");

    let err = ToolExecutionError::SubAgentNotFound("missing_agent".to_string());
    assert_eq!(err.to_string(), "Sub-agent not found: missing_agent");
}
