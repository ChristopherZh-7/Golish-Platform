//! Core types and traits for the Golish application.
//!
//! This crate provides the foundation types used across all other golish crates.
//! Its only internal dependency is `golish-platform` (a same-layer L1 sibling
//! providing OS / path / open helpers); otherwise it depends only on external
//! libraries.
//!
//! ## Architecture Principle
//!
//! golish-core sits at the bottom of the dependency hierarchy (L1 Foundation,
//! alongside `golish-platform`):
//! - Layer 1 (Foundation): golish-platform, golish-core ← YOU ARE HERE
//! - Layer 2 (Infrastructure): golish-settings, golish-db, golish-pty, etc.
//! - Layer 3 (Domain): golish-prompts, golish-sub-agents, etc.
//! - Layer 6 (Application): golish (main crate)

// Module declarations (will be populated in next steps)
pub mod agent_mode;
pub mod agent_session;
pub mod api_request_stats;
pub mod attack_execution;
pub mod events;
pub mod message;
pub mod runtime;
pub mod session;
pub mod session_kind;
pub mod tool;
pub mod tool_name;

pub mod event_emitter;
pub mod hitl;
pub mod investigation_contract;
pub mod jsonl;
pub mod os;
pub mod paths;
pub mod pentest_context;
pub mod plan;
pub mod prompt;
pub mod ready_gate;
pub mod session_manager;
pub mod skill_provider;
pub mod textual_tool_call;
pub mod time;
pub mod tool_args;
pub mod utils;
pub mod vault;
pub mod web_fetch;

// Re-exports
pub use agent_mode::AgentMode;
pub use agent_session::{
    current_agent_session, current_agent_tool_cancellation, current_agent_tool_context,
    current_agent_tool_output_sender, emit_current_agent_tool_output_chunk, with_agent_session,
    with_agent_tool_cancellation, with_agent_tool_context, with_agent_tool_output_sender,
    AgentToolCancellation, AgentToolContext, WorkerLeaseContext,
};
pub use api_request_stats::{
    ApiRequestStats, ApiRequestStatsSnapshot, ProviderRequestStatsSnapshot,
};
pub use attack_execution::{
    check_candidate_tool_boundary, check_candidate_tool_boundary_mode, AttackExecutionContract,
    CandidateAttemptContextRef, CandidateToolBoundaryError,
};
pub use event_emitter::{emit_opt, EventEmitter, EventEmitterHandle, NullEmitter};
pub use events::*; // Re-export all event types
pub use hitl::{
    ApprovalDecision, ApprovalPattern, RiskLevel, ToolApprovalConfig,
    HITL_AUTO_APPROVE_MIN_APPROVALS, HITL_AUTO_APPROVE_THRESHOLD,
};
pub use investigation_contract::{
    CampaignWritePolicy, ComparePolicy, InvestigationAuthority, InvestigationContractParseError,
    InvestigationContractVersion, InvestigationErrorCode, InvestigationModePolicy,
    InvestigationRolloutMode, LegacyProjectionPolicy,
};
pub use message::{PromptPart, PromptPayload};
pub use pentest_context::PentestEngineContext;
pub use plan::{PlanStep, PlanSummary, StepStatus, TaskPlan, MAX_PLAN_STEPS, MIN_PLAN_STEPS};
pub use prompt::{
    PromptContext, PromptContributor, PromptMatchedSkill, PromptPriority, PromptSection,
    PromptSkillInfo,
};
pub use ready_gate::DbReadyGate;
pub use runtime::{ApprovalResult, GolishRuntime, RuntimeError, RuntimeEvent};
pub use session::{
    find_session_by_identifier, list_recent_sessions, MessageContent, MessageRole, SessionArchive,
    SessionArchiveMetadata, SessionListing, SessionMessage, SessionSnapshot,
};
pub use session_kind::{is_title_gen_session_id, title_gen_session_id, TITLE_GEN_SESSION_PREFIX};
pub use session_manager::{SessionManager, SessionManagerFactory};
pub use skill_provider::{SkillMatch, SkillMetadata, SkillProvider};
pub use textual_tool_call::{
    finalize_assistant_text, parse_textual_tool_calls, select_textual_tool_call,
    select_textual_tool_calls, strip_textual_tool_call_markup, FinalizedAssistantText,
    TextualToolCall,
};
pub use time::{now_ms, now_ts, ts_from_dt};
pub use tool::Tool;
pub use tool_args::{has_complete_tool_args, initial_tool_args_fragment};
pub use tool_name::{ToolCategory, ToolName};
pub use web_fetch::{WebFetchProvider, WebFetchResult};
