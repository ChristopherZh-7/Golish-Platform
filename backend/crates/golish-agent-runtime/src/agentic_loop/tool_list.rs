//! Build the per-turn list of `rig::completion::ToolDefinition`s by
//! delegating to the active [`crate::execution_mode::ExecutionModePolicy`].
//!
//! The hard-coded chat / task `if/else` branches that used to live here —
//! including the `tool.name.starts_with("pentest_")` filter that silently
//! dropped the eight `pentest_bridge` tools in chat mode — have been
//! lifted out into per-mode policies under
//! `crate::execution_mode::modes::*`. Adding a new mode is now a matter of
//! creating one new file and registering it in
//! `ExecutionModeRegistry::default`. This module no longer needs touching.

use golish_sub_agents::SubAgentContext;

use super::context::AgenticLoopContext;
use crate::execution_mode::context::PolicyContext;
use crate::execution_mode::selection_apply::apply_tool_selection;

pub(crate) async fn build_tool_list(
    ctx: &AgenticLoopContext<'_>,
    sub_agent_context: &SubAgentContext,
) -> Vec<rig::completion::ToolDefinition> {
    let mode_id: &str = ctx.execution_mode.into();
    let policy = match ctx.execution_mode_registry.get(mode_id) {
        Some(p) => p,
        None => {
            tracing::error!(
                "[tool_list] unknown execution mode '{}', falling back to chat",
                mode_id
            );
            ctx.execution_mode_registry
                .get("chat")
                .expect("default ExecutionModeRegistry must contain `chat`")
        }
    };

    let workspace_guard = ctx.workspace.read().await;
    let policy_ctx = PolicyContext::new(&workspace_guard, golish_core::AgentMode::default())
        .with_depth(sub_agent_context.depth)
        .with_mcp_tool_count(ctx.additional_tool_definitions.len());
    let selection = if sub_agent_context.depth == 0 {
        policy.primary_tools(&policy_ctx).await
    } else {
        policy.subtask_tools(&policy_ctx).await
    };
    drop(workspace_guard);

    apply_tool_selection(selection, ctx, sub_agent_context).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestContextBuilder;
    use golish_agent_kit::execution_mode::ExecutionMode;
    use golish_agent_kit::tool_definitions::{ToolConfig, ToolPreset};
    use golish_llm_providers::LlmClient;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    /// Chat-mode primary turn must include the static toolbox basics
    /// (read_file, run_pty_cmd, ask_human). This is the live regression
    /// guard for the `tool.name.starts_with("pentest_")` filter bug.
    #[tokio::test]
    async fn chat_mode_includes_static_tools_and_run_command() {
        let test_ctx = TestContextBuilder::new()
            .execution_mode(ExecutionMode::Chat)
            .build()
            .await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);

        let names: Vec<String> = build_tool_list(&ctx, &SubAgentContext::default())
            .await
            .into_iter()
            .map(|t| t.name)
            .collect();

        assert!(
            names.iter().any(|n| n == "read_file"),
            "chat must expose static file_ops; got: {:?}",
            names
        );
        assert!(names.iter().any(|n| n == "run_pty_cmd"));
        assert!(names.iter().any(|n| n == "ask_human"));
        assert!(
            names.iter().all(|n| !n.starts_with("sub_agent_")),
            "chat mode must NOT expose sub_agent_* dispatchers"
        );
    }

    /// ToolPreset::None is used by silent utility sessions such as
    /// title generation. It must suppress policy-level aliases too.
    #[tokio::test]
    async fn none_tool_preset_exposes_no_tools_even_in_chat_mode() {
        let test_ctx = TestContextBuilder::new()
            .execution_mode(ExecutionMode::Chat)
            .tool_config(ToolConfig::with_preset(ToolPreset::None))
            .build()
            .await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);

        let names: Vec<String> = build_tool_list(&ctx, &SubAgentContext::default())
            .await
            .into_iter()
            .map(|t| t.name)
            .collect();

        assert!(
            names.is_empty(),
            "ToolPreset::None must expose no tools; got: {:?}",
            names
        );
    }

    /// Task primary (depth=0) is orchestration-only: no static tools, no
    /// run_command — only ask_human (sub-agent dispatchers come from the
    /// sub_agent_registry which is empty in this test fixture).
    #[tokio::test]
    async fn task_primary_has_no_static_tools_or_run_command() {
        let test_ctx = TestContextBuilder::new()
            .execution_mode(ExecutionMode::Task)
            .build()
            .await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);

        let names: Vec<String> = build_tool_list(&ctx, &SubAgentContext::default())
            .await
            .into_iter()
            .map(|t| t.name)
            .collect();

        assert!(
            !names.iter().any(|n| n == "read_file"),
            "task primary must NOT expose static file_ops; got: {:?}",
            names
        );
        assert!(!names.iter().any(|n| n == "run_pty_cmd"));
        assert!(names.iter().any(|n| n == "ask_human"));
    }

    /// Task subtask (depth=1) inherits the full toolbox minus update_plan
    /// and ask_human (subtasks must not block on user input).
    #[tokio::test]
    async fn task_subtask_includes_static_minus_update_plan_and_ask_human() {
        let test_ctx = TestContextBuilder::new()
            .execution_mode(ExecutionMode::Task)
            .build()
            .await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);

        let subtask = SubAgentContext {
            depth: 1,
            ..Default::default()
        };

        let names: Vec<String> = build_tool_list(&ctx, &subtask)
            .await
            .into_iter()
            .map(|t| t.name)
            .collect();

        assert!(names.iter().any(|n| n == "read_file"));
        assert!(names.iter().any(|n| n == "run_pty_cmd"));
        assert!(
            !names.iter().any(|n| n == "update_plan"),
            "subtask must NOT expose update_plan; got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n == "ask_human"),
            "subtasks must NOT block on ask_human"
        );
    }
}
