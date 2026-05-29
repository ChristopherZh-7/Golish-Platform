//! AI agent commands — init, chat, context, tools, sessions, policies, analytics.
//!
//! Expected command domains exposed here (documentation only; the
//! authoritative list is whatever `crate::ai::commands` re-exports):
//! - **Lifecycle**: `init_ai_agent` (unified, takes `ProviderConfig`),
//!   `shutdown_ai_agent`, `is_ai_initialized`
//! - **Chat**: `send_ai_prompt*`, `clear_ai_conversation*`, `cancel_ai_generation`
//! - **Tools**: `execute_ai_tool`, `get_available_tools`, `list_sub_agents`,
//!   `*_sub_agent_model`, `get_vision_capabilities`, `signal_frontend_ready`
//! - **Sessions**: `init_ai_session`, `shutdown_ai_session`,
//!   `is_ai_session_initialized`, `get_session_ai_config`
//! - **Session archive**: `list_ai_sessions`, `find_ai_session`,
//!   `load_ai_session`, `export_ai_session_transcript`, `restore_ai_session`
//! - **Config**: `get_*_api_key`, `get_project_settings`, `save_project_model`,
//!   `get_vertex_ai_config`, `load_env_file`, `update_ai_workspace`,
//!   `*_ai_session_persistence`
//! - **Agents**: `list_agent_definitions`, `read_agent_prompt`,
//!   `save_agent_definition`, `delete_agent_definition`, `seed_agents`
//! - **HITL**: `*_approval_patterns`, `*_tool_approval_pattern`,
//!   `*_hitl_config`, `*_tool_always_allow`, `respond_to_tool_approval`
//! - **Policy**: `*_tool_policy*`, `*_full_auto_mode`
//! - **Mode**: `*_agent_mode`, `*_use_agents`, `*_execution_mode`
//! - **Analytics**: `get_api_request_stats`, `get_tool_call_stats`,
//!   `get_db_token_usage_stats`, `get_usage_by_agent`, `get_audit_log`
//! - **Context**: `get_plan`, `get_context_summary`, `get_token_*`,
//!   `get_context_*`, `get_remaining_tokens`, `reset_context_manager`,
//!   `retry_compaction`, `is_context_management_enabled`
//! - **Loop protection**: `*_loop_protection_config`, `*_loop_detect*`
//! - **Misc**: `finalize_ai_session`,
//!   `search_memories`, `list_recent_memories`, `get_memory_count`

pub use crate::ai::commands::*;
