//! Backward-compatibility umbrella for the AI agent stack.
//!
//! After A1-1/A1-2/A1-3 the implementation lives in:
//! [`golish-prompts`](golish_prompts) (prompts/summariser),
//! [`golish-agent-loop`](golish_agent_loop) (runtime), and
//! [`golish-agent-bridge`](golish_agent_bridge) (`AgentBridge` +
//! bridge_executor). New code should depend on those crates directly.

#![allow(deprecated)]

pub use golish_agent_loop::{
    agent_mode, agentic_loop, db_shim, db_traits, db_tracking, eval_support, execution_mode,
    get_all_tool_definitions_with_config, get_tool_definitions_for_preset,
    get_tool_definitions_with_config, hitl, llm_client, loop_detection, memory_file,
    memory_gatekeeper, normalize_run_pty_cmd_args, planner, route_tool_execution, sidecar_trait,
    system_hooks, tool_definitions, tool_execution, tool_executors, tool_policy,
    tool_provider_impl, AgentMode, DefaultToolProvider, SharedComponentsConfig, ToolConfig,
    ToolExecutionConfig, ToolExecutionContext, ToolExecutionError, ToolExecutionResult, ToolPreset,
    ToolRoutingCategory, ToolSource,
};
pub use golish_agent_loop::agentic_loop::{OutputClassifier, PostShellHook};
pub use golish_agent_bridge::{agent_bridge, AgentBridge};
pub use golish_events::{
    build_summarizer_input, format_for_summarizer, read_transcript, save_summarizer_input,
    save_summary, transcript_path, CoordinatorHandle, CoordinatorState, EventCoordinator,
    TranscriptEvent, TranscriptWriter,
};
pub use golish_prompts::{
    build_summarizer_user_prompt, codex_prompt, contributors, generate_summary, prompt_registry,
    summarizer, system_prompt, PromptContributorRegistry, SummaryResponse,
    SUMMARIZER_SYSTEM_PROMPT,
};

/// Task orchestration facade: runtime types from [`golish_agent_loop`]
/// plus `bridge_executor` from [`golish_agent_bridge`].
pub mod task_orchestrator {
    pub use golish_agent_bridge::bridge_executor;
    pub use golish_agent_loop::task_orchestrator::*;
}
