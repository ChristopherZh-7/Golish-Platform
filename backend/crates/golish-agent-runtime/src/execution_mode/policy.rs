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
    pub record_finding: bool,
    pub vault: bool,
    pub run_pipeline: bool,
    pub flow_compose: bool,
    pub js_collect: bool,
    pub js_extract_apis: bool,
    pub auth_probe: bool,
}

impl BridgeToolSelection {
    pub const fn all_enabled() -> Self {
        Self {
            manage_targets: true,
            record_finding: true,
            vault: true,
            run_pipeline: true,
            flow_compose: true,
            js_collect: true,
            js_extract_apis: true,
            auth_probe: true,
        }
    }

    pub const fn none() -> Self {
        Self {
            manage_targets: false,
            record_finding: false,
            vault: false,
            run_pipeline: false,
            flow_compose: false,
            js_collect: false,
            js_extract_apis: false,
            auth_probe: false,
        }
    }

    /// Returns the static names of every flag that is currently enabled.
    /// Order is stable to keep prompt output deterministic.
    pub fn enabled_tool_names(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.manage_targets {
            out.push("manage_targets");
        }
        if self.record_finding {
            out.push("record_finding");
        }
        if self.vault {
            out.push("vault");
        }
        if self.run_pipeline {
            out.push("run_pipeline");
        }
        if self.flow_compose {
            out.push("flow_compose");
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
    fn bridge_all_enabled_lists_eight_tools_in_stable_order() {
        let names = BridgeToolSelection::all_enabled().enabled_tool_names();
        assert_eq!(
            names,
            vec![
                "manage_targets",
                "record_finding",
                "vault",
                "run_pipeline",
                "flow_compose",
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
    fn static_any_enabled_works() {
        assert!(StaticGroupSelection::all_enabled().any_enabled());
        assert!(!StaticGroupSelection::none().any_enabled());
        let mut g = StaticGroupSelection::none();
        g.file_ops = true;
        assert!(g.any_enabled());
    }
}
