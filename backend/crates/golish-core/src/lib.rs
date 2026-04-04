//! Core types and traits for the Qbit application.
//!
//! This crate provides the foundation types used across all other golish crates.
//! It has ZERO internal crate dependencies and only depends on external libraries.
//!
//! ## Architecture Principle
//!
//! golish-core sits at the bottom of the dependency hierarchy:
//! - Layer 1 (Foundation): golish-core ← YOU ARE HERE
//! - Layer 2 (Infrastructure): golish-settings, golish-runtime
//! - Layer 3 (Domain): golish-tools, golish-pty, etc.
//! - Layer 4 (Application): golish (main crate)

// Module declarations (will be populated in next steps)
pub mod api_request_stats;
pub mod events;
pub mod message;
pub mod runtime;
pub mod session;
pub mod tool;
pub mod tool_name;

pub mod hitl;
pub mod plan;
pub mod prompt;
pub mod utils;

// Re-exports
pub use api_request_stats::{
    ApiRequestStats, ApiRequestStatsSnapshot, ProviderRequestStatsSnapshot,
};
pub use events::*; // Re-export all event types
pub use hitl::{
    ApprovalDecision, ApprovalPattern, RiskLevel, ToolApprovalConfig,
    HITL_AUTO_APPROVE_MIN_APPROVALS, HITL_AUTO_APPROVE_THRESHOLD,
};
pub use message::{PromptPart, PromptPayload};
pub use plan::{PlanStep, PlanSummary, StepStatus, TaskPlan, MAX_PLAN_STEPS, MIN_PLAN_STEPS};
pub use prompt::{
    PromptContext, PromptContributor, PromptMatchedSkill, PromptPriority, PromptSection,
    PromptSkillInfo,
};
pub use runtime::{ApprovalResult, QbitRuntime, RuntimeError, RuntimeEvent};
pub use session::{
    find_session_by_identifier, list_recent_sessions, MessageContent, MessageRole, SessionArchive,
    SessionArchiveMetadata, SessionListing, SessionMessage, SessionSnapshot,
};
pub use tool::Tool;
pub use tool_name::{ToolCategory, ToolName};
