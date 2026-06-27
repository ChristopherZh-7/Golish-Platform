//! Read-only context passed to [`super::policy::ExecutionModePolicy`]
//! methods.
//!
//! The agentic loop owns a heavy [`crate::agentic_loop::AgenticLoopContext`]
//! with locks, registries, channels, and provider clients. Policies should
//! not need any of that — they only need a few facts about the current
//! turn (workspace, agent_mode, depth, whether sub-agents are user-enabled).
//! `PolicyContext` is that narrowed surface.

use std::path::Path;

use golish_core::AgentMode;

pub struct PolicyContext<'a> {
    pub agent_mode: AgentMode,
    pub workspace: &'a Path,
    pub mcp_tool_count: usize,
    pub depth: usize,
    pub harness_active: bool,
}

impl<'a> PolicyContext<'a> {
    pub fn new(workspace: &'a Path, agent_mode: AgentMode) -> Self {
        Self {
            agent_mode,
            workspace,
            mcp_tool_count: 0,
            depth: 0,
            harness_active: false,
        }
    }

    pub fn with_depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    pub fn with_mcp_tool_count(mut self, count: usize) -> Self {
        self.mcp_tool_count = count;
        self
    }

    pub fn with_harness_active(mut self, active: bool) -> Self {
        self.harness_active = active;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_chaining_sets_all_fields() {
        let ws = Path::new("/tmp");
        let ctx = PolicyContext::new(ws, AgentMode::default())
            .with_depth(3)
            .with_mcp_tool_count(5)
            .with_harness_active(true);
        assert_eq!(ctx.depth, 3);
        assert_eq!(ctx.mcp_tool_count, 5);
        assert!(ctx.harness_active);
        assert_eq!(ctx.workspace, ws);
    }
}
