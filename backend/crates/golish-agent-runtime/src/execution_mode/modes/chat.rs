//! `ChatModePolicy` — single-agent conversational mode with the full toolbox.
//!
//! This policy is what finally gives the chat-mode LLM access to
//! `js_collect / manage_targets / run_pipeline / record_finding / vault /
//! flow_compose / js_extract_apis / auth_probe` (the eight `pentest_bridge`
//! tools that the legacy `tool.name.starts_with("pentest_")` filter was
//! silently dropping).

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
        // Chat mode is single-agent oriented but the user still wants
        // the LLM to be able to "phone a friend": e.g. ask
        // `sub_agent_browser` to do a JS bundle pull, or
        // `sub_agent_pentester` to run an active scan, then come back
        // with the result. Planner / refiner / reflector remain off —
        // those are task-mode orchestration concerns, not chat.
        ToolSelection {
            static_groups: StaticGroupSelection::all_enabled(),
            bridge_tools: BridgeToolSelection::all_enabled(),
            runtime_tools: RuntimeToolSelection {
                pentest_runtime: true,
                tavily: true,
            },
            agent_tools: AgentToolSelection {
                include_dispatch_tools: true,
                allow_planner: false,
                allow_refiner: false,
                allow_reflector: false,
            },
            include_run_command: true,
            include_ask_human: true,
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
        assert!(s.bridge_tools.run_pipeline);
        assert!(s.bridge_tools.auth_probe);
        assert!(s.bridge_tools.record_finding);
        assert!(s.bridge_tools.vault);
        assert!(s.bridge_tools.flow_compose);
        assert!(s.bridge_tools.js_extract_apis);
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
    async fn chat_dispatches_worker_sub_agents() {
        // Chat mode keeps `allows_sub_agents()` false because that
        // metadata is consumed by the picker UI (legacy contract:
        // chat is "single-agent" from the user's perspective). The
        // runtime still exposes worker-sub-agent dispatchers so the
        // chat-mode LLM can phone a friend when needed.
        let s = ChatModePolicy.primary_tools(&mock_ctx()).await;
        assert!(s.agent_tools.include_dispatch_tools);
        assert!(!s.agent_tools.allow_planner);
        assert!(!s.agent_tools.allow_refiner);
        assert!(!s.agent_tools.allow_reflector);
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
