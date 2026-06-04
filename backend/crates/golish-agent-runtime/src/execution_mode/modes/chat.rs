//! `ChatModePolicy` — single-agent conversational mode with the full toolbox.
//!
//! This policy is what finally gives the chat-mode LLM access to
//! `js_collect / manage_targets / record_finding / vault / js_extract_apis /
//! auth_probe` (the `pentest_bridge` tools that the legacy
//! `tool.name.starts_with("pentest_")` filter was silently dropping).
//! Note: `run_pipeline` / `flow_compose` (the pipeline tools) are intentionally
//! NOT exposed to agents.

use async_trait::async_trait;

use crate::execution_mode::context::PolicyContext;
use crate::execution_mode::policy::{
    AgentToolSelection, BridgeToolSelection, ExecutionModePolicy, ModeLabel, RuntimeToolSelection,
    StaticGroupSelection, ToolSelection,
};

pub struct ChatModePolicy;

#[async_trait]
impl ExecutionModePolicy for ChatModePolicy {
    fn id(&self) -> &'static str {
        "chat"
    }

    fn label(&self) -> ModeLabel {
        ModeLabel {
            display_name: "Chat",
            icon: "MessageSquare",
            badge_color: "muted",
        }
    }

    fn description(&self) -> &'static str {
        "Conversational single-agent mode with the full toolbox."
    }

    async fn primary_tools(&self, _ctx: &PolicyContext<'_>) -> ToolSelection {
        // Chat mode is strictly single-agent: the LLM has the full
        // direct toolbox (files / shell / pentest_bridge / pentest_runtime
        // / tavily / memory / knowledge / graph / sploitus / ask_human)
        // and is expected to solve one task in one turn without
        // delegating to specialists. Multi-agent orchestration belongs
        // to task mode.
        ToolSelection {
            static_groups: StaticGroupSelection::all_enabled(),
            bridge_tools: BridgeToolSelection::all_enabled(),
            runtime_tools: RuntimeToolSelection {
                pentest_runtime: true,
                tavily: true,
            },
            agent_tools: AgentToolSelection::none(),
            include_run_command: true,
            include_ask_human: true,
            // Chat already gets `update_plan` via the full static groups above;
            // no targeted opt-in needed (avoids a duplicate definition).
            include_update_plan: false,
            deny_overrides: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn mock_ctx() -> PolicyContext<'static> {
        PolicyContext::new(Path::new("/tmp"), golish_core::AgentMode::default())
    }

    #[tokio::test]
    async fn chat_primary_includes_js_collect() {
        let s = ChatModePolicy.primary_tools(&mock_ctx()).await;
        assert!(
            s.bridge_tools.js_collect,
            "chat must expose js_collect (regression guard for the bug fixed in PR2)"
        );
        assert!(s.bridge_tools.manage_targets);
        // pipeline tools are intentionally NOT exposed to agents
        assert!(!s.bridge_tools.run_pipeline);
        assert!(!s.bridge_tools.flow_compose);
        assert!(s.bridge_tools.auth_probe);
        assert!(s.bridge_tools.record_finding);
        assert!(s.bridge_tools.vault);
        assert!(s.bridge_tools.js_extract_apis);
        // The harness deliverable channel is task-mode-only: chat has no active
        // stage, so it must never expose submit_stage_deliverable.
        assert!(
            !s.bridge_tools.submit_stage_deliverable,
            "chat mode must NOT expose the harness-only submit_stage_deliverable"
        );
    }

    #[tokio::test]
    async fn chat_primary_full_static_groups() {
        let s = ChatModePolicy.primary_tools(&mock_ctx()).await;
        assert!(s.static_groups.file_ops);
        assert!(s.static_groups.security_analysis);
        assert!(s.static_groups.graph);
        assert!(s.static_groups.sploitus);
    }

    #[tokio::test]
    async fn chat_does_not_dispatch_sub_agents() {
        // Product decision (2026-05): chat mode is strictly
        // single-agent. The LLM uses tools directly to answer one
        // question; multi-agent orchestration is the job of task
        // mode. allows_sub_agents() reports the same intent at the
        // metadata level for the picker UI.
        let s = ChatModePolicy.primary_tools(&mock_ctx()).await;
        assert!(!s.agent_tools.include_dispatch_tools);
        assert!(!s.agent_tools.allow_planner);
        assert!(!s.agent_tools.allow_refiner);
        assert!(!s.agent_tools.allow_reflector);
        assert!(!ChatModePolicy.allows_sub_agents());
    }

    #[tokio::test]
    async fn chat_includes_run_command_and_ask_human() {
        let s = ChatModePolicy.primary_tools(&mock_ctx()).await;
        assert!(s.include_run_command);
        assert!(s.include_ask_human);
    }

    #[tokio::test]
    async fn chat_subtask_inherits_primary() {
        // ChatMode is single-agent; even if asked for subtask shape, it
        // returns the primary selection unchanged.
        let primary = ChatModePolicy.primary_tools(&mock_ctx()).await;
        let subtask = ChatModePolicy.subtask_tools(&mock_ctx()).await;
        assert_eq!(primary, subtask);
    }
}
