//! Tool execution helpers used by the agentic loop.
//!
//! - [`direct`]: `execute_tool_direct_generic` (auto-approved path) plus
//!   the private sub-agent dispatch helper.
//! - [`hitl`]: `execute_with_hitl_generic` which wraps the direct path
//!   with approval prompting.

mod direct;
mod hitl;

pub use direct::execute_tool_direct_generic;
pub use hitl::execute_with_hitl_generic;
