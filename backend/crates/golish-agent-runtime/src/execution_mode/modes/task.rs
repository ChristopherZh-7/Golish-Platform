//! `TaskModePolicy` — multi-agent orchestration mode.
//!
//! - **Primary** (depth == 0): the LLM acts as project manager. It only
//!   sees `sub_agent_*` dispatchers + `ask_human`; no file ops, no shell,
//!   no pentest tools — those are the specialists' job.
//! - **Subtask** (depth > 0): the dispatched specialist sees the full
//!   toolbox minus `update_plan` (only the primary may rewrite the plan).

use async_trait::async_trait;

use crate::execution_mode::context::PolicyContext;
use crate::execution_mode::policy::{
    AgentToolSelection, BridgeToolSelection, ExecutionModePolicy, ModeLabel, RuntimeToolSelection,
    StaticGroupSelection, ToolSelection,
};

pub struct TaskModePolicy;

#[async_trait]
impl ExecutionModePolicy for TaskModePolicy {
    fn id(&self) -> &'static str {
        "task"
    }

    fn label(&self) -> ModeLabel {
        ModeLabel {
            display_name: "Task",
            icon: "Zap",
            badge_color: "magenta",
        }
    }

    fn description(&self) -> &'static str {
        "Auto: plan -> execute -> refine -> report (multi-agent orchestration)."
    }

    fn allows_sub_agents(&self) -> bool {
        true
    }

    async fn primary_tools(&self, _ctx: &PolicyContext<'_>) -> ToolSelection {
        ToolSelection {
            static_groups: StaticGroupSelection::none(),
            bridge_tools: BridgeToolSelection::none(),
            runtime_tools: RuntimeToolSelection::none(),
            agent_tools: AgentToolSelection {
                include_dispatch_tools: true,
                allow_planner: true,
                allow_refiner: false,   // pipeline-only, never exposed
                allow_reflector: false, // pipeline-only, never exposed
            },
            include_run_command: false,
            include_ask_human: true,
            deny_overrides: vec![],
        }
    }

    async fn subtask_tools(&self, _ctx: &PolicyContext<'_>) -> ToolSelection {
        ToolSelection {
            static_groups: StaticGroupSelection::all_enabled(),
            bridge_tools: BridgeToolSelection::all_enabled(),
            runtime_tools: RuntimeToolSelection {
                pentest_runtime: true,
                tavily: true,
            },
            agent_tools: AgentToolSelection::none(),
            include_run_command: true,
            include_ask_human: false,
            deny_overrides: vec!["update_plan".into()],
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
    async fn task_primary_only_dispatches() {
        let s = TaskModePolicy.primary_tools(&mock_ctx()).await;
        assert!(!s.bridge_tools.js_collect);
        assert!(!s.static_groups.file_ops);
        assert!(s.agent_tools.include_dispatch_tools);
        assert!(s.agent_tools.allow_planner);
        assert!(!s.agent_tools.allow_refiner);
        assert!(!s.agent_tools.allow_reflector);
        assert!(s.include_ask_human);
        assert!(!s.include_run_command);
    }

    #[tokio::test]
    async fn task_subtask_full_minus_update_plan() {
        let s = TaskModePolicy.subtask_tools(&mock_ctx()).await;
        assert!(s.bridge_tools.js_collect);
        assert!(s.static_groups.file_ops);
        assert!(s.deny_overrides.iter().any(|n| n == "update_plan"));
        assert!(!s.include_ask_human);
        assert!(s.include_run_command);
        assert!(!s.agent_tools.include_dispatch_tools);
    }

    #[tokio::test]
    async fn task_allows_sub_agents() {
        assert!(TaskModePolicy.allows_sub_agents());
    }
}
