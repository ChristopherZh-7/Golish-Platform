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
    /// Canned `wiki_search_fts` result for the P3 RAG-prior wiring tests.
    /// Defaults to `Null` (no hits); the transition-driver tests never read it.
    wiki_result: Mutex<serde_json::Value>,
    /// In-memory `tasks` table for the P1 `fail_task_if_active` tests.
    tasks: Mutex<HashMap<Uuid, TaskView>>,
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
                state_blob: serde_json::Value::Null,
            },
        );
        Arc::new(Self {
            op_state: Mutex::new(m),
            wiki_result: Mutex::new(serde_json::Value::Null),
            tasks: Mutex::new(HashMap::new()),
        })
    }

    fn stage(&self, operation_id: Uuid) -> Option<String> {
        self.op_state
            .lock()
            .unwrap()
            .get(&operation_id)
            .map(|s| s.current_stage.clone())
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
                state_blob: serde_json::Value::Null,
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
        // P3 RAG-prior wiring tests read this; transition-driver tests never do.
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
        repair_correction: None,
        evidence_summary: None,
        evidence_refs: Vec::new(),
        required_evidence_kinds: Vec::new(),
        findings_count: 1,
    }
}

fn block(stage: StageKind) -> HarnessGateOutcome {
    HarnessGateOutcome {
        gated_stage: stage,
        gate_allowed: false,
        repair_correction: None,
        evidence_summary: None,
        evidence_refs: Vec::new(),
        required_evidence_kinds: Vec::new(),
        findings_count: 0,
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
    let mut orch = TaskOrchestrator::new(repo.clone(), Uuid::new_v4(), tx);

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

/// A gate BLOCK must NOT emit `stage_passed` — completion is gated on a real
/// PASS, so a blocked stage shows no "Stage complete".
#[tokio::test]
async fn block_emits_no_stage_passed() {
    let op = Uuid::new_v4();
    let repo = MemRepo::seed(op, "assessment", "scoping");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut orch = TaskOrchestrator::new(repo.clone(), Uuid::new_v4(), tx);

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
    let orch = TaskOrchestrator::new(repo.clone(), Uuid::new_v4(), tx);

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
    let orch = TaskOrchestrator::new(repo.clone(), Uuid::new_v4(), tx);

    orch.fail_task_if_active(task_id, &anyhow::anyhow!("late error"))
        .await;

    assert_eq!(
        repo.task_status(task_id),
        Some(TaskStatus::Finished),
        "a terminal status must never be clobbered"
    );
    assert_eq!(repo.task_result(task_id).as_deref(), Some("real report"));
}
