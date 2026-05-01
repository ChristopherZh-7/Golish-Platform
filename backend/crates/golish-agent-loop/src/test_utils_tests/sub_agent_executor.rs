use super::*;

// ========================================================================
// Sub-Agent Executor Tests (Phase 0.5)
// ========================================================================

use golish_sub_agents::{
    execute_sub_agent, SubAgentDefinition, SubAgentExecutorContext, ToolProvider,
    MAX_AGENT_DEPTH,
};
use rig::completion::request::ToolDefinition;

/// Mock ToolProvider for testing sub-agent execution.
struct MockToolProvider {
    allowed_tools: Vec<String>,
}

impl MockToolProvider {
    fn new() -> Self {
        Self {
            allowed_tools: vec![
                "read_file".to_string(),
                "glob".to_string(),
                "grep".to_string(),
            ],
        }
    }

    fn with_allowed_tools(tools: Vec<String>) -> Self {
        Self {
            allowed_tools: tools,
        }
    }
}

#[async_trait::async_trait]
impl ToolProvider for MockToolProvider {
    fn get_all_tool_definitions(&self) -> Vec<ToolDefinition> {
        // Return minimal tool definitions for testing
        self.allowed_tools
            .iter()
            .map(|name| ToolDefinition {
                name: name.clone(),
                description: format!("Mock {} tool", name),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            })
            .collect()
    }

    fn filter_tools_by_allowed(
        &self,
        tools: Vec<ToolDefinition>,
        allowed: &[String],
    ) -> Vec<ToolDefinition> {
        if allowed.is_empty() {
            tools
        } else {
            tools
                .into_iter()
                .filter(|t| allowed.contains(&t.name))
                .collect()
        }
    }

    async fn execute_web_fetch_tool(
        &self,
        tool_name: &str,
        _args: &serde_json::Value,
    ) -> (serde_json::Value, bool) {
        (
            serde_json::json!({ "error": format!("Mock web_fetch tool {} not implemented", tool_name) }),
            false,
        )
    }

    async fn execute_memory_tool(
        &self,
        _tool_name: &str,
        _args: &serde_json::Value,
    ) -> Option<(serde_json::Value, bool)> {
        None
    }

    fn normalize_run_pty_cmd_args(&self, args: serde_json::Value) -> serde_json::Value {
        args
    }
}

/// Create a test sub-agent definition
fn test_sub_agent_definition_for_executor(id: &str) -> SubAgentDefinition {
    SubAgentDefinition::new(
        id,
        format!("Test Agent {}", id),
        "A test sub-agent for unit testing",
        "You are a test sub-agent. Respond with a simple message.",
    )
    .with_tools(vec!["read_file".to_string(), "glob".to_string()])
    .with_max_iterations(3)
}


mod core;
mod events;
