//! `ExecutionModePolicy` — per-execution-mode tool exposure strategy.
//!
//! Each execution mode (`chat`, `task`, future `plan` / `debug` …) has one
//! `Policy` implementation that declares **which tools should be visible to
//! the LLM** for the current turn. The agentic-loop calls
//! [`crate::agentic_loop::tool_list::build_tool_list`] which delegates the
//! decision entirely to the active policy.
//!
//! The policy returns a [`ToolSelection`]: a structured set of "named groups
//! of tools to enable", **not** a list of tool name strings. Adding a new
//! `pentest_bridge` tool is therefore one boolean flag here, plus one entry
//! in `BridgeToolSelection::all_enabled` and the registry — no string
//! filters scattered across the codebase.

use async_trait::async_trait;

use super::context::PolicyContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeLabel {
    pub display_name: &'static str,
    pub icon: &'static str,
    pub badge_color: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolSelection {
    pub static_groups: StaticGroupSelection,
    pub bridge_tools: BridgeToolSelection,
    pub runtime_tools: RuntimeToolSelection,
    pub agent_tools: AgentToolSelection,
    pub include_run_command: bool,
    pub include_ask_human: bool,
    /// Targeted opt-in for the `update_plan` planning/todo tool **without**
    /// pulling in the rest of a static group. The task-mode depth-0 primary
    /// runs each harness stage as its own agentic loop and self-manages the
    /// stage's todo list via `update_plan`, but is otherwise orchestration-only
    /// (`static_groups::none()`), so it needs this single tool surfaced on its
    /// own. Chat mode already gets `update_plan` via `static_groups`, so it
    /// leaves this `false` to avoid a duplicate.
    pub include_update_plan: bool,
    /// Tool names to forcibly exclude after positive selection has been
    /// applied. Use sparingly — the primary mechanism for "don't expose X"
    /// should be flipping the corresponding `bool` to `false` above.
    pub deny_overrides: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StaticGroupSelection {
    pub file_ops: bool,
    pub core: bool,
    pub memory: bool,
    pub knowledge_base: bool,
    pub security_analysis: bool,
    pub graph: bool,
    pub sploitus: bool,
}

impl StaticGroupSelection {
    pub const fn all_enabled() -> Self {
        Self {
            file_ops: true,
            core: true,
            memory: true,
            knowledge_base: true,
            security_analysis: true,
            graph: true,
            sploitus: true,
        }
    }

    pub const fn none() -> Self {
        Self {
            file_ops: false,
            core: false,
            memory: false,
            knowledge_base: false,
            security_analysis: false,
            graph: false,
            sploitus: false,
        }
    }

    pub fn any_enabled(&self) -> bool {
        self.file_ops
            || self.core
            || self.memory
            || self.knowledge_base
            || self.security_analysis
            || self.graph
            || self.sploitus
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BridgeToolSelection {
    pub manage_targets: bool,
    pub manage_organizations: bool,
    /// `recon_discover_subsidiaries` — passive ENScan subsidiary discovery
    /// (harness target_intel, 设计 2026-06-06-intel-stage-ai-driven-per-mode).
    pub recon_discover_subsidiaries: bool,
    /// `recon_map_assets` — cyberspace/intel-provider survey (0.zone / quake / …).
    pub recon_map_assets: bool,
    /// `recon_lookup_whois` — standalone WHOIS-via-RDAP lookup (per-org).
    pub recon_lookup_whois: bool,
    /// `recon_lookup_company` — scoping 纠名 step 1: resolve raw company names
    /// to canonical registered names before creating organizations
    /// (设计 2026-06-13-engagement-scoping-fanout §6.2).
    pub recon_lookup_company: bool,
    /// `recon_list_providers` — read-only: which passive providers have a
    /// configured credential (so the AI invokes only usable ones).
    pub recon_list_providers: bool,
    pub record_finding: bool,
    pub vault: bool,
    pub js_collect: bool,
    pub js_extract_apis: bool,
    pub auth_probe: bool,
    /// `submit_stage_deliverable` — the deterministic harness deliverable
    /// channel. Deliberately NOT part of [`Self::all_enabled`]: it is only
    /// meaningful while a harness stage is active (task mode), so chat mode
    /// must never see it. Task mode turns it on explicitly for both the
    /// orchestrator (depth 0) and the specialists (depth > 0).
    pub submit_stage_deliverable: bool,
    /// `start_operation` — control-plane handoff: the lead orchestrator turn
    /// calls this to begin the structured multi-stage planner. Task-primary
    /// (depth 0) only; never exposed in chat or to subtask specialists.
    pub start_operation: bool,
}

impl BridgeToolSelection {
    pub const fn all_enabled() -> Self {
        Self {
            manage_targets: true,
            manage_organizations: true,
            recon_discover_subsidiaries: true,
            recon_map_assets: true,
            recon_lookup_whois: true,
            recon_lookup_company: true,
            recon_list_providers: true,
            record_finding: true,
            vault: true,
            js_collect: true,
            js_extract_apis: true,
            auth_probe: true,
            // Harness-stage-only; opted into per-mode (task) rather than via the
            // generic "all bridge tools" set so chat mode never exposes it.
            submit_stage_deliverable: false,
            start_operation: false,
        }
    }

    pub const fn none() -> Self {
        Self {
            manage_targets: false,
            manage_organizations: false,
            recon_discover_subsidiaries: false,
            recon_map_assets: false,
            recon_lookup_whois: false,
            recon_lookup_company: false,
            recon_list_providers: false,
            record_finding: false,
            vault: false,
            js_collect: false,
            js_extract_apis: false,
            auth_probe: false,
            submit_stage_deliverable: false,
            start_operation: false,
        }
    }

    /// Returns the static names of every flag that is currently enabled.
    /// Order is stable to keep prompt output deterministic.
    pub fn enabled_tool_names(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.manage_targets {
            out.push("manage_targets");
        }
        if self.manage_organizations {
            out.push("manage_organizations");
        }
        if self.recon_discover_subsidiaries {
            out.push("recon_discover_subsidiaries");
        }
        if self.recon_map_assets {
            out.push("recon_map_assets");
        }
        if self.recon_lookup_whois {
            out.push("recon_lookup_whois");
        }
        if self.recon_lookup_company {
            out.push("recon_lookup_company");
        }
        if self.recon_list_providers {
            out.push("recon_list_providers");
        }
        if self.record_finding {
            out.push("record_finding");
        }
        if self.vault {
            out.push("vault");
        }
        if self.js_collect {
            out.push("js_collect");
        }
        if self.js_extract_apis {
            out.push("js_extract_apis");
        }
        if self.auth_probe {
            out.push("auth_probe");
        }
        if self.submit_stage_deliverable {
            out.push("submit_stage_deliverable");
        }
        if self.start_operation {
            out.push("start_operation");
        }
        out
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeToolSelection {
    pub pentest_runtime: bool,
    pub tavily: bool,
}

impl RuntimeToolSelection {
    pub const fn none() -> Self {
        Self {
            pentest_runtime: false,
            tavily: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgentToolSelection {
    pub include_dispatch_tools: bool,
    pub allow_planner: bool,
    pub allow_refiner: bool,
    pub allow_reflector: bool,
}

impl AgentToolSelection {
    pub const fn none() -> Self {
        Self {
            include_dispatch_tools: false,
            allow_planner: false,
            allow_refiner: false,
            allow_reflector: false,
        }
    }
}

#[async_trait]
pub trait ExecutionModePolicy: Send + Sync + 'static {
    fn id(&self) -> &'static str;
    fn label(&self) -> ModeLabel;
    fn description(&self) -> &'static str;
    fn allows_sub_agents(&self) -> bool {
        false
    }

    async fn primary_tools(&self, ctx: &PolicyContext<'_>) -> ToolSelection;

    async fn subtask_tools(&self, ctx: &PolicyContext<'_>) -> ToolSelection {
        self.primary_tools(ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_all_enabled_lists_tools_in_stable_order() {
        let names = BridgeToolSelection::all_enabled().enabled_tool_names();
        assert_eq!(
            names,
            vec![
                "manage_targets",
                "manage_organizations",
                "recon_discover_subsidiaries",
                "recon_map_assets",
                "recon_lookup_whois",
                "recon_lookup_company",
                "recon_list_providers",
                "record_finding",
                "vault",
                "js_collect",
                "js_extract_apis",
                "auth_probe",
            ]
        );
    }

    #[test]
    fn bridge_none_yields_empty_list() {
        assert!(BridgeToolSelection::none().enabled_tool_names().is_empty());
    }

    #[test]
    fn submit_stage_deliverable_excluded_from_all_enabled_but_listable() {
        // Regression guard: the harness deliverable tool must NOT leak into the
        // generic "all bridge tools" set (otherwise chat mode would expose it
        // with no active stage). It only surfaces when a mode opts in explicitly.
        assert!(!BridgeToolSelection::all_enabled().submit_stage_deliverable);
        assert!(!BridgeToolSelection::all_enabled()
            .enabled_tool_names()
            .contains(&"submit_stage_deliverable"));

        let sel = BridgeToolSelection {
            submit_stage_deliverable: true,
            ..BridgeToolSelection::none()
        };
        assert_eq!(
            sel.enabled_tool_names(),
            vec!["submit_stage_deliverable"],
            "opting the flag in must surface exactly that tool name for the allow-list"
        );
    }

    #[test]
    fn static_any_enabled_works() {
        assert!(StaticGroupSelection::all_enabled().any_enabled());
        assert!(!StaticGroupSelection::none().any_enabled());
        let mut g = StaticGroupSelection::none();
        g.file_ops = true;
        assert!(g.any_enabled());
    }
}
