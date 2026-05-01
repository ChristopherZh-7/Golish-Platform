//! Shared tool execution and routing.
//!
//! Unified tool routing for all agent implementations, eliminating
//! duplication between main agent loops and sub-agent execution.
//!
//! ## Layout
//!
//! - [`hitl`]: configuration and context types — `ToolExecutionConfig`,
//!   `ToolSource`, `ToolExecutionContext`. The "human-in-the-loop" gating
//!   knobs live here (require_hitl, allow_sub_agents).
//! - [`direct`]: the actual execution helpers — `ToolExecutionResult`,
//!   `ToolExecutionError`, `execute_registry_tool`,
//!   `normalize_run_pty_cmd_args`, plus the placeholder handlers for
//!   web_fetch, update_plan, and sub_agent_*.
//! - [`route`]: the routing dispatcher — `ToolRoutingCategory` +
//!   `route_tool_execution`. Picks which `direct` helper to call.
//!
//! ## Tool categories
//!
//! Tools are routed based on their name prefix:
//! - `web_fetch` — Web content fetching with readability extraction.
//! - `web_search*`, `web_extract` — Tavily web search tools.
//! - `update_plan` — Task planning updates.
//! - `sub_agent_*` — Sub-agent delegation (main agent only).
//! - `run_command` — Alias for `run_pty_cmd`.
//! - Everything else — Standard registry-based tools.
//!
//! ## Usage
//!
//! ```ignore
//! use golish_ai::tool_execution::{route_tool_execution, ToolExecutionConfig, ToolSource};
//!
//! let config = ToolExecutionConfig {
//!     require_hitl: true,
//!     source: ToolSource::MainAgent,
//!     allow_sub_agents: true,
//! };
//!
//! let result = route_tool_execution(tool_name, &tool_args, &ctx, &config).await?;
//! ```

mod direct;
mod hitl;
mod route;

#[cfg(test)]
mod tests;

pub use direct::{
    normalize_run_pty_cmd_args, ToolExecutionError, ToolExecutionResult,
};
pub use hitl::{ToolExecutionConfig, ToolExecutionContext, ToolSource};
pub use route::{route_tool_execution, ToolRoutingCategory};
