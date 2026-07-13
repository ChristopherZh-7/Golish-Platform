// Commands module for AI agent interaction.
//
// This module provides Tauri command handlers for the AI agent system,
// organized into logical submodules for maintainability.

pub mod agents;
pub mod analytics;
pub mod attack;
pub mod cleanup;
pub mod config;
pub mod context;
pub mod core;
pub mod debug;
pub mod dispatch;
pub mod graph;
pub mod harness_dev;
pub mod hitl;
pub mod loop_detection;
pub mod mode;
pub mod plan;
pub mod policy;
pub mod reporting;
pub mod session;
pub mod stage_coverage;
pub mod summarizer;
pub mod temporal_graph;
pub mod workflow;

mod bridge_config;

// Re-export all commands for easier access
pub use agents::*;
pub use analytics::*;
pub use attack::*;
pub use cleanup::*;
pub use config::*;
pub use context::*;
pub use core::*;
pub use debug::*;
pub use dispatch::*;
pub use graph::*;
pub use harness_dev::*;
pub use hitl::*;
pub use loop_detection::*;
pub use mode::*;
pub use plan::*;
pub use policy::*;
pub use reporting::*;
pub use session::*;
pub use stage_coverage::*;
pub use summarizer::*;
pub use temporal_graph::*;
pub use workflow::*;

// Bridge wiring lives in `bridge_config`; re-export at the previous paths.
// `setup_bridge_mcp_tools` + `McpManagerToolExecutor` are `pub` (not
// `pub(crate)`) because the main `golish` crate reaches them across the crate
// boundary via its `crate::ai` shim (app/mcp_bootstrap, mcp/commands,
// cli/bootstrap) now that the agent command surface lives here (M4-proper).
pub(crate) use bridge_config::{
    activate_bridge_background_listeners, prepare_bridge_background_listeners,
};
pub use bridge_config::{configure_bridge, configure_bridge_background_listeners};
pub use bridge_config::{setup_bridge_mcp_tools, McpManagerToolExecutor};

// `AiState` + the agent error helpers live in this crate's `state` module
// (crate-per-service M4-A). Re-export here so existing
// `crate::ai::commands::*` / `crate::ai::AiState` paths resolve.
pub use crate::{ai_session_not_initialized_error, AiState};
