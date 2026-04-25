//! Default implementation of ToolProvider for golish-ai.
//!
//! This module provides a concrete implementation of the ToolProvider trait
//! that uses the local tool_definitions and tool_executors modules.

use golish_sub_agents::ToolProvider;
use rig::completion::request::ToolDefinition;

use crate::db_tracking::DbTracker;
use crate::tool_definitions::{filter_tools_by_allowed, get_all_tool_definitions};
use crate::tool_executors::{execute_memory_tool, execute_web_fetch_tool, normalize_run_pty_cmd_args};

/// Default tool provider that uses golish-ai's tool definitions and executors.
pub struct DefaultToolProvider<'a> {
    db_tracker: Option<&'a DbTracker>,
}

impl<'a> DefaultToolProvider<'a> {
    /// Create a new DefaultToolProvider.
    pub fn new() -> Self {
        Self { db_tracker: None }
    }

    pub fn with_db_tracker(db_tracker: Option<&'a DbTracker>) -> Self {
        Self { db_tracker }
    }
}

impl Default for DefaultToolProvider<'_> {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ToolProvider for DefaultToolProvider<'_> {
    fn get_all_tool_definitions(&self) -> Vec<ToolDefinition> {
        get_all_tool_definitions()
    }

    fn filter_tools_by_allowed(
        &self,
        tools: Vec<ToolDefinition>,
        allowed: &[String],
    ) -> Vec<ToolDefinition> {
        filter_tools_by_allowed(tools, allowed)
    }

    async fn execute_web_fetch_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> (serde_json::Value, bool) {
        execute_web_fetch_tool(tool_name, args).await
    }

    async fn execute_memory_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<(serde_json::Value, bool)> {
        execute_memory_tool(tool_name, args, self.db_tracker).await
    }

    fn normalize_run_pty_cmd_args(&self, args: serde_json::Value) -> serde_json::Value {
        normalize_run_pty_cmd_args(args)
    }
}
