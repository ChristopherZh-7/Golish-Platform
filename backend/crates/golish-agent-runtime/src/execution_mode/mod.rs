//! Execution mode policy framework.
//!
//! Each execution mode (`chat`, `task`, future `plan` / `debug` …) is a
//! [`policy::ExecutionModePolicy`] implementation that decides which tools
//! the LLM should see this turn. The agentic-loop's `tool_list::build_tool_list`
//! delegates entirely to the active policy via [`registry::ExecutionModeRegistry`].
//!
//! # Adding a new mode
//!
//! 1. Create `modes/<name>.rs` implementing `ExecutionModePolicy`.
//! 2. Add `pub mod <name>;` to `modes/mod.rs`.
//! 3. Register it in [`registry::ExecutionModeRegistry::default`].
//! 4. (Optional) Add a Tera template under `templates/<name>.tera` and
//!    wire it via PR3's `prompt_render`.
//!
//! No edits to `tool_list.rs` are required — that is the entire point of
//! this abstraction.

pub mod context;
pub mod modes;
pub mod policy;
pub mod registry;
pub mod selection_apply;

pub use context::PolicyContext;
pub use policy::{
    AgentToolSelection, BridgeToolSelection, ExecutionModePolicy, ModeLabel, RuntimeToolSelection,
    StaticGroupSelection, ToolSelection,
};
pub use registry::ExecutionModeRegistry;
