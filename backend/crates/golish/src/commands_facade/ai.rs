//! AI agent commands — init, chat, context, tools, sessions, policies, analytics.

pub use crate::ai::commands::core::lifecycle::{init_ai_agent, init_ai_agent_vertex, init_ai_agent_openai, init_ai_agent_unified, shutdown_ai_agent, is_ai_initialized};
pub use crate::ai::commands::core::chat::{send_ai_prompt, send_ai_prompt_session, send_ai_prompt_with_attachments, get_vision_capabilities, clear_ai_conversation_session, get_ai_conversation_length_session, signal_frontend_ready, cancel_ai_generation};
pub use crate::ai::commands::core::tools::{execute_ai_tool, get_available_tools};
pub use crate::ai::commands::core::session::{init_ai_session, shutdown_ai_session, is_ai_session_initialized, get_session_ai_config};
pub use crate::ai::commands::session::{clear_ai_conversation, get_ai_conversation_length, restore_ai_conversation, list_ai_sessions, find_ai_session, load_ai_session, export_ai_session_transcript};
pub use crate::ai::commands::config::{get_openrouter_api_key, get_openai_api_key, get_project_settings, save_project_model, get_vertex_ai_config, load_env_file, update_ai_workspace, set_ai_session_persistence, is_ai_session_persistence_enabled};
pub use crate::ai::commands::agents::{list_agent_definitions, read_agent_prompt, save_agent_definition, delete_agent_definition, seed_agents, list_sub_agents, get_sub_agent_model, set_sub_agent_model};
pub use crate::ai::commands::hitl::{get_approval_patterns, get_tool_approval_pattern, get_hitl_config, set_hitl_config, add_tool_always_allow, remove_tool_always_allow, reset_approval_patterns, respond_to_tool_approval};
pub use crate::ai::commands::policy::{get_tool_policy_config, set_tool_policy_config, get_tool_policy, set_tool_policy, reset_tool_policies, enable_full_auto_mode, disable_full_auto_mode, is_full_auto_mode_enabled};
pub use crate::ai::commands::mode::{get_agent_mode, set_agent_mode, save_project_agent_mode, set_use_agents, get_use_agents, set_execution_mode, get_execution_mode};
pub use crate::ai::commands::analytics::{get_api_request_stats, get_tool_call_stats, get_db_token_usage_stats, get_usage_by_agent, get_audit_log};
pub use crate::ai::commands::context::{get_context_summary, get_token_usage_stats, get_token_alert_level, get_context_utilization, get_remaining_tokens, reset_context_manager, get_context_trim_config, is_context_management_enabled, retry_compaction};
pub use crate::ai::commands::loop_detection::{get_loop_protection_config, set_loop_protection_config, get_loop_detector_stats, is_loop_detection_enabled, disable_loop_detection, enable_loop_detection, reset_loop_detector};
pub use crate::ai::commands::plan::get_plan;
pub use crate::ai::commands::commit_writer::generate_commit_message;
pub use crate::ai::commands::summarizer::finalize_ai_session;
pub use crate::ai::commands::workflow::restore_ai_session;
pub use crate::ai::commands::debug::search_memories;
pub use crate::ai::commands::{list_recent_memories, get_memory_count};
