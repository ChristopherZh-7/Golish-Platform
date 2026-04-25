//! Hand-rolled tool definitions that don't come from the
//! [`golish_tools::build_function_declarations`] catalogue:
//!
//! - [`get_run_command_tool_definition`]: friendly alias for `run_pty_cmd`.
//! - [`get_ask_human_tool_definition`]: HITL barrier tool.
//! - [`get_sub_agent_tool_definitions`]: registry-driven `sub_agent_*` shims.

use golish_core::ToolName;
use golish_sub_agents::SubAgentRegistry;
use rig::completion::ToolDefinition;
use serde_json::json;

use super::sanitize::sanitize_schema;

/// Get the `run_command` tool definition.
///
/// This is a wrapper around `run_pty_cmd` with a more intuitive name. The
/// execution layer maps `run_command` calls to `run_pty_cmd`.
pub fn get_run_command_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: ToolName::RunCommand.as_str().to_string(),
        description: "Execute a shell command and return the output. Use for running builds, tests, git operations, and other CLI commands. The command runs in a shell environment with access to common tools.".to_string(),
        parameters: sanitize_schema(json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory for the command (relative to workspace)"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 120)"
                }
            },
            "required": ["command"]
        })),
    }
}

/// Get the `ask_human` tool definition.
///
/// This barrier tool pauses the agentic loop and asks the user for input.
/// Used when the AI needs credentials, decisions, or guidance it cannot
/// determine on its own.
pub fn get_ask_human_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "ask_human".to_string(),
        description: "Ask the user for information, credentials, or a decision. Use when you need input you cannot determine on your own: login credentials, scope decisions, authorization for risky actions, or expert guidance. This pauses execution until the user responds.".to_string(),
        parameters: sanitize_schema(json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question or information request to show the user"
                },
                "input_type": {
                    "type": "string",
                    "enum": ["credentials", "choice", "freetext", "confirmation"],
                    "description": "Type of input expected: 'credentials' for username/password, 'choice' for selection from options, 'freetext' for open text input, 'confirmation' for yes/no"
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Options for 'choice' type (ignored for other types)"
                },
                "context": {
                    "type": "string",
                    "description": "Additional context about why you need this information"
                }
            },
            "required": ["question", "input_type"]
        })),
    }
}

/// Get sub-agent tool definitions from the registry.
///
/// Each registered sub-agent is exposed as a `sub_agent_<id>` tool with a
/// `task` and optional `context` parameter.
pub async fn get_sub_agent_tool_definitions(registry: &SubAgentRegistry) -> Vec<ToolDefinition> {
    registry
        .all()
        .map(|agent| ToolDefinition {
            name: format!("sub_agent_{}", agent.id),
            description: format!("[{}] {}", agent.name, agent.description),
            parameters: sanitize_schema(json!({
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "The specific task or question for this sub-agent to handle"
                    },
                    "context": {
                        "type": "string",
                        "description": "Optional additional context to help the sub-agent understand the task"
                    }
                },
                "required": ["task"]
            })),
        })
        .collect()
}
