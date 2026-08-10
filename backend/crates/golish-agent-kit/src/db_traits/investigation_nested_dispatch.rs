//! SQL-free compound lifecycle for one bounded Investigation nested worker.
//!
//! The port deliberately combines StageTeam queue authority and the PentAGI
//! logical ledger.  A caller can never receive a child lease before its exact
//! nested dispatch receipt exists, and cannot terminalize that lease without
//! the matching dispatch attempt in the same durable commit.

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use super::{
    AgentType, ClaimedStageWorkItemView, CompletedStageWorkerView, NewStageWorkerOutput,
    RuntimeWorkerFence, UnifiedInvestigationDispatch, UnifiedInvestigationDispatchAttempt,
    UnifiedInvestigationDispatchOutcome, UnifiedInvestigationUnitIdentity,
};

pub type InvestigationNestedDispatchResult<T> =
    Result<T, InvestigationNestedDispatchRepositoryError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvestigationNestedDispatchRepositoryError {
    #[error("investigation_nested_dispatch_unavailable: {operation}")]
    Unavailable { operation: &'static str },
    #[error("investigation_nested_dispatch_invalid_request: {detail}")]
    InvalidRequest { detail: String },
    #[error("investigation_nested_dispatch_not_found: {detail}")]
    NotFound { detail: String },
    #[error("investigation_nested_dispatch_conflict: {detail}")]
    Conflict { detail: String },
    #[error("investigation_nested_dispatch_authority_mismatch: {detail}")]
    AuthorityMismatch { detail: String },
    #[error("investigation_nested_dispatch_lease_lost: {worker_run_id}:{attempt_epoch}")]
    LeaseLost {
        worker_run_id: Uuid,
        attempt_epoch: i64,
    },
    #[error("investigation_nested_dispatch_infrastructure: {detail}")]
    Infrastructure { detail: String },
}

/// Host-authored material for one cognition-only nested dispatch.  Identifiers
/// above StageTeam are explicit so replay must match the same Task, Subtask,
/// parent dispatch and parent worker fence.  Physical child identifiers are
/// derived by the repository from `stable_request_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginInvestigationNestedDispatch {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub stable_request_id: Uuid,
    pub task_plan_id: Uuid,
    pub subtask_id: Uuid,
    pub parent_dispatch_receipt_id: Uuid,
    pub parent_fence: RuntimeWorkerFence,
    pub stage_team_plan_id: Uuid,
    pub parent_work_item_id: Uuid,
    pub expected_dispatch_epoch: i64,
    pub nested_tool_request_id: String,
    pub requested_role: String,
    pub objective: String,
    pub args_sha256: String,
    pub snapshot_sha256: String,
    pub dispatch_ordinal: u32,
    pub session_id: Uuid,
    pub agent: AgentType,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub lease_owner: String,
    pub lease_seconds: i32,
    pub initial_chain: Value,
    pub initial_checkpoint: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BegunInvestigationNestedDispatch {
    pub begin_receipt_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_plan_id: Uuid,
    pub subtask_id: Uuid,
    pub parent_dispatch_receipt_id: Uuid,
    pub stage_worker_request_id: Uuid,
    pub args_sha256: String,
    pub request_sha256: String,
    pub begin_receipt_sha256: String,
    pub child: ClaimedStageWorkItemView,
    pub dispatch: UnifiedInvestigationDispatch,
    pub replayed: bool,
}

/// Exact terminal commit for the child returned by `begin`.  The ordinary
/// StageWorker output authority is retained: cognition may return advisory
/// structured output (or a typed blocked output), never direct external I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishInvestigationNestedDispatch {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub stable_request_id: Uuid,
    pub begin_receipt_id: Uuid,
    pub task_plan_id: Uuid,
    pub subtask_id: Uuid,
    pub parent_dispatch_receipt_id: Uuid,
    pub dispatch_receipt_id: Uuid,
    pub child_fence: RuntimeWorkerFence,
    pub stage_team_plan_id: Uuid,
    pub work_item_id: Uuid,
    pub expected_work_item_row_version: i64,
    pub output: NewStageWorkerOutput,
    pub terminal_checkpoint: Value,
    pub evidence_watermark: Option<i64>,
    pub outcome: UnifiedInvestigationDispatchOutcome,
    pub result_sha256: String,
    pub fence_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedInvestigationNestedDispatch {
    pub finish_receipt_id: Uuid,
    pub stable_request_id: Uuid,
    pub begin_receipt_id: Uuid,
    pub task_plan_id: Uuid,
    pub subtask_id: Uuid,
    pub parent_dispatch_receipt_id: Uuid,
    pub dispatch_receipt_id: Uuid,
    pub result_sha256: String,
    pub finish_receipt_sha256: String,
    pub completion: CompletedStageWorkerView,
    pub dispatch_attempt: UnifiedInvestigationDispatchAttempt,
    pub replayed: bool,
}

#[async_trait]
pub trait InvestigationNestedDispatchRepository: Send + Sync {
    async fn begin(
        &self,
        request: BeginInvestigationNestedDispatch,
    ) -> InvestigationNestedDispatchResult<BegunInvestigationNestedDispatch>;

    async fn finish(
        &self,
        request: FinishInvestigationNestedDispatch,
    ) -> InvestigationNestedDispatchResult<FinishedInvestigationNestedDispatch>;
}
