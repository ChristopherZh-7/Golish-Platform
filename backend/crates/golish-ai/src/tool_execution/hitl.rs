//! Human-in-the-loop gating types: how a tool call is described, who it
//! comes from, what dependencies it needs, and whether approval is required.
//!
//! This module is deliberately *I/O-free*: it just owns the configuration
//! and context shapes consumed by the routing layer (see [`super::route`])
//! and the direct execution layer (see [`super::direct`]).

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::indexer::IndexerState;
use crate::planner::PlanManager;
use golish_sub_agents::SubAgentRegistry;
use golish_tools::ToolRegistry;

/// Configuration for tool execution behavior.
#[derive(Debug, Clone)]
pub struct ToolExecutionConfig {
    /// Whether HITL approval is required (false for trusted sub-agents).
    pub require_hitl: bool,
    /// Source identifier for logging and event emission.
    pub source: ToolSource,
    /// Whether sub-agent tools are allowed (false for sub-agents to prevent nesting).
    pub allow_sub_agents: bool,
}

impl Default for ToolExecutionConfig {
    fn default() -> Self {
        Self {
            require_hitl: true,
            source: ToolSource::MainAgent,
            allow_sub_agents: true,
        }
    }
}

/// Identifies the source of a tool execution for logging and events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSource {
    /// Tool called from the main agent loop.
    MainAgent,
    /// Tool called from a sub-agent.
    SubAgent {
        /// Sub-agent identifier.
        name: String,
        /// Current nesting depth.
        depth: u32,
    },
}

impl ToolSource {
    /// Create a sub-agent source.
    pub fn sub_agent(name: impl Into<String>, depth: u32) -> Self {
        Self::SubAgent {
            name: name.into(),
            depth,
        }
    }

    /// Check if this is from the main agent.
    pub fn is_main_agent(&self) -> bool {
        matches!(self, Self::MainAgent)
    }
}

/// Context providing access to tool execution dependencies.
///
/// This struct holds references to all the state and services needed for
/// tool execution, allowing the routing logic to be decoupled from specific
/// agent implementations.
pub struct ToolExecutionContext<'a> {
    /// Tool registry for standard tool execution.
    pub tool_registry: &'a Arc<RwLock<ToolRegistry>>,
    /// Sub-agent registry (only used if `allow_sub_agents` is true).
    pub sub_agent_registry: &'a Arc<RwLock<SubAgentRegistry>>,
    /// Indexer state for code search tools (optional).
    pub indexer_state: Option<&'a Arc<IndexerState>>,
    /// Plan manager for the `update_plan` tool.
    pub plan_manager: &'a Arc<PlanManager>,
    /// Current workspace path.
    pub workspace: &'a Arc<RwLock<std::path::PathBuf>>,
}
