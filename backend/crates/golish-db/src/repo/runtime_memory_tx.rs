//! Compound runtime-memory transaction vocabulary and lock-order contract.
//!
//! Operation creation and later runtime transitions share the typed error and
//! lock vocabulary here. Keeping fencing and table order together prevents
//! independently implemented transitions from taking incompatible locks or
//! splitting dual writes across transactions.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use golish_memory_domain::{CanonicalRowId, CanonicalSourceKind};
use golish_memory_domain::{
    EpisodeVerdict, KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, KnowledgeEventPayloadV1,
    OperationScope, ProjectScopeId, SourceRef, StageEpisode,
};

use crate::models::{AgentType, Task};
use crate::repo::canonical_fact_refs::{CanonicalFactKey, CanonicalFactRef};
use crate::repo::operation_org_scope::{FrozenOperationOrgScope, ScopeFreezeError};
use crate::repo::operation_scope_decisions::{OperationScopeDecisionRow, ScopeDecisionError};
use crate::repo::operation_state::OperationStateRow;
use crate::repo::stage_deliverable_submissions::StageDeliverableSubmissionRow;
use crate::repo::stage_handoffs::StageHandoffRow;
use crate::repo::stage_run_units::StageRunUnitRow;
use crate::repo::stage_worker_runs::StageWorkerRunRow;
use crate::repo::{
    attack_candidates, attack_execution_rollout, attack_execution_shadow, canonical_fact_refs,
    message_chains, operation_org_scope, operation_scope_decisions, operation_state,
    operation_turns, project_scopes, runtime_memory_rollout, runtime_memory_shadow,
    stage_asset_waves, stage_episodes, stage_handoffs, stage_run_units, stage_runs, stage_teams,
    stage_worker_runs, tasks,
};

const MEMORY_EPISODE_STAGE_KINDS: [&str; 4] = [
    "target_intel",
    "external_attack_surface",
    "enumeration",
    "vuln_triage",
];

#[derive(Debug, thiserror::Error)]
pub enum RuntimeMemoryStoreError {
    #[error("invalid runtime contract transition: {from:?} -> {to:?}")]
    InvalidContractTransition {
        from: runtime_memory_rollout::RuntimeMemoryContract,
        to: runtime_memory_rollout::RuntimeMemoryContract,
    },
    #[error("stale runtime row version for {entity}: expected {expected}, actual {actual}")]
    StaleVersion {
        entity: &'static str,
        expected: i64,
        actual: i64,
    },
    #[error("runtime memory conflict: {code}")]
    Conflict { code: &'static str },
    #[error("runtime memory identity mismatch: {code}")]
    IdentityMismatch { code: &'static str },
    #[error("runtime memory row missing: {entity}")]
    Missing { entity: &'static str },
    #[error("runtime worker lease lost: worker={worker_run_id}, epoch={attempt_epoch}")]
    LeaseLost {
        worker_run_id: Uuid,
        attempt_epoch: i64,
    },
    #[error("runtime memory SQL failure: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("runtime memory repository failure: {0}")]
    Repository(#[from] crate::DbError),
}

pub type RuntimeMemoryStoreResult<T> = Result<T, RuntimeMemoryStoreError>;

const WORKER_TOOL_TRANSACTION_ATTEMPTS: usize = 3;
const MAX_COMPANY_CONTROLLER_SUCCESSOR_TURNS: i64 = 2;

fn is_retryable_runtime_transaction_sqlstate(code: &str) -> bool {
    matches!(code, "40P01" | "40001")
}

fn is_retryable_sqlx_transaction_error(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database)
            if database
                .code()
                .as_deref()
                .is_some_and(is_retryable_runtime_transaction_sqlstate)
    )
}

fn is_retryable_runtime_transaction_error(error: &RuntimeMemoryStoreError) -> bool {
    match error {
        RuntimeMemoryStoreError::Sqlx(error) => is_retryable_sqlx_transaction_error(error),
        RuntimeMemoryStoreError::Repository(crate::DbError::Sqlx(error)) => {
            is_retryable_sqlx_transaction_error(error)
        }
        _ => false,
    }
}

async fn worker_tool_transaction_retry_runner<T, Operation, OperationFuture, Retryable>(
    phase: &'static str,
    worker_run_id: Uuid,
    tool_call_record_id: Uuid,
    mut operation: Operation,
    retryable: Retryable,
) -> RuntimeMemoryStoreResult<T>
where
    Operation: FnMut() -> OperationFuture,
    OperationFuture: std::future::Future<Output = RuntimeMemoryStoreResult<T>>,
    Retryable: Fn(&RuntimeMemoryStoreError) -> bool,
{
    for attempt in 1..=WORKER_TOOL_TRANSACTION_ATTEMPTS {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if attempt < WORKER_TOOL_TRANSACTION_ATTEMPTS && retryable(&error) => {
                tracing::warn!(
                    worker_run_id = %worker_run_id,
                    tool_call_record_id = %tool_call_record_id,
                    phase,
                    attempt,
                    max_attempts = WORKER_TOOL_TRANSACTION_ATTEMPTS,
                    error = %error,
                    "retrying transient worker tool-fence transaction"
                );
                tokio::time::sleep(std::time::Duration::from_millis(
                    5 * u64::try_from(attempt).unwrap_or(1),
                ))
                .await;
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("bounded worker tool-fence transaction loop always returns")
}

#[cfg(test)]
mod transient_runtime_tx_tests {
    use super::{
        is_retryable_runtime_transaction_sqlstate, stage_team_tool_recovery_policy,
        worker_tool_transaction_retry_runner, RuntimeMemoryStoreError, StageTeamToolRecoveryPolicy,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn only_postgres_transaction_abort_states_are_retried() {
        assert!(is_retryable_runtime_transaction_sqlstate("40P01"));
        assert!(is_retryable_runtime_transaction_sqlstate("40001"));
        assert!(!is_retryable_runtime_transaction_sqlstate("23505"));
        assert!(!is_retryable_runtime_transaction_sqlstate("08006"));
    }

    #[tokio::test]
    async fn worker_tool_transaction_retry_runner_retries_only_retryable_failures() {
        let attempts = AtomicUsize::new(0);
        let value = worker_tool_transaction_retry_runner(
            "tool_begin",
            uuid::Uuid::from_u128(1),
            uuid::Uuid::from_u128(2),
            || {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    if attempt < 2 {
                        Err(RuntimeMemoryStoreError::Conflict {
                            code: "transient_test",
                        })
                    } else {
                        Ok(42_u8)
                    }
                }
            },
            |error| {
                matches!(
                    error,
                    RuntimeMemoryStoreError::Conflict {
                        code: "transient_test"
                    }
                )
            },
        )
        .await
        .expect("retryable operation converges");
        assert_eq!(value, 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);

        let attempts = AtomicUsize::new(0);
        let error = worker_tool_transaction_retry_runner(
            "tool_finish",
            uuid::Uuid::from_u128(3),
            uuid::Uuid::from_u128(4),
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                async {
                    Err::<u8, _>(RuntimeMemoryStoreError::Conflict {
                        code: "permanent_test",
                    })
                }
            },
            |_error| false,
        )
        .await
        .expect_err("non-retryable operation must fail");
        assert!(matches!(
            error,
            RuntimeMemoryStoreError::Conflict {
                code: "permanent_test"
            }
        ));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stage_team_automatic_tool_recovery_is_a_closed_policy() {
        let fence_failure =
            "worker tool result rejected by lease fence: runtime memory storage failure";
        assert_eq!(
            stage_team_tool_recovery_policy("recon_list_providers", "failed", Some(fence_failure)),
            Some(StageTeamToolRecoveryPolicy::RetrySafeTerminalLocalTool)
        );
        assert_eq!(
            stage_team_tool_recovery_policy("enum_crawl_same_origin_urls", "running", None),
            Some(StageTeamToolRecoveryPolicy::ResumeAfterInterruptedBoundedReadOnlyTool)
        );
        assert_eq!(
            stage_team_tool_recovery_policy("enum_crawl_same_origin_urls", "received", None),
            Some(StageTeamToolRecoveryPolicy::ResumeAfterInterruptedBoundedReadOnlyTool)
        );
        for tool_name in [
            "eas_probe_http_liveness",
            "eas_discover_ports",
            "eas_fingerprint_services",
            "eas_fingerprint_web_stack",
        ] {
            assert_eq!(
                stage_team_tool_recovery_policy(tool_name, "running", None),
                Some(StageTeamToolRecoveryPolicy::ResumeAfterInterruptedBoundedReadOnlyTool),
                "EAS wrapper must reconcile from durable coverage: {tool_name}"
            );
        }
        for (tool_name, status, result) in [
            ("recon_lookup_company", "failed", Some(fence_failure)),
            ("recon_list_providers", "running", Some(fence_failure)),
            (
                "recon_list_providers",
                "failed",
                Some("provider lookup failed"),
            ),
            ("browser_collect_js_api", "running", None),
            ("route_probe_paths", "running", None),
            ("vuln_nuclei_general", "running", None),
        ] {
            assert_eq!(
                stage_team_tool_recovery_policy(tool_name, status, result),
                None,
                "unexpected automatic recovery policy for {tool_name}"
            );
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRuntimeOperationRow {
    pub operation_id: Uuid,
    pub initial_stage_execution_id: Uuid,
    pub session_id: Uuid,
    pub title: Option<String>,
    pub input: String,
    pub profile: String,
    pub entry_stage: String,
    pub project_scope_id: Uuid,
    pub cli_scope: Option<CliRuntimeScopeRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageForkCreateRow {
    pub source_operation_id: Uuid,
    pub source_scope_snapshot_id: Uuid,
    pub entry_stage: String,
    pub terminal_stage: String,
    pub adopted_stage_kinds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliRuntimeScopeUnitRow {
    pub organization_id: Uuid,
    pub parent_organization_id: Option<Uuid>,
    pub organization_name: String,
    pub depth: i32,
    pub ordinal: i32,
    pub ownership_percent: Option<String>,
    pub approval_source: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliRuntimeScopeRow {
    pub root_organization_id: Uuid,
    pub include_subsidiaries: bool,
    pub subsidiary_threshold: u8,
    pub units: Vec<CliRuntimeScopeUnitRow>,
}

#[derive(Debug, Clone)]
pub struct CreatedRuntimeOperationRow {
    pub task: Task,
    pub operation: OperationStateRow,
    pub initial_stage_execution_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionStageExecutionRow {
    pub operation_id: Uuid,
    pub current_stage_execution_id: Uuid,
    pub next_stage_execution_id: Uuid,
    pub next_stage: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteTerminalStageExecutionRow {
    pub operation_id: Uuid,
    pub current_stage_execution_id: Uuid,
    pub terminal_stage: String,
    pub task_result: String,
}

#[derive(Debug, Clone)]
pub struct TransitionedStageExecutionRow {
    pub previous_stage_execution: stage_runs::StageRunRow,
    pub current_stage_execution: stage_runs::StageRunRow,
    pub operation: OperationStateRow,
}

#[derive(Debug, Clone)]
pub struct SupersedeStageCheckpointRow {
    pub operation_id: Uuid,
    pub expected_current_stage: String,
    pub selected_stage: String,
    pub affected_stage_kinds: Vec<String>,
    pub next_state_blob: Value,
    /// Trusted stage-spec specialist. `Some` seeds one queued Unit per frozen
    /// org for the replacement execution; `None` seeds a root-only Unit when a
    /// sealed scope already exists (Scoping pre-freeze remains empty). The
    /// canonical Team seed/claim path owns all replacement Worker creation.
    pub replacement_specialist: Option<String>,
    /// `None` is the clear-repair mode (legacy mirror only). Restart modes
    /// preallocate a fresh active execution identity for `selected_stage`.
    pub replacement_stage_execution_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SupersededStageCheckpointStats {
    pub workers_superseded: u64,
    pub units_superseded: u64,
    pub executions_superseded: u64,
    pub handoffs_invalidated: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizeScopingScopeRow {
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub stage_execution_id: Uuid,
    pub root_organization_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub scoping_root_unit_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct FinalizedScopingScopeRow {
    pub decision: OperationScopeDecisionRow,
    pub scope: FrozenOperationOrgScope,
    pub root_unit: StageRunUnitRow,
    pub submission: StageDeliverableSubmissionRow,
    pub replayed: bool,
}

/// Server-side recipe for seeding one exact stage execution from its sealed
/// organization snapshot. Optional organization identities are an authority
/// filter only: the transaction still resolves and validates every identity
/// against `operation_org_scope_units`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedStageRuntimeRow {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_kind: String,
    pub unit_generation: i32,
    pub specialist: String,
    pub worker_generation: i32,
    pub work_item_kind: String,
    pub work_item_key: String,
    pub agent_path_prefix: String,
    /// Optional server-authorized subset of the sealed organization snapshot.
    /// `None` preserves the legacy all-frozen-organizations fan-out.
    pub organization_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Clone)]
pub struct SeededStageRuntimeRow {
    pub unit: StageRunUnitRow,
    pub worker: StageWorkerRunRow,
    pub organization_name: String,
    pub scope_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StageTeamPlanSeedRow {
    pub schema_version: i32,
    pub plan_version: i32,
    pub plan_hash: String,
    pub leader_role: String,
    pub allowed_roles: Vec<String>,
    pub aggregator_kind: String,
    pub aggregator_role: Option<String>,
    pub max_workers_total: i32,
    pub max_workers_active: i32,
    pub dynamic_requests_enabled: bool,
    pub dynamic_request_policy: serde_json::Value,
    pub final_submitter_kind: String,
    pub created_from_stage_spec_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StageWorkItemSeedRow {
    pub stable_key: String,
    pub work_item_kind: String,
    pub role: String,
    pub input_manifest: serde_json::Value,
    pub input_manifest_hash: String,
    pub conflict_key: Option<String>,
    pub priority: i32,
    pub required_for_barrier: bool,
    pub is_aggregator: bool,
    pub attempt_policy: serde_json::Value,
    pub budget: serde_json::Value,
    pub output_schema: String,
    pub created_by: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeedStageTeamRuntimeRow {
    pub base: SeedStageRuntimeRow,
    pub plan: StageTeamPlanSeedRow,
    pub work_items: Vec<StageWorkItemSeedRow>,
}

#[derive(Debug, Clone)]
pub struct SeededStageTeamRuntimeRow {
    pub unit: StageRunUnitRow,
    pub plan: crate::repo::stage_teams::StageTeamPlanRow,
    pub work_items: Vec<crate::repo::stage_teams::StageWorkItemRow>,
    pub organization_name: String,
    pub scope_hash: String,
    pub replayed: bool,
}

/// Compound worker-claim input. The expected Unit and Worker versions are
/// caller-observed concurrency tokens; lease and chain identities are generated
/// inside the transaction and cannot be selected by a model.
#[derive(Debug, Clone)]
pub struct ClaimWorkerAndBindChainRow {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub worker_run_id: Uuid,
    pub expected_unit_status: crate::repo::stage_run_units::StageRunUnitStatus,
    pub expected_unit_row_version: i64,
    pub expected_worker_status: crate::repo::stage_worker_runs::StageWorkerRunStatus,
    pub expected_attempt_epoch: i64,
    pub session_id: Uuid,
    pub subtask_id: Option<Uuid>,
    pub agent: AgentType,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub parent_chain_id: Option<Uuid>,
    pub lease_owner: String,
    pub lease_seconds: i32,
    pub initial_chain: serde_json::Value,
    pub initial_checkpoint: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ClaimedWorkerAndChainRow {
    pub unit: StageRunUnitRow,
    pub worker: StageWorkerRunRow,
    pub message_chain_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct ClaimStageWorkItemRow {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub lease_owner: String,
    pub lease_seconds: i32,
    pub session_id: Uuid,
    pub subtask_id: Option<Uuid>,
    pub agent: AgentType,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub parent_chain_id: Option<Uuid>,
    pub initial_chain: serde_json::Value,
    pub initial_checkpoint: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ClaimedStageWorkItemRow {
    pub unit: stage_run_units::StageRunUnitRow,
    pub plan: stage_teams::StageTeamPlanRow,
    pub work_item: stage_teams::StageWorkItemRow,
    pub worker: StageWorkerRunRow,
    pub message_chain_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct ClaimStageAggregatorRow {
    pub claim: ClaimStageWorkItemRow,
    pub expected_dispatch_epoch: i64,
    pub expected_manifest_hash: String,
}

#[derive(Debug, Clone)]
pub struct ClaimStageTeamLeaderRow {
    pub claim: ClaimStageWorkItemRow,
}

#[derive(Debug, Clone)]
pub struct ParkStageTeamLeaderRow {
    pub fence: RuntimeMemoryTxFence,
    pub stage_team_plan_id: Uuid,
    pub leader_work_item_id: Uuid,
    pub expected_work_item_row_version: i64,
    pub checkpoint: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ParkedStageTeamLeaderRow {
    pub plan: stage_teams::StageTeamPlanRow,
    pub work_item: stage_teams::StageWorkItemRow,
    pub worker: StageWorkerRunRow,
    pub dependency_count: i64,
}

#[derive(Debug, Clone)]
pub struct BindStageTeamLeaderFinalSubmitterRow {
    pub fence: RuntimeMemoryTxFence,
    pub stage_team_plan_id: Uuid,
    pub leader_work_item_id: Uuid,
    pub expected_plan_row_version: i64,
    pub expected_dispatch_epoch: i64,
    pub expected_manifest_hash: String,
}

#[derive(Debug, Clone)]
pub struct BoundStageTeamLeaderFinalSubmitterRow {
    pub plan: stage_teams::StageTeamPlanRow,
    pub barrier: stage_teams::StageTeamBarrierRow,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReopenStageTeamLeaderAfterGateBlockRow {
    pub request_id: String,
    pub fence: RuntimeMemoryTxFence,
    pub stage_team_plan_id: Uuid,
    pub leader_work_item_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub expected_dispatch_epoch: i64,
    pub expected_manifest_hash: String,
    pub gate_decision_hash: String,
    pub gap_manifest: serde_json::Value,
    pub gap_manifest_hash: String,
    pub checkpoint: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ReopenedStageTeamLeaderAfterGateBlockRow {
    pub plan: stage_teams::StageTeamPlanRow,
    pub unit: StageRunUnitRow,
    pub gap: Option<stage_teams::StageTeamUnitGapRow>,
    pub repair_generation: i32,
    pub fuel_exhausted: bool,
    pub leader_work_item: stage_teams::StageWorkItemRow,
    pub leader_worker: StageWorkerRunRow,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestStageWorkerRow {
    pub fence: RuntimeMemoryTxFence,
    pub stage_team_plan_id: Uuid,
    pub parent_work_item_id: Uuid,
    pub expected_dispatch_epoch: i64,
    pub requested_role: String,
    pub requested_kind: String,
    pub subject_refs: Vec<serde_json::Value>,
    pub reason: String,
    pub output_schema: serde_json::Value,
    pub budget_hint: serde_json::Value,
    pub dedupe_key: String,
    pub request_sha256: String,
}

#[derive(Debug, Clone)]
pub struct RequestedStageWorkerRow {
    pub plan: stage_teams::StageTeamPlanRow,
    pub request: stage_teams::StageWorkerRequestRow,
    pub work_item: Option<stage_teams::StageWorkItemRow>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseStageRequestEpochRow {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub expected_dispatch_epoch: i64,
    pub expected_plan_row_version: i64,
}

#[derive(Debug, Clone)]
pub struct ClosedStageRequestEpochRow {
    pub plan: stage_teams::StageTeamPlanRow,
    pub barrier: stage_teams::StageTeamBarrierRow,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadStageTeamBarrierRow {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub dispatch_epoch: i64,
}

#[derive(Debug, Clone)]
pub struct FinalizeStageTeamUnitRow {
    pub stage_team_plan_id: Uuid,
    pub aggregator_work_item_id: Uuid,
    pub expected_dispatch_epoch: i64,
    pub expected_manifest_hash: String,
    pub final_seal: FinalizeUnitPassRow,
}

#[derive(Debug, Clone)]
pub struct FinalizedStageTeamUnitRow {
    pub plan: stage_teams::StageTeamPlanRow,
    pub aggregator_work_item: stage_teams::StageWorkItemRow,
    pub finalized: FinalizedUnitPassRow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockStageTeamUnitRow {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub expected_dispatch_epoch: i64,
    pub expected_manifest_hash: String,
}

#[derive(Debug, Clone)]
pub struct BlockedStageTeamUnitRow {
    pub plan: stage_teams::StageTeamPlanRow,
    pub aggregator_work_item: stage_teams::StageWorkItemRow,
    pub unit: StageRunUnitRow,
    pub barrier: stage_teams::StageTeamBarrierRow,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenStageTeamRepairRow {
    pub request_id: String,
    pub fence: RuntimeMemoryTxFence,
    pub stage_team_plan_id: Uuid,
    pub aggregator_work_item_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub expected_dispatch_epoch: i64,
    pub expected_manifest_hash: String,
    pub gate_decision_hash: String,
    pub gap_manifest: serde_json::Value,
    pub gap_manifest_hash: String,
}

#[derive(Debug, Clone)]
pub struct OpenedStageTeamRepairRow {
    pub plan: stage_teams::StageTeamPlanRow,
    pub unit: StageRunUnitRow,
    pub gap: stage_teams::StageTeamUnitGapRow,
    pub generation: Option<stage_teams::StageTeamRepairGenerationRow>,
    pub repair_work_item: Option<stage_teams::StageWorkItemRow>,
    pub aggregator_work_item: Option<stage_teams::StageWorkItemRow>,
    pub aggregator_worker: StageWorkerRunRow,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct FinishWorkerAttemptRow {
    pub fence: RuntimeMemoryTxFence,
    pub expected_status: crate::repo::stage_worker_runs::StageWorkerRunStatus,
    pub next_status: crate::repo::stage_worker_runs::StageWorkerRunStatus,
    pub expected_unit_status: crate::repo::stage_run_units::StageRunUnitStatus,
    pub expected_unit_row_version: i64,
    pub next_unit_status: crate::repo::stage_run_units::StageRunUnitStatus,
    pub checkpoint: serde_json::Value,
    pub evidence_watermark: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct FinishedWorkerAttemptRow {
    pub unit: StageRunUnitRow,
    pub worker: StageWorkerRunRow,
}

#[derive(Debug, Clone)]
pub struct PauseWorkerForContinuationRow {
    pub fence: RuntimeMemoryTxFence,
    pub expected_unit_row_version: i64,
    pub checkpoint: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMemoryRecordSource {
    Legacy,
    V2,
    LegacyFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadWorkerCheckpointRow {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub worker_run_id: Uuid,
    pub selected_source: Option<RuntimeMemoryRecordSource>,
}

#[derive(Debug, Clone)]
pub struct LoadedWorkerCheckpointRow {
    pub source: RuntimeMemoryRecordSource,
    pub worker: StageWorkerRunRow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointBoundWorkerChainRow {
    pub fence: RuntimeMemoryTxFence,
    pub message_chain_id: Uuid,
    pub chain: serde_json::Value,
    pub checkpoint: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadBoundWorkerChainRow {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub worker_run_id: Uuid,
    pub message_chain_id: Uuid,
    pub session_id: Uuid,
    pub agent: AgentType,
    pub selected_source: Option<RuntimeMemoryRecordSource>,
}

#[derive(Debug, Clone)]
pub struct LoadedBoundWorkerChainRow {
    pub source: RuntimeMemoryRecordSource,
    pub worker: StageWorkerRunRow,
    pub chain: serde_json::Value,
}

/// Server-owned final-seal command. The caller supplies only trusted Gate
/// output plus canonical key hints; catalog timestamps, hashes, ownership and
/// the persisted handoff payload are rebuilt under database locks.
#[derive(Debug, Clone)]
pub struct FinalizeUnitPassRow {
    pub fence: RuntimeMemoryTxFence,
    pub deliverable_submission_id: Uuid,
    pub expected_unit_status: stage_run_units::StageRunUnitStatus,
    pub expected_unit_row_version: i64,
    pub scope_hash: String,
    pub gate_decision: serde_json::Value,
    pub gate_decision_hash: String,
    pub aggregate_pass_token_hash: Option<String>,
    pub canonical_fact_keys: Vec<CanonicalFactKey>,
    pub typed_claims: Vec<serde_json::Value>,
    pub coverage_watermark: serde_json::Value,
    pub evidence_ids: Vec<i64>,
    pub terminal_checkpoint: serde_json::Value,
    pub candidate_acceptance: Option<super::attack_candidates::CandidateAcceptanceInput>,
}

#[derive(Debug, Clone)]
pub struct FinalizedUnitPassRow {
    pub unit: StageRunUnitRow,
    pub worker: StageWorkerRunRow,
    pub handoff: StageHandoffRow,
    pub canonical_fact_refs: Vec<CanonicalFactRef>,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct CloseWaveGatePassRow {
    pub final_seal: FinalizeUnitPassRow,
    pub wave_id: Uuid,
    pub next_wave_limit: i64,
    pub continuation_pass_watermark: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum ClosedWaveGatePassRow {
    WaitingBackground {
        unit: StageRunUnitRow,
        worker: StageWorkerRunRow,
        next_wave: stage_asset_waves::StageAssetWaveWithItems,
    },
    Finalized(FinalizedUnitPassRow),
}

#[derive(Debug, sqlx::FromRow)]
struct CliScopeOrganizationIdentity {
    id: Uuid,
    name: String,
    parent_id: Option<Uuid>,
}

fn validate_cli_scope_shape(scope: &CliRuntimeScopeRow) -> RuntimeMemoryStoreResult<()> {
    let Some(root) = scope.units.first() else {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "cli_scope_empty",
        });
    };
    if root.organization_id != scope.root_organization_id
        || root.parent_organization_id.is_some()
        || root.depth != 0
        || root.ordinal != 0
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "cli_scope_root_mismatch",
        });
    }
    if !scope.include_subsidiaries && scope.units.len() != 1 {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "cli_root_only_scope_has_descendants",
        });
    }

    let mut seen_ids = std::collections::HashSet::new();
    let mut seen_ordinals = std::collections::HashSet::new();
    let mut depth_by_id = std::collections::HashMap::new();
    for unit in &scope.units {
        if unit.organization_name.trim().is_empty()
            || unit.depth < 0
            || unit.ordinal < 0
            || !seen_ids.insert(unit.organization_id)
            || !seen_ordinals.insert(unit.ordinal)
        {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "cli_scope_unit_malformed",
            });
        }
        if unit.organization_id != scope.root_organization_id {
            let Some(parent_id) = unit.parent_organization_id else {
                return Err(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "cli_scope_parent_missing",
                });
            };
            if depth_by_id.get(&parent_id).copied() != Some(unit.depth - 1) {
                return Err(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "cli_scope_parent_not_selected",
                });
            }
            let ownership = unit
                .ownership_percent
                .as_deref()
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| value.is_finite());
            if ownership.is_none_or(|value| value < f64::from(scope.subsidiary_threshold)) {
                return Err(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "cli_scope_ownership_below_threshold",
                });
            }
        }
        depth_by_id.insert(unit.organization_id, unit.depth);
    }
    if seen_ordinals.len() != scope.units.len()
        || !(0..scope.units.len() as i32).all(|ordinal| seen_ordinals.contains(&ordinal))
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "cli_scope_ordinal_gap",
        });
    }
    Ok(())
}

async fn freeze_cli_scope_with_connection(
    connection: &mut sqlx::PgConnection,
    operation_id: Uuid,
    project_scope_id: Uuid,
    project_path: &str,
    stage_execution_id: Uuid,
    decision_mode: operation_scope_decisions::ScopeDecisionMode,
    scope: &CliRuntimeScopeRow,
) -> RuntimeMemoryStoreResult<FrozenOperationOrgScope> {
    validate_cli_scope_shape(scope)?;

    for unit in &scope.units {
        let live = sqlx::query_as::<_, CliScopeOrganizationIdentity>(
            r#"SELECT id, name, parent_id
                 FROM organizations
                WHERE id=$1 AND project_path=$2
                FOR SHARE"#,
        )
        .bind(unit.organization_id)
        .bind(project_path)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "cli_scope_organization",
        })?;
        let expected_parent = if unit.organization_id == scope.root_organization_id {
            live.parent_id
        } else {
            unit.parent_organization_id
        };
        if live.id != unit.organization_id
            || live.name != unit.organization_name
            || live.parent_id != expected_parent
        {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "cli_scope_live_identity_changed",
            });
        }
    }

    let units = scope
        .units
        .iter()
        .map(|unit| operation_scope_decisions::ApprovedOrgUnit {
            decision_row_id: format!("cli:{}", unit.organization_id),
            candidate_id: String::new(),
            organization_id: unit.organization_id,
            parent_organization_id: unit.parent_organization_id,
            organization_name: unit.organization_name.clone(),
            depth: unit.depth,
            ordinal: unit.ordinal,
            ownership_percent: unit.ownership_percent.clone(),
            aliases: Vec::new(),
            domains: Vec::new(),
            approval_source: unit.approval_source.clone(),
        })
        .collect::<Vec<_>>();
    let decision_rows = serde_json::json!({
        "schema_version": 1,
        "source": decision_mode.as_str(),
        "include_subsidiaries": scope.include_subsidiaries,
        "subsidiary_threshold": scope.subsidiary_threshold,
        "approved_units": units,
    });
    let decision_id = Uuid::new_v4();
    let hash_payload = serde_json::json!({
        "schema_version": 1,
        "operation_id": operation_id,
        "project_scope_id": project_scope_id,
        "stage_execution_id": stage_execution_id,
        "root_organization_id": scope.root_organization_id,
        "mode": decision_mode.as_str(),
        "choice_tool_call_id": Value::Null,
        "proposal_tool_call_id": Value::Null,
        "review_tool_call_id": Value::Null,
        "decision_rows": decision_rows,
    });
    let decision = operation_scope_decisions::ApprovedOrgScopeDecision {
        id: decision_id,
        operation_id,
        project_scope_id,
        stage_execution_id,
        root_organization_id: scope.root_organization_id,
        mode: decision_mode,
        units,
        choice_tool_call_id: None,
        proposal_tool_call_id: None,
        review_tool_call_id: None,
        decision_rows,
        decision_hash: operation_scope_decisions::sha256_json(&hash_payload),
    };
    let draft = operation_org_scope::NewOperationOrgScope::from_decision(
        Uuid::new_v4(),
        project_path.to_string(),
        &decision,
    )
    .map_err(map_scope_freeze_error)?;
    operation_org_scope::freeze_with_connection(connection, &draft)
        .await
        .map_err(map_scope_freeze_error)
}

pub(crate) async fn reconcile_deployment_rollouts_best_effort(
    pool: &sqlx::PgPool,
    trigger: &'static str,
) {
    // Runtime is the dependency and therefore always reconciles first. Attack
    // may then consume the new compatible runtime default in its own
    // transaction; neither best-effort attempt is coupled to business truth.
    runtime_memory_rollout::reconcile_best_effort(pool, trigger).await;
    attack_execution_rollout::reconcile_attack_execution_rollout_best_effort(pool, trigger).await;
}

/// Atomically create the task, operation, and initial stage-execution roots for
/// a new runtime operation.
/// The contract is read from the persisted singleton under `FOR SHARE` and the
/// project identity must still be active. Any failure before commit rolls both
/// inserts back together.
pub async fn create_runtime_operation(
    pool: &sqlx::PgPool,
    input: &CreateRuntimeOperationRow,
) -> RuntimeMemoryStoreResult<CreatedRuntimeOperationRow> {
    create_runtime_operation_inner(pool, input, None).await
}

pub async fn create_runtime_operation_with_stage_fork(
    pool: &sqlx::PgPool,
    input: &CreateRuntimeOperationRow,
    stage_fork: &StageForkCreateRow,
) -> RuntimeMemoryStoreResult<CreatedRuntimeOperationRow> {
    create_runtime_operation_inner(pool, input, Some(stage_fork)).await
}

async fn create_runtime_operation_inner(
    pool: &sqlx::PgPool,
    input: &CreateRuntimeOperationRow,
    stage_fork: Option<&StageForkCreateRow>,
) -> RuntimeMemoryStoreResult<CreatedRuntimeOperationRow> {
    // Reconcile in its own transaction before freezing either deployment
    // contract. A not-ready cohort is a typed no-op; an infrastructure failure
    // is logged but does not make operation creation unavailable.
    reconcile_deployment_rollouts_best_effort(pool, "create_runtime_operation").await;
    let mut tx = pool.begin().await?;
    runtime_memory_rollout::lock_execution_rollout_pair(&mut tx).await?;
    let rollout = runtime_memory_rollout::get_for_share(&mut *tx).await?;
    let attack_rollout = attack_execution_rollout::get_for_share(&mut tx).await?;
    let attack_contract = match attack_rollout.contract.as_str() {
        "legacy" => golish_core::AttackExecutionContract::Legacy,
        "dual_write_read_legacy" => golish_core::AttackExecutionContract::DualWriteReadLegacy,
        "dual_write_read_v2_fallback" => {
            golish_core::AttackExecutionContract::DualWriteReadV2Fallback
        }
        "v2_only" => golish_core::AttackExecutionContract::V2Only,
        _ => {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "unknown_attack_execution_contract",
            });
        }
    };
    let project = project_scopes::get_active_for_share(&mut *tx, input.project_scope_id)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: project_scopes::TABLE_NAME,
        })?;
    let task = tasks::insert_with_id(
        &mut *tx,
        input.operation_id,
        input.session_id,
        input.title.as_deref(),
        &input.input,
    )
    .await?;
    let operation = operation_state::insert_with_executor(
        &mut *tx,
        input.operation_id,
        &input.profile,
        &input.entry_stage,
        &rollout.contract,
        input.project_scope_id,
        attack_contract,
    )
    .await?;
    operation_turns::insert_initial_with_executor(&mut *tx, input.operation_id, &input.input)
        .await?;
    let initial_stage_execution = stage_runs::insert_with_executor(
        &mut *tx,
        input.initial_stage_execution_id,
        input.operation_id,
        &input.entry_stage,
    )
    .await?;
    let mut frozen_scope = None;
    if let Some(cli_scope) = input.cli_scope.as_ref() {
        if rollout.contract == runtime_memory_rollout::RuntimeMemoryContract::LegacyV1.as_str() {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "cli_scope_requires_v2_writing_contract",
            });
        }
        frozen_scope = Some(
            freeze_cli_scope_with_connection(
                &mut tx,
                input.operation_id,
                input.project_scope_id,
                &project.canonical_project_path,
                initial_stage_execution.id,
                if stage_fork.is_some() {
                    operation_scope_decisions::ScopeDecisionMode::ReuseReconfirmed
                } else {
                    operation_scope_decisions::ScopeDecisionMode::CliFlags
                },
                cli_scope,
            )
            .await?,
        );
    }
    if stage_fork.is_some() && input.cli_scope.is_none() {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_fork_requires_frozen_scope",
        });
    }
    if let Some(stage_fork) = stage_fork {
        let target_scope_snapshot_id = frozen_scope.as_ref().map(|scope| scope.snapshot.id).ok_or(
            RuntimeMemoryStoreError::Missing {
                entity: "stage_fork_target_scope",
            },
        )?;
        super::operation_stage_forks::materialize_with_connection(
            &mut tx,
            &super::operation_stage_forks::MaterializeOperationStageFork {
                operation_id: input.operation_id,
                target_scope_snapshot_id,
                project_scope_id: input.project_scope_id,
                source_operation_id: stage_fork.source_operation_id,
                source_scope_snapshot_id: stage_fork.source_scope_snapshot_id,
                entry_stage: stage_fork.entry_stage.clone(),
                terminal_stage: stage_fork.terminal_stage.clone(),
                adopted_stage_kinds: stage_fork.adopted_stage_kinds.clone(),
            },
        )
        .await
        .map_err(map_stage_fork_error)?;
    }
    tx.commit().await?;
    Ok(CreatedRuntimeOperationRow {
        task,
        operation,
        initial_stage_execution_id: initial_stage_execution.id,
    })
}

const LOCK_OPERATION_STATE_ROW_SQL: &str = r#"SELECT operation_id, profile, current_stage,
    runtime_memory_contract, project_scope_id, stage_started_at,
    last_evidence_audit_id, last_classification_id, last_scope_version,
    state_blob, superseded_by, engagement_org_id
FROM operation_state
WHERE operation_id = $1
FOR UPDATE"#;

const UPDATE_OPERATION_STAGE_SQL: &str = r#"UPDATE operation_state
SET current_stage = $2,
    stage_started_at = NOW()
WHERE operation_id = $1
  AND current_stage = $3
  AND superseded_by IS NULL
RETURNING operation_id, profile, current_stage, runtime_memory_contract,
          project_scope_id, stage_started_at, last_evidence_audit_id,
          last_classification_id, last_scope_version, state_blob,
          superseded_by, engagement_org_id"#;

/// Atomically close the exact current execution, open the next one, and move
/// the operation cursor. The operation row is the serialization lock while the
/// foundation remains compatible with legacy duplicate-active rows.
pub async fn transition_stage_execution(
    pool: &sqlx::PgPool,
    input: &TransitionStageExecutionRow,
) -> RuntimeMemoryStoreResult<TransitionedStageExecutionRow> {
    transition_stage_execution_inner(pool, input, false).await
}

async fn transition_stage_execution_inner(
    pool: &sqlx::PgPool,
    input: &TransitionStageExecutionRow,
    inject_failure_after_insert: bool,
) -> RuntimeMemoryStoreResult<TransitionedStageExecutionRow> {
    if input.current_stage_execution_id == input.next_stage_execution_id {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_execution_identity_reused",
        });
    }
    if input.next_stage.trim().is_empty() {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "empty_next_stage",
        });
    }

    let mut tx = pool.begin().await?;
    let locked_operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    if locked_operation.superseded_by.is_some() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "operation_superseded",
        });
    }

    let active =
        stage_runs::list_active_for_operation_with_executor(&mut *tx, input.operation_id).await?;
    let current = match active.as_slice() {
        [] => {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "missing_active_stage_execution",
            });
        }
        [current] => current,
        _ => {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "multiple_active_stage_executions",
            });
        }
    };
    if current.id != input.current_stage_execution_id {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "active_stage_execution_mismatch",
        });
    }
    if current.stage_kind != locked_operation.current_stage {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "operation_stage_execution_mismatch",
        });
    }

    let previous_stage_execution = stage_runs::mark_terminal_cas(
        &mut *tx,
        input.operation_id,
        input.current_stage_execution_id,
        stage_runs::StageExecutionTerminal::Completed,
    )
    .await?;
    let current_stage_execution = stage_runs::insert_with_executor(
        &mut *tx,
        input.next_stage_execution_id,
        input.operation_id,
        &input.next_stage,
    )
    .await?;

    if inject_failure_after_insert {
        tx.rollback().await?;
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "injected_after_new_stage_execution",
        });
    }

    let mut operation = sqlx::query_as::<_, OperationStateRow>(UPDATE_OPERATION_STAGE_SQL)
        .bind(input.operation_id)
        .bind(&input.next_stage)
        .bind(&locked_operation.current_stage)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "operation_stage_cursor_changed",
        })?;
    let contract = frozen_runtime_contract(&locked_operation)?;
    if contract_writes_legacy_checkpoint(contract) {
        let mut legacy_blob = locked_operation.state_blob.clone();
        apply_legacy_stage_execution_mirror(
            &mut legacy_blob,
            &locked_operation.profile,
            &input.next_stage,
            input.next_stage_execution_id,
        );
        write_locked_legacy_state_blob(
            &mut tx,
            input.operation_id,
            &locked_operation.runtime_memory_contract,
            &legacy_blob,
        )
        .await?;
        operation.state_blob = legacy_blob;
    }

    let final_active =
        stage_runs::list_active_for_operation_with_executor(&mut *tx, input.operation_id).await?;
    if final_active.len() != 1
        || final_active[0].id != input.next_stage_execution_id
        || final_active[0].stage_kind != input.next_stage
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "next_stage_execution_not_unique",
        });
    }

    tx.commit().await?;
    Ok(TransitionedStageExecutionRow {
        previous_stage_execution,
        current_stage_execution,
        operation,
    })
}

/// Atomically close the exact active execution at a projected DAG terminal and
/// finish the operation's task with its generated result. No successor is
/// created and the operation cursor remains on the terminal stage.
///
/// An exact response-loss replay is idempotent: the same completed execution
/// and identical finished task result return the persisted row. Any identity,
/// stage, status, or result drift fails closed.
pub async fn complete_terminal_stage_execution(
    pool: &sqlx::PgPool,
    input: &CompleteTerminalStageExecutionRow,
) -> RuntimeMemoryStoreResult<stage_runs::StageRunRow> {
    complete_terminal_stage_execution_inner(pool, input, false).await
}

async fn complete_terminal_stage_execution_inner(
    pool: &sqlx::PgPool,
    input: &CompleteTerminalStageExecutionRow,
    inject_failure_after_stage_close: bool,
) -> RuntimeMemoryStoreResult<stage_runs::StageRunRow> {
    if input.terminal_stage.trim().is_empty() {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "empty_terminal_stage",
        });
    }

    let mut tx = pool.begin().await?;
    let locked_operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&locked_operation)?;
    if locked_operation.current_stage != input.terminal_stage {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "operation_terminal_stage_mismatch",
        });
    }

    let active =
        stage_runs::list_active_for_operation_with_executor(&mut *tx, input.operation_id).await?;
    match active.as_slice() {
        [] => {
            let sql = format!(
                "SELECT {} FROM stage_runs \
                 WHERE id=$1 AND operation_id=$2 FOR UPDATE",
                r#"id, operation_id, stage_kind, started_at,
                   completed_at, status, active_sprint_contract_id"#
            );
            let completed = sqlx::query_as::<_, stage_runs::StageRunRow>(&sql)
                .bind(input.current_stage_execution_id)
                .bind(input.operation_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(RuntimeMemoryStoreError::Missing {
                    entity: "stage_runs",
                })?;
            if completed.stage_kind != input.terminal_stage || completed.status != "completed" {
                return Err(RuntimeMemoryStoreError::Conflict {
                    code: "terminal_stage_completion_replay_mismatch",
                });
            }
            let task = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE id=$1 FOR UPDATE")
                .bind(input.operation_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(RuntimeMemoryStoreError::Missing { entity: "tasks" })?;
            if task.status != crate::models::TaskStatus::Finished
                || task.result.as_deref() != Some(input.task_result.as_str())
            {
                return Err(RuntimeMemoryStoreError::Conflict {
                    code: "terminal_task_completion_replay_mismatch",
                });
            }
            tx.commit().await?;
            return Ok(completed);
        }
        [_] => {}
        _ => {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "multiple_active_stage_executions",
            });
        }
    }

    let current = &active[0];
    if current.id != input.current_stage_execution_id {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "active_stage_execution_mismatch",
        });
    }
    if current.stage_kind != input.terminal_stage {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "operation_stage_execution_mismatch",
        });
    }

    let task = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE id=$1 FOR UPDATE")
        .bind(input.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing { entity: "tasks" })?;
    if task.status != crate::models::TaskStatus::Running || task.result.is_some() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "terminal_task_not_running",
        });
    }

    let completed = stage_runs::mark_terminal_cas(
        &mut *tx,
        input.operation_id,
        input.current_stage_execution_id,
        stage_runs::StageExecutionTerminal::Completed,
    )
    .await?;
    if inject_failure_after_stage_close {
        tx.rollback().await?;
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "injected_after_terminal_stage_close",
        });
    }

    let finished = sqlx::query_as::<_, Task>(
        r#"UPDATE tasks
              SET result=$2, status='finished', updated_at=NOW()
            WHERE id=$1 AND status='running' AND result IS NULL
            RETURNING *"#,
    )
    .bind(input.operation_id)
    .bind(&input.task_result)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Conflict {
        code: "terminal_task_finish_cas_failed",
    })?;
    if finished.status != crate::models::TaskStatus::Finished
        || finished.result.as_deref() != Some(input.task_result.as_str())
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "terminal_task_finish_mismatch",
        });
    }
    if !stage_runs::list_active_for_operation_with_executor(&mut *tx, input.operation_id)
        .await?
        .is_empty()
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "terminal_stage_execution_still_active",
        });
    }

    tx.commit().await?;
    Ok(completed)
}

/// Developer reset compound: supersede live relational runtime, invalidate
/// downstream handoffs, rewind the graph/legacy mirror, close the active
/// execution, and open the selected replacement execution in one transaction.
///
/// The frozen migration currently permits only `failed` as the compatible
/// terminal value for `stage_runs`; the state-blob audit marker records the
/// semantic `superseded` disposition until Task 9 widens that CHECK constraint.
pub async fn supersede_stage_checkpoint(
    pool: &sqlx::PgPool,
    input: &SupersedeStageCheckpointRow,
) -> RuntimeMemoryStoreResult<SupersededStageCheckpointStats> {
    if input.expected_current_stage.trim().is_empty()
        || input.selected_stage.trim().is_empty()
        || input.affected_stage_kinds.is_empty()
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_checkpoint_reset",
        });
    }
    let mut affected = input.affected_stage_kinds.clone();
    affected.sort();
    affected.dedup();

    let mut tx = pool.begin().await?;
    let locked_operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&locked_operation)?;
    if locked_operation.current_stage != input.expected_current_stage {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "operation_stage_cursor_changed",
        });
    }
    let contract = frozen_runtime_contract(&locked_operation)?;
    let mut stats = SupersededStageCheckpointStats::default();
    let mut active_execution_ids = Vec::new();
    let mut next_state_blob = if contract == runtime_memory_rollout::RuntimeMemoryContract::V2Only {
        v2_only_reset_state_blob(&locked_operation.state_blob)
    } else {
        input.next_state_blob.clone()
    };
    let mut dual_mutated_worker_ids = Vec::new();

    if let Some(replacement_stage_execution_id) = input.replacement_stage_execution_id {
        let active =
            stage_runs::list_active_for_operation_with_executor(&mut *tx, input.operation_id)
                .await?;
        if active.len() > 1 {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "multiple_active_stage_executions",
            });
        }
        if contract_writes_v2(contract) && active.is_empty() {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "missing_active_stage_execution",
            });
        }
        if let Some(execution) = active.first() {
            if execution.stage_kind != locked_operation.current_stage {
                return Err(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "operation_stage_execution_mismatch",
                });
            }
            active_execution_ids.push(execution.id);
        }

        if contract_writes_v2(contract) {
            let affected_workers = sqlx::query_as::<_, StageWorkerRunRow>(
                r#"SELECT worker.*
                     FROM stage_worker_runs worker
                    WHERE worker.operation_id=$1
                      AND worker.status<>'superseded'
                      AND (
                          worker.stage_execution_id=ANY($2)
                          OR EXISTS (
                              SELECT 1 FROM stage_run_units unit
                              WHERE unit.id=worker.stage_run_unit_id
                                AND unit.operation_id=$1
                                AND unit.stage_kind=ANY($3)
                          )
                      )
                    ORDER BY worker.id
                    FOR UPDATE"#,
            )
            .bind(input.operation_id)
            .bind(&active_execution_ids)
            .bind(&affected)
            .fetch_all(&mut *tx)
            .await?;
            let reset_tool_result = serde_json::to_string(&serde_json::json!({
                "kind": "runtime_stage_checkpoint_superseded",
                "outcome": "unknown_not_replayed",
                "reason": "developer_reset",
                "schema_version": 1,
            }))
            .map_err(|_| RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_checkpoint_reset_tool_result_invalid",
            })?;
            for worker in &affected_workers {
                let Some(active_tool_call_id) = worker.active_tool_call_id else {
                    continue;
                };
                let tool_rows = sqlx::query(
                    r#"UPDATE tool_calls
                          SET status='failed',result=$2,updated_at=NOW()
                        WHERE id=$1 AND worker_run_id=$3 AND operation_id=$4
                          AND stage_execution_id=$5 AND stage_run_unit_id=$6
                          AND organization_id=$7 AND attempt_epoch=$8
                          AND status IN ('received','running')"#,
                )
                .bind(active_tool_call_id)
                .bind(&reset_tool_result)
                .bind(worker.id)
                .bind(worker.operation_id)
                .bind(worker.stage_execution_id)
                .bind(worker.stage_run_unit_id)
                .bind(worker.organization_id)
                .bind(worker.attempt_epoch)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                if tool_rows == 0 {
                    let exact_terminal = sqlx::query_scalar::<_, bool>(
                        r#"SELECT EXISTS(
                               SELECT 1 FROM tool_calls
                                WHERE id=$1 AND worker_run_id=$2 AND operation_id=$3
                                  AND stage_execution_id=$4 AND stage_run_unit_id=$5
                                  AND organization_id=$6 AND attempt_epoch=$7
                                  AND status IN ('finished','failed')
                           )"#,
                    )
                    .bind(active_tool_call_id)
                    .bind(worker.id)
                    .bind(worker.operation_id)
                    .bind(worker.stage_execution_id)
                    .bind(worker.stage_run_unit_id)
                    .bind(worker.organization_id)
                    .bind(worker.attempt_epoch)
                    .fetch_one(&mut *tx)
                    .await?;
                    if !exact_terminal {
                        return Err(RuntimeMemoryStoreError::IdentityMismatch {
                            code: "stage_checkpoint_reset_active_tool_identity_mismatch",
                        });
                    }
                } else if tool_rows != 1 {
                    return Err(RuntimeMemoryStoreError::Conflict {
                        code: "stage_checkpoint_reset_active_tool_cas_failed",
                    });
                }
            }
            let superseded_workers = sqlx::query_as::<_, StageWorkerRunRow>(
                r#"UPDATE stage_worker_runs worker
                      SET status='superseded',
                          active_tool_call_id=NULL, active_tool_started_at=NULL,
                          lease_token=NULL, lease_owner=NULL,
                          lease_acquired_at=NULL, lease_expires_at=NULL,
                          heartbeat_at=NULL, updated_at=NOW(), terminal_at=NOW()
                    WHERE worker.operation_id=$1
                      AND worker.status<>'superseded'
                      AND (
                          worker.stage_execution_id=ANY($2)
                          OR EXISTS (
                              SELECT 1 FROM stage_run_units unit
                              WHERE unit.id=worker.stage_run_unit_id
                                AND unit.operation_id=$1
                                AND unit.stage_kind=ANY($3)
                          )
                      )
                    RETURNING worker.*"#,
            )
            .bind(input.operation_id)
            .bind(&active_execution_ids)
            .bind(&affected)
            .fetch_all(&mut *tx)
            .await?;
            stats.workers_superseded = superseded_workers.len() as u64;
            if contract_writes_legacy_mirror(contract) {
                for worker in &superseded_workers {
                    let unit =
                        stage_run_units::get_with_executor(&mut *tx, worker.stage_run_unit_id)
                            .await?
                            .ok_or(RuntimeMemoryStoreError::Missing {
                                entity: stage_run_units::TABLE_NAME,
                            })?;
                    let organization_name = frozen_organization_name(&mut tx, &unit).await?;
                    apply_legacy_worker_mirror(
                        &mut next_state_blob,
                        &unit.stage_kind,
                        &organization_name,
                        worker,
                    );
                    dual_mutated_worker_ids.push(worker.id);
                }
            }
            stats.units_superseded = sqlx::query(
                r#"UPDATE stage_run_units
                      SET status='superseded', row_version=row_version+1,
                          updated_at=NOW(), terminal_at=NOW()
                    WHERE operation_id=$1
                      AND status<>'superseded'
                      AND (stage_execution_id=ANY($2) OR stage_kind=ANY($3))"#,
            )
            .bind(input.operation_id)
            .bind(&active_execution_ids)
            .bind(&affected)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            stats.handoffs_invalidated = sqlx::query(
                r#"UPDATE stage_handoffs
                      SET invalidated_at=NOW()
                    WHERE operation_id=$1
                      AND from_stage_kind=ANY($2)
                      AND invalidated_at IS NULL"#,
            )
            .bind(input.operation_id)
            .bind(&affected)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        }

        if let Some(active_execution_id) = active_execution_ids.first().copied() {
            stage_runs::mark_terminal_cas(
                &mut *tx,
                input.operation_id,
                active_execution_id,
                stage_runs::StageExecutionTerminal::Failed,
            )
            .await?;
            stats.executions_superseded = 1;
        }
        stage_runs::insert_with_executor(
            &mut *tx,
            replacement_stage_execution_id,
            input.operation_id,
            &input.selected_stage,
        )
        .await?;

        if contract_writes_v2(contract) {
            let scope = operation_org_scope::load_for_operation_with_connection(
                &mut tx,
                input.operation_id,
            )
            .await
            .map_err(map_scope_freeze_error)?;
            match (scope, input.replacement_specialist.as_deref()) {
                (Some(scope), specialist) => {
                    if scope.snapshot.sealed_at.is_none() {
                        return Err(RuntimeMemoryStoreError::Conflict {
                            code: "operation_scope_not_sealed",
                        });
                    }
                    let selected_scope_units = if specialist.is_some() {
                        scope.units.iter().collect::<Vec<_>>()
                    } else {
                        scope
                            .units
                            .iter()
                            .filter(|unit| {
                                unit.organization_id == scope.snapshot.root_organization_id
                            })
                            .collect::<Vec<_>>()
                    };
                    for scope_unit in selected_scope_units {
                        stage_run_units::insert_with_executor(
                            &mut *tx,
                            &stage_run_units::NewStageRunUnit {
                                id: Uuid::new_v4(),
                                operation_id: input.operation_id,
                                stage_execution_id: replacement_stage_execution_id,
                                scope_snapshot_id: scope.snapshot.id,
                                organization_id: scope_unit.organization_id,
                                stage_kind: input.selected_stage.clone(),
                                generation: 1,
                                specialist: specialist.map(str::to_string),
                            },
                        )
                        .await?;
                    }
                }
                (None, Some(_)) => {
                    return Err(RuntimeMemoryStoreError::Missing {
                        entity: "operation_org_scope_snapshots",
                    });
                }
                (None, None) if input.selected_stage != "scoping" => {
                    return Err(RuntimeMemoryStoreError::Missing {
                        entity: "operation_org_scope_snapshots",
                    });
                }
                (None, None) => {} // exact V2 Scoping pre-freeze reset
            }
        }
    }

    if input.replacement_stage_execution_id.is_some() && contract_writes_v2(contract) {
        if !next_state_blob.is_object() {
            next_state_blob = serde_json::json!({});
        }
        next_state_blob
            .as_object_mut()
            .expect("normalized object")
            .insert(
                "runtime_v2_dev_reset".to_string(),
                serde_json::json!({
                    "semantic_stage_execution_status": "superseded",
                    "compat_stage_run_status": "failed",
                    "superseded_stage_execution_ids": active_execution_ids,
                    "replacement_stage_execution_id": input.replacement_stage_execution_id,
                    "selected_stage": input.selected_stage,
                    "affected_stage_kinds": affected,
                }),
            );
    }
    let next_stage = if input.replacement_stage_execution_id.is_some() {
        input.selected_stage.as_str()
    } else {
        locked_operation.current_stage.as_str()
    };
    let updated = sqlx::query(
        r#"UPDATE operation_state
              SET state_blob=$4,
                  current_stage=$3,
                  stage_started_at=CASE WHEN $5 THEN NOW() ELSE stage_started_at END
            WHERE operation_id=$1
              AND current_stage=$2
              AND superseded_by IS NULL"#,
    )
    .bind(input.operation_id)
    .bind(&input.expected_current_stage)
    .bind(next_stage)
    .bind(&next_state_blob)
    .bind(input.replacement_stage_execution_id.is_some())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if updated != 1 {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "operation_stage_cursor_changed",
        });
    }
    for worker_run_id in &dual_mutated_worker_ids {
        runtime_memory_shadow::persist_worker_sample(&mut tx, *worker_run_id, "developer_reset")
            .await?;
    }

    if let Some(replacement_stage_execution_id) = input.replacement_stage_execution_id {
        let active =
            stage_runs::list_active_for_operation_with_executor(&mut *tx, input.operation_id)
                .await?;
        if active.len() != 1
            || active[0].id != replacement_stage_execution_id
            || active[0].stage_kind != input.selected_stage
        {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "reset_stage_execution_not_unique",
            });
        }
    }
    tx.commit().await?;
    if !dual_mutated_worker_ids.is_empty() {
        reconcile_deployment_rollouts_best_effort(pool, "supersede_stage_checkpoint").await;
    }
    Ok(stats)
}

fn map_scope_decision_error(error: ScopeDecisionError) -> RuntimeMemoryStoreError {
    match error {
        ScopeDecisionError::IdentityMismatch { code } => {
            RuntimeMemoryStoreError::IdentityMismatch { code }
        }
        ScopeDecisionError::Conflict { code } => RuntimeMemoryStoreError::Conflict { code },
        ScopeDecisionError::Missing { entity } => RuntimeMemoryStoreError::Missing { entity },
        ScopeDecisionError::Sqlx(error) => RuntimeMemoryStoreError::Sqlx(error),
        ScopeDecisionError::Repository(error) => RuntimeMemoryStoreError::Repository(error),
    }
}

fn map_scope_freeze_error(error: ScopeFreezeError) -> RuntimeMemoryStoreError {
    match error {
        ScopeFreezeError::IdentityMismatch { code } => {
            RuntimeMemoryStoreError::IdentityMismatch { code }
        }
        ScopeFreezeError::Conflict { code } => RuntimeMemoryStoreError::Conflict { code },
        ScopeFreezeError::Decision(error) => map_scope_decision_error(error),
        ScopeFreezeError::Sqlx(error) => RuntimeMemoryStoreError::Sqlx(error),
    }
}

fn map_stage_fork_error(
    error: super::operation_stage_forks::OperationStageForkError,
) -> RuntimeMemoryStoreError {
    use super::operation_stage_forks::OperationStageForkError;
    match error {
        OperationStageForkError::IdentityMismatch { code } => {
            RuntimeMemoryStoreError::IdentityMismatch { code }
        }
        OperationStageForkError::Conflict { code } => RuntimeMemoryStoreError::Conflict { code },
        OperationStageForkError::Missing { entity } => RuntimeMemoryStoreError::Missing { entity },
        OperationStageForkError::Sqlx(error) => RuntimeMemoryStoreError::Sqlx(error),
    }
}

async fn load_scoping_submission_for_update(
    connection: &mut sqlx::PgConnection,
    input: &FinalizeScopingScopeRow,
) -> RuntimeMemoryStoreResult<StageDeliverableSubmissionRow> {
    sqlx::query_as::<_, StageDeliverableSubmissionRow>(
        r#"SELECT submission.*
             FROM stage_deliverable_submissions AS submission
             JOIN tool_calls AS tool
               ON tool.id=submission.tool_call_record_id
              AND tool.operation_id=submission.operation_id
              AND tool.stage_execution_id=submission.stage_execution_id
            WHERE submission.id=$1
              AND submission.operation_id=$2
              AND submission.stage_execution_id=$3
              AND submission.stage_kind='scoping'
              AND tool.name='submit_stage_deliverable'
              AND tool.status='finished'::toolcall_status
            FOR UPDATE OF submission, tool"#,
    )
    .bind(input.deliverable_submission_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .fetch_optional(connection)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "trusted_scoping_submission",
    })
}

fn validate_scoping_submission_shape(
    submission: &StageDeliverableSubmissionRow,
) -> RuntimeMemoryStoreResult<()> {
    if submission.worker_run_id.is_some()
        || submission.attempt_epoch.is_some()
        || submission.lease_token.is_some()
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "scoping_submission_worker_identity_present",
        });
    }
    let unit_bound = submission.stage_run_unit_id.is_some();
    let org_bound = submission.organization_id.is_some();
    if unit_bound != org_bound {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "scoping_submission_partial_binding",
        });
    }
    Ok(())
}

fn validate_active_scoping_execution(
    active: &[stage_runs::StageRunRow],
    operation: &OperationStateRow,
    input: &FinalizeScopingScopeRow,
) -> RuntimeMemoryStoreResult<()> {
    let execution = match active {
        [] => {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "missing_active_stage_execution",
            });
        }
        [execution] => execution,
        _ => {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "multiple_active_stage_executions",
            });
        }
    };
    if operation.current_stage != "scoping" || execution.stage_kind != "scoping" {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "operation_not_in_scoping",
        });
    }
    if execution.id != input.stage_execution_id {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "active_stage_execution_mismatch",
        });
    }
    Ok(())
}

async fn load_finalized_scoping_replay(
    connection: &mut sqlx::PgConnection,
    input: &FinalizeScopingScopeRow,
    scope: FrozenOperationOrgScope,
) -> RuntimeMemoryStoreResult<FinalizedScopingScopeRow> {
    if scope.snapshot.id != input.scope_snapshot_id
        || scope.snapshot.project_scope_id != input.project_scope_id
        || scope.snapshot.root_organization_id != input.root_organization_id
        || scope.snapshot.sealed_at.is_none()
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "finalized_scope_replay_mismatch",
        });
    }
    let decision = operation_scope_decisions::load_for_execution_with_connection(
        connection,
        input.operation_id,
        input.stage_execution_id,
    )
    .await
    .map_err(map_scope_decision_error)?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "operation_scope_decisions",
    })?;
    if decision.id != scope.snapshot.scope_decision_id
        || decision.project_scope_id != input.project_scope_id
        || decision.root_organization_id != input.root_organization_id
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "finalized_scope_decision_mismatch",
        });
    }
    let root_unit = crate::repo::stage_run_units::get_with_executor(
        &mut *connection,
        input.scoping_root_unit_id,
    )
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "scoping_root_unit",
    })?;
    if root_unit.operation_id != input.operation_id
        || root_unit.stage_execution_id != input.stage_execution_id
        || root_unit.scope_snapshot_id != input.scope_snapshot_id
        || root_unit.organization_id != input.root_organization_id
        || root_unit.stage_kind != "scoping"
        || root_unit.status != "passed"
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "finalized_scoping_root_unit_mismatch",
        });
    }
    let submission = load_scoping_submission_for_update(connection, input).await?;
    validate_scoping_submission_shape(&submission)?;
    if submission.stage_run_unit_id != Some(input.scoping_root_unit_id)
        || submission.organization_id != Some(input.root_organization_id)
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "finalized_scoping_submission_mismatch",
        });
    }
    Ok(FinalizedScopingScopeRow {
        decision,
        scope,
        root_unit,
        submission,
        replayed: true,
    })
}

/// Atomically freeze the exact approved organization scope, bind the trusted
/// pre-freeze Scoping submission to its newly seeded root unit, and pass that
/// unit. The Scoping execution deliberately remains `started`: the Task3
/// stage-entry transaction closes it while opening the next execution, so a
/// successful freeze can never expose an operation with no active execution.
/// Replaying the same complete identity tuple returns the sealed rows without
/// deriving a second decision or mutating the terminal root unit.
pub async fn finalize_scoping_scope(
    pool: &sqlx::PgPool,
    input: &FinalizeScopingScopeRow,
) -> RuntimeMemoryStoreResult<FinalizedScopingScopeRow> {
    let mut tx = pool.begin().await?;
    let locked_operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    if locked_operation.superseded_by.is_some() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "operation_superseded",
        });
    }
    if locked_operation.project_scope_id != Some(input.project_scope_id) {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "operation_project_scope_mismatch",
        });
    }
    let active =
        stage_runs::list_active_for_operation_with_executor(&mut *tx, input.operation_id).await?;
    validate_active_scoping_execution(&active, &locked_operation, input)?;

    if let Some(scope) =
        operation_org_scope::load_for_operation_with_connection(&mut tx, input.operation_id)
            .await
            .map_err(map_scope_freeze_error)?
    {
        let replay = load_finalized_scoping_replay(&mut tx, input, scope).await?;
        tx.commit().await?;
        return Ok(replay);
    }

    // This read validates the immutable trusted row before expensive decision
    // derivation. The later FOR UPDATE plus null-binding CAS is authoritative;
    // the operation lock serializes all conforming finalizers for this run.
    let preliminary_submission = sqlx::query_as::<_, StageDeliverableSubmissionRow>(
        r#"SELECT * FROM stage_deliverable_submissions
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_kind='scoping'"#,
    )
    .bind(input.deliverable_submission_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "trusted_scoping_submission",
    })?;
    validate_scoping_submission_shape(&preliminary_submission)?;
    if preliminary_submission.stage_run_unit_id.is_some() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "scoping_submission_bound_without_scope",
        });
    }

    let project = project_scopes::get_active_for_share(&mut *tx, input.project_scope_id)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: project_scopes::TABLE_NAME,
        })?;
    let decision = operation_scope_decisions::derive_exact_with_connection(
        &mut tx,
        &operation_scope_decisions::ExactScopeDecisionInput {
            operation_id: input.operation_id,
            project_scope_id: input.project_scope_id,
            stage_execution_id: input.stage_execution_id,
            root_organization_id: input.root_organization_id,
        },
    )
    .await
    .map_err(map_scope_decision_error)?;
    let draft = operation_org_scope::NewOperationOrgScope::from_decision(
        input.scope_snapshot_id,
        project.canonical_project_path,
        &decision,
    )
    .map_err(map_scope_freeze_error)?;
    let scope = operation_org_scope::freeze_with_connection(&mut tx, &draft)
        .await
        .map_err(map_scope_freeze_error)?;
    let queued_root_unit = crate::repo::stage_run_units::insert_with_executor(
        &mut *tx,
        &crate::repo::stage_run_units::NewStageRunUnit {
            id: input.scoping_root_unit_id,
            operation_id: input.operation_id,
            stage_execution_id: input.stage_execution_id,
            scope_snapshot_id: input.scope_snapshot_id,
            organization_id: input.root_organization_id,
            stage_kind: "scoping".to_string(),
            generation: 0,
            specialist: None,
        },
    )
    .await?;

    let submission = load_scoping_submission_for_update(&mut tx, input).await?;
    validate_scoping_submission_shape(&submission)?;
    if submission.stage_run_unit_id.is_some() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "scoping_submission_already_bound",
        });
    }
    let bound_tool_rows = sqlx::query(
        r#"UPDATE tool_calls
              SET stage_run_unit_id=$2, organization_id=$3
            WHERE id=$1
              AND operation_id=$4
              AND stage_execution_id=$5
              AND stage_run_unit_id IS NULL
              AND worker_run_id IS NULL
              AND organization_id IS NULL
              AND attempt_epoch IS NULL
              AND lease_token IS NULL"#,
    )
    .bind(submission.tool_call_record_id)
    .bind(input.scoping_root_unit_id)
    .bind(input.root_organization_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if bound_tool_rows != 1 {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "scoping_submission_tool_bind_mismatch",
        });
    }
    let submission = sqlx::query_as::<_, StageDeliverableSubmissionRow>(
        r#"UPDATE stage_deliverable_submissions
              SET stage_run_unit_id=$2, organization_id=$3
            WHERE id=$1
              AND operation_id=$4
              AND stage_execution_id=$5
              AND stage_kind='scoping'
              AND stage_run_unit_id IS NULL
              AND worker_run_id IS NULL
              AND organization_id IS NULL
              AND attempt_epoch IS NULL
              AND lease_token IS NULL
        RETURNING *"#,
    )
    .bind(input.deliverable_submission_id)
    .bind(input.scoping_root_unit_id)
    .bind(input.root_organization_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
        code: "scoping_submission_bind_mismatch",
    })?;

    let running_root_unit = crate::repo::stage_run_units::transition_cas(
        &mut *tx,
        queued_root_unit.id,
        input.operation_id,
        input.stage_execution_id,
        input.root_organization_id,
        crate::repo::stage_run_units::StageRunUnitStatus::Queued,
        queued_root_unit.row_version,
        crate::repo::stage_run_units::StageRunUnitStatus::Running,
        None,
    )
    .await?;
    let pass_watermark = serde_json::json!({
        "deliverable_submission_id": input.deliverable_submission_id,
        "scope_decision_id": decision.id,
        "scope_snapshot_id": input.scope_snapshot_id,
        "scope_hash": scope.snapshot.scope_hash,
    });
    let root_unit = crate::repo::stage_run_units::transition_to_passed_for_final_seal(
        &mut *tx,
        running_root_unit.id,
        input.operation_id,
        input.stage_execution_id,
        input.root_organization_id,
        crate::repo::stage_run_units::StageRunUnitStatus::Running,
        running_root_unit.row_version,
        &pass_watermark,
    )
    .await?;
    let decision = operation_scope_decisions::load_for_execution_with_connection(
        &mut tx,
        input.operation_id,
        input.stage_execution_id,
    )
    .await
    .map_err(map_scope_decision_error)?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "operation_scope_decisions",
    })?;

    // Do not terminalize Scoping here. The next stage-entry transaction closes
    // this exact execution and opens its successor under the same operation
    // lock, preserving the one-active-execution invariant without a gap.
    let final_active =
        stage_runs::list_active_for_operation_with_executor(&mut *tx, input.operation_id).await?;
    validate_active_scoping_execution(&final_active, &locked_operation, input)?;
    tx.commit().await?;
    Ok(FinalizedScopingScopeRow {
        decision,
        scope,
        root_unit,
        submission,
        replayed: false,
    })
}

fn frozen_runtime_contract(
    operation: &OperationStateRow,
) -> RuntimeMemoryStoreResult<runtime_memory_rollout::RuntimeMemoryContract> {
    use runtime_memory_rollout::RuntimeMemoryContract;
    match operation.runtime_memory_contract.as_str() {
        "legacy_v1" => Ok(RuntimeMemoryContract::LegacyV1),
        "dual_write_legacy_read" => Ok(RuntimeMemoryContract::DualWriteLegacyRead),
        "dual_write_v2_preferred" => Ok(RuntimeMemoryContract::DualWriteV2Preferred),
        "v2_only" => Ok(RuntimeMemoryContract::V2Only),
        _ => Err(RuntimeMemoryStoreError::Conflict {
            code: "unknown_runtime_memory_contract",
        }),
    }
}

fn ensure_runtime_operation_active(operation: &OperationStateRow) -> RuntimeMemoryStoreResult<()> {
    if operation.superseded_by.is_some() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "operation_superseded",
        });
    }
    Ok(())
}

fn contract_writes_v2(contract: runtime_memory_rollout::RuntimeMemoryContract) -> bool {
    !matches!(
        contract,
        runtime_memory_rollout::RuntimeMemoryContract::LegacyV1
    )
}

fn contract_writes_legacy_mirror(contract: runtime_memory_rollout::RuntimeMemoryContract) -> bool {
    matches!(
        contract,
        runtime_memory_rollout::RuntimeMemoryContract::DualWriteLegacyRead
            | runtime_memory_rollout::RuntimeMemoryContract::DualWriteV2Preferred
    )
}

fn contract_writes_legacy_checkpoint(
    contract: runtime_memory_rollout::RuntimeMemoryContract,
) -> bool {
    !matches!(
        contract,
        runtime_memory_rollout::RuntimeMemoryContract::V2Only
    )
}

const LEGACY_RUNTIME_CHECKPOINT_NAMESPACES: [&str; 11] = [
    "graph_flow",
    "profile",
    "current_stage",
    "current_stage_run_id",
    "queue_titles",
    "completed_count",
    "continuity_adoption",
    "schema_v",
    "stage_run_workers",
    "stage_run_handoffs",
    "agent_run",
];

/// A V2-only reset is reconstructed from relational state. The caller's legacy
/// checkpoint payload is never an input; only already-persisted sibling state
/// survives, with every legacy runtime namespace removed before the server
/// writes its own reset marker.
fn v2_only_reset_state_blob(current_state_blob: &serde_json::Value) -> serde_json::Value {
    let mut next_state_blob = current_state_blob.clone();
    let root = ensure_json_object(&mut next_state_blob);
    for namespace in LEGACY_RUNTIME_CHECKPOINT_NAMESPACES {
        root.remove(namespace);
    }
    next_state_blob
}

async fn frozen_attack_execution_contract(
    connection: &mut sqlx::PgConnection,
    operation_id: Uuid,
) -> RuntimeMemoryStoreResult<String> {
    let contract: Option<String> = sqlx::query_scalar(
        "SELECT attack_execution_contract
           FROM operation_state
          WHERE operation_id=$1 AND superseded_by IS NULL",
    )
    .bind(operation_id)
    .fetch_optional(connection)
    .await?;
    contract.ok_or(RuntimeMemoryStoreError::Missing {
        entity: "operation_state",
    })
}

async fn validate_runtime_stage_execution(
    connection: &mut sqlx::PgConnection,
    operation: &OperationStateRow,
    stage_execution_id: Uuid,
    stage_kind: &str,
) -> RuntimeMemoryStoreResult<()> {
    let active =
        stage_runs::list_active_for_operation_with_executor(connection, operation.operation_id)
            .await?;
    let execution = match active.as_slice() {
        [execution] => execution,
        [] => {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "missing_active_stage_execution",
            });
        }
        _ => {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "multiple_active_stage_executions",
            });
        }
    };
    if execution.id != stage_execution_id {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "active_stage_execution_mismatch",
        });
    }
    if execution.stage_kind != stage_kind || operation.current_stage != stage_kind {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "runtime_stage_kind_mismatch",
        });
    }
    Ok(())
}

fn ensure_json_object(
    value: &mut serde_json::Value,
) -> &mut serde_json::Map<String, serde_json::Value> {
    if !value.is_object() {
        *value = serde_json::json!({});
    }
    value
        .as_object_mut()
        .expect("value was normalized to a JSON object")
}

fn apply_legacy_worker_mirror(
    state_blob: &mut serde_json::Value,
    stage_kind: &str,
    organization_name: &str,
    worker: &StageWorkerRunRow,
) {
    let root = ensure_json_object(state_blob);
    let workers = root
        .entry("stage_run_workers".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let stage = ensure_json_object(workers)
        .entry(stage_kind.to_string())
        .or_insert_with(|| serde_json::json!({}));
    let record = serde_json::json!({
        "schema_v": 2,
        "id": worker.id,
        "operation_id": worker.operation_id,
        "stage_execution_id": worker.stage_execution_id,
        "stage_run_unit_id": worker.stage_run_unit_id,
        "worker_run_id": worker.id,
        "organization_id": worker.organization_id,
        "org_name": organization_name,
        "worker_generation": worker.worker_generation,
        "specialist": worker.specialist,
        "work_item_kind": worker.work_item_kind,
        "work_item_key": worker.work_item_key,
        "agent_path": worker.agent_path,
        "parent_request_id": worker.parent_request_id,
        "chain_id": worker.message_chain_id,
        "message_chain_id": worker.message_chain_id,
        "status": worker.status,
        "gate_attempt": worker.gate_attempt,
        "checkpoint": worker.checkpoint,
        "checkpoint_version": worker.checkpoint_version,
        "lease_token": worker.lease_token,
        "lease_owner": worker.lease_owner,
        "lease_acquired_at": worker.lease_acquired_at,
        "lease_expires_at": worker.lease_expires_at,
        "heartbeat_at": worker.heartbeat_at,
        "attempt_epoch": worker.attempt_epoch,
        "active_tool_call_id": worker.active_tool_call_id,
        "active_tool_started_at": worker.active_tool_started_at,
        "evidence_watermark": worker.evidence_watermark,
        "started_at": worker.started_at,
        "updated_at": worker.updated_at,
        "terminal_at": worker.terminal_at,
    });
    let org = ensure_json_object(stage)
        .entry(worker.organization_id.to_string())
        .or_insert_with(|| serde_json::json!({}));
    let org = ensure_json_object(org);
    for (key, value) in record
        .as_object()
        .expect("legacy worker record is an object")
    {
        org.insert(key.clone(), value.clone());
    }
    let records = org
        .entry("worker_records".to_string())
        .or_insert_with(|| serde_json::json!({}));
    ensure_json_object(records).insert(worker.id.to_string(), record);
}

fn apply_legacy_final_seal_mirror(
    state_blob: &mut serde_json::Value,
    unit: &StageRunUnitRow,
    worker: &StageWorkerRunRow,
    handoff: &StageHandoffRow,
    organization_name: &str,
) {
    apply_legacy_worker_mirror(state_blob, &unit.stage_kind, organization_name, worker);
    let root = ensure_json_object(state_blob);
    let handoffs = root
        .entry("stage_run_handoffs".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let stage = ensure_json_object(handoffs)
        .entry(unit.stage_kind.clone())
        .or_insert_with(|| serde_json::json!({}));
    ensure_json_object(stage).insert(
        unit.organization_id.to_string(),
        serde_json::json!({
            "schema_v": handoff.schema_version,
            "handoff_id": handoff.id,
            "operation_id": handoff.operation_id,
            "stage_execution_id": handoff.stage_execution_id,
            "stage_run_unit_id": handoff.source_stage_run_unit_id,
            "deliverable_submission_id": handoff.deliverable_submission_id,
            "organization_id": handoff.organization_id,
            "status": "passed",
            "scope_hash": handoff.scope_hash,
            "payload_sha256": handoff.payload_sha256,
            "unit_gate_decision_hash": handoff.unit_gate_decision_hash,
            "gate_passed_at": handoff.gate_passed_at,
        }),
    );
}

fn apply_legacy_stage_execution_mirror(
    state_blob: &mut serde_json::Value,
    profile: &str,
    stage_kind: &str,
    stage_execution_id: Uuid,
) {
    let root = ensure_json_object(state_blob);
    root.insert("profile".to_string(), serde_json::json!(profile));
    root.insert("current_stage".to_string(), serde_json::json!(stage_kind));
    root.insert(
        "current_stage_run_id".to_string(),
        serde_json::json!(stage_execution_id),
    );
    root.entry("queue_titles".to_string())
        .or_insert_with(|| serde_json::json!([]));
    root.entry("completed_count".to_string())
        .or_insert_with(|| serde_json::json!(0));
    root.entry("schema_v".to_string())
        .or_insert_with(|| serde_json::json!(1));
}

async fn write_locked_legacy_state_blob(
    connection: &mut sqlx::PgConnection,
    operation_id: Uuid,
    expected_contract: &str,
    state_blob: &serde_json::Value,
) -> RuntimeMemoryStoreResult<()> {
    let rows = sqlx::query(
        r#"UPDATE operation_state
              SET state_blob=$2
            WHERE operation_id=$1
              AND runtime_memory_contract=$3
              AND superseded_by IS NULL"#,
    )
    .bind(operation_id)
    .bind(state_blob)
    .bind(expected_contract)
    .execute(connection)
    .await?
    .rows_affected();
    if rows != 1 {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "legacy_runtime_mirror_operation_changed",
        });
    }
    Ok(())
}

fn validate_seed_input(input: &SeedStageRuntimeRow) -> RuntimeMemoryStoreResult<()> {
    if input.stage_kind.trim().is_empty()
        || input.specialist.trim().is_empty()
        || input.work_item_kind.trim().is_empty()
        || input.work_item_key.trim().is_empty()
        || input.agent_path_prefix.trim().is_empty()
        || input.unit_generation < 0
        || input.worker_generation < 0
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_runtime_seed",
        });
    }
    if let Some(organization_ids) = &input.organization_ids {
        if organization_ids.is_empty() {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "empty_stage_runtime_seed_organizations",
            });
        }
        let unique_organization_ids = organization_ids
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        if unique_organization_ids.len() != organization_ids.len() {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "duplicate_stage_runtime_seed_organization",
            });
        }
    }
    Ok(())
}

/// Seed exactly one Unit and logical primary Worker for either every frozen
/// organization or an explicit server-authorized subset. Replays return the
/// same rows after exact identity validation; scope is always resolved from
/// the immutable operation snapshot rather than caller/model material.
pub async fn seed_stage_runtime(
    pool: &sqlx::PgPool,
    input: &SeedStageRuntimeRow,
) -> RuntimeMemoryStoreResult<Vec<SeededStageRuntimeRow>> {
    validate_seed_input(input)?;
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    if operation.superseded_by.is_some() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "operation_superseded",
        });
    }
    let contract = frozen_runtime_contract(&operation)?;
    if !contract_writes_v2(contract) {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "runtime_v2_not_enabled",
        });
    }
    validate_runtime_stage_execution(
        &mut tx,
        &operation,
        input.stage_execution_id,
        &input.stage_kind,
    )
    .await?;
    let scope =
        operation_org_scope::load_for_operation_with_connection(&mut tx, input.operation_id)
            .await
            .map_err(map_scope_freeze_error)?
            .ok_or(RuntimeMemoryStoreError::Missing {
                entity: "operation_org_scope_snapshots",
            })?;
    if scope.snapshot.sealed_at.is_none() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "operation_scope_not_sealed",
        });
    }
    let selected_organization_ids = input.organization_ids.as_ref().map(|organization_ids| {
        organization_ids
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
    });
    if selected_organization_ids.as_ref().is_some_and(|selected| {
        selected.iter().any(|organization_id| {
            !scope
                .units
                .iter()
                .any(|scope_unit| scope_unit.organization_id == *organization_id)
        })
    }) {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_runtime_seed_organization_outside_frozen_scope",
        });
    }
    let selected_scope_units = scope
        .units
        .iter()
        .filter(|scope_unit| {
            selected_organization_ids
                .as_ref()
                .is_none_or(|selected| selected.contains(&scope_unit.organization_id))
        })
        .collect::<Vec<_>>();

    let existing_units = crate::repo::stage_run_units::list_for_execution_with_executor(
        &mut *tx,
        input.operation_id,
        input.stage_execution_id,
    )
    .await?;
    if existing_units.iter().any(|existing| {
        !scope
            .units
            .iter()
            .any(|scope_unit| scope_unit.organization_id == existing.organization_id)
    }) {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_runtime_unit_outside_frozen_scope",
        });
    }

    let mut legacy_blob = operation.state_blob.clone();
    let mut seeded = Vec::with_capacity(selected_scope_units.len());
    for scope_unit in selected_scope_units {
        let unit = match existing_units
            .iter()
            .find(|existing| existing.organization_id == scope_unit.organization_id)
        {
            Some(existing) => {
                if existing.operation_id != input.operation_id
                    || existing.stage_execution_id != input.stage_execution_id
                    || existing.scope_snapshot_id != scope.snapshot.id
                    || existing.stage_kind != input.stage_kind
                    || existing.generation != input.unit_generation
                    || existing.specialist.as_deref() != Some(input.specialist.as_str())
                {
                    return Err(RuntimeMemoryStoreError::IdentityMismatch {
                        code: "stage_runtime_seed_replay_mismatch",
                    });
                }
                existing.clone()
            }
            None => {
                crate::repo::stage_run_units::insert_with_executor(
                    &mut *tx,
                    &crate::repo::stage_run_units::NewStageRunUnit {
                        id: Uuid::new_v4(),
                        operation_id: input.operation_id,
                        stage_execution_id: input.stage_execution_id,
                        scope_snapshot_id: scope.snapshot.id,
                        organization_id: scope_unit.organization_id,
                        stage_kind: input.stage_kind.clone(),
                        generation: input.unit_generation,
                        specialist: Some(input.specialist.clone()),
                    },
                )
                .await?
            }
        };
        let agent_path = format!(
            "{}>org:{}>{}",
            input.agent_path_prefix, scope_unit.organization_id, input.specialist
        );
        let worker = match stage_worker_runs::get_logical_with_executor(
            &mut *tx,
            unit.id,
            &input.work_item_kind,
            &input.work_item_key,
            input.worker_generation,
        )
        .await?
        {
            Some(existing) => {
                if existing.operation_id != input.operation_id
                    || existing.stage_execution_id != input.stage_execution_id
                    || existing.organization_id != scope_unit.organization_id
                    || existing.specialist != input.specialist
                    || existing.agent_path != agent_path
                {
                    return Err(RuntimeMemoryStoreError::IdentityMismatch {
                        code: "stage_worker_seed_replay_mismatch",
                    });
                }
                existing
            }
            None => {
                stage_worker_runs::insert_with_executor(
                    &mut *tx,
                    &stage_worker_runs::NewStageWorkerRun {
                        id: Uuid::new_v4(),
                        operation_id: input.operation_id,
                        stage_execution_id: input.stage_execution_id,
                        stage_run_unit_id: unit.id,
                        work_item_id: None,
                        organization_id: scope_unit.organization_id,
                        worker_generation: input.worker_generation,
                        specialist: input.specialist.clone(),
                        work_item_kind: input.work_item_kind.clone(),
                        work_item_key: input.work_item_key.clone(),
                        agent_path,
                        parent_request_id: None,
                    },
                )
                .await?
            }
        };
        if contract_writes_legacy_mirror(contract) {
            apply_legacy_worker_mirror(
                &mut legacy_blob,
                &input.stage_kind,
                &scope_unit.organization_name_at_freeze,
                &worker,
            );
        }
        seeded.push(SeededStageRuntimeRow {
            unit,
            worker,
            organization_name: scope_unit.organization_name_at_freeze.clone(),
            scope_hash: scope.snapshot.scope_hash.clone(),
        });
    }
    if contract_writes_legacy_mirror(contract) {
        write_locked_legacy_state_blob(
            &mut tx,
            input.operation_id,
            &operation.runtime_memory_contract,
            &legacy_blob,
        )
        .await?;
        for row in &seeded {
            runtime_memory_shadow::persist_worker_sample(&mut tx, row.worker.id, "seed").await?;
        }
    }
    tx.commit().await?;
    if contract_writes_legacy_mirror(contract) {
        reconcile_deployment_rollouts_best_effort(pool, "seed_stage_runtime").await;
    }
    Ok(seeded)
}

fn validate_stage_team_seed(input: &SeedStageTeamRuntimeRow) -> RuntimeMemoryStoreResult<()> {
    validate_seed_input(&input.base)?;
    let plan = &input.plan;
    if plan.schema_version <= 0
        || plan.plan_version <= 0
        || plan.leader_role.trim().is_empty()
        || plan.allowed_roles.is_empty()
        || plan.max_workers_total <= 0
        || plan.max_workers_active <= 0
        || plan.max_workers_active > plan.max_workers_total
        || !matches!(plan.aggregator_kind.as_str(), "worker" | "deterministic")
        || !matches!(
            plan.final_submitter_kind.as_str(),
            "worker" | "deterministic"
        )
        || !plan.dynamic_request_policy.is_object()
        || input.work_items.is_empty()
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_team_seed",
        });
    }
    let roles = plan
        .allowed_roles
        .iter()
        .map(|role| role.trim())
        .collect::<std::collections::HashSet<_>>();
    if roles.len() != plan.allowed_roles.len()
        || roles.contains("")
        || !roles.contains(plan.leader_role.trim())
        || plan
            .aggregator_role
            .as_deref()
            .is_some_and(|role| !roles.contains(role.trim()))
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_team_roles",
        });
    }
    let stable_keys = input
        .work_items
        .iter()
        .map(|item| (item.work_item_kind.trim(), item.stable_key.trim()))
        .collect::<std::collections::HashSet<_>>();
    let aggregators = input
        .work_items
        .iter()
        .filter(|item| item.is_aggregator)
        .count();
    if stable_keys.len() != input.work_items.len()
        || aggregators != usize::from(plan.aggregator_kind == "worker")
        || input.work_items.iter().any(|item| {
            item.stable_key.trim().is_empty()
                || item.work_item_kind.trim().is_empty()
                || !roles.contains(item.role.trim())
                || !item.input_manifest.is_object()
                || !item.attempt_policy.is_object()
                || !item.budget.is_object()
                || item.output_schema.trim().is_empty()
                || item.created_by != "server_seed"
                || (item.is_aggregator
                    && (plan.aggregator_role.as_deref() != Some(item.role.as_str())
                        || item.required_for_barrier))
        })
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_team_work_items",
        });
    }
    Ok(())
}

fn stage_team_dynamic_attempt_policy(plan: &stage_teams::StageTeamPlanRow) -> Value {
    plan.dynamic_request_policy
        .get("attempt_policy")
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({"max_attempts": 3}))
}

fn static_stage_work_item_replays_exactly(
    existing: &stage_teams::StageWorkItemRow,
    seed: &StageWorkItemSeedRow,
    plan: &stage_teams::StageTeamPlanRow,
    unit: &StageRunUnitRow,
) -> bool {
    existing.team_plan_id == plan.id
        && existing.operation_id == plan.operation_id
        && existing.stage_execution_id == plan.stage_execution_id
        && existing.stage_run_unit_id == unit.id
        && existing.scope_snapshot_id == plan.scope_snapshot_id
        && existing.organization_id == plan.organization_id
        // A static WorkItem keeps the dispatch epoch at which it was created,
        // while a Gate repair advances the mutable TeamPlan epoch. Re-seeding
        // after that advance must compare only immutable seed identity.
        && existing.kind == seed.work_item_kind
        && existing.stable_key == seed.stable_key
        && existing.role == seed.role
        && existing.input_manifest_hash == seed.input_manifest_hash
        && existing.input_refs == serde_json::json!([seed.input_manifest.clone()])
        && existing.required_for_barrier == seed.required_for_barrier
        && existing.conflict_key == seed.conflict_key
        && existing.priority == seed.priority
        && existing.attempt_policy == seed.attempt_policy
        && existing.budget == seed.budget
        && existing.output_schema == seed.output_schema
        && existing.created_by == "server_seed"
}

async fn validate_stage_team_replay_extra(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan: &stage_teams::StageTeamPlanRow,
    item: &stage_teams::StageWorkItemRow,
) -> RuntimeMemoryStoreResult<()> {
    if item.team_plan_id != plan.id
        || item.operation_id != plan.operation_id
        || item.stage_execution_id != plan.stage_execution_id
        || item.stage_run_unit_id != plan.stage_run_unit_id
        || item.scope_snapshot_id != plan.scope_snapshot_id
        || item.organization_id != plan.organization_id
        || item.dispatch_epoch > plan.dispatch_epoch
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_replay_extra_owner_mismatch",
        });
    }

    match item.created_by.as_str() {
        "accepted_worker_request" => {
            let request = sqlx::query_as::<_, stage_teams::StageWorkerRequestRow>(
                "SELECT * FROM stage_worker_requests
                  WHERE accepted_work_item_id=$1 AND team_plan_id=$2
                    AND operation_id=$3 AND stage_execution_id=$4
                    AND stage_run_unit_id=$5 AND scope_snapshot_id=$6
                    AND organization_id=$7 AND dispatch_epoch=$8
                  FOR SHARE",
            )
            .bind(item.id)
            .bind(plan.id)
            .bind(plan.operation_id)
            .bind(plan.stage_execution_id)
            .bind(plan.stage_run_unit_id)
            .bind(plan.scope_snapshot_id)
            .bind(plan.organization_id)
            .bind(item.dispatch_epoch)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_team_dynamic_work_item_authority_missing",
            })?;
            let is_company_controller = plan
                .dynamic_request_policy
                .get("coordination_mode")
                .and_then(Value::as_str)
                == Some("company_controller");
            let (input_material, expected_input_refs) =
                stage_team_dynamic_work_item_authority_material(
                    is_company_controller,
                    request.parent_work_item_id,
                    request.parent_worker_run_id,
                    &request.reason_code,
                    &request.bounded_subject_refs,
                );
            let expected_input_hash = format!(
                "sha256:{}",
                operation_scope_decisions::sha256_json(&input_material)
            );
            if request.status != "accepted"
                || request.decision_reason_code.is_some()
                || request.accepted_work_item_id != Some(item.id)
                || request.requested_role != item.role
                || request.request_kind != item.kind
                || request.expected_output_schema != item.output_schema
                || item.stable_key != format!("dynamic:{}", request.id)
                || item.input_refs != expected_input_refs
                || item.input_manifest_hash != expected_input_hash
                || !item.required_for_barrier
                || item.conflict_key.is_some()
                || item.attempt_policy != stage_team_dynamic_attempt_policy(plan)
                || item.budget != request.budget_hint
            {
                return Err(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "stage_team_dynamic_work_item_authority_mismatch",
                });
            }
            Ok(())
        }
        "gate_repair" => {
            let generation_sql = format!(
                "SELECT {} FROM stage_team_repair_generations
                  WHERE team_plan_id=$1 AND operation_id=$2 AND stage_execution_id=$3
                    AND stage_run_unit_id=$4 AND scope_snapshot_id=$5
                    AND organization_id=$6 AND dispatch_epoch=$7 AND status='sealed'
                    AND (repair_work_item_id=$8 OR aggregator_work_item_id=$8)
                  FOR SHARE",
                stage_teams::REPAIR_GENERATION_COLUMNS
            );
            let generation =
                sqlx::query_as::<_, stage_teams::StageTeamRepairGenerationRow>(&generation_sql)
                    .bind(plan.id)
                    .bind(plan.operation_id)
                    .bind(plan.stage_execution_id)
                    .bind(plan.stage_run_unit_id)
                    .bind(plan.scope_snapshot_id)
                    .bind(plan.organization_id)
                    .bind(item.dispatch_epoch)
                    .bind(item.id)
                    .fetch_optional(&mut **tx)
                    .await?
                    .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                        code: "stage_team_gate_repair_authority_missing",
                    })?;
            let manifest_hash = format!(
                "sha256:{}",
                operation_scope_decisions::sha256_json(&generation.manifest)
            );
            let repair_generation = generation
                .manifest
                .get("repair_generation")
                .and_then(Value::as_i64);
            let manifest_epoch = generation
                .manifest
                .get("dispatch_epoch")
                .and_then(Value::as_i64);
            let input_hash = format!(
                "sha256:{}",
                operation_scope_decisions::sha256_json(&item.input_refs)
            );
            let common_exact = generation.manifest_hash == manifest_hash
                && manifest_epoch == Some(item.dispatch_epoch)
                && repair_generation.is_some()
                && item.attempt_policy == serde_json::json!({"max_attempts": 2})
                && item.budget == serde_json::json!({"repair_generation": repair_generation})
                && item.created_by == "gate_repair";
            let item_exact = if generation.repair_work_item_id == Some(item.id) {
                let expected_hash = generation
                    .manifest
                    .get("repair_input_hash")
                    .and_then(Value::as_str);
                let gap_hash = generation
                    .manifest
                    .get("gap_manifest_hash")
                    .and_then(Value::as_str);
                let repair_role = generation
                    .manifest
                    .get("repair_role")
                    .and_then(Value::as_str);
                expected_hash == Some(item.input_manifest_hash.as_str())
                    && input_hash == item.input_manifest_hash
                    && gap_hash.is_some_and(|hash| {
                        item.stable_key == format!("gate-repair:{}:{hash}", item.dispatch_epoch)
                    })
                    && repair_role == Some(item.role.as_str())
                    && item.kind == "gate_repair"
                    && item.required_for_barrier
                    && item.conflict_key.as_deref() == Some("stage_unit_gate_repair")
                    && item.priority == i32::MIN
                    && item.output_schema == "stage_worker_output.v1"
            } else if generation.aggregator_work_item_id == Some(item.id) {
                let expected_hash = generation
                    .manifest
                    .get("aggregator_input_hash")
                    .and_then(Value::as_str);
                expected_hash == Some(item.input_manifest_hash.as_str())
                    && input_hash == item.input_manifest_hash
                    && item.stable_key == format!("aggregator:repair:{}", item.dispatch_epoch)
                    && plan.aggregator_role.as_deref() == Some(item.role.as_str())
                    && item.kind == "stage_aggregate"
                    && !item.required_for_barrier
                    && item.conflict_key.as_deref() == Some("stage_unit_finalizer")
                    && item.priority == i32::MAX
                    && item.output_schema == "stage_unit_aggregate.v1"
            } else {
                false
            };
            if !common_exact || !item_exact {
                return Err(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "stage_team_gate_repair_authority_mismatch",
                });
            }
            Ok(())
        }
        _ => Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_work_item_replay_extra",
        }),
    }
}

/// Seed a frozen TeamPlan and stable WorkItems for every selected organization.
/// No Worker is created here: each WorkItem claim receives its own sibling
/// WorkerRun, lease and message chain in a later short transaction.
pub async fn seed_stage_team_runtime(
    pool: &sqlx::PgPool,
    input: &SeedStageTeamRuntimeRow,
) -> RuntimeMemoryStoreResult<Vec<SeededStageTeamRuntimeRow>> {
    validate_stage_team_seed(input)?;
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.base.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    if operation.superseded_by.is_some() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "operation_superseded",
        });
    }
    let contract = frozen_runtime_contract(&operation)?;
    if contract != runtime_memory_rollout::RuntimeMemoryContract::V2Only {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_requires_v2_only",
        });
    }
    validate_runtime_stage_execution(
        &mut tx,
        &operation,
        input.base.stage_execution_id,
        &input.base.stage_kind,
    )
    .await?;
    let scope =
        operation_org_scope::load_for_operation_with_connection(&mut tx, input.base.operation_id)
            .await
            .map_err(map_scope_freeze_error)?
            .ok_or(RuntimeMemoryStoreError::Missing {
                entity: "operation_org_scope_snapshots",
            })?;
    if scope.snapshot.sealed_at.is_none() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "operation_scope_not_sealed",
        });
    }
    let selected_ids = input.base.organization_ids.as_ref().map(|ids| {
        ids.iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
    });
    if selected_ids.as_ref().is_some_and(|selected| {
        selected.iter().any(|organization_id| {
            !scope
                .units
                .iter()
                .any(|unit| unit.organization_id == *organization_id)
        })
    }) {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_seed_organization_outside_frozen_scope",
        });
    }
    let existing_units = stage_run_units::list_for_execution_with_executor(
        &mut *tx,
        input.base.operation_id,
        input.base.stage_execution_id,
    )
    .await?;
    let mut seeded = Vec::new();
    for scope_unit in scope.units.iter().filter(|unit| {
        selected_ids
            .as_ref()
            .is_none_or(|selected| selected.contains(&unit.organization_id))
    }) {
        let unit = match existing_units
            .iter()
            .find(|existing| existing.organization_id == scope_unit.organization_id)
        {
            Some(existing) => {
                if existing.operation_id != input.base.operation_id
                    || existing.stage_execution_id != input.base.stage_execution_id
                    || existing.scope_snapshot_id != scope.snapshot.id
                    || existing.stage_kind != input.base.stage_kind
                    || existing.generation != input.base.unit_generation
                {
                    return Err(RuntimeMemoryStoreError::IdentityMismatch {
                        code: "stage_team_unit_replay_mismatch",
                    });
                }
                existing.clone()
            }
            None => {
                stage_run_units::insert_with_executor(
                    &mut *tx,
                    &stage_run_units::NewStageRunUnit {
                        id: Uuid::new_v4(),
                        operation_id: input.base.operation_id,
                        stage_execution_id: input.base.stage_execution_id,
                        scope_snapshot_id: scope.snapshot.id,
                        organization_id: scope_unit.organization_id,
                        stage_kind: input.base.stage_kind.clone(),
                        generation: input.base.unit_generation,
                        specialist: Some(input.base.specialist.clone()),
                    },
                )
                .await?
            }
        };
        let plan_id = Uuid::new_v5(
            &unit.id,
            format!("stage-team-plan:v{}", input.plan.plan_version).as_bytes(),
        );
        let existing_plan = stage_teams::get_plan_for_unit_with_executor(&mut *tx, unit.id).await?;
        let replayed = existing_plan.is_some();
        let plan = match existing_plan {
            Some(existing) => {
                if existing.id != plan_id
                    || existing.operation_id != input.base.operation_id
                    || existing.stage_execution_id != input.base.stage_execution_id
                    || existing.scope_snapshot_id != scope.snapshot.id
                    || existing.organization_id != scope_unit.organization_id
                    || existing.plan_hash != input.plan.plan_hash
                    || existing.created_from_stage_spec_hash
                        != input.plan.created_from_stage_spec_hash
                {
                    return Err(RuntimeMemoryStoreError::IdentityMismatch {
                        code: "stage_team_plan_replay_mismatch",
                    });
                }
                existing
            }
            None => {
                stage_teams::insert_plan_with_executor(
                    &mut *tx,
                    &stage_teams::NewStageTeamPlan {
                        id: plan_id,
                        operation_id: input.base.operation_id,
                        stage_execution_id: input.base.stage_execution_id,
                        stage_run_unit_id: unit.id,
                        scope_snapshot_id: scope.snapshot.id,
                        organization_id: scope_unit.organization_id,
                        stage_kind: input.base.stage_kind.clone(),
                        unit_generation: input.base.unit_generation,
                        schema_version: input.plan.schema_version,
                        plan_version: input.plan.plan_version,
                        plan_hash: input.plan.plan_hash.clone(),
                        leader_role: input.plan.leader_role.clone(),
                        aggregator_kind: input.plan.aggregator_kind.clone(),
                        aggregator_role: input.plan.aggregator_role.clone(),
                        allowed_worker_roles: serde_json::json!(input.plan.allowed_roles.clone()),
                        max_workers_total: input.plan.max_workers_total,
                        max_workers_active: input.plan.max_workers_active,
                        dynamic_requests_allowed: input.plan.dynamic_requests_enabled,
                        dynamic_request_policy: input.plan.dynamic_request_policy.clone(),
                        final_submitter_kind: input.plan.final_submitter_kind.clone(),
                        created_from_stage_spec_hash: input
                            .plan
                            .created_from_stage_spec_hash
                            .clone(),
                    },
                )
                .await?
            }
        };
        let existing_items = stage_teams::list_work_items_with_executor(&mut *tx, plan.id).await?;
        let existing_static_items = existing_items
            .iter()
            .filter(|item| item.created_by == "server_seed")
            .collect::<Vec<_>>();
        let mut work_items = Vec::with_capacity(input.work_items.len());
        for seed in &input.work_items {
            let item_id = Uuid::new_v5(
                &plan.id,
                format!("{}:{}", seed.work_item_kind, seed.stable_key).as_bytes(),
            );
            let item = match existing_static_items
                .iter()
                .copied()
                .find(|item| item.id == item_id)
            {
                Some(existing) => {
                    if !static_stage_work_item_replays_exactly(existing, seed, &plan, &unit) {
                        return Err(RuntimeMemoryStoreError::IdentityMismatch {
                            code: "stage_team_work_item_replay_mismatch",
                        });
                    }
                    existing.clone()
                }
                None if replayed => {
                    return Err(RuntimeMemoryStoreError::IdentityMismatch {
                        code: "stage_team_work_item_replay_missing",
                    });
                }
                None => {
                    stage_teams::insert_work_item_with_executor(
                        &mut *tx,
                        &stage_teams::NewStageWorkItem {
                            id: item_id,
                            team_plan_id: plan.id,
                            operation_id: input.base.operation_id,
                            stage_execution_id: input.base.stage_execution_id,
                            stage_run_unit_id: unit.id,
                            scope_snapshot_id: scope.snapshot.id,
                            organization_id: scope_unit.organization_id,
                            dispatch_epoch: plan.dispatch_epoch,
                            kind: seed.work_item_kind.clone(),
                            stable_key: seed.stable_key.clone(),
                            role: seed.role.clone(),
                            input_manifest_hash: seed.input_manifest_hash.clone(),
                            input_refs: serde_json::json!([seed.input_manifest.clone()]),
                            required_for_barrier: seed.required_for_barrier,
                            conflict_key: seed.conflict_key.clone(),
                            priority: seed.priority,
                            attempt_policy: seed.attempt_policy.clone(),
                            budget: seed.budget.clone(),
                            output_schema: seed.output_schema.clone(),
                            created_by: seed.created_by.clone(),
                        },
                    )
                    .await?
                }
            };
            work_items.push(item);
        }
        if replayed && existing_static_items.len() != input.work_items.len() {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_team_work_item_replay_extra",
            });
        }
        if replayed {
            for extra in existing_items
                .iter()
                .filter(|item| item.created_by != "server_seed")
            {
                validate_stage_team_replay_extra(&mut tx, &plan, extra).await?;
            }
        }
        seeded.push(SeededStageTeamRuntimeRow {
            unit,
            plan,
            work_items,
            organization_name: scope_unit.organization_name_at_freeze.clone(),
            scope_hash: scope.snapshot.scope_hash.clone(),
            replayed,
        });
    }
    tx.commit().await?;
    Ok(seeded)
}

fn stage_worker_request_output_schema(value: &Value) -> RuntimeMemoryStoreResult<String> {
    if value.is_null() {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_worker_request",
        });
    }
    Ok(value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| operation_scope_decisions::canonical_json(value)))
}

pub fn stage_worker_request_payload_hash(input: &RequestStageWorkerRow) -> String {
    let material = serde_json::json!({
        "budget_hint": input.budget_hint,
        "dedupe_key": input.dedupe_key,
        "dispatch_epoch": input.expected_dispatch_epoch,
        "operation_id": input.fence.operation_id,
        "output_schema": input.output_schema,
        "parent_work_item_id": input.parent_work_item_id,
        "reason": input.reason,
        "requested_kind": input.requested_kind,
        "requested_role": input.requested_role,
        "stage_execution_id": input.fence.stage_execution_id,
        "stage_run_unit_id": input.fence.stage_run_unit_id,
        "stage_team_plan_id": input.stage_team_plan_id,
        "subject_refs": input.subject_refs,
    });
    format!(
        "sha256:{}",
        operation_scope_decisions::sha256_json(&material)
    )
}

fn controller_request_envelope(reason: &str) -> (String, Option<String>) {
    let Ok(value) = serde_json::from_str::<Value>(reason) else {
        return (reason.to_string(), None);
    };
    if value.get("schema").and_then(Value::as_str) != Some("stage_team_controller_request.v1") {
        return (reason.to_string(), None);
    }
    let objective = value
        .get("objective")
        .and_then(Value::as_str)
        .filter(|objective| !objective.trim().is_empty())
        .unwrap_or(reason)
        .to_string();
    let parent_request_id = value
        .get("parent_tool_request_id")
        .and_then(Value::as_str)
        .filter(|request_id| !request_id.trim().is_empty())
        .map(str::to_string);
    (objective, parent_request_id)
}

fn stage_team_dynamic_work_item_authority_material(
    is_company_controller: bool,
    parent_work_item_id: Uuid,
    parent_worker_run_id: Uuid,
    reason: &str,
    subject_refs: &Value,
) -> (Value, Value) {
    let (objective, _) = controller_request_envelope(reason);
    let input_material = serde_json::json!({
        "parent_work_item_id": parent_work_item_id,
        "parent_worker_run_id": parent_worker_run_id,
        "reason": objective.clone(),
        "subject_refs": subject_refs,
    });
    let input_refs = if is_company_controller {
        serde_json::json!([{
            "assignment_schema": "stage_team_controller_assignment.v1",
            "objective": objective,
            "subject_refs": subject_refs,
        }])
    } else {
        subject_refs.clone()
    };
    (input_material, input_refs)
}

fn dynamic_request_rejection(
    plan: &stage_teams::StageTeamPlanRow,
    input: &RequestStageWorkerRow,
    accepted_requests: i64,
    work_item_count: i64,
    allow_implicit_organization_scope: bool,
) -> Option<&'static str> {
    if plan.requests_closed_at.is_some() {
        return Some("stage_team_request_epoch_closed");
    }
    if !plan.dynamic_requests_allowed {
        return Some("stage_team_dynamic_requests_disabled");
    }
    if plan
        .dynamic_request_policy
        .get("canonical_subject_refs_only")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Some("stage_team_dynamic_request_contract_unversioned");
    }
    if plan.aggregator_role.as_deref() == Some(input.requested_role.as_str()) {
        return Some("stage_team_aggregator_role_is_server_owned");
    }
    let allowed_roles = plan
        .allowed_worker_roles
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<std::collections::HashSet<_>>();
    if !allowed_roles.contains(input.requested_role.as_str()) {
        return Some("stage_team_worker_role_not_allowed");
    }
    if stage_teams::enforces_lifetime_worker_total(&plan.dynamic_request_policy) {
        let max_requests = plan
            .dynamic_request_policy
            .get("max_requests")
            .or_else(|| plan.dynamic_request_policy.get("max_dynamic_requests"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if max_requests <= 0 || accepted_requests >= max_requests {
            return Some("stage_team_dynamic_request_limit_reached");
        }
        if work_item_count >= i64::from(plan.max_workers_total) {
            return Some("stage_team_worker_total_limit_reached");
        }
    }
    let max_subject_refs = plan
        .dynamic_request_policy
        .get("max_subject_refs")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if max_subject_refs == 0
        || (!allow_implicit_organization_scope && input.subject_refs.is_empty())
        || input.subject_refs.len() as u64 > max_subject_refs
    {
        return Some("stage_team_request_subject_limit_reached");
    }
    let Some(allowed_kinds) = plan
        .dynamic_request_policy
        .get("allowed_request_kinds")
        .and_then(Value::as_array)
        .filter(|kinds| !kinds.is_empty())
    else {
        return Some("stage_team_dynamic_request_contract_unversioned");
    };
    if !allowed_kinds
        .iter()
        .filter_map(Value::as_str)
        .any(|kind| kind == input.requested_kind)
    {
        return Some("stage_team_request_kind_not_allowed");
    }
    None
}

async fn dynamic_request_subject_rejection(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan: &stage_teams::StageTeamPlanRow,
    unit: &stage_run_units::StageRunUnitRow,
    input: &RequestStageWorkerRow,
    allow_implicit_organization_scope: bool,
) -> RuntimeMemoryStoreResult<Option<&'static str>> {
    if allow_implicit_organization_scope && input.subject_refs.is_empty() {
        return Ok(None);
    }
    let keys = match input
        .subject_refs
        .iter()
        .cloned()
        .map(serde_json::from_value::<CanonicalFactKey>)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(keys) if !keys.is_empty() => keys,
        _ => return Ok(Some("stage_team_request_subject_not_canonical")),
    };
    let project_path_at_freeze: Option<String> = sqlx::query_scalar(
        "SELECT project_path_at_freeze FROM operation_org_scope_snapshots
          WHERE id=$1 AND operation_id=$2 AND sealed_at IS NOT NULL FOR SHARE",
    )
    .bind(plan.scope_snapshot_id)
    .bind(plan.operation_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(project_path_at_freeze) = project_path_at_freeze else {
        return Ok(Some("stage_team_request_scope_not_authorized"));
    };
    let Some(freshness_floor) = unit.started_at else {
        return Ok(Some("stage_team_request_scope_not_authorized"));
    };
    match canonical_fact_refs::resolve_for_handoff(
        tx,
        plan.operation_id,
        plan.organization_id,
        &project_path_at_freeze,
        freshness_floor,
        &keys,
    )
    .await
    {
        Ok(resolved) if resolved.len() == keys.len() => Ok(None),
        Ok(_) | Err(canonical_fact_refs::CanonicalFactRefError::Rejected { .. }) => {
            Ok(Some("stage_team_request_subject_not_authorized"))
        }
        Err(canonical_fact_refs::CanonicalFactRefError::Sqlx(error)) => {
            Err(RuntimeMemoryStoreError::Sqlx(error))
        }
    }
}

async fn stage_team_request_parent_epoch_is_authorized(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan: &stage_teams::StageTeamPlanRow,
    parent_item: &stage_teams::StageWorkItemRow,
    parent_worker_run_id: Uuid,
) -> RuntimeMemoryStoreResult<bool> {
    if parent_item.dispatch_epoch == plan.dispatch_epoch {
        return Ok(true);
    }
    if plan
        .dynamic_request_policy
        .get("coordination_mode")
        .and_then(Value::as_str)
        != Some("company_controller")
        || parent_item.stable_key != "leader:primary"
        || parent_item.role != plan.leader_role
        || plan.aggregator_role.as_deref() != Some(parent_item.role.as_str())
        || parent_item.required_for_barrier
    {
        return Ok(false);
    }

    sqlx::query_scalar::<_, bool>(
        r#"SELECT
               EXISTS(
                   SELECT 1
                     FROM stage_team_repair_generations generation
                    WHERE generation.team_plan_id=$1
                      AND generation.operation_id=$2
                      AND generation.stage_execution_id=$3
                      AND generation.stage_run_unit_id=$4
                      AND generation.scope_snapshot_id=$5
                      AND generation.organization_id=$6
                      AND generation.dispatch_epoch=$7
                      AND generation.status IN ('building','sealed')
                      AND generation.manifest->>'kind'='company_controller_gate_reopen'
                      AND generation.manifest->>'leader_work_item_id'=$8
                      AND generation.manifest->>'leader_worker_run_id'=$9
               )
               OR EXISTS(
                   SELECT 1
                     FROM stage_team_controller_turn_resumes resume
                    WHERE resume.team_plan_id=$1
                      AND resume.operation_id=$2
                      AND resume.stage_execution_id=$3
                      AND resume.stage_run_unit_id=$4
                      AND resume.scope_snapshot_id=$5
                      AND resume.organization_id=$6
                      AND resume.resume_dispatch_epoch=$7
                      AND resume.leader_work_item_id=$8::UUID
                      AND resume.leader_worker_run_id=$9::UUID
                      AND resume.status='applied'
               )"#,
    )
    .bind(plan.id)
    .bind(plan.operation_id)
    .bind(plan.stage_execution_id)
    .bind(plan.stage_run_unit_id)
    .bind(plan.scope_snapshot_id)
    .bind(plan.organization_id)
    .bind(plan.dispatch_epoch)
    .bind(parent_item.id.to_string())
    .bind(parent_worker_run_id.to_string())
    .fetch_one(&mut **tx)
    .await
    .map_err(RuntimeMemoryStoreError::from)
}

/// Persist one context-bound dynamic sibling request.  Both acceptance and
/// rejection are durable decisions; an accepted WorkItem is inserted in this
/// same transaction and receives a server-owned stable identity.
pub async fn request_stage_worker(
    pool: &sqlx::PgPool,
    input: &RequestStageWorkerRow,
) -> RuntimeMemoryStoreResult<RequestedStageWorkerRow> {
    if input.requested_role.trim().is_empty()
        || input.requested_kind.trim().is_empty()
        || input.reason.trim().is_empty()
        || input.dedupe_key.trim().is_empty()
        || !input.budget_hint.is_object()
        || input.subject_refs.iter().any(Value::is_null)
        || stage_worker_request_payload_hash(input) != input.request_sha256
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_worker_request",
        });
    }
    let requested_output_schema = stage_worker_request_output_schema(&input.output_schema)?;
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.fence.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    if frozen_runtime_contract(&operation)? != runtime_memory_rollout::RuntimeMemoryContract::V2Only
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_requires_v2_only",
        });
    }
    let unit = load_runtime_unit_for_update(
        &mut tx,
        input.fence.operation_id,
        input.fence.stage_execution_id,
        input.fence.stage_run_unit_id,
    )
    .await?;
    validate_runtime_stage_execution(
        &mut tx,
        &operation,
        input.fence.stage_execution_id,
        &unit.stage_kind,
    )
    .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "SELECT * FROM stage_team_plans
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
            AND stage_run_unit_id=$4 FOR UPDATE",
    )
    .bind(input.stage_team_plan_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_team_plans",
    })?;
    if plan.dispatch_epoch != input.expected_dispatch_epoch {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_dispatch_epoch_mismatch",
        });
    }
    if unit.status != "running" {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_parent_unit_not_running",
        });
    }
    let parent_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "SELECT * FROM stage_work_items
          WHERE id=$1 AND team_plan_id=$2 AND operation_id=$3
            AND stage_execution_id=$4 AND stage_run_unit_id=$5 FOR UPDATE",
    )
    .bind(input.parent_work_item_id)
    .bind(plan.id)
    .bind(plan.operation_id)
    .bind(plan.stage_execution_id)
    .bind(plan.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_work_items",
    })?;
    let is_company_controller = plan
        .dynamic_request_policy
        .get("coordination_mode")
        .and_then(Value::as_str)
        == Some("company_controller")
        && parent_item.stable_key == "leader:primary"
        && parent_item.role == plan.leader_role
        && plan.aggregator_role.as_deref() == Some(parent_item.role.as_str());
    if !stage_team_request_parent_epoch_is_authorized(
        &mut tx,
        &plan,
        &parent_item,
        input.fence.worker_run_id,
    )
    .await?
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_controller_parent_epoch_not_authorized",
        });
    }
    if parent_item.status != "running"
        || (plan.aggregator_role.as_deref() == Some(parent_item.role.as_str())
            && !is_company_controller)
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_parent_work_item_not_running",
        });
    }
    // Request identity is stable across a legal retry of the parent WorkItem,
    // but authority is not.  Validate the caller's *current* WorkerRun fence
    // before either creating or replaying the durable request.
    let parent_worker = sqlx::query_as::<_, StageWorkerRunRow>(
        r#"SELECT * FROM stage_worker_runs
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_run_unit_id=$4 AND work_item_id=$5 AND lease_token=$6
              AND attempt_epoch=$7 AND checkpoint_version=$8 AND status='running'
              AND lease_expires_at > NOW()
            FOR UPDATE"#,
    )
    .bind(input.fence.worker_run_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .bind(parent_item.id)
    .bind(input.fence.lease_token)
    .bind(input.fence.attempt_epoch)
    .bind(input.fence.expected_checkpoint_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::LeaseLost {
        worker_run_id: input.fence.worker_run_id,
        attempt_epoch: input.fence.attempt_epoch,
    })?;
    let (expected_output_schema, expected_budget, allow_implicit_organization_scope) =
        if is_company_controller {
            let output_schema = plan
                .dynamic_request_policy
                .get("child_output_schema")
                .and_then(Value::as_str)
                .filter(|schema| !schema.trim().is_empty())
                .ok_or(RuntimeMemoryStoreError::Conflict {
                    code: "stage_team_dynamic_request_contract_unversioned",
                })?
                .to_string();
            let budget = plan
                .dynamic_request_policy
                .get("child_budget")
                .filter(|budget| budget.is_object())
                .cloned()
                .ok_or(RuntimeMemoryStoreError::Conflict {
                    code: "stage_team_dynamic_request_contract_unversioned",
                })?;
            let implicit_scope = plan
                .dynamic_request_policy
                .get("organization_scope_implicit")
                .and_then(Value::as_bool)
                == Some(true);
            (output_schema, budget, implicit_scope)
        } else {
            (
                requested_output_schema.clone(),
                input.budget_hint.clone(),
                false,
            )
        };
    let existing = sqlx::query_as::<_, stage_teams::StageWorkerRequestRow>(
        "SELECT * FROM stage_worker_requests
          WHERE team_plan_id=$1 AND dispatch_epoch=$2 AND dedupe_key=$3 FOR SHARE",
    )
    .bind(plan.id)
    .bind(plan.dispatch_epoch)
    .bind(&input.dedupe_key)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(existing) = existing {
        if existing.operation_id != input.fence.operation_id
            || existing.stage_execution_id != input.fence.stage_execution_id
            || existing.stage_run_unit_id != input.fence.stage_run_unit_id
            || existing.parent_work_item_id != input.parent_work_item_id
            || existing.requested_role != input.requested_role
            || existing.request_kind != input.requested_kind
            || existing.bounded_subject_refs != Value::Array(input.subject_refs.clone())
            || existing.reason_code != input.reason
            || existing.expected_output_schema != expected_output_schema
            || existing.budget_hint != expected_budget
            || existing.request_payload_hash != input.request_sha256
        {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_worker_request_replay_mismatch",
            });
        }
        let work_item = if let Some(work_item_id) = existing.accepted_work_item_id {
            Some(
                sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
                    "SELECT * FROM stage_work_items WHERE id=$1 FOR SHARE",
                )
                .bind(work_item_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(RuntimeMemoryStoreError::Missing {
                    entity: "stage_work_items",
                })?,
            )
        } else {
            None
        };
        tx.commit().await?;
        return Ok(RequestedStageWorkerRow {
            plan,
            request: existing,
            work_item,
            replayed: true,
        });
    }
    let accepted_requests: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stage_worker_requests WHERE team_plan_id=$1 AND status='accepted'",
    )
    .bind(plan.id)
    .fetch_one(&mut *tx)
    .await?;
    let work_item_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stage_work_items WHERE team_plan_id=$1")
            .bind(plan.id)
            .fetch_one(&mut *tx)
            .await?;
    let subject_rejection = dynamic_request_subject_rejection(
        &mut tx,
        &plan,
        &unit,
        input,
        allow_implicit_organization_scope,
    )
    .await?;
    let request_shape_rejection = if !is_company_controller
        && (expected_output_schema != parent_item.output_schema
            || input.budget_hint != parent_item.budget)
    {
        Some("stage_team_request_cannot_expand_parent_contract")
    } else {
        None
    };
    let rejection = dynamic_request_rejection(
        &plan,
        input,
        accepted_requests,
        work_item_count,
        allow_implicit_organization_scope,
    )
    .or(request_shape_rejection)
    .or(subject_rejection);
    let request_id = Uuid::new_v5(
        &plan.id,
        format!(
            "stage-worker-request:{}:{}",
            plan.dispatch_epoch, input.dedupe_key
        )
        .as_bytes(),
    );
    let mut work_item = None;
    let accepted_work_item_id = if rejection.is_none() {
        let work_item_id = Uuid::new_v5(&request_id, b"accepted-stage-work-item-v1");
        let request_subject_refs = Value::Array(input.subject_refs.clone());
        let (input_material, child_input_refs) = stage_team_dynamic_work_item_authority_material(
            is_company_controller,
            parent_item.id,
            parent_worker.id,
            &input.reason,
            &request_subject_refs,
        );
        let priority: i32 = sqlx::query_scalar(
            r#"SELECT COALESCE(MAX(priority),-1)+1 FROM stage_work_items
                WHERE team_plan_id=$1 AND role IS DISTINCT FROM $2"#,
        )
        .bind(plan.id)
        .bind(&plan.aggregator_role)
        .fetch_one(&mut *tx)
        .await?;
        let attempt_policy = stage_team_dynamic_attempt_policy(&plan);
        work_item = Some(
            stage_teams::insert_work_item_with_executor(
                &mut *tx,
                &stage_teams::NewStageWorkItem {
                    id: work_item_id,
                    team_plan_id: plan.id,
                    operation_id: plan.operation_id,
                    stage_execution_id: plan.stage_execution_id,
                    stage_run_unit_id: plan.stage_run_unit_id,
                    scope_snapshot_id: plan.scope_snapshot_id,
                    organization_id: plan.organization_id,
                    dispatch_epoch: plan.dispatch_epoch,
                    kind: input.requested_kind.clone(),
                    stable_key: format!("dynamic:{request_id}"),
                    role: input.requested_role.clone(),
                    input_manifest_hash: format!(
                        "sha256:{}",
                        operation_scope_decisions::sha256_json(&input_material)
                    ),
                    input_refs: child_input_refs,
                    required_for_barrier: true,
                    conflict_key: None,
                    priority,
                    attempt_policy,
                    budget: expected_budget.clone(),
                    output_schema: expected_output_schema.clone(),
                    created_by: "accepted_worker_request".to_string(),
                },
            )
            .await?,
        );
        if is_company_controller {
            sqlx::query(
                r#"INSERT INTO stage_work_item_dependencies (
                       team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                       scope_snapshot_id,organization_id,work_item_id,depends_on_work_item_id
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)
                   ON CONFLICT (work_item_id,depends_on_work_item_id) DO NOTHING"#,
            )
            .bind(plan.id)
            .bind(plan.operation_id)
            .bind(plan.stage_execution_id)
            .bind(plan.stage_run_unit_id)
            .bind(plan.scope_snapshot_id)
            .bind(plan.organization_id)
            .bind(parent_item.id)
            .bind(work_item_id)
            .execute(&mut *tx)
            .await?;
        }
        Some(work_item_id)
    } else {
        None
    };
    let request = sqlx::query_as::<_, stage_teams::StageWorkerRequestRow>(
        r#"INSERT INTO stage_worker_requests(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,parent_work_item_id,parent_worker_run_id,
               dispatch_epoch,requested_role,request_kind,bounded_subject_refs,reason_code,
               expected_output_schema,budget_hint,dedupe_key,request_payload_hash,status,
               decision_reason_code,accepted_work_item_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)
           RETURNING *"#,
    )
    .bind(request_id)
    .bind(plan.id)
    .bind(plan.operation_id)
    .bind(plan.stage_execution_id)
    .bind(plan.stage_run_unit_id)
    .bind(plan.scope_snapshot_id)
    .bind(plan.organization_id)
    .bind(parent_item.id)
    .bind(parent_worker.id)
    .bind(plan.dispatch_epoch)
    .bind(&input.requested_role)
    .bind(&input.requested_kind)
    .bind(Value::Array(input.subject_refs.clone()))
    .bind(&input.reason)
    .bind(&expected_output_schema)
    .bind(&expected_budget)
    .bind(&input.dedupe_key)
    .bind(&input.request_sha256)
    .bind(if rejection.is_some() {
        "rejected"
    } else {
        "accepted"
    })
    .bind(rejection)
    .bind(accepted_work_item_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(RequestedStageWorkerRow {
        plan,
        request,
        work_item,
        replayed: false,
    })
}

pub async fn close_stage_request_epoch(
    pool: &sqlx::PgPool,
    input: &CloseStageRequestEpochRow,
) -> RuntimeMemoryStoreResult<ClosedStageRequestEpochRow> {
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    if frozen_runtime_contract(&operation)? != runtime_memory_rollout::RuntimeMemoryContract::V2Only
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_requires_v2_only",
        });
    }
    let unit = load_runtime_unit_for_update(
        &mut tx,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
    )
    .await?;
    validate_runtime_stage_execution(
        &mut tx,
        &operation,
        input.stage_execution_id,
        &unit.stage_kind,
    )
    .await?;
    let current = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "SELECT * FROM stage_team_plans
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
            AND stage_run_unit_id=$4 FOR UPDATE",
    )
    .bind(input.stage_team_plan_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(input.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_team_plans",
    })?;
    if current.dispatch_epoch != input.expected_dispatch_epoch {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_dispatch_epoch_mismatch",
        });
    }
    let replayed = current.requests_closed_at.is_some();
    let plan = if replayed {
        if current.row_version < input.expected_plan_row_version {
            return Err(RuntimeMemoryStoreError::StaleVersion {
                entity: "stage_team_plans",
                expected: input.expected_plan_row_version,
                actual: current.row_version,
            });
        }
        current
    } else {
        if current.row_version != input.expected_plan_row_version {
            return Err(RuntimeMemoryStoreError::StaleVersion {
                entity: "stage_team_plans",
                expected: input.expected_plan_row_version,
                actual: current.row_version,
            });
        }
        sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
            "UPDATE stage_team_plans
                SET requests_closed_at=NOW(),row_version=row_version+1,updated_at=NOW()
              WHERE id=$1 AND dispatch_epoch=$2 AND row_version=$3
                AND requests_closed_at IS NULL AND final_submitter_worker_run_id IS NULL
              RETURNING *",
        )
        .bind(current.id)
        .bind(input.expected_dispatch_epoch)
        .bind(input.expected_plan_row_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_team_plans",
            expected: input.expected_plan_row_version,
            actual: current.row_version,
        })?
    };
    let barrier = stage_teams::load_barrier_with_connection(&mut tx, plan.id).await?;
    tx.commit().await?;
    Ok(ClosedStageRequestEpochRow {
        plan,
        barrier,
        replayed,
    })
}

pub async fn load_stage_team_barrier(
    pool: &sqlx::PgPool,
    input: &LoadStageTeamBarrierRow,
) -> RuntimeMemoryStoreResult<stage_teams::StageTeamBarrierRow> {
    let mut tx = pool.begin().await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "SELECT * FROM stage_team_plans
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
            AND stage_run_unit_id=$4 FOR UPDATE",
    )
    .bind(input.stage_team_plan_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(input.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_team_plans",
    })?;
    if plan.dispatch_epoch != input.dispatch_epoch {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_dispatch_epoch_mismatch",
        });
    }
    let barrier = stage_teams::load_barrier_with_connection(&mut tx, plan.id).await?;
    tx.commit().await?;
    Ok(barrier)
}

async fn load_runtime_unit_for_update(
    connection: &mut sqlx::PgConnection,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
) -> RuntimeMemoryStoreResult<StageRunUnitRow> {
    sqlx::query_as::<_, StageRunUnitRow>(
        r#"SELECT id, operation_id, stage_execution_id, scope_snapshot_id,
                  organization_id, stage_kind, generation, specialist, status,
                  gate_attempt, pass_watermark, row_version, started_at,
                  updated_at, terminal_at
             FROM stage_run_units
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
            FOR UPDATE"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .fetch_optional(connection)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_run_units",
    })
}

async fn frozen_organization_name(
    connection: &mut sqlx::PgConnection,
    unit: &StageRunUnitRow,
) -> RuntimeMemoryStoreResult<String> {
    sqlx::query_scalar::<_, String>(
        r#"SELECT organization_name_at_freeze
             FROM operation_org_scope_units
            WHERE snapshot_id=$1 AND organization_id=$2"#,
    )
    .bind(unit.scope_snapshot_id)
    .bind(unit.organization_id)
    .fetch_optional(connection)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "operation_org_scope_units",
    })
}

enum StageTeamExpiredRecovery {
    None,
    Requeued { previous_worker_run_id: Uuid },
    RecoveryRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StageTeamToolRecoveryPolicy {
    RetrySafeTerminalLocalTool,
    ResumeAfterInterruptedBoundedReadOnlyTool,
}

fn stage_team_tool_recovery_policy(
    tool_name: &str,
    tool_status: &str,
    result: Option<&str>,
) -> Option<StageTeamToolRecoveryPolicy> {
    if tool_name == "recon_list_providers"
        && tool_status == "failed"
        && result
            .is_some_and(|value| value.starts_with("worker tool result rejected by lease fence:"))
    {
        return Some(StageTeamToolRecoveryPolicy::RetrySafeTerminalLocalTool);
    }
    // This is intentionally a closed allowlist. These stage wrappers are
    // backend-owned, exact-scope, bounded foreground reads. Their authoritative
    // progress is persisted per coverage cell, so an interrupted worker can
    // reconcile from the durable worklist without replaying old arguments or
    // repeating terminal cells. The same worker chain gets a new Turn and must
    // refresh the worklist before deciding which exact gaps remain.
    if matches!(tool_status, "received" | "running")
        && matches!(
            tool_name,
            "enum_crawl_same_origin_urls"
                | "eas_probe_http_liveness"
                | "eas_discover_ports"
                | "eas_fingerprint_services"
                | "eas_fingerprint_web_stack"
        )
    {
        return Some(StageTeamToolRecoveryPolicy::ResumeAfterInterruptedBoundedReadOnlyTool);
    }
    None
}

/// A local provider-registry read may finish successfully while its worker
/// fence transaction is rolled back by PostgreSQL. The generic tool row is
/// then terminal failed, so there is no in-flight side effect left to resolve,
/// but the Worker still points at it. The same transaction also admits the
/// closed bounded stage-wrapper policy above; every other network-capable tool
/// remains operator-owned recovery.
async fn recover_retry_safe_stage_team_tool(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan: &stage_teams::StageTeamPlanRow,
    aggregator: bool,
) -> RuntimeMemoryStoreResult<Option<Uuid>> {
    let candidate = sqlx::query_as::<_, (Uuid, Uuid, String, String, Option<String>)>(
        r#"SELECT item.id,worker.id,item.status,tool.status::text,tool.result
             FROM stage_work_items AS item
             JOIN stage_worker_runs AS worker ON worker.work_item_id=item.id
             JOIN tool_calls AS tool
               ON tool.id=worker.active_tool_call_id
              AND tool.worker_run_id=worker.id
              AND tool.operation_id=worker.operation_id
              AND tool.stage_execution_id=worker.stage_execution_id
              AND tool.stage_run_unit_id=worker.stage_run_unit_id
              AND tool.organization_id=worker.organization_id
              AND tool.attempt_epoch=worker.attempt_epoch
              AND tool.lease_token=worker.lease_token
            WHERE item.team_plan_id=$1
              AND item.status IN ('running','recovery_required')
              AND worker.status IN ('running','recovery_required')
              AND (worker.status='recovery_required' OR worker.lease_expires_at<=NOW())
              AND (
                    ($2 AND item.role=$3)
                    OR (NOT $2 AND item.role IS DISTINCT FROM $3)
                  )
            ORDER BY item.priority,item.created_at,item.id
            LIMIT 1 FOR UPDATE OF item,worker,tool SKIP LOCKED"#,
    )
    .bind(plan.id)
    .bind(aggregator)
    .bind(&plan.aggregator_role)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((item_id, worker_id, item_status, tool_status, tool_result)) = candidate else {
        return Ok(None);
    };
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "SELECT * FROM stage_work_items WHERE id=$1",
    )
    .bind(item_id)
    .fetch_one(&mut **tx)
    .await?;
    let worker =
        sqlx::query_as::<_, StageWorkerRunRow>("SELECT * FROM stage_worker_runs WHERE id=$1")
            .bind(worker_id)
            .fetch_one(&mut **tx)
            .await?;
    let tool_name = sqlx::query_scalar::<_, String>("SELECT name FROM tool_calls WHERE id=$1")
        .bind(
            worker
                .active_tool_call_id
                .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "stage_team_terminal_tool_recovery_identity_mismatch",
                })?,
        )
        .fetch_one(&mut **tx)
        .await?;
    let Some(recovery_policy) =
        stage_team_tool_recovery_policy(&tool_name, &tool_status, tool_result.as_deref())
    else {
        return Ok(None);
    };
    if item.status != item_status
        || worker.work_item_id != Some(item.id)
        || worker.organization_id != plan.organization_id
        || worker.stage_run_unit_id != plan.stage_run_unit_id
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_terminal_tool_recovery_identity_mismatch",
        });
    }
    let attempts_used = stage_teams::work_item_attempts_used(tx, item.id).await?;
    let max_attempts = stage_teams::work_item_max_attempts(&item)?;
    if attempts_used >= max_attempts {
        return Ok(None);
    }
    let active_tool_call_id =
        worker
            .active_tool_call_id
            .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_team_terminal_tool_recovery_identity_mismatch",
            })?;
    let recovery_checkpoint = match recovery_policy {
        StageTeamToolRecoveryPolicy::RetrySafeTerminalLocalTool => serde_json::json!({
            "previous_checkpoint": worker.checkpoint,
            "stage_team_terminal_tool_fence_recovery": {
                "kind": "retry_safe_local_tool",
                "schema_version": 1,
                "tool_call_record_id": active_tool_call_id,
                "tool_name": tool_name,
            }
        }),
        StageTeamToolRecoveryPolicy::ResumeAfterInterruptedBoundedReadOnlyTool => {
            let tool_result = serde_json::to_string(&serde_json::json!({
                "kind": "stage_team_interrupted_tool_reconciled",
                "outcome": "unknown_requeued_same_worker_chain",
                "recovery_policy": "resume_after_reconcile",
                "schema_version": 1,
                "tool_call_record_id": active_tool_call_id,
                "tool_name": tool_name,
            }))
            .map_err(|_| RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_team_interrupted_tool_recovery_result_invalid",
            })?;
            let tool_rows = sqlx::query(
                r#"UPDATE tool_calls
                      SET status='failed',result=$2,updated_at=NOW()
                    WHERE id=$1 AND worker_run_id=$3 AND operation_id=$4
                      AND stage_execution_id=$5 AND stage_run_unit_id=$6
                      AND organization_id=$7 AND attempt_epoch=$8
                      AND lease_token=$9 AND status IN ('received','running')"#,
            )
            .bind(active_tool_call_id)
            .bind(tool_result)
            .bind(worker.id)
            .bind(plan.operation_id)
            .bind(plan.stage_execution_id)
            .bind(plan.stage_run_unit_id)
            .bind(plan.organization_id)
            .bind(worker.attempt_epoch)
            .bind(worker.lease_token)
            .execute(&mut **tx)
            .await?
            .rows_affected();
            if tool_rows != 1 {
                return Err(RuntimeMemoryStoreError::Conflict {
                    code: "stage_team_interrupted_tool_recovery_tool_cas_failed",
                });
            }
            serde_json::json!({
                "previous_checkpoint": worker.checkpoint,
                "stage_team_interrupted_tool_recovery": {
                    "kind": "resume_after_reconcile",
                    "schema_version": 1,
                    "tool_call_record_id": active_tool_call_id,
                    "tool_name": tool_name,
                }
            })
        }
    };
    let worker_rows = sqlx::query(
        r#"UPDATE stage_worker_runs
              SET status='queued',checkpoint=$5,
                  checkpoint_version=checkpoint_version+1,
                  active_tool_call_id=NULL,active_tool_started_at=NULL,
                  lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
                  lease_expires_at=NULL,heartbeat_at=NULL,
                  terminal_at=NULL,updated_at=NOW()
            WHERE id=$1 AND status=$2 AND attempt_epoch=$3
              AND checkpoint_version=$4 AND active_tool_call_id=$6"#,
    )
    .bind(worker.id)
    .bind(&worker.status)
    .bind(worker.attempt_epoch)
    .bind(worker.checkpoint_version)
    .bind(&recovery_checkpoint)
    .bind(active_tool_call_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if worker_rows != 1 {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_terminal_tool_recovery_worker_cas_failed",
        });
    }
    let queued_rows = if item.status == "recovery_required" {
        sqlx::query(
            "UPDATE stage_work_items
                SET status='queued',row_version=row_version+1,terminal_at=NULL,updated_at=NOW()
              WHERE id=$1 AND status='recovery_required' AND row_version=$2",
        )
        .bind(item.id)
        .bind(item.row_version)
        .execute(&mut **tx)
        .await?
        .rows_affected()
    } else {
        let retry_version: i64 = sqlx::query_scalar(
            "UPDATE stage_work_items
                SET status='retry_pending',row_version=row_version+1,updated_at=NOW()
              WHERE id=$1 AND status='running' AND row_version=$2
              RETURNING row_version",
        )
        .bind(item.id)
        .bind(item.row_version)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_work_items",
            expected: item.row_version,
            actual: -1,
        })?;
        sqlx::query(
            "UPDATE stage_work_items
                SET status='queued',row_version=row_version+1,terminal_at=NULL,updated_at=NOW()
              WHERE id=$1 AND status='retry_pending' AND row_version=$2",
        )
        .bind(item.id)
        .bind(retry_version)
        .execute(&mut **tx)
        .await?
        .rows_affected()
    };
    if queued_rows != 1 {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_terminal_tool_recovery_item_cas_failed",
        });
    }
    Ok(Some(worker.id))
}

async fn recover_expired_stage_team_item(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan: &stage_teams::StageTeamPlanRow,
    aggregator: bool,
) -> RuntimeMemoryStoreResult<StageTeamExpiredRecovery> {
    if let Some(previous_worker_run_id) =
        recover_retry_safe_stage_team_tool(tx, plan, aggregator).await?
    {
        return Ok(StageTeamExpiredRecovery::Requeued {
            previous_worker_run_id,
        });
    }
    let expired = sqlx::query_as::<_, (Uuid, i64, Uuid, i64, Option<Uuid>)>(
        r#"SELECT item.id,item.row_version,worker.id,worker.attempt_epoch,
                  worker.active_tool_call_id
             FROM stage_work_items AS item
             JOIN stage_worker_runs AS worker ON worker.work_item_id=item.id
            WHERE item.team_plan_id=$1 AND item.status='running'
              AND worker.status='running' AND worker.lease_expires_at <= NOW()
              AND (
                    ($2 AND item.role=$3)
                    OR (NOT $2 AND item.role IS DISTINCT FROM $3)
                  )
            ORDER BY item.priority,item.created_at,item.id
            LIMIT 1 FOR UPDATE OF item,worker SKIP LOCKED"#,
    )
    .bind(plan.id)
    .bind(aggregator)
    .bind(&plan.aggregator_role)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((item_id, item_row_version, worker_id, worker_attempt_epoch, active_tool_call_id)) =
        expired
    else {
        return Ok(StageTeamExpiredRecovery::None);
    };
    if active_tool_call_id.is_some() {
        sqlx::query(
            "UPDATE stage_worker_runs
                SET status='recovery_required',updated_at=NOW()
              WHERE id=$1 AND status='running' AND attempt_epoch=$2",
        )
        .bind(worker_id)
        .bind(worker_attempt_epoch)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE stage_work_items
                SET status='recovery_required',row_version=row_version+1,updated_at=NOW()
              WHERE id=$1 AND status='running' AND row_version=$2",
        )
        .bind(item_id)
        .bind(item_row_version)
        .execute(&mut **tx)
        .await?;
        return Ok(StageTeamExpiredRecovery::RecoveryRequired);
    }
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "SELECT * FROM stage_work_items WHERE id=$1",
    )
    .bind(item_id)
    .fetch_one(&mut **tx)
    .await?;
    let worker =
        sqlx::query_as::<_, StageWorkerRunRow>("SELECT * FROM stage_worker_runs WHERE id=$1")
            .bind(worker_id)
            .fetch_one(&mut **tx)
            .await?;
    if item.row_version != item_row_version || worker.attempt_epoch != worker_attempt_epoch {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_expired_worker_identity_mismatch",
        });
    }
    let reaped = stage_teams::reap_expired_clean_stage_worker(tx, plan, &item, &worker).await?;
    Ok(if reaped.retry_scheduled {
        StageTeamExpiredRecovery::Requeued {
            previous_worker_run_id: reaped.worker.id,
        }
    } else {
        StageTeamExpiredRecovery::None
    })
}

/// Claim the exact worker, start/restart its Unit, create and bind its initial
/// provider-safe chain when absent, and persist the initial checkpoint in one
/// transaction. No provider call is permitted before this function commits.
async fn claim_stage_team_item(
    pool: &sqlx::PgPool,
    input: &ClaimStageWorkItemRow,
    aggregator: Option<(i64, &str)>,
    leader: bool,
) -> RuntimeMemoryStoreResult<Option<ClaimedStageWorkItemRow>> {
    if input.lease_owner.trim().is_empty()
        || input.lease_seconds <= 0
        || input.parent_chain_id.is_some()
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_stage_team_work_item_claim",
        });
    }
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    if frozen_runtime_contract(&operation)? != runtime_memory_rollout::RuntimeMemoryContract::V2Only
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_requires_v2_only",
        });
    }
    let locked_unit = load_runtime_unit_for_update(
        &mut tx,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
    )
    .await?;
    if !matches!(locked_unit.status.as_str(), "queued" | "running") {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_unit_not_runnable",
        });
    }
    validate_runtime_stage_execution(
        &mut tx,
        &operation,
        input.stage_execution_id,
        &locked_unit.stage_kind,
    )
    .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "SELECT * FROM stage_team_plans
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
            AND stage_run_unit_id=$4 FOR UPDATE",
    )
    .bind(input.stage_team_plan_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(input.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_team_plans",
    })?;
    let is_aggregator_claim = aggregator.is_some();
    let is_leader_claim = leader;
    if is_aggregator_claim && is_leader_claim {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_stage_team_work_item_claim",
        });
    }
    if is_aggregator_claim
        && (plan.aggregator_kind != "worker" || plan.final_submitter_kind != "worker")
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_aggregator_is_not_worker_owned",
        });
    }
    if is_leader_claim
        && (plan.requests_closed_at.is_some()
            || plan.aggregator_kind != "worker"
            || plan.aggregator_role.as_deref() != Some(plan.leader_role.as_str())
            || plan
                .dynamic_request_policy
                .get("coordination_mode")
                .and_then(Value::as_str)
                != Some("company_controller"))
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_leader_not_claimable",
        });
    }
    let mut startup_reaped_final_submitter = None;
    if let Some((expected_epoch, expected_manifest_hash)) = aggregator {
        if plan.dispatch_epoch != expected_epoch || plan.requests_closed_at.is_none() {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_aggregator_epoch_not_closed",
            });
        }
        if let Some(final_submitter_worker_run_id) = plan.final_submitter_worker_run_id {
            let existing_worker = sqlx::query_as::<_, StageWorkerRunRow>(
                "SELECT * FROM stage_worker_runs WHERE id=$1 FOR UPDATE",
            )
            .bind(final_submitter_worker_run_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(RuntimeMemoryStoreError::Missing {
                entity: "stage_worker_runs",
            })?;
            let existing_work_item_id =
                existing_worker
                    .work_item_id
                    .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                        code: "stage_team_final_submitter_identity_mismatch",
                    })?;
            let existing_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
                "SELECT * FROM stage_work_items WHERE id=$1 FOR UPDATE",
            )
            .bind(existing_work_item_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(RuntimeMemoryStoreError::Missing {
                entity: "stage_work_items",
            })?;
            if existing_item.team_plan_id != plan.id
                || plan.aggregator_role.as_deref() != Some(existing_item.role.as_str())
            {
                return Err(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "stage_team_final_submitter_identity_mismatch",
                });
            }
            if existing_worker.status == "running"
                && existing_worker
                    .lease_expires_at
                    .is_some_and(|expires_at| expires_at > chrono::Utc::now())
            {
                if existing_worker.lease_owner.as_deref() != Some(input.lease_owner.as_str())
                    || existing_item.status != "running"
                {
                    return Err(RuntimeMemoryStoreError::Conflict {
                        code: "stage_team_aggregator_claim_replay_mismatch",
                    });
                }
                let chain_id = existing_worker.message_chain_id.ok_or(
                    RuntimeMemoryStoreError::IdentityMismatch {
                        code: "stage_team_aggregator_chain_missing",
                    },
                )?;
                let barrier = stage_teams::load_barrier_with_connection_ignoring_worker(
                    &mut tx,
                    plan.id,
                    Some(existing_worker.id),
                )
                .await?;
                if !barrier.ready_to_finalize() || barrier.manifest_hash != expected_manifest_hash {
                    return Err(RuntimeMemoryStoreError::Conflict {
                        code: "stage_team_sibling_barrier_not_ready",
                    });
                }
                tx.commit().await?;
                return Ok(Some(ClaimedStageWorkItemRow {
                    unit: locked_unit.clone(),
                    plan,
                    work_item: existing_item,
                    worker: existing_worker,
                    message_chain_id: chain_id,
                }));
            }
            if existing_worker.status == "recovery_required" {
                return Err(RuntimeMemoryStoreError::Conflict {
                    code: "stage_team_worker_recovery_required",
                });
            }
            if matches!(existing_worker.status.as_str(), "queued" | "superseded")
                && existing_item.status == "queued"
            {
                // Current recovery requeues the exact Aggregator WorkerRun;
                // `superseded` remains readable only for pre-migration rows.
                // Preserve the pointer as the exact final-submitter witness.
                startup_reaped_final_submitter = Some(existing_worker.id);
            } else if existing_worker.status != "running" {
                return Err(RuntimeMemoryStoreError::Conflict {
                    code: "stage_team_final_submitter_not_replaceable",
                });
            }
        }
    }
    let recovered = recover_expired_stage_team_item(&mut tx, &plan, is_aggregator_claim).await?;
    if matches!(recovered, StageTeamExpiredRecovery::RecoveryRequired) {
        tx.commit().await?;
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_worker_recovery_required",
        });
    }
    let replaced_final_submitter = match recovered {
        StageTeamExpiredRecovery::Requeued {
            previous_worker_run_id,
        } if is_aggregator_claim => Some(previous_worker_run_id),
        _ => startup_reaped_final_submitter,
    };
    if is_leader_claim {
        let leader_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
            "SELECT * FROM stage_work_items
              WHERE team_plan_id=$1 AND role=$2 AND stable_key='leader:primary'
              FOR UPDATE",
        )
        .bind(plan.id)
        .bind(&plan.leader_role)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_work_items",
        })?;
        if matches!(
            leader_item.status.as_str(),
            "running" | "waiting_dependency"
        ) {
            let leader_worker = sqlx::query_as::<_, StageWorkerRunRow>(
                "SELECT * FROM stage_worker_runs
                  WHERE work_item_id=$1
                    AND status IN ('running','waiting_background','recovery_required')
                  ORDER BY worker_generation DESC,id DESC LIMIT 1 FOR UPDATE",
            )
            .bind(leader_item.id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(RuntimeMemoryStoreError::Missing {
                entity: "stage_worker_runs",
            })?;
            if leader_worker.status == "recovery_required" {
                return Err(RuntimeMemoryStoreError::Conflict {
                    code: "stage_team_worker_recovery_required",
                });
            }
            let chain_id = leader_worker.message_chain_id.ok_or(
                RuntimeMemoryStoreError::IdentityMismatch {
                    code: "stage_team_leader_chain_missing",
                },
            )?;
            if leader_item.status == "running" {
                if leader_worker.status != "running" {
                    return Err(RuntimeMemoryStoreError::Conflict {
                        code: "stage_team_leader_claim_replay_mismatch",
                    });
                }
                if leader_worker
                    .lease_expires_at
                    .is_some_and(|expires_at| expires_at > chrono::Utc::now())
                {
                    if leader_worker.lease_owner.as_deref() != Some(input.lease_owner.as_str()) {
                        return Err(RuntimeMemoryStoreError::Conflict {
                            code: "stage_team_leader_claim_replay_mismatch",
                        });
                    }
                    tx.commit().await?;
                    return Ok(Some(ClaimedStageWorkItemRow {
                        unit: locked_unit.clone(),
                        plan,
                        work_item: leader_item,
                        worker: leader_worker,
                        message_chain_id: chain_id,
                    }));
                }
                if leader_worker.active_tool_call_id.is_some() {
                    sqlx::query(
                        "UPDATE stage_worker_runs
                            SET status='recovery_required',updated_at=NOW()
                          WHERE id=$1 AND status='running' AND attempt_epoch=$2",
                    )
                    .bind(leader_worker.id)
                    .bind(leader_worker.attempt_epoch)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        "UPDATE stage_work_items
                            SET status='recovery_required',row_version=row_version+1,updated_at=NOW()
                          WHERE id=$1 AND status='running' AND row_version=$2",
                    )
                    .bind(leader_item.id)
                    .bind(leader_item.row_version)
                    .execute(&mut *tx)
                    .await?;
                    tx.commit().await?;
                    return Err(RuntimeMemoryStoreError::Conflict {
                        code: "stage_team_worker_recovery_required",
                    });
                }
                let lease_token = Uuid::new_v4();
                let resumed_worker = sqlx::query_as::<_, StageWorkerRunRow>(
                    r#"UPDATE stage_worker_runs
                          SET lease_token=$7,lease_owner=$8,lease_acquired_at=NOW(),
                              lease_expires_at=NOW()+make_interval(secs => $9),heartbeat_at=NOW(),
                              attempt_epoch=attempt_epoch+1,updated_at=NOW()
                        WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
                          AND stage_run_unit_id=$4 AND work_item_id=$5
                          AND organization_id=$6 AND status='running' AND attempt_epoch=$10
                          AND checkpoint_version=$11 AND active_tool_call_id IS NULL
                          AND lease_expires_at<=NOW()
                        RETURNING *"#,
                )
                .bind(leader_worker.id)
                .bind(plan.operation_id)
                .bind(plan.stage_execution_id)
                .bind(plan.stage_run_unit_id)
                .bind(leader_item.id)
                .bind(plan.organization_id)
                .bind(lease_token)
                .bind(&input.lease_owner)
                .bind(input.lease_seconds)
                .bind(leader_worker.attempt_epoch)
                .bind(leader_worker.checkpoint_version)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(RuntimeMemoryStoreError::LeaseLost {
                    worker_run_id: leader_worker.id,
                    attempt_epoch: leader_worker.attempt_epoch,
                })?;
                tx.commit().await?;
                return Ok(Some(ClaimedStageWorkItemRow {
                    unit: locked_unit.clone(),
                    plan,
                    work_item: leader_item,
                    worker: resumed_worker,
                    message_chain_id: chain_id,
                }));
            }
            if leader_worker.status != "waiting_background" {
                return Err(RuntimeMemoryStoreError::Conflict {
                    code: "stage_team_leader_wait_state_mismatch",
                });
            }
            let dependencies_ready: bool = sqlx::query_scalar(
                r#"SELECT NOT EXISTS (
                       SELECT 1
                         FROM stage_work_item_dependencies AS dependency
                         JOIN stage_work_items AS child
                           ON child.id=dependency.depends_on_work_item_id
                        WHERE dependency.work_item_id=$1
                          AND child.status NOT IN ('completed','exhausted','superseded')
                   ) AND NOT EXISTS (
                       SELECT 1
                         FROM stage_work_item_dependencies AS dependency
                         JOIN stage_work_items AS child
                           ON child.id=dependency.depends_on_work_item_id
                         LEFT JOIN stage_worker_outputs AS output
                           ON output.work_item_id=child.id
                        WHERE dependency.work_item_id=$1
                          AND child.status='completed' AND output.id IS NULL
                   )"#,
            )
            .bind(leader_item.id)
            .fetch_one(&mut *tx)
            .await?;
            if !dependencies_ready {
                tx.commit().await?;
                return Ok(None);
            }
            let resumed_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
                "UPDATE stage_work_items
                    SET status='running',row_version=row_version+1,updated_at=NOW()
                  WHERE id=$1 AND status='waiting_dependency' AND row_version=$2
                  RETURNING *",
            )
            .bind(leader_item.id)
            .bind(leader_item.row_version)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(RuntimeMemoryStoreError::StaleVersion {
                entity: "stage_work_items",
                expected: leader_item.row_version,
                actual: -1,
            })?;
            let lease_token = Uuid::new_v4();
            let resumed_worker = stage_worker_runs::claim_cas(
                &mut *tx,
                leader_worker.id,
                plan.stage_run_unit_id,
                stage_worker_runs::StageWorkerRunStatus::WaitingBackground,
                leader_worker.attempt_epoch,
                lease_token,
                &input.lease_owner,
                input.lease_seconds,
            )
            .await?;
            tx.commit().await?;
            return Ok(Some(ClaimedStageWorkItemRow {
                unit: locked_unit.clone(),
                plan,
                work_item: resumed_item,
                worker: resumed_worker,
                message_chain_id: chain_id,
            }));
        }
    }
    if let Some((_expected_epoch, expected_manifest_hash)) = aggregator {
        let barrier = stage_teams::load_barrier_with_connection_ignoring_worker(
            &mut tx,
            plan.id,
            replaced_final_submitter,
        )
        .await?;
        if !barrier.ready_to_finalize() || barrier.manifest_hash != expected_manifest_hash {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_sibling_barrier_not_ready",
            });
        }
    }
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        r#"SELECT item.*
             FROM stage_work_items AS item
            WHERE item.team_plan_id=$1 AND item.status='queued'
              AND (
                    ($2 AND item.role=$3)
                    OR ($4 AND item.role=$5 AND item.stable_key='leader:primary')
                    OR (NOT $2 AND NOT $4 AND item.role IS DISTINCT FROM $3)
                  )
              AND NOT EXISTS (
                    SELECT 1
                      FROM stage_work_item_dependencies AS dependency
                      JOIN stage_work_items AS prerequisite
                        ON prerequisite.id=dependency.depends_on_work_item_id
                     WHERE dependency.work_item_id=item.id
                       AND prerequisite.status NOT IN ('completed','exhausted','superseded')
                  )
            ORDER BY item.priority,item.created_at,item.id
            LIMIT 1 FOR UPDATE OF item SKIP LOCKED"#,
    )
    .bind(plan.id)
    .bind(is_aggregator_claim)
    .bind(&plan.aggregator_role)
    .bind(is_leader_claim)
    .bind(&plan.leader_role)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(item) = item else {
        tx.commit().await?;
        return Ok(None);
    };
    let resumable_worker = sqlx::query_as::<_, StageWorkerRunRow>(
        "SELECT * FROM stage_worker_runs
          WHERE work_item_id=$1 AND status='queued'
          ORDER BY worker_generation DESC,id DESC
          LIMIT 1 FOR UPDATE",
    )
    .bind(item.id)
    .fetch_optional(&mut *tx)
    .await?;
    if resumable_worker.is_none() {
        let active_workers: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM stage_worker_runs
              WHERE stage_run_unit_id=$1
                AND status IN ('queued','running','waiting_background')",
        )
        .bind(input.stage_run_unit_id)
        .fetch_one(&mut *tx)
        .await?;
        if active_workers >= i64::from(plan.max_workers_active) {
            return Ok(None);
        }
        if stage_teams::enforces_lifetime_worker_total(&plan.dynamic_request_policy) {
            let total_workers: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM stage_worker_runs WHERE stage_run_unit_id=$1",
            )
            .bind(input.stage_run_unit_id)
            .fetch_one(&mut *tx)
            .await?;
            if total_workers >= i64::from(plan.max_workers_total) {
                return Err(RuntimeMemoryStoreError::Conflict {
                    code: "stage_team_worker_lifetime_budget_exhausted",
                });
            }
        }
    }
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "UPDATE stage_work_items
            SET status='running',started_at=COALESCE(started_at,NOW()),
                row_version=row_version+1,updated_at=NOW()
          WHERE id=$1 AND status='queued' AND row_version=$2
          RETURNING *",
    )
    .bind(item.id)
    .bind(item.row_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::StaleVersion {
        entity: "stage_work_items",
        expected: item.row_version,
        actual: -1,
    })?;
    let unit = if locked_unit.status == "queued" {
        stage_run_units::transition_cas(
            &mut *tx,
            locked_unit.id,
            locked_unit.operation_id,
            locked_unit.stage_execution_id,
            locked_unit.organization_id,
            stage_run_units::StageRunUnitStatus::Queued,
            locked_unit.row_version,
            stage_run_units::StageRunUnitStatus::Running,
            None,
        )
        .await?
    } else {
        locked_unit
    };
    if let Some(resumable_worker) = resumable_worker {
        let chain_id =
            resumable_worker
                .message_chain_id
                .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "stage_team_resumable_worker_chain_missing",
                })?;
        if is_aggregator_claim && plan.final_submitter_worker_run_id != Some(resumable_worker.id) {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_team_final_submitter_identity_mismatch",
            });
        }
        let lease_token = Uuid::new_v4();
        let worker = stage_worker_runs::claim_cas(
            &mut *tx,
            resumable_worker.id,
            unit.id,
            stage_worker_runs::StageWorkerRunStatus::Queued,
            resumable_worker.attempt_epoch,
            lease_token,
            &input.lease_owner,
            input.lease_seconds,
        )
        .await?;
        tx.commit().await?;
        return Ok(Some(ClaimedStageWorkItemRow {
            unit,
            plan,
            work_item: item,
            worker,
            message_chain_id: chain_id,
        }));
    }
    let worker_generation_i64: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stage_worker_runs WHERE work_item_id=$1")
            .bind(item.id)
            .fetch_one(&mut *tx)
            .await?;
    let worker_generation =
        i32::try_from(worker_generation_i64).map_err(|_| RuntimeMemoryStoreError::Conflict {
            code: "stage_team_worker_generation_overflow",
        })?;
    let parent_request_id = if item.created_by == "accepted_worker_request" {
        let reason = sqlx::query_scalar::<_, String>(
            "SELECT reason_code FROM stage_worker_requests
              WHERE accepted_work_item_id=$1 AND team_plan_id=$2",
        )
        .bind(item.id)
        .bind(plan.id)
        .fetch_optional(&mut *tx)
        .await?;
        reason.and_then(|reason| controller_request_envelope(&reason).1)
    } else {
        None
    };
    let worker = stage_worker_runs::insert_with_executor(
        &mut *tx,
        &stage_worker_runs::NewStageWorkerRun {
            id: Uuid::new_v4(),
            operation_id: plan.operation_id,
            stage_execution_id: plan.stage_execution_id,
            stage_run_unit_id: plan.stage_run_unit_id,
            work_item_id: Some(item.id),
            organization_id: plan.organization_id,
            worker_generation,
            specialist: item.role.clone(),
            work_item_kind: item.kind.clone(),
            work_item_key: item.stable_key.clone(),
            agent_path: format!(
                "main>stage_run:{}>org:{}>{}:{}",
                plan.stage_kind, plan.organization_id, item.role, item.stable_key
            ),
            parent_request_id,
        },
    )
    .await?;
    let lease_token = Uuid::new_v4();
    let claimed = stage_worker_runs::claim_cas(
        &mut *tx,
        worker.id,
        unit.id,
        stage_worker_runs::StageWorkerRunStatus::Queued,
        0,
        lease_token,
        &input.lease_owner,
        input.lease_seconds,
    )
    .await?;
    let chain_id = Uuid::new_v4();
    message_chains::create_bound_with_executor(
        &mut *tx,
        chain_id,
        input.session_id,
        input.operation_id,
        input.subtask_id,
        input.agent,
        input.model.as_deref(),
        input.provider.as_deref(),
        &input.initial_chain,
    )
    .await?;
    let bound = stage_worker_runs::bind_message_chain_cas(
        &mut *tx,
        claimed.id,
        unit.id,
        lease_token,
        claimed.attempt_epoch,
        chain_id,
    )
    .await?;
    let worker = stage_worker_runs::checkpoint_cas(
        &mut *tx,
        &RuntimeMemoryTxFence {
            operation_id: plan.operation_id,
            stage_execution_id: plan.stage_execution_id,
            stage_run_unit_id: plan.stage_run_unit_id,
            worker_run_id: bound.id,
            lease_token,
            attempt_epoch: bound.attempt_epoch,
            expected_checkpoint_version: bound.checkpoint_version,
        },
        &input.initial_checkpoint,
    )
    .await?;
    let plan = if is_aggregator_claim {
        sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
            "UPDATE stage_team_plans
                SET final_submitter_worker_run_id=$2,row_version=row_version+1,updated_at=NOW()
              WHERE id=$1 AND final_submitter_worker_run_id IS NOT DISTINCT FROM $3
              RETURNING *",
        )
        .bind(plan.id)
        .bind(worker.id)
        .bind(replaced_final_submitter)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_final_submitter_already_claimed",
        })?
    } else {
        plan
    };
    tx.commit().await?;
    Ok(Some(ClaimedStageWorkItemRow {
        unit,
        plan,
        work_item: item,
        worker,
        message_chain_id: chain_id,
    }))
}

/// Park the exact Company Controller behind its already accepted durable
/// children.  WorkItem, WorkerRun, checkpoint and lease release move together
/// so a crash cannot leave an LLM turn live without queue authority.
pub async fn park_stage_team_leader(
    pool: &sqlx::PgPool,
    input: &ParkStageTeamLeaderRow,
) -> RuntimeMemoryStoreResult<ParkedStageTeamLeaderRow> {
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.fence.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    if frozen_runtime_contract(&operation)? != runtime_memory_rollout::RuntimeMemoryContract::V2Only
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_requires_v2_only",
        });
    }
    let unit = load_runtime_unit_for_update(
        &mut tx,
        input.fence.operation_id,
        input.fence.stage_execution_id,
        input.fence.stage_run_unit_id,
    )
    .await?;
    if unit.status != "running" {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_parent_unit_not_running",
        });
    }
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "SELECT * FROM stage_team_plans
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
            AND stage_run_unit_id=$4 FOR UPDATE",
    )
    .bind(input.stage_team_plan_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_team_plans",
    })?;
    if plan.requests_closed_at.is_some()
        || plan.aggregator_role.as_deref() != Some(plan.leader_role.as_str())
        || plan
            .dynamic_request_policy
            .get("coordination_mode")
            .and_then(Value::as_str)
            != Some("company_controller")
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_leader_not_parkable",
        });
    }
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "SELECT * FROM stage_work_items
          WHERE id=$1 AND team_plan_id=$2 AND operation_id=$3
            AND stage_execution_id=$4 AND stage_run_unit_id=$5 FOR UPDATE",
    )
    .bind(input.leader_work_item_id)
    .bind(plan.id)
    .bind(plan.operation_id)
    .bind(plan.stage_execution_id)
    .bind(plan.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_work_items",
    })?;
    if item.role != plan.leader_role
        || item.stable_key != "leader:primary"
        || item.row_version != input.expected_work_item_row_version
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_leader_identity_mismatch",
        });
    }
    let dependency_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stage_work_item_dependencies
          WHERE work_item_id=$1",
    )
    .bind(item.id)
    .fetch_one(&mut *tx)
    .await?;
    if dependency_count == 0 {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_leader_has_no_dependencies",
        });
    }
    if item.status == "waiting_dependency" {
        let worker = sqlx::query_as::<_, StageWorkerRunRow>(
            "SELECT * FROM stage_worker_runs WHERE id=$1 AND work_item_id=$2 FOR SHARE",
        )
        .bind(input.fence.worker_run_id)
        .bind(item.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_worker_runs",
        })?;
        if worker.status == "waiting_background"
            && worker.attempt_epoch == input.fence.attempt_epoch
            && worker.checkpoint_version
                == input.fence.expected_checkpoint_version.saturating_add(1)
            && worker.checkpoint == input.checkpoint
        {
            tx.commit().await?;
            return Ok(ParkedStageTeamLeaderRow {
                plan,
                work_item: item,
                worker,
                dependency_count,
            });
        }
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_leader_park_replay_mismatch",
        });
    }
    if item.status != "running" {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_leader_not_running",
        });
    }
    let worker = sqlx::query_as::<_, StageWorkerRunRow>(
        r#"SELECT * FROM stage_worker_runs
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_run_unit_id=$4 AND work_item_id=$5 AND lease_token=$6
              AND attempt_epoch=$7 AND checkpoint_version=$8 AND status='running'
              AND active_tool_call_id IS NULL AND lease_expires_at > NOW()
            FOR UPDATE"#,
    )
    .bind(input.fence.worker_run_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .bind(item.id)
    .bind(input.fence.lease_token)
    .bind(input.fence.attempt_epoch)
    .bind(input.fence.expected_checkpoint_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::LeaseLost {
        worker_run_id: input.fence.worker_run_id,
        attempt_epoch: input.fence.attempt_epoch,
    })?;
    let parked_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "UPDATE stage_work_items
            SET status='waiting_dependency',row_version=row_version+1,updated_at=NOW()
          WHERE id=$1 AND status='running' AND row_version=$2
          RETURNING *",
    )
    .bind(item.id)
    .bind(item.row_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::StaleVersion {
        entity: "stage_work_items",
        expected: item.row_version,
        actual: -1,
    })?;
    let parked_worker = stage_worker_runs::finish_attempt_cas(
        &mut *tx,
        &input.fence,
        stage_worker_runs::StageWorkerRunStatus::Running,
        stage_worker_runs::StageWorkerRunStatus::WaitingBackground,
        &input.checkpoint,
        worker.evidence_watermark,
    )
    .await?;
    tx.commit().await?;
    Ok(ParkedStageTeamLeaderRow {
        plan,
        work_item: parked_item,
        worker: parked_worker,
        dependency_count,
    })
}

/// After the Controller explicitly prepares final submission and the request
/// epoch is closed, bind that already-running exact WorkerRun as the sole Unit
/// submitter.  No replacement Aggregator is created.
pub async fn bind_stage_team_leader_final_submitter(
    pool: &sqlx::PgPool,
    input: &BindStageTeamLeaderFinalSubmitterRow,
) -> RuntimeMemoryStoreResult<BoundStageTeamLeaderFinalSubmitterRow> {
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.fence.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "SELECT * FROM stage_team_plans
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
            AND stage_run_unit_id=$4 FOR UPDATE",
    )
    .bind(input.stage_team_plan_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_team_plans",
    })?;
    if plan.dispatch_epoch != input.expected_dispatch_epoch
        || plan.requests_closed_at.is_none()
        || plan.aggregator_role.as_deref() != Some(plan.leader_role.as_str())
        || plan
            .dynamic_request_policy
            .get("coordination_mode")
            .and_then(Value::as_str)
            != Some("company_controller")
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_leader_final_submitter_not_bindable",
        });
    }
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "SELECT * FROM stage_work_items
          WHERE id=$1 AND team_plan_id=$2 AND role=$3
            AND stable_key='leader:primary' AND status='running' FOR SHARE",
    )
    .bind(input.leader_work_item_id)
    .bind(plan.id)
    .bind(&plan.leader_role)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_work_items",
    })?;
    let worker = sqlx::query_as::<_, StageWorkerRunRow>(
        r#"SELECT * FROM stage_worker_runs
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_run_unit_id=$4 AND work_item_id=$5 AND lease_token=$6
              AND attempt_epoch=$7 AND checkpoint_version=$8 AND status='running'
              AND active_tool_call_id IS NULL AND lease_expires_at > NOW()
            FOR SHARE"#,
    )
    .bind(input.fence.worker_run_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .bind(item.id)
    .bind(input.fence.lease_token)
    .bind(input.fence.attempt_epoch)
    .bind(input.fence.expected_checkpoint_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::LeaseLost {
        worker_run_id: input.fence.worker_run_id,
        attempt_epoch: input.fence.attempt_epoch,
    })?;
    let barrier = stage_teams::load_barrier_with_connection_ignoring_worker(
        &mut tx,
        plan.id,
        Some(worker.id),
    )
    .await?;
    if !barrier.ready_to_finalize() || barrier.manifest_hash != input.expected_manifest_hash {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_sibling_barrier_not_ready",
        });
    }
    if plan.final_submitter_worker_run_id == Some(worker.id) {
        if plan.row_version == input.expected_plan_row_version
            || plan.row_version == input.expected_plan_row_version.saturating_add(1)
        {
            tx.commit().await?;
            return Ok(BoundStageTeamLeaderFinalSubmitterRow {
                plan,
                barrier,
                replayed: true,
            });
        }
        return Err(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_team_plans",
            expected: input.expected_plan_row_version,
            actual: plan.row_version,
        });
    }
    if plan.final_submitter_worker_run_id.is_some() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_final_submitter_already_claimed",
        });
    }
    if plan.row_version != input.expected_plan_row_version {
        return Err(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_team_plans",
            expected: input.expected_plan_row_version,
            actual: plan.row_version,
        });
    }
    let bound = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "UPDATE stage_team_plans
            SET final_submitter_worker_run_id=$2,row_version=row_version+1,updated_at=NOW()
          WHERE id=$1 AND final_submitter_worker_run_id IS NULL AND row_version=$3
          RETURNING *",
    )
    .bind(plan.id)
    .bind(worker.id)
    .bind(plan.row_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::StaleVersion {
        entity: "stage_team_plans",
        expected: plan.row_version,
        actual: -1,
    })?;
    tx.commit().await?;
    Ok(BoundStageTeamLeaderFinalSubmitterRow {
        plan: bound,
        barrier,
        replayed: false,
    })
}

fn controller_gate_block_checkpoint(
    input: &ReopenStageTeamLeaderAfterGateBlockRow,
    repair_generation: i32,
    fuel_exhausted: bool,
) -> serde_json::Value {
    let runtime_gate = serde_json::json!({
        "deliverable_submission_id": input.deliverable_submission_id,
        "fuel_exhausted": fuel_exhausted,
        "gap_manifest": input.gap_manifest,
        "gap_manifest_hash": input.gap_manifest_hash,
        "gate_decision_hash": input.gate_decision_hash,
        "repair_generation": repair_generation,
        "request_id": input.request_id.trim(),
        "schema_version": 1,
        "source_dispatch_epoch": input.expected_dispatch_epoch,
        "source_manifest_hash": input.expected_manifest_hash,
    });
    match &input.checkpoint {
        serde_json::Value::Object(object) => {
            let mut checkpoint = object.clone();
            checkpoint.insert("_runtime_stage_team_gate_block".to_string(), runtime_gate);
            serde_json::Value::Object(checkpoint)
        }
        checkpoint => serde_json::json!({
            "_runtime_stage_team_gate_block": runtime_gate,
            "controller_checkpoint": checkpoint,
        }),
    }
}

fn controller_gate_gap_replays_exactly(
    gap: &stage_teams::StageTeamUnitGapRow,
    input: &ReopenStageTeamLeaderAfterGateBlockRow,
) -> bool {
    gap.team_plan_id == input.stage_team_plan_id
        && gap.operation_id == input.fence.operation_id
        && gap.stage_execution_id == input.fence.stage_execution_id
        && gap.stage_run_unit_id == input.fence.stage_run_unit_id
        && gap.source_dispatch_epoch == input.expected_dispatch_epoch
        && gap.source_manifest_hash == input.expected_manifest_hash
        && gap.source_attempt_epoch == input.fence.attempt_epoch
        && gap.source_checkpoint_version == input.fence.expected_checkpoint_version
        && gap.source_lease_token == input.fence.lease_token
        && gap.source_aggregator_work_item_id == input.leader_work_item_id
        && gap.source_aggregator_worker_run_id == input.fence.worker_run_id
        && gap.deliverable_submission_id == input.deliverable_submission_id
        && gap.gate_decision_hash == input.gate_decision_hash
        && gap.gap_manifest_hash == input.gap_manifest_hash
        && gap.gap_manifest == input.gap_manifest
}

async fn insert_company_controller_gate_gap(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    input: &ReopenStageTeamLeaderAfterGateBlockRow,
    plan: &stage_teams::StageTeamPlanRow,
    leader_work_item: &stage_teams::StageWorkItemRow,
    leader_worker: &StageWorkerRunRow,
    repair_generation: i32,
    disposition: &str,
) -> RuntimeMemoryStoreResult<stage_teams::StageTeamUnitGapRow> {
    if !matches!(disposition, "opened" | "fuel_exhausted") {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_team_controller_gap_disposition",
        });
    }
    let gap_id = Uuid::new_v5(&plan.id, input.request_id.trim().as_bytes());
    let gap_sql = format!(
        r#"INSERT INTO stage_team_unit_gaps(
               id,request_id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,source_dispatch_epoch,source_manifest_hash,source_attempt_epoch,
               source_checkpoint_version,source_lease_token,source_aggregator_work_item_id,
               source_aggregator_worker_run_id,deliverable_submission_id,gate_decision_hash,
               gap_manifest,gap_manifest_hash,repair_generation,disposition
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)
           RETURNING {}"#,
        stage_teams::UNIT_GAP_COLUMNS
    );
    Ok(
        sqlx::query_as::<_, stage_teams::StageTeamUnitGapRow>(&gap_sql)
            .bind(gap_id)
            .bind(input.request_id.trim())
            .bind(plan.id)
            .bind(plan.operation_id)
            .bind(plan.stage_execution_id)
            .bind(plan.stage_run_unit_id)
            .bind(plan.scope_snapshot_id)
            .bind(plan.organization_id)
            .bind(plan.dispatch_epoch)
            .bind(&input.expected_manifest_hash)
            .bind(input.fence.attempt_epoch)
            .bind(input.fence.expected_checkpoint_version)
            .bind(input.fence.lease_token)
            .bind(leader_work_item.id)
            .bind(leader_worker.id)
            .bind(input.deliverable_submission_id)
            .bind(&input.gate_decision_hash)
            .bind(&input.gap_manifest)
            .bind(&input.gap_manifest_hash)
            .bind(repair_generation)
            .bind(disposition)
            .fetch_one(&mut **tx)
            .await?,
    )
}

/// Persist a deterministic Gate BLOCK for a Company Controller.  A repairable
/// BLOCK reopens the plan and parks the exact same WorkerRun/message chain;
/// fuel exhaustion terminalizes that same Controller and its Unit.  This path
/// never creates a replacement Aggregator or another Controller WorkerRun.
pub async fn reopen_stage_team_leader_after_gate_block(
    pool: &sqlx::PgPool,
    input: &ReopenStageTeamLeaderAfterGateBlockRow,
) -> RuntimeMemoryStoreResult<ReopenedStageTeamLeaderAfterGateBlockRow> {
    let expected_gap_hash = format!(
        "sha256:{}",
        operation_scope_decisions::sha256_json(&input.gap_manifest)
    );
    if input.request_id.trim().is_empty()
        || input.request_id.len() > 256
        || !input.gap_manifest.is_object()
        || input.gap_manifest_hash != expected_gap_hash
        || input
            .gap_manifest
            .get("gate_decision_hash")
            .and_then(Value::as_str)
            != Some(input.gate_decision_hash.as_str())
        || input.expected_manifest_hash.len() != 71
        || !input.expected_manifest_hash.starts_with("sha256:")
        || input.gate_decision_hash.len() != 71
        || !input.gate_decision_hash.starts_with("sha256:")
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_team_controller_gate_gap",
        });
    }

    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.fence.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    if frozen_runtime_contract(&operation)? != runtime_memory_rollout::RuntimeMemoryContract::V2Only
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_requires_v2_only",
        });
    }

    let gap_sql = format!(
        "SELECT {} FROM stage_team_unit_gaps WHERE request_id=$1 FOR UPDATE",
        stage_teams::UNIT_GAP_COLUMNS
    );
    if let Some(gap) = sqlx::query_as::<_, stage_teams::StageTeamUnitGapRow>(&gap_sql)
        .bind(input.request_id.trim())
        .fetch_optional(&mut *tx)
        .await?
    {
        if !controller_gate_gap_replays_exactly(&gap, input) {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_controller_gate_replay_mismatch",
            });
        }
        let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
            "SELECT * FROM stage_team_plans WHERE id=$1 FOR SHARE",
        )
        .bind(input.stage_team_plan_id)
        .fetch_one(&mut *tx)
        .await?;
        let unit = load_runtime_unit_for_update(
            &mut tx,
            input.fence.operation_id,
            input.fence.stage_execution_id,
            input.fence.stage_run_unit_id,
        )
        .await?;
        let leader_work_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
            "SELECT * FROM stage_work_items WHERE id=$1 AND team_plan_id=$2 FOR SHARE",
        )
        .bind(input.leader_work_item_id)
        .bind(input.stage_team_plan_id)
        .fetch_one(&mut *tx)
        .await?;
        let leader_worker = sqlx::query_as::<_, StageWorkerRunRow>(
            "SELECT * FROM stage_worker_runs WHERE id=$1 AND work_item_id=$2 FOR SHARE",
        )
        .bind(input.fence.worker_run_id)
        .bind(input.leader_work_item_id)
        .fetch_one(&mut *tx)
        .await?;
        let fuel_exhausted = gap.disposition == "fuel_exhausted";
        let expected_checkpoint =
            controller_gate_block_checkpoint(input, gap.repair_generation, fuel_exhausted);
        let replay_state_matches = if fuel_exhausted {
            plan.dispatch_epoch == input.expected_dispatch_epoch
                && plan.requests_closed_at.is_some()
                && plan.final_submitter_worker_run_id == Some(leader_worker.id)
                && unit.status == "gate_blocked"
                && leader_work_item.status == "superseded"
                && leader_work_item.terminal_at.is_some()
                && leader_worker.status == "gate_blocked"
                && leader_worker.lease_token.is_none()
                && leader_worker.active_tool_call_id.is_none()
        } else {
            gap.disposition == "opened"
                && plan.dispatch_epoch == input.expected_dispatch_epoch.saturating_add(1)
                && plan.requests_closed_at.is_none()
                && plan.final_submitter_worker_run_id.is_none()
                && unit.status == "running"
                && leader_work_item.status == "waiting_dependency"
                && leader_worker.status == "waiting_background"
        };
        if !replay_state_matches
            || leader_worker.checkpoint_version
                != input.fence.expected_checkpoint_version.saturating_add(1)
            || leader_worker.checkpoint != expected_checkpoint
        {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_controller_gate_replay_state_mismatch",
            });
        }
        let repair_generation = gap.repair_generation;
        tx.commit().await?;
        return Ok(ReopenedStageTeamLeaderAfterGateBlockRow {
            plan,
            unit,
            gap: Some(gap),
            repair_generation,
            fuel_exhausted,
            leader_work_item,
            leader_worker,
            replayed: true,
        });
    }

    let unit = load_runtime_unit_for_update(
        &mut tx,
        input.fence.operation_id,
        input.fence.stage_execution_id,
        input.fence.stage_run_unit_id,
    )
    .await?;
    if !matches!(unit.status.as_str(), "running" | "gate_blocked") {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_controller_unit_not_gate_repairable",
        });
    }
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "SELECT * FROM stage_team_plans
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
            AND stage_run_unit_id=$4 FOR UPDATE",
    )
    .bind(input.stage_team_plan_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_team_plans",
    })?;
    if plan.dispatch_epoch != input.expected_dispatch_epoch
        || plan.requests_closed_at.is_none()
        || plan.final_submitter_worker_run_id != Some(input.fence.worker_run_id)
        || plan.aggregator_kind != "worker"
        || plan.final_submitter_kind != "worker"
        || plan.aggregator_role.as_deref() != Some(plan.leader_role.as_str())
        || plan
            .dynamic_request_policy
            .get("coordination_mode")
            .and_then(Value::as_str)
            != Some("company_controller")
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_controller_gate_not_reopenable",
        });
    }
    let leader_work_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "SELECT * FROM stage_work_items
          WHERE id=$1 AND team_plan_id=$2 AND operation_id=$3
            AND stage_execution_id=$4 AND stage_run_unit_id=$5 FOR UPDATE",
    )
    .bind(input.leader_work_item_id)
    .bind(plan.id)
    .bind(plan.operation_id)
    .bind(plan.stage_execution_id)
    .bind(plan.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_work_items",
    })?;
    if leader_work_item.status == "superseded" && unit.status == "gate_blocked" {
        let leader_worker = sqlx::query_as::<_, StageWorkerRunRow>(
            "SELECT * FROM stage_worker_runs WHERE id=$1 AND work_item_id=$2 FOR SHARE",
        )
        .bind(input.fence.worker_run_id)
        .bind(leader_work_item.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_worker_runs",
        })?;
        let repair_generation = leader_worker
            .checkpoint
            .pointer("/_runtime_stage_team_gate_block/repair_generation")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .ok_or(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_controller_gate_replay_state_mismatch",
            })?;
        let expected_checkpoint = controller_gate_block_checkpoint(input, repair_generation, true);
        let submission_is_exact = sqlx::query_scalar::<_, bool>(
            r#"SELECT EXISTS(
                   SELECT 1 FROM stage_deliverable_submissions
                    WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
                      AND stage_run_unit_id=$4 AND organization_id=$5 AND worker_run_id=$6
                      AND attempt_epoch=$7 AND lease_token=$8
               )"#,
        )
        .bind(input.deliverable_submission_id)
        .bind(plan.operation_id)
        .bind(plan.stage_execution_id)
        .bind(plan.stage_run_unit_id)
        .bind(plan.organization_id)
        .bind(leader_worker.id)
        .bind(input.fence.attempt_epoch)
        .bind(input.fence.lease_token)
        .fetch_one(&mut *tx)
        .await?;
        if leader_work_item.role != plan.leader_role
            || leader_work_item.stable_key != "leader:primary"
            || leader_work_item.required_for_barrier
            || leader_worker.status != "gate_blocked"
            || leader_worker.attempt_epoch != input.fence.attempt_epoch
            || leader_worker.checkpoint_version
                != input.fence.expected_checkpoint_version.saturating_add(1)
            || leader_worker.checkpoint != expected_checkpoint
            || leader_worker.lease_token.is_some()
            || leader_worker.active_tool_call_id.is_some()
            || !submission_is_exact
        {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_controller_gate_replay_state_mismatch",
            });
        }
        tx.commit().await?;
        return Ok(ReopenedStageTeamLeaderAfterGateBlockRow {
            plan,
            unit,
            gap: None,
            repair_generation,
            fuel_exhausted: true,
            leader_work_item,
            leader_worker,
            replayed: true,
        });
    }
    if leader_work_item.role != plan.leader_role
        || leader_work_item.stable_key != "leader:primary"
        || leader_work_item.required_for_barrier
        || leader_work_item.status != "running"
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_controller_gate_leader_mismatch",
        });
    }
    let leader_worker = sqlx::query_as::<_, StageWorkerRunRow>(
        r#"SELECT * FROM stage_worker_runs
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_run_unit_id=$4 AND work_item_id=$5 AND lease_token=$6
              AND attempt_epoch=$7 AND checkpoint_version=$8 AND status='running'
              AND active_tool_call_id IS NULL AND lease_expires_at > NOW()
            FOR UPDATE"#,
    )
    .bind(input.fence.worker_run_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .bind(leader_work_item.id)
    .bind(input.fence.lease_token)
    .bind(input.fence.attempt_epoch)
    .bind(input.fence.expected_checkpoint_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::LeaseLost {
        worker_run_id: input.fence.worker_run_id,
        attempt_epoch: input.fence.attempt_epoch,
    })?;
    if leader_worker.message_chain_id.is_none() {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_controller_chain_missing",
        });
    }
    let submission_is_exact = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
               SELECT 1 FROM stage_deliverable_submissions
                WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
                  AND stage_run_unit_id=$4 AND organization_id=$5 AND worker_run_id=$6
                  AND attempt_epoch=$7 AND lease_token=$8
           )"#,
    )
    .bind(input.deliverable_submission_id)
    .bind(plan.operation_id)
    .bind(plan.stage_execution_id)
    .bind(plan.stage_run_unit_id)
    .bind(plan.organization_id)
    .bind(leader_worker.id)
    .bind(input.fence.attempt_epoch)
    .bind(input.fence.lease_token)
    .fetch_one(&mut *tx)
    .await?;
    if !submission_is_exact {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_controller_gate_submission_mismatch",
        });
    }
    let barrier = stage_teams::load_barrier_with_connection_ignoring_worker(
        &mut tx,
        plan.id,
        Some(leader_worker.id),
    )
    .await?;
    if !barrier.ready_to_finalize() || barrier.manifest_hash != input.expected_manifest_hash {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_sibling_barrier_not_ready",
        });
    }

    let prior_repairs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stage_team_repair_generations WHERE team_plan_id=$1",
    )
    .bind(plan.id)
    .fetch_one(&mut *tx)
    .await?;
    let max_repairs = plan
        .dynamic_request_policy
        .get("max_controller_gate_repairs")
        .and_then(Value::as_i64)
        .or_else(|| {
            plan.dynamic_request_policy
                .get("max_repair_generations")
                .and_then(Value::as_i64)
        })
        .unwrap_or(1)
        .clamp(0, 3);
    let fuel_available = prior_repairs < max_repairs;
    let repair_generation = i32::try_from(prior_repairs.saturating_add(1)).map_err(|_| {
        RuntimeMemoryStoreError::Conflict {
            code: "stage_team_controller_repair_generation_overflow",
        }
    })?;
    let checkpoint = controller_gate_block_checkpoint(input, repair_generation, !fuel_available);
    let gap = insert_company_controller_gate_gap(
        &mut tx,
        input,
        &plan,
        &leader_work_item,
        &leader_worker,
        repair_generation,
        if fuel_available {
            "opened"
        } else {
            "fuel_exhausted"
        },
    )
    .await?;

    if !fuel_available {
        let leader_worker = stage_worker_runs::finish_attempt_cas(
            &mut *tx,
            &input.fence,
            stage_worker_runs::StageWorkerRunStatus::Running,
            stage_worker_runs::StageWorkerRunStatus::GateBlocked,
            &checkpoint,
            leader_worker.evidence_watermark,
        )
        .await?;
        let leader_work_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
            "UPDATE stage_work_items
                SET status='superseded',terminal_at=NOW(),row_version=row_version+1,updated_at=NOW()
              WHERE id=$1 AND status='running' AND row_version=$2
              RETURNING *",
        )
        .bind(leader_work_item.id)
        .bind(leader_work_item.row_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_work_items",
            expected: leader_work_item.row_version,
            actual: -1,
        })?;
        let unit = if unit.status == "running" {
            stage_run_units::transition_cas(
                &mut *tx,
                unit.id,
                unit.operation_id,
                unit.stage_execution_id,
                unit.organization_id,
                stage_run_units::StageRunUnitStatus::Running,
                unit.row_version,
                stage_run_units::StageRunUnitStatus::GateBlocked,
                None,
            )
            .await?
        } else {
            unit
        };
        tx.commit().await?;
        return Ok(ReopenedStageTeamLeaderAfterGateBlockRow {
            plan,
            unit,
            gap: Some(gap),
            repair_generation,
            fuel_exhausted: true,
            leader_work_item,
            leader_worker,
            replayed: false,
        });
    }

    let dispatch_epoch = plan.dispatch_epoch.saturating_add(1);
    let generation_manifest = serde_json::json!({
        "dispatch_epoch": dispatch_epoch,
        "gap_manifest_hash": gap.gap_manifest_hash,
        "kind": "company_controller_gate_reopen",
        "leader_work_item_id": leader_work_item.id,
        "leader_worker_run_id": leader_worker.id,
        "repair_generation": repair_generation,
        "schema_version": 1,
        "source_dispatch_epoch": gap.source_dispatch_epoch,
        "source_gap_id": gap.id,
    });
    let generation_manifest_hash = format!(
        "sha256:{}",
        operation_scope_decisions::sha256_json(&generation_manifest)
    );
    let generation_id = Uuid::new_v5(&gap.id, b"company-controller-repair-generation-v1");
    sqlx::query(
        r#"INSERT INTO stage_team_repair_generations(
               id,team_plan_id,source_gap_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,dispatch_epoch,
               manifest,manifest_hash,status
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'building')"#,
    )
    .bind(generation_id)
    .bind(plan.id)
    .bind(gap.id)
    .bind(plan.operation_id)
    .bind(plan.stage_execution_id)
    .bind(plan.stage_run_unit_id)
    .bind(plan.scope_snapshot_id)
    .bind(plan.organization_id)
    .bind(dispatch_epoch)
    .bind(generation_manifest)
    .bind(generation_manifest_hash)
    .execute(&mut *tx)
    .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "UPDATE stage_team_plans
            SET dispatch_epoch=$2,requests_closed_at=NULL,final_submitter_worker_run_id=NULL,
                row_version=row_version+1,updated_at=NOW()
          WHERE id=$1 AND dispatch_epoch=$3 AND requests_closed_at IS NOT NULL
            AND final_submitter_worker_run_id=$4 AND row_version=$5
          RETURNING *",
    )
    .bind(plan.id)
    .bind(dispatch_epoch)
    .bind(input.expected_dispatch_epoch)
    .bind(leader_worker.id)
    .bind(plan.row_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Conflict {
        code: "stage_team_controller_repair_epoch_advance_cas_failed",
    })?;
    let leader_work_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "UPDATE stage_work_items
            SET status='waiting_dependency',row_version=row_version+1,updated_at=NOW()
          WHERE id=$1 AND status='running' AND row_version=$2
          RETURNING *",
    )
    .bind(leader_work_item.id)
    .bind(leader_work_item.row_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::StaleVersion {
        entity: "stage_work_items",
        expected: leader_work_item.row_version,
        actual: -1,
    })?;
    let leader_worker = stage_worker_runs::finish_attempt_cas(
        &mut *tx,
        &input.fence,
        stage_worker_runs::StageWorkerRunStatus::Running,
        stage_worker_runs::StageWorkerRunStatus::WaitingBackground,
        &checkpoint,
        leader_worker.evidence_watermark,
    )
    .await?;
    let unit = if unit.status == "gate_blocked" {
        stage_run_units::transition_cas(
            &mut *tx,
            unit.id,
            unit.operation_id,
            unit.stage_execution_id,
            unit.organization_id,
            stage_run_units::StageRunUnitStatus::GateBlocked,
            unit.row_version,
            stage_run_units::StageRunUnitStatus::Running,
            None,
        )
        .await?
    } else {
        unit
    };
    tx.commit().await?;
    Ok(ReopenedStageTeamLeaderAfterGateBlockRow {
        plan,
        unit,
        gap: Some(gap),
        repair_generation,
        fuel_exhausted: false,
        leader_work_item,
        leader_worker,
        replayed: false,
    })
}

fn controller_turn_resume_checkpoint(
    checkpoint: &serde_json::Value,
    authority_id: Uuid,
    prior_turn_id: Uuid,
    resume_turn_id: Uuid,
    source_gap_id: Option<Uuid>,
    source_request_id: &str,
    source_gate_decision_hash: &str,
    source_gap_manifest_hash: &str,
    source_dispatch_epoch: i64,
    resume_dispatch_epoch: i64,
    source_gap_manifest: Option<&serde_json::Value>,
) -> RuntimeMemoryStoreResult<serde_json::Value> {
    let serde_json::Value::Object(mut body) = checkpoint.clone() else {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_controller_turn_resume_checkpoint_not_object",
        });
    };
    let mut turn_resume = serde_json::json!({
        "authority_id": authority_id,
        "prior_turn_id": prior_turn_id,
        "resume_turn_id": resume_turn_id,
        "resume_dispatch_epoch": resume_dispatch_epoch,
        "schema_version": 1,
        "source_dispatch_epoch": source_dispatch_epoch,
        "source_gap_id": source_gap_id,
        "source_gap_manifest_hash": source_gap_manifest_hash,
        "source_gate_decision_hash": source_gate_decision_hash,
        "source_request_id": source_request_id,
    });
    if let (Some(turn_resume), Some(source_gap_manifest)) =
        (turn_resume.as_object_mut(), source_gap_manifest)
    {
        turn_resume.insert(
            "source_gap_manifest".to_string(),
            source_gap_manifest.clone(),
        );
    }
    body.insert("_runtime_stage_team_turn_resume".to_string(), turn_resume);
    Ok(serde_json::Value::Object(body))
}

/// Within the exact Task/Operation Turn-claim transaction, re-arm every
/// current-stage Company Controller that was durably terminalized only because
/// its one automatic Gate-repair round was exhausted.  All authority is read
/// from locked DB rows; the caller supplies only the operation and the two Turn
/// witnesses already participating in the top-level CAS.
pub(crate) async fn resume_company_controllers_for_successor_turn_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    operation_id: Uuid,
    prior_turn_id: Uuid,
    resume_turn_id: Uuid,
) -> RuntimeMemoryStoreResult<usize> {
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(operation_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    if frozen_runtime_contract(&operation)? != runtime_memory_rollout::RuntimeMemoryContract::V2Only
    {
        let terminal_controller_exists = sqlx::query_scalar::<_, bool>(
            r#"SELECT EXISTS(
                   SELECT 1
                     FROM stage_team_plans plan
                     JOIN stage_worker_runs worker
                       ON worker.id=plan.final_submitter_worker_run_id
                    WHERE plan.operation_id=$1
                      AND plan.dynamic_request_policy->>'coordination_mode'='company_controller'
                      AND worker.status='gate_blocked'
                      AND worker.checkpoint #>> '{_runtime_stage_team_gate_block,fuel_exhausted}'='true'
               )"#,
        )
        .bind(operation_id)
        .fetch_one(&mut **tx)
        .await?;
        if terminal_controller_exists {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_controller_turn_resume_requires_v2_only",
            });
        }
        return Ok(0);
    }

    let turns_match = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM operation_turns prior_turn
                 JOIN operation_turns resume_turn
                   ON resume_turn.operation_id=prior_turn.operation_id
                  AND resume_turn.ordinal=prior_turn.ordinal+1
                WHERE prior_turn.id=$1 AND resume_turn.id=$2
                  AND prior_turn.operation_id=$3
                  AND prior_turn.status='interrupted'
                  AND resume_turn.status='running'
           )"#,
    )
    .bind(prior_turn_id)
    .bind(resume_turn_id)
    .bind(operation_id)
    .fetch_one(&mut **tx)
    .await?;
    if !turns_match {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_controller_turn_resume_turn_mismatch",
        });
    }

    let terminal_controller_count = sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*)
              FROM stage_runs execution
              JOIN stage_run_units unit
                ON unit.operation_id=execution.operation_id
               AND unit.stage_execution_id=execution.id
               AND unit.stage_kind=execution.stage_kind
              JOIN stage_team_plans plan
                ON plan.operation_id=unit.operation_id
               AND plan.stage_execution_id=unit.stage_execution_id
               AND plan.stage_run_unit_id=unit.id
               AND plan.organization_id=unit.organization_id
              JOIN stage_worker_runs worker
                ON worker.id=plan.final_submitter_worker_run_id
             WHERE execution.operation_id=$1
               AND execution.status='started'
               AND execution.stage_kind=$2
               AND unit.status='gate_blocked'
               AND plan.requests_closed_at IS NOT NULL
               AND plan.dynamic_request_policy->>'coordination_mode'='company_controller'
               AND worker.status='gate_blocked'"#,
    )
    .bind(operation_id)
    .bind(&operation.current_stage)
    .fetch_one(&mut **tx)
    .await?;

    let candidates = sqlx::query_as::<_, (Uuid, Uuid, Uuid, Uuid)>(
        r#"SELECT plan.id,unit.id,item.id,worker.id
              FROM stage_runs execution
              JOIN stage_run_units unit
                ON unit.operation_id=execution.operation_id
               AND unit.stage_execution_id=execution.id
               AND unit.stage_kind=execution.stage_kind
              JOIN stage_team_plans plan
                ON plan.operation_id=unit.operation_id
               AND plan.stage_execution_id=unit.stage_execution_id
               AND plan.stage_run_unit_id=unit.id
               AND plan.scope_snapshot_id=unit.scope_snapshot_id
               AND plan.organization_id=unit.organization_id
              JOIN stage_worker_runs worker
                ON worker.id=plan.final_submitter_worker_run_id
               AND worker.operation_id=plan.operation_id
               AND worker.stage_execution_id=plan.stage_execution_id
               AND worker.stage_run_unit_id=plan.stage_run_unit_id
               AND worker.organization_id=plan.organization_id
              JOIN stage_work_items item
                ON item.id=worker.work_item_id
               AND item.team_plan_id=plan.id
               AND item.operation_id=plan.operation_id
               AND item.stage_execution_id=plan.stage_execution_id
               AND item.stage_run_unit_id=plan.stage_run_unit_id
               AND item.organization_id=plan.organization_id
             WHERE execution.operation_id=$1
               AND execution.status='started'
               AND execution.stage_kind=$2
               AND unit.status='gate_blocked'
               AND plan.requests_closed_at IS NOT NULL
               AND plan.dynamic_request_policy->>'coordination_mode'='company_controller'
               AND plan.aggregator_kind='worker'
               AND plan.aggregator_role=plan.leader_role
               AND plan.final_submitter_kind='worker'
               AND item.stable_key='leader:primary'
               AND item.role=plan.leader_role
               AND item.required_for_barrier=FALSE
               AND item.created_by='server_seed'
               AND item.status='superseded'
               AND item.terminal_at IS NOT NULL
               AND worker.status='gate_blocked'
               AND worker.message_chain_id IS NOT NULL
               AND worker.lease_token IS NULL
               AND worker.active_tool_call_id IS NULL
               AND worker.checkpoint #>> '{_runtime_stage_team_gate_block,fuel_exhausted}'='true'
             ORDER BY plan.organization_id,plan.id
             FOR UPDATE OF unit,plan,item,worker"#,
    )
    .bind(operation_id)
    .bind(&operation.current_stage)
    .fetch_all(&mut **tx)
    .await?;
    if i64::try_from(candidates.len()).unwrap_or(i64::MAX) != terminal_controller_count {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_controller_turn_resume_candidate_mismatch",
        });
    }

    let mut resumed = 0usize;
    for (plan_id, unit_id, item_id, worker_id) in candidates {
        let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
            "SELECT * FROM stage_team_plans WHERE id=$1 FOR UPDATE",
        )
        .bind(plan_id)
        .fetch_one(&mut **tx)
        .await?;
        let unit =
            load_runtime_unit_for_update(tx, plan.operation_id, plan.stage_execution_id, unit_id)
                .await?;
        let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
            "SELECT * FROM stage_work_items WHERE id=$1 FOR UPDATE",
        )
        .bind(item_id)
        .fetch_one(&mut **tx)
        .await?;
        let worker = sqlx::query_as::<_, StageWorkerRunRow>(
            "SELECT * FROM stage_worker_runs WHERE id=$1 FOR UPDATE",
        )
        .bind(worker_id)
        .fetch_one(&mut **tx)
        .await?;
        let message_chain_id =
            worker
                .message_chain_id
                .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "stage_team_controller_turn_resume_chain_missing",
                })?;
        let runtime_gate = worker
            .checkpoint
            .pointer("/_runtime_stage_team_gate_block")
            .and_then(serde_json::Value::as_object)
            .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_team_controller_turn_resume_gate_checkpoint_missing",
            })?;
        let gate_string = |key: &str| {
            runtime_gate
                .get(key)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "stage_team_controller_turn_resume_gate_checkpoint_invalid",
                })
        };
        let request_id = gate_string("request_id")?;
        let deliverable_submission_id = Uuid::parse_str(gate_string("deliverable_submission_id")?)
            .map_err(|_| RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_team_controller_turn_resume_submission_invalid",
            })?;
        let source_manifest_hash = gate_string("source_manifest_hash")?;
        let gate_decision_hash = gate_string("gate_decision_hash")?;
        let gap_manifest_hash = gate_string("gap_manifest_hash")?;
        let source_dispatch_epoch = runtime_gate
            .get("source_dispatch_epoch")
            .and_then(serde_json::Value::as_i64)
            .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_team_controller_turn_resume_source_epoch_missing",
            })?;
        let repair_generation = runtime_gate
            .get("repair_generation")
            .and_then(serde_json::Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_team_controller_turn_resume_generation_invalid",
            })?;
        let expected_request_id = format!(
            "stage-team-repair:{}:{}:{}",
            plan.id, source_dispatch_epoch, gate_decision_hash
        );
        if runtime_gate
            .get("fuel_exhausted")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
            || source_dispatch_epoch != plan.dispatch_epoch
            || unit.status != "gate_blocked"
            || item.status != "superseded"
            || worker.status != "gate_blocked"
            || request_id != expected_request_id
            || worker.lease_token.is_some()
            || worker.active_tool_call_id.is_some()
        {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_controller_turn_resume_source_state_mismatch",
            });
        }

        let submission = sqlx::query_as::<_, StageDeliverableSubmissionRow>(
            "SELECT * FROM stage_deliverable_submissions WHERE id=$1 FOR SHARE",
        )
        .bind(deliverable_submission_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_deliverable_submissions",
        })?;
        let source_lease_token =
            submission
                .lease_token
                .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "stage_team_controller_turn_resume_submission_lease_missing",
                })?;
        if submission.operation_id != plan.operation_id
            || submission.stage_execution_id != plan.stage_execution_id
            || submission.stage_run_unit_id != Some(plan.stage_run_unit_id)
            || submission.organization_id != Some(plan.organization_id)
            || submission.worker_run_id != Some(worker.id)
            || submission.attempt_epoch != Some(worker.attempt_epoch)
            || worker.checkpoint_version <= 0
        {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_team_controller_turn_resume_submission_mismatch",
            });
        }
        let source_checkpoint_version = worker.checkpoint_version.saturating_sub(1);
        let source_checkpoint_hash = format!(
            "sha256:{}",
            operation_scope_decisions::sha256_json(&worker.checkpoint)
        );

        let gap_sql = format!(
            "SELECT {} FROM stage_team_unit_gaps WHERE request_id=$1 FOR UPDATE",
            stage_teams::UNIT_GAP_COLUMNS
        );
        let gap = sqlx::query_as::<_, stage_teams::StageTeamUnitGapRow>(&gap_sql)
            .bind(request_id)
            .fetch_optional(&mut **tx)
            .await?;
        if gap.is_none() {
            let legacy_witness = sqlx::query_scalar::<_, Option<String>>(
                "SELECT legacy_controller_gap_checkpoint_hash
                   FROM stage_worker_runs WHERE id=$1",
            )
            .bind(worker.id)
            .fetch_one(&mut **tx)
            .await?;
            if legacy_witness.as_deref() != Some(source_checkpoint_hash.as_str())
                || runtime_gate
                    .get("schema_version")
                    .and_then(serde_json::Value::as_i64)
                    != Some(1)
            {
                return Err(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "stage_team_controller_turn_resume_legacy_witness_missing",
                });
            }
        }
        if let Some(gap) = gap.as_ref() {
            if gap.team_plan_id != plan.id
                || gap.operation_id != plan.operation_id
                || gap.stage_execution_id != plan.stage_execution_id
                || gap.stage_run_unit_id != plan.stage_run_unit_id
                || gap.scope_snapshot_id != plan.scope_snapshot_id
                || gap.organization_id != plan.organization_id
                || gap.source_dispatch_epoch != plan.dispatch_epoch
                || gap.source_manifest_hash != source_manifest_hash
                || gap.source_attempt_epoch != worker.attempt_epoch
                || gap.source_checkpoint_version != source_checkpoint_version
                || gap.source_lease_token != source_lease_token
                || gap.source_aggregator_work_item_id != item.id
                || gap.source_aggregator_worker_run_id != worker.id
                || gap.deliverable_submission_id != submission.id
                || gap.gate_decision_hash != gate_decision_hash
                || gap.gap_manifest_hash != gap_manifest_hash
                || gap.repair_generation != repair_generation
                || gap.disposition != "fuel_exhausted"
            {
                return Err(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "stage_team_controller_turn_resume_gap_mismatch",
                });
            }
        }
        let barrier =
            stage_teams::load_barrier_with_connection_ignoring_worker(tx, plan.id, Some(worker.id))
                .await?;
        if !barrier.ready_to_finalize() || barrier.manifest_hash != source_manifest_hash {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_controller_turn_resume_barrier_mismatch",
            });
        }

        let prior_generations: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM stage_team_repair_generations WHERE team_plan_id=$1",
        )
        .bind(plan.id)
        .fetch_one(&mut **tx)
        .await?;
        if i64::from(repair_generation) != prior_generations.saturating_add(1) {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_team_controller_turn_resume_generation_mismatch",
            });
        }
        let prior_turn_resumes: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM stage_team_controller_turn_resumes WHERE team_plan_id=$1",
        )
        .bind(plan.id)
        .fetch_one(&mut **tx)
        .await?;
        let max_controller_turn_resumes = plan
            .dynamic_request_policy
            .get("max_controller_turn_resumes")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(MAX_COMPANY_CONTROLLER_SUCCESSOR_TURNS)
            .clamp(0, MAX_COMPANY_CONTROLLER_SUCCESSOR_TURNS);
        if prior_turn_resumes >= max_controller_turn_resumes {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_controller_turn_resume_fuel_exhausted",
            });
        }

        let resume_dispatch_epoch = plan.dispatch_epoch.saturating_add(1);
        let authority_id = Uuid::new_v5(
            &resume_turn_id,
            format!("company-controller-successor-turn:{}", plan.id).as_bytes(),
        );
        sqlx::query(
            r#"INSERT INTO stage_team_controller_turn_resumes(
                   id,operation_id,prior_turn_id,resume_turn_id,team_plan_id,source_gap_id,
                   source_request_id,deliverable_submission_id,source_lease_token,
                   source_manifest_hash,source_gate_decision_hash,
                   source_gap_manifest_hash,source_repair_generation,
                   stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
                   leader_work_item_id,leader_worker_run_id,message_chain_id,
                   source_dispatch_epoch,resume_dispatch_epoch,source_plan_row_version,
                   source_unit_row_version,source_item_row_version,source_attempt_epoch,
                   source_checkpoint_version,source_checkpoint,source_checkpoint_hash,status
               ) VALUES(
                   $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,
                   $16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,'building'
               )"#,
        )
        .bind(authority_id)
        .bind(operation_id)
        .bind(prior_turn_id)
        .bind(resume_turn_id)
        .bind(plan.id)
        .bind(gap.as_ref().map(|gap| gap.id))
        .bind(request_id)
        .bind(deliverable_submission_id)
        .bind(source_lease_token)
        .bind(source_manifest_hash)
        .bind(gate_decision_hash)
        .bind(gap_manifest_hash)
        .bind(repair_generation)
        .bind(plan.stage_execution_id)
        .bind(plan.stage_run_unit_id)
        .bind(plan.scope_snapshot_id)
        .bind(plan.organization_id)
        .bind(item.id)
        .bind(worker.id)
        .bind(message_chain_id)
        .bind(plan.dispatch_epoch)
        .bind(resume_dispatch_epoch)
        .bind(plan.row_version)
        .bind(unit.row_version)
        .bind(item.row_version)
        .bind(worker.attempt_epoch)
        .bind(worker.checkpoint_version)
        .bind(&worker.checkpoint)
        .bind(source_checkpoint_hash)
        .execute(&mut **tx)
        .await?;

        let resumed_plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
            r#"UPDATE stage_team_plans
                  SET dispatch_epoch=$2,requests_closed_at=NULL,
                      final_submitter_worker_run_id=NULL,
                      row_version=row_version+1,updated_at=NOW()
                WHERE id=$1 AND dispatch_epoch=$3 AND row_version=$4
                  AND requests_closed_at IS NOT NULL
                  AND final_submitter_worker_run_id=$5
                RETURNING *"#,
        )
        .bind(plan.id)
        .bind(resume_dispatch_epoch)
        .bind(plan.dispatch_epoch)
        .bind(plan.row_version)
        .bind(worker.id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_controller_turn_resume_plan_cas_failed",
        })?;
        let resumed_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
            r#"UPDATE stage_work_items
                  SET status='waiting_dependency',terminal_at=NULL,
                      row_version=row_version+1,updated_at=NOW()
                WHERE id=$1 AND status='superseded' AND row_version=$2
                RETURNING *"#,
        )
        .bind(item.id)
        .bind(item.row_version)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_controller_turn_resume_item_cas_failed",
        })?;
        let resumed_checkpoint = controller_turn_resume_checkpoint(
            &worker.checkpoint,
            authority_id,
            prior_turn_id,
            resume_turn_id,
            gap.as_ref().map(|gap| gap.id),
            request_id,
            gate_decision_hash,
            gap_manifest_hash,
            plan.dispatch_epoch,
            resume_dispatch_epoch,
            gap.as_ref().map(|gap| &gap.gap_manifest),
        )?;
        let resumed_worker = sqlx::query_as::<_, StageWorkerRunRow>(
            r#"UPDATE stage_worker_runs
                  SET status='waiting_background',checkpoint=$2,
                      checkpoint_version=checkpoint_version+1,
                      terminal_at=NULL,updated_at=NOW()
                WHERE id=$1 AND status='gate_blocked'
                  AND attempt_epoch=$3 AND checkpoint_version=$4
                  AND message_chain_id=$5 AND checkpoint=$6
                  AND lease_token IS NULL AND active_tool_call_id IS NULL
                RETURNING *"#,
        )
        .bind(worker.id)
        .bind(&resumed_checkpoint)
        .bind(worker.attempt_epoch)
        .bind(worker.checkpoint_version)
        .bind(message_chain_id)
        .bind(&worker.checkpoint)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_controller_turn_resume_worker_cas_failed",
        })?;
        let resumed_unit = stage_run_units::transition_cas(
            &mut **tx,
            unit.id,
            unit.operation_id,
            unit.stage_execution_id,
            unit.organization_id,
            stage_run_units::StageRunUnitStatus::GateBlocked,
            unit.row_version,
            stage_run_units::StageRunUnitStatus::Running,
            None,
        )
        .await?;
        let applied = sqlx::query_scalar::<_, Uuid>(
            r#"UPDATE stage_team_controller_turn_resumes
                  SET status='applied',applied_at=NOW()
                WHERE id=$1 AND status='building'
                RETURNING id"#,
        )
        .bind(authority_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_controller_turn_resume_apply_failed",
        })?;
        debug_assert_eq!(applied, authority_id);
        debug_assert_eq!(resumed_plan.dispatch_epoch, resume_dispatch_epoch);
        debug_assert_eq!(resumed_item.status, "waiting_dependency");
        debug_assert_eq!(resumed_worker.status, "waiting_background");
        debug_assert_eq!(resumed_unit.status, "running");
        resumed = resumed.saturating_add(1);
    }
    Ok(resumed)
}

pub async fn claim_stage_work_item(
    pool: &sqlx::PgPool,
    input: &ClaimStageWorkItemRow,
) -> RuntimeMemoryStoreResult<Option<ClaimedStageWorkItemRow>> {
    claim_stage_team_item(pool, input, None, false).await
}

pub async fn claim_stage_team_leader(
    pool: &sqlx::PgPool,
    input: &ClaimStageTeamLeaderRow,
) -> RuntimeMemoryStoreResult<Option<ClaimedStageWorkItemRow>> {
    claim_stage_team_item(pool, &input.claim, None, true).await
}

pub async fn claim_stage_aggregator(
    pool: &sqlx::PgPool,
    input: &ClaimStageAggregatorRow,
) -> RuntimeMemoryStoreResult<ClaimedStageWorkItemRow> {
    claim_stage_team_item(
        pool,
        &input.claim,
        Some((input.expected_dispatch_epoch, &input.expected_manifest_hash)),
        false,
    )
    .await?
    .ok_or(RuntimeMemoryStoreError::Conflict {
        code: "stage_team_aggregator_not_claimable",
    })
}

pub async fn claim_worker_and_bind_chain(
    pool: &sqlx::PgPool,
    input: &ClaimWorkerAndBindChainRow,
) -> RuntimeMemoryStoreResult<ClaimedWorkerAndChainRow> {
    if input.lease_owner.trim().is_empty()
        || input.lease_seconds <= 0
        || input.parent_chain_id.is_some()
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_worker_chain_claim",
        });
    }
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    let contract = frozen_runtime_contract(&operation)?;
    if !contract_writes_v2(contract) {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "runtime_v2_not_enabled",
        });
    }
    let locked_unit = load_runtime_unit_for_update(
        &mut tx,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
    )
    .await?;
    validate_runtime_stage_execution(
        &mut tx,
        &operation,
        input.stage_execution_id,
        &locked_unit.stage_kind,
    )
    .await?;
    if locked_unit.status != input.expected_unit_status.as_str()
        || locked_unit.row_version != input.expected_unit_row_version
    {
        return Err(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_run_units",
            expected: input.expected_unit_row_version,
            actual: locked_unit.row_version,
        });
    }
    let unit = if input.expected_unit_status
        == crate::repo::stage_run_units::StageRunUnitStatus::Running
    {
        locked_unit
    } else {
        crate::repo::stage_run_units::transition_cas(
            &mut *tx,
            input.stage_run_unit_id,
            input.operation_id,
            input.stage_execution_id,
            locked_unit.organization_id,
            input.expected_unit_status,
            input.expected_unit_row_version,
            crate::repo::stage_run_units::StageRunUnitStatus::Running,
            None,
        )
        .await?
    };
    let lease_token = Uuid::new_v4();
    let claimed = stage_worker_runs::claim_cas(
        &mut *tx,
        input.worker_run_id,
        input.stage_run_unit_id,
        input.expected_worker_status,
        input.expected_attempt_epoch,
        lease_token,
        &input.lease_owner,
        input.lease_seconds,
    )
    .await?;
    if claimed.operation_id != input.operation_id
        || claimed.stage_execution_id != input.stage_execution_id
        || claimed.organization_id != unit.organization_id
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "claimed_worker_unit_identity_mismatch",
        });
    }

    let (message_chain_id, worker) = match claimed.message_chain_id {
        Some(chain_id) => {
            let exact = sqlx::query_scalar::<_, bool>(
                r#"SELECT EXISTS (
                       SELECT 1 FROM message_chains
                        WHERE id=$1 AND session_id=$2 AND task_id=$3 AND agent=$4
                   )"#,
            )
            .bind(chain_id)
            .bind(input.session_id)
            .bind(input.operation_id)
            .bind(input.agent)
            .fetch_one(&mut *tx)
            .await?;
            if !exact {
                return Err(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "bound_worker_chain_identity_mismatch",
                });
            }
            (chain_id, claimed)
        }
        None => {
            let chain_id = Uuid::new_v4();
            message_chains::create_bound_with_executor(
                &mut *tx,
                chain_id,
                input.session_id,
                input.operation_id,
                input.subtask_id,
                input.agent,
                input.model.as_deref(),
                input.provider.as_deref(),
                &input.initial_chain,
            )
            .await?;
            let bound = stage_worker_runs::bind_message_chain_cas(
                &mut *tx,
                input.worker_run_id,
                input.stage_run_unit_id,
                lease_token,
                claimed.attempt_epoch,
                chain_id,
            )
            .await?;
            let initial_fence = RuntimeMemoryTxFence {
                operation_id: input.operation_id,
                stage_execution_id: input.stage_execution_id,
                stage_run_unit_id: input.stage_run_unit_id,
                worker_run_id: input.worker_run_id,
                lease_token,
                attempt_epoch: bound.attempt_epoch,
                expected_checkpoint_version: bound.checkpoint_version,
            };
            let checkpointed = stage_worker_runs::checkpoint_cas(
                &mut *tx,
                &initial_fence,
                &input.initial_checkpoint,
            )
            .await?;
            (chain_id, checkpointed)
        }
    };
    if contract_writes_legacy_mirror(contract) {
        let mut legacy_blob = operation.state_blob.clone();
        let organization_name = frozen_organization_name(&mut tx, &unit).await?;
        apply_legacy_worker_mirror(
            &mut legacy_blob,
            &unit.stage_kind,
            &organization_name,
            &worker,
        );
        write_locked_legacy_state_blob(
            &mut tx,
            input.operation_id,
            &operation.runtime_memory_contract,
            &legacy_blob,
        )
        .await?;
        runtime_memory_shadow::persist_worker_sample(&mut tx, worker.id, "claim_and_bind_chain")
            .await?;
    }
    tx.commit().await?;
    if contract_writes_legacy_mirror(contract) {
        reconcile_deployment_rollouts_best_effort(pool, "claim_worker_and_bind_chain").await;
    }
    Ok(ClaimedWorkerAndChainRow {
        unit,
        worker,
        message_chain_id,
    })
}

async fn lock_operation_and_unit_for_fence(
    connection: &mut sqlx::PgConnection,
    fence: &RuntimeMemoryTxFence,
) -> RuntimeMemoryStoreResult<(
    OperationStateRow,
    runtime_memory_rollout::RuntimeMemoryContract,
    StageRunUnitRow,
)> {
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(fence.operation_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    let contract = frozen_runtime_contract(&operation)?;
    if !contract_writes_v2(contract) {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "runtime_v2_not_enabled",
        });
    }
    let unit = load_runtime_unit_for_update(
        connection,
        fence.operation_id,
        fence.stage_execution_id,
        fence.stage_run_unit_id,
    )
    .await?;
    validate_runtime_stage_execution(
        connection,
        &operation,
        fence.stage_execution_id,
        &unit.stage_kind,
    )
    .await?;
    Ok((operation, contract, unit))
}

async fn mirror_worker_if_required(
    connection: &mut sqlx::PgConnection,
    operation: &OperationStateRow,
    contract: runtime_memory_rollout::RuntimeMemoryContract,
    unit: &StageRunUnitRow,
    worker: &StageWorkerRunRow,
    mutation_kind: &str,
) -> RuntimeMemoryStoreResult<()> {
    if contract_writes_legacy_mirror(contract) {
        let organization_name = frozen_organization_name(connection, unit).await?;
        let mut legacy_blob = operation.state_blob.clone();
        apply_legacy_worker_mirror(
            &mut legacy_blob,
            &unit.stage_kind,
            &organization_name,
            worker,
        );
        write_locked_legacy_state_blob(
            connection,
            operation.operation_id,
            &operation.runtime_memory_contract,
            &legacy_blob,
        )
        .await?;
        runtime_memory_shadow::persist_worker_sample(connection, worker.id, mutation_kind).await?;
    }
    Ok(())
}

pub async fn checkpoint_worker(
    pool: &sqlx::PgPool,
    fence: &RuntimeMemoryTxFence,
    checkpoint: &serde_json::Value,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow> {
    let mut tx = pool.begin().await?;
    let (operation, contract, unit) = lock_operation_and_unit_for_fence(&mut tx, fence).await?;
    let worker = stage_worker_runs::checkpoint_cas(&mut *tx, fence, checkpoint).await?;
    mirror_worker_if_required(&mut tx, &operation, contract, &unit, &worker, "checkpoint").await?;
    tx.commit().await?;
    if contract_writes_legacy_mirror(contract) {
        reconcile_deployment_rollouts_best_effort(pool, "checkpoint_worker").await;
    }
    Ok(worker)
}

/// Persist the provider chain body and its worker checkpoint under the same
/// fencing tuple and transaction. A crash cannot expose a new chain with an old
/// checkpoint (or the inverse).
pub async fn checkpoint_bound_worker_chain(
    pool: &sqlx::PgPool,
    input: &CheckpointBoundWorkerChainRow,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow> {
    let mut tx = pool.begin().await?;
    let (worker, contract) = checkpoint_bound_worker_chain_in_transaction(&mut tx, input).await?;
    tx.commit().await?;
    if contract_writes_legacy_mirror(contract) {
        reconcile_deployment_rollouts_best_effort(pool, "checkpoint_bound_worker_chain").await;
    }
    Ok(worker)
}

/// Transaction-owned variant used by the Candidate terminal barrier. The
/// caller appends the immutable barrier before committing, so the chain,
/// Worker checkpoint and barrier can never become partially visible.
pub(super) async fn checkpoint_bound_worker_chain_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    input: &CheckpointBoundWorkerChainRow,
) -> RuntimeMemoryStoreResult<(
    StageWorkerRunRow,
    runtime_memory_rollout::RuntimeMemoryContract,
)> {
    if !input.chain.is_array() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_bound_chain_shape",
        });
    }
    let (operation, contract, unit) = lock_operation_and_unit_for_fence(tx, &input.fence).await?;
    let exact_worker = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM stage_worker_runs
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_run_unit_id=$4 AND lease_token=$5
              AND attempt_epoch=$6 AND checkpoint_version=$7
              AND message_chain_id=$8
              AND status IN ('running','waiting_background','gate_blocked')
              AND lease_expires_at > NOW()
            FOR UPDATE"#,
    )
    .bind(input.fence.worker_run_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .bind(input.fence.lease_token)
    .bind(input.fence.attempt_epoch)
    .bind(input.fence.expected_checkpoint_version)
    .bind(input.message_chain_id)
    .fetch_optional(&mut **tx)
    .await?;
    if exact_worker.is_none() {
        return Err(RuntimeMemoryStoreError::LeaseLost {
            worker_run_id: input.fence.worker_run_id,
            attempt_epoch: input.fence.attempt_epoch,
        });
    }
    let chain_rows = message_chains::update_bound_chain_cas_with_executor(
        &mut **tx,
        input.message_chain_id,
        input.fence.operation_id,
        &input.chain,
    )
    .await?;
    if chain_rows != 1 {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "bound_worker_chain_identity_mismatch",
        });
    }
    let worker =
        stage_worker_runs::checkpoint_cas(&mut **tx, &input.fence, &input.checkpoint).await?;
    mirror_worker_if_required(
        tx,
        &operation,
        contract,
        &unit,
        &worker,
        "checkpoint_bound_chain",
    )
    .await?;
    Ok((worker, contract))
}

pub async fn heartbeat_worker(
    pool: &sqlx::PgPool,
    fence: &RuntimeMemoryTxFence,
    extend_seconds: i32,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow> {
    let mut tx = pool.begin().await?;
    let (operation, contract, unit) = lock_operation_and_unit_for_fence(&mut tx, fence).await?;
    let worker = stage_worker_runs::heartbeat_cas(&mut *tx, fence, extend_seconds).await?;
    mirror_worker_if_required(&mut tx, &operation, contract, &unit, &worker, "heartbeat").await?;
    tx.commit().await?;
    if contract_writes_legacy_mirror(contract) {
        reconcile_deployment_rollouts_best_effort(pool, "heartbeat_worker").await;
    }
    Ok(worker)
}

async fn begin_worker_tool_once(
    pool: &sqlx::PgPool,
    fence: &RuntimeMemoryTxFence,
    tool_call_record_id: Uuid,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow> {
    let mut tx = pool.begin().await?;
    let (operation, contract, unit) = lock_operation_and_unit_for_fence(&mut tx, fence).await?;
    let worker = stage_worker_runs::begin_tool_cas(&mut *tx, fence, tool_call_record_id).await?;
    mirror_worker_if_required(&mut tx, &operation, contract, &unit, &worker, "tool_begin").await?;
    tx.commit().await?;
    if contract_writes_legacy_mirror(contract) {
        reconcile_deployment_rollouts_best_effort(pool, "begin_worker_tool").await;
    }
    Ok(worker)
}

pub async fn begin_worker_tool(
    pool: &sqlx::PgPool,
    fence: &RuntimeMemoryTxFence,
    tool_call_record_id: Uuid,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow> {
    worker_tool_transaction_retry_runner(
        "tool_begin",
        fence.worker_run_id,
        tool_call_record_id,
        || begin_worker_tool_once(pool, fence, tool_call_record_id),
        is_retryable_runtime_transaction_error,
    )
    .await
}

async fn finish_worker_tool_once(
    pool: &sqlx::PgPool,
    fence: &RuntimeMemoryTxFence,
    tool_call_record_id: Uuid,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow> {
    let mut tx = pool.begin().await?;
    let (operation, contract, unit) = lock_operation_and_unit_for_fence(&mut tx, fence).await?;
    let worker = stage_worker_runs::finish_tool_cas(&mut *tx, fence, tool_call_record_id).await?;
    mirror_worker_if_required(&mut tx, &operation, contract, &unit, &worker, "tool_finish").await?;
    tx.commit().await?;
    if contract_writes_legacy_mirror(contract) {
        reconcile_deployment_rollouts_best_effort(pool, "finish_worker_tool").await;
    }
    Ok(worker)
}

pub async fn finish_worker_tool(
    pool: &sqlx::PgPool,
    fence: &RuntimeMemoryTxFence,
    tool_call_record_id: Uuid,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow> {
    worker_tool_transaction_retry_runner(
        "tool_finish",
        fence.worker_run_id,
        tool_call_record_id,
        || finish_worker_tool_once(pool, fence, tool_call_record_id),
        is_retryable_runtime_transaction_error,
    )
    .await
}

pub async fn finish_worker_attempt(
    pool: &sqlx::PgPool,
    input: &FinishWorkerAttemptRow,
) -> RuntimeMemoryStoreResult<FinishedWorkerAttemptRow> {
    if input.next_status == stage_worker_runs::StageWorkerRunStatus::Passed
        || input.next_unit_status == stage_run_units::StageRunUnitStatus::Passed
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "unit_pass_requires_final_seal",
        });
    }
    let statuses_align = matches!(
        (input.next_status, input.next_unit_status),
        (
            stage_worker_runs::StageWorkerRunStatus::GateBlocked,
            stage_run_units::StageRunUnitStatus::GateBlocked
        ) | (
            stage_worker_runs::StageWorkerRunStatus::Exhausted,
            stage_run_units::StageRunUnitStatus::Exhausted
        ) | (
            stage_worker_runs::StageWorkerRunStatus::Superseded,
            stage_run_units::StageRunUnitStatus::Superseded
        )
    );
    if !statuses_align {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "worker_unit_gate_outcome_mismatch",
        });
    }
    let mut tx = pool.begin().await?;
    let (operation, contract, locked_unit) =
        lock_operation_and_unit_for_fence(&mut tx, &input.fence).await?;
    let stage_team_plan_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM stage_team_plans WHERE stage_run_unit_id=$1 FOR SHARE",
    )
    .bind(input.fence.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?;
    if stage_team_plan_id.is_some() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_worker_requires_team_lifecycle",
        });
    }
    if locked_unit.status != input.expected_unit_status.as_str()
        || locked_unit.row_version != input.expected_unit_row_version
    {
        return Err(RuntimeMemoryStoreError::StaleVersion {
            entity: stage_run_units::TABLE_NAME,
            expected: input.expected_unit_row_version,
            actual: locked_unit.row_version,
        });
    }
    let worker = stage_worker_runs::finish_attempt_cas(
        &mut *tx,
        &input.fence,
        input.expected_status,
        input.next_status,
        &input.checkpoint,
        input.evidence_watermark,
    )
    .await?;
    let unit = stage_run_units::transition_cas(
        &mut *tx,
        input.fence.stage_run_unit_id,
        input.fence.operation_id,
        input.fence.stage_execution_id,
        locked_unit.organization_id,
        input.expected_unit_status,
        input.expected_unit_row_version,
        input.next_unit_status,
        None,
    )
    .await?;
    mirror_worker_if_required(
        &mut tx,
        &operation,
        contract,
        &unit,
        &worker,
        "attempt_finish",
    )
    .await?;
    tx.commit().await?;
    if contract_writes_legacy_mirror(contract) {
        reconcile_deployment_rollouts_best_effort(pool, "finish_worker_attempt").await;
    }
    Ok(FinishedWorkerAttemptRow { unit, worker })
}

/// Release a successfully checkpointed worker for a bounded continuation while
/// keeping its Unit running. This is the wave/pagination seam: it must not use
/// `gate_blocked`, because no Gate failure occurred and no handoff is published.
pub async fn pause_worker_for_continuation(
    pool: &sqlx::PgPool,
    input: &PauseWorkerForContinuationRow,
) -> RuntimeMemoryStoreResult<FinishedWorkerAttemptRow> {
    let mut tx = pool.begin().await?;
    let (operation, contract, unit) =
        lock_operation_and_unit_for_fence(&mut tx, &input.fence).await?;
    if unit.status != stage_run_units::StageRunUnitStatus::Running.as_str()
        || unit.row_version != input.expected_unit_row_version
    {
        return Err(RuntimeMemoryStoreError::StaleVersion {
            entity: stage_run_units::TABLE_NAME,
            expected: input.expected_unit_row_version,
            actual: unit.row_version,
        });
    }
    let worker = stage_worker_runs::finish_attempt_cas(
        &mut *tx,
        &input.fence,
        stage_worker_runs::StageWorkerRunStatus::Running,
        stage_worker_runs::StageWorkerRunStatus::WaitingBackground,
        &input.checkpoint,
        None,
    )
    .await?;
    mirror_worker_if_required(
        &mut tx,
        &operation,
        contract,
        &unit,
        &worker,
        "continuation_pause",
    )
    .await?;
    tx.commit().await?;
    if contract_writes_legacy_mirror(contract) {
        reconcile_deployment_rollouts_best_effort(pool, "pause_worker_for_continuation").await;
    }
    Ok(FinishedWorkerAttemptRow { unit, worker })
}

async fn complete_wave_and_create_next_without_completion(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    unit: &StageRunUnitRow,
    wave_id: Uuid,
    limit: i64,
) -> RuntimeMemoryStoreResult<Option<stage_asset_waves::StageAssetWaveWithItems>> {
    if limit <= 0 {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_stage_asset_wave_limit",
        });
    }
    sqlx::query("LOCK TABLE stage_asset_waves IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut **tx)
        .await?;
    sqlx::query("LOCK TABLE targets IN SHARE MODE")
        .execute(&mut **tx)
        .await?;
    let wave = sqlx::query_as::<_, stage_asset_waves::StageAssetWaveRow>(
        r#"SELECT * FROM stage_asset_waves
            WHERE id=$1 FOR UPDATE"#,
    )
    .bind(wave_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_asset_waves",
    })?;
    if wave.operation_id != unit.operation_id
        || wave.organization_id != unit.organization_id
        || wave.stage_kind != unit.stage_kind
        || wave.status != "running"
        || wave.completed_at.is_some()
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_asset_wave_close_identity_mismatch",
        });
    }
    let updated = sqlx::query(
        r#"UPDATE stage_asset_waves
              SET status='completed', completed_at=NOW(), updated_at=NOW()
            WHERE id=$1 AND status='running' AND completed_at IS NULL"#,
    )
    .bind(wave_id)
    .execute(&mut **tx)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_asset_wave_close_cas_failed",
        });
    }

    let candidates = sqlx::query_as::<_, stage_asset_waves::WaveTargetCandidate>(
        r#"SELECT t.id AS target_id, t.value AS asset_value,
                  t.target_type::text AS asset_type, t.source
             FROM targets t
            WHERE t.scope::text='in' AND t.organization_id=$2
              AND NOT EXISTS (
                    SELECT 1
                      FROM stage_asset_wave_items i
                      JOIN stage_asset_waves w ON w.id=i.wave_id
                     WHERE w.operation_id=$1 AND w.organization_id=$2
                       AND w.stage_kind=$3 AND i.target_id=t.id
                  )
            ORDER BY t.created_at ASC, t.value ASC, t.id ASC
            LIMIT $4"#,
    )
    .bind(unit.operation_id)
    .bind(unit.organization_id)
    .bind(&unit.stage_kind)
    .bind(limit)
    .fetch_all(&mut **tx)
    .await?;
    if candidates.is_empty() {
        return Ok(None);
    }
    let wave_index = sqlx::query_scalar::<_, i32>(
        r#"SELECT COALESCE(MAX(wave_index), -1) + 1
             FROM stage_asset_waves
            WHERE operation_id=$1 AND organization_id=$2 AND stage_kind=$3"#,
    )
    .bind(unit.operation_id)
    .bind(unit.organization_id)
    .bind(&unit.stage_kind)
    .fetch_one(&mut **tx)
    .await?;
    let asset_hash = stage_asset_waves::stable_asset_hash(&candidates);
    let next_wave = sqlx::query_as::<_, stage_asset_waves::StageAssetWaveRow>(
        r#"INSERT INTO stage_asset_waves
               (operation_id, organization_id, stage_kind, wave_index, status,
                started_at, parent_wave_id, asset_hash, updated_at)
           VALUES ($1,$2,$3,$4,'running',NOW(),$5,$6,NOW())
           RETURNING *"#,
    )
    .bind(unit.operation_id)
    .bind(unit.organization_id)
    .bind(&unit.stage_kind)
    .bind(wave_index)
    .bind(wave_id)
    .bind(asset_hash)
    .fetch_one(&mut **tx)
    .await?;
    let mut items = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let item = sqlx::query_as::<_, stage_asset_waves::StageAssetWaveItemRow>(
            r#"INSERT INTO stage_asset_wave_items
                   (wave_id, target_id, asset_value, asset_type, source)
               VALUES ($1,$2,$3,$4,$5)
               RETURNING *"#,
        )
        .bind(next_wave.id)
        .bind(candidate.target_id)
        .bind(candidate.asset_value)
        .bind(candidate.asset_type)
        .bind(candidate.source)
        .fetch_one(&mut **tx)
        .await?;
        items.push(item);
    }
    Ok(Some(stage_asset_waves::StageAssetWaveWithItems {
        wave: next_wave,
        items,
    }))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_final_gate_decision(input: &FinalizeUnitPassRow) -> RuntimeMemoryStoreResult<()> {
    let decision = input
        .gate_decision
        .as_object()
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "invalid_final_gate_decision",
        })?;
    let exact = decision.get("outcome").and_then(serde_json::Value::as_str) == Some("pass")
        && decision
            .get("operation_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value == input.fence.operation_id.to_string())
        && decision
            .get("stage_execution_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value == input.fence.stage_execution_id.to_string())
        && decision
            .get("stage_run_unit_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value == input.fence.stage_run_unit_id.to_string())
        && decision
            .get("deliverable_submission_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value == input.deliverable_submission_id.to_string())
        && decision
            .get("scope_hash")
            .and_then(serde_json::Value::as_str)
            == Some(input.scope_hash.as_str());
    let seal_material = serde_json::json!({
        "canonical_fact_keys": input.canonical_fact_keys,
        "typed_claims": input.typed_claims,
        "coverage_watermark": input.coverage_watermark,
        "evidence_ids": input.evidence_ids,
        "terminal_checkpoint": input.terminal_checkpoint,
        "deterministic_gate_details": decision
            .get("details")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
        "candidate_acceptance": input.candidate_acceptance,
    });
    let material_hash = operation_scope_decisions::sha256_json(&seal_material);
    let material_exact = decision
        .get("seal_material_sha256")
        .and_then(serde_json::Value::as_str)
        == Some(material_hash.as_str());
    if !exact || !material_exact {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: if exact {
                "final_gate_decision_material_mismatch"
            } else {
                "final_gate_decision_identity_mismatch"
            },
        });
    }
    let actual_hash = operation_scope_decisions::sha256_json(&input.gate_decision);
    if !is_sha256(&input.gate_decision_hash) || actual_hash != input.gate_decision_hash {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "final_gate_decision_hash_mismatch",
        });
    }
    if input
        .aggregate_pass_token_hash
        .as_deref()
        .is_some_and(|hash| !is_sha256(hash))
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_aggregate_pass_token_hash",
        });
    }
    if input.expected_unit_status != stage_run_units::StageRunUnitStatus::Running
        || !input.coverage_watermark.is_object()
        || input.terminal_checkpoint.is_null()
        || input.typed_claims.len() > canonical_fact_refs::MAX_TYPED_CLAIMS
        || input.typed_claims.iter().any(|claim| !claim.is_object())
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_final_seal_payload",
        });
    }
    Ok(())
}

fn bind_candidate_acceptance(
    unit: &StageRunUnitRow,
    input: &FinalizeUnitPassRow,
) -> RuntimeMemoryStoreResult<Option<attack_candidates::AcceptCandidateBatch>> {
    match (&*unit.stage_kind, input.candidate_acceptance.clone()) {
        ("attack_candidate", Some(acceptance)) => {
            Ok(Some(attack_candidates::AcceptCandidateBatch {
                operation_id: input.fence.operation_id,
                scope_snapshot_id: unit.scope_snapshot_id,
                wave_run_id: acceptance.wave_run_id,
                wave_unit_id: acceptance.wave_unit_id,
                organization_id: unit.organization_id,
                decision_stage_execution_id: input.fence.stage_execution_id,
                decision_stage_run_unit_id: input.fence.stage_run_unit_id,
                decision_deliverable_submission_id: input.deliverable_submission_id,
                manifest_hash: acceptance.manifest_hash,
                expected_work_item_ids: acceptance.expected_work_item_ids,
                candidates: acceptance.candidates,
                no_candidate_decisions: acceptance.no_candidate_decisions,
            }))
        }
        ("attack_candidate", None) => Err(RuntimeMemoryStoreError::Conflict {
            code: "attack_candidate_final_seal_requires_manifest_acceptance",
        }),
        (_, Some(_)) => Err(RuntimeMemoryStoreError::Conflict {
            code: "candidate_acceptance_forbidden_for_non_candidate_stage",
        }),
        (_, None) => Ok(None),
    }
}

fn sorted_unique_uuids(values: &[Uuid]) -> Option<Vec<Uuid>> {
    let mut values = values.to_vec();
    values.sort_unstable();
    let original_len = values.len();
    values.dedup();
    (values.len() == original_len).then_some(values)
}

fn validate_stage_specific_final_material(
    unit: &StageRunUnitRow,
    input: &FinalizeUnitPassRow,
    candidate_acceptance: Option<&attack_candidates::AcceptCandidateBatch>,
) -> RuntimeMemoryStoreResult<()> {
    let outcome_sets = input
        .canonical_fact_keys
        .iter()
        .filter_map(|key| match key {
            CanonicalFactKey::TechniqueOutcomeSet {
                organization_id,
                run_id,
                stage,
                terminal_cell_count,
                outcome_set_sha256,
            } => Some((
                *organization_id,
                run_id.as_str(),
                stage.as_str(),
                *terminal_cell_count,
                outcome_set_sha256.as_str(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    let has_individual_outcome = input
        .canonical_fact_keys
        .iter()
        .any(|key| matches!(key, CanonicalFactKey::TechniqueOutcome { .. }));
    if unit.stage_kind == "vuln_triage" {
        let expected_run_id = unit.operation_id.to_string();
        let expected_count = input
            .coverage_watermark
            .get("terminal_cells")
            .and_then(serde_json::Value::as_u64)
            .and_then(|count| u32::try_from(count).ok());
        let expected_hash = input
            .coverage_watermark
            .get("canonical_outcome_set_sha256")
            .and_then(serde_json::Value::as_str);
        let exact_set = match (expected_count, expected_hash) {
            (Some(expected_count), Some(expected_hash)) => {
                outcome_sets.as_slice()
                    == [(
                        unit.organization_id,
                        expected_run_id.as_str(),
                        "vuln_triage",
                        expected_count,
                        expected_hash,
                    )]
            }
            _ => false,
        };
        let exact_watermark = input
            .coverage_watermark
            .get("canonical_outcome_mode")
            .and_then(serde_json::Value::as_str)
            == Some("technique_outcome_set_v1")
            && expected_count.is_some()
            && expected_hash.is_some()
            && input
                .coverage_watermark
                .get("canonical_outcome_cells")
                .and_then(serde_json::Value::as_u64)
                == expected_count.map(u64::from)
            && input
                .coverage_watermark
                .get("canonical_ref_total")
                .and_then(serde_json::Value::as_u64)
                == Some(input.canonical_fact_keys.len() as u64)
            && input
                .coverage_watermark
                .get("canonical_ref_included")
                .and_then(serde_json::Value::as_u64)
                == Some(input.canonical_fact_keys.len() as u64)
            && input
                .coverage_watermark
                .get("canonical_ref_truncated")
                .and_then(serde_json::Value::as_bool)
                == Some(false);
        if !exact_set || !exact_watermark || has_individual_outcome {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "vuln_final_material_outcome_set_mismatch",
            });
        }
    } else if !outcome_sets.is_empty() {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "technique_outcome_set_forbidden_for_stage",
        });
    }
    let has_candidate_work_item_key = input
        .canonical_fact_keys
        .iter()
        .any(|key| matches!(key, CanonicalFactKey::AttackCandidateWorkItem { .. }));
    let Some(acceptance) = candidate_acceptance else {
        if has_candidate_work_item_key {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "candidate_work_item_ref_forbidden_for_non_candidate_stage",
            });
        }
        return Ok(());
    };

    let expected_work_item_ids = sorted_unique_uuids(&acceptance.expected_work_item_ids)
        .filter(|ids| !ids.is_empty())
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "candidate_final_material_manifest_invalid",
        })?;
    let mut keyed_work_item_ids = input
        .canonical_fact_keys
        .iter()
        .map(|key| match key {
            CanonicalFactKey::AttackCandidateWorkItem { work_item_id } => Ok(*work_item_id),
            _ => Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "candidate_final_material_canonical_ref_invalid",
            }),
        })
        .collect::<RuntimeMemoryStoreResult<Vec<_>>>()?;
    keyed_work_item_ids.sort_unstable();
    if keyed_work_item_ids != expected_work_item_ids {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "candidate_final_material_manifest_mismatch",
        });
    }

    let mut candidate_ids = acceptance
        .candidates
        .iter()
        .map(|decision| decision.candidate_id)
        .collect::<Vec<_>>();
    candidate_ids.sort_unstable();
    let mut no_candidate_work_item_ids = acceptance
        .no_candidate_decisions
        .iter()
        .map(|decision| decision.work_item_id)
        .collect::<Vec<_>>();
    no_candidate_work_item_ids.sort_unstable();
    let mut decision_evidence_ids = acceptance
        .candidates
        .iter()
        .flat_map(|decision| decision.evidence_ids.iter().copied())
        .chain(
            acceptance
                .no_candidate_decisions
                .iter()
                .flat_map(|decision| decision.evidence_ids.iter().copied()),
        )
        .collect::<Vec<_>>();
    decision_evidence_ids.sort_unstable();
    decision_evidence_ids.dedup();
    let mut supplied_evidence_ids = input.evidence_ids.clone();
    supplied_evidence_ids.sort_unstable();
    supplied_evidence_ids.dedup();
    if supplied_evidence_ids != decision_evidence_ids {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "candidate_final_material_evidence_mismatch",
        });
    }

    let expected_watermark = serde_json::json!({
        "kind": "candidate_manifest_v1",
        "stage": "attack_candidate",
        "organization_id": unit.organization_id,
        "wave_run_id": acceptance.wave_run_id,
        "wave_unit_id": acceptance.wave_unit_id,
        "manifest_hash": acceptance.manifest_hash,
        "expected_work_item_ids": expected_work_item_ids,
        "candidate_ids": candidate_ids,
        "no_candidate_work_item_ids": no_candidate_work_item_ids,
        "decision_evidence_ids": decision_evidence_ids,
        "terminal_count": acceptance.candidates.len()
            + acceptance.no_candidate_decisions.len(),
        "canonical_ref_total": input.canonical_fact_keys.len(),
        "canonical_ref_included": input.canonical_fact_keys.len(),
        "canonical_ref_truncated": false,
        "typed_claim_total": input.typed_claims.len(),
        "typed_claim_included": input.typed_claims.len(),
        "typed_claim_truncated": false,
        "evidence_id_total": input.evidence_ids.len(),
        "evidence_id_included": input.evidence_ids.len(),
        "evidence_id_truncated": false,
    });
    if input.coverage_watermark != expected_watermark {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "candidate_final_material_watermark_mismatch",
        });
    }

    let mut expected_claims = acceptance
        .candidates
        .iter()
        .map(|decision| {
            serde_json::json!({
                "kind": "attack_candidate_decision",
                "payload": {
                    "candidate_id": decision.candidate_id,
                    "work_item_id": decision.work_item_id,
                    "hypothesis": decision.hypothesis,
                    "technique": decision.technique,
                    "rationale": decision.rationale,
                    "candidate_plan_hash": decision.candidate_plan_hash,
                    "risk_class": decision.risk_class,
                    "evidence_ids": decision.evidence_ids,
                }
            })
        })
        .chain(acceptance.no_candidate_decisions.iter().map(|decision| {
            serde_json::json!({
                "kind": "attack_no_candidate_decision",
                "payload": {
                    "work_item_id": decision.work_item_id,
                    "reason_code": decision.reason_code,
                    "detail": decision.detail,
                    "evidence_ids": decision.evidence_ids,
                }
            })
        }))
        .collect::<Vec<_>>();
    let sort_claims = |claims: &mut Vec<serde_json::Value>| {
        claims.sort_by_key(operation_scope_decisions::canonical_json);
    };
    sort_claims(&mut expected_claims);
    let mut supplied_claims = input.typed_claims.clone();
    sort_claims(&mut supplied_claims);
    if supplied_claims != expected_claims {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "candidate_final_material_typed_claim_mismatch",
        });
    }
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
struct FinalSealEvidenceRow {
    id: i64,
    target_id: Option<Uuid>,
    project_path: String,
    detail: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
    run_id: Option<Uuid>,
}

pub(super) async fn validate_final_seal_evidence(
    connection: &mut sqlx::PgConnection,
    evidence_ids: &[i64],
    operation_id: Uuid,
    organization_id: Uuid,
    project_path_at_freeze: &str,
    freshness_floor: chrono::DateTime<chrono::Utc>,
    allowed_inherited_evidence_ids: &std::collections::BTreeSet<i64>,
) -> RuntimeMemoryStoreResult<()> {
    if evidence_ids.len() > canonical_fact_refs::MAX_EVIDENCE_IDS
        || evidence_ids.iter().any(|id| *id <= 0)
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_final_seal_evidence_ids",
        });
    }
    if evidence_ids.is_empty() {
        return Ok(());
    }
    let rows = sqlx::query_as::<_, FinalSealEvidenceRow>(
        r#"SELECT id, target_id, project_path, detail, created_at, run_id
             FROM audit_log
            WHERE id=ANY($1) AND audit_role='evidence'
            FOR SHARE"#,
    )
    .bind(evidence_ids)
    .fetch_all(&mut *connection)
    .await?;
    if rows.len() != evidence_ids.len() {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "final_seal_evidence_unknown_or_duplicate",
        });
    }
    let returned_ids = rows
        .iter()
        .map(|row| row.id)
        .collect::<std::collections::BTreeSet<_>>();
    if returned_ids.len() != evidence_ids.len()
        || evidence_ids.iter().any(|id| !returned_ids.contains(id))
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "final_seal_evidence_unknown_or_duplicate",
        });
    }
    for row in rows {
        if row.project_path != project_path_at_freeze
            || (row.created_at < freshness_floor
                && !allowed_inherited_evidence_ids.contains(&row.id))
            || row.run_id != Some(operation_id)
        {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "final_seal_evidence_stale_or_foreign",
            });
        }
        if let Some(target_id) = row.target_id {
            let owned = sqlx::query_scalar::<_, Uuid>(
                r#"SELECT id FROM targets
                    WHERE id=$1 AND organization_id=$2
                      AND project_path=$3 AND scope='in'
                    FOR SHARE"#,
            )
            .bind(target_id)
            .bind(organization_id)
            .bind(project_path_at_freeze)
            .fetch_optional(&mut *connection)
            .await?;
            if owned.is_none() {
                return Err(RuntimeMemoryStoreError::IdentityMismatch {
                    code: "final_seal_evidence_stale_or_foreign",
                });
            }
        } else if row
            .detail
            .get("organization_id")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|value| value != organization_id.to_string())
        {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "final_seal_evidence_stale_or_foreign",
            });
        }
    }
    Ok(())
}

fn map_canonical_fact_error(
    error: canonical_fact_refs::CanonicalFactRefError,
) -> RuntimeMemoryStoreError {
    match error {
        canonical_fact_refs::CanonicalFactRefError::Rejected { code } => {
            RuntimeMemoryStoreError::IdentityMismatch { code }
        }
        canonical_fact_refs::CanonicalFactRefError::Sqlx(error) => {
            RuntimeMemoryStoreError::Sqlx(error)
        }
    }
}

async fn load_candidate_inherited_evidence_ids(
    connection: &mut sqlx::PgConnection,
    unit: &StageRunUnitRow,
    acceptance: Option<&attack_candidates::AcceptCandidateBatch>,
) -> RuntimeMemoryStoreResult<std::collections::BTreeSet<i64>> {
    let Some(acceptance) = acceptance else {
        return Ok(std::collections::BTreeSet::new());
    };
    let ids = super::attack_candidate_work_items::load_frozen_entry_evidence_ids_with_connection(
        connection,
        unit.operation_id,
        unit.scope_snapshot_id,
        acceptance.wave_run_id,
        acceptance.wave_unit_id,
        unit.organization_id,
    )
    .await
    .map_err(|_| RuntimeMemoryStoreError::IdentityMismatch {
        code: "candidate_inherited_evidence_authority_mismatch",
    })?;
    Ok(ids.into_iter().collect())
}

async fn replay_existing_final_seal(
    connection: &mut sqlx::PgConnection,
    operation: &OperationStateRow,
    contract: runtime_memory_rollout::RuntimeMemoryContract,
    unit: &StageRunUnitRow,
    handoff: StageHandoffRow,
    input: &FinalizeUnitPassRow,
) -> RuntimeMemoryStoreResult<FinalizedUnitPassRow> {
    let checkpoint_version = input
        .fence
        .expected_checkpoint_version
        .checked_add(1)
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "final_seal_checkpoint_version_overflow",
        })?;
    let submission = sqlx::query_as::<_, StageDeliverableSubmissionRow>(
        r#"SELECT * FROM stage_deliverable_submissions
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_run_unit_id=$4 AND worker_run_id=$5
              AND organization_id=$6 AND stage_kind=$7
              AND attempt_epoch=$8 AND lease_token=$9
            FOR UPDATE"#,
    )
    .bind(input.deliverable_submission_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .bind(input.fence.worker_run_id)
    .bind(unit.organization_id)
    .bind(&unit.stage_kind)
    .bind(input.fence.attempt_epoch)
    .bind(input.fence.lease_token)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
        code: "final_seal_replay_submission_mismatch",
    })?;
    let worker = sqlx::query_as::<_, StageWorkerRunRow>(
        r#"SELECT * FROM stage_worker_runs
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_run_unit_id=$4 AND organization_id=$5
              AND attempt_epoch=$6 AND checkpoint_version=$7
              AND status='passed' AND lease_token IS NULL
              AND active_tool_call_id IS NULL AND checkpoint=$8
            FOR UPDATE"#,
    )
    .bind(input.fence.worker_run_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .bind(unit.organization_id)
    .bind(input.fence.attempt_epoch)
    .bind(checkpoint_version)
    .bind(&input.terminal_checkpoint)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
        code: "final_seal_replay_worker_mismatch",
    })?;
    let canonical_fact_refs = handoff
        .payload
        .get("canonical_fact_refs")
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<CanonicalFactRef>>(value).ok())
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "final_seal_replay_payload_invalid",
        })?;
    let persisted_keys = canonical_fact_refs
        .iter()
        .map(|canonical_ref| canonical_ref.key.clone())
        .collect::<Vec<_>>();
    let mut evidence_ids = input.evidence_ids.clone();
    evidence_ids.extend(
        canonical_fact_refs
            .iter()
            .flat_map(|canonical_ref| canonical_ref.evidence_ids.iter().copied()),
    );
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    let payload_matches = handoff
        .payload
        .get("schema_version")
        .and_then(Value::as_i64)
        == Some(1)
        && persisted_keys == input.canonical_fact_keys
        && handoff.payload.get("typed_claims") == Some(&Value::Array(input.typed_claims.clone()))
        && handoff.payload.get("coverage_watermark") == Some(&input.coverage_watermark)
        && handoff.payload.get("evidence_ids")
            == Some(&serde_json::to_value(&evidence_ids).expect("evidence ids serialize"))
        && operation_scope_decisions::sha256_json(&handoff.payload) == handoff.payload_sha256;
    let identity_matches = unit.status == stage_run_units::StageRunUnitStatus::Passed.as_str()
        && handoff.invalidated_at.is_none()
        && handoff.operation_id == input.fence.operation_id
        && handoff.organization_id == unit.organization_id
        && handoff.scope_snapshot_id == unit.scope_snapshot_id
        && handoff.from_stage_kind == unit.stage_kind
        && handoff.stage_execution_id == input.fence.stage_execution_id
        && handoff.source_stage_run_unit_id == input.fence.stage_run_unit_id
        && handoff.deliverable_submission_id == submission.id
        && handoff.scope_hash == input.scope_hash
        && handoff.unit_gate_decision_hash == input.gate_decision_hash
        && handoff.aggregate_pass_token_hash == input.aggregate_pass_token_hash
        && handoff.coverage_watermark == input.coverage_watermark
        && handoff.evidence_ids == evidence_ids
        && unit
            .pass_watermark
            .get("handoff_id")
            .and_then(Value::as_str)
            == Some(handoff.id.to_string().as_str());
    if !payload_matches || !identity_matches {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "final_seal_replay_mismatch",
        });
    }
    let completion = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, Option<String>)>(
        r#"SELECT passed_at, stage_run_id FROM org_stage_completions
            WHERE organization_id=$1 AND stage_kind=$2
            FOR SHARE"#,
    )
    .bind(unit.organization_id)
    .bind(&unit.stage_kind)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(RuntimeMemoryStoreError::Conflict {
        code: "final_seal_replay_completion_missing",
    })?;
    if completion.0 != handoff.gate_passed_at
        || completion.1.as_deref() != Some(input.fence.operation_id.to_string().as_str())
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "final_seal_replay_completion_mismatch",
        });
    }
    if contract_writes_legacy_mirror(contract) {
        let mirrored = &operation.state_blob["stage_run_handoffs"][&unit.stage_kind]
            [unit.organization_id.to_string()];
        if mirrored.get("handoff_id").and_then(Value::as_str)
            != Some(handoff.id.to_string().as_str())
            || operation.state_blob["stage_run_workers"][&unit.stage_kind]
                [unit.organization_id.to_string()]["status"]
                != "passed"
        {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "final_seal_replay_legacy_mirror_mismatch",
            });
        }
    }
    Ok(FinalizedUnitPassRow {
        unit: unit.clone(),
        worker,
        handoff,
        canonical_fact_refs,
        replayed: true,
    })
}

async fn validate_replayed_authoritative_material(
    connection: &mut sqlx::PgConnection,
    unit: &StageRunUnitRow,
    input: &FinalizeUnitPassRow,
    candidate_acceptance: Option<&attack_candidates::AcceptCandidateBatch>,
    observation_ceiling: chrono::DateTime<chrono::Utc>,
    persisted_refs: &[CanonicalFactRef],
) -> RuntimeMemoryStoreResult<()> {
    let freshness_floor = unit.started_at.ok_or(RuntimeMemoryStoreError::Conflict {
        code: "final_seal_replay_mismatch",
    })?;
    let scope = sqlx::query_as::<_, (String, String, Option<chrono::DateTime<chrono::Utc>>)>(
        r#"SELECT scope_hash,project_path_at_freeze,sealed_at
             FROM operation_org_scope_snapshots
            WHERE id=$1 AND operation_id=$2
            FOR SHARE"#,
    )
    .bind(unit.scope_snapshot_id)
    .bind(unit.operation_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(RuntimeMemoryStoreError::Conflict {
        code: "final_seal_replay_mismatch",
    })?;
    if scope.2.is_none() || scope.0 != input.scope_hash {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "final_seal_replay_mismatch",
        });
    }
    let reloaded_refs = canonical_fact_refs::resolve_for_final_seal(
        connection,
        unit.operation_id,
        unit.organization_id,
        &scope.1,
        freshness_floor,
        observation_ceiling,
        &input.canonical_fact_keys,
    )
    .await
    .map_err(|_| RuntimeMemoryStoreError::Conflict {
        code: "final_seal_replay_mismatch",
    })?;
    if reloaded_refs != persisted_refs {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "final_seal_replay_mismatch",
        });
    }
    let allowed_inherited_evidence_ids =
        load_candidate_inherited_evidence_ids(connection, unit, candidate_acceptance).await?;
    let mut evidence_ids = input.evidence_ids.clone();
    evidence_ids.extend(
        reloaded_refs
            .iter()
            .flat_map(|canonical_ref| canonical_ref.evidence_ids.iter().copied()),
    );
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    validate_final_seal_evidence(
        connection,
        &evidence_ids,
        unit.operation_id,
        unit.organization_id,
        &scope.1,
        freshness_floor,
        &allowed_inherited_evidence_ids,
    )
    .await
    .map_err(|_| RuntimeMemoryStoreError::Conflict {
        code: "final_seal_replay_mismatch",
    })
}

fn stage_publishes_memory_episode(stage_kind: &str) -> bool {
    MEMORY_EPISODE_STAGE_KINDS.contains(&stage_kind)
}

/// Publish the immutable StageEpisode source event inside the caller-owned
/// final-seal transaction. The outbox catalog creates projector deliveries;
/// Assertion promotion remains an asynchronous deterministic projector concern
/// and must never be split into an after-commit write from this producer.
async fn close_final_seal_memory_episode(
    connection: &mut sqlx::PgConnection,
    operation: &OperationStateRow,
    unit: &StageRunUnitRow,
    worker: &StageWorkerRunRow,
    handoff: &StageHandoffRow,
) -> RuntimeMemoryStoreResult<()> {
    if !stage_publishes_memory_episode(&unit.stage_kind) {
        return Ok(());
    }
    let project_scope_id =
        operation
            .project_scope_id
            .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                code: "memory_episode_project_scope_missing",
            })?;
    let started_at = unit.started_at.ok_or(RuntimeMemoryStoreError::Conflict {
        code: "memory_episode_unit_start_missing",
    })?;
    let source_version = worker
        .attempt_epoch
        .checked_add(1)
        .filter(|version| *version > 0)
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "memory_episode_source_version_invalid",
        })?;
    let episode_identity = format!(
        "stage_episode\0{}\0{}\0{}",
        unit.operation_id, unit.stage_execution_id, unit.id
    );
    let episode_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, episode_identity.as_bytes());
    let source_stream_key = format!(
        "stage_episode:{}:{}:{}",
        unit.operation_id, unit.stage_kind, unit.id
    );
    let source = SourceRef {
        source_kind: CanonicalSourceKind::StageEpisode,
        row_id: CanonicalRowId::Uuid(episode_id),
        source_stream_key: source_stream_key.clone(),
        version: source_version,
    };
    let episode = StageEpisode {
        episode_id,
        scope: OperationScope {
            project_scope_id: ProjectScopeId(project_scope_id),
            source_operation_id: unit.operation_id,
            organization_id_at_time: unit.organization_id,
            scope_snapshot_hash: handoff.scope_hash.clone(),
        },
        stage_execution_id: unit.stage_execution_id,
        stage_run_unit_id: Some(unit.id),
        worker_run_id: Some(worker.id),
        candidate_attempt_id: None,
        stage_kind: unit.stage_kind.clone(),
        wave: None,
        verdict: EpisodeVerdict::Passed,
        deliverable_submission_id: Some(handoff.deliverable_submission_id),
        handoff_id: Some(handoff.id),
        reason_codes: vec!["deterministic_gate_pass".to_string()],
        // The typed handoff id and evidence ids carry the canonical provenance.
        // The Memory source vocabulary intentionally has no generic model-owned
        // prose or untyped StageHandoff SourceRef variant.
        fact_refs: Vec::new(),
        evidence_ids: handoff.evidence_ids.clone(),
        started_at,
        ended_at: handoff.gate_passed_at,
    };
    let structured_payload =
        serde_json::to_value(&episode).map_err(|_| RuntimeMemoryStoreError::Conflict {
            code: "memory_episode_serialization_failed",
        })?;
    let event = KnowledgeEventEnvelopeV1 {
        event_id: Uuid::new_v5(
            &episode_id,
            KnowledgeEventNameV1::StageEpisodeClosed.as_str().as_bytes(),
        ),
        project_scope_id: Some(ProjectScopeId(project_scope_id)),
        organization_id_at_time: Some(unit.organization_id),
        source_operation_id: unit.operation_id,
        event_name: KnowledgeEventNameV1::StageEpisodeClosed,
        schema_version: 1,
        payload: KnowledgeEventPayloadV1 {
            source,
            source_stream_key,
            source_version,
            structured_payload,
        },
        occurred_at: handoff.gate_passed_at,
    };
    stage_episodes::close_episode_with_event_with_connection(connection, &episode, &event)
        .await
        .map_err(|error| RuntimeMemoryStoreError::Conflict { code: error.code() })?;
    Ok(())
}

async fn finalize_unit_pass_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    input: &FinalizeUnitPassRow,
    stage_team_authority: Option<Uuid>,
) -> RuntimeMemoryStoreResult<FinalizedUnitPassRow> {
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.fence.operation_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    let contract = frozen_runtime_contract(&operation)?;
    if !contract_writes_v2(contract) {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "runtime_v2_not_enabled",
        });
    }
    let persisted_team_plan_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM stage_team_plans WHERE stage_run_unit_id=$1 FOR SHARE",
    )
    .bind(input.fence.stage_run_unit_id)
    .fetch_optional(&mut **tx)
    .await?;
    match (persisted_team_plan_id, stage_team_authority) {
        (Some(persisted), Some(authorized)) if persisted == authorized => {}
        (Some(_), _) => {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_finalizer_required",
            });
        }
        (None, Some(_)) => {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_team_finalizer_authority_mismatch",
            });
        }
        (None, None) => {}
    }
    let attack_contract = frozen_attack_execution_contract(tx, input.fence.operation_id).await?;
    let locked_unit = load_runtime_unit_for_update(
        tx,
        input.fence.operation_id,
        input.fence.stage_execution_id,
        input.fence.stage_run_unit_id,
    )
    .await?;
    let existing_handoff = sqlx::query_as::<_, StageHandoffRow>(
        "SELECT * FROM stage_handoffs WHERE source_stage_run_unit_id=$1 FOR UPDATE",
    )
    .bind(input.fence.stage_run_unit_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(handoff) = existing_handoff {
        let replayed =
            replay_existing_final_seal(tx, &operation, contract, &locked_unit, handoff, input)
                .await?;
        // First compare the request with the already-persisted seal so any
        // response-loss drift has one stable replay error. Only an exact
        // persisted shape reaches Gate hash/stage-specific revalidation.
        validate_final_gate_decision(input)?;
        let candidate_acceptance = bind_candidate_acceptance(&locked_unit, input)?;
        validate_stage_specific_final_material(&locked_unit, input, candidate_acceptance.as_ref())?;
        validate_replayed_authoritative_material(
            tx,
            &locked_unit,
            input,
            candidate_acceptance.as_ref(),
            replayed.handoff.gate_passed_at,
            &replayed.canonical_fact_refs,
        )
        .await?;
        if let Some(command) = candidate_acceptance.as_ref() {
            attack_candidates::accept_gate_passed_candidate_batch_with_connection(
                tx,
                command.clone(),
            )
            .await?;
            attack_execution_shadow::persist_candidate_legacy_mirror(tx, &attack_contract, command)
                .await?;
        }
        close_final_seal_memory_episode(
            tx,
            &operation,
            &replayed.unit,
            &replayed.worker,
            &replayed.handoff,
        )
        .await?;
        return Ok(replayed);
    }
    validate_final_gate_decision(input)?;
    let candidate_acceptance = bind_candidate_acceptance(&locked_unit, input)?;
    validate_stage_specific_final_material(&locked_unit, input, candidate_acceptance.as_ref())?;
    validate_runtime_stage_execution(
        tx,
        &operation,
        input.fence.stage_execution_id,
        &locked_unit.stage_kind,
    )
    .await?;
    if locked_unit.status != input.expected_unit_status.as_str()
        || locked_unit.row_version != input.expected_unit_row_version
    {
        return Err(RuntimeMemoryStoreError::StaleVersion {
            entity: stage_run_units::TABLE_NAME,
            expected: input.expected_unit_row_version,
            actual: locked_unit.row_version,
        });
    }
    let freshness_floor = locked_unit
        .started_at
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "final_seal_unit_not_started",
        })?;
    let locked_worker = sqlx::query_as::<_, StageWorkerRunRow>(
        r#"SELECT * FROM stage_worker_runs
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_run_unit_id=$4 AND organization_id=$5
              AND lease_token=$6 AND attempt_epoch=$7
              AND checkpoint_version=$8 AND status='running'
              AND active_tool_call_id IS NULL AND lease_expires_at > NOW()
            FOR UPDATE"#,
    )
    .bind(input.fence.worker_run_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .bind(locked_unit.organization_id)
    .bind(input.fence.lease_token)
    .bind(input.fence.attempt_epoch)
    .bind(input.fence.expected_checkpoint_version)
    .fetch_optional(&mut **tx)
    .await?;
    if locked_worker.is_none() {
        return Err(RuntimeMemoryStoreError::LeaseLost {
            worker_run_id: input.fence.worker_run_id,
            attempt_epoch: input.fence.attempt_epoch,
        });
    }
    let scope = sqlx::query_as::<_, (String, String, Option<chrono::DateTime<chrono::Utc>>)>(
        r#"SELECT scope_hash, project_path_at_freeze, sealed_at
             FROM operation_org_scope_snapshots
            WHERE id=$1 AND operation_id=$2
            FOR SHARE"#,
    )
    .bind(locked_unit.scope_snapshot_id)
    .bind(input.fence.operation_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "operation_org_scope_snapshots",
    })?;
    if scope.2.is_none() || scope.0 != input.scope_hash {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "final_seal_scope_hash_mismatch",
        });
    }

    let submission = sqlx::query_as::<_, StageDeliverableSubmissionRow>(
        r#"SELECT * FROM stage_deliverable_submissions
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_run_unit_id=$4 AND worker_run_id=$5
              AND organization_id=$6 AND stage_kind=$7
              AND attempt_epoch=$8 AND lease_token=$9
            FOR UPDATE"#,
    )
    .bind(input.deliverable_submission_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .bind(input.fence.worker_run_id)
    .bind(locked_unit.organization_id)
    .bind(&locked_unit.stage_kind)
    .bind(input.fence.attempt_epoch)
    .bind(input.fence.lease_token)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
        code: "final_seal_submission_identity_mismatch",
    })?;

    let allowed_inherited_evidence_ids =
        load_candidate_inherited_evidence_ids(tx, &locked_unit, candidate_acceptance.as_ref())
            .await?;

    let seal_observation_ceiling =
        sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>("SELECT NOW()")
            .fetch_one(&mut **tx)
            .await?;
    let refs = canonical_fact_refs::resolve_for_final_seal(
        tx,
        input.fence.operation_id,
        locked_unit.organization_id,
        &scope.1,
        freshness_floor,
        seal_observation_ceiling,
        &input.canonical_fact_keys,
    )
    .await
    .map_err(map_canonical_fact_error)?;
    let mut evidence_ids = input.evidence_ids.clone();
    evidence_ids.extend(
        refs.iter()
            .flat_map(|canonical_ref| canonical_ref.evidence_ids.iter().copied()),
    );
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    validate_final_seal_evidence(
        tx,
        &evidence_ids,
        input.fence.operation_id,
        locked_unit.organization_id,
        &scope.1,
        freshness_floor,
        &allowed_inherited_evidence_ids,
    )
    .await?;

    let payload = serde_json::json!({
        "schema_version": 1,
        "canonical_fact_refs": refs.clone(),
        "typed_claims": input.typed_claims.clone(),
        "coverage_watermark": input.coverage_watermark.clone(),
        "evidence_ids": evidence_ids.clone(),
    });
    let canonical_payload = operation_scope_decisions::canonical_json(&payload);
    if canonical_payload.len() > canonical_fact_refs::MAX_CANONICAL_PAYLOAD_BYTES {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_handoff_payload_too_large",
        });
    }
    let payload_sha256 = operation_scope_decisions::sha256_json(&payload);
    let evidence_watermark = evidence_ids.iter().copied().max();
    let worker = stage_worker_runs::finish_passed_for_final_seal(
        &mut **tx,
        &input.fence,
        &input.terminal_checkpoint,
        evidence_watermark,
    )
    .await?;
    let handoff_id = Uuid::new_v4();
    let pass_watermark = serde_json::json!({
        "handoff_id": handoff_id,
        "deliverable_submission_id": submission.id,
        "scope_hash": scope.0.clone(),
        "coverage_watermark": input.coverage_watermark.clone(),
        "gate_decision_hash": input.gate_decision_hash.clone(),
        "evidence_watermark": evidence_watermark,
    });
    let unit = stage_run_units::transition_to_passed_for_final_seal(
        &mut **tx,
        input.fence.stage_run_unit_id,
        input.fence.operation_id,
        input.fence.stage_execution_id,
        locked_unit.organization_id,
        input.expected_unit_status,
        input.expected_unit_row_version,
        &pass_watermark,
    )
    .await?;
    let handoff = stage_handoffs::insert_with_executor(
        &mut **tx,
        &stage_handoffs::NewStageHandoffRow {
            id: handoff_id,
            operation_id: input.fence.operation_id,
            organization_id: locked_unit.organization_id,
            scope_snapshot_id: locked_unit.scope_snapshot_id,
            from_stage_kind: locked_unit.stage_kind.clone(),
            stage_execution_id: input.fence.stage_execution_id,
            source_stage_run_unit_id: input.fence.stage_run_unit_id,
            deliverable_submission_id: submission.id,
            scope_hash: scope.0.clone(),
            payload,
            payload_sha256,
            evidence_ids,
            coverage_watermark: input.coverage_watermark.clone(),
            unit_gate_decision_hash: input.gate_decision_hash.clone(),
            aggregate_pass_token_hash: input.aggregate_pass_token_hash.clone(),
            schema_version: 1,
        },
    )
    .await?;
    let mut next_state_blob = operation.state_blob.clone();
    let mut state_blob_changed = false;
    if let Some(command) = candidate_acceptance.as_ref() {
        attack_candidates::accept_gate_passed_candidate_batch_with_connection(tx, command.clone())
            .await?;
        attack_execution_shadow::persist_candidate_legacy_mirror(tx, &attack_contract, command)
            .await?;
    }
    sqlx::query(
        r#"INSERT INTO org_stage_completions
               (organization_id, stage_kind, passed_at, stage_run_id, updated_at)
           VALUES ($1,$2,$3,$4,NOW())
           ON CONFLICT (organization_id, stage_kind)
           DO UPDATE SET passed_at=EXCLUDED.passed_at,
                         stage_run_id=EXCLUDED.stage_run_id,
                         updated_at=NOW()"#,
    )
    .bind(locked_unit.organization_id)
    .bind(&locked_unit.stage_kind)
    .bind(handoff.gate_passed_at)
    .bind(input.fence.operation_id.to_string())
    .execute(&mut **tx)
    .await?;
    if contract_writes_legacy_mirror(contract) {
        let organization_name = frozen_organization_name(tx, &unit).await?;
        apply_legacy_final_seal_mirror(
            &mut next_state_blob,
            &unit,
            &worker,
            &handoff,
            &organization_name,
        );
        state_blob_changed = true;
    }
    if state_blob_changed {
        write_locked_legacy_state_blob(
            tx,
            input.fence.operation_id,
            &operation.runtime_memory_contract,
            &next_state_blob,
        )
        .await?;
        runtime_memory_shadow::persist_worker_sample(tx, worker.id, "final_seal").await?;
    }
    close_final_seal_memory_episode(tx, &operation, &unit, &worker, &handoff).await?;
    Ok(FinalizedUnitPassRow {
        unit,
        worker,
        handoff,
        canonical_fact_refs: refs,
        replayed: false,
    })
}

/// Application bridge seam for composing Candidate whole-record shadow
/// selection with the final seal under one transaction. Domain callers still
/// use [`finalize_unit_pass`]; only the concrete DB bridge owns this executor.
pub async fn finalize_unit_pass_with_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    input: &FinalizeUnitPassRow,
) -> RuntimeMemoryStoreResult<FinalizedUnitPassRow> {
    finalize_unit_pass_in_transaction(tx, input, None).await
}

/// The only compound runtime-memory path that may turn a post-Scoping Unit and
/// Worker into PASS and publish evidence for downstream stages.
pub async fn finalize_unit_pass(
    pool: &sqlx::PgPool,
    input: &FinalizeUnitPassRow,
) -> RuntimeMemoryStoreResult<FinalizedUnitPassRow> {
    let mut tx = pool.begin().await?;
    let finalized = finalize_unit_pass_with_transaction(&mut tx, input).await?;
    tx.commit().await?;
    reconcile_deployment_rollouts_best_effort(pool, "finalize_unit_pass").await;
    Ok(finalized)
}

/// Persist a deterministic Aggregator Gate BLOCK and, while frozen repair
/// fuel remains, atomically advance to a new request epoch containing one
/// bounded repair WorkItem and one fresh Aggregator WorkItem.  The source
/// epoch, submission, gap, Aggregator Worker and lease/checkpoint fence are
/// retained immutably; the old Worker is never made runnable again.
pub async fn open_stage_team_repair(
    pool: &sqlx::PgPool,
    input: &OpenStageTeamRepairRow,
) -> RuntimeMemoryStoreResult<OpenedStageTeamRepairRow> {
    let expected_gap_hash = format!(
        "sha256:{}",
        operation_scope_decisions::sha256_json(&input.gap_manifest)
    );
    if input.request_id.trim().is_empty()
        || input.request_id.len() > 256
        || !input.gap_manifest.is_object()
        || input.gap_manifest_hash != expected_gap_hash
        || input
            .gap_manifest
            .get("gate_decision_hash")
            .and_then(Value::as_str)
            != Some(input.gate_decision_hash.as_str())
        || input.expected_manifest_hash.len() != 71
        || !input.expected_manifest_hash.starts_with("sha256:")
        || input.gate_decision_hash.len() != 71
        || !input.gate_decision_hash.starts_with("sha256:")
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_team_repair_gap",
        });
    }
    let mut tx = pool.begin().await?;
    let gap_sql = format!(
        "SELECT {} FROM stage_team_unit_gaps WHERE request_id=$1 FOR UPDATE",
        stage_teams::UNIT_GAP_COLUMNS
    );
    if let Some(gap) = sqlx::query_as::<_, stage_teams::StageTeamUnitGapRow>(&gap_sql)
        .bind(input.request_id.trim())
        .fetch_optional(&mut *tx)
        .await?
    {
        if gap.team_plan_id != input.stage_team_plan_id
            || gap.operation_id != input.fence.operation_id
            || gap.stage_execution_id != input.fence.stage_execution_id
            || gap.stage_run_unit_id != input.fence.stage_run_unit_id
            || gap.source_dispatch_epoch != input.expected_dispatch_epoch
            || gap.source_manifest_hash != input.expected_manifest_hash
            || gap.source_attempt_epoch != input.fence.attempt_epoch
            || gap.source_checkpoint_version != input.fence.expected_checkpoint_version
            || gap.source_lease_token != input.fence.lease_token
            || gap.source_aggregator_work_item_id != input.aggregator_work_item_id
            || gap.source_aggregator_worker_run_id != input.fence.worker_run_id
            || gap.deliverable_submission_id != input.deliverable_submission_id
            || gap.gate_decision_hash != input.gate_decision_hash
            || gap.gap_manifest_hash != input.gap_manifest_hash
            || gap.gap_manifest != input.gap_manifest
        {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_repair_replay_mismatch",
            });
        }
        let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
            "SELECT * FROM stage_team_plans WHERE id=$1",
        )
        .bind(input.stage_team_plan_id)
        .fetch_one(&mut *tx)
        .await?;
        if plan.scope_snapshot_id != gap.scope_snapshot_id
            || plan.operation_id != gap.operation_id
            || plan.stage_execution_id != gap.stage_execution_id
            || plan.stage_run_unit_id != gap.stage_run_unit_id
            || plan.organization_id != gap.organization_id
        {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_repair_replay_owner_mismatch",
            });
        }
        let unit = load_runtime_unit_for_update(
            &mut tx,
            input.fence.operation_id,
            input.fence.stage_execution_id,
            input.fence.stage_run_unit_id,
        )
        .await?;
        let aggregator_worker =
            sqlx::query_as::<_, StageWorkerRunRow>("SELECT * FROM stage_worker_runs WHERE id=$1")
                .bind(input.fence.worker_run_id)
                .fetch_one(&mut *tx)
                .await?;
        let generation_sql = format!(
            "SELECT {} FROM stage_team_repair_generations WHERE source_gap_id=$1",
            stage_teams::REPAIR_GENERATION_COLUMNS
        );
        let generation =
            sqlx::query_as::<_, stage_teams::StageTeamRepairGenerationRow>(&generation_sql)
                .bind(gap.id)
                .fetch_optional(&mut *tx)
                .await?;
        let (repair_work_item, aggregator_work_item) = if let Some(generation) = &generation {
            let repair_id =
                generation
                    .repair_work_item_id
                    .ok_or(RuntimeMemoryStoreError::Conflict {
                        code: "stage_team_repair_generation_not_sealed",
                    })?;
            let aggregator_id =
                generation
                    .aggregator_work_item_id
                    .ok_or(RuntimeMemoryStoreError::Conflict {
                        code: "stage_team_repair_generation_not_sealed",
                    })?;
            (
                Some(
                    sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
                        "SELECT * FROM stage_work_items WHERE id=$1",
                    )
                    .bind(repair_id)
                    .fetch_one(&mut *tx)
                    .await?,
                ),
                Some(
                    sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
                        "SELECT * FROM stage_work_items WHERE id=$1",
                    )
                    .bind(aggregator_id)
                    .fetch_one(&mut *tx)
                    .await?,
                ),
            )
        } else {
            (None, None)
        };
        tx.commit().await?;
        return Ok(OpenedStageTeamRepairRow {
            plan,
            unit,
            gap,
            generation,
            repair_work_item,
            aggregator_work_item,
            aggregator_worker,
            replayed: true,
        });
    }

    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.fence.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    if frozen_runtime_contract(&operation)? != runtime_memory_rollout::RuntimeMemoryContract::V2Only
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_requires_v2_only",
        });
    }
    let unit = load_runtime_unit_for_update(
        &mut tx,
        input.fence.operation_id,
        input.fence.stage_execution_id,
        input.fence.stage_run_unit_id,
    )
    .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "SELECT * FROM stage_team_plans
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
            AND stage_run_unit_id=$4 FOR UPDATE",
    )
    .bind(input.stage_team_plan_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_team_plans",
    })?;
    let aggregator_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "SELECT * FROM stage_work_items WHERE id=$1 AND team_plan_id=$2 FOR UPDATE",
    )
    .bind(input.aggregator_work_item_id)
    .bind(plan.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_work_items",
    })?;
    let aggregator_worker = sqlx::query_as::<_, StageWorkerRunRow>(
        "SELECT * FROM stage_worker_runs WHERE id=$1 FOR UPDATE",
    )
    .bind(input.fence.worker_run_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_worker_runs",
    })?;
    if unit.status != "running"
        || plan.dispatch_epoch != input.expected_dispatch_epoch
        || plan.requests_closed_at.is_none()
        || plan.final_submitter_worker_run_id != Some(input.fence.worker_run_id)
        || plan.aggregator_role.as_deref() != Some(aggregator_item.role.as_str())
        || aggregator_item.required_for_barrier
        || aggregator_item.status != "running"
        || aggregator_worker.work_item_id != Some(aggregator_item.id)
        || aggregator_worker.status != "running"
        || aggregator_worker.lease_token != Some(input.fence.lease_token)
        || aggregator_worker.attempt_epoch != input.fence.attempt_epoch
        || aggregator_worker.checkpoint_version != input.fence.expected_checkpoint_version
        || aggregator_worker.active_tool_call_id.is_some()
        || aggregator_worker
            .lease_expires_at
            .is_none_or(|expires_at| expires_at <= chrono::Utc::now())
    {
        return Err(RuntimeMemoryStoreError::LeaseLost {
            worker_run_id: input.fence.worker_run_id,
            attempt_epoch: input.fence.attempt_epoch,
        });
    }
    let submission_is_exact = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
               SELECT 1 FROM stage_deliverable_submissions
                WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
                  AND stage_run_unit_id=$4 AND organization_id=$5 AND worker_run_id=$6
           )"#,
    )
    .bind(input.deliverable_submission_id)
    .bind(plan.operation_id)
    .bind(plan.stage_execution_id)
    .bind(plan.stage_run_unit_id)
    .bind(plan.organization_id)
    .bind(aggregator_worker.id)
    .fetch_one(&mut *tx)
    .await?;
    if !submission_is_exact {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_repair_submission_mismatch",
        });
    }
    let barrier = stage_teams::load_barrier_with_connection_ignoring_worker(
        &mut tx,
        plan.id,
        Some(aggregator_worker.id),
    )
    .await?;
    if !barrier.ready_to_finalize() || barrier.manifest_hash != input.expected_manifest_hash {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_sibling_barrier_not_ready",
        });
    }
    let prior_generations: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stage_team_repair_generations WHERE team_plan_id=$1",
    )
    .bind(plan.id)
    .fetch_one(&mut *tx)
    .await?;
    let max_repair_generations = plan
        .dynamic_request_policy
        .get("max_repair_generations")
        .and_then(Value::as_i64)
        .unwrap_or(1)
        .clamp(0, 3);
    let fuel_available = prior_generations < max_repair_generations;
    let repair_generation = i32::try_from(prior_generations.saturating_add(1)).map_err(|_| {
        RuntimeMemoryStoreError::Conflict {
            code: "stage_team_repair_generation_overflow",
        }
    })?;
    let gate_checkpoint = serde_json::json!({
        "stage_team_gate_block": {
            "gate_decision_hash": input.gate_decision_hash,
            "gap_manifest_hash": input.gap_manifest_hash,
            "repair_generation": repair_generation,
            "schema_version": 1,
        }
    });
    let aggregator_worker = stage_worker_runs::finish_attempt_cas(
        &mut *tx,
        &input.fence,
        stage_worker_runs::StageWorkerRunStatus::Running,
        stage_worker_runs::StageWorkerRunStatus::GateBlocked,
        &gate_checkpoint,
        aggregator_worker.evidence_watermark,
    )
    .await?;
    let aggregator_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "UPDATE stage_work_items
            SET status='superseded',terminal_at=NOW(),row_version=row_version+1,updated_at=NOW()
          WHERE id=$1 AND team_plan_id=$2 AND status='running' AND row_version=$3
          RETURNING *",
    )
    .bind(aggregator_item.id)
    .bind(plan.id)
    .bind(aggregator_item.row_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::StaleVersion {
        entity: "stage_work_items",
        expected: aggregator_item.row_version,
        actual: -1,
    })?;
    let gap_id = Uuid::new_v5(&plan.id, input.request_id.trim().as_bytes());
    let gap_sql = format!(
        r#"INSERT INTO stage_team_unit_gaps(
               id,request_id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,source_dispatch_epoch,source_manifest_hash,source_attempt_epoch,
               source_checkpoint_version,source_lease_token,source_aggregator_work_item_id,
               source_aggregator_worker_run_id,deliverable_submission_id,gate_decision_hash,
               gap_manifest,gap_manifest_hash,repair_generation,disposition
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)
           RETURNING {}"#,
        stage_teams::UNIT_GAP_COLUMNS
    );
    let gap = sqlx::query_as::<_, stage_teams::StageTeamUnitGapRow>(&gap_sql)
        .bind(gap_id)
        .bind(input.request_id.trim())
        .bind(plan.id)
        .bind(plan.operation_id)
        .bind(plan.stage_execution_id)
        .bind(plan.stage_run_unit_id)
        .bind(plan.scope_snapshot_id)
        .bind(plan.organization_id)
        .bind(plan.dispatch_epoch)
        .bind(&input.expected_manifest_hash)
        .bind(input.fence.attempt_epoch)
        .bind(input.fence.expected_checkpoint_version)
        .bind(input.fence.lease_token)
        .bind(aggregator_item.id)
        .bind(aggregator_worker.id)
        .bind(input.deliverable_submission_id)
        .bind(&input.gate_decision_hash)
        .bind(&input.gap_manifest)
        .bind(&input.gap_manifest_hash)
        .bind(repair_generation)
        .bind(if fuel_available {
            "opened"
        } else {
            "fuel_exhausted"
        })
        .fetch_one(&mut *tx)
        .await?;
    let gate_blocked_unit = stage_run_units::transition_cas(
        &mut *tx,
        unit.id,
        unit.operation_id,
        unit.stage_execution_id,
        unit.organization_id,
        stage_run_units::StageRunUnitStatus::Running,
        unit.row_version,
        stage_run_units::StageRunUnitStatus::GateBlocked,
        None,
    )
    .await?;
    if !fuel_available {
        tx.commit().await?;
        return Ok(OpenedStageTeamRepairRow {
            plan,
            unit: gate_blocked_unit,
            gap,
            generation: None,
            repair_work_item: None,
            aggregator_work_item: None,
            aggregator_worker,
            replayed: false,
        });
    }

    let dispatch_epoch = plan.dispatch_epoch.saturating_add(1);
    let repair_role = plan
        .allowed_worker_roles
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|role| Some(*role) != plan.aggregator_role.as_deref())
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_repair_role_missing",
        })?
        .to_string();
    let repair_work_item_id = Uuid::new_v5(&gap.id, b"stage-team-repair-work-item-v1");
    let aggregator_work_item_id = Uuid::new_v5(&gap.id, b"stage-team-repair-aggregator-v1");
    let repair_input_refs = serde_json::json!([{
        "gap_id": gap.id,
        "gap_manifest": gap.gap_manifest,
        "gap_manifest_hash": gap.gap_manifest_hash,
        "gate_decision_hash": gap.gate_decision_hash,
        "kind": "stage_team_gate_gap",
        "source_dispatch_epoch": gap.source_dispatch_epoch,
    }]);
    let aggregator_input_refs = serde_json::json!([{
        "kind": "stage_team_repair_generation",
        "repair_generation": repair_generation,
        "source_gap_id": gap.id,
    }]);
    let repair_input_hash = format!(
        "sha256:{}",
        operation_scope_decisions::sha256_json(&repair_input_refs)
    );
    let aggregator_input_hash = format!(
        "sha256:{}",
        operation_scope_decisions::sha256_json(&aggregator_input_refs)
    );
    let generation_manifest = serde_json::json!({
        "aggregator_input_hash": aggregator_input_hash,
        "aggregator_work_item_id": aggregator_work_item_id,
        "dispatch_epoch": dispatch_epoch,
        "gap_manifest_hash": gap.gap_manifest_hash,
        "repair_generation": repair_generation,
        "repair_input_hash": repair_input_hash,
        "repair_role": repair_role,
        "repair_work_item_id": repair_work_item_id,
        "schema_version": 1,
        "source_dispatch_epoch": gap.source_dispatch_epoch,
        "source_gap_id": gap.id,
    });
    let generation_manifest_hash = format!(
        "sha256:{}",
        operation_scope_decisions::sha256_json(&generation_manifest)
    );
    let generation_id = Uuid::new_v5(&gap.id, b"stage-team-repair-generation-v1");
    let generation_sql = format!(
        r#"INSERT INTO stage_team_repair_generations(
               id,team_plan_id,source_gap_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,dispatch_epoch,
               manifest,manifest_hash,status
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'building') RETURNING {}"#,
        stage_teams::REPAIR_GENERATION_COLUMNS
    );
    sqlx::query_as::<_, stage_teams::StageTeamRepairGenerationRow>(&generation_sql)
        .bind(generation_id)
        .bind(plan.id)
        .bind(gap.id)
        .bind(plan.operation_id)
        .bind(plan.stage_execution_id)
        .bind(plan.stage_run_unit_id)
        .bind(plan.scope_snapshot_id)
        .bind(plan.organization_id)
        .bind(dispatch_epoch)
        .bind(&generation_manifest)
        .bind(&generation_manifest_hash)
        .fetch_one(&mut *tx)
        .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "UPDATE stage_team_plans
            SET dispatch_epoch=$2,requests_closed_at=NULL,final_submitter_worker_run_id=NULL,
                row_version=row_version+1,updated_at=NOW()
          WHERE id=$1 AND dispatch_epoch=$3 AND requests_closed_at IS NOT NULL
            AND final_submitter_worker_run_id=$4 AND row_version=$5
          RETURNING *",
    )
    .bind(plan.id)
    .bind(dispatch_epoch)
    .bind(input.expected_dispatch_epoch)
    .bind(aggregator_worker.id)
    .bind(plan.row_version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Conflict {
        code: "stage_team_repair_epoch_advance_cas_failed",
    })?;
    let repair_work_item = stage_teams::insert_work_item_with_executor(
        &mut *tx,
        &stage_teams::NewStageWorkItem {
            id: repair_work_item_id,
            team_plan_id: plan.id,
            operation_id: plan.operation_id,
            stage_execution_id: plan.stage_execution_id,
            stage_run_unit_id: plan.stage_run_unit_id,
            scope_snapshot_id: plan.scope_snapshot_id,
            organization_id: plan.organization_id,
            dispatch_epoch,
            kind: "gate_repair".to_string(),
            stable_key: format!("gate-repair:{dispatch_epoch}:{}", gap.gap_manifest_hash),
            role: repair_role,
            input_manifest_hash: repair_input_hash,
            input_refs: repair_input_refs,
            required_for_barrier: true,
            conflict_key: Some("stage_unit_gate_repair".to_string()),
            priority: i32::MIN,
            attempt_policy: serde_json::json!({"max_attempts": 2}),
            budget: serde_json::json!({"repair_generation": repair_generation}),
            output_schema: "stage_worker_output.v1".to_string(),
            created_by: "gate_repair".to_string(),
        },
    )
    .await?;
    let aggregator_work_item = stage_teams::insert_work_item_with_executor(
        &mut *tx,
        &stage_teams::NewStageWorkItem {
            id: aggregator_work_item_id,
            team_plan_id: plan.id,
            operation_id: plan.operation_id,
            stage_execution_id: plan.stage_execution_id,
            stage_run_unit_id: plan.stage_run_unit_id,
            scope_snapshot_id: plan.scope_snapshot_id,
            organization_id: plan.organization_id,
            dispatch_epoch,
            kind: "stage_aggregate".to_string(),
            stable_key: format!("aggregator:repair:{dispatch_epoch}"),
            role: plan.aggregator_role.clone().ok_or(
                RuntimeMemoryStoreError::IdentityMismatch {
                    code: "stage_team_repair_aggregator_role_missing",
                },
            )?,
            input_manifest_hash: aggregator_input_hash,
            input_refs: aggregator_input_refs,
            required_for_barrier: false,
            conflict_key: Some("stage_unit_finalizer".to_string()),
            priority: i32::MAX,
            attempt_policy: serde_json::json!({"max_attempts": 2}),
            budget: serde_json::json!({"repair_generation": repair_generation}),
            output_schema: "stage_unit_aggregate.v1".to_string(),
            created_by: "gate_repair".to_string(),
        },
    )
    .await?;
    let generation_sql = format!(
        "UPDATE stage_team_repair_generations
            SET repair_work_item_id=$2,aggregator_work_item_id=$3,status='sealed',sealed_at=NOW()
          WHERE id=$1 AND status='building'
          RETURNING {}",
        stage_teams::REPAIR_GENERATION_COLUMNS
    );
    let generation =
        sqlx::query_as::<_, stage_teams::StageTeamRepairGenerationRow>(&generation_sql)
            .bind(generation_id)
            .bind(repair_work_item.id)
            .bind(aggregator_work_item.id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_repair_generation_seal_failed",
            })?;
    let unit = stage_run_units::transition_cas(
        &mut *tx,
        gate_blocked_unit.id,
        gate_blocked_unit.operation_id,
        gate_blocked_unit.stage_execution_id,
        gate_blocked_unit.organization_id,
        stage_run_units::StageRunUnitStatus::GateBlocked,
        gate_blocked_unit.row_version,
        stage_run_units::StageRunUnitStatus::Running,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(OpenedStageTeamRepairRow {
        plan,
        unit,
        gap,
        generation: Some(generation),
        repair_work_item: Some(repair_work_item),
        aggregator_work_item: Some(aggregator_work_item),
        aggregator_worker,
        replayed: false,
    })
}

/// Deterministically close a Team Unit as GateBlocked when the closed producer
/// manifest contains at least one immutable blocked output.  No model
/// deliverable or Aggregator claim is accepted on this path: the barrier and
/// output ledger are the complete server authority.
pub async fn block_stage_team_unit(
    pool: &sqlx::PgPool,
    input: &BlockStageTeamUnitRow,
) -> RuntimeMemoryStoreResult<BlockedStageTeamUnitRow> {
    if input.expected_manifest_hash.len() != 71
        || !input.expected_manifest_hash.starts_with("sha256:")
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_team_manifest_hash",
        });
    }
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    if frozen_runtime_contract(&operation)? != runtime_memory_rollout::RuntimeMemoryContract::V2Only
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_requires_v2_only",
        });
    }
    let unit = load_runtime_unit_for_update(
        &mut tx,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
    )
    .await?;
    validate_runtime_stage_execution(
        &mut tx,
        &operation,
        input.stage_execution_id,
        &unit.stage_kind,
    )
    .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "SELECT * FROM stage_team_plans
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
            AND stage_run_unit_id=$4 FOR UPDATE",
    )
    .bind(input.stage_team_plan_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(input.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_team_plans",
    })?;
    if plan.dispatch_epoch != input.expected_dispatch_epoch
        || plan.requests_closed_at.is_none()
        || plan.final_submitter_worker_run_id.is_some()
        || plan.aggregator_kind != "worker"
        || plan.final_submitter_kind != "worker"
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_block_authority_mismatch",
        });
    }
    let barrier = stage_teams::load_barrier_with_connection(&mut tx, plan.id).await?;
    if !barrier.ready_to_finalize() || barrier.manifest_hash != input.expected_manifest_hash {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_sibling_barrier_not_ready",
        });
    }
    let blocked_outputs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stage_worker_outputs
          WHERE team_plan_id=$1 AND business_disposition='blocked'",
    )
    .bind(plan.id)
    .fetch_one(&mut *tx)
    .await?;
    if blocked_outputs == 0 {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_block_requires_blocked_output",
        });
    }
    let aggregator_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "SELECT * FROM stage_work_items
          WHERE team_plan_id=$1 AND role=$2 AND dispatch_epoch=$3
            AND required_for_barrier=FALSE
          ORDER BY id FOR UPDATE",
    )
    .bind(plan.id)
    .bind(&plan.aggregator_role)
    .bind(plan.dispatch_epoch)
    .fetch_one(&mut *tx)
    .await?;
    let replayed = unit.status == "gate_blocked" && aggregator_item.status == "superseded";
    let (unit, aggregator_item) = if replayed {
        (unit, aggregator_item)
    } else {
        if unit.status != "running" || aggregator_item.status != "queued" {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_block_lifecycle_mismatch",
            });
        }
        let aggregator_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
            "UPDATE stage_work_items
                SET status='superseded',terminal_at=NOW(),row_version=row_version+1,
                    updated_at=NOW()
              WHERE id=$1 AND team_plan_id=$2 AND status='queued' AND row_version=$3
              RETURNING *",
        )
        .bind(aggregator_item.id)
        .bind(plan.id)
        .bind(aggregator_item.row_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_work_items",
            expected: aggregator_item.row_version,
            actual: -1,
        })?;
        let unit = stage_run_units::transition_cas(
            &mut *tx,
            unit.id,
            unit.operation_id,
            unit.stage_execution_id,
            unit.organization_id,
            stage_run_units::StageRunUnitStatus::Running,
            unit.row_version,
            stage_run_units::StageRunUnitStatus::GateBlocked,
            None,
        )
        .await?;
        (unit, aggregator_item)
    };
    tx.commit().await?;
    Ok(BlockedStageTeamUnitRow {
        plan,
        aggregator_work_item: aggregator_item,
        unit,
        barrier,
        replayed,
    })
}

/// Team-mode final seal.  The producer barrier, exact final submitter and the
/// ordinary Unit seal are all re-locked and committed together, so no sibling
/// can change the closed manifest between validation and PASS.
pub async fn finalize_stage_team_unit(
    pool: &sqlx::PgPool,
    input: &FinalizeStageTeamUnitRow,
) -> RuntimeMemoryStoreResult<FinalizedStageTeamUnitRow> {
    if input.expected_manifest_hash.len() != 71
        || !input.expected_manifest_hash.starts_with("sha256:")
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_team_manifest_hash",
        });
    }
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.final_seal.fence.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    if frozen_runtime_contract(&operation)? != runtime_memory_rollout::RuntimeMemoryContract::V2Only
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_requires_v2_only",
        });
    }
    let unit = load_runtime_unit_for_update(
        &mut tx,
        input.final_seal.fence.operation_id,
        input.final_seal.fence.stage_execution_id,
        input.final_seal.fence.stage_run_unit_id,
    )
    .await?;
    validate_runtime_stage_execution(
        &mut tx,
        &operation,
        input.final_seal.fence.stage_execution_id,
        &unit.stage_kind,
    )
    .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "SELECT * FROM stage_team_plans
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
            AND stage_run_unit_id=$4 FOR UPDATE",
    )
    .bind(input.stage_team_plan_id)
    .bind(input.final_seal.fence.operation_id)
    .bind(input.final_seal.fence.stage_execution_id)
    .bind(input.final_seal.fence.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_team_plans",
    })?;
    if plan.dispatch_epoch != input.expected_dispatch_epoch
        || plan.requests_closed_at.is_none()
        || plan.aggregator_kind != "worker"
        || plan.final_submitter_kind != "worker"
        || plan.final_submitter_worker_run_id != Some(input.final_seal.fence.worker_run_id)
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_final_submitter_identity_mismatch",
        });
    }
    let aggregator_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "SELECT * FROM stage_work_items
          WHERE id=$1 AND team_plan_id=$2 AND operation_id=$3
            AND stage_execution_id=$4 AND stage_run_unit_id=$5 FOR UPDATE",
    )
    .bind(input.aggregator_work_item_id)
    .bind(plan.id)
    .bind(plan.operation_id)
    .bind(plan.stage_execution_id)
    .bind(plan.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_work_items",
    })?;
    if plan.aggregator_role.as_deref() != Some(aggregator_item.role.as_str())
        || aggregator_item.required_for_barrier
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_aggregator_work_item_mismatch",
        });
    }
    let aggregator_worker = sqlx::query_as::<_, StageWorkerRunRow>(
        "SELECT * FROM stage_worker_runs WHERE id=$1 FOR UPDATE",
    )
    .bind(input.final_seal.fence.worker_run_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_worker_runs",
    })?;
    if aggregator_worker.work_item_id != Some(aggregator_item.id)
        || aggregator_worker.operation_id != plan.operation_id
        || aggregator_worker.stage_execution_id != plan.stage_execution_id
        || aggregator_worker.stage_run_unit_id != plan.stage_run_unit_id
        || aggregator_worker.organization_id != plan.organization_id
        || aggregator_worker.attempt_epoch != input.final_seal.fence.attempt_epoch
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_final_submitter_identity_mismatch",
        });
    }
    let replayed_state = unit.status == "passed"
        && aggregator_item.status == "completed"
        && aggregator_worker.status == "passed";
    if !replayed_state
        && (unit.status != "running"
            || aggregator_item.status != "running"
            || aggregator_worker.status != "running"
            || aggregator_worker.lease_token != Some(input.final_seal.fence.lease_token)
            || aggregator_worker.checkpoint_version
                != input.final_seal.fence.expected_checkpoint_version
            || aggregator_worker.active_tool_call_id.is_some()
            || aggregator_worker
                .lease_expires_at
                .is_none_or(|expires_at| expires_at <= chrono::Utc::now()))
    {
        return Err(RuntimeMemoryStoreError::LeaseLost {
            worker_run_id: input.final_seal.fence.worker_run_id,
            attempt_epoch: input.final_seal.fence.attempt_epoch,
        });
    }
    let live_worker_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM stage_worker_runs
          WHERE stage_run_unit_id=$1
            AND status IN ('queued','running','waiting_background')
          ORDER BY id FOR UPDATE",
    )
    .bind(plan.stage_run_unit_id)
    .fetch_all(&mut *tx)
    .await?;
    if (!replayed_state && live_worker_ids.as_slice() != [input.final_seal.fence.worker_run_id])
        || (replayed_state && !live_worker_ids.is_empty())
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_final_submitter_not_unique_live_worker",
        });
    }
    let barrier = stage_teams::load_barrier_with_connection_ignoring_worker(
        &mut tx,
        plan.id,
        Some(aggregator_worker.id),
    )
    .await?;
    if !barrier.ready_to_finalize() || barrier.manifest_hash != input.expected_manifest_hash {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_sibling_barrier_not_ready",
        });
    }
    let finalized = finalize_unit_pass_in_transaction(
        &mut tx,
        &input.final_seal,
        Some(input.stage_team_plan_id),
    )
    .await?;
    let aggregator_item = if aggregator_item.status == "completed" {
        aggregator_item
    } else {
        sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
            "UPDATE stage_work_items
                SET status='completed',terminal_at=NOW(),row_version=row_version+1,
                    updated_at=NOW()
              WHERE id=$1 AND team_plan_id=$2 AND status='running' AND row_version=$3
              RETURNING *",
        )
        .bind(aggregator_item.id)
        .bind(plan.id)
        .bind(aggregator_item.row_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_work_items",
            expected: aggregator_item.row_version,
            actual: -1,
        })?
    };
    tx.commit().await?;
    reconcile_deployment_rollouts_best_effort(pool, "finalize_stage_team_unit").await;
    Ok(FinalizedStageTeamUnitRow {
        plan,
        aggregator_work_item: aggregator_item,
        finalized,
    })
}

/// Wave-aware V2 Gate PASS close. The exact running wave, Worker landing and
/// either next-wave creation or final Unit seal share one database transaction.
/// This path deliberately never writes the legacy completion ledger unless the
/// final seal succeeds.
pub async fn close_wave_gate_pass(
    pool: &sqlx::PgPool,
    input: &CloseWaveGatePassRow,
) -> RuntimeMemoryStoreResult<ClosedWaveGatePassRow> {
    validate_final_gate_decision(&input.final_seal)?;
    if !input.continuation_pass_watermark.is_object() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_wave_continuation_watermark",
        });
    }
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.final_seal.fence.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    let contract = frozen_runtime_contract(&operation)?;
    if !contract_writes_v2(contract) {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "runtime_v2_not_enabled",
        });
    }
    let locked_unit = load_runtime_unit_for_update(
        &mut tx,
        input.final_seal.fence.operation_id,
        input.final_seal.fence.stage_execution_id,
        input.final_seal.fence.stage_run_unit_id,
    )
    .await?;
    validate_runtime_stage_execution(
        &mut tx,
        &operation,
        input.final_seal.fence.stage_execution_id,
        &locked_unit.stage_kind,
    )
    .await?;

    if locked_unit.status == stage_run_units::StageRunUnitStatus::Passed.as_str() {
        let completed_wave = sqlx::query_as::<_, stage_asset_waves::StageAssetWaveRow>(
            "SELECT * FROM stage_asset_waves WHERE id=$1 FOR SHARE",
        )
        .bind(input.wave_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_asset_waves",
        })?;
        if completed_wave.operation_id != locked_unit.operation_id
            || completed_wave.organization_id != locked_unit.organization_id
            || completed_wave.stage_kind != locked_unit.stage_kind
            || completed_wave.status != "completed"
            || completed_wave.completed_at.is_none()
        {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_asset_wave_replay_identity_mismatch",
            });
        }
        let terminal_wave_id = input
            .final_seal
            .coverage_watermark
            .get("waves")
            .and_then(serde_json::Value::as_array)
            .and_then(|waves| waves.last())
            .and_then(|wave| wave.get("id"))
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok());
        let terminal_wave_index = input
            .final_seal
            .coverage_watermark
            .get("waves")
            .and_then(serde_json::Value::as_array)
            .and_then(|waves| waves.last())
            .and_then(|wave| wave.get("wave_index"))
            .and_then(serde_json::Value::as_i64);
        let child_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM stage_asset_waves WHERE parent_wave_id=$1",
        )
        .bind(input.wave_id)
        .fetch_one(&mut *tx)
        .await?;
        let latest_wave_index = sqlx::query_scalar::<_, Option<i32>>(
            r#"SELECT MAX(wave_index) FROM stage_asset_waves
                WHERE operation_id=$1 AND organization_id=$2 AND stage_kind=$3"#,
        )
        .bind(locked_unit.operation_id)
        .bind(locked_unit.organization_id)
        .bind(&locked_unit.stage_kind)
        .fetch_one(&mut *tx)
        .await?;
        if terminal_wave_id != Some(input.wave_id)
            || terminal_wave_index != Some(i64::from(completed_wave.wave_index))
            || latest_wave_index != Some(completed_wave.wave_index)
            || child_count != 0
        {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_asset_wave_final_replay_mismatch",
            });
        }
        let replayed = finalize_unit_pass_in_transaction(&mut tx, &input.final_seal, None).await?;
        tx.commit().await?;
        return Ok(ClosedWaveGatePassRow::Finalized(replayed));
    }

    let replay_row_version = input
        .final_seal
        .expected_unit_row_version
        .checked_add(1)
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "stage_asset_wave_replay_row_version_overflow",
        })?;
    if locked_unit.status == stage_run_units::StageRunUnitStatus::Running.as_str()
        && locked_unit.row_version == replay_row_version
        && locked_unit.pass_watermark == input.continuation_pass_watermark
    {
        let completed_wave = sqlx::query_as::<_, stage_asset_waves::StageAssetWaveRow>(
            "SELECT * FROM stage_asset_waves WHERE id=$1 FOR SHARE",
        )
        .bind(input.wave_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_asset_waves",
        })?;
        let mut child_waves = sqlx::query_as::<_, stage_asset_waves::StageAssetWaveRow>(
            r#"SELECT * FROM stage_asset_waves
                WHERE parent_wave_id=$1 AND operation_id=$2
                  AND organization_id=$3 AND stage_kind=$4
                ORDER BY wave_index, id
                FOR SHARE"#,
        )
        .bind(input.wave_id)
        .bind(locked_unit.operation_id)
        .bind(locked_unit.organization_id)
        .bind(&locked_unit.stage_kind)
        .fetch_all(&mut *tx)
        .await?;
        let replay_checkpoint_version = input
            .final_seal
            .fence
            .expected_checkpoint_version
            .checked_add(1)
            .ok_or(RuntimeMemoryStoreError::Conflict {
                code: "stage_asset_wave_replay_checkpoint_version_overflow",
            })?;
        let worker = sqlx::query_as::<_, StageWorkerRunRow>(
            r#"SELECT * FROM stage_worker_runs
                WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
                  AND stage_run_unit_id=$4 AND organization_id=$5
                  AND attempt_epoch=$6 AND checkpoint_version=$7
                  AND status='waiting_background' AND lease_token IS NULL
                  AND lease_owner IS NULL AND lease_expires_at IS NULL
                  AND active_tool_call_id IS NULL AND checkpoint=$8
                FOR UPDATE"#,
        )
        .bind(input.final_seal.fence.worker_run_id)
        .bind(locked_unit.operation_id)
        .bind(locked_unit.stage_execution_id)
        .bind(locked_unit.id)
        .bind(locked_unit.organization_id)
        .bind(input.final_seal.fence.attempt_epoch)
        .bind(replay_checkpoint_version)
        .bind(&input.final_seal.terminal_checkpoint)
        .fetch_optional(&mut *tx)
        .await?;
        let completion_run_id = sqlx::query_scalar::<_, Option<String>>(
            r#"SELECT stage_run_id FROM org_stage_completions
                WHERE organization_id=$1 AND stage_kind=$2 FOR SHARE"#,
        )
        .bind(locked_unit.organization_id)
        .bind(&locked_unit.stage_kind)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
        let operation_run_id = locked_unit.operation_id.to_string();
        let no_false_completion = completion_run_id.as_deref() != Some(operation_run_id.as_str());
        if completed_wave.operation_id != locked_unit.operation_id
            || completed_wave.organization_id != locked_unit.organization_id
            || completed_wave.stage_kind != locked_unit.stage_kind
            || completed_wave.status != "completed"
            || completed_wave.completed_at.is_none()
            || child_waves.len() != 1
            || child_waves[0].status != "running"
            || child_waves[0].wave_index != completed_wave.wave_index + 1
            || worker.is_none()
            || !no_false_completion
        {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_asset_wave_continuation_replay_mismatch",
            });
        }
        let child_wave = child_waves.remove(0);
        let items = sqlx::query_as::<_, stage_asset_waves::StageAssetWaveItemRow>(
            "SELECT * FROM stage_asset_wave_items WHERE wave_id=$1 ORDER BY id FOR SHARE",
        )
        .bind(child_wave.id)
        .fetch_all(&mut *tx)
        .await?;
        stage_asset_waves::validate_wave_items(&child_wave, &items).map_err(|_| {
            RuntimeMemoryStoreError::Conflict {
                code: "stage_asset_wave_continuation_replay_items_invalid",
            }
        })?;
        tx.commit().await?;
        return Ok(ClosedWaveGatePassRow::WaitingBackground {
            unit: locked_unit,
            worker: worker.expect("validated replay worker"),
            next_wave: stage_asset_waves::StageAssetWaveWithItems {
                wave: child_wave,
                items,
            },
        });
    }

    if locked_unit.status != input.final_seal.expected_unit_status.as_str()
        || locked_unit.row_version != input.final_seal.expected_unit_row_version
    {
        return Err(RuntimeMemoryStoreError::StaleVersion {
            entity: stage_run_units::TABLE_NAME,
            expected: input.final_seal.expected_unit_row_version,
            actual: locked_unit.row_version,
        });
    }
    // Apply the same stage-specific candidate acceptance shape constraint even
    // on the continuation alternative. Wave-aware information stages must not
    // smuggle an attack-candidate command into a non-final transaction.
    let _ = bind_candidate_acceptance(&locked_unit, &input.final_seal)?;

    let next_wave = complete_wave_and_create_next_without_completion(
        &mut tx,
        &locked_unit,
        input.wave_id,
        input.next_wave_limit,
    )
    .await?;
    if let Some(next_wave) = next_wave {
        let worker = stage_worker_runs::finish_attempt_cas(
            &mut *tx,
            &input.final_seal.fence,
            stage_worker_runs::StageWorkerRunStatus::Running,
            stage_worker_runs::StageWorkerRunStatus::WaitingBackground,
            &input.final_seal.terminal_checkpoint,
            None,
        )
        .await?;
        let unit = stage_run_units::checkpoint_running_pass_watermark(
            &mut *tx,
            locked_unit.id,
            locked_unit.operation_id,
            locked_unit.stage_execution_id,
            locked_unit.organization_id,
            locked_unit.row_version,
            &input.continuation_pass_watermark,
        )
        .await?;
        mirror_worker_if_required(
            &mut tx,
            &operation,
            contract,
            &unit,
            &worker,
            "wave_continuation",
        )
        .await?;
        tx.commit().await?;
        if contract_writes_legacy_mirror(contract) {
            reconcile_deployment_rollouts_best_effort(pool, "close_wave_gate_pass").await;
        }
        return Ok(ClosedWaveGatePassRow::WaitingBackground {
            unit,
            worker,
            next_wave,
        });
    }

    let finalized = finalize_unit_pass_in_transaction(&mut tx, &input.final_seal, None).await?;
    tx.commit().await?;
    if contract_writes_legacy_mirror(contract) || input.final_seal.candidate_acceptance.is_some() {
        reconcile_deployment_rollouts_best_effort(pool, "close_wave_gate_pass").await;
    }
    Ok(ClosedWaveGatePassRow::Finalized(finalized))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct StartupWorkerReaperStats {
    pub requeued: u64,
    pub recovery_required: u64,
    pub shadow_samples_written: u64,
}

/// Reconcile Team-owned Workers by their exact WorkItem/TeamPlan identity.
/// A Team Unit deliberately does not share one specialist with all sibling
/// roles, so the legacy `worker.specialist = unit.specialist` join is invalid
/// for these rows. Startup preserves active tools as `recovery_required`; the
/// later exact claim transaction may reconcile the closed bounded-crawler
/// policy, while every other active tool stays operator-owned. A cleanly
/// expired attempt requeues the same WorkerRun/message chain.
async fn reap_expired_stage_team_workers_on_startup(
    connection: &mut sqlx::PgConnection,
    operation_id: Uuid,
    cutoff: chrono::DateTime<chrono::Utc>,
) -> RuntimeMemoryStoreResult<(Vec<StageWorkerRunRow>, Vec<StageWorkerRunRow>)> {
    let expired = sqlx::query_as::<_, (Uuid, i64, Option<Uuid>, Uuid, i64, Uuid)>(
        r#"SELECT worker.id,worker.attempt_epoch,worker.active_tool_call_id,
                  item.id,item.row_version,plan.id
             FROM tasks AS task
             JOIN operation_state AS operation ON operation.operation_id=task.id
             JOIN stage_runs AS execution
               ON execution.operation_id=operation.operation_id
              AND execution.stage_kind=operation.current_stage
              AND execution.status='started'
             JOIN stage_run_units AS unit
               ON unit.operation_id=operation.operation_id
              AND unit.stage_execution_id=execution.id
             JOIN stage_team_plans AS plan
               ON plan.operation_id=operation.operation_id
              AND plan.stage_execution_id=execution.id
              AND plan.stage_run_unit_id=unit.id
              AND plan.scope_snapshot_id=unit.scope_snapshot_id
              AND plan.organization_id=unit.organization_id
             JOIN stage_work_items AS item
               ON item.team_plan_id=plan.id
              AND item.operation_id=plan.operation_id
              AND item.stage_execution_id=plan.stage_execution_id
              AND item.stage_run_unit_id=plan.stage_run_unit_id
              AND item.scope_snapshot_id=plan.scope_snapshot_id
              AND item.organization_id=plan.organization_id
             JOIN stage_worker_runs AS worker
               ON worker.work_item_id=item.id
              AND worker.operation_id=item.operation_id
              AND worker.stage_execution_id=item.stage_execution_id
              AND worker.stage_run_unit_id=item.stage_run_unit_id
              AND worker.organization_id=item.organization_id
              AND worker.specialist=item.role
             JOIN operation_org_scope_snapshots AS snapshot
               ON snapshot.operation_id=operation.operation_id
              AND snapshot.id=unit.scope_snapshot_id
              AND snapshot.project_scope_id=operation.project_scope_id
              AND snapshot.sealed_at IS NOT NULL
             JOIN operation_org_scope_units AS member
               ON member.snapshot_id=snapshot.id
              AND member.organization_id=unit.organization_id
            WHERE task.status IN ('running','waiting')
              AND task.updated_at<$1
              AND operation.operation_id=$2
              AND operation.superseded_by IS NULL
              AND operation.runtime_memory_contract='v2_only'
              AND item.status='running'
              AND worker.status IN ('running','waiting_background','gate_blocked')
              AND worker.lease_token IS NOT NULL
              AND worker.lease_expires_at<=NOW()
            ORDER BY plan.id,item.priority,item.id,worker.id
            FOR UPDATE OF item,worker"#,
    )
    .bind(cutoff)
    .bind(operation_id)
    .fetch_all(&mut *connection)
    .await?;

    let mut recovery_required = Vec::new();
    let mut requeued = Vec::new();
    for (worker_id, attempt_epoch, active_tool_call_id, item_id, item_row_version, plan_id) in
        expired
    {
        if let Some(active_tool_call_id) = active_tool_call_id {
            let exact_active_tool = sqlx::query_scalar::<_, bool>(
                r#"SELECT EXISTS (
                       SELECT 1
                         FROM tool_calls AS active_tool
                         JOIN stage_worker_runs AS worker ON worker.id=$1
                        WHERE active_tool.id=$2
                          AND active_tool.worker_run_id=worker.id
                          AND active_tool.operation_id=worker.operation_id
                          AND active_tool.stage_execution_id=worker.stage_execution_id
                          AND active_tool.stage_run_unit_id=worker.stage_run_unit_id
                          AND active_tool.organization_id=worker.organization_id
                          AND active_tool.attempt_epoch=worker.attempt_epoch
                          AND active_tool.lease_token=worker.lease_token
                          AND active_tool.status IN ('received','running')
                   )"#,
            )
            .bind(worker_id)
            .bind(active_tool_call_id)
            .fetch_one(&mut *connection)
            .await?;
            if !exact_active_tool {
                // Keep malformed authority untouched so the shared task
                // predicate fails the operation closed instead of inventing a
                // recoverable shape.
                continue;
            }
            let worker = sqlx::query_as::<_, StageWorkerRunRow>(
                r#"UPDATE stage_worker_runs
                      SET status='recovery_required',updated_at=NOW()
                    WHERE id=$1 AND attempt_epoch=$2
                      AND status IN ('running','waiting_background','gate_blocked')
                    RETURNING *"#,
            )
            .bind(worker_id)
            .bind(attempt_epoch)
            .fetch_optional(&mut *connection)
            .await?
            .ok_or(RuntimeMemoryStoreError::Conflict {
                code: "startup_team_worker_recovery_cas_failed",
            })?;
            let rows = sqlx::query(
                r#"UPDATE stage_work_items
                      SET status='recovery_required',row_version=row_version+1,updated_at=NOW()
                    WHERE id=$1 AND status='running' AND row_version=$2"#,
            )
            .bind(item_id)
            .bind(item_row_version)
            .execute(&mut *connection)
            .await?
            .rows_affected();
            if rows != 1 {
                return Err(RuntimeMemoryStoreError::StaleVersion {
                    entity: "stage_work_items",
                    expected: item_row_version,
                    actual: -1,
                });
            }
            recovery_required.push(worker);
            continue;
        }

        let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
            "SELECT * FROM stage_team_plans WHERE id=$1 FOR UPDATE",
        )
        .bind(plan_id)
        .fetch_one(&mut *connection)
        .await?;
        let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
            "SELECT * FROM stage_work_items WHERE id=$1 FOR UPDATE",
        )
        .bind(item_id)
        .fetch_one(&mut *connection)
        .await?;
        let worker = sqlx::query_as::<_, StageWorkerRunRow>(
            "SELECT * FROM stage_worker_runs WHERE id=$1 FOR UPDATE",
        )
        .bind(worker_id)
        .fetch_one(&mut *connection)
        .await?;
        if item.row_version != item_row_version || worker.attempt_epoch != attempt_epoch {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "startup_team_worker_reap_cas_failed",
            });
        }
        let resolved =
            stage_teams::reap_expired_clean_stage_worker(connection, &plan, &item, &worker).await?;
        if resolved.retry_scheduled {
            requeued.push(resolved.worker);
        }
    }
    Ok((recovery_required, requeued))
}

/// Reconcile expired V2 WorkerRuns while the startup task reaper owns one
/// transaction. Identity joins are deliberately redundant with FKs: malformed
/// or cross-operation rows are not mutated into a plausible shape and the task
/// predicate will fail them closed instead.
pub(crate) async fn reap_expired_workers_on_startup(
    connection: &mut sqlx::PgConnection,
    cutoff: chrono::DateTime<chrono::Utc>,
) -> RuntimeMemoryStoreResult<StartupWorkerReaperStats> {
    let operation_ids = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT DISTINCT operation.operation_id
         FROM tasks task
         JOIN operation_state operation ON operation.operation_id=task.id
         JOIN stage_runs execution
           ON execution.operation_id=operation.operation_id
          AND execution.stage_kind=operation.current_stage
          AND execution.status='started'
         JOIN stage_run_units unit
           ON unit.operation_id=operation.operation_id
          AND unit.stage_execution_id=execution.id
         JOIN stage_worker_runs worker
           ON worker.operation_id=operation.operation_id
          AND worker.stage_execution_id=execution.id
          AND worker.stage_run_unit_id=unit.id
          AND worker.organization_id=unit.organization_id
         JOIN operation_org_scope_snapshots snapshot
           ON snapshot.operation_id=operation.operation_id
          AND snapshot.id=unit.scope_snapshot_id
          AND snapshot.project_scope_id=operation.project_scope_id
          AND snapshot.sealed_at IS NOT NULL
         JOIN operation_org_scope_units member
           ON member.snapshot_id=snapshot.id
          AND member.organization_id=unit.organization_id
        WHERE task.status IN ('running','waiting')
          AND task.updated_at<$1
          AND operation.superseded_by IS NULL
          AND operation.runtime_memory_contract<>'legacy_v1'
          AND (
              SELECT COUNT(*) FROM stage_runs active_count
              WHERE active_count.operation_id=operation.operation_id
                AND active_count.status='started'
          )=1
          AND worker.status IN ('running','waiting_background','gate_blocked')
          AND worker.lease_token IS NOT NULL
          AND worker.lease_expires_at<=NOW()
          AND (
              (
                  worker.specialist=unit.specialist
                  AND NOT EXISTS (
                      SELECT 1 FROM stage_team_plans legacy_plan
                      WHERE legacy_plan.stage_run_unit_id=unit.id
                  )
              )
              OR EXISTS (
                  SELECT 1
                    FROM stage_team_plans team_plan
                    JOIN stage_work_items team_item
                      ON team_item.team_plan_id=team_plan.id
                   WHERE team_plan.stage_run_unit_id=unit.id
                     AND team_plan.operation_id=worker.operation_id
                     AND team_plan.stage_execution_id=worker.stage_execution_id
                     AND team_item.id=worker.work_item_id
                     AND team_item.organization_id=worker.organization_id
                     AND team_item.role=worker.specialist
              )
          )
        ORDER BY operation.operation_id"#,
    )
    .bind(cutoff)
    .fetch_all(&mut *connection)
    .await?;

    let common = r#"FROM tasks task
         JOIN operation_state operation ON operation.operation_id=task.id
         JOIN stage_runs execution
           ON execution.operation_id=operation.operation_id
          AND execution.stage_kind=operation.current_stage
          AND execution.status='started'
         JOIN stage_run_units unit
           ON unit.operation_id=operation.operation_id
          AND unit.stage_execution_id=execution.id
         JOIN operation_org_scope_snapshots snapshot
           ON snapshot.operation_id=operation.operation_id
          AND snapshot.id=unit.scope_snapshot_id
          AND snapshot.project_scope_id=operation.project_scope_id
          AND snapshot.sealed_at IS NOT NULL
         JOIN operation_org_scope_units member
           ON member.snapshot_id=snapshot.id
          AND member.organization_id=unit.organization_id
        WHERE worker.operation_id=operation.operation_id
          AND worker.stage_execution_id=execution.id
          AND worker.stage_run_unit_id=unit.id
          AND worker.organization_id=unit.organization_id
          AND worker.specialist=unit.specialist
          AND NOT EXISTS (
              SELECT 1 FROM stage_team_plans team_plan
              WHERE team_plan.stage_run_unit_id=unit.id
          )
          AND task.status IN ('running','waiting')
          AND task.updated_at<$1
          AND operation.operation_id=$2
          AND operation.superseded_by IS NULL
          AND operation.runtime_memory_contract<>'legacy_v1'
          AND (
              SELECT COUNT(*) FROM stage_runs active_count
              WHERE active_count.operation_id=operation.operation_id
                AND active_count.status='started'
          )=1
          AND worker.status IN ('running','waiting_background','gate_blocked')
          AND worker.lease_token IS NOT NULL
          AND worker.lease_expires_at<=NOW()"#;
    let mut stats = StartupWorkerReaperStats::default();
    for operation_id in operation_ids {
        let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
            .bind(operation_id)
            .fetch_optional(&mut *connection)
            .await?
            .ok_or(RuntimeMemoryStoreError::Missing {
                entity: "operation_state",
            })?;
        ensure_runtime_operation_active(&operation)?;
        let contract = frozen_runtime_contract(&operation)?;

        let recovery_sql = format!(
            r#"UPDATE stage_worker_runs worker
              SET status='recovery_required', updated_at=NOW()
            {common}
              AND worker.active_tool_call_id IS NOT NULL
              AND EXISTS (
                  SELECT 1 FROM tool_calls active_tool
                  WHERE active_tool.id=worker.active_tool_call_id
                    AND active_tool.worker_run_id=worker.id
                    AND active_tool.operation_id=worker.operation_id
                    AND active_tool.stage_execution_id=worker.stage_execution_id
                    AND active_tool.stage_run_unit_id=worker.stage_run_unit_id
                    AND active_tool.organization_id=worker.organization_id
                    AND active_tool.attempt_epoch=worker.attempt_epoch
                    AND active_tool.lease_token=worker.lease_token
                    AND active_tool.status IN ('received','running')
              )
            RETURNING worker.*"#
        );
        let recovery_required = sqlx::query_as::<_, StageWorkerRunRow>(&recovery_sql)
            .bind(cutoff)
            .bind(operation_id)
            .fetch_all(&mut *connection)
            .await?;
        let requeue_sql = format!(
            r#"UPDATE stage_worker_runs worker
              SET status='queued', lease_token=NULL, lease_owner=NULL,
                  lease_acquired_at=NULL, lease_expires_at=NULL,
                  heartbeat_at=NULL, updated_at=NOW(), terminal_at=NULL
            {common}
              AND worker.active_tool_call_id IS NULL
            RETURNING worker.*"#
        );
        let requeued = sqlx::query_as::<_, StageWorkerRunRow>(&requeue_sql)
            .bind(cutoff)
            .bind(operation_id)
            .fetch_all(&mut *connection)
            .await?;
        let (team_recovery_required, team_requeued) =
            reap_expired_stage_team_workers_on_startup(connection, operation_id, cutoff).await?;
        stats.recovery_required += recovery_required.len() as u64;
        stats.requeued += requeued.len() as u64;
        stats.recovery_required += team_recovery_required.len() as u64;
        stats.requeued += team_requeued.len() as u64;

        if contract_writes_legacy_mirror(contract)
            && (!recovery_required.is_empty() || !requeued.is_empty())
        {
            let mut legacy_blob = operation.state_blob.clone();
            for worker in recovery_required.iter().chain(&requeued) {
                let unit =
                    stage_run_units::get_with_executor(&mut *connection, worker.stage_run_unit_id)
                        .await?
                        .ok_or(RuntimeMemoryStoreError::Missing {
                            entity: "stage_run_units",
                        })?;
                if unit.operation_id != worker.operation_id
                    || unit.stage_execution_id != worker.stage_execution_id
                    || unit.organization_id != worker.organization_id
                    || unit.specialist.as_deref() != Some(worker.specialist.as_str())
                {
                    return Err(RuntimeMemoryStoreError::IdentityMismatch {
                        code: "startup_reaper_worker_unit_mismatch",
                    });
                }
                let organization_name = frozen_organization_name(connection, &unit).await?;
                apply_legacy_worker_mirror(
                    &mut legacy_blob,
                    &unit.stage_kind,
                    &organization_name,
                    worker,
                );
            }
            write_locked_legacy_state_blob(
                connection,
                operation_id,
                &operation.runtime_memory_contract,
                &legacy_blob,
            )
            .await?;
            for worker in recovery_required.iter().chain(&requeued) {
                runtime_memory_shadow::persist_worker_sample(
                    connection,
                    worker.id,
                    "startup_reaper",
                )
                .await?;
            }
            stats.shadow_samples_written += u64::try_from(recovery_required.len() + requeued.len())
                .map_err(|_| RuntimeMemoryStoreError::Conflict {
                    code: "startup_reaper_sample_count_overflow",
                })?;
        }
    }
    Ok(stats)
}

pub async fn reap_expired_worker(
    pool: &sqlx::PgPool,
    input: &LoadWorkerCheckpointRow,
) -> RuntimeMemoryStoreResult<(
    stage_worker_runs::ExpiredWorkerDisposition,
    StageWorkerRunRow,
)> {
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    let contract = frozen_runtime_contract(&operation)?;
    if !contract_writes_v2(contract) {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "runtime_v2_not_enabled",
        });
    }
    let unit = load_runtime_unit_for_update(
        &mut tx,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
    )
    .await?;
    validate_runtime_stage_execution(
        &mut tx,
        &operation,
        input.stage_execution_id,
        &unit.stage_kind,
    )
    .await?;
    let (disposition, worker) =
        stage_worker_runs::reap_expired_with_connection(&mut tx, input.worker_run_id).await?;
    validate_loaded_worker_identity(&worker, input, &unit)?;
    mirror_worker_if_required(
        &mut tx,
        &operation,
        contract,
        &unit,
        &worker,
        "expired_worker_reap",
    )
    .await?;
    tx.commit().await?;
    if contract_writes_legacy_mirror(contract) {
        reconcile_deployment_rollouts_best_effort(pool, "reap_expired_worker").await;
    }
    Ok((disposition, worker))
}

fn validate_loaded_worker_identity(
    worker: &StageWorkerRunRow,
    input: &LoadWorkerCheckpointRow,
    unit: &StageRunUnitRow,
) -> RuntimeMemoryStoreResult<()> {
    if worker.id != input.worker_run_id
        || worker.operation_id != input.operation_id
        || worker.stage_execution_id != input.stage_execution_id
        || worker.stage_run_unit_id != input.stage_run_unit_id
        || worker.organization_id != unit.organization_id
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "worker_checkpoint_identity_mismatch",
        });
    }
    if worker.message_chain_id.is_none() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "worker_checkpoint_missing_bound_chain",
        });
    }
    Ok(())
}

fn legacy_worker_from_blob(
    operation: &OperationStateRow,
    unit: &StageRunUnitRow,
    input: &LoadWorkerCheckpointRow,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow> {
    let org_record = operation
        .state_blob
        .get("stage_run_workers")
        .and_then(|workers| workers.get(&unit.stage_kind))
        .and_then(|stage| stage.get(unit.organization_id.to_string()))
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "legacy_stage_run_worker",
        })?;
    let value = org_record
        .get("worker_records")
        .and_then(|records| records.get(input.worker_run_id.to_string()))
        .unwrap_or(org_record)
        .clone();
    let worker: StageWorkerRunRow =
        serde_json::from_value(value).map_err(|error| RuntimeMemoryStoreError::Conflict {
            code: if error.is_data() {
                "legacy_worker_record_invalid"
            } else {
                "legacy_worker_record_unreadable"
            },
        })?;
    validate_loaded_worker_identity(&worker, input, unit)?;
    Ok(worker)
}

async fn select_worker_checkpoint_locked(
    connection: &mut sqlx::PgConnection,
    operation: &OperationStateRow,
    contract: runtime_memory_rollout::RuntimeMemoryContract,
    unit: &StageRunUnitRow,
    input: &LoadWorkerCheckpointRow,
) -> RuntimeMemoryStoreResult<LoadedWorkerCheckpointRow> {
    let v2 = stage_worker_runs::get_with_executor(connection, input.worker_run_id).await?;
    if let Some(selected) = input.selected_source {
        return match (contract, selected) {
            (
                runtime_memory_rollout::RuntimeMemoryContract::DualWriteV2Preferred
                | runtime_memory_rollout::RuntimeMemoryContract::V2Only,
                RuntimeMemoryRecordSource::V2,
            ) => {
                let worker = v2.ok_or(RuntimeMemoryStoreError::Missing {
                    entity: "stage_worker_runs",
                })?;
                validate_loaded_worker_identity(&worker, input, unit)?;
                Ok(LoadedWorkerCheckpointRow {
                    source: RuntimeMemoryRecordSource::V2,
                    worker,
                })
            }
            (
                runtime_memory_rollout::RuntimeMemoryContract::DualWriteV2Preferred,
                RuntimeMemoryRecordSource::LegacyFallback,
            ) => Ok(LoadedWorkerCheckpointRow {
                source: RuntimeMemoryRecordSource::LegacyFallback,
                worker: legacy_worker_from_blob(operation, unit, input)?,
            }),
            (
                runtime_memory_rollout::RuntimeMemoryContract::DualWriteLegacyRead,
                RuntimeMemoryRecordSource::Legacy,
            ) => Ok(LoadedWorkerCheckpointRow {
                source: RuntimeMemoryRecordSource::Legacy,
                worker: legacy_worker_from_blob(operation, unit, input)?,
            }),
            _ => Err(RuntimeMemoryStoreError::Conflict {
                code: "selected_runtime_memory_source_contract_mismatch",
            }),
        };
    }
    match contract {
        runtime_memory_rollout::RuntimeMemoryContract::LegacyV1 => {
            Err(RuntimeMemoryStoreError::Conflict {
                code: "runtime_v2_not_enabled",
            })
        }
        runtime_memory_rollout::RuntimeMemoryContract::DualWriteLegacyRead => {
            let worker = legacy_worker_from_blob(operation, unit, input)?;
            Ok(LoadedWorkerCheckpointRow {
                source: RuntimeMemoryRecordSource::Legacy,
                worker,
            })
        }
        runtime_memory_rollout::RuntimeMemoryContract::DualWriteV2Preferred => match v2 {
            Some(worker) if validate_loaded_worker_identity(&worker, input, unit).is_ok() => {
                Ok(LoadedWorkerCheckpointRow {
                    source: RuntimeMemoryRecordSource::V2,
                    worker,
                })
            }
            _ => Ok(LoadedWorkerCheckpointRow {
                source: RuntimeMemoryRecordSource::LegacyFallback,
                worker: legacy_worker_from_blob(operation, unit, input)?,
            }),
        },
        runtime_memory_rollout::RuntimeMemoryContract::V2Only => {
            let worker = v2.ok_or(RuntimeMemoryStoreError::Missing {
                entity: "stage_worker_runs",
            })?;
            validate_loaded_worker_identity(&worker, input, unit)?;
            Ok(LoadedWorkerCheckpointRow {
                source: RuntimeMemoryRecordSource::V2,
                worker,
            })
        }
    }
}

/// Select one complete worker checkpoint according to the operation-frozen
/// rollout contract. This never merges fields across V2 and legacy records.
pub async fn load_worker_checkpoint(
    pool: &sqlx::PgPool,
    input: &LoadWorkerCheckpointRow,
) -> RuntimeMemoryStoreResult<LoadedWorkerCheckpointRow> {
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    let contract = frozen_runtime_contract(&operation)?;
    if contract == runtime_memory_rollout::RuntimeMemoryContract::LegacyV1 {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "runtime_v2_not_enabled",
        });
    }
    let unit = load_runtime_unit_for_update(
        &mut tx,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
    )
    .await?;
    validate_runtime_stage_execution(
        &mut tx,
        &operation,
        input.stage_execution_id,
        &unit.stage_kind,
    )
    .await?;

    let loaded =
        select_worker_checkpoint_locked(&mut tx, &operation, contract, &unit, input).await?;
    tx.commit().await?;
    Ok(loaded)
}

/// Load one complete selected worker record together with the exact chain body
/// bound to it. Session/task/agent ownership is checked in the same transaction.
pub async fn load_bound_worker_chain(
    pool: &sqlx::PgPool,
    input: &LoadBoundWorkerChainRow,
) -> RuntimeMemoryStoreResult<LoadedBoundWorkerChainRow> {
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, OperationStateRow>(LOCK_OPERATION_STATE_ROW_SQL)
        .bind(input.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "operation_state",
        })?;
    ensure_runtime_operation_active(&operation)?;
    let contract = frozen_runtime_contract(&operation)?;
    if !contract_writes_v2(contract) {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "runtime_v2_not_enabled",
        });
    }
    let unit = load_runtime_unit_for_update(
        &mut tx,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
    )
    .await?;
    validate_runtime_stage_execution(
        &mut tx,
        &operation,
        input.stage_execution_id,
        &unit.stage_kind,
    )
    .await?;
    let checkpoint_input = LoadWorkerCheckpointRow {
        operation_id: input.operation_id,
        stage_execution_id: input.stage_execution_id,
        stage_run_unit_id: input.stage_run_unit_id,
        worker_run_id: input.worker_run_id,
        selected_source: input.selected_source,
    };
    let loaded =
        select_worker_checkpoint_locked(&mut tx, &operation, contract, &unit, &checkpoint_input)
            .await?;
    if loaded.worker.message_chain_id != Some(input.message_chain_id) {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "bound_worker_chain_identity_mismatch",
        });
    }
    let chain = sqlx::query_scalar::<_, Option<serde_json::Value>>(
        r#"SELECT chain FROM message_chains
            WHERE id=$1 AND session_id=$2 AND task_id=$3 AND agent=$4
            FOR SHARE"#,
    )
    .bind(input.message_chain_id)
    .bind(input.session_id)
    .bind(input.operation_id)
    .bind(input.agent)
    .fetch_optional(&mut *tx)
    .await?
    .flatten()
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "bound_message_chain",
    })?;
    if !chain.is_array() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_bound_chain_shape",
        });
    }
    tx.commit().await?;
    Ok(LoadedBoundWorkerChainRow {
        source: loaded.source,
        worker: loaded.worker,
        chain,
    })
}

#[cfg(test)]
async fn transition_stage_execution_with_injected_failure(
    pool: &sqlx::PgPool,
    input: &TransitionStageExecutionRow,
) -> RuntimeMemoryStoreResult<TransitionedStageExecutionRow> {
    transition_stage_execution_inner(pool, input, true).await
}

pub const LOCK_ORDER: &[&str] = &[
    "operation_state",
    "stage_runs",
    "operation_org_scope_snapshots",
    "operation_org_scope_units",
    "stage_run_units",
    "stage_worker_runs",
    "stage_deliverable_submissions",
    "org_stage_completions",
    "stage_handoffs",
];
pub const FINAL_SEAL_TABLES: &[&str] = &[
    "stage_deliverable_submissions",
    "stage_run_units",
    "org_stage_completions",
    "stage_handoffs",
];
pub const DUAL_WRITE_TABLES: &[&str] = &[
    "stage_worker_runs",
    "operation_state",
    "org_stage_completions",
];
pub const LOCK_OPERATION_SQL: &str =
    "SELECT operation_id FROM operation_state WHERE operation_id = $1 FOR UPDATE";
pub const LOCK_STAGE_UNIT_SQL: &str =
    "SELECT id FROM stage_run_units WHERE id = $1 AND operation_id = $2 FOR UPDATE";
pub const LOCK_WORKER_SQL: &str =
    "SELECT id FROM stage_worker_runs WHERE id = $1 AND stage_run_unit_id = $2 FOR UPDATE";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeMemoryTxFence {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub worker_run_id: Uuid,
    pub lease_token: Uuid,
    pub attempt_epoch: i64,
    pub expected_checkpoint_version: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMemoryTxOutcome {
    Applied,
    Replayed,
    Conflict,
}

impl RuntimeMemoryTxOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Replayed => "replayed",
            Self::Conflict => "conflict",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::NewSession;
    use crate::repo::{
        operation_org_scope, operation_state, organizations, project_scopes,
        runtime_memory_rollout, sessions, stage_runs, tasks,
    };
    use crate::{DbConfig, GolishDb};
    use serial_test::serial;
    use tempfile::TempDir;

    #[test]
    fn successor_turn_checkpoint_carries_exact_server_gap_manifest() {
        let gap_manifest = serde_json::json!({
            "reasons": ["retry exact origin"],
            "schema_version": 1,
        });
        let checkpoint = controller_turn_resume_checkpoint(
            &serde_json::json!({"chain": []}),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Some(Uuid::new_v4()),
            "repair-request",
            &format!("sha256:{}", "1".repeat(64)),
            &format!("sha256:{}", "2".repeat(64)),
            1,
            2,
            Some(&gap_manifest),
        )
        .expect("build successor checkpoint");

        assert_eq!(
            checkpoint.pointer("/_runtime_stage_team_turn_resume/source_gap_manifest"),
            Some(&gap_manifest)
        );
    }

    fn reserve_local_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("reserve local postgres port")
            .local_addr()
            .expect("read reserved postgres port")
            .port()
    }

    async fn fixture(label: &str) -> (GolishDb, TempDir) {
        let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
        let config = DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port(),
            database: format!("runtime_memory_store_{label}_{}", Uuid::new_v4().simple()),
            ..DbConfig::default()
        };
        let db = GolishDb::start(config)
            .await
            .expect("start migrated embedded postgres");
        (db, data_dir)
    }

    /// Full migrations leave both deployment defaults at rank-one sampling.
    /// These rollout-transition unit tests deliberately exercise the legacy
    /// predecessor, so their private database simulates a pre-cutover pair
    /// without weakening the production pair/promotion triggers.
    async fn reset_deployment_rollouts_to_legacy_for_test(db: &GolishDb) {
        let mut tx = db
            .pool()
            .begin()
            .await
            .expect("begin rollout reset fixture");
        sqlx::raw_sql(
            r#"SET LOCAL session_replication_role = 'replica';
               UPDATE runtime_memory_rollout
                  SET contract='legacy_v1', contract_rank=0, row_version=0,
                      updated_at=NOW()
                WHERE singleton_id=1;
               UPDATE attack_execution_rollout
                  SET contract='legacy', rank=0, row_version=0, updated_at=NOW()
                WHERE singleton=TRUE;
               SET LOCAL session_replication_role = 'origin';"#,
        )
        .execute(&mut *tx)
        .await
        .expect("reset deployment rollouts for predecessor-state unit fixture");
        tx.commit().await.expect("commit rollout reset fixture");
    }

    /// Position a private test database at a deployment state whose promotion
    /// semantics are covered by the dedicated retained-cohort suites.
    async fn set_deployment_rollouts_to_v2_only_for_test(db: &GolishDb) {
        let mut tx = db
            .pool()
            .begin()
            .await
            .expect("begin V2-only rollout fixture");
        sqlx::raw_sql(
            r#"SET LOCAL session_replication_role = 'replica';
               UPDATE runtime_memory_rollout
                  SET contract='v2_only', contract_rank=3, row_version=3,
                      updated_at=NOW()
                WHERE singleton_id=1;
               UPDATE attack_execution_rollout
                  SET contract='v2_only', rank=3, row_version=3, updated_at=NOW()
                WHERE singleton=TRUE;
               SET LOCAL session_replication_role = 'origin';"#,
        )
        .execute(&mut *tx)
        .await
        .expect("position deployment rollouts at V2-only");
        tx.commit().await.expect("commit V2-only rollout fixture");
    }

    async fn set_runtime_rollout_to_v2_preferred_for_test(db: &GolishDb) {
        let mut tx = db
            .pool()
            .begin()
            .await
            .expect("begin V2-preferred rollout fixture");
        sqlx::raw_sql(
            r#"SET LOCAL session_replication_role = 'replica';
               UPDATE runtime_memory_rollout
                  SET contract='dual_write_v2_preferred', contract_rank=2,
                      row_version=2, updated_at=NOW()
                WHERE singleton_id=1;
               SET LOCAL session_replication_role = 'origin';"#,
        )
        .execute(&mut *tx)
        .await
        .expect("position runtime rollout at V2-preferred");
        tx.commit()
            .await
            .expect("commit V2-preferred rollout fixture");
    }

    async fn create_session(db: &GolishDb) -> Uuid {
        sessions::create(
            db.pool(),
            NewSession {
                title: Some("runtime-memory fixture".to_string()),
                workspace_path: Some("/tmp/runtime-memory".to_string()),
                workspace_label: None,
                model: None,
                provider: None,
                project_path: Some("/tmp/runtime-memory".to_string()),
            },
        )
        .await
        .expect("create fixture session")
        .id
    }

    async fn create_operation_at(db: &GolishDb, label: &str, entry_stage: &str) -> (Uuid, Uuid) {
        let session_id = create_session(db).await;
        let scope = project_scopes::register_first_open(
            db.pool(),
            &format!("/tmp/runtime-memory-{label}"),
            &format!("sha-{label}"),
        )
        .await
        .expect("register transition project scope");
        let operation_id = Uuid::new_v4();
        let initial_stage_execution_id = Uuid::new_v4();
        create_runtime_operation(
            db.pool(),
            &CreateRuntimeOperationRow {
                operation_id,
                initial_stage_execution_id,
                session_id,
                title: Some(format!("transition {label}")),
                input: "transition runtime stage".to_string(),
                profile: "assessment".to_string(),
                entry_stage: entry_stage.to_string(),
                project_scope_id: scope.project_scope_id,
                cli_scope: None,
            },
        )
        .await
        .expect("create transition operation");
        (operation_id, initial_stage_execution_id)
    }

    #[test]
    fn runtime_memory_repo_contract_compound_transitions_have_one_lock_order() {
        assert_eq!(LOCK_ORDER.first(), Some(&"operation_state"));
        assert_eq!(LOCK_ORDER.last(), Some(&"stage_handoffs"));
        assert!(FINAL_SEAL_TABLES.contains(&"stage_run_units"));
        assert!(FINAL_SEAL_TABLES.contains(&"stage_handoffs"));
        assert!(DUAL_WRITE_TABLES.contains(&"org_stage_completions"));
        assert_eq!(RuntimeMemoryTxOutcome::Conflict.as_str(), "conflict");
        assert!(LOCK_OPERATION_SQL.ends_with("FOR UPDATE"));
        assert!(LOCK_STAGE_UNIT_SQL.contains("operation_id = $2"));
        assert!(LOCK_WORKER_SQL.contains("stage_run_unit_id = $2"));
    }

    #[test]
    fn v2_only_reset_removes_every_legacy_checkpoint_namespace() {
        let reset = v2_only_reset_state_blob(&serde_json::json!({
            "eas_web_transport_failures": {"slot": {"attempts": 2}},
            "graph_flow": {},
            "profile": "assessment",
            "current_stage": "target_intel",
            "current_stage_run_id": Uuid::new_v4(),
            "queue_titles": ["legacy"],
            "completed_count": 1,
            "continuity_adoption": {"legacy": true},
            "schema_v": 1,
            "stage_run_workers": {"legacy": true},
            "stage_run_handoffs": {"legacy": true},
            "agent_run": {"legacy": true}
        }));

        assert_eq!(reset["eas_web_transport_failures"]["slot"]["attempts"], 2);
        for forbidden in [
            "graph_flow",
            "profile",
            "current_stage",
            "current_stage_run_id",
            "queue_titles",
            "completed_count",
            "continuity_adoption",
            "schema_v",
            "stage_run_workers",
            "stage_run_handoffs",
            "agent_run",
        ] {
            assert!(
                reset.get(forbidden).is_none(),
                "V2-only reset retained {forbidden}: {reset}"
            );
        }
    }

    #[tokio::test]
    #[serial]
    async fn runtime_memory_store_create_operation_freezes_rollout_and_project_scope() {
        let (mut db, _data_dir) = fixture("freeze").await;
        reset_deployment_rollouts_to_legacy_for_test(&db).await;
        let session_id = create_session(&db).await;
        let scope = project_scopes::register_first_open(db.pool(), "/tmp/ws-a", "sha-ws-a")
            .await
            .expect("register project scope");
        runtime_memory_rollout::advance(
            db.pool(),
            runtime_memory_rollout::RuntimeMemoryContract::LegacyV1,
            runtime_memory_rollout::RuntimeMemoryContract::DualWriteLegacyRead,
            0,
        )
        .await
        .expect("advance rollout");

        let operation_id = Uuid::new_v4();
        let initial_stage_execution_id = Uuid::new_v4();
        let created = create_runtime_operation(
            db.pool(),
            &CreateRuntimeOperationRow {
                operation_id,
                initial_stage_execution_id,
                session_id,
                title: Some("atomic operation".to_string()),
                input: "inspect scope".to_string(),
                profile: "assessment".to_string(),
                entry_stage: "scoping".to_string(),
                project_scope_id: scope.project_scope_id,
                cli_scope: None,
            },
        )
        .await
        .expect("create task and operation atomically");

        assert_eq!(created.task.id, operation_id);
        assert_eq!(created.operation.operation_id, operation_id);
        assert_eq!(
            created.initial_stage_execution_id,
            initial_stage_execution_id
        );
        assert_eq!(
            created.operation.runtime_memory_contract,
            "dual_write_legacy_read"
        );
        assert_eq!(
            created.operation.project_scope_id,
            Some(scope.project_scope_id)
        );
        let initial_stage_runs = stage_runs::list_for_operation(db.pool(), operation_id)
            .await
            .expect("read initial stage execution");
        assert_eq!(
            initial_stage_runs.len(),
            1,
            "compound operation creation must open exactly one stage execution"
        );
        assert_eq!(initial_stage_runs[0].stage_kind, "scoping");
        assert_eq!(initial_stage_runs[0].status, "started");
        assert_eq!(initial_stage_runs[0].id, initial_stage_execution_id);

        set_runtime_rollout_to_v2_preferred_for_test(&db).await;
        let frozen = operation_state::get(db.pool(), operation_id)
            .await
            .expect("read operation")
            .expect("operation exists");
        assert_eq!(frozen.runtime_memory_contract, "dual_write_legacy_read");
        assert_eq!(frozen.project_scope_id, Some(scope.project_scope_id));

        db.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn fresh_runtime_operation_starts_first_durable_turn() {
        let (mut db, _data_dir) = fixture("initial-operation-turn").await;
        set_deployment_rollouts_to_v2_only_for_test(&db).await;
        let (operation_id, _stage_execution_id) =
            create_operation_at(&db, "initial-operation-turn", "scoping").await;

        let turn = sqlx::query_as::<
            _,
            (
                Uuid,
                i64,
                String,
                String,
                Option<chrono::DateTime<chrono::Utc>>,
            ),
        >(
            "SELECT operation_id,ordinal,trigger_input,status,terminal_at
               FROM operation_turns
              WHERE operation_id=$1",
        )
        .bind(operation_id)
        .fetch_one(db.pool())
        .await
        .expect("fresh operation owns one durable Turn");
        assert_eq!(turn.0, operation_id);
        assert_eq!(turn.1, 1);
        assert_eq!(turn.2, "transition runtime stage");
        assert_eq!(turn.3, "running");
        assert!(turn.4.is_none());

        db.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn cli_descendants_share_one_operation_and_snapshot() {
        let (mut db, _data_dir) = fixture("cli-one-operation").await;
        set_deployment_rollouts_to_v2_only_for_test(&db).await;
        let session_id = create_session(&db).await;
        let project_path = "/tmp/runtime-v2-cli-one-operation";
        let project_scope =
            project_scopes::register_first_open(db.pool(), project_path, "cli-scope-sha")
                .await
                .expect("register CLI project scope");
        let root = organizations::create(db.pool(), project_path, "Root", None, "", "")
            .await
            .expect("create root");
        let child = organizations::create(db.pool(), project_path, "Child", Some(root.id), "", "")
            .await
            .expect("create child");
        let grandchild = organizations::create(
            db.pool(),
            project_path,
            "Grandchild",
            Some(child.id),
            "",
            "",
        )
        .await
        .expect("create grandchild");
        let cli_unit =
            |organization_id,
             parent_organization_id,
             organization_name: &str,
             depth,
             ordinal,
             ownership_percent: Option<&str>| CliRuntimeScopeUnitRow {
                organization_id,
                parent_organization_id,
                organization_name: organization_name.to_string(),
                depth,
                ordinal,
                ownership_percent: ownership_percent.map(str::to_string),
                approval_source: serde_json::json!({"kind": "cli_flags"}),
            };
        let operation_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        let created = create_runtime_operation(
            db.pool(),
            &CreateRuntimeOperationRow {
                operation_id,
                initial_stage_execution_id: stage_execution_id,
                session_id,
                title: Some("one CLI operation".to_string()),
                input: "run one frozen fleet".to_string(),
                profile: "red_team".to_string(),
                entry_stage: "target_intel".to_string(),
                project_scope_id: project_scope.project_scope_id,
                cli_scope: Some(CliRuntimeScopeRow {
                    root_organization_id: root.id,
                    include_subsidiaries: true,
                    subsidiary_threshold: 51,
                    units: vec![
                        cli_unit(root.id, None, &root.name, 0, 0, None),
                        cli_unit(child.id, Some(root.id), &child.name, 1, 1, Some("75")),
                        cli_unit(
                            grandchild.id,
                            Some(child.id),
                            &grandchild.name,
                            2,
                            2,
                            Some("60"),
                        ),
                    ],
                }),
            },
        )
        .await
        .expect("create operation and freeze CLI snapshot atomically");
        assert_eq!(created.operation.runtime_memory_contract, "v2_only");
        assert_eq!(
            operation_state::get_attack_execution_contract(db.pool(), operation_id)
                .await
                .expect("read frozen attack execution contract")
                .map(|contract| contract.as_str()),
            Some("v2_only")
        );
        let seeded = seed_stage_runtime(
            db.pool(),
            &SeedStageRuntimeRow {
                operation_id,
                stage_execution_id,
                stage_kind: "target_intel".to_string(),
                unit_generation: 1,
                specialist: "recon".to_string(),
                worker_generation: 1,
                work_item_kind: "organization".to_string(),
                work_item_key: "target_intel".to_string(),
                agent_path_prefix: "main>stage_run:target_intel".to_string(),
                organization_ids: None,
            },
        )
        .await
        .expect("seed all frozen stage units");

        let tasks = tasks::list_by_session(db.pool(), session_id)
            .await
            .expect("list CLI operations");
        let snapshot = operation_org_scope::load_for_operation(db.pool(), operation_id)
            .await
            .expect("load CLI snapshot")
            .expect("CLI snapshot exists");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, operation_id);
        assert_eq!(snapshot.units.len(), 3);
        assert_eq!(seeded.len(), 3);
        assert!(seeded.iter().all(|runtime| {
            runtime.unit.operation_id == operation_id
                && runtime.worker.operation_id == operation_id
                && runtime.unit.scope_snapshot_id == snapshot.snapshot.id
        }));

        let mut claimed = Vec::with_capacity(seeded.len());
        for (index, runtime) in seeded.iter().enumerate() {
            claimed.push(
                claim_worker_and_bind_chain(
                    db.pool(),
                    &ClaimWorkerAndBindChainRow {
                        operation_id,
                        stage_execution_id,
                        stage_run_unit_id: runtime.unit.id,
                        worker_run_id: runtime.worker.id,
                        expected_unit_status: stage_run_units::StageRunUnitStatus::Queued,
                        expected_unit_row_version: runtime.unit.row_version,
                        expected_worker_status: stage_worker_runs::StageWorkerRunStatus::Queued,
                        expected_attempt_epoch: runtime.worker.attempt_epoch,
                        session_id,
                        subtask_id: None,
                        agent: AgentType::Pentester,
                        model: None,
                        provider: None,
                        parent_chain_id: None,
                        lease_owner: format!("runtime-{index}"),
                        lease_seconds: 3_600,
                        initial_chain: serde_json::json!([]),
                        initial_checkpoint: serde_json::json!({"turn": 0}),
                    },
                )
                .await
                .expect("claim worker and bind exact chain"),
            );
        }
        for worker in [&claimed[0].worker, &claimed[1].worker] {
            sqlx::query(
                r#"UPDATE stage_worker_runs
                      SET lease_owner='dead-runtime',
                          lease_acquired_at=NOW()-INTERVAL '2 hours',
                          lease_expires_at=NOW()-INTERVAL '1 hour',
                          heartbeat_at=NOW()-INTERVAL '1 hour',
                          started_at=NOW()-INTERVAL '2 hours'
                    WHERE id=$1 AND lease_token=$2"#,
            )
            .bind(worker.id)
            .bind(worker.lease_token.expect("claimed lease token"))
            .execute(db.pool())
            .await
            .expect("age claimed worker lease");
        }
        let active_tool_lease = claimed[1]
            .worker
            .lease_token
            .expect("active-tool worker lease token");
        let live_lease = claimed[2]
            .worker
            .lease_token
            .expect("live worker lease token");
        let active_tool_id = crate::repo::tool_calls::record_tracked_start(
            db.pool(),
            "startup-reaper-active-tool",
            session_id,
            Some(operation_id),
            None,
            "eas_discover_ports",
            &serde_json::json!({}),
            Some(&crate::repo::tool_calls::RuntimeToolIdentity {
                operation_id,
                stage_execution_id,
                stage_run_unit_id: Some(seeded[1].unit.id),
                worker_run_id: Some(seeded[1].worker.id),
                organization_id: Some(seeded[1].unit.organization_id),
                attempt_epoch: Some(claimed[1].worker.attempt_epoch),
                lease_token: Some(active_tool_lease),
            }),
        )
        .await
        .expect("record active worker tool");
        sqlx::query(
            r#"UPDATE stage_worker_runs
                  SET active_tool_call_id=$2,
                      active_tool_started_at=NOW()-INTERVAL '90 minutes'
                WHERE id=$1"#,
        )
        .bind(seeded[1].worker.id)
        .bind(active_tool_id)
        .execute(db.pool())
        .await
        .expect("bind active tool to expired worker");
        sqlx::query(
            r#"UPDATE tasks
                  SET status='running', updated_at=NOW()-INTERVAL '7 hours'
                WHERE id=$1"#,
        )
        .bind(operation_id)
        .execute(db.pool())
        .await
        .expect("age abandoned V2 task");

        let reaped = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
            .await
            .expect("reap expired V2 workers while preserving live lease");
        assert_eq!(reaped.workers_requeued, 1);
        assert_eq!(reaped.workers_recovery_required, 1);
        assert_eq!(reaped.paused, 0);
        assert_eq!(reaped.failed, 0);
        assert_eq!(
            tasks::get(db.pool(), operation_id)
                .await
                .expect("load live-lease task")
                .expect("live-lease task exists")
                .status,
            crate::models::TaskStatus::Running,
            "a valid live lease must keep the task running"
        );
        sqlx::query(
            r#"UPDATE stage_worker_runs
                  SET status='queued', lease_token=NULL, lease_owner=NULL,
                      lease_acquired_at=NULL, lease_expires_at=NULL,
                      heartbeat_at=NULL
                WHERE id=$1 AND lease_token=$2"#,
        )
        .bind(seeded[2].worker.id)
        .bind(live_lease)
        .execute(db.pool())
        .await
        .expect("release live lease fixture");
        let paused = tasks::startup_reap_abandoned(db.pool(), chrono::Duration::zero())
            .await
            .expect("pause V2 task after the last live lease exits");
        assert_eq!(paused.workers_requeued, 0);
        assert_eq!(paused.workers_recovery_required, 0);
        assert_eq!(paused.paused, 1);
        assert_eq!(paused.failed, 0);
        let requeued_worker = stage_worker_runs::get(db.pool(), seeded[0].worker.id)
            .await
            .expect("load requeued worker")
            .expect("requeued worker exists");
        let recovery_worker = stage_worker_runs::get(db.pool(), seeded[1].worker.id)
            .await
            .expect("load recovery worker")
            .expect("recovery worker exists");
        assert_eq!(requeued_worker.status, "queued");
        assert!(requeued_worker.lease_token.is_none());
        assert_eq!(recovery_worker.status, "recovery_required");
        assert_eq!(recovery_worker.active_tool_call_id, Some(active_tool_id));
        assert_eq!(
            tasks::get(db.pool(), operation_id)
                .await
                .expect("load paused task")
                .expect("paused task exists")
                .status,
            crate::models::TaskStatus::Waiting
        );
        assert_eq!(
            tasks::latest_resumable_by_session(db.pool(), session_id)
                .await
                .expect("select relational V2 resume target")
                .map(|task| task.id),
            Some(operation_id)
        );
        sqlx::query("UPDATE tool_calls SET status='finished' WHERE id=$1")
            .bind(active_tool_id)
            .execute(db.pool())
            .await
            .expect("make recovery-required active-tool marker stale");
        assert!(
            tasks::latest_resumable_by_session(db.pool(), session_id)
                .await
                .expect("reject stale active-tool recovery identity")
                .is_none(),
            "recovery_required must reference the exact still-active fenced tool"
        );
        sqlx::query("UPDATE tool_calls SET status='running' WHERE id=$1")
            .bind(active_tool_id)
            .execute(db.pool())
            .await
            .expect("restore active tool fixture");

        let replacement_stage_execution_id = Uuid::new_v4();
        let reset = supersede_stage_checkpoint(
            db.pool(),
            &SupersedeStageCheckpointRow {
                operation_id,
                expected_current_stage: "target_intel".to_string(),
                selected_stage: "target_intel".to_string(),
                affected_stage_kinds: vec!["target_intel".to_string()],
                next_state_blob: serde_json::json!({
                    "graph_flow": {"next_node": "target_intel", "state": {}}
                }),
                replacement_specialist: Some("recon".to_string()),
                replacement_stage_execution_id: Some(replacement_stage_execution_id),
            },
        )
        .await
        .expect("atomically supersede CLI runtime");
        assert_eq!(reset.workers_superseded, 3);
        assert_eq!(reset.units_superseded, 3);
        assert_eq!(reset.executions_superseded, 1);
        let old_execution = stage_runs::get(db.pool(), stage_execution_id)
            .await
            .expect("load superseded-compatible execution")
            .expect("old execution exists");
        let new_execution = stage_runs::get(db.pool(), replacement_stage_execution_id)
            .await
            .expect("load replacement execution")
            .expect("replacement exists");
        assert_eq!(old_execution.status, "failed");
        assert_eq!(new_execution.status, "started");
        let reset_operation = operation_state::get(db.pool(), operation_id)
            .await
            .expect("load reset operation")
            .expect("reset operation exists");
        assert_eq!(reset_operation.current_stage, "target_intel");
        assert_eq!(
            reset_operation.state_blob["runtime_v2_dev_reset"]["semantic_stage_execution_status"],
            "superseded"
        );
        let superseded_units = crate::repo::stage_run_units::list_for_execution(
            db.pool(),
            operation_id,
            stage_execution_id,
        )
        .await
        .expect("load superseded units");
        assert!(superseded_units
            .iter()
            .all(|unit| unit.status == "superseded"));
        for runtime in &seeded {
            let worker = stage_worker_runs::get(db.pool(), runtime.worker.id)
                .await
                .expect("load superseded worker")
                .expect("worker exists");
            assert_eq!(worker.status, "superseded");
            assert!(worker.lease_token.is_none());
        }
        let replacement_units = crate::repo::stage_run_units::list_for_execution(
            db.pool(),
            operation_id,
            replacement_stage_execution_id,
        )
        .await
        .expect("load replacement units");
        assert_eq!(replacement_units.len(), 3);
        assert!(replacement_units.iter().all(|unit| unit.status == "queued"));
        assert_eq!(
            tasks::latest_resumable_by_session(db.pool(), session_id)
                .await
                .expect("select reset relational V2 target")
                .map(|task| task.id),
            Some(operation_id),
            "compound reset must leave plan-first replacement units for Team seed"
        );

        let root_only_execution_id = Uuid::new_v4();
        let root_only_reset = supersede_stage_checkpoint(
            db.pool(),
            &SupersedeStageCheckpointRow {
                operation_id,
                expected_current_stage: "target_intel".to_string(),
                selected_stage: "reporting".to_string(),
                affected_stage_kinds: vec!["target_intel".to_string(), "reporting".to_string()],
                next_state_blob: serde_json::json!({
                    "graph_flow": {"next_node": "reporting", "state": {}}
                }),
                replacement_specialist: None,
                replacement_stage_execution_id: Some(root_only_execution_id),
            },
        )
        .await
        .expect("atomically reset to a root-only stage");
        assert_eq!(root_only_reset.workers_superseded, 0);
        assert_eq!(root_only_reset.units_superseded, 3);
        let root_only_units = crate::repo::stage_run_units::list_for_execution(
            db.pool(),
            operation_id,
            root_only_execution_id,
        )
        .await
        .expect("load root-only replacement unit");
        let root_only_workers =
            stage_worker_runs::list_for_execution(db.pool(), operation_id, root_only_execution_id)
                .await
                .expect("load root-only replacement workers");
        assert_eq!(root_only_units.len(), 1);
        assert_eq!(root_only_units[0].organization_id, root.id);
        assert!(root_only_units[0].specialist.is_none());
        assert!(root_only_workers.is_empty());
        assert_eq!(
            tasks::latest_resumable_by_session(db.pool(), session_id)
                .await
                .expect("select root-only relational V2 target")
                .map(|task| task.id),
            Some(operation_id),
            "a non-specialist stage owns one root unit even when scope has descendants"
        );

        let scoping_session_id = create_session(&db).await;
        let scoping_project = project_scopes::register_first_open(
            db.pool(),
            "/tmp/runtime-v2-scoping-prefreeze",
            "scoping-prefreeze-sha",
        )
        .await
        .expect("register Scoping pre-freeze scope");
        let scoping_operation_id = Uuid::new_v4();
        create_runtime_operation(
            db.pool(),
            &CreateRuntimeOperationRow {
                operation_id: scoping_operation_id,
                initial_stage_execution_id: Uuid::new_v4(),
                session_id: scoping_session_id,
                title: Some("V2 Scoping pre-freeze".to_string()),
                input: "resolve scope".to_string(),
                profile: "red_team".to_string(),
                entry_stage: "scoping".to_string(),
                project_scope_id: scoping_project.project_scope_id,
                cli_scope: None,
            },
        )
        .await
        .expect("create V2 Scoping pre-freeze operation");
        tasks::update_status(
            db.pool(),
            scoping_operation_id,
            crate::models::TaskStatus::Waiting,
        )
        .await
        .expect("pause Scoping pre-freeze task");
        assert_eq!(
            tasks::latest_resumable_by_session(db.pool(), scoping_session_id)
                .await
                .expect("select V2 Scoping pre-freeze")
                .map(|task| task.id),
            Some(scoping_operation_id)
        );
        stage_runs::insert(db.pool(), Uuid::new_v4(), scoping_operation_id, "scoping")
            .await
            .expect("inject duplicate active Scoping execution");
        assert!(
            tasks::latest_resumable_by_session(db.pool(), scoping_session_id)
                .await
                .expect("reject duplicate-active V2 Scoping")
                .is_none(),
            "malformed active-execution cardinality must fail closed"
        );
        db.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn runtime_memory_store_task_insert_rolls_back_when_operation_insert_fails() {
        let (mut db, _data_dir) = fixture("rollback").await;
        let session_id = create_session(&db).await;
        let scope = project_scopes::register_first_open(db.pool(), "/tmp/ws-b", "sha-ws-b")
            .await
            .expect("register project scope");
        let operation_id = Uuid::new_v4();
        let initial_stage_execution_id = Uuid::new_v4();
        operation_state::insert(db.pool(), operation_id, "preexisting", "scoping", "v2_only")
            .await
            .expect("seed duplicate operation identity");

        let result = create_runtime_operation(
            db.pool(),
            &CreateRuntimeOperationRow {
                operation_id,
                initial_stage_execution_id,
                session_id,
                title: None,
                input: "must roll back".to_string(),
                profile: "assessment".to_string(),
                entry_stage: "scoping".to_string(),
                project_scope_id: scope.project_scope_id,
                cli_scope: None,
            },
        )
        .await;
        assert!(
            result.is_err(),
            "duplicate operation must abort the transaction"
        );
        assert!(
            tasks::get(db.pool(), operation_id)
                .await
                .expect("read rolled-back task")
                .is_none(),
            "task insert must roll back with operation insert"
        );
        let preexisting = operation_state::get(db.pool(), operation_id)
            .await
            .expect("read preexisting operation")
            .expect("preexisting operation remains");
        assert_eq!(preexisting.profile, "preexisting");
        assert_eq!(preexisting.project_scope_id, None);

        db.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn runtime_memory_store_stage_transition_is_atomic_and_has_one_active_execution() {
        let (mut db, _data_dir) = fixture("stage-transition").await;
        set_deployment_rollouts_to_v2_only_for_test(&db).await;
        let (operation_id, initial_stage_execution_id) =
            create_operation_at(&db, "stage-transition", "scoping").await;
        let next_stage_execution_id = Uuid::new_v4();

        let transitioned = transition_stage_execution(
            db.pool(),
            &TransitionStageExecutionRow {
                operation_id,
                current_stage_execution_id: initial_stage_execution_id,
                next_stage_execution_id,
                next_stage: "target_intel".to_string(),
            },
        )
        .await
        .expect("transition exact active stage execution");

        assert_eq!(
            transitioned.previous_stage_execution.id,
            initial_stage_execution_id
        );
        assert_eq!(transitioned.previous_stage_execution.status, "completed");
        assert_eq!(
            transitioned.current_stage_execution.id,
            next_stage_execution_id
        );
        assert_eq!(transitioned.current_stage_execution.status, "started");
        assert_eq!(transitioned.operation.current_stage, "target_intel");
        assert_eq!(transitioned.operation.runtime_memory_contract, "v2_only");
        assert!(
            transitioned
                .operation
                .state_blob
                .get("current_stage")
                .is_none()
                && transitioned
                    .operation
                    .state_blob
                    .get("current_stage_run_id")
                    .is_none(),
            "V2-only transition must not recreate legacy checkpoint fields"
        );
        assert_eq!(
            transitioned.operation.stage_started_at,
            transitioned.current_stage_execution.started_at,
            "cursor and stage execution timestamps must come from one transaction"
        );

        let rows = stage_runs::list_for_operation(db.pool(), operation_id)
            .await
            .expect("list transitioned stage executions");
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter().filter(|row| row.status == "started").count(),
            1,
            "transition must leave exactly one active execution"
        );

        let stale = transition_stage_execution(
            db.pool(),
            &TransitionStageExecutionRow {
                operation_id,
                current_stage_execution_id: initial_stage_execution_id,
                next_stage_execution_id: Uuid::new_v4(),
                next_stage: "external_attack_surface".to_string(),
            },
        )
        .await;
        assert!(matches!(
            stale,
            Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "active_stage_execution_mismatch"
            })
        ));

        db.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn runtime_memory_store_terminal_stage_completion_closes_exact_execution_and_task_atomically(
    ) {
        let (mut db, _data_dir) = fixture("terminal-stage-completion").await;
        let (operation_id, stage_execution_id) =
            create_operation_at(&db, "terminal-stage-completion", "attack_candidate").await;
        tasks::update_status(db.pool(), operation_id, crate::models::TaskStatus::Running)
            .await
            .expect("mark terminal fixture task running");

        let completed = complete_terminal_stage_execution(
            db.pool(),
            &CompleteTerminalStageExecutionRow {
                operation_id,
                current_stage_execution_id: stage_execution_id,
                terminal_stage: "attack_candidate".to_string(),
                task_result: "Candidate slice complete".to_string(),
            },
        )
        .await
        .expect("atomically complete exact terminal execution and task");

        assert_eq!(completed.id, stage_execution_id);
        assert_eq!(completed.operation_id, operation_id);
        assert_eq!(completed.stage_kind, "attack_candidate");
        assert_eq!(completed.status, "completed");
        assert!(completed.completed_at.is_some());
        assert!(
            stage_runs::get_exact_active_for_operation(db.pool(), operation_id)
                .await
                .is_err()
        );
        let task = tasks::get(db.pool(), operation_id)
            .await
            .expect("load completed task")
            .expect("completed task exists");
        assert_eq!(task.status, crate::models::TaskStatus::Finished);
        assert_eq!(task.result.as_deref(), Some("Candidate slice complete"));
        let operation = operation_state::get(db.pool(), operation_id)
            .await
            .expect("load terminal operation")
            .expect("terminal operation exists");
        assert_eq!(operation.current_stage, "attack_candidate");

        let replay = complete_terminal_stage_execution(
            db.pool(),
            &CompleteTerminalStageExecutionRow {
                operation_id,
                current_stage_execution_id: stage_execution_id,
                terminal_stage: "attack_candidate".to_string(),
                task_result: "Candidate slice complete".to_string(),
            },
        )
        .await
        .expect("exact response-loss replay returns the same terminal row");
        assert_eq!(replay.id, completed.id);
        assert_eq!(replay.completed_at, completed.completed_at);

        db.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn runtime_memory_store_terminal_stage_completion_rolls_back_both_rows_on_failure() {
        let (mut db, _data_dir) = fixture("terminal-stage-completion-rollback").await;
        let (operation_id, stage_execution_id) = create_operation_at(
            &db,
            "terminal-stage-completion-rollback",
            "attack_candidate",
        )
        .await;
        tasks::update_status(db.pool(), operation_id, crate::models::TaskStatus::Running)
            .await
            .expect("mark rollback fixture task running");

        let result = complete_terminal_stage_execution_inner(
            db.pool(),
            &CompleteTerminalStageExecutionRow {
                operation_id,
                current_stage_execution_id: stage_execution_id,
                terminal_stage: "attack_candidate".to_string(),
                task_result: "must roll back".to_string(),
            },
            true,
        )
        .await;
        assert!(matches!(
            result,
            Err(RuntimeMemoryStoreError::Conflict {
                code: "injected_after_terminal_stage_close"
            })
        ));

        let active = stage_runs::get_exact_active_for_operation(db.pool(), operation_id)
            .await
            .expect("stage close must roll back");
        assert_eq!(active.id, stage_execution_id);
        assert_eq!(active.status, "started");
        let task = tasks::get(db.pool(), operation_id)
            .await
            .expect("load rolled-back task")
            .expect("rolled-back task exists");
        assert_eq!(task.status, crate::models::TaskStatus::Running);
        assert!(task.result.is_none());

        db.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn runtime_memory_store_active_stage_read_requires_exactly_one_started_execution() {
        let (mut db, _data_dir) = fixture("active-stage-exact").await;
        let (operation_id, initial_stage_execution_id) =
            create_operation_at(&db, "active-stage-exact", "scoping").await;

        let active = stage_runs::get_exact_active_for_operation(db.pool(), operation_id)
            .await
            .expect("read exact active stage execution");
        assert_eq!(active.id, initial_stage_execution_id);
        assert_eq!(active.operation_id, operation_id);
        assert_eq!(active.stage_kind, "scoping");
        assert_eq!(active.status, "started");

        stage_runs::insert(db.pool(), Uuid::new_v4(), operation_id, "scoping")
            .await
            .expect("seed legacy duplicate active execution");
        let duplicate = stage_runs::get_exact_active_for_operation(db.pool(), operation_id).await;
        assert!(matches!(
            duplicate,
            Err(RuntimeMemoryStoreError::Conflict {
                code: "multiple_active_stage_executions"
            })
        ));

        db.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn runtime_memory_store_initial_stage_insert_failure_rolls_back_task_and_operation() {
        let (mut db, _data_dir) = fixture("initial-stage-rollback").await;
        let session_id = create_session(&db).await;
        let scope = project_scopes::register_first_open(
            db.pool(),
            "/tmp/runtime-memory-initial-stage-rollback",
            "sha-initial-stage-rollback",
        )
        .await
        .expect("register rollback project scope");
        let occupied_operation_id = Uuid::new_v4();
        operation_state::insert(
            db.pool(),
            occupied_operation_id,
            "legacy",
            "scoping",
            "v2_only",
        )
        .await
        .expect("insert operation owning occupied stage identity");
        let occupied_stage_execution_id = Uuid::new_v4();
        stage_runs::insert(
            db.pool(),
            occupied_stage_execution_id,
            occupied_operation_id,
            "scoping",
        )
        .await
        .expect("occupy initial stage execution identity");

        let operation_id = Uuid::new_v4();
        let result = create_runtime_operation(
            db.pool(),
            &CreateRuntimeOperationRow {
                operation_id,
                initial_stage_execution_id: occupied_stage_execution_id,
                session_id,
                title: Some("must roll back at stage insert".to_string()),
                input: "rollback all operation roots".to_string(),
                profile: "assessment".to_string(),
                entry_stage: "scoping".to_string(),
                project_scope_id: scope.project_scope_id,
                cli_scope: None,
            },
        )
        .await;
        assert!(
            result.is_err(),
            "duplicate stage identity must abort create"
        );
        assert!(tasks::get(db.pool(), operation_id)
            .await
            .expect("read rolled-back task")
            .is_none());
        assert!(operation_state::get(db.pool(), operation_id)
            .await
            .expect("read rolled-back operation")
            .is_none());
        let occupied = stage_runs::get(db.pool(), occupied_stage_execution_id)
            .await
            .expect("read occupied stage execution")
            .expect("occupied stage execution remains");
        assert_eq!(occupied.operation_id, occupied_operation_id);

        db.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn runtime_memory_store_stage_transition_rolls_back_after_injected_insert_failure() {
        let (mut db, _data_dir) = fixture("stage-transition-rollback").await;
        let (operation_id, initial_stage_execution_id) =
            create_operation_at(&db, "stage-transition-rollback", "scoping").await;
        let rejected_stage_execution_id = Uuid::new_v4();

        let result = transition_stage_execution_with_injected_failure(
            db.pool(),
            &TransitionStageExecutionRow {
                operation_id,
                current_stage_execution_id: initial_stage_execution_id,
                next_stage_execution_id: rejected_stage_execution_id,
                next_stage: "target_intel".to_string(),
            },
        )
        .await;
        assert!(matches!(
            result,
            Err(RuntimeMemoryStoreError::Conflict {
                code: "injected_after_new_stage_execution"
            })
        ));

        let operation = operation_state::get(db.pool(), operation_id)
            .await
            .expect("read operation after rollback")
            .expect("operation remains");
        assert_eq!(operation.current_stage, "scoping");
        let rows = stage_runs::list_for_operation(db.pool(), operation_id)
            .await
            .expect("list stage executions after rollback");
        assert_eq!(rows.len(), 1, "new stage execution insert must roll back");
        assert_eq!(rows[0].id, initial_stage_execution_id);
        assert_eq!(rows[0].status, "started");
        assert!(stage_runs::get(db.pool(), rejected_stage_execution_id)
            .await
            .expect("read rejected stage execution")
            .is_none());

        db.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn runtime_memory_store_stage_transition_fails_closed_on_multiple_active_rows() {
        let (mut db, _data_dir) = fixture("stage-transition-duplicate").await;
        let (operation_id, initial_stage_execution_id) =
            create_operation_at(&db, "stage-transition-duplicate", "scoping").await;
        stage_runs::insert(db.pool(), Uuid::new_v4(), operation_id, "scoping")
            .await
            .expect("seed legacy duplicate active execution");

        let result = transition_stage_execution(
            db.pool(),
            &TransitionStageExecutionRow {
                operation_id,
                current_stage_execution_id: initial_stage_execution_id,
                next_stage_execution_id: Uuid::new_v4(),
                next_stage: "target_intel".to_string(),
            },
        )
        .await;
        assert!(matches!(
            result,
            Err(RuntimeMemoryStoreError::Conflict {
                code: "multiple_active_stage_executions"
            })
        ));
        let operation = operation_state::get(db.pool(), operation_id)
            .await
            .expect("read fail-closed operation")
            .expect("operation remains");
        assert_eq!(operation.current_stage, "scoping");
        assert_eq!(
            stage_runs::list_for_operation(db.pool(), operation_id)
                .await
                .expect("list fail-closed active executions")
                .iter()
                .filter(|row| row.status == "started")
                .count(),
            2,
            "transition must not mutate pre-existing duplicate legacy rows"
        );

        db.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn runtime_memory_store_rollout_rejects_skip_downgrade_and_stale_version() {
        let (mut db, _data_dir) = fixture("rollout").await;
        reset_deployment_rollouts_to_legacy_for_test(&db).await;
        let skip = runtime_memory_rollout::advance(
            db.pool(),
            runtime_memory_rollout::RuntimeMemoryContract::LegacyV1,
            runtime_memory_rollout::RuntimeMemoryContract::V2Only,
            0,
        )
        .await;
        assert!(matches!(
            skip,
            Err(RuntimeMemoryStoreError::InvalidContractTransition { .. })
        ));

        runtime_memory_rollout::advance(
            db.pool(),
            runtime_memory_rollout::RuntimeMemoryContract::LegacyV1,
            runtime_memory_rollout::RuntimeMemoryContract::DualWriteLegacyRead,
            0,
        )
        .await
        .expect("adjacent rollout succeeds");
        let stale = runtime_memory_rollout::advance(
            db.pool(),
            runtime_memory_rollout::RuntimeMemoryContract::DualWriteLegacyRead,
            runtime_memory_rollout::RuntimeMemoryContract::DualWriteV2Preferred,
            0,
        )
        .await;
        assert!(matches!(
            stale,
            Err(RuntimeMemoryStoreError::StaleVersion {
                entity: "runtime_memory_rollout",
                expected: 0,
                actual: 1,
            })
        ));
        let downgrade = runtime_memory_rollout::advance(
            db.pool(),
            runtime_memory_rollout::RuntimeMemoryContract::DualWriteLegacyRead,
            runtime_memory_rollout::RuntimeMemoryContract::LegacyV1,
            1,
        )
        .await;
        assert!(matches!(
            downgrade,
            Err(RuntimeMemoryStoreError::InvalidContractTransition { .. })
        ));

        db.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn runtime_memory_store_project_scope_first_open_and_rename_use_identity_cas() {
        let (mut db, _data_dir) = fixture("project_scope").await;
        let first = project_scopes::register_first_open(db.pool(), "/tmp/ws-c", "sha-ws-c")
            .await
            .expect("register first open");
        let replay = project_scopes::register_first_open(db.pool(), "/tmp/ws-c", "sha-ws-c")
            .await
            .expect("replay first open");
        assert_eq!(first.project_scope_id, replay.project_scope_id);
        assert_eq!(replay.row_version, 0);

        let mismatch =
            project_scopes::register_first_open(db.pool(), "/tmp/ws-c", "wrong-sha").await;
        assert!(matches!(
            mismatch,
            Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "project_scope_path_hash_mismatch"
            })
        ));

        let renamed = project_scopes::rename(
            db.pool(),
            first.project_scope_id,
            "/tmp/ws-c",
            0,
            "/tmp/ws-c-renamed",
            "sha-ws-c-renamed",
        )
        .await
        .expect("rename with current identity and version");
        assert_eq!(renamed.project_scope_id, first.project_scope_id);
        assert_eq!(renamed.canonical_project_path, "/tmp/ws-c-renamed");
        assert_eq!(renamed.row_version, 1);

        let stale = project_scopes::rename(
            db.pool(),
            first.project_scope_id,
            "/tmp/ws-c-renamed",
            0,
            "/tmp/ws-c-second",
            "sha-ws-c-second",
        )
        .await;
        assert!(matches!(
            stale,
            Err(RuntimeMemoryStoreError::StaleVersion {
                entity: "project_scopes",
                expected: 0,
                actual: 1,
            })
        ));

        db.stop().await;
    }
}
