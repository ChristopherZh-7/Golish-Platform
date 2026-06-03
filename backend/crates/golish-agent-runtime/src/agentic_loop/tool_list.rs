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

    let mut tools = apply_tool_selection(selection, ctx, sub_agent_context).await;
    // D1 · hide scan tools entirely for an active stage that permits none (e.g.
    // scoping / target_intel / reporting have empty `allowed_tool_types`). The
    // per-call guard already blocks them, but hiding them stops the model from
    // wasting turns trying a tool it could only ever be denied.
    hide_scans_for_zero_scan_stage(&mut tools, ctx.harness_stage);
    tools
}

/// D1 · when an active harness stage allows no scan tools, strip scan-execution
/// tools AND offensive sub-agent dispatchers (pentester / browser) from the
/// exposed list so the model never attempts (or delegates) work it could only be
/// denied. No-op when no stage is active or the stage permits ≥1 scan type.
fn hide_scans_for_zero_scan_stage(
    tools: &mut Vec<rig::completion::ToolDefinition>,
    harness_stage: Option<golish_agent_kit::harness::StageKind>,
) {
    let Some(kind) = harness_stage else {
        return;
    };
    let Ok(spec) = golish_agent_kit::harness::load_embedded_stage_spec(kind) else {
        return;
    };
    if !spec.allowed_tool_types.is_empty() {
        return;
    }
    let before = tools.len();
    // Hide scan-execution tools AND offensive sub-agent dispatchers: a stage that
    // permits no scans must not delegate active recon / exploitation either, or a
    // weak model burns the whole stage re-submitting + spawning a pentester it
    // could only ever be blocked on (the per-call guard still backstops scans).
    tools.retain(|t| {
        !golish_agent_kit::harness::is_scan_tool_name(&t.name)
            && !golish_agent_kit::harness::is_offensive_sub_agent(&t.name)
    });
    if tools.len() != before {
        tracing::debug!(
            target: "harness::hook",
            stage = %kind.as_str(),
            removed = before - tools.len(),
            "tool-list: hid scan + offensive sub-agent tools for a stage that permits none"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestContextBuilder;

    fn td(name: &str) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: name.to_string(),
            description: "d".to_string(),
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        }
    }

    /// D1 · a stage with empty `allowed_tool_types` (scoping) hides scan tools but
    /// keeps meta/control-plane tools; a scan-permitting stage (enumeration) and
    /// the no-stage case leave the list untouched.
    #[test]
    fn hide_scans_strips_scan_tools_only_in_zero_scan_stage() {
        use golish_agent_kit::harness::StageKind;

        let mut tools = vec![
            td("pentest_run"),
            td("run_pty_cmd"),
            td("submit_stage_deliverable"),
            td("query_target_data"),
            td("sub_agent_pentester"),
            td("sub_agent_reporter"),
        ];
        hide_scans_for_zero_scan_stage(&mut tools, Some(StageKind::Scoping));
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            !names.contains(&"pentest_run"),
            "scan wrapper must be hidden"
        );
        assert!(
            !names.contains(&"run_pty_cmd"),
            "scan wrapper must be hidden"
        );
        assert!(
            !names.contains(&"sub_agent_pentester"),
            "offensive sub-agent must be hidden in a zero-scan stage: {names:?}"
        );
        assert!(
            names.contains(&"submit_stage_deliverable")
                && names.contains(&"query_target_data")
                && names.contains(&"sub_agent_reporter"),
            "meta + non-offensive sub-agents must be kept: {names:?}"
        );

        // enumeration permits scans → nothing stripped.
        let mut tools2 = vec![td("pentest_run")];
        hide_scans_for_zero_scan_stage(&mut tools2, Some(StageKind::Enumeration));
        assert_eq!(tools2.len(), 1, "scan-permitting stage must not hide scans");

        // no active stage → no-op.
        let mut tools3 = vec![td("pentest_run")];
        hide_scans_for_zero_scan_stage(&mut tools3, None);
        assert_eq!(tools3.len(), 1, "no stage → no filtering");
    }
    use golish_agent_kit::execution_mode::ExecutionMode;
    use golish_agent_kit::tool_definitions::{ToolPreset, ToolSelectionConfig};
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
            .tool_config(ToolSelectionConfig::with_preset(ToolPreset::None))
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

    /// A minimal registry tool used to prove the bridge allow-list path
    /// actually surfaces a dynamically-registered tool by name.
    struct MockNamedTool(&'static str);

    #[async_trait::async_trait]
    impl golish_core::Tool for MockNamedTool {
        fn name(&self) -> &'static str {
            self.0
        }
        fn description(&self) -> &'static str {
            "mock tool for tool-list wiring tests"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object", "properties": {} })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            _workspace: &std::path::Path,
        ) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!({ "status": "ok" }))
        }
    }

    /// End-to-end wiring guard: a registry tool named `submit_stage_deliverable`
    /// must reach the LLM tool list in task mode (both the depth-0 orchestrator
    /// and depth-1 specialists) via the bridge allow-list, but must NOT leak
    /// into chat mode (no active harness stage there).
    #[tokio::test]
    async fn submit_stage_deliverable_surfaces_in_task_not_chat() {
        async fn names_for(mode: ExecutionMode, depth: usize) -> Vec<String> {
            let test_ctx = TestContextBuilder::new().execution_mode(mode).build().await;
            {
                let mut reg = test_ctx.tool_registry.write().await;
                reg.register_tool(Arc::new(MockNamedTool("submit_stage_deliverable")));
            }
            let client = Arc::new(RwLock::new(LlmClient::Mock));
            let ctx = test_ctx.as_agentic_context_with_client(&client);
            let sub = SubAgentContext {
                depth,
                ..Default::default()
            };
            build_tool_list(&ctx, &sub)
                .await
                .into_iter()
                .map(|t| t.name)
                .collect()
        }

        let task_primary = names_for(ExecutionMode::Task, 0).await;
        assert!(
            task_primary.iter().any(|n| n == "submit_stage_deliverable"),
            "task primary (orchestrator) must expose submit_stage_deliverable; got: {task_primary:?}"
        );

        let task_subtask = names_for(ExecutionMode::Task, 1).await;
        assert!(
            task_subtask.iter().any(|n| n == "submit_stage_deliverable"),
            "task subtask (specialist) must expose submit_stage_deliverable; got: {task_subtask:?}"
        );

        let chat_primary = names_for(ExecutionMode::Chat, 0).await;
        assert!(
            !chat_primary.iter().any(|n| n == "submit_stage_deliverable"),
            "chat mode must NOT expose submit_stage_deliverable; got: {chat_primary:?}"
        );
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
