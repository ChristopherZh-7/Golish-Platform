//! AI module - re-exports from the agent runtime stack.
//!
//! Historically this glob-re-exported the `golish-ai` umbrella crate. A3
//! deleted that umbrella; the equivalent re-exports now come directly
//! from the implementation crates (`golish-agent-kit`,
//! `golish-agent-runtime`, `golish-agent-bridge`, `golish-prompts`,
//! `golish-events`), mirroring exactly what the umbrella used to expose.

pub mod candidate_submit_tool;
pub mod commands;
pub mod db_bridge;
pub mod embedder_bridge;
pub mod graph_bridge;
pub mod harness_submit_tool;
pub mod harness_trace_tool;
pub mod knowledge_policy_adapter;
pub mod llm_one_shot;
pub mod session_bridge;
pub mod sidecar_bridge;
pub mod start_operation_tool;
pub mod tracking_bridge;

// --- A3: flat re-exports replacing `pub use golish_ai::*;` ---

pub use golish_agent_bridge::{agent_bridge, AgentBridge};
pub use golish_agent_kit::{
    agent_mode, db_shim, db_tracking, db_traits, execution_mode,
    get_all_tool_definitions_with_config, get_tool_definitions_for_preset,
    get_tool_definitions_with_config, hitl, llm_client, loop_detection, memory_file,
    memory_gatekeeper, normalize_run_pty_cmd_args, planner, route_tool_execution, sidecar_trait,
    system_hooks, tool_definitions, tool_execution, tool_executors, tool_policy,
    tool_provider_impl, AgentMode, DefaultToolProvider, SharedComponentsConfig,
    ToolExecutionConfig, ToolExecutionContext, ToolExecutionError, ToolExecutionResult, ToolPreset,
    ToolRoutingCategory, ToolSelectionConfig, ToolSource,
};
pub use golish_agent_runtime::agentic_loop::{OutputClassifier, PostShellHook};
pub use golish_agent_runtime::{agentic_loop, eval_support};
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

/// Task orchestration facade: runtime types from [`golish_agent_kit`]
/// plus `bridge_executor` from [`golish_agent_bridge`].
pub mod task_orchestrator {
    pub use golish_agent_bridge::bridge_executor;
    pub use golish_agent_kit::task_orchestrator::*;
}

pub use commands::{
    add_tool_always_allow, cancel_ai_generation, check_recon_tools_cmd, clear_ai_conversation,
    clear_ai_conversation_session, delete_agent_definition, disable_full_auto_mode,
    disable_loop_detection, enable_full_auto_mode, enable_loop_detection,
    export_ai_session_transcript, finalize_ai_session, find_ai_session, get_agent_mode,
    get_ai_conversation_length, get_ai_conversation_length_session, get_api_request_stats,
    get_approval_patterns, get_audit_log, get_context_summary, get_context_trim_config,
    get_context_utilization, get_db_token_usage_stats, get_execution_mode, get_hitl_config,
    get_loop_detector_stats, get_loop_protection_config, get_memory_count, get_openai_api_key,
    get_openrouter_api_key, get_plan, get_project_settings, get_remaining_tokens,
    get_session_ai_config, get_sub_agent_model, get_token_alert_level, get_token_usage_stats,
    get_tool_approval_pattern, get_tool_call_stats, get_tool_policy, get_tool_policy_config,
    get_usage_by_agent, get_vertex_ai_config, get_vision_capabilities, init_ai_session,
    is_ai_session_initialized, is_ai_session_persistence_enabled, is_context_management_enabled,
    is_full_auto_mode_enabled, is_loop_detection_enabled, kg_get_neighbors, kg_list_entities,
    kg_search_entities, list_agent_definitions, list_ai_sessions, list_recent_memories,
    list_running_sub_agent_dispatches, load_ai_session, load_env_file, read_agent_prompt,
    remove_tool_always_allow, reset_approval_patterns, reset_context_manager, reset_loop_detector,
    reset_tool_policies, respond_to_tool_approval, restore_ai_conversation, restore_ai_session,
    retry_compaction, save_agent_definition, save_project_agent_mode, save_project_model,
    search_memories, seed_agents, send_ai_prompt_session, send_ai_prompt_with_attachments,
    set_agent_mode, set_ai_session_persistence, set_execution_mode, set_hitl_config,
    set_loop_protection_config, set_sub_agent_model, set_tool_policy, set_tool_policy_config,
    shutdown_ai_session, signal_frontend_ready, update_ai_workspace, AiState,
};
