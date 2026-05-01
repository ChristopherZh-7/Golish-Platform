//! Backward-compatibility umbrella crate.
//!
//! After **A1-2** of the architecture upgrade plan, the agentic loop, tool
//! execution, task orchestration, HITL approval, tool policy, planner,
//! db tracking, and llm_client subsystems were extracted into the new
//! [`golish-agent-loop`](https://docs.rs/golish-agent-loop) crate. The
//! prompt subsystem moved to `golish-prompts` in **A1-1**.
//!
//! This crate continues to expose the original module paths
//! (`golish_ai::agentic_loop::*`, `golish_ai::tool_executors::*`, …) by
//! re-exporting from the extracted crates so existing callers keep
//! compiling unchanged.
//!
//! # Layering after A1-2
//!
//! - **`golish-prompts`** (A1-1): prompt composition + summarisation
//! - **`golish-agent-loop`** (A1-2): agentic runtime
//! - **`golish-ai`** (this crate): umbrella facade + remaining bridge
//!   layer (`agent_bridge`, `bridge_*`, `task_orchestrator::bridge_executor`)
//!   pending **A1-3** extraction into `golish-agent-bridge`.
//!
//! New code should depend directly on `golish-agent-loop` /
//! `golish-prompts`. Importing from `golish_ai::*` will start emitting
//! `#[deprecated]` warnings once A1-4 lands.

#![allow(deprecated)]

pub use golish_agent_loop::{
    agent_mode, agentic_loop, db_shim, db_traits, db_tracking, eval_support, execution_mode, hitl,
    llm_client, loop_detection, memory_file, memory_gatekeeper, planner, sidecar_trait,
    system_hooks, tool_definitions, tool_execution, tool_executors, tool_policy,
    tool_provider_impl,
};

pub mod agent_bridge;
mod bridge_context;
mod bridge_hitl;
mod bridge_policy;
mod bridge_session;

pub mod codex_prompt;
pub mod contributors;
pub mod prompt_registry;
pub mod summarizer;
pub mod system_prompt;

pub(crate) use golish_events::event_coordinator;
pub(crate) use golish_events::transcript;

/// Task orchestration facade.
///
/// Re-exports everything that has moved into `golish-agent-loop` while
/// keeping `bridge_executor` as a `golish-ai`-owned submodule (it
/// depends on `agent_bridge` and will move out together in A1-3).
pub mod task_orchestrator {
    pub use golish_agent_loop::task_orchestrator::*;

    pub mod bridge_executor;
}

pub use agentic_loop::{OutputClassifier, PostShellHook};
pub use golish_agent_loop::AgentMode;
pub use golish_agent_loop::{
    get_all_tool_definitions_with_config, get_tool_definitions_for_preset,
    get_tool_definitions_with_config, ToolConfig, ToolPreset,
};
pub use golish_agent_loop::{
    normalize_run_pty_cmd_args, route_tool_execution, ToolExecutionConfig, ToolExecutionContext,
    ToolExecutionError, ToolExecutionResult, ToolRoutingCategory, ToolSource,
};
pub use golish_agent_loop::DefaultToolProvider;
pub use golish_agent_loop::SharedComponentsConfig;
pub use golish_events::{
    build_summarizer_input, format_for_summarizer, read_transcript, save_summarizer_input,
    save_summary, transcript_path, CoordinatorHandle, CoordinatorState, EventCoordinator,
    TranscriptEvent, TranscriptWriter,
};
pub use prompt_registry::PromptContributorRegistry;
pub use summarizer::{
    build_summarizer_user_prompt, generate_summary, SummaryResponse, SUMMARIZER_SYSTEM_PROMPT,
};
