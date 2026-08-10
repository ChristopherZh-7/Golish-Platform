//! Default implementation of ToolProvider for golish-ai.

use std::sync::Arc;

use golish_core::WebFetchProvider;
use golish_sub_agents::ToolProvider;
use rig::completion::request::ToolDefinition;

use crate::db_tracking::DbTracker;
use crate::tool_definitions::{filter_tools_by_allowed, get_all_tool_definitions};
use crate::tool_executors::{
    execute_intel_public_tool, execute_knowledge_base_tool, execute_memory_tool,
    execute_web_fetch_tool, intel_public_tool_definitions, normalize_run_pty_cmd_args,
    IntelPublicEvidenceAdapter,
};

/// Default tool provider that uses golish-ai's tool definitions and executors.
pub struct DefaultToolProvider<'a> {
    db_tracker: Option<&'a DbTracker>,
    web_fetcher: Option<Arc<dyn WebFetchProvider>>,
    intel_public_adapter: Option<Arc<dyn IntelPublicEvidenceAdapter>>,
    intel_public_fixture_enabled: bool,
}

impl<'a> DefaultToolProvider<'a> {
    pub fn new() -> Self {
        Self {
            db_tracker: None,
            web_fetcher: None,
            intel_public_adapter: None,
            intel_public_fixture_enabled: false,
        }
    }

    pub fn with_db_tracker(db_tracker: Option<&'a DbTracker>) -> Self {
        Self {
            db_tracker,
            web_fetcher: None,
            intel_public_adapter: None,
            intel_public_fixture_enabled: false,
        }
    }

    pub fn with_web_fetcher(mut self, fetcher: Option<Arc<dyn WebFetchProvider>>) -> Self {
        self.web_fetcher = fetcher;
        self
    }

    pub fn with_intel_public_adapter(
        mut self,
        adapter: Option<Arc<dyn IntelPublicEvidenceAdapter>>,
    ) -> Self {
        self.intel_public_adapter = adapter;
        self.intel_public_fixture_enabled = self.intel_public_adapter.is_some();
        self
    }

    pub fn with_intel_public_fixture(
        mut self,
        enabled: bool,
        adapter: Option<Arc<dyn IntelPublicEvidenceAdapter>>,
    ) -> Self {
        self.intel_public_fixture_enabled = enabled;
        self.intel_public_adapter = adapter;
        self
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
        if let Some(ref fetcher) = self.web_fetcher {
            execute_web_fetch_tool(fetcher.as_ref(), tool_name, args).await
        } else {
            (
                serde_json::json!({"error": "Web fetch provider not configured"}),
                false,
            )
        }
    }

    fn intel_public_tool_definitions(&self) -> Option<Vec<ToolDefinition>> {
        self.intel_public_fixture_enabled
            .then(intel_public_tool_definitions)
    }

    async fn execute_intel_public_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<(serde_json::Value, bool)> {
        let adapter = self.intel_public_adapter.as_ref()?;
        Some(execute_intel_public_tool(adapter.as_ref(), tool_name, args).await)
    }

    async fn execute_memory_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<(serde_json::Value, bool)> {
        execute_memory_tool(tool_name, args, self.db_tracker).await
    }

    async fn execute_knowledge_base_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<(serde_json::Value, bool)> {
        execute_knowledge_base_tool(tool_name, args, self.db_tracker).await
    }

    fn normalize_run_pty_cmd_args(&self, args: serde_json::Value) -> serde_json::Value {
        normalize_run_pty_cmd_args(args)
    }
}
