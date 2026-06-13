//! Bridge layer between the Tauri application and the agent runtime.
//!
//! Extracted from `golish-ai` in **A1-3** of the architecture upgrade plan.
//! Owns the **`AgentBridge`** struct (lifecycle + dispatch) plus the
//! `bridge_*` companions and the `bridge_executor` orchestrator
//! implementation that depends on `AgentBridge`.
//!
//! # Layering
//!
//! - depends on: `golish-agent-kit` (runtime), `golish-prompts`,
//!   `golish-events`, `golish-context`, `golish-sub-agents`,
//!   `golish-llm-providers`, `golish-tools`, `golish-session`,
//!   `golish-indexer`
//! - consumed by: `golish-ai` umbrella (facade for backward compat)
//!   and the `golish` Tauri application.
//!
//! # Internal aliases
//!
//! Code migrated from `golish-ai` references many `crate::xxx` paths
//! (e.g. `crate::agentic_loop`, `crate::transcript`,
//! `crate::contributors`). Instead of rewriting every site, this crate
//! re-exports those modules at the crate root so the existing import
//! paths keep working unchanged.

pub use golish_agent_kit::{
    agent_mode, db_shim, db_tracking, db_traits, execution_mode, hitl, llm_client, loop_detection,
    memory_file, memory_gatekeeper, planner, sidecar_trait, system_hooks, tool_definitions,
    tool_execution, tool_executors, tool_policy, tool_provider_impl,
};
pub use golish_agent_runtime::agentic_loop;

pub use golish_prompts::{prompt_registry, system_prompt};

pub(crate) use golish_events::event_coordinator;
pub(crate) use golish_events::transcript;

pub mod agent_bridge;
pub mod bridge_executor;
pub mod contributors;

mod bridge_context;
mod bridge_hitl;
mod bridge_policy;
mod bridge_session;

pub use agent_bridge::{AgentBridge, BridgeBackends, EngagementWorkerScope};
