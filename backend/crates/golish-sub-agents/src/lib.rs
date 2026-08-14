#![allow(clippy::too_many_arguments)]

//! Sub-agent system for Golish.
//!
//! This crate provides the sub-agent system infrastructure including:
//! - Sub-agent definitions with custom system prompts and tool restrictions
//! - Sub-agent registry for managing available agents
//! - Context management for passing state between agents
//! - Sub-agent execution with tool support
//! - Default sub-agent definitions for common tasks
//!
//! # Architecture
//!
//! This is a **Layer 2 (Infrastructure)** crate:
//! - Depends on: golish-core, golish-udiff, golish-web, rig-core, vtcode-core
//! - Used by: golish-ai (for sub-agent orchestration)
//!
//! # Core Components
//!
//! - **SubAgentDefinition**: Defines a specialized sub-agent
//! - **SubAgentRegistry**: Registry of available sub-agents
//! - **SubAgentContext**: Context passed between agents
//! - **SubAgentResult**: Result returned by sub-agent execution
//! - **execute_sub_agent**: Main execution function
//! - **ToolProvider**: Trait for tool definition/execution injection
//!
//! # Example
//!
//! ```ignore
//! use golish_sub_agents::{SubAgentDefinition, SubAgentRegistry, create_default_sub_agents};
//!
//! // Create a registry with default sub-agents
//! let mut registry = SubAgentRegistry::new();
//! registry.register_multiple(create_default_sub_agents());
//!
//! // Get a specific sub-agent
//! if let Some(analyzer) = registry.get("analyzer") {
//!     println!("Found: {}", analyzer.name);
//! }
//! ```

pub mod defaults;
pub mod definition;
pub mod discovery;
pub mod executor;
pub(crate) mod executor_helpers;
pub(crate) mod executor_types;
pub(crate) mod executor_udiff;
pub mod file_loader;
pub mod intel_goal;
pub mod prompt_contributor;
pub mod prompt_registry;
pub mod schemas;
pub mod transcript;

// Re-export main types from definition module
pub use definition::{
    AgentSource, SubAgentContext, SubAgentDefinition, SubAgentRegistry, SubAgentResult,
    MAX_AGENT_DEPTH,
};

// Re-export default sub-agents function
pub use defaults::create_default_sub_agents;

// Re-export discovery
pub use discovery::discover_agents;

// Re-export file loader types
pub use file_loader::AgentFileInfo;

// Re-export executor types
pub use executor::{
    execute_sub_agent, refine_eas_web_repair_mode_from_worklist,
    retain_eas_web_repair_targets_for_same_gap, submit_coverage_gap_repair_mode_from_reasons,
    submit_repair_mode_from_submit_result, SubAgentExecutorContext, ToolProvider,
};
pub use executor_types::{
    is_investigation_asset_verification_cognition_tool, is_investigation_asset_verification_tool,
    BegunBoundWorkerNestedDelegation, BoundTerminalExecutionContract, BoundTerminalResultValidator,
    BoundTerminalValidationError, BoundWorkerChainContext, BoundWorkerNestedDelegationCompletion,
    BoundWorkerNestedDelegationLifecycle, BoundWorkerNestedDispatchToken,
    BoundWorkerRuntimeMemorySource, BoundWorkerToolLifecycle, CoverageGapAction,
    EasWebRepairTarget, InvestigationActorContract, InvestigationAssetLaneIdentity,
    InvestigationAssetVerificationActorBinding, InvestigationAssetVerificationActorObservationV2,
    InvestigationAssetVerificationPrimaryBinding, InvestigationDynamicVerificationDispositionV1,
    InvestigationDynamicVerificationHypothesisProposalV1,
    InvestigationDynamicVerificationPrimaryTurnV1, InvestigationDynamicVerificationSubjectRefV1,
    InvestigationDynamicVerificationSubtaskV1, PostShellHook, StageCapabilitySuggestion,
    StageTeamCompiledActionBinding, StageTeamLeaderBinding, StageToolGuard, StageToolHider,
    SubAgentChainError, SubAgentChainPersistence, SubAgentToolObservation, SubAgentToolObserver,
    SubAgentToolResultHook, SubAgentToolRouter, SubmitRepairKind, SubmitRepairMode,
    TargetIntelReviewBinding, ENUMERATION_REDUCE_PARAMETERS_TOOL_NAME,
    ENUMERATION_REVIEW_COVERAGE_TOOL_NAME, INVESTIGATION_ANALYSIS_ROLE_IDS,
    INVESTIGATION_ASSET_VERIFICATION_ACTOR_OBSERVATION_SCHEMA,
    INVESTIGATION_ASSET_VERIFICATION_COGNITION_TOOL_NAMES,
    INVESTIGATION_ASSET_VERIFICATION_PRIMARY_RESOLUTION_SCHEMA,
    INVESTIGATION_ASSET_VERIFICATION_TOOL_NAMES,
    INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_SCHEMA,
    INVESTIGATION_PRIMARY_SYNTHESIS_RESULT_SCHEMA, INVESTIGATION_REFINER_PATCH_RESULT_SCHEMA,
    INVESTIGATION_TASK_PLAN_RESULT_SCHEMA, INVESTIGATION_VERIFICATION_ROLE_IDS,
    STAGE_TEAM_DISPATCH_ACCEPTED_STATUS, STAGE_TEAM_DISPATCH_WORKERS_TOOL_NAME,
    STAGE_TEAM_PREPARE_FINAL_STATUS, STAGE_TEAM_PREPARE_FINAL_SUBMISSION_TOOL_NAME,
    STAGE_TEAM_UPDATE_PLAN_TOOL_NAME,
};
pub use intel_goal::{
    adapt_target_intel_batch, adapt_target_intel_task, advisory_rework_runtime_enabled,
    evaluate_advisory_rework, neutral_target_intel_worker_system_prompt,
    render_neutral_controller_prompt, render_neutral_reviewer_prompt, render_neutral_worker_prompt,
    target_intel_read_review_section_schema, target_intel_record_review_verdict_schema,
    target_intel_request_review_schema, target_intel_spawn_subagents_schema,
    AdvisoryReworkDecision, IntelDynamicSpawnRequest, IntelDynamicTaskRequest,
    IntelFindingMateriality, IntelGoalLeaderBinding, IntelGoalPrimitiveError, IntelReviewDecision,
    IntelReviewFindingV1, IntelReviewV1, IntelStampedWorkItem, INTEL_REVIEW_KIND,
    INTEL_REVIEW_SCHEMA, INTEL_WORKER_KIND, INTEL_WORKER_ROLE, STAGE_TEAM_REQUEST_INTEL_REVIEW,
    STAGE_TEAM_SPAWN_INTEL_SUBAGENTS, TARGET_INTEL_READ_REVIEW_SECTION,
    TARGET_INTEL_RECORD_REVIEW_VERDICT,
};

// Re-export prompt registry
pub use prompt_registry::{PromptContext, PromptRegistry};

// Re-export prompt contributor (moved from golish-prompts in A1)
pub use prompt_contributor::SubAgentPromptContributor;
