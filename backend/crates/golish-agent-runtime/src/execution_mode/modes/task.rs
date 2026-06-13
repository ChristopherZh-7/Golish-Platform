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
        // Task primary (depth==0) is orchestration-only. The legacy
        // `tool_list.rs` explicitly filtered out four "internal"
        // sub-agents from the dispatch list: orchestrator (always
        // pipeline-only), planner, refiner, reflector. We keep the
        // same shape — only the worker specialists (pentester / browser
        // / coder / researcher / memorist / installer / adviser /
        // reporter / enricher) are exposed to the primary LLM.
        ToolSelection {
            static_groups: StaticGroupSelection::none(),
            // The orchestrator is dispatch-only EXCEPT for the harness
            // deliverable channel: when it delegates report-writing to a
            // specialist it must still be able to take that result and call
            // `submit_stage_deliverable` itself (design §4.1), otherwise the
            // stage gate has nothing to validate and the cursor never advances.
            bridge_tools: BridgeToolSelection {
                submit_stage_deliverable: true,
                // D1=B (设计 2026-06-04): the lead decision turn is gone — task mode
                // enters the harness directly, so the depth-0 primary runs *inside*
                // a stage and must NOT see `start_operation` (no nested operations).
                start_operation: false,
                // Engagement-scope bookkeeping is orchestration-level work the
                // depth-0 primary must do itself, not specialist scanning. The
                // scoping gate HARD-requires manage_organizations(create) (+ the
                // red_team unit-candidate flow via ask_human(unit_review), already a
                // depth-0 tool), and target_intel needs the passive recon_* tools.
                // No specialist sub-agent carries manage_organizations, so without
                // these at depth 0 the scoping gate is unsatisfiable and the stage
                // dead-loops on BLOCK→retry (设计 2026-06-06-scoping-per-mode-gate-hitl
                // §3.4 + 2026-06-06-intel-stage-ai-driven-per-mode §3.5).
                manage_organizations: true,
                manage_targets: true,
                recon_discover_subsidiaries: true,
                recon_enrich_assets: true,
                recon_lookup_company: true,
                recon_list_providers: true,
                ..BridgeToolSelection::none()
            },
            runtime_tools: RuntimeToolSelection::none(),
            agent_tools: AgentToolSelection {
                include_dispatch_tools: true,
                allow_planner: false,
                allow_refiner: false,
                allow_reflector: false,
            },
            include_run_command: false,
            include_ask_human: true,
            // The depth-0 primary runs each harness stage as its own agentic
            // loop and self-manages that stage's todo list via `update_plan`.
            // It has no static groups, so surface this one tool explicitly.
            include_update_plan: true,
            deny_overrides: vec![],
        }
    }

    async fn subtask_tools(&self, _ctx: &PolicyContext<'_>) -> ToolSelection {
        // Task subtask (depth>0) gets the full toolbox so a specialist
        // can do real work, *and* keeps the sub-agent dispatch tools
        // so it can delegate further (e.g. `pentester` may need to
        // call `coder` / `researcher` / `memorist` / `installer` /
        // `enricher` / `browser`). Planner / refiner / reflector are
        // intentionally still allowed at this depth — the legacy
        // tool_list only ring-fenced them at the primary layer.
        ToolSelection {
            static_groups: StaticGroupSelection::all_enabled(),
            // Full bridge toolbox PLUS the harness deliverable channel so a
            // specialist (e.g. the reporter at depth 1) can submit the
            // StageDeliverable directly via `submit_stage_deliverable`.
            bridge_tools: BridgeToolSelection {
                submit_stage_deliverable: true,
                ..BridgeToolSelection::all_enabled()
            },
            runtime_tools: RuntimeToolSelection {
                pentest_runtime: true,
                tavily: true,
            },
            agent_tools: AgentToolSelection {
                include_dispatch_tools: true,
                allow_planner: true,
                allow_refiner: true,
                allow_reflector: true,
            },
            include_run_command: true,
            include_ask_human: false,
            // Only the primary owns the plan; specialists never rewrite it.
            include_update_plan: false,
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
    async fn task_primary_only_dispatches_workers() {
        let s = TaskModePolicy.primary_tools(&mock_ctx()).await;
        assert!(!s.bridge_tools.js_collect);
        assert!(!s.static_groups.file_ops);
        // The orchestrator still gets the harness deliverable channel so it can
        // submit the StageDeliverable itself after delegating report-writing.
        assert!(
            s.bridge_tools.submit_stage_deliverable,
            "task primary must expose submit_stage_deliverable (design §4.1)"
        );
        // Engagement-scope bookkeeping tools must be reachable by the depth-0
        // primary: the scoping gate hard-requires manage_organizations(create)
        // and target_intel needs the passive recon_* tools. No specialist
        // sub-agent carries manage_organizations, so omitting these here makes
        // the red_team scoping gate unsatisfiable (regression guard 2026-06-07).
        assert!(
            s.bridge_tools.manage_organizations,
            "task primary must expose manage_organizations or the scoping gate dead-loops"
        );
        assert!(s.bridge_tools.manage_targets);
        assert!(s.bridge_tools.recon_discover_subsidiaries);
        assert!(s.bridge_tools.recon_enrich_assets);
        assert!(
            s.bridge_tools.recon_lookup_company,
            "task primary must expose recon_lookup_company or scoping 纠名 (Phase A) is unsatisfiable"
        );
        assert!(s.bridge_tools.recon_list_providers);
        assert!(s.agent_tools.include_dispatch_tools);
        // Legacy parity: the four "internal" sub-agents are filtered
        // out at the primary layer.
        assert!(!s.agent_tools.allow_planner);
        assert!(!s.agent_tools.allow_refiner);
        assert!(!s.agent_tools.allow_reflector);
        assert!(s.include_ask_human);
        assert!(!s.include_run_command);
        // The primary runs each harness stage as its own agentic loop and
        // self-manages that stage's todo list, so it must expose update_plan
        // despite being otherwise orchestration-only (no static groups).
        assert!(
            s.include_update_plan,
            "task primary must expose update_plan for stage todo self-management"
        );
    }

    #[tokio::test]
    async fn task_subtask_full_with_dispatch() {
        let s = TaskModePolicy.subtask_tools(&mock_ctx()).await;
        assert!(s.bridge_tools.js_collect);
        assert!(s.static_groups.file_ops);
        // Specialists (e.g. the reporter) must be able to submit the deliverable.
        assert!(
            s.bridge_tools.submit_stage_deliverable,
            "task subtask must expose submit_stage_deliverable for the reporter"
        );
        assert!(s.deny_overrides.iter().any(|n| n == "update_plan"));
        assert!(!s.include_update_plan, "only the primary owns the plan");
        assert!(!s.include_ask_human);
        assert!(s.include_run_command);
        // Subtask agents must be able to delegate further so the
        // legacy chain pentester -> coder / researcher / browser
        // continues to work.
        assert!(s.agent_tools.include_dispatch_tools);
        assert!(s.agent_tools.allow_planner);
        assert!(s.agent_tools.allow_refiner);
        assert!(s.agent_tools.allow_reflector);
    }

    #[tokio::test]
    async fn task_allows_sub_agents() {
        assert!(TaskModePolicy.allows_sub_agents());
    }
}
