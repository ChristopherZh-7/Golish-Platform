//! Direct (non-routed) tool execution helpers and shared result/error types.
//!
//! "Direct" here means "actually invoke the tool" — as opposed to the
//! routing layer in [`super::route`] which decides *which* direct executor to
//! call. This module owns:
//!
//! - The shared [`ToolExecutionResult`] / [`ToolExecutionError`] types every
//!   executor returns.
//! - [`execute_registry_tool`]: the workhorse path that delegates to
//!   `ToolRegistry` (and re-maps the friendly `run_command` alias to
//!   `run_pty_cmd`).
//! - The 3 specialised handlers ([`execute_web_fetch_tool_routed`],
//!   [`execute_plan_tool_routed`], [`execute_sub_agent_placeholder`]) that
//!   the router dispatches to for non-registry tools.
//! - [`normalize_run_pty_cmd_args`]: shell-array → string normalisation.

use std::sync::Arc;

use serde_json::Value;
use thiserror::Error;
use tokio::sync::RwLock;

use crate::planner::PlanManager;
use golish_core::ToolName;
use golish_sub_agents::SubAgentRegistry;
use golish_tools::ToolRegistry;

/// Result of successful tool execution with metadata.
#[derive(Debug, Clone)]
pub struct ToolExecutionResult {
    /// The result content (JSON serialisable).
    pub content: Value,
    /// Whether the tool execution was successful.
    pub success: bool,
    /// Files modified by this tool execution (if any).
    pub files_modified: Vec<String>,
}

impl ToolExecutionResult {
    /// Create a successful result.
    pub fn success(content: Value) -> Self {
        Self {
            content,
            success: true,
            files_modified: vec![],
        }
    }

    /// Create a successful result with modified files.
    pub fn success_with_files(content: Value, files: Vec<String>) -> Self {
        Self {
            content,
            success: true,
            files_modified: files,
        }
    }

    /// Create a failure result.
    pub fn failure(content: Value) -> Self {
        Self {
            content,
            success: false,
            files_modified: vec![],
        }
    }

    /// Create an error result from a message.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: serde_json::json!({ "error": message.into() }),
            success: false,
            files_modified: vec![],
        }
    }
}

/// Errors that can occur during tool execution.
#[derive(Debug, Error)]
pub enum ToolExecutionError {
    /// Tool was not found in the registry.
    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    /// Tool is not allowed in the current context.
    #[error("Tool not allowed: {0}")]
    ToolNotAllowed(String),

    /// Required state (indexer, tavily, etc.) is not initialised.
    #[error("Required state not initialized: {0}")]
    StateNotInitialized(String),

    /// Sub-agent not found in the registry.
    #[error("Sub-agent not found: {0}")]
    SubAgentNotFound(String),

    /// Tool execution failed.
    #[error("Tool execution failed: {0}")]
    ExecutionFailed(String),

    /// Invalid tool arguments.
    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),
}

/// Execute a `web_fetch` tool (currently a placeholder; the routed call is
/// re-wired by the agentic loop to the real implementation).
pub(super) async fn execute_web_fetch_tool_routed(
    tool_name: &str,
    tool_args: &Value,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    if ToolName::from_str(tool_name) != Some(ToolName::WebFetch) {
        return Err(ToolExecutionError::ToolNotFound(tool_name.to_string()));
    }

    tracing::debug!(tool = %tool_name, "Routing to web fetch tool executor");

    Ok(ToolExecutionResult::success(serde_json::json!({
        "_placeholder": true,
        "_tool": tool_name,
        "_args": tool_args,
        "_routed_to": "web_fetch"
    })))
}

/// Execute the `update_plan` tool (placeholder — wired up by the agentic
/// loop's plan executor).
pub(super) async fn execute_plan_tool_routed(
    _plan_manager: &Arc<PlanManager>,
    tool_args: &Value,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    tracing::debug!("Routing to plan tool executor");

    Ok(ToolExecutionResult::success(serde_json::json!({
        "_placeholder": true,
        "_tool": "update_plan",
        "_args": tool_args,
        "_routed_to": "plan"
    })))
}

/// Placeholder for sub-agent execution.
///
/// Actual sub-agent execution requires the model and full context, which is
/// wired up in the agentic loop. Here we only verify the agent exists and
/// return a marker payload so the router has something to log.
pub(super) async fn execute_sub_agent_placeholder(
    sub_agent_registry: &Arc<RwLock<SubAgentRegistry>>,
    tool_name: &str,
    tool_args: &Value,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let agent_id = tool_name.strip_prefix("sub_agent_").ok_or_else(|| {
        ToolExecutionError::InvalidArguments("Invalid sub-agent tool name".to_string())
    })?;

    let registry = sub_agent_registry.read().await;
    if registry.get(agent_id).is_none() {
        return Err(ToolExecutionError::SubAgentNotFound(agent_id.to_string()));
    }

    tracing::debug!(agent_id = %agent_id, "Routing to sub-agent executor");

    Ok(ToolExecutionResult::success(serde_json::json!({
        "_placeholder": true,
        "_tool": tool_name,
        "_args": tool_args,
        "_routed_to": "sub_agent",
        "_agent_id": agent_id
    })))
}

/// Execute a standard registry-based tool.
///
/// Maps `run_command` to `run_pty_cmd` (run_command is a user-friendly
/// alias). Detects failure via `exit_code != 0` *or* presence of an `error`
/// field and reports the result accordingly.
pub(super) async fn execute_registry_tool(
    tool_registry: &Arc<RwLock<ToolRegistry>>,
    tool_name: &str,
    tool_args: &Value,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let effective_tool_name = match ToolName::from_str(tool_name) {
        Some(ToolName::RunCommand) => ToolName::RunPtyCmd.as_str(),
        _ => tool_name,
    };

    let registry = tool_registry.read().await;
    let result = registry
        .execute_tool(effective_tool_name, tool_args.clone())
        .await;

    match result {
        Ok(value) => {
            // Check for failure: exit_code != 0 OR presence of "error" field.
            let is_failure_by_exit_code = value
                .get("exit_code")
                .and_then(|ec| ec.as_i64())
                .map(|ec| ec != 0)
                .unwrap_or(false);
            let has_error_field = value.get("error").is_some();
            let is_success = !is_failure_by_exit_code && !has_error_field;

            if is_success {
                Ok(ToolExecutionResult::success(value))
            } else {
                Ok(ToolExecutionResult::failure(value))
            }
        }
        Err(e) => Ok(ToolExecutionResult::error(e.to_string())),
    }
}

/// Normalise tool arguments for `run_pty_cmd`.
///
/// If the command is passed as an array, convert it to a space-joined
/// string. This prevents `shell_words::join()` downstream from quoting
/// metacharacters like `&&`, `||`, `|`, etc.
pub fn normalize_run_pty_cmd_args(mut args: Value) -> Value {
    if let Some(obj) = args.as_object_mut() {
        if let Some(command) = obj.get_mut("command") {
            if let Some(arr) = command.as_array() {
                let cmd_str: String = arr
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                *command = Value::String(cmd_str);
            }
        }
    }
    args
}
