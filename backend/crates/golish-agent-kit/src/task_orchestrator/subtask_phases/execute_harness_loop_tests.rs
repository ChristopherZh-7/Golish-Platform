//! Tests for harness post-gate handling + Scoping-only routine human confirmation.
//!
//! Exercises [`TaskOrchestrator::consume_gate_outcome`] (the post-gate chokepoint
//! that records the cross-stage handoff + emits `stage_passed` + accumulates the
//! stage flow outcome) and [`TaskOrchestrator::two_level_phase_gate`] (the live
//! graph-flow transition gate: typed security barriers stay fail-closed while
//! post-Scoping routine phase crossings auto-advance), against an in-memory repo.
//! Plus P3 RAG-prior wiring and
//!   `fail_task_if_active`. Deterministic regardless of `GOLISH_HARNESS_PROFILE`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::db_traits::*;
use crate::harness::graph_engine::{Checkpointer, Executor, RunOutcome};
use crate::harness::operation_flow::{build_operation_flow_graph, OperationFlowState};
use crate::harness::StageKind;
use crate::task_orchestrator::{
    AgentExecutor, AgentResult, ExecutionContext, PlannedSubtask, TaskOrchestrator,
};
use golish_core::events::AiEvent;

use super::HarnessGateOutcome;

const COMPANY_PARITY_FIXTURE: &str = "广州有创网络科技有限公司";
const COMPANY_PARITY_TARGET: &str = "http://127.0.0.1:18080";

/// In-memory [`DbRepoProvider`]: only the `operation_state_*` trio is real; the
/// transition driver touches nothing else, so every other method is a stub.
struct MemRepo {
    op_state: Mutex<HashMap<Uuid, OperationStateView>>,
    active_stage_executions:
        Mutex<HashMap<Uuid, crate::task_orchestrator::stage_execution::StageExecution>>,
    completed_stage_executions:
        Mutex<HashMap<Uuid, crate::task_orchestrator::stage_execution::StageExecution>>,
    fail_operation_insert: AtomicBool,
    fail_stage_transition: AtomicBool,
    stage_transition_count: Mutex<usize>,
    /// Canned `wiki_search_fts` result for the P3 RAG-prior wiring tests.
    /// Defaults to `Null` (no hits); the transition-driver tests never read it.
    wiki_result: Mutex<serde_json::Value>,
    wiki_search_calls: AtomicUsize,
    /// In-memory `tasks` table for the P1 `fail_task_if_active` tests.
    tasks: Mutex<HashMap<Uuid, TaskView>>,
    submissions: Mutex<HashMap<Uuid, PersistedStageDeliverableSubmission>>,
    attack_execution_contract: Mutex<golish_core::AttackExecutionContract>,
    candidate_review_barrier: Mutex<Option<AttackV2ReviewBarrierView>>,
    candidate_review_reads: AtomicUsize,
    verification_truth: Mutex<Option<crate::harness::attack_execution::VerificationTruthSet>>,
    wave_consolidation: Mutex<Option<AttackV2WaveConsolidationView>>,
    wave_consolidation_calls: AtomicUsize,
    fail_wave_consolidation: AtomicBool,
    reporting_truth: Mutex<Option<crate::harness::ReportingGateTruth>>,
    reporting_build_calls: AtomicUsize,
    reporting_gate_reads: AtomicUsize,
    state_blob_write_count: AtomicUsize,
    scoping_target_snapshots: Mutex<HashMap<Uuid, Vec<ScopingReviewedTarget>>>,
    scoping_target_snapshot_reads: AtomicUsize,
    fail_scoping_target_snapshot: AtomicBool,
    active_recon_scope_candidates: Mutex<HashMap<(Uuid, Uuid), Vec<ScopingReviewedTarget>>>,
    active_recon_scope_approvals: Mutex<HashMap<(Uuid, Uuid), Vec<ScopingReviewedTarget>>>,
    active_recon_scope_apply_calls: AtomicUsize,
    scoping_finalizations: Mutex<HashMap<Uuid, (FinalizeScopingScope, FinalizedScopingScope)>>,
    scoping_finalize_calls: AtomicUsize,
    scoping_finalize_replays: AtomicUsize,
    fail_scoping_finalize: AtomicBool,
    mismatch_scoping_finalize_identity: AtomicBool,
}

struct ExhaustedStageRunExecutor {
    execute_calls: AtomicUsize,
}

struct ReportingStageExecutor {
    execute_calls: AtomicUsize,
}

#[async_trait]
impl AgentExecutor for ReportingStageExecutor {
    async fn execute_subtask(
        &self,
        _subtask_title: &str,
        _subtask_description: &str,
        _execution_context: &ExecutionContext,
        _agent_type: Option<&str>,
    ) -> anyhow::Result<AgentResult> {
        self.execute_calls.fetch_add(1, Ordering::SeqCst);
        let stage_run_id = Uuid::new_v4();
        Ok(AgentResult::new(format!(
            "```json\n{{\"stage_id\":\"reporting\",\"stage_run_id\":\"{stage_run_id}\",\"claims\":[{{\"kind\":\"report_read_model_ready\",\"subject\":\"canonical_report\",\"summary\":\"Server-built canonical report revision is ready for deterministic Gate evaluation.\",\"evidence_ids\":[],\"technique\":null}}],\"evidence_refs\":[],\"skipped_checks\":[],\"findings\":[],\"required_checks_done\":[],\"coverage\":[],\"candidates\":[],\"candidate_decisions\":[]}}\n```"
        )))
    }

    async fn generate_report(
        &self,
        _execution_context: &ExecutionContext,
    ) -> anyhow::Result<AgentResult> {
        Ok(AgentResult::new("Canonical report prepared".to_string()))
    }

    async fn enrich_subtask(
        &self,
        _subtask_title: &str,
        _subtask_description: &str,
        _execution_context: &ExecutionContext,
        _agent_type: &str,
    ) -> anyhow::Result<Option<String>> {
        unreachable!("Reporting must not retrieve generic memory/wiki enrichment")
    }

    async fn plan_subtask(
        &self,
        _subtask_title: &str,
        _subtask_description: &str,
        _execution_context: &ExecutionContext,
        _agent_type: &str,
    ) -> anyhow::Result<Option<String>> {
        unreachable!("Reporting uses the deterministic server-owned stage task")
    }

    async fn reflect(&self, _subtask_title: &str, _agent_response: &str) -> anyhow::Result<String> {
        unreachable!("the deterministic refiner replaced this callback")
    }
}

#[async_trait]
impl AgentExecutor for ExhaustedStageRunExecutor {
    async fn execute_subtask(
        &self,
        _subtask_title: &str,
        _subtask_description: &str,
        _execution_context: &ExecutionContext,
        _agent_type: Option<&str>,
    ) -> anyhow::Result<AgentResult> {
        self.execute_calls.fetch_add(1, Ordering::SeqCst);
        Ok(AgentResult::new(
            "I would retry the specialist stage now, but the request-scoped stage_run budget is already exhausted and needs a later user continuation."
                .to_string(),
        ))
    }

    fn stage_run_retry_budget_exhausted(&self, _stage: StageKind) -> bool {
        true
    }

    async fn generate_report(
        &self,
        _execution_context: &ExecutionContext,
    ) -> anyhow::Result<AgentResult> {
        unreachable!("report generation is outside this focused subtask test")
    }

    async fn reflect(&self, _subtask_title: &str, _agent_response: &str) -> anyhow::Result<String> {
        unreachable!("the deterministic refiner replaced this callback")
    }
}

impl MemRepo {
    fn seed(operation_id: Uuid, profile: &str, current_stage: &str) -> Arc<Self> {
        let mut m = HashMap::new();
        m.insert(
            operation_id,
            OperationStateView {
                operation_id,
                profile: profile.to_string(),
                current_stage: current_stage.to_string(),
                runtime_memory_contract: crate::runtime_memory::RuntimeMemoryContract::LegacyV1,
                tool_truth_contract: golish_pentest_domain::tool_truth::ToolTruthContract::LegacyV1,
                investigation_contract_version:
                    golish_core::InvestigationContractVersion::LegacyCandidateV1,
                investigation_rollout_mode: golish_core::InvestigationRolloutMode::LegacyOnly,
                application_model_contract: golish_core::ApplicationModelContract::LegacyNoModel,
                stage_topology_contract:
                    golish_core::StageTopologyContract::LegacyCandidateVerificationV1
                        .freeze_material(),
                project_scope_id: None,
                engagement_org_id: None,
                state_blob: serde_json::Value::Null,
                stage_started_at: chrono::Utc::now(),
            },
        );
        let stage = StageKind::try_parse(current_stage).expect("fixture stage kind");
        let initial_stage_execution = crate::task_orchestrator::stage_execution::StageExecution {
            id: Uuid::new_v4(),
            operation_id,
            stage,
            status: crate::task_orchestrator::stage_execution::StageExecutionStatus::Started,
        };
        Arc::new(Self {
            op_state: Mutex::new(m),
            active_stage_executions: Mutex::new(HashMap::from([(
                operation_id,
                initial_stage_execution,
            )])),
            completed_stage_executions: Mutex::new(HashMap::new()),
            fail_operation_insert: AtomicBool::new(false),
            fail_stage_transition: AtomicBool::new(false),
            stage_transition_count: Mutex::new(0),
            wiki_result: Mutex::new(serde_json::Value::Null),
            wiki_search_calls: AtomicUsize::new(0),
            tasks: Mutex::new(HashMap::new()),
            submissions: Mutex::new(HashMap::new()),
            attack_execution_contract: Mutex::new(golish_core::AttackExecutionContract::Legacy),
            candidate_review_barrier: Mutex::new(None),
            candidate_review_reads: AtomicUsize::new(0),
            verification_truth: Mutex::new(None),
            wave_consolidation: Mutex::new(None),
            wave_consolidation_calls: AtomicUsize::new(0),
            fail_wave_consolidation: AtomicBool::new(false),
            reporting_truth: Mutex::new(None),
            reporting_build_calls: AtomicUsize::new(0),
            reporting_gate_reads: AtomicUsize::new(0),
            state_blob_write_count: AtomicUsize::new(0),
            scoping_target_snapshots: Mutex::new(HashMap::new()),
            scoping_target_snapshot_reads: AtomicUsize::new(0),
            fail_scoping_target_snapshot: AtomicBool::new(false),
            active_recon_scope_candidates: Mutex::new(HashMap::new()),
            active_recon_scope_approvals: Mutex::new(HashMap::new()),
            active_recon_scope_apply_calls: AtomicUsize::new(0),
            scoping_finalizations: Mutex::new(HashMap::new()),
            scoping_finalize_calls: AtomicUsize::new(0),
            scoping_finalize_replays: AtomicUsize::new(0),
            fail_scoping_finalize: AtomicBool::new(false),
            mismatch_scoping_finalize_identity: AtomicBool::new(false),
        })
    }

    #[allow(dead_code)]
    fn stage(&self, operation_id: Uuid) -> Option<String> {
        self.op_state
            .lock()
            .unwrap()
            .get(&operation_id)
            .map(|s| s.current_stage.clone())
    }

    fn active_stage_execution_id(&self, operation_id: Uuid) -> Option<Uuid> {
        self.active_stage_executions
            .lock()
            .unwrap()
            .get(&operation_id)
            .map(|execution| execution.id)
    }

    fn stage_transition_count(&self) -> usize {
        *self.stage_transition_count.lock().unwrap()
    }

    fn fail_next_stage_transition(&self) {
        self.fail_stage_transition.store(true, Ordering::SeqCst);
    }

    fn set_candidate_review_barrier(&self, barrier: AttackV2ReviewBarrierView) {
        *self.candidate_review_barrier.lock().unwrap() = Some(barrier);
    }

    fn enable_exact_candidate_v2(&self, operation_id: Uuid) {
        self.op_state
            .lock()
            .unwrap()
            .get_mut(&operation_id)
            .expect("fixture operation")
            .runtime_memory_contract = crate::runtime_memory::RuntimeMemoryContract::V2Only;
        *self.attack_execution_contract.lock().unwrap() =
            golish_core::AttackExecutionContract::V2Only;
    }

    fn enable_v2_scoping(&self, operation_id: Uuid, project_scope_id: Uuid) {
        let mut operations = self.op_state.lock().unwrap();
        let operation = operations
            .get_mut(&operation_id)
            .expect("fixture operation");
        operation.runtime_memory_contract =
            crate::runtime_memory::RuntimeMemoryContract::DualWriteLegacyRead;
        operation.project_scope_id = Some(project_scope_id);
    }

    fn fail_next_scoping_finalize(&self) {
        self.fail_scoping_finalize.store(true, Ordering::SeqCst);
    }

    fn mismatch_next_scoping_finalize_identity(&self) {
        self.mismatch_scoping_finalize_identity
            .store(true, Ordering::SeqCst);
    }

    fn set_state_blob(&self, operation_id: Uuid, state_blob: serde_json::Value) {
        self.op_state
            .lock()
            .unwrap()
            .get_mut(&operation_id)
            .expect("fixture operation")
            .state_blob = state_blob;
    }

    fn state_blob(&self, operation_id: Uuid) -> serde_json::Value {
        self.op_state
            .lock()
            .unwrap()
            .get(&operation_id)
            .expect("fixture operation")
            .state_blob
            .clone()
    }

    fn set_verification_truth(
        &self,
        truth: crate::harness::attack_execution::VerificationTruthSet,
    ) {
        *self.verification_truth.lock().unwrap() = Some(truth);
    }

    fn set_wave_consolidation(&self, result: AttackV2WaveConsolidationView) {
        *self.wave_consolidation.lock().unwrap() = Some(result);
    }

    fn fail_next_wave_consolidation(&self) {
        self.fail_wave_consolidation.store(true, Ordering::SeqCst);
    }

    fn set_reporting_truth(&self, truth: crate::harness::ReportingGateTruth) {
        *self.reporting_truth.lock().unwrap() = Some(truth);
    }

    fn fail_next_operation_insert(&self) {
        self.fail_operation_insert.store(true, Ordering::SeqCst);
    }

    fn set_scoping_target_snapshot(
        &self,
        organization_id: Uuid,
        targets: Vec<ScopingReviewedTarget>,
    ) {
        self.scoping_target_snapshots
            .lock()
            .unwrap()
            .insert(organization_id, targets);
    }

    fn fail_next_scoping_target_snapshot(&self) {
        self.fail_scoping_target_snapshot
            .store(true, Ordering::SeqCst);
    }

    fn set_active_recon_scope_candidates(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        targets: Vec<ScopingReviewedTarget>,
    ) {
        self.active_recon_scope_candidates
            .lock()
            .unwrap()
            .insert((operation_id, organization_id), targets);
    }

    /// Seed a `tasks` row with a given status (P1 finalize tests).
    fn insert_task(&self, status: TaskStatus, result: Option<&str>) -> Uuid {
        let id = Uuid::new_v4();
        self.tasks.lock().unwrap().insert(
            id,
            TaskView {
                id,
                input: "test task".to_string(),
                status,
                result: result.map(|s| s.to_string()),
            },
        );
        id
    }

    fn task_status(&self, id: Uuid) -> Option<TaskStatus> {
        self.tasks.lock().unwrap().get(&id).map(|t| t.status)
    }

    fn task_result(&self, id: Uuid) -> Option<String> {
        self.tasks
            .lock()
            .unwrap()
            .get(&id)
            .and_then(|t| t.result.clone())
    }

    /// Seed the canned wiki search result returned by `wiki_search_fts`.
    fn set_wiki(&self, value: serde_json::Value) {
        *self.wiki_result.lock().unwrap() = value;
    }
}

#[async_trait]
impl RuntimeMemoryRepository for MemRepo {
    async fn attack_execution_contract_for_operation(
        &self,
        _operation_id: Uuid,
    ) -> Result<golish_core::AttackExecutionContract, RuntimeMemoryError> {
        Ok(*self.attack_execution_contract.lock().unwrap())
    }

    async fn project_scope_register_first_open(
        &self,
        _canonical_path: &str,
        _path_sha256: &str,
    ) -> Result<ProjectScopeRegistration, RuntimeMemoryError> {
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn project_scope_rename(
        &self,
        _project_scope_id: Uuid,
        _expected_old_path: &str,
        _expected_row_version: i64,
        _new_path: &str,
        _new_path_sha256: &str,
    ) -> Result<ProjectScopeRegistration, RuntimeMemoryError> {
        Err(RuntimeMemoryError::Unavailable)
    }

    async fn create_runtime_operation(
        &self,
        input: CreateRuntimeOperation,
    ) -> Result<CreatedRuntimeOperation, RuntimeMemoryError> {
        if self.fail_operation_insert.swap(false, Ordering::SeqCst) {
            return Err(RuntimeMemoryError::Storage(
                "injected atomic runtime operation failure".to_string(),
            ));
        }
        let task = TaskView {
            id: input.operation_id,
            input: input.input,
            status: TaskStatus::Created,
            result: None,
        };
        let operation = OperationStateView {
            operation_id: input.operation_id,
            profile: input.profile,
            current_stage: input.entry_stage,
            runtime_memory_contract:
                crate::runtime_memory::RuntimeMemoryContract::DualWriteLegacyRead,
            tool_truth_contract: golish_pentest_domain::tool_truth::ToolTruthContract::LegacyV1,
            investigation_contract_version:
                golish_core::InvestigationContractVersion::LegacyCandidateV1,
            investigation_rollout_mode: golish_core::InvestigationRolloutMode::LegacyOnly,
            application_model_contract: golish_core::ApplicationModelContract::LegacyNoModel,
            stage_topology_contract:
                golish_core::StageTopologyContract::LegacyCandidateVerificationV1.freeze_material(),
            project_scope_id: Some(input.project_scope.project_scope_id),
            engagement_org_id: None,
            state_blob: serde_json::Value::Null,
            stage_started_at: chrono::Utc::now(),
        };
        self.tasks.lock().unwrap().insert(task.id, task.clone());
        self.op_state
            .lock()
            .unwrap()
            .insert(operation.operation_id, operation.clone());
        self.active_stage_executions.lock().unwrap().insert(
            operation.operation_id,
            crate::task_orchestrator::stage_execution::StageExecution {
                id: input.initial_stage_execution_id,
                operation_id: operation.operation_id,
                stage: StageKind::try_parse(&operation.current_stage)
                    .expect("created operation stage kind"),
                status: crate::task_orchestrator::stage_execution::StageExecutionStatus::Started,
            },
        );
        Ok(CreatedRuntimeOperation {
            task,
            operation,
            initial_stage_execution_id: input.initial_stage_execution_id,
        })
    }

    async fn active_stage_execution(
        &self,
        operation_id: Uuid,
    ) -> Result<crate::task_orchestrator::stage_execution::StageExecution, RuntimeMemoryError> {
        self.active_stage_executions
            .lock()
            .unwrap()
            .get(&operation_id)
            .cloned()
            .ok_or(RuntimeMemoryError::Missing {
                entity: "stage_runs",
            })
    }

    async fn transition_stage_execution(
        &self,
        input: crate::task_orchestrator::stage_execution::TransitionStageExecution,
    ) -> Result<
        crate::task_orchestrator::stage_execution::TransitionedStageExecution,
        RuntimeMemoryError,
    > {
        use crate::task_orchestrator::stage_execution::{
            StageExecution, StageExecutionStatus, TransitionedStageExecution,
        };

        if self.fail_stage_transition.swap(false, Ordering::SeqCst) {
            return Err(RuntimeMemoryError::Storage(
                "injected stage transition failure".to_string(),
            ));
        }
        let mut active = self.active_stage_executions.lock().unwrap();
        let previous =
            active
                .get(&input.operation_id)
                .cloned()
                .ok_or(RuntimeMemoryError::Missing {
                    entity: "stage_runs",
                })?;
        if previous.id != input.current_stage_execution_id {
            return Err(RuntimeMemoryError::IdentityMismatch {
                code: "active_stage_execution_mismatch",
            });
        }
        if previous.status != StageExecutionStatus::Started {
            return Err(RuntimeMemoryError::Conflict {
                code: "stage_execution_not_active",
            });
        }
        let mut operation = self.op_state.lock().unwrap();
        let operation =
            operation
                .get_mut(&input.operation_id)
                .ok_or(RuntimeMemoryError::Missing {
                    entity: "operation_state",
                })?;
        if operation.current_stage != previous.stage.as_str() {
            return Err(RuntimeMemoryError::IdentityMismatch {
                code: "operation_stage_execution_mismatch",
            });
        }

        let mut completed = previous;
        completed.status = StageExecutionStatus::Completed;
        let current = StageExecution {
            id: input.next_stage_execution_id,
            operation_id: input.operation_id,
            stage: input.next_stage,
            status: StageExecutionStatus::Started,
        };
        operation.current_stage = input.next_stage.as_str().to_string();
        operation.stage_started_at = chrono::Utc::now();
        active.insert(input.operation_id, current.clone());
        *self.stage_transition_count.lock().unwrap() += 1;
        Ok(TransitionedStageExecution {
            previous: completed,
            current,
        })
    }

    async fn complete_terminal_stage_execution(
        &self,
        input: crate::task_orchestrator::stage_execution::CompleteTerminalStageExecution,
    ) -> Result<crate::task_orchestrator::stage_execution::StageExecution, RuntimeMemoryError> {
        use crate::task_orchestrator::stage_execution::StageExecutionStatus;

        let mut active = self.active_stage_executions.lock().unwrap();
        let current = active
            .remove(&input.operation_id)
            .ok_or(RuntimeMemoryError::Missing {
                entity: "stage_runs",
            })?;
        if current.id != input.current_stage_execution_id
            || current.operation_id != input.operation_id
            || current.stage != input.terminal_stage
            || current.status != StageExecutionStatus::Started
        {
            active.insert(input.operation_id, current);
            return Err(RuntimeMemoryError::IdentityMismatch {
                code: "terminal_stage_execution_mismatch",
            });
        }
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks
            .get_mut(&input.operation_id)
            .ok_or(RuntimeMemoryError::Missing { entity: "tasks" })?;
        if task.status != TaskStatus::Running || task.result.is_some() {
            active.insert(input.operation_id, current);
            return Err(RuntimeMemoryError::Conflict {
                code: "terminal_task_not_running",
            });
        }
        let mut completed = current;
        completed.status = StageExecutionStatus::Completed;
        task.status = TaskStatus::Finished;
        task.result = Some(input.task_result);
        self.completed_stage_executions
            .lock()
            .unwrap()
            .insert(input.operation_id, completed.clone());
        Ok(completed)
    }

    async fn runtime_memory_contract_for_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<crate::runtime_memory::RuntimeMemoryContract, RuntimeMemoryError> {
        self.op_state
            .lock()
            .unwrap()
            .get(&operation_id)
            .map(|operation| operation.runtime_memory_contract)
            .ok_or(RuntimeMemoryError::Missing {
                entity: "operation_state",
            })
    }

    async fn load_stage_deliverable_submission(
        &self,
        deliverable_submission_id: Uuid,
        operation_id: Uuid,
        stage_execution_id: Uuid,
    ) -> Result<Option<PersistedStageDeliverableSubmission>, RuntimeMemoryError> {
        Ok(self
            .submissions
            .lock()
            .unwrap()
            .get(&deliverable_submission_id)
            .filter(|submission| {
                submission.operation_id == operation_id
                    && submission.stage_execution_id == stage_execution_id
            })
            .cloned())
    }

    async fn finalize_scoping_scope(
        &self,
        input: FinalizeScopingScope,
    ) -> Result<FinalizedScopingScope, RuntimeMemoryError> {
        self.scoping_finalize_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_scoping_finalize.swap(false, Ordering::SeqCst) {
            return Err(RuntimeMemoryError::Storage(
                "injected Scoping finalization failure".to_string(),
            ));
        }

        let mut finalizations = self.scoping_finalizations.lock().unwrap();
        if let Some((frozen_input, frozen_result)) = finalizations.get(&input.operation_id) {
            if frozen_input != &input {
                return Err(RuntimeMemoryError::IdentityMismatch {
                    code: "scoping_finalization_replay_mismatch",
                });
            }
            self.scoping_finalize_replays.fetch_add(1, Ordering::SeqCst);
            let mut replayed = frozen_result.clone();
            replayed.replayed = true;
            return Ok(replayed);
        }

        let mut finalized = FinalizedScopingScope {
            operation_id: input.operation_id,
            project_scope_id: input.project_scope_id,
            stage_execution_id: input.stage_execution_id,
            root_organization_id: input.root_organization_id,
            deliverable_submission_id: input.deliverable_submission_id,
            scope_decision_id: Uuid::new_v5(
                &input.deliverable_submission_id,
                b"test:scoping-scope-decision:v1",
            ),
            scope_snapshot_id: input.scope_snapshot_id,
            scoping_root_unit_id: input.scoping_root_unit_id,
            mode: "create".to_string(),
            scope_hash: "test-scope-hash".to_string(),
            units: vec![FrozenOrganizationScopeUnit {
                organization_id: input.root_organization_id,
                parent_organization_id: None,
                organization_name_at_freeze: COMPANY_PARITY_FIXTURE.to_string(),
                role: "root".to_string(),
                depth: 0,
                ordinal: 0,
                ownership_percent: None,
                decision_row_id: "root".to_string(),
                approval_source: serde_json::json!({"kind": "test"}),
            }],
            replayed: false,
        };
        if self
            .mismatch_scoping_finalize_identity
            .swap(false, Ordering::SeqCst)
        {
            finalized.scope_snapshot_id = Uuid::new_v4();
        }
        finalizations.insert(input.operation_id, (input, finalized.clone()));
        Ok(finalized)
    }
}

#[async_trait]
impl DbRepoProvider for MemRepo {
    // ── Operation state (the only methods these tests exercise) ─────────
    async fn operation_state_insert(
        &self,
        operation_id: Uuid,
        profile: &str,
        current_stage: &str,
        runtime_memory_contract: crate::runtime_memory::RuntimeMemoryContract,
    ) -> anyhow::Result<()> {
        if self.fail_operation_insert.swap(false, Ordering::SeqCst) {
            anyhow::bail!("injected operation contract persistence failure");
        }
        self.op_state.lock().unwrap().insert(
            operation_id,
            OperationStateView {
                operation_id,
                profile: profile.to_string(),
                current_stage: current_stage.to_string(),
                runtime_memory_contract,
                tool_truth_contract: golish_pentest_domain::tool_truth::ToolTruthContract::LegacyV1,
                investigation_contract_version:
                    golish_core::InvestigationContractVersion::LegacyCandidateV1,
                investigation_rollout_mode: golish_core::InvestigationRolloutMode::LegacyOnly,
                application_model_contract: golish_core::ApplicationModelContract::LegacyNoModel,
                stage_topology_contract:
                    golish_core::StageTopologyContract::LegacyCandidateVerificationV1
                        .freeze_material(),
                project_scope_id: None,
                engagement_org_id: None,
                state_blob: serde_json::Value::Null,
                stage_started_at: chrono::Utc::now(),
            },
        );
        Ok(())
    }
    async fn operation_state_get(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<Option<OperationStateView>> {
        Ok(self.op_state.lock().unwrap().get(&operation_id).cloned())
    }
    async fn operation_state_advance_stage(
        &self,
        operation_id: Uuid,
        new_stage: &str,
    ) -> anyhow::Result<()> {
        if let Some(s) = self.op_state.lock().unwrap().get_mut(&operation_id) {
            s.current_stage = new_stage.to_string();
        }
        Ok(())
    }
    async fn operation_state_write_state_blob(
        &self,
        operation_id: Uuid,
        state_blob: serde_json::Value,
    ) -> anyhow::Result<()> {
        self.state_blob_write_count.fetch_add(1, Ordering::SeqCst);
        self.set_state_blob(operation_id, state_blob);
        Ok(())
    }

    async fn scoping_target_snapshot(
        &self,
        organization_id: Uuid,
    ) -> anyhow::Result<Vec<ScopingReviewedTarget>> {
        self.scoping_target_snapshot_reads
            .fetch_add(1, Ordering::SeqCst);
        if self
            .fail_scoping_target_snapshot
            .swap(false, Ordering::SeqCst)
        {
            anyhow::bail!("injected scoping target snapshot failure");
        }
        Ok(self
            .scoping_target_snapshots
            .lock()
            .unwrap()
            .get(&organization_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn active_recon_scope_review_candidates(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<Vec<ScopingReviewedTarget>> {
        Ok(self
            .active_recon_scope_candidates
            .lock()
            .unwrap()
            .get(&(operation_id, organization_id))
            .cloned()
            .unwrap_or_default())
    }

    async fn active_recon_scope_review_apply(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        approval: ActiveReconScopeReviewApproval,
    ) -> anyhow::Result<Vec<ScopingReviewedTarget>> {
        self.active_recon_scope_apply_calls
            .fetch_add(1, Ordering::SeqCst);
        self.active_recon_scope_approvals
            .lock()
            .unwrap()
            .insert((operation_id, organization_id), approval.selected.clone());
        self.set_scoping_target_snapshot(organization_id, approval.selected.clone());
        Ok(approval.selected)
    }

    async fn active_recon_scope_review_authorized(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<bool> {
        Ok(self
            .active_recon_scope_approvals
            .lock()
            .unwrap()
            .contains_key(&(operation_id, organization_id)))
    }

    async fn attack_v2_review_barrier_for_operation(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<AttackV2ReviewBarrierView> {
        self.candidate_review_reads.fetch_add(1, Ordering::SeqCst);
        self.candidate_review_barrier
            .lock()
            .unwrap()
            .clone()
            .filter(|barrier| barrier.operation_id == operation_id)
            .ok_or_else(|| anyhow::anyhow!("ATTACK_V2_REVIEW_REPO_UNAVAILABLE"))
    }

    async fn attack_v2_verification_truth_for_operation(
        &self,
        operation_id: Uuid,
        _organization_id: Option<Uuid>,
    ) -> anyhow::Result<Option<crate::harness::attack_execution::VerificationTruthSet>> {
        Ok(self
            .verification_truth
            .lock()
            .unwrap()
            .clone()
            .filter(|truth| truth.authority.operation_id == operation_id))
    }

    async fn attack_v2_consolidate_wave(
        &self,
        input: AttackV2ConsolidateWave,
    ) -> anyhow::Result<AttackV2WaveConsolidationView> {
        self.wave_consolidation_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_wave_consolidation.swap(false, Ordering::SeqCst) {
            anyhow::bail!("injected consolidation failure");
        }
        self.wave_consolidation
            .lock()
            .unwrap()
            .clone()
            .filter(|result| {
                result.scope_snapshot_id == input.scope_snapshot_id
                    && result.source_wave_run_id == input.source_wave_run_id
            })
            .ok_or_else(|| anyhow::anyhow!("ATTACK_V2_CONSOLIDATION_UNAVAILABLE"))
    }

    async fn reporting_build_validated_revision(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<crate::harness::ReportingGateTruth> {
        self.reporting_build_calls.fetch_add(1, Ordering::SeqCst);
        self.reporting_truth
            .lock()
            .unwrap()
            .clone()
            .filter(|truth| truth.operation_id == operation_id)
            .ok_or_else(|| anyhow::anyhow!("REPORTING_TRUTH_REPO_UNAVAILABLE"))
    }

    async fn reporting_gate_truth(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<Option<crate::harness::ReportingGateTruth>> {
        self.reporting_gate_reads.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .reporting_truth
            .lock()
            .unwrap()
            .clone()
            .filter(|truth| truth.operation_id == operation_id))
    }

    // ── Unreachable in the transition-driver paths under test ───────────
    async fn wiki_upsert_page(&self, _page: &NewWikiPage) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn wiki_link_cve(&self, _cve: &str, _path: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn wiki_delete_refs_from(&self, _path: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn wiki_upsert_page_ref(
        &self,
        _from_path: &str,
        _to_path: &str,
        _context: &str,
    ) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn wiki_add_changelog(&self, _entry: &NewWikiChangelog) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn wiki_search_fts(
        &self,
        _query: &str,
        _limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        // P3 RAG-prior wiring tests read this; transition-driver tests never do.
        self.wiki_search_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.wiki_result.lock().unwrap().clone())
    }
    async fn wiki_search_by_category(
        &self,
        _category: &str,
        _limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        unimplemented!()
    }
    async fn wiki_search_by_tag(
        &self,
        _tag: &str,
        _limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        unimplemented!()
    }
    async fn wiki_list_cves_with_pocs(&self) -> anyhow::Result<serde_json::Value> {
        unimplemented!()
    }
    async fn wiki_list_unresearched_cves(&self, _limit: i64) -> anyhow::Result<serde_json::Value> {
        unimplemented!()
    }
    async fn wiki_poc_stats(&self) -> anyhow::Result<serde_json::Value> {
        unimplemented!()
    }
    #[allow(clippy::too_many_arguments)]
    async fn wiki_upsert_poc_full(
        &self,
        _cve_id: &str,
        _name: &str,
        _poc_type: &str,
        _language: &str,
        _content: &str,
        _source: &str,
        _source_url: &str,
        _severity: &str,
        _description: &str,
        _tags: &[String],
    ) -> anyhow::Result<serde_json::Value> {
        unimplemented!()
    }
    async fn vuln_intel_search(
        &self,
        _cve_id: &str,
        _limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        unimplemented!()
    }
    #[allow(clippy::too_many_arguments)]
    async fn audit_log_operation(
        &self,
        _summary: &str,
        _op_type: &str,
        _description: &str,
        _project_path: Option<&str>,
        _source: &str,
        _target_id: Option<Uuid>,
        _session_id: Option<&str>,
        _tool_name: Option<&str>,
        _status: &str,
        _detail: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        unimplemented!()
    }
    #[allow(clippy::too_many_arguments)]
    async fn api_endpoints_insert(
        &self,
        _target_id: Uuid,
        _project_path: Option<&str>,
        _url: &str,
        _method: &str,
        _path: &str,
        _params: &serde_json::Value,
        _raw_data: &serde_json::Value,
        _auth_type: Option<&str>,
        _source: &str,
        _risk_level: &str,
    ) -> anyhow::Result<serde_json::Value> {
        unimplemented!()
    }
    async fn js_analysis_insert(
        &self,
        _target_id: Uuid,
        _project_path: &str,
        _url: &str,
        _filename: &str,
        _analysis: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        unimplemented!()
    }
    async fn js_analysis_update_file_path(
        &self,
        _id: Uuid,
        _file_path: &str,
    ) -> anyhow::Result<()> {
        unimplemented!()
    }
    #[allow(clippy::too_many_arguments)]
    async fn fingerprints_upsert(
        &self,
        _target_id: Uuid,
        _project_path: &str,
        _category: &str,
        _name: &str,
        _version: Option<&str>,
        _confidence: f64,
        _raw_data: Option<&serde_json::Value>,
    ) -> anyhow::Result<bool> {
        unimplemented!()
    }
    #[allow(clippy::too_many_arguments)]
    async fn passive_scans_insert(
        &self,
        _operation_id: Option<Uuid>,
        _target_id: Uuid,
        _project_path: &str,
        _scan_type: &str,
        _tool_name: &str,
        _findings: &serde_json::Value,
        _raw_output: Option<&str>,
        _severity: &str,
    ) -> anyhow::Result<serde_json::Value> {
        unimplemented!()
    }
    async fn query_target_data(
        &self,
        _target_id: Uuid,
        _sections: &[String],
    ) -> anyhow::Result<serde_json::Value> {
        unimplemented!()
    }
    async fn task_create(&self, task: NewTask) -> anyhow::Result<TaskView> {
        let id = Uuid::new_v4();
        let view = TaskView {
            id,
            input: task.input.clone(),
            status: TaskStatus::Created,
            result: None,
        };
        self.tasks.lock().unwrap().insert(id, view.clone());
        Ok(view)
    }
    async fn task_get(&self, id: Uuid) -> anyhow::Result<Option<TaskView>> {
        Ok(self.tasks.lock().unwrap().get(&id).cloned())
    }
    async fn task_update_status(&self, id: Uuid, status: TaskStatus) -> anyhow::Result<()> {
        if let Some(t) = self.tasks.lock().unwrap().get_mut(&id) {
            t.status = status;
        }
        Ok(())
    }
    async fn task_set_result(&self, id: Uuid, result: &str) -> anyhow::Result<()> {
        if let Some(t) = self.tasks.lock().unwrap().get_mut(&id) {
            t.result = Some(result.to_string());
        }
        Ok(())
    }
    async fn subtask_create(
        &self,
        _task_id: Uuid,
        _session_id: Uuid,
        _title: &str,
        _description: &str,
        _agent: Option<AgentType>,
    ) -> anyhow::Result<SubtaskView> {
        unimplemented!()
    }
    async fn subtask_update_status(&self, _id: Uuid, _status: SubtaskStatus) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn subtask_set_result(&self, _id: Uuid, _result: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn subtask_next_pending(&self, _task_id: Uuid) -> anyhow::Result<Option<SubtaskView>> {
        unimplemented!()
    }
    async fn subtask_list_by_task(&self, _task_id: Uuid) -> anyhow::Result<Vec<SubtaskView>> {
        unimplemented!()
    }
    async fn subtask_delete_pending(&self, _task_id: Uuid) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn message_chain_create(
        &self,
        _session_id: Uuid,
        _task_id: Option<Uuid>,
        _subtask_id: Option<Uuid>,
        _agent_type: AgentType,
        _parent_chain_id: Option<Uuid>,
        _model: Option<&str>,
    ) -> anyhow::Result<MessageChainView> {
        unimplemented!()
    }
    async fn message_chain_update_chain(
        &self,
        _id: Uuid,
        _chain_json: &serde_json::Value,
    ) -> anyhow::Result<()> {
        unimplemented!()
    }
    #[allow(clippy::too_many_arguments)]
    async fn message_chain_update_usage(
        &self,
        _id: Uuid,
        _input_tokens: i32,
        _output_tokens: i32,
        _cache_read_tokens: i32,
        _input_cost: f64,
        _output_cost: f64,
        _duration_ms: i32,
    ) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn plan_list_active(
        &self,
        _project_path: &str,
    ) -> anyhow::Result<Vec<ExecutionPlanView>> {
        unimplemented!()
    }
    async fn plan_update_steps(
        &self,
        _id: Uuid,
        _steps: &serde_json::Value,
        _current_step: i32,
        _status: PlanStatus,
    ) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn plan_create(&self, _plan: NewExecutionPlan) -> anyhow::Result<ExecutionPlanView> {
        unimplemented!()
    }
    async fn dispatch_record_start(
        &self,
        _session_id: Uuid,
        _parent_dispatch_id: Option<Uuid>,
        _agent_id: &str,
        _tool_call_id: Option<&str>,
        _depth: i32,
        _args: &serde_json::Value,
    ) -> anyhow::Result<Uuid> {
        unimplemented!()
    }
    async fn dispatch_record_finish(
        &self,
        _id: Uuid,
        _status: DispatchStatus,
        _result: Option<&serde_json::Value>,
        _error_message: Option<&str>,
    ) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn dispatch_list_running(
        &self,
        _session_id: Uuid,
    ) -> anyhow::Result<Vec<SubAgentDispatchView>> {
        unimplemented!()
    }
}

/// Gate PASS **with progress** (`findings_count > 0`). Under the default
/// graph-flow routing a multi-successor stage then takes the main-path (first)
/// candidate; with no progress it would bail to reporting (covered separately in
/// `operation_flow`). These cursor-walk tests want the main path.
fn pass(stage: StageKind) -> HarnessGateOutcome {
    HarnessGateOutcome {
        gated_stage: stage,
        gate_allowed: true,
        trusted_submission: None,
        engagement_org_id: None,
        repair_correction: None,
        evidence_summary: None,
        evidence_refs: Vec::new(),
        required_evidence_kinds: Vec::new(),
        findings_count: 1,
        fabricated_evidence_refs: Vec::new(),
        available_real_ids: Vec::new(),
        missing_deliverable: false,
        gate_reasons: Vec::new(),
        gate_recovery: None,
        missing_kinds: Vec::new(),
        expired: Vec::new(),
        red_team_flow_correction: None,
        confirm_only_stage: false,
        evidence_kind_labels: std::collections::HashMap::new(),
        spawned_candidates: Vec::new(),
    }
}

fn trusted_v2_scoping_pass(
    operation_id: Uuid,
    stage_execution_id: Uuid,
    deliverable_submission_id: Uuid,
    root_organization_id: Uuid,
) -> HarnessGateOutcome {
    use sha2::{Digest, Sha256};

    let canonical_deliverable_json = format!(
        r#"{{"stage_id":"scoping","claims":[{{"kind":"scope_human_approved","subject":"{root_organization_id}"}}]}}"#
    );
    let payload_sha256 = Sha256::digest(canonical_deliverable_json.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let mut outcome = pass(StageKind::Scoping);
    outcome.engagement_org_id = Some(root_organization_id);
    outcome.trusted_submission = Some(CapturedStageSubmission {
        deliverable_submission_id,
        operation_id,
        stage_execution_id,
        stage_run_unit_id: None,
        canonical_deliverable_json,
        payload_sha256,
    });
    outcome
}

fn block(stage: StageKind) -> HarnessGateOutcome {
    HarnessGateOutcome {
        gated_stage: stage,
        gate_allowed: false,
        trusted_submission: None,
        engagement_org_id: None,
        repair_correction: None,
        evidence_summary: None,
        evidence_refs: Vec::new(),
        required_evidence_kinds: Vec::new(),
        findings_count: 0,
        fabricated_evidence_refs: Vec::new(),
        available_real_ids: Vec::new(),
        missing_deliverable: false,
        gate_reasons: Vec::new(),
        gate_recovery: None,
        missing_kinds: Vec::new(),
        expired: Vec::new(),
        red_team_flow_correction: None,
        confirm_only_stage: false,
        evidence_kind_labels: std::collections::HashMap::new(),
        spawned_candidates: Vec::new(),
    }
}

fn valid_reporting_truth(operation_id: Uuid) -> crate::harness::ReportingGateTruth {
    let revision_id = Uuid::new_v4();
    crate::harness::ReportingGateTruth {
        operation_id,
        report_id: Uuid::new_v4(),
        current_revision_id: revision_id,
        revision_id,
        validation_status: "validated".to_string(),
        publication_status: "unpublished".to_string(),
        stored_source_set_hash: "a".repeat(64),
        current_source_set_hash: "a".repeat(64),
        source_snapshot_exact: true,
        claims_citations_valid: true,
        validation_attestation_valid: true,
        cleanup_closeout_valid: true,
    }
}

fn pass_without_findings(stage: StageKind, evidence_refs: Vec<i64>) -> HarnessGateOutcome {
    let mut outcome = pass(stage);
    outcome.findings_count = 0;
    outcome.evidence_refs = evidence_refs;
    outcome.evidence_summary = Some(
        "- claims: stage_run_pass_token (external_attack_surface)\n- evidence refs: 25".to_string(),
    );
    outcome
}

/// A minimal attack candidate (serde defaults for the optional fields) for the
/// chain-wave decision tests.
fn candidate_for_test(hypothesis: &str) -> crate::harness::AttackCandidate {
    serde_json::from_str(&format!(
        r#"{{"candidate_id":"3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33",
            "target":"api.example.com","hypothesis":"{hypothesis}","rationale":"r"}}"#
    ))
    .expect("candidate parses")
}

fn exact_verification_truth(
    operation_id: Uuid,
) -> crate::harness::attack_execution::VerificationTruthSet {
    use crate::harness::attack_execution::{
        VerificationTruthAuthority, VerificationTruthSet, VerificationTruthSnapshot,
        VerificationUnitAuthority,
    };

    let scope_snapshot_id = Uuid::new_v4();
    let wave_run_id = Uuid::new_v4();
    let wave_unit_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    VerificationTruthSet {
        authority: VerificationTruthAuthority {
            operation_id,
            scope_snapshot_id,
            wave_run_id,
            expected_units: vec![VerificationUnitAuthority {
                wave_unit_id,
                organization_id,
            }],
        },
        snapshots: vec![VerificationTruthSnapshot {
            operation_id,
            scope_snapshot_id,
            wave_run_id,
            wave_unit_id,
            organization_id,
            review_closed: true,
            pending_work_items: 0,
            approved_ever: 0,
            attempts: Vec::new(),
            residual_risks: Vec::new(),
        }],
    }
}

fn exact_verification_truth_with_attempts(
    operation_id: Uuid,
    attempts: Vec<crate::harness::attack_execution::AttemptTerminalTruth>,
) -> crate::harness::attack_execution::VerificationTruthSet {
    let mut truth = exact_verification_truth(operation_id);
    truth.snapshots[0].approved_ever =
        u32::try_from(attempts.len()).expect("fixture attempt count fits u32");
    truth.snapshots[0].attempts = attempts;
    truth
}

fn verified_attempt() -> crate::harness::attack_execution::AttemptTerminalTruth {
    crate::harness::attack_execution::AttemptTerminalTruth {
        candidate_id: Uuid::new_v4(),
        attempt_id: Uuid::new_v4(),
        candidate_plan_hash: "verified-plan".to_string(),
        status: "verified".to_string(),
        proof_evidence_ids: vec![901],
        refutation_evidence_ids: Vec::new(),
        blocker_evidence_ids: Vec::new(),
        blocker_reason_code: None,
        finding_id: Some(Uuid::new_v4()),
        finding_lineage_exact: true,
    }
}

fn refuted_attempt() -> crate::harness::attack_execution::AttemptTerminalTruth {
    crate::harness::attack_execution::AttemptTerminalTruth {
        candidate_id: Uuid::new_v4(),
        attempt_id: Uuid::new_v4(),
        candidate_plan_hash: "refuted-plan".to_string(),
        status: "refuted".to_string(),
        proof_evidence_ids: Vec::new(),
        refutation_evidence_ids: vec![902],
        blocker_evidence_ids: Vec::new(),
        blocker_reason_code: None,
        finding_id: None,
        finding_lineage_exact: false,
    }
}

fn blocked_attempt() -> crate::harness::attack_execution::AttemptTerminalTruth {
    crate::harness::attack_execution::AttemptTerminalTruth {
        candidate_id: Uuid::new_v4(),
        attempt_id: Uuid::new_v4(),
        candidate_plan_hash: "blocked-plan".to_string(),
        status: "blocked".to_string(),
        proof_evidence_ids: Vec::new(),
        refutation_evidence_ids: Vec::new(),
        blocker_evidence_ids: Vec::new(),
        blocker_reason_code: Some("scope_blocked".to_string()),
        finding_id: None,
        finding_lineage_exact: false,
    }
}

fn consolidation_for(
    truth: &crate::harness::attack_execution::VerificationTruthSet,
    decision_kind: &str,
    target_wave_run_id: Option<Uuid>,
) -> AttackV2WaveConsolidationView {
    AttackV2WaveConsolidationView {
        operation_id: truth.authority.operation_id,
        scope_snapshot_id: truth.authority.scope_snapshot_id,
        consolidation_id: Uuid::new_v4(),
        source_wave_run_id: truth.authority.wave_run_id,
        target_wave_run_id,
        decision_kind: decision_kind.to_string(),
        accepted_fact_delta_count: usize::from(target_wave_run_id.is_some()),
        rejected_fact_delta_count: 0,
        residual_risk_count: 0,
        pending_enrichment_count: 0,
        replayed: false,
    }
}

fn drain(rx: &mut mpsc::UnboundedReceiver<AiEvent>) -> Vec<AiEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

fn saw_waiting_approval(events: &[AiEvent]) -> bool {
    events
        .iter()
        .any(|e| matches!(e, AiEvent::TaskProgress { status, .. } if status == "waiting_approval"))
}

fn scoping_target(value: &str, scope: &str) -> ScopingReviewedTarget {
    ScopingReviewedTarget {
        value: value.to_string(),
        target_type: "domain".to_string(),
        scope: scope.to_string(),
    }
}

fn company_parity_target(scope: &str) -> ScopingReviewedTarget {
    ScopingReviewedTarget {
        value: COMPANY_PARITY_TARGET.to_string(),
        target_type: "url".to_string(),
        scope: scope.to_string(),
    }
}

/// Block until the next `AskHumanRequest` arrives, returning its `request_id`
/// (used to resolve exact target-scope cards through the coordinator).
async fn recv_ask_human_request_id(rx: &mut mpsc::UnboundedReceiver<AiEvent>) -> String {
    loop {
        match rx.recv().await {
            Some(AiEvent::AskHumanRequest { request_id, .. }) => return request_id,
            Some(_) => continue,
            None => panic!("event channel closed before an AskHumanRequest arrived"),
        }
    }
}

/// Minimal [`GolishRuntime`] so gate tests can spawn a real `EventCoordinator`
/// (the orchestrator emits the cards over its own `event_tx`; the coordinator is
/// used only to register + resolve the approval decisions).
struct GateMockRuntime;

#[async_trait]
impl golish_core::runtime::GolishRuntime for GateMockRuntime {
    fn emit(
        &self,
        _event: golish_core::runtime::RuntimeEvent,
    ) -> Result<(), golish_core::runtime::RuntimeError> {
        Ok(())
    }

    async fn request_approval(
        &self,
        _request_id: String,
        _tool_name: String,
        _args: serde_json::Value,
        _risk_level: String,
    ) -> Result<golish_core::runtime::ApprovalResult, golish_core::runtime::RuntimeError> {
        Ok(golish_core::runtime::ApprovalResult::Approved)
    }

    fn is_interactive(&self) -> bool {
        true
    }

    fn auto_approve(&self) -> bool {
        false
    }

    async fn shutdown(&self) -> Result<(), golish_core::runtime::RuntimeError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[tokio::test]
async fn new_operation_fails_closed_when_runtime_contract_cannot_persist() {
    let repo = MemRepo::seed(Uuid::new_v4(), "assessment", StageKind::Scoping.as_str());
    repo.fail_next_operation_insert();
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let mut orchestrator =
        TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), event_tx);
    let executor = ExhaustedStageRunExecutor {
        execute_calls: AtomicUsize::new(0),
    };

    let error = orchestrator
        .run(
            "runtime contract failure fixture",
            ProjectScopeRegistration {
                project_scope_id: Uuid::new_v4(),
                canonical_project_path: "/tmp/runtime-contract-failure".to_string(),
                path_sha256: "a".repeat(64),
                row_version: 0,
            },
            &executor,
        )
        .await
        .expect_err("operation creation must stop when its contract is not durable");

    assert!(error
        .to_string()
        .contains("Failed to create task and operation atomically"));
    let tasks = repo.tasks.lock().unwrap();
    assert!(
        tasks.is_empty(),
        "atomic create failure must leave no task row"
    );
    assert_eq!(executor.execute_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn exhausted_stage_run_signal_stops_text_only_automatic_repair_turn() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "assessment", "external_attack_surface");
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo, Uuid::new_v4(), tx);
    let executor = ExhaustedStageRunExecutor {
        execute_calls: AtomicUsize::new(0),
    };
    let planned = PlannedSubtask {
        title: "EAS continuation".to_string(),
        description: "Resume the parked specialist stage.".to_string(),
        agent: Some("pentester".to_string()),
        // This focused test exercises the outer-loop policy without invoking a
        // DB-backed harness gate; the runtime stage still carries the guard.
        harness_stage: None,
        nl_slice: None,
        acceptance_criteria: Vec::new(),
    };
    let exec_ctx = ExecutionContext {
        operation_id: Some(operation_id),
        harness_stage: Some(StageKind::ExternalAttackSurface),
        ..ExecutionContext::default()
    };

    let _ = orchestrator
        .execute_single_subtask(&planned, &exec_ctx, &executor, &None, operation_id)
        .await;

    assert_eq!(
        executor.execute_calls.load(Ordering::SeqCst),
        1,
        "the orchestrator must return the BLOCK to the user instead of opening a text-only repair turn in the same request"
    );
}

/// A gate PASS routed through `consume_gate_outcome` (the single chokepoint both
/// gate sites flow through) emits an authoritative `stage_passed` TaskProgress
/// carrying the stage id. The UI keys the "Stage complete" milestone + per-stage
/// card completion off this — not the structural `submit_stage_deliverable`
/// preview — so completion shows only after the real evidence gate accepts the
/// stage. Uses the terminal `reporting` stage so the transition completes without
/// pausing on an approval gate.
#[tokio::test]
async fn pass_emits_stage_passed_progress() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "assessment", "reporting");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    orch.consume_gate_outcome(op, pass(StageKind::Reporting))
        .await;

    let events = drain(&mut rx);
    assert!(
        events.iter().any(|e| matches!(
            e,
            AiEvent::TaskProgress { status, message, .. }
                if status == "stage_passed" && message == "reporting"
        )),
        "gate PASS must emit a stage_passed TaskProgress carrying the stage id"
    );
}

#[tokio::test]
async fn v2_scoping_pass_finalizes_scope_before_stage_passed() {
    let operation_id = Uuid::new_v4();
    let project_scope_id = Uuid::new_v4();
    let root_organization_id = Uuid::new_v4();
    let deliverable_submission_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", StageKind::Scoping.as_str());
    repo.enable_v2_scoping(operation_id, project_scope_id);
    repo.op_state
        .lock()
        .unwrap()
        .get_mut(&operation_id)
        .unwrap()
        .engagement_org_id = Some(root_organization_id);
    let stage_execution_id = repo
        .active_stage_execution_id(operation_id)
        .expect("active Scoping execution");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    orchestrator
        .consume_gate_outcome(
            operation_id,
            trusted_v2_scoping_pass(
                operation_id,
                stage_execution_id,
                deliverable_submission_id,
                root_organization_id,
            ),
        )
        .await;

    assert_eq!(repo.scoping_finalize_calls.load(Ordering::SeqCst), 1);
    let finalizations = repo.scoping_finalizations.lock().unwrap();
    let (input, finalized) = finalizations
        .get(&operation_id)
        .expect("Scoping finalization recorded");
    assert_eq!(input.project_scope_id, project_scope_id);
    assert_eq!(input.stage_execution_id, stage_execution_id);
    assert_eq!(input.root_organization_id, root_organization_id);
    assert_eq!(input.deliverable_submission_id, deliverable_submission_id);
    assert_eq!(
        input.scope_snapshot_id,
        Uuid::new_v5(
            &deliverable_submission_id,
            super::SCOPING_SCOPE_SNAPSHOT_ID_V1
        )
    );
    assert_eq!(
        input.scoping_root_unit_id,
        Uuid::new_v5(&deliverable_submission_id, super::SCOPING_ROOT_UNIT_ID_V1)
    );
    assert_eq!(finalized.scope_snapshot_id, input.scope_snapshot_id);
    assert_eq!(finalized.scoping_root_unit_id, input.scoping_root_unit_id);
    assert!(!finalized.replayed);
    drop(finalizations);

    let flow = orchestrator
        .stage_outcome_acc
        .expect("Scoping flow outcome");
    assert!(flow.gate_allowed);
    assert_eq!(
        repo.stage_transition_count(),
        0,
        "consume only publishes the finalized PASS; graph stage entry owns the later transition"
    );
    assert!(drain(&mut rx).iter().any(|event| matches!(
        event,
        AiEvent::TaskProgress { status, message, .. }
            if status == "stage_passed" && message == StageKind::Scoping.as_str()
    )));
}

#[tokio::test]
async fn v2_scoping_same_submission_replays_deterministic_finalization() {
    let operation_id = Uuid::new_v4();
    let root_organization_id = Uuid::new_v4();
    let deliverable_submission_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", StageKind::Scoping.as_str());
    repo.enable_v2_scoping(operation_id, Uuid::new_v4());
    let stage_execution_id = repo
        .active_stage_execution_id(operation_id)
        .expect("active Scoping execution");
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    orchestrator
        .consume_gate_outcome(
            operation_id,
            trusted_v2_scoping_pass(
                operation_id,
                stage_execution_id,
                deliverable_submission_id,
                root_organization_id,
            ),
        )
        .await;
    orchestrator
        .consume_gate_outcome(
            operation_id,
            trusted_v2_scoping_pass(
                operation_id,
                stage_execution_id,
                deliverable_submission_id,
                root_organization_id,
            ),
        )
        .await;

    assert_eq!(repo.scoping_finalize_calls.load(Ordering::SeqCst), 2);
    assert_eq!(repo.scoping_finalize_replays.load(Ordering::SeqCst), 1);
    assert_eq!(repo.scoping_finalizations.lock().unwrap().len(), 1);
    assert!(
        orchestrator
            .stage_outcome_acc
            .expect("replayed Scoping flow outcome")
            .gate_allowed
    );
}

#[tokio::test]
async fn v2_scoping_finalize_failure_blocks_without_stage_passed() {
    let operation_id = Uuid::new_v4();
    let root_organization_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", StageKind::Scoping.as_str());
    repo.enable_v2_scoping(operation_id, Uuid::new_v4());
    repo.fail_next_scoping_finalize();
    let stage_execution_id = repo
        .active_stage_execution_id(operation_id)
        .expect("active Scoping execution");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    orchestrator
        .consume_gate_outcome(
            operation_id,
            trusted_v2_scoping_pass(
                operation_id,
                stage_execution_id,
                Uuid::new_v4(),
                root_organization_id,
            ),
        )
        .await;

    assert!(
        !orchestrator
            .stage_outcome_acc
            .expect("blocked Scoping flow outcome")
            .gate_allowed
    );
    assert_eq!(repo.stage_transition_count(), 0);
    let events = drain(&mut rx);
    assert!(!events.iter().any(
        |event| matches!(event, AiEvent::TaskProgress { status, .. } if status == "stage_passed")
    ));
    assert!(events.iter().any(|event| matches!(
        event,
        AiEvent::HarnessTrace {
            trace: golish_core::events::HarnessTraceKind::GateDecision {
                gate,
                first_blocking_reason: Some(reason),
                ..
            },
            ..
        } if gate == "BLOCK" && reason.contains("Scoping finalization failed")
    )));
}

#[tokio::test]
async fn v2_scoping_mismatched_finalizer_identity_blocks_without_stage_passed() {
    let operation_id = Uuid::new_v4();
    let root_organization_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", StageKind::Scoping.as_str());
    repo.enable_v2_scoping(operation_id, Uuid::new_v4());
    repo.mismatch_next_scoping_finalize_identity();
    let stage_execution_id = repo
        .active_stage_execution_id(operation_id)
        .expect("active Scoping execution");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    orchestrator
        .consume_gate_outcome(
            operation_id,
            trusted_v2_scoping_pass(
                operation_id,
                stage_execution_id,
                Uuid::new_v4(),
                root_organization_id,
            ),
        )
        .await;

    assert!(
        !orchestrator
            .stage_outcome_acc
            .expect("identity mismatch must block Scoping")
            .gate_allowed
    );
    let events = drain(&mut rx);
    assert!(!events.iter().any(
        |event| matches!(event, AiEvent::TaskProgress { status, .. } if status == "stage_passed")
    ));
    assert!(events.iter().any(|event| matches!(
        event,
        AiEvent::HarnessTrace {
            trace: golish_core::events::HarnessTraceKind::GateDecision {
                gate,
                first_blocking_reason: Some(reason),
                ..
            },
            ..
        } if gate == "BLOCK" && reason.contains("mismatched authority identity")
    )));
}

#[tokio::test]
async fn v2_scoping_prebound_org_mismatch_blocks_before_finalize() {
    let operation_id = Uuid::new_v4();
    let claimed_root_organization_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", StageKind::Scoping.as_str());
    repo.enable_v2_scoping(operation_id, Uuid::new_v4());
    repo.op_state
        .lock()
        .unwrap()
        .get_mut(&operation_id)
        .unwrap()
        .engagement_org_id = Some(Uuid::new_v4());
    let stage_execution_id = repo
        .active_stage_execution_id(operation_id)
        .expect("active Scoping execution");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    orchestrator
        .consume_gate_outcome(
            operation_id,
            trusted_v2_scoping_pass(
                operation_id,
                stage_execution_id,
                Uuid::new_v4(),
                claimed_root_organization_id,
            ),
        )
        .await;

    assert_eq!(
        repo.scoping_finalize_calls.load(Ordering::SeqCst),
        0,
        "an untrusted claimed root must not reach the atomic finalizer"
    );
    assert!(
        !orchestrator
            .stage_outcome_acc
            .expect("prebound org mismatch must block Scoping")
            .gate_allowed
    );
    let events = drain(&mut rx);
    assert!(!events.iter().any(
        |event| matches!(event, AiEvent::TaskProgress { status, .. } if status == "stage_passed")
    ));
    assert!(events.iter().any(|event| matches!(
        event,
        AiEvent::HarnessTrace {
            trace: golish_core::events::HarnessTraceKind::GateDecision {
                gate,
                first_blocking_reason: Some(reason),
                ..
            },
            ..
        } if gate == "BLOCK" && reason.contains("trusted organization binding")
    )));
}

#[tokio::test]
async fn reporting_stage_builds_and_validates_without_an_agent_or_auto_finalize() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "assessment", "reporting");
    let truth = valid_reporting_truth(operation_id);
    repo.set_reporting_truth(truth.clone());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    let executor = ReportingStageExecutor {
        execute_calls: AtomicUsize::new(0),
    };
    let mut exec_ctx = ExecutionContext {
        operation_id: Some(operation_id),
        stage_execution_id: repo.active_stage_execution_id(operation_id),
        task_input: "compile the canonical report".to_string(),
        ..ExecutionContext::default()
    };

    let outcome = orchestrator
        .run_stage_subtasks(
            StageKind::Reporting,
            &[],
            &[],
            &mut exec_ctx,
            None,
            &executor,
            operation_id,
            None,
        )
        .await;

    assert!(
        outcome.gate_allowed,
        "validated Reporting truth must pass, outcome={outcome:?}, result={:?}",
        exec_ctx
            .completed_results
            .last()
            .map(|result| &result.result)
    );
    assert_eq!(
        repo.reporting_build_calls.load(Ordering::SeqCst),
        1,
        "stage entry builds or reuses exactly one canonical revision"
    );
    assert!(
        repo.reporting_gate_reads.load(Ordering::SeqCst) >= 1,
        "stage close must re-read current truth instead of trusting entry state"
    );
    assert_eq!(
        executor.execute_calls.load(Ordering::SeqCst),
        0,
        "the canonical evidence summary closes Reporting without an agent turn"
    );
    assert_eq!(
        repo.wiki_search_calls.load(Ordering::SeqCst),
        0,
        "Reporting must not retrieve wiki/RAG prior"
    );
    assert_eq!(truth.validation_status, "validated");
    assert_eq!(
        truth.publication_status, "unpublished",
        "the stage seam has no artifact/finalizer operation"
    );
}

#[tokio::test]
async fn terminal_graph_completion_closes_exact_active_stage_execution_and_task() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "assessment", "reporting");
    let stage_execution_id = repo
        .active_stage_execution_id(operation_id)
        .expect("fixture has an active Reporting execution");
    repo.tasks.lock().unwrap().insert(
        operation_id,
        TaskView {
            id: operation_id,
            input: "compile the canonical report".to_string(),
            status: TaskStatus::Running,
            result: None,
        },
    );
    repo.set_reporting_truth(valid_reporting_truth(operation_id));
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    orchestrator.set_stage_allowlist(Some(std::collections::HashSet::from([
        StageKind::Reporting,
    ])));
    let executor = ReportingStageExecutor {
        execute_calls: AtomicUsize::new(0),
    };

    let report = orchestrator
        .run_executor_driven(
            operation_id,
            &[],
            &executor,
            false,
            None,
            Some(stage_execution_id),
        )
        .await
        .expect("terminal Reporting slice completes");

    assert!(report.contains("Validated evidence summary is ready"));
    assert!(report.contains("publication remains unpublished"));
    assert!(repo.active_stage_execution_id(operation_id).is_none());
    let completed = repo
        .completed_stage_executions
        .lock()
        .unwrap()
        .get(&operation_id)
        .cloned()
        .expect("terminal execution was closed");
    assert_eq!(completed.id, stage_execution_id);
    assert_eq!(
        completed.status,
        crate::task_orchestrator::stage_execution::StageExecutionStatus::Completed
    );
    let task = repo
        .tasks
        .lock()
        .unwrap()
        .get(&operation_id)
        .cloned()
        .expect("terminal task exists");
    assert_eq!(task.status, TaskStatus::Finished);
    assert_eq!(task.result.as_deref(), Some(report.as_str()));
}

#[tokio::test]
async fn reporting_stage_blocks_before_agent_when_canonical_build_fails() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "assessment", "reporting");
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    let executor = ReportingStageExecutor {
        execute_calls: AtomicUsize::new(0),
    };
    let mut exec_ctx = ExecutionContext {
        operation_id: Some(operation_id),
        stage_execution_id: repo.active_stage_execution_id(operation_id),
        task_input: "compile the canonical report".to_string(),
        ..ExecutionContext::default()
    };

    let outcome = orchestrator
        .run_stage_subtasks(
            StageKind::Reporting,
            &[],
            &[],
            &mut exec_ctx,
            None,
            &executor,
            operation_id,
            None,
        )
        .await;

    assert!(!outcome.gate_allowed);
    assert_eq!(repo.reporting_build_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        executor.execute_calls.load(Ordering::SeqCst),
        0,
        "missing canonical truth must stop before any model/agent turn"
    );
    assert_eq!(repo.reporting_gate_reads.load(Ordering::SeqCst), 0);
}

/// A gate BLOCK must NOT emit `stage_passed` — completion is gated on a real
/// PASS, so a blocked stage shows no "Stage complete".
#[tokio::test]
async fn block_emits_no_stage_passed() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "assessment", "scoping");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    orch.consume_gate_outcome(op, block(StageKind::Scoping))
        .await;

    let events = drain(&mut rx);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AiEvent::TaskProgress { status, .. } if status == "stage_passed")),
        "gate BLOCK must not emit stage_passed"
    );
}

/// Wave loop (设计 2026-07-02-attack-stage §3.5): a verification PASS carrying a
/// fresh (unseen) candidate opens the next attack_candidate wave — the servicer
/// advances its wave counter, records the hypothesis, and sets the flow's
/// `reopen_wave` signal the graph node routes on.
#[tokio::test]
async fn verification_pass_with_new_candidate_opens_wave() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "pentest", "verification");
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    let mut outcome = pass(StageKind::Verification);
    outcome.spawned_candidates = vec![candidate_for_test("IDOR on /orders/{id}")];
    orch.consume_gate_outcome(op, outcome).await;

    assert_eq!(orch.chain_wave, 1, "a new hypothesis opens wave 1");
    assert!(
        orch.stage_outcome_acc.expect("acc set").reopen_wave,
        "verification must signal reopen_wave when a new candidate surfaced"
    );
}

/// Re-testing the SAME hypothesis (already recorded this run) must NOT reopen
/// another wave — the cross-wave dedupe stops a↔b oscillation.
#[tokio::test]
async fn verification_pass_with_seen_candidate_does_not_reopen() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "pentest", "verification");
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    let c = candidate_for_test("IDOR on /orders/{id}");
    let mut first = pass(StageKind::Verification);
    first.spawned_candidates = vec![c.clone()];
    orch.consume_gate_outcome(op, first).await;
    assert_eq!(orch.chain_wave, 1);
    orch.stage_outcome_acc = None; // fresh accumulator for the next stage run

    let mut second = pass(StageKind::Verification);
    second.spawned_candidates = vec![c];
    orch.consume_gate_outcome(op, second).await;

    assert_eq!(
        orch.chain_wave, 1,
        "same hypothesis must not open another wave"
    );
    assert!(
        !orch.stage_outcome_acc.expect("acc set").reopen_wave,
        "an already-seen hypothesis must not signal reopen"
    );
}

/// A non-verification PASS never opens a wave, even if a deliverable somehow
/// carried candidates.
#[tokio::test]
async fn non_verification_pass_never_opens_wave() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "pentest", "vuln_triage");
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    let mut outcome = pass(StageKind::VulnTriage);
    outcome.spawned_candidates = vec![candidate_for_test("h")];
    orch.consume_gate_outcome(op, outcome).await;

    assert_eq!(orch.chain_wave, 0, "only verification opens waves");
    assert!(!orch.stage_outcome_acc.expect("acc set").reopen_wave);
}

#[tokio::test]
async fn exact_v2_verification_uses_durable_opened_next_wave_decision() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", "verification");
    repo.enable_exact_candidate_v2(operation_id);
    let truth = exact_verification_truth(operation_id);
    let target_wave_run_id = Uuid::new_v4();
    repo.set_verification_truth(truth.clone());
    repo.set_wave_consolidation(consolidation_for(
        &truth,
        "opened_next_wave",
        Some(target_wave_run_id),
    ));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    orchestrator
        .consume_gate_outcome(operation_id, pass(StageKind::Verification))
        .await;

    let flow = orchestrator.stage_outcome_acc.expect("flow outcome");
    assert!(flow.gate_allowed);
    assert!(flow.made_progress);
    assert!(flow.reopen_wave);
    assert!(flow.durable_wave_cursor);
    assert_eq!(repo.wave_consolidation_calls.load(Ordering::SeqCst), 1);
    assert!(drain(&mut rx).iter().any(|event| matches!(
        event,
        AiEvent::HarnessTrace {
            trace: golish_core::events::HarnessTraceKind::AttackWaveConsolidated {
                source_wave_run_id,
                target_wave_run_id: emitted_target,
                decision_kind,
                ..
            },
            ..
        } if source_wave_run_id == &truth.authority.wave_run_id.to_string()
            && emitted_target.as_deref() == Some(target_wave_run_id.to_string().as_str())
            && decision_kind == "opened_next_wave"
    )));
}

#[tokio::test]
async fn exact_v2_verification_closed_no_delta_does_not_reopen() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", "verification");
    repo.enable_exact_candidate_v2(operation_id);
    let truth = exact_verification_truth(operation_id);
    repo.set_verification_truth(truth.clone());
    repo.set_wave_consolidation(consolidation_for(&truth, "closed_no_delta", None));
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    orchestrator
        .consume_gate_outcome(operation_id, pass(StageKind::Verification))
        .await;

    let flow = orchestrator.stage_outcome_acc.expect("flow outcome");
    assert!(flow.gate_allowed);
    assert!(
        !flow.made_progress,
        "checked-empty Verification must take the normal Reporting branch"
    );
    assert!(!flow.reopen_wave);
    assert!(flow.durable_wave_cursor);
    assert_eq!(repo.wave_consolidation_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn exact_v2_pending_enrichment_is_an_explicit_observable_block() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", "verification");
    repo.enable_exact_candidate_v2(operation_id);
    let truth = exact_verification_truth(operation_id);
    repo.set_verification_truth(truth.clone());
    let mut consolidation = consolidation_for(&truth, "pending_enrichment", None);
    consolidation.accepted_fact_delta_count = 1;
    consolidation.pending_enrichment_count = 1;
    repo.set_wave_consolidation(consolidation);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    orchestrator
        .consume_gate_outcome(operation_id, pass(StageKind::Verification))
        .await;

    assert_eq!(
        orchestrator.stage_outcome_acc,
        Some(crate::harness::operation_flow::StageFlowOutcome::blocked())
    );
    assert_eq!(repo.wave_consolidation_calls.load(Ordering::SeqCst), 1);
    let events = drain(&mut rx);
    assert!(events.iter().any(|event| matches!(
        event,
        AiEvent::HarnessTrace {
            trace: golish_core::events::HarnessTraceKind::AttackWaveConsolidated {
                decision_kind,
                target_wave_run_id: None,
                pending_enrichment_count: 1,
                ..
            },
            ..
        } if decision_kind == "pending_enrichment"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        AiEvent::HarnessTrace {
            trace: golish_core::events::HarnessTraceKind::GateDecision {
                gate,
                first_blocking_reason: Some(reason),
                ..
            },
            ..
        } if gate == "BLOCK" && reason.contains("ATTACK_FACT_DELTA_ENRICHMENT_REQUIRED")
    )));
}

#[tokio::test]
async fn exact_v2_closed_no_delta_routes_to_access_validation_only_for_verified_finding_truth() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", "verification");
    repo.enable_exact_candidate_v2(operation_id);
    let truth = exact_verification_truth_with_attempts(operation_id, vec![verified_attempt()]);
    repo.set_verification_truth(truth.clone());
    repo.set_wave_consolidation(consolidation_for(&truth, "closed_no_delta", None));
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo, Uuid::new_v4(), tx);

    orchestrator
        .consume_gate_outcome(operation_id, pass(StageKind::Verification))
        .await;

    let flow = orchestrator.stage_outcome_acc.expect("flow outcome");
    assert!(flow.gate_allowed);
    assert!(
        flow.made_progress,
        "verified Finding truth takes the main branch"
    );
    assert!(!flow.reopen_wave);
}

#[tokio::test]
async fn exact_v2_closed_no_delta_routes_all_refuted_and_blocked_truth_to_reporting() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", "verification");
    repo.enable_exact_candidate_v2(operation_id);
    let truth = exact_verification_truth_with_attempts(
        operation_id,
        vec![refuted_attempt(), blocked_attempt()],
    );
    repo.set_verification_truth(truth.clone());
    repo.set_wave_consolidation(consolidation_for(&truth, "closed_no_delta", None));
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo, Uuid::new_v4(), tx);

    orchestrator
        .consume_gate_outcome(operation_id, pass(StageKind::Verification))
        .await;

    let flow = orchestrator.stage_outcome_acc.expect("flow outcome");
    assert!(flow.gate_allowed);
    assert!(
        !flow.made_progress,
        "terminal non-findings must not be promoted into access validation"
    );
    assert!(!flow.reopen_wave);
}

#[tokio::test]
async fn exact_v2_exhausted_always_routes_to_reporting_even_with_verified_truth_and_residuals() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", "verification");
    repo.enable_exact_candidate_v2(operation_id);
    let truth = exact_verification_truth_with_attempts(operation_id, vec![verified_attempt()]);
    repo.set_verification_truth(truth.clone());
    let mut consolidation = consolidation_for(&truth, "exhausted", None);
    consolidation.accepted_fact_delta_count = 2;
    consolidation.residual_risk_count = 3;
    repo.set_wave_consolidation(consolidation);
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo, Uuid::new_v4(), tx);

    orchestrator
        .consume_gate_outcome(operation_id, pass(StageKind::Verification))
        .await;

    let flow = orchestrator.stage_outcome_acc.expect("flow outcome");
    assert!(flow.gate_allowed);
    assert!(
        !flow.made_progress,
        "fuel exhaustion is terminal and must take the normal Reporting branch"
    );
    assert!(!flow.reopen_wave);
    assert!(flow.durable_wave_cursor);
}

#[tokio::test]
async fn exact_v2_verification_consolidation_failure_blocks() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", "verification");
    repo.enable_exact_candidate_v2(operation_id);
    let truth = exact_verification_truth(operation_id);
    repo.set_verification_truth(truth);
    repo.fail_next_wave_consolidation();
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    orchestrator
        .consume_gate_outcome(operation_id, pass(StageKind::Verification))
        .await;

    assert_eq!(
        orchestrator.stage_outcome_acc,
        Some(crate::harness::operation_flow::StageFlowOutcome::blocked())
    );
    assert_eq!(repo.wave_consolidation_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn legacy_verification_never_calls_durable_consolidation() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", "verification");
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    let mut outcome = pass(StageKind::Verification);
    outcome.spawned_candidates = vec![candidate_for_test("legacy hypothesis")];

    orchestrator
        .consume_gate_outcome(operation_id, outcome)
        .await;

    assert!(
        orchestrator
            .stage_outcome_acc
            .expect("flow outcome")
            .reopen_wave
    );
    assert_eq!(repo.wave_consolidation_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn info_stage_evidence_counts_as_progress_without_findings() {
    let outcome = pass_without_findings(StageKind::ExternalAttackSurface, vec![9592, 9591]);

    assert!(
        super::gate_outcome_made_progress(&outcome),
        "EAS/other recon stages suppress findings; evidence handoff still means progress"
    );
}

#[test]
fn vulnerability_stage_evidence_counts_as_progress_without_findings() {
    let outcome = pass_without_findings(StageKind::VulnTriage, vec![42]);

    assert!(
        super::gate_outcome_made_progress(&outcome),
        "VulnTriage suppresses findings; authoritative scan evidence must still feed the Candidate path"
    );
}

#[tokio::test]
async fn v2_gate_close_rejects_pass_without_durable_submission_capture() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "red_team", "scoping");
    repo.op_state
        .lock()
        .unwrap()
        .get_mut(&op)
        .unwrap()
        .runtime_memory_contract =
        crate::runtime_memory::RuntimeMemoryContract::DualWriteLegacyRead;
    let stage_execution_id = repo.active_stage_execution_id(op).unwrap();
    let (tx, _rx) = mpsc::unbounded_channel();
    let orch = TaskOrchestrator::new(repo.clone(), repo, Uuid::new_v4(), tx);
    let exec_ctx = ExecutionContext {
        operation_id: Some(op),
        stage_execution_id: Some(stage_execution_id),
        stage_run_unit_id: Some(Uuid::new_v4()),
        ..ExecutionContext::default()
    };
    let mut outcome = pass(StageKind::Scoping);

    orch.enforce_trusted_submission(&mut outcome, &exec_ctx)
        .await;

    assert!(!outcome.gate_allowed);
    assert!(outcome
        .gate_reasons
        .iter()
        .any(|reason| reason.contains("durable deliverable submission")));
}

#[tokio::test]
async fn v2_gate_close_accepts_exact_scoped_immutable_submission() {
    use sha2::{Digest, Sha256};

    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "red_team", "scoping");
    repo.op_state
        .lock()
        .unwrap()
        .get_mut(&op)
        .unwrap()
        .runtime_memory_contract =
        crate::runtime_memory::RuntimeMemoryContract::DualWriteLegacyRead;
    let stage_execution_id = repo.active_stage_execution_id(op).unwrap();
    let stage_run_unit_id = Uuid::new_v4();
    let submission_id = Uuid::new_v4();
    let canonical = r#"{"claims":[],"evidence":[],"findings":[],"stage":"scoping"}"#;
    let payload_sha256 = Sha256::digest(canonical.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    repo.submissions.lock().unwrap().insert(
        submission_id,
        PersistedStageDeliverableSubmission {
            deliverable_submission_id: submission_id,
            operation_id: op,
            stage_execution_id,
            stage_run_unit_id: Some(stage_run_unit_id),
            worker_run_id: Some(Uuid::new_v4()),
            organization_id: None,
            tool_call_record_id: Uuid::new_v4(),
            tool_request_id: "trusted-tool-request".to_string(),
            stage_kind: "scoping".to_string(),
            attempt_epoch: Some(1),
            lease_token: Some(Uuid::new_v4()),
            payload: serde_json::from_str(canonical).unwrap(),
            payload_sha256: payload_sha256.clone(),
        },
    );
    let (tx, _rx) = mpsc::unbounded_channel();
    let orch = TaskOrchestrator::new(repo.clone(), repo, Uuid::new_v4(), tx);
    let exec_ctx = ExecutionContext {
        operation_id: Some(op),
        stage_execution_id: Some(stage_execution_id),
        stage_run_unit_id: Some(stage_run_unit_id),
        ..ExecutionContext::default()
    };
    let mut outcome = pass(StageKind::Scoping);
    outcome.trusted_submission = Some(CapturedStageSubmission {
        deliverable_submission_id: submission_id,
        operation_id: op,
        stage_execution_id,
        stage_run_unit_id: Some(stage_run_unit_id),
        canonical_deliverable_json: canonical.to_string(),
        payload_sha256,
    });

    orch.enforce_trusted_submission(&mut outcome, &exec_ctx)
        .await;

    assert!(outcome.gate_allowed, "{:?}", outcome.gate_reasons);
}

/// Entering a different stage rotates the exact durable stage-execution id and
/// clears identities owned by the previous execution. Re-entering that same
/// stage must retain the id: a resume is not a new execution attempt and must
/// not refresh the durable freshness anchor.
#[tokio::test]
async fn stage_entry_sync_rotates_execution_id_once_and_keeps_same_stage_stable() {
    use golish_core::agent_session::WorkerLeaseContext;

    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "red_team", "target_intel");
    let initial_execution_id = repo.active_stage_execution_id(op).unwrap();
    let unit_id = Uuid::new_v4();
    let (tx, _rx) = mpsc::unbounded_channel();
    let orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    let mut exec_ctx = ExecutionContext {
        operation_id: Some(op),
        stage_execution_id: Some(initial_execution_id),
        stage_run_unit_id: Some(unit_id),
        worker_lease: Some(WorkerLeaseContext {
            worker_run_id: Uuid::new_v4(),
            stage_run_unit_id: unit_id,
            lease_token: Uuid::new_v4(),
            attempt_epoch: 4,
        }),
        ..ExecutionContext::default()
    };

    orch.sync_stage_execution_on_entry(&mut exec_ctx, op, StageKind::ExternalAttackSurface)
        .await
        .expect("stage transition must persist");

    assert_eq!(repo.stage(op).as_deref(), Some("external_attack_surface"));
    let rotated_execution_id = repo.active_stage_execution_id(op).unwrap();
    assert_ne!(rotated_execution_id, initial_execution_id);
    assert_eq!(exec_ctx.stage_execution_id, Some(rotated_execution_id));
    assert_eq!(exec_ctx.stage_run_unit_id, None);
    assert_eq!(exec_ctx.worker_lease, None);
    assert_eq!(
        repo.stage_transition_count(),
        1,
        "entering a different stage should rotate the durable identity once"
    );

    orch.sync_stage_execution_on_entry(&mut exec_ctx, op, StageKind::ExternalAttackSurface)
        .await
        .expect("same-stage resume must load the current execution");

    assert_eq!(
        repo.active_stage_execution_id(op),
        Some(rotated_execution_id)
    );
    assert_eq!(exec_ctx.stage_execution_id, Some(rotated_execution_id));
    assert_eq!(
        repo.stage_transition_count(),
        1,
        "same-stage resume must not rotate or refresh the stage execution"
    );
}

#[tokio::test]
async fn stage_entry_sync_failure_leaves_database_and_context_on_previous_execution() {
    use golish_core::agent_session::WorkerLeaseContext;

    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "red_team", "target_intel");
    let initial_execution_id = repo.active_stage_execution_id(op).unwrap();
    let unit_id = Uuid::new_v4();
    let lease = WorkerLeaseContext {
        worker_run_id: Uuid::new_v4(),
        stage_run_unit_id: unit_id,
        lease_token: Uuid::new_v4(),
        attempt_epoch: 9,
    };
    let (tx, _rx) = mpsc::unbounded_channel();
    let orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    let mut exec_ctx = ExecutionContext {
        operation_id: Some(op),
        stage_execution_id: Some(initial_execution_id),
        stage_run_unit_id: Some(unit_id),
        worker_lease: Some(lease.clone()),
        ..ExecutionContext::default()
    };
    repo.fail_next_stage_transition();

    let error = orch
        .sync_stage_execution_on_entry(&mut exec_ctx, op, StageKind::ExternalAttackSurface)
        .await
        .expect_err("a failed atomic transition must stop stage entry");

    assert!(format!("{error:#}").contains("injected stage transition failure"));
    assert_eq!(repo.stage(op).as_deref(), Some("target_intel"));
    assert_eq!(
        repo.active_stage_execution_id(op),
        Some(initial_execution_id)
    );
    assert_eq!(repo.stage_transition_count(), 0);
    assert_eq!(exec_ctx.stage_execution_id, Some(initial_execution_id));
    assert_eq!(exec_ctx.stage_run_unit_id, Some(unit_id));
    assert_eq!(exec_ctx.worker_lease, Some(lease));
}

#[tokio::test]
async fn fresh_executor_run_rejects_initial_execution_that_is_not_exactly_active() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "red_team", "target_intel");
    let expected_but_inactive_id = Uuid::new_v4();
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    let executor = ExhaustedStageRunExecutor {
        execute_calls: AtomicUsize::new(0),
    };

    let error = orch
        .run_executor_driven(
            op,
            &[],
            &executor,
            false,
            None,
            Some(expected_but_inactive_id),
        )
        .await
        .expect_err("fresh execution must validate the id returned by atomic create");

    assert!(format!("{error:#}").contains("initial stage execution that is not active"));
    assert_eq!(executor.execute_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn resume_fails_before_stage_work_when_exact_active_execution_is_missing() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "red_team", "target_intel");
    repo.active_stage_executions.lock().unwrap().remove(&op);
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo, Uuid::new_v4(), tx);
    let executor = ExhaustedStageRunExecutor {
        execute_calls: AtomicUsize::new(0),
    };

    let error = orch
        .run_executor_driven(op, &[], &executor, true, Some("continue"), None)
        .await
        .expect_err("resume must load an exact active execution from durable storage");

    assert!(format!("{error:#}").contains("stage_runs"));
    assert_eq!(executor.execute_calls.load(Ordering::SeqCst), 0);
}

/// Company-only input without a clickable coordinator cannot promote passive
/// discovery. It holds with the dedicated target-scope status instead of
/// pretending that a generic approval can manufacture target authority.
#[tokio::test]
async fn red_team_pre_eas_barrier_holds_company_only_scope_before_phase_approval() {
    use crate::harness::operation_flow::StageFlowOutcome;

    let op = Uuid::new_v4();
    let org_id = Uuid::new_v4();
    let repo = MemRepo::seed(op, "red_team", "target_intel");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    orch.set_harness_org_id(Some(org_id));

    let graph = crate::harness::base_operation_graph().unwrap();
    let profile = crate::harness::load_embedded_profile("red_team")
        .unwrap()
        .unwrap();
    let dag = graph.project(&profile.allowed_stage_set());

    // A generic affirmative phase reply must never authorize an empty trusted
    // target snapshot. The deterministic barrier must hold before consuming it.
    orch.user_input_sender()
        .send("approve".to_string())
        .unwrap();
    let decision = orch
        .two_level_phase_gate(
            op,
            StageKind::TargetIntel,
            &StageFlowOutcome::pass_with_progress(),
            &dag,
            Some(&profile),
        )
        .await;

    assert!(matches!(decision, super::PhaseGateDecision::Held));
    assert_eq!(repo.stage_transition_count(), 0);
    assert!(repo.submissions.lock().unwrap().is_empty());
    assert_eq!(repo.candidate_review_reads.load(Ordering::SeqCst), 0);
    assert!(drain(&mut rx).iter().any(|event| matches!(
        event,
        AiEvent::TaskProgress { status, message, .. }
            if status == "waiting_target_scope"
                && message.contains("ACTIVE_RECON_TRUSTED_TARGET_REQUIRED")
    )));
}

#[tokio::test]
async fn active_recon_scope_review_accepts_subset_and_skips_generic_phase_approval() {
    use crate::harness::operation_flow::StageFlowOutcome;
    use golish_core::hitl::ApprovalDecision;

    let op = Uuid::new_v4();
    let org_id = Uuid::new_v4();
    let repo = MemRepo::seed(op, "red_team", "target_intel");
    let presented = vec![
        scoping_target("a.example", "in"),
        scoping_target("b.example", "in"),
    ];
    let selected = vec![scoping_target("b.example", "in")];
    repo.set_active_recon_scope_candidates(op, org_id, presented.clone());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    orch.set_harness_org_id(Some(org_id));
    orch.set_current_invocation_target_authority(Some(false));
    let coordinator = crate::EventCoordinator::spawn(
        "active-recon-scope-review".to_string(),
        Arc::new(GateMockRuntime),
        None,
    );
    orch.set_approval_coordinator(Some(coordinator.clone()));

    let graph = crate::harness::base_operation_graph().expect("embedded operation graph");
    let profile = crate::harness::load_embedded_profile("red_team")
        .expect("load profile")
        .expect("red_team profile");
    let dag = graph.project(&profile.allowed_stage_set());
    let outcome = StageFlowOutcome::pass_with_progress();

    let responder = async {
        let mut saw_waiting_target_scope = false;
        loop {
            match rx.recv().await {
                Some(AiEvent::TaskProgress { status, .. }) => {
                    assert_ne!(status, "waiting_approval");
                    saw_waiting_target_scope |= status == "waiting_target_scope";
                }
                Some(AiEvent::AskHumanRequest {
                    request_id,
                    input_type,
                    context,
                    ..
                }) => {
                    assert_eq!(input_type, "scope_review");
                    let rows: Vec<ScopingReviewedTarget> =
                        serde_json::from_str(&context).expect("exact review rows");
                    assert_eq!(rows, presented);
                    coordinator.resolve_approval(ApprovalDecision {
                        request_id,
                        approved: true,
                        reason: Some(serde_json::to_string(&selected).unwrap()),
                        remember: false,
                        always_allow: false,
                    });
                    return saw_waiting_target_scope;
                }
                Some(_) => {}
                None => panic!("event channel closed before target-scope review"),
            }
        }
    };
    let gate =
        orch.two_level_phase_gate(op, StageKind::TargetIntel, &outcome, &dag, Some(&profile));
    let (decision, saw_waiting_target_scope) = tokio::join!(gate, responder);

    assert!(matches!(decision, super::PhaseGateDecision::Allowed));
    assert!(saw_waiting_target_scope);
    assert_eq!(
        repo.active_recon_scope_apply_calls.load(Ordering::SeqCst),
        1
    );
    assert_eq!(
        repo.scoping_target_snapshots.lock().unwrap().get(&org_id),
        Some(&selected)
    );
    assert!(!saw_waiting_approval(&drain(&mut rx)));
}

#[tokio::test]
async fn active_recon_scope_review_rejects_edited_target_without_persisting() {
    use crate::harness::operation_flow::StageFlowOutcome;
    use golish_core::hitl::ApprovalDecision;

    let op = Uuid::new_v4();
    let org_id = Uuid::new_v4();
    let repo = MemRepo::seed(op, "red_team", "target_intel");
    repo.set_active_recon_scope_candidates(op, org_id, vec![scoping_target("a.example", "in")]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    orch.set_harness_org_id(Some(org_id));
    let coordinator = crate::EventCoordinator::spawn(
        "active-recon-scope-review-edited".to_string(),
        Arc::new(GateMockRuntime),
        None,
    );
    orch.set_approval_coordinator(Some(coordinator.clone()));
    let graph = crate::harness::base_operation_graph().unwrap();
    let profile = crate::harness::load_embedded_profile("red_team")
        .unwrap()
        .unwrap();
    let dag = graph.project(&profile.allowed_stage_set());
    let outcome = StageFlowOutcome::pass_with_progress();

    let responder = async {
        let request_id = recv_ask_human_request_id(&mut rx).await;
        coordinator.resolve_approval(ApprovalDecision {
            request_id,
            approved: true,
            reason: Some(
                serde_json::to_string(&vec![scoping_target("new.example", "in")]).unwrap(),
            ),
            remember: false,
            always_allow: false,
        });
    };
    let gate =
        orch.two_level_phase_gate(op, StageKind::TargetIntel, &outcome, &dag, Some(&profile));
    let (decision, ()) = tokio::join!(gate, responder);

    assert!(matches!(decision, super::PhaseGateDecision::Held));
    assert_eq!(
        repo.active_recon_scope_apply_calls.load(Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn persisted_confirmed_org_only_launch_still_holds_historical_target_on_resume() {
    use crate::harness::operation_flow::StageFlowOutcome;

    let op = Uuid::new_v4();
    let org_id = Uuid::new_v4();
    let repo = MemRepo::seed(op, "red_team", "target_intel");
    repo.set_scoping_target_snapshot(org_id, vec![company_parity_target("in")]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    orch.set_harness_org_id(Some(org_id));
    let persisted = crate::task_orchestrator::harness_resume::state_blob_with_current_invocation_target_authority(
        serde_json::json!({"profile": "red_team", "current_stage": "target_intel"}),
        false,
    );
    let restored = crate::task_orchestrator::harness_resume::current_invocation_target_authority_from_state_blob(&persisted)
        .expect("restore server-owned fresh authority marker");
    orch.set_current_invocation_target_authority(restored);

    let graph = crate::harness::base_operation_graph().expect("embedded operation graph");
    let profile = crate::harness::load_embedded_profile("red_team")
        .expect("load profile")
        .expect("red_team profile");
    let dag = graph.project(&profile.allowed_stage_set());
    orch.user_input_sender()
        .send("approve".to_string())
        .expect("queue generic phase reply that must remain unconsumed");

    let decision = orch
        .two_level_phase_gate(
            op,
            StageKind::TargetIntel,
            &StageFlowOutcome::pass_with_progress(),
            &dag,
            Some(&profile),
        )
        .await;

    assert!(matches!(decision, super::PhaseGateDecision::Held));
    assert_eq!(repo.stage_transition_count(), 0);
    assert!(repo.submissions.lock().unwrap().is_empty());
    assert_eq!(repo.candidate_review_reads.load(Ordering::SeqCst), 0);
    assert_eq!(
        repo.scoping_target_snapshot_reads.load(Ordering::SeqCst),
        0,
        "confirmed-organization-only authority must HOLD before reading historical target rows"
    );
    assert!(drain(&mut rx).iter().any(|event| matches!(
        event,
        AiEvent::TaskProgress { status, message, .. }
            if status == "waiting_target_scope"
                && message.contains("ACTIVE_RECON_TRUSTED_TARGET_REQUIRED")
                && message.contains("current CLI invocation")
    )));
}

#[tokio::test]
async fn direct_eas_stage_slice_without_trusted_target_holds_before_stage_work() {
    let op = Uuid::new_v4();
    let org_id = Uuid::new_v4();
    let repo = MemRepo::seed(op, "red_team", "external_attack_surface");
    let initial_stage_execution_id = repo
        .active_stage_execution_id(op)
        .expect("fixture has an active EAS execution shell");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    orch.set_harness_org_id(Some(org_id));
    orch.set_stage_allowlist(Some(std::collections::HashSet::from([
        StageKind::ExternalAttackSurface,
    ])));
    let executor = ExhaustedStageRunExecutor {
        execute_calls: AtomicUsize::new(0),
    };

    let paused = orch
        .run_executor_driven(
            op,
            &[],
            &executor,
            false,
            Some(COMPANY_PARITY_FIXTURE),
            Some(initial_stage_execution_id),
        )
        .await
        .expect("an untrusted direct EAS slice must pause cleanly");

    assert!(!paused.trim().is_empty());
    assert_eq!(executor.execute_calls.load(Ordering::SeqCst), 0);
    assert_eq!(repo.stage_transition_count(), 0);
    assert!(repo.submissions.lock().unwrap().is_empty());
    assert_eq!(repo.candidate_review_reads.load(Ordering::SeqCst), 0);
    assert_eq!(
        repo.active_stage_execution_id(op),
        Some(initial_stage_execution_id),
        "the preflight must not rotate the initial resumable EAS shell"
    );
    assert!(drain(&mut rx).iter().any(|event| matches!(
        event,
        AiEvent::TaskProgress { status, message, .. }
            if status == "waiting_target_scope"
                && message.contains("ACTIVE_RECON_TRUSTED_TARGET_REQUIRED")
    )));
}

#[tokio::test]
async fn direct_eas_entry_accepts_company_fixture_with_exact_in_scope_target() {
    let op = Uuid::new_v4();
    let org_id = Uuid::new_v4();
    let repo = MemRepo::seed(op, "red_team", "external_attack_surface");
    repo.set_scoping_target_snapshot(org_id, vec![company_parity_target("in")]);
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    orch.set_harness_org_id(Some(org_id));
    orch.set_current_invocation_target_authority(Some(true));

    assert!(orch.active_recon_trusted_target_ready(op).await);
    assert_eq!(
        repo.scoping_target_snapshot_reads.load(Ordering::SeqCst),
        1,
        "confirmed exact-target authority must still be verified against DB truth"
    );
}

#[tokio::test]
async fn red_team_loopback_target_passes_pre_eas_barrier_without_duplicate_approval() {
    use crate::harness::operation_flow::StageFlowOutcome;

    let op = Uuid::new_v4();
    let org_id = Uuid::new_v4();
    let repo = MemRepo::seed(op, "red_team", "target_intel");
    repo.set_scoping_target_snapshot(org_id, vec![company_parity_target("in")]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo, Uuid::new_v4(), tx);
    orch.set_harness_org_id(Some(org_id));

    let graph = crate::harness::base_operation_graph().expect("embedded operation graph");
    let profile = crate::harness::load_embedded_profile("red_team")
        .expect("load profile")
        .expect("red_team profile");
    let dag = graph.project(&profile.allowed_stage_set());
    let decision = orch
        .two_level_phase_gate(
            op,
            StageKind::TargetIntel,
            &StageFlowOutcome::pass_with_progress(),
            &dag,
            Some(&profile),
        )
        .await;

    assert!(matches!(decision, super::PhaseGateDecision::Allowed));
    assert!(!saw_waiting_approval(&drain(&mut rx)));
}

#[tokio::test]
async fn pre_eas_barrier_treats_only_out_of_scope_targets_as_empty() {
    use crate::harness::operation_flow::StageFlowOutcome;

    let op = Uuid::new_v4();
    let org_id = Uuid::new_v4();
    let repo = MemRepo::seed(op, "pentest", "target_intel");
    repo.set_scoping_target_snapshot(org_id, vec![scoping_target("example.test", "out")]);
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo, Uuid::new_v4(), tx);
    orch.set_harness_org_id(Some(org_id));

    let graph = crate::harness::base_operation_graph().unwrap();
    let profile = crate::harness::load_embedded_profile("pentest")
        .unwrap()
        .unwrap();
    let dag = graph.project(&profile.allowed_stage_set());
    orch.user_input_sender()
        .send("approve".to_string())
        .unwrap();

    let decision = orch
        .two_level_phase_gate(
            op,
            StageKind::TargetIntel,
            &StageFlowOutcome::pass_with_progress(),
            &dag,
            Some(&profile),
        )
        .await;

    assert!(matches!(decision, super::PhaseGateDecision::Held));
}

#[tokio::test]
async fn pre_eas_barrier_fails_closed_when_trusted_snapshot_read_fails() {
    use crate::harness::operation_flow::StageFlowOutcome;

    let op = Uuid::new_v4();
    let org_id = Uuid::new_v4();
    let repo = MemRepo::seed(op, "pentest", "target_intel");
    repo.fail_next_scoping_target_snapshot();
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo, Uuid::new_v4(), tx);
    orch.set_harness_org_id(Some(org_id));

    let graph = crate::harness::base_operation_graph().unwrap();
    let profile = crate::harness::load_embedded_profile("pentest")
        .unwrap()
        .unwrap();
    let dag = graph.project(&profile.allowed_stage_set());

    let decision = orch
        .two_level_phase_gate(
            op,
            StageKind::TargetIntel,
            &StageFlowOutcome::pass_with_progress(),
            &dag,
            Some(&profile),
        )
        .await;

    assert!(matches!(decision, super::PhaseGateDecision::Held));
}

#[tokio::test]
async fn two_level_phase_gate_ignores_non_affirmative_reply_after_scoping() {
    use crate::harness::operation_flow::StageFlowOutcome;
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "pentest", "enumeration");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    let graph = crate::harness::base_operation_graph().unwrap();
    let profile = crate::harness::load_embedded_profile("pentest")
        .unwrap()
        .unwrap();
    let dag = graph.project(&profile.allowed_stage_set());

    orch.user_input_sender().send("no".to_string()).unwrap();
    let decision = orch
        .two_level_phase_gate(
            op,
            StageKind::Enumeration,
            &StageFlowOutcome::pass_with_progress(),
            &dag,
            Some(&profile),
        )
        .await;

    assert!(
        matches!(decision, super::PhaseGateDecision::Allowed),
        "post-Scoping phase progression must not depend on a queued human reply"
    );
    assert!(!saw_waiting_approval(&drain(&mut rx)));
}

#[tokio::test]
async fn two_level_phase_gate_auto_advances_after_scoping_without_reply() {
    use crate::harness::operation_flow::StageFlowOutcome;
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "pentest", "enumeration");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    let graph = crate::harness::base_operation_graph().unwrap();
    let profile = crate::harness::load_embedded_profile("pentest")
        .unwrap()
        .unwrap();
    let dag = graph.project(&profile.allowed_stage_set());

    let outcome = StageFlowOutcome::pass_with_progress();
    let decision = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        orch.two_level_phase_gate(op, StageKind::Enumeration, &outcome, &dag, Some(&profile)),
    )
    .await
    .expect("post-Scoping phase progression must not wait for user input");

    assert!(
        matches!(decision, super::PhaseGateDecision::Allowed),
        "Enumeration Gate PASS must auto-advance into the next projected stage"
    );
    assert!(!saw_waiting_approval(&drain(&mut rx)));
}

#[tokio::test]
async fn review_barrier_holds_attack_candidate_until_the_exact_db_wave_is_resumed() {
    use crate::harness::operation_flow::StageFlowOutcome;

    let operation_id = Uuid::new_v4();
    let wave_run_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", "attack_candidate");
    repo.enable_exact_candidate_v2(operation_id);
    let barrier = AttackV2ReviewBarrierView {
        operation_id,
        wave_run_id,
        status: "open".to_string(),
        resume_version: 1,
        wave_unit_count: 2,
        review_closed_unit_count: 1,
        candidate_count: 2,
        proposed_candidate_count: 1,
        dispatch_is_stale: false,
    };
    repo.set_candidate_review_barrier(barrier.clone());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    let graph = crate::harness::base_operation_graph().unwrap();
    let profile = crate::harness::load_embedded_profile("red_team")
        .unwrap()
        .unwrap();
    let dag = graph.project(&profile.allowed_stage_set());

    let held = orchestrator
        .two_level_phase_gate(
            operation_id,
            StageKind::AttackCandidate,
            &StageFlowOutcome::pass_with_progress(),
            &dag,
            Some(&profile),
        )
        .await;
    assert!(matches!(held, super::PhaseGateDecision::Held));
    assert!(drain(&mut rx).iter().any(|event| matches!(
        event,
        AiEvent::HarnessTrace {
            trace: golish_core::events::HarnessTraceKind::CandidateReviewRequired {
                wave_run_id: emitted_wave,
                ..
            },
            ..
        } if emitted_wave == &wave_run_id.to_string()
    )));

    repo.set_candidate_review_barrier(AttackV2ReviewBarrierView {
        status: "resumed".to_string(),
        proposed_candidate_count: 0,
        review_closed_unit_count: 2,
        resume_version: 3,
        ..barrier
    });
    let allowed = orchestrator
        .two_level_phase_gate(
            operation_id,
            StageKind::AttackCandidate,
            &StageFlowOutcome::pass_with_progress(),
            &dag,
            Some(&profile),
        )
        .await;
    assert!(matches!(allowed, super::PhaseGateDecision::Allowed));
    assert_eq!(
        repo.candidate_review_reads.load(Ordering::SeqCst),
        2,
        "exact V2Only must reload the durable barrier on both hold and resume"
    );
}

#[tokio::test]
async fn v2_only_terminal_attack_candidate_slice_never_reads_review_barrier() {
    use crate::harness::operation_flow::StageFlowOutcome;

    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", "attack_candidate");
    repo.enable_exact_candidate_v2(operation_id);
    repo.set_candidate_review_barrier(AttackV2ReviewBarrierView {
        operation_id,
        wave_run_id: Uuid::new_v4(),
        status: "open".to_string(),
        resume_version: 1,
        wave_unit_count: 1,
        review_closed_unit_count: 0,
        candidate_count: 1,
        proposed_candidate_count: 1,
        dispatch_is_stale: false,
    });
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    let graph = crate::harness::base_operation_graph().expect("embedded operation graph");
    let dag = graph.project(&std::collections::HashSet::from([
        StageKind::AttackCandidate,
    ]));

    let decision = orchestrator
        .two_level_phase_gate(
            operation_id,
            StageKind::AttackCandidate,
            &StageFlowOutcome::pass_with_progress(),
            &dag,
            None,
        )
        .await;

    assert!(matches!(decision, super::PhaseGateDecision::Allowed));
    assert_eq!(
        repo.candidate_review_reads.load(Ordering::SeqCst),
        0,
        "a terminal Candidate slice has no Verification crossing to review"
    );
}

#[tokio::test]
async fn legacy_attack_candidate_never_reads_the_v2_review_barrier() {
    use crate::harness::operation_flow::StageFlowOutcome;

    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", "attack_candidate");
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    let graph = crate::harness::base_operation_graph().unwrap();
    let profile = crate::harness::load_embedded_profile("red_team")
        .unwrap()
        .unwrap();
    let dag = graph.project(&profile.allowed_stage_set());

    let decision = orchestrator
        .two_level_phase_gate(
            operation_id,
            StageKind::AttackCandidate,
            &StageFlowOutcome::pass_with_progress(),
            &dag,
            Some(&profile),
        )
        .await;

    assert!(matches!(decision, super::PhaseGateDecision::Allowed));
    assert_eq!(repo.candidate_review_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn dual_write_attack_candidate_synthesizes_v2_but_never_reads_the_blocking_review_barrier() {
    use crate::harness::operation_flow::StageFlowOutcome;

    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", "attack_candidate");
    repo.op_state
        .lock()
        .unwrap()
        .get_mut(&operation_id)
        .unwrap()
        .runtime_memory_contract =
        crate::runtime_memory::RuntimeMemoryContract::DualWriteV2Preferred;
    *repo.attack_execution_contract.lock().unwrap() =
        golish_core::AttackExecutionContract::DualWriteReadV2Fallback;
    repo.set_candidate_review_barrier(AttackV2ReviewBarrierView {
        operation_id,
        wave_run_id: Uuid::new_v4(),
        status: "open".to_string(),
        resume_version: 1,
        wave_unit_count: 1,
        review_closed_unit_count: 0,
        candidate_count: 1,
        proposed_candidate_count: 1,
        dispatch_is_stale: false,
    });
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orchestrator = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);
    let graph = crate::harness::base_operation_graph().unwrap();
    let profile = crate::harness::load_embedded_profile("red_team")
        .unwrap()
        .unwrap();
    let dag = graph.project(&profile.allowed_stage_set());

    assert!(
        orchestrator
            .candidate_v2_specialist_for_operation(
                operation_id,
                StageKind::AttackCandidate,
                "dual_write_test",
            )
            .await,
        "dual-write must retain relational Candidate synthesis/mirror"
    );
    let decision = orchestrator
        .two_level_phase_gate(
            operation_id,
            StageKind::AttackCandidate,
            &StageFlowOutcome::pass_with_progress(),
            &dag,
            Some(&profile),
        )
        .await;

    assert!(matches!(decision, super::PhaseGateDecision::Allowed));
    assert_eq!(
        repo.candidate_review_reads.load(Ordering::SeqCst),
        0,
        "shadow data must not become a blocking authority before double V2Only cutover"
    );
}

#[tokio::test]
async fn dual_write_verification_never_enables_the_v2_verifier_specialist() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "red_team", "verification");
    repo.op_state
        .lock()
        .unwrap()
        .get_mut(&operation_id)
        .unwrap()
        .runtime_memory_contract =
        crate::runtime_memory::RuntimeMemoryContract::DualWriteV2Preferred;
    *repo.attack_execution_contract.lock().unwrap() =
        golish_core::AttackExecutionContract::DualWriteReadV2Fallback;
    let (tx, _rx) = mpsc::unbounded_channel();
    let orchestrator = TaskOrchestrator::new(repo.clone(), repo, Uuid::new_v4(), tx);

    assert!(
        !orchestrator
            .candidate_v2_specialist_for_operation(
                operation_id,
                StageKind::Verification,
                "dual_write_test",
            )
            .await,
        "dual-write Verification must remain shadow-only and never dispatch candidate_verifier"
    );
}

#[tokio::test]
async fn two_level_phase_gate_does_not_open_confirmation_card_after_scoping() {
    use crate::harness::operation_flow::StageFlowOutcome;

    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "pentest", "enumeration");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    let coordinator = crate::EventCoordinator::spawn(
        "phase-approval-test".to_string(),
        Arc::new(GateMockRuntime),
        None,
    );
    orch.set_approval_coordinator(Some(coordinator));

    let graph = crate::harness::base_operation_graph().unwrap();
    let profile = crate::harness::load_embedded_profile("pentest")
        .unwrap()
        .unwrap();
    let dag = graph.project(&profile.allowed_stage_set());
    let outcome = StageFlowOutcome::pass_with_progress();

    let decision = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        orch.two_level_phase_gate(op, StageKind::Enumeration, &outcome, &dag, Some(&profile)),
    )
    .await
    .expect("post-Scoping phase progression must not wait on the coordinator");

    assert!(matches!(decision, super::PhaseGateDecision::Allowed));
    let events = drain(&mut rx);
    assert!(!saw_waiting_approval(&events));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AiEvent::AskHumanRequest { .. })),
        "post-Scoping progression must not open a generic confirmation card"
    );
}

/// P3 RAG-prior wiring: inside a harness stage, `execute_single_subtask` pulls
/// prior writeups from the wiki KB via `retrieve_wiki_prior` and renders them as
/// the PRIOR KNOWLEDGE block prepended to the stage charter. This exercises the
/// exact wired composition (real `DbRepoProvider` trait object → parse → render)
/// end-to-end, which the rag_prior module's own tests do not cover for the
/// `retrieve_wiki_prior` entry point.
#[tokio::test]
async fn rag_prior_renders_wiki_writeups_for_stage_prompt() {
    use crate::harness::rag_prior::{render_prior_knowledge, retrieve_wiki_prior};

    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "assessment", "external_attack_surface");
    repo.set_wiki(serde_json::json!([
        {"title": "CVE-2021-44228 Log4Shell", "snippet": "JNDI lookup RCE in log4j"}
    ]));

    let pk = retrieve_wiki_prior(&*repo, "log4j rce", 5).await;
    let rendered = render_prior_knowledge(&pk);

    assert!(rendered.contains("PRIOR KNOWLEDGE"), "rendered={rendered}");
    assert!(rendered.contains("[wiki]"), "rendered={rendered}");
    assert!(rendered.contains("Log4Shell"), "rendered={rendered}");
}

/// RAG-prior degrades to an empty block when the wiki KB has no hits, so the
/// stage charter never gets a noisy/empty PRIOR KNOWLEDGE section (best-effort
/// priming must never inject empty scaffolding).
#[tokio::test]
async fn rag_prior_empty_block_when_no_wiki_hits() {
    use crate::harness::rag_prior::{render_prior_knowledge, retrieve_wiki_prior};

    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "assessment", "external_attack_surface");
    // Default canned wiki result is `Null` → parse yields zero writeups.

    let pk = retrieve_wiki_prior(&*repo, "nothing-matches", 5).await;
    assert!(
        render_prior_knowledge(&pk).is_empty(),
        "no wiki hits must render an empty prior-knowledge block"
    );
}

/// P1 · `fail_task_if_active` finalizes a still-`running` task as `failed` (with
/// the underlying error surfaced in the result) so an errored run never zombies
/// in `running`. This is the in-process counterpart to the DB startup reaper.
#[tokio::test]
async fn fail_task_if_active_marks_running_task_failed() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "assessment", "scoping");
    let task_id = repo.insert_task(TaskStatus::Running, None);
    let (tx, _rx) = mpsc::unbounded_channel();
    let orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    orch.fail_task_if_active(task_id, &anyhow::anyhow!("execution blew up"))
        .await;

    assert_eq!(repo.task_status(task_id), Some(TaskStatus::Failed));
    assert!(
        repo.task_result(task_id)
            .unwrap_or_default()
            .contains("execution blew up"),
        "the failure result must surface the underlying error"
    );
}

/// P1 · `fail_task_if_active` must NOT clobber a task that already reached a
/// terminal status — the normal completion path writes `finished` + the real
/// report, and a late best-effort finalize must leave both untouched.
#[tokio::test]
async fn fail_task_if_active_does_not_clobber_finished() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "assessment", "scoping");
    let task_id = repo.insert_task(TaskStatus::Finished, Some("real report"));
    let (tx, _rx) = mpsc::unbounded_channel();
    let orch = TaskOrchestrator::new(repo.clone(), repo.clone(), Uuid::new_v4(), tx);

    orch.fail_task_if_active(task_id, &anyhow::anyhow!("late error"))
        .await;

    assert_eq!(
        repo.task_status(task_id),
        Some(TaskStatus::Finished),
        "a terminal status must never be clobbered"
    );
    assert_eq!(repo.task_result(task_id).as_deref(), Some("real report"));
}

// ── Engagement-org isolation (设计 2026-06-15-engagement-org-isolation) ──────
// `extract_engagement_org_if_scoping` pulls the scoping-confirmed engagement root
// org id (UUID claim subject) out of a scoping deliverable; the orchestrator binds
// + persists it so downstream stages confine to that org's subtree. Pure logic.

fn scoping_deliverable_with_claim(kind: &str, subject: &str) -> crate::harness::StageDeliverable {
    crate::harness::StageDeliverable {
        stage_id: "scoping".to_string(),
        stage_run_id: Uuid::new_v4(),
        claims: vec![crate::harness::StageClaim {
            kind: kind.to_string(),
            subject: subject.to_string(),
            summary: "scope".to_string(),
            evidence_ids: vec![],
            technique: None,
        }],
        evidence_refs: vec![],
        skipped_checks: vec![],
        findings: vec![],
        required_checks_done: vec![],
        coverage: vec![],
        candidates: vec![],
        candidate_decisions: vec![],
    }
}

#[test]
fn extract_engagement_org_pulls_uuid_subject_from_scoping_claim() {
    let org = Uuid::new_v4();
    let d = scoping_deliverable_with_claim("scope_human_approved", &org.to_string());
    assert_eq!(
        super::extract_engagement_org_if_scoping(StageKind::Scoping, &d),
        Some(org),
        "a scoping deliverable's UUID claim subject is the engagement org binding"
    );
}

#[test]
fn extract_engagement_org_fails_open_on_non_uuid_or_non_scoping() {
    // Legacy domain-text subject → no binding (fail-open to whole-DB axis).
    let d = scoping_deliverable_with_claim("scope_confirmed", "example.com");
    assert_eq!(
        super::extract_engagement_org_if_scoping(StageKind::Scoping, &d),
        None,
        "non-UUID subject must not bind an engagement org"
    );
    // UUID subject but a non-scoping stage → no binding.
    let org = Uuid::new_v4();
    let d = scoping_deliverable_with_claim("scope_confirmed", &org.to_string());
    assert_eq!(
        super::extract_engagement_org_if_scoping(StageKind::TargetIntel, &d),
        None,
        "only the scoping stage binds the engagement org"
    );
}

/// Engagement-org staleness fix: `exec_ctx.harness_org_id` is a snapshot taken at
/// run start (None on the chat path, before scoping binds the org). Once scoping
/// has bound the org (`set_harness_org_id`), entering the next stage must re-sync
/// the snapshot so the bound org reaches that stage's tools + gate — otherwise
/// `manage_targets` orphans discovered assets (organization_id NULL) and the
/// submit gate skips its org-keyed DB-truth projection (coverage "never
/// attempted").
#[test]
fn sync_engagement_org_refreshes_stale_exec_ctx() {
    use crate::task_orchestrator::types::ExecutionContext;

    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "pentest", "target_intel");
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), repo, Uuid::new_v4(), tx);

    // Scoping bound the engagement root org mid-run.
    let org = Uuid::new_v4();
    orch.set_harness_org_id(Some(org));

    // exec_ctx is the stale run-start snapshot (org was not yet known then).
    let mut ctx = ExecutionContext {
        harness_org_id: None,
        ..Default::default()
    };

    orch.sync_engagement_org_into(&mut ctx);

    assert_eq!(
        ctx.harness_org_id,
        Some(org),
        "entering a stage after scoping bound the org must re-sync the stale exec_ctx snapshot"
    );
}

#[tokio::test]
async fn attack_v2_repo_defaults_fail_closed_instead_of_returning_empty_manifests() {
    let operation_id = Uuid::new_v4();
    let unit_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "pentest", "attack_candidate");

    let load_error = repo
        .attack_v2_candidate_manifest_for_unit(operation_id, unit_id, organization_id)
        .await
        .expect_err("an unavailable Candidate V2 repository must fail closed");
    assert_eq!(load_error.to_string(), "ATTACK_V2_REPO_UNAVAILABLE");

    let entry_error = repo
        .attack_v2_seed_candidate_manifest_for_unit(operation_id, unit_id, organization_id)
        .await
        .expect_err("server-only stage entry must fail closed without a repository");
    assert_eq!(entry_error.to_string(), "ATTACK_V2_REPO_UNAVAILABLE");
}

#[tokio::test]
async fn v2_only_flow_checkpointer_ignores_legacy_blob_and_never_saves_it() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "assessment", "enumeration");
    repo.enable_exact_candidate_v2(operation_id);
    repo.set_state_blob(
        operation_id,
        serde_json::json!({
            "eas_web_transport_failures": {"server-slot": {"attempts": 2}},
            "graph_flow": {
                "next_node": "reporting",
                "state": {"visited": ["scoping"], "seeded": {}, "applied": {}}
            },
            "agent_run": {"status": "running"},
            "stage_run_workers": {"legacy": true}
        }),
    );
    let checkpointer = crate::task_orchestrator::stage_execution::DbFlowCheckpointer::new(
        repo.clone(),
        operation_id,
    );

    let mut attempted = OperationFlowState::default();
    attempted.visited.push(StageKind::Scoping);
    checkpointer
        .save("ignored-thread-id", &attempted, "reporting")
        .await
        .expect("V2-only save is an idempotent no-op");
    assert_eq!(repo.state_blob_write_count.load(Ordering::SeqCst), 0);

    let (loaded, next_node) = checkpointer
        .load("ignored-thread-id")
        .await
        .expect("load V2-only relational checkpoint")
        .expect("a live relational cursor is resumable");
    assert_eq!(next_node, "enumeration");
    assert!(loaded.seeded.is_empty());
    assert!(loaded.visited.is_empty());
    assert!(loaded.applied.is_empty());
    assert_eq!(loaded.wave, 0);
    assert!(!loaded.reopen_wave);
}

#[tokio::test]
async fn normal_executor_resumes_v2_only_from_relational_stage_without_legacy_blob() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "assessment", "enumeration");
    repo.enable_exact_candidate_v2(operation_id);
    repo.set_state_blob(
        operation_id,
        serde_json::json!({
            "eas_web_transport_failures": {"server-slot": {"attempts": 1}}
        }),
    );
    let original_blob = repo.state_blob(operation_id);
    let checkpointer = Arc::new(
        crate::task_orchestrator::stage_execution::DbFlowCheckpointer::new(
            repo.clone(),
            operation_id,
        ),
    );
    let profile = crate::harness::load_embedded_profile("assessment")
        .expect("load assessment profile")
        .expect("assessment profile exists");
    let dag = crate::harness::operation_graph::base_operation_graph()
        .expect("load operation graph")
        .project(&profile.allowed_stage_set());
    let executor = Executor::new(build_operation_flow_graph(&dag).expect("compile graph"))
        .with_checkpointer(checkpointer);

    let outcome = executor
        .resume(&operation_id.to_string(), None)
        .await
        .expect("normal Executor resumes from relational V2 cursor");
    let RunOutcome::Completed(state) = outcome else {
        panic!("relational V2 resume did not complete: {outcome:?}");
    };
    assert_eq!(state.visited.first(), Some(&StageKind::Enumeration));
    assert_eq!(repo.state_blob_write_count.load(Ordering::SeqCst), 0);
    assert_eq!(repo.state_blob(operation_id), original_blob);
    for forbidden in ["graph_flow", "agent_run", "stage_run_workers"] {
        assert!(repo.state_blob(operation_id).get(forbidden).is_none());
    }
}

#[tokio::test]
async fn legacy_flow_checkpointer_keeps_existing_save_and_load_semantics() {
    let operation_id = Uuid::new_v4();
    let repo = MemRepo::seed(operation_id, "assessment", "target_intel");
    repo.set_state_blob(
        operation_id,
        serde_json::json!({"eas_web_transport_failures": {"server-slot": {"attempts": 3}}}),
    );
    let checkpointer = crate::task_orchestrator::stage_execution::DbFlowCheckpointer::new(
        repo.clone(),
        operation_id,
    );
    let mut state = OperationFlowState::default();
    state.visited.push(StageKind::Scoping);

    checkpointer
        .save("legacy-thread", &state, "target_intel")
        .await
        .expect("legacy checkpoint saves");
    assert_eq!(repo.state_blob_write_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        repo.state_blob(operation_id)["eas_web_transport_failures"]["server-slot"]["attempts"],
        3
    );
    let (loaded, next_node) = checkpointer
        .load("legacy-thread")
        .await
        .expect("legacy checkpoint loads")
        .expect("legacy graph checkpoint exists");
    assert_eq!(next_node, "target_intel");
    assert_eq!(loaded.visited, vec![StageKind::Scoping]);
}
