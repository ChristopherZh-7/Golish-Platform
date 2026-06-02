//! Closed-loop integration tests for the harness stage-transition driver.
//!
//! Exercises [`TaskOrchestrator::drive_stage_transition`] — the real glue that
//! turns a gate outcome into an `operation_state` cursor move — against an
//! in-memory `operation_state` repo and the live user-input approval channel.
//! Unlike the orchestrator's full `run()` path, the transition driver does not
//! gate on `stage_mode_enabled()`, so these tests are deterministic regardless
//! of the `GOLISH_HARNESS_STAGE_MODE` / `GOLISH_HARNESS_PROFILE` env flags.
//!
//! Covered closed-loop behaviours (Doc 3 §6.2 / C5):
//! - gate PASS walks the cursor along the profile-projected DAG, including
//!   branch first-candidate selection and terminal-stage completion;
//! - gate BLOCK holds the cursor (no advance);
//! - entering an approval-gated stage pauses on `waiting_approval`, then resumes
//!   on an affirmative reply / holds on a non-affirmative reply.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::db_traits::*;
use crate::harness::StageKind;
use crate::task_orchestrator::TaskOrchestrator;
use golish_core::events::AiEvent;

use super::HarnessGateOutcome;

/// In-memory [`DbRepoProvider`]: only the `operation_state_*` trio is real; the
/// transition driver touches nothing else, so every other method is a stub.
struct MemRepo {
    op_state: Mutex<HashMap<Uuid, OperationStateView>>,
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
            },
        );
        Arc::new(Self {
            op_state: Mutex::new(m),
        })
    }

    fn stage(&self, operation_id: Uuid) -> Option<String> {
        self.op_state
            .lock()
            .unwrap()
            .get(&operation_id)
            .map(|s| s.current_stage.clone())
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
    ) -> anyhow::Result<()> {
        self.op_state.lock().unwrap().insert(
            operation_id,
            OperationStateView {
                operation_id,
                profile: profile.to_string(),
                current_stage: current_stage.to_string(),
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
        unimplemented!()
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
    async fn task_create(&self, _task: NewTask) -> anyhow::Result<TaskView> {
        unimplemented!()
    }
    async fn task_get(&self, _id: Uuid) -> anyhow::Result<Option<TaskView>> {
        unimplemented!()
    }
    async fn task_update_status(&self, _id: Uuid, _status: TaskStatus) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn task_set_result(&self, _id: Uuid, _result: &str) -> anyhow::Result<()> {
        unimplemented!()
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

fn pass(stage: StageKind) -> HarnessGateOutcome {
    HarnessGateOutcome {
        gated_stage: stage,
        gate_allowed: true,
        repair_correction: None,
        evidence_summary: None,
        evidence_refs: Vec::new(),
    }
}

fn block(stage: StageKind) -> HarnessGateOutcome {
    HarnessGateOutcome {
        gated_stage: stage,
        gate_allowed: false,
        repair_correction: None,
        evidence_summary: None,
        evidence_refs: Vec::new(),
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

/// PASS at each stage advances the `operation_state` cursor along the
/// assessment-projected DAG: scoping → target_intel → external_attack_surface →
/// (branch first) enumeration → reporting, and a terminal PASS at reporting
/// completes without moving the cursor. Assessment's approval policy is on, so
/// the intermediate stages pause for approval — pre-feed affirmative replies.
#[tokio::test]
async fn pass_walks_cursor_along_assessment_dag() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "assessment", "scoping");
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), Uuid::new_v4(), tx);

    // target_intel / external_attack_surface / enumeration each require approval
    // under the assessment policy; reporting does not.
    let approvals = orch.user_input_sender();
    for _ in 0..3 {
        approvals.send("approve".to_string()).unwrap();
    }

    orch.drive_stage_transition(op, pass(StageKind::Scoping))
        .await;
    assert_eq!(repo.stage(op).as_deref(), Some("target_intel"));

    orch.drive_stage_transition(op, pass(StageKind::TargetIntel))
        .await;
    assert_eq!(repo.stage(op).as_deref(), Some("external_attack_surface"));

    // external_attack_surface → {enumeration, reporting}: first candidate wins.
    orch.drive_stage_transition(op, pass(StageKind::ExternalAttackSurface))
        .await;
    assert_eq!(repo.stage(op).as_deref(), Some("enumeration"));

    orch.drive_stage_transition(op, pass(StageKind::Enumeration))
        .await;
    assert_eq!(repo.stage(op).as_deref(), Some("reporting"));

    // reporting is terminal in the projected DAG → Complete → cursor unchanged.
    orch.drive_stage_transition(op, pass(StageKind::Reporting))
        .await;
    assert_eq!(repo.stage(op).as_deref(), Some("reporting"));
}

/// A blocked gate holds the cursor: no advance regardless of available
/// successors (the approval channel is never even consulted).
#[tokio::test]
async fn block_holds_cursor() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "assessment", "scoping");
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), Uuid::new_v4(), tx);

    orch.drive_stage_transition(op, block(StageKind::Scoping))
        .await;
    assert_eq!(repo.stage(op).as_deref(), Some("scoping"));
}

/// Entering an approval-gated stage (pentest: vuln_triage → verification) emits
/// `waiting_approval` and, on a non-affirmative reply, holds the cursor.
#[tokio::test]
async fn approval_gate_holds_on_non_affirmative_reply() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "pentest", "vuln_triage");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), Uuid::new_v4(), tx);

    orch.user_input_sender().send("no".to_string()).unwrap();
    orch.drive_stage_transition(op, pass(StageKind::VulnTriage))
        .await;

    assert_eq!(
        repo.stage(op).as_deref(),
        Some("vuln_triage"),
        "non-affirmative reply must hold the cursor"
    );
    assert!(saw_waiting_approval(&drain(&mut rx)));
}

/// Same approval gate, but an affirmative reply resumes the transition and
/// advances the cursor to the gated stage.
#[tokio::test]
async fn approval_gate_resumes_on_affirmative_reply() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "pentest", "vuln_triage");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), Uuid::new_v4(), tx);

    orch.user_input_sender()
        .send("approve".to_string())
        .unwrap();
    orch.drive_stage_transition(op, pass(StageKind::VulnTriage))
        .await;

    assert_eq!(
        repo.stage(op).as_deref(),
        Some("verification"),
        "affirmative reply must resume + advance the cursor"
    );
    assert!(saw_waiting_approval(&drain(&mut rx)));
}
