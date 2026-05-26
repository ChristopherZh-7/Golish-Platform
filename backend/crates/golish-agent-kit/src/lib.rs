// These lints are intentionally allowed across the crate:
// - `too_many_arguments`: many DB / agent helpers thread a wide context
//   through the call stack; redesigning each into a struct is out of scope.
// - `needless_borrow`: tool-trait methods accept `&T` parameters that callers
//   pass through unchanged, so the explicit `&` is documented intent.
// - `manual_async_fn`: a few traits use `Box<dyn Future>` for object safety.
#![allow(
    clippy::too_many_arguments,
    clippy::needless_borrow,
    clippy::manual_async_fn
)]

//! Lower-level building blocks of the agent runtime (**Layer 4a**).
//!
//! Renamed from `golish-agent-loop` in A2. The high-level streaming
//! loop (`agentic_loop/`), evals harness and mocks live in the sibling
//! crate [`golish_agent_runtime`](::golish_agent_runtime) so that
//! editing loop logic does not recompile this (much larger)
//! infrastructure layer.
//!
//! # What lives here
//!
//! - [`task_orchestrator`] — PentAGI-style multi-phase task orchestration
//! - [`tool_execution`]    — shared `route_tool_execution` dispatcher
//! - [`tool_executors`]    — concrete executors (memory, web, security, …)
//! - [`tool_definitions`]  — tool schema + preset selection
//! - [`tool_provider_impl`]— default implementation of `ToolProvider`
//! - [`loop_detection`]    — guard against runaway loops
//! - [`hitl`]              — Human-in-the-Loop approval recorder
//! - [`tool_policy`]       — declarative tool policy manager
//! - [`system_hooks`]      — pre/post tool hook registry
//! - [`sidecar_trait`]     — abstraction over the sidecar capture backend
//! - [`planner`]           — multi-step plan manager
//! - [`db_traits`]         — traits + DTOs for repo/tracking abstractions
//! - [`db_shim`]           — pass-through DB helpers used by orchestration
//! - [`db_tracking`]       — agent-side memory + recording layer
//! - [`memory_file`] / [`memory_gatekeeper`] — long-term memory utilities
//! - [`llm_client`]        — per-provider component builders + factory
//! - [`execution_mode`]    — Chat vs Task execution mode enum
//!
//! # Architecture
//!
//! - depends on: `golish-core`, `golish-events`, `golish-context`,
//!   `golish-tools`, `golish-prompts`, `golish-llm-providers`,
//!   `golish-sub-agents`, `golish-indexer`, `golish-json-repair`
//! - consumed by: `golish-agent-runtime` (L4b) and the `golish-ai` /
//!   `golish-agent-bridge` facades.

pub mod db_shim;
pub mod db_tracking;
pub mod db_traits;
pub mod execution_mode;
pub mod harness;
pub mod hitl;
pub mod llm_client;
pub mod loop_detection;
pub mod memory_file;
pub mod memory_gatekeeper;
pub mod planner;
pub mod sidecar_trait;
pub mod system_hooks;
pub mod task_orchestrator;
pub mod tool_definitions;
pub mod tool_execution;
pub mod tool_executors;
pub mod tool_policy;
pub mod tool_provider_impl;

pub(crate) use golish_events::event_coordinator;

pub mod agent_mode {
    //! Backward-compatibility alias: `AgentMode` lives in `golish-core`.
    //!
    //! Existing modules inside `golish-agent-kit` reference
    //! `crate::agent_mode::AgentMode`. To keep those references valid
    //! without rewriting every callsite, this module simply re-exports
    //! the canonical type from `golish-core`.
    pub use golish_core::AgentMode;
}

pub use agent_mode::AgentMode;
pub use execution_mode::ExecutionMode;
pub use golish_events::{
    build_summarizer_input, format_for_summarizer, read_transcript, save_summarizer_input,
    save_summary, transcript_path, CoordinatorHandle, CoordinatorState, EventCoordinator,
    TranscriptEvent, TranscriptWriter,
};
pub use golish_prompts::PromptContributorRegistry;
pub use golish_prompts::{
    build_summarizer_user_prompt, generate_summary, SummaryResponse, SUMMARIZER_SYSTEM_PROMPT,
};
pub use llm_client::SharedComponentsConfig;
pub use tool_definitions::{
    get_all_tool_definitions_with_config, get_tool_definitions_for_preset,
    get_tool_definitions_with_config, ToolConfig, ToolPreset,
};
pub use tool_execution::{
    normalize_run_pty_cmd_args, route_tool_execution, ToolExecutionConfig, ToolExecutionContext,
    ToolExecutionError, ToolExecutionResult, ToolRoutingCategory, ToolSource,
};
pub use tool_provider_impl::DefaultToolProvider;
