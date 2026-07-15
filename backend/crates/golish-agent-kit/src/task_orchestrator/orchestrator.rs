//! [`TaskOrchestrator`] entry points (`new`, `user_input_sender`, `run`,
//! `resume`) and shared event-emission helpers.
//!
//! The actual subtask execution / refinement phases live in
//! [`super::subtask_phases`] as a separate `impl TaskOrchestrator` block.

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::db_shim::{subtasks, tasks};
use crate::db_traits::{
    CliRuntimeScope, CreateRuntimeOperation, DbRepoProvider, ProjectScopeRegistration,
    RuntimeMemoryRepository, TaskStatus,
};
use golish_core::events::AiEvent;
use golish_core::plan::{PlanStep, PlanSummary, StepStatus};

use super::helpers::parse_agent_type;
use super::types::{AgentExecutor, PlannedSubtask};

/// Phase 2 (2026-06-12-redteam-phase2): when set on a stage run, the scoping
/// gate requires subsidiary discovery (an org tree landed in the DB) before it
/// PASSes. `None` = legacy scoping (no subsidiary gate; zero behaviour change).
#[derive(Debug, Clone, Copy)]
pub struct SubsidiaryScopePolicy {
    /// Minimum investment/ownership percentage for a subsidiary to be in scope.
    /// The threshold filter itself runs in the asset-intel promote layer
    /// (`auto_promote_child_decisions`); this is surfaced for prompt/diagnostics.
    pub threshold_pct: u8,
}

/// The main Task orchestrator.
///
/// Mirrors PentAGI's `taskWorker.Run()` flow:
/// ```text
/// GenerateSubtasks → loop { PopSubtask → Run → Enrich → RefineSubtasks } → GetTaskResult
/// ```
///
/// Supports:
/// - **Subtask persistence**: Each subtask gets a message chain stored in the DB.
/// - **User input pause**: Subtasks can pause and wait for user input.
/// - **Task resume**: If interrupted, the task can be resumed from the last completed subtask.
/// - **Enricher**: After each subtask, searches for additional context to inject.
pub struct TaskOrchestrator {
    pub(super) repo: Arc<dyn DbRepoProvider>,
    pub(super) runtime_repo: Arc<dyn RuntimeMemoryRepository>,
    pub(super) session_id: Uuid,
    pub(super) event_tx: mpsc::UnboundedSender<AiEvent>,
    pub(super) user_input_rx: Option<mpsc::UnboundedReceiver<String>>,
    user_input_tx: mpsc::UnboundedSender<String>,
    /// C6 · cross-stage evidence handoff store (stage_mode). Keyed by
    /// `StageKind::as_str()`; value is a compact summary of that stage's gate-
    /// passed deliverable (claims/findings/evidence counts). A downstream stage
    /// whose `inherits_evidence_from` lists an upstream stage gets the real
    /// summary injected into its subtask context, not just the static kind hint.
    pub(super) harness_evidence: std::collections::HashMap<String, String>,
    /// C7 · per-run operation profile override from the chat-panel mode picker.
    /// `None` = fall back to the `GOLISH_HARNESS_PROFILE` env default
    /// ([`crate::harness::active_profile_id`]).
    pub(super) profile_override: Option<String>,
    /// Current operation's organization id (coverage asset-axis isolation,
    /// design 2026-06-09). Passed to `in_scope_assets` lookups so the coverage
    /// gate's denominator (and the agent-facing in-scope asset prompt section)
    /// only contains THIS org's in-scope targets instead of the whole persistent
    /// DB. `None` is legal only while Scoping has not bound an engagement org;
    /// org-keyed reads and active-recon entry fail closed in that state.
    pub(super) harness_org_id: Option<Uuid>,
    /// Phase 2 (2026-06-12-redteam-phase2): subsidiary scope policy for this run.
    /// `Some` activates the scoping gate's `GOLISH-INTEL-SUBSIDIARY` coverage
    /// dimension (an org tree must land in the DB). `None` keeps legacy scoping.
    pub(super) harness_subsidiary_policy: Option<SubsidiaryScopePolicy>,
    /// P2 方案 C · accumulated [`StageFlowOutcome`] for the stage currently being
    /// run under the Executor (merged across that stage's subtasks: gate ANDed,
    /// progress ORed). Read + cleared by `run_stage_subtasks`.
    pub(super) stage_outcome_acc: Option<crate::harness::operation_flow::StageFlowOutcome>,
    /// Chat-session string (e.g. `pentest-chat-…`) used to scope evidence-ledger
    /// lookups. Both evidence write paths (sync runtime hook + background-job
    /// listener) stamp this string on `audit_log.session_id`, so it is the join
    /// key used for ledger lookups/debug hints and fabricated-ref repair
    /// correction. Model-authored evidence ids are optional; if a model cites one,
    /// submit/runtime can use this session scope to reject fabricated refs.
    pub(super) chat_session_id: Option<String>,
    /// C5 · HITL approval channel (the **same coordinator** the `ask_human` tool
    /// uses). When wired, the two-level phase-boundary approval gate requests a
    /// Confirm/Skip decision through this coordinator — surfaced as an
    /// `AskHumanRequest` card the user can click **without** stopping the running
    /// task — instead of the legacy `user_input_rx` text channel (which has no
    /// production feeder, so the gate would otherwise wedge forever). `None`
    /// (e.g. unit tests) → text fallback over `user_input_rx`.
    pub(super) approval_coordinator: Option<crate::CoordinatorHandle>,
    /// 方案 2 · headless 单/区间阶段实跑的 DAG allowlist. `Some` 时,
    /// `run_executor_driven` 投影 DAG 用 `profile.allowed ∩ allowlist`, 把可执行
    /// 阶段裁到一段切片 (`scoping..=to` 或 `{only}`)——切片终点无后继 → 跑完即
    /// Complete 停下. `None` (默认 / GUI 正常 run) = 用整张 profile DAG, 行为不变.
    pub(super) stage_allowlist: Option<std::collections::HashSet<crate::harness::StageKind>>,
    /// Cross-session adoption plan selected by the user before this new
    /// operation starts. When present, `run()` starts at `entry_stage` and
    /// restricts the DAG to `remaining_stages`, so previously satisfied stages
    /// are skipped deterministically instead of being prompt-only context.
    pub(super) continuity_adoption: Option<crate::harness::ContinuityAdoptionPlan>,
    /// One-shot fast path for a bare continuation prompt. The next resumed stage
    /// request consumes this flag; if that stage is a DB-root-bound specialist
    /// stage, its first primary-agent turn is locked to `stage_run`.
    pub(super) force_stage_run_on_resume_once: bool,
    /// Complete runtime-memory source selected by a trusted resume preflight.
    /// `DualWriteV2Preferred` cannot infer this from its frozen contract alone.
    pub(super) resume_runtime_memory_source: Option<crate::db_traits::RuntimeMemoryRecordSource>,
    /// The trusted caller already changed the exact durable task from waiting to
    /// running with an ownership/source CAS. Consumed by the next `resume()` so
    /// the generic path cannot issue a second, unfenced status update.
    pub(super) resume_task_preclaimed: bool,
    /// Wave loop (设计 2026-07-02-attack-stage §3.5): current chain-wave counter
    /// for this run. Advanced by `consume_gate_outcome` when a `verification` PASS
    /// opens a new attack_candidate wave; fed to `decide_chain_wave` as the fuel/
    /// depth cap baseline. Starts at 0 (a fresh run has run no waves).
    pub(super) chain_wave: u32,
    /// Wave loop · dedupe keys ([`crate::harness::chain_wave::candidate_dedup_key`])
    /// of every attack hypothesis already tested this run. `decide_chain_wave`
    /// treats a candidate whose key is already here as "not new", so an
    /// a↔b oscillation cannot reopen waves forever (only genuine a→b→c progress
    /// does). Rebuilt fresh per run (resume re-derives via DB dedupe on upsert).
    pub(super) chain_wave_seen: std::collections::HashSet<String>,
    /// Trusted headless-CLI scope frozen during compound operation creation.
    /// GUI/model runs leave this unset and continue through Scoping lifecycle
    /// evidence. A CLI V2 operation sets it exactly once before `run_stage`.
    pub(super) cli_runtime_scope: Option<CliRuntimeScope>,
    /// Adapter-neutral fresh-launch target authority. `Some(true)` means an
    /// exact target came from this invocation and must still match DB truth;
    /// `Some(false)` means the launch confirmed only an organization, so even
    /// historical targets on that org cannot unlock active recon. `None` keeps
    /// the interactive Scoping lifecycle. The headless exact-resume adapter
    /// restores a valid marker when present and otherwise fails closed to
    /// `Some(false)`; it never treats a missing hint as target authority.
    pub(super) current_invocation_target_authority: Option<bool>,
}

impl TaskOrchestrator {
    pub fn new(
        repo: Arc<dyn DbRepoProvider>,
        runtime_repo: Arc<dyn RuntimeMemoryRepository>,
        session_id: Uuid,
        event_tx: mpsc::UnboundedSender<AiEvent>,
    ) -> Self {
        let (user_input_tx, user_input_rx) = mpsc::unbounded_channel();
        Self {
            repo,
            runtime_repo,
            session_id,
            event_tx,
            user_input_rx: Some(user_input_rx),
            user_input_tx,
            harness_evidence: std::collections::HashMap::new(),
            profile_override: None,
            harness_org_id: None,
            harness_subsidiary_policy: None,
            stage_outcome_acc: None,
            chat_session_id: None,
            approval_coordinator: None,
            stage_allowlist: None,
            continuity_adoption: None,
            force_stage_run_on_resume_once: false,
            resume_runtime_memory_source: None,
            resume_task_preclaimed: false,
            chain_wave: 0,
            chain_wave_seen: std::collections::HashSet::new(),
            cli_runtime_scope: None,
            current_invocation_target_authority: None,
        }
    }

    /// Returns a sender that can be used to provide user input to a waiting subtask.
    pub fn user_input_sender(&self) -> mpsc::UnboundedSender<String> {
        self.user_input_tx.clone()
    }

    /// Set the chat-session string used to scope evidence-ledger lookups (see
    /// [`Self::chat_session_id`]). Call this right after `new` in the chat
    /// command so gate repair corrections can name this operation's real
    /// evidence ids. `None`/unset keeps the prior behaviour (no id hint).
    pub fn set_chat_session_id(&mut self, chat_session_id: impl Into<String>) {
        self.chat_session_id = Some(chat_session_id.into());
    }

    /// Override the operation profile for this run (set from the chat-panel mode
    /// picker). `None` keeps the `GOLISH_HARNESS_PROFILE` env default.
    pub fn set_profile_override(&mut self, profile: Option<String>) {
        self.profile_override = profile;
    }

    /// Bind the current operation's organization (coverage asset-axis
    /// isolation, design 2026-06-09). The coverage gate's asset denominator and
    /// the agent-facing in-scope asset list are then narrowed to this org's
    /// in-scope targets. `None` keeps the legacy whole-DB asset axis.
    pub fn set_harness_org_id(&mut self, org_id: Option<Uuid>) {
        self.harness_org_id = org_id;
    }

    /// Phase 2: enable subsidiary scoping for this run (org-tree gate). `include`
    /// false leaves the policy `None` (legacy scoping, zero behaviour change).
    pub fn set_subsidiary_scope(&mut self, include: bool, threshold_pct: u8) {
        self.harness_subsidiary_policy = include.then_some(SubsidiaryScopePolicy { threshold_pct });
    }

    /// Attach the trusted, already-resolved CLI scope to the next fresh
    /// operation. It is consumed by `run_from_stage`; resume never rebuilds or
    /// re-reads a mutable organization tree.
    pub fn set_cli_runtime_scope(&mut self, scope: Option<CliRuntimeScope>) {
        self.cli_runtime_scope = scope;
    }

    /// Freeze whether this fresh typed launch carried an exact target from the
    /// current invocation. Organization-only launch adapters set `Some(false)`;
    /// interactive paths leave it `None`; headless exact resume may restore a
    /// validated marker but otherwise deliberately supplies `Some(false)`.
    pub fn set_current_invocation_target_authority(&mut self, authority: Option<bool>) {
        self.current_invocation_target_authority = authority;
    }

    /// Wire the HITL coordinator so the two-level phase-approval gate can request
    /// a clickable Confirm/Skip decision (the shared `ask_human` channel) instead
    /// of the legacy `user_input_rx` text channel. Call right after [`Self::new`]
    /// in the chat command. `None` keeps the text fallback (unit tests).
    pub fn set_approval_coordinator(&mut self, coordinator: Option<crate::CoordinatorHandle>) {
        self.approval_coordinator = coordinator;
    }

    /// 方案 2 · restrict the executable DAG to a slice of stages. `Some(set)`
    /// makes `run_executor_driven` project with `profile.allowed ∩ set` so a
    /// headless run can execute just `{only}` or `scoping..=to` and stop. `None`
    /// (default) = full profile DAG (GUI / normal `run` behaviour, unchanged).
    pub fn set_stage_allowlist(
        &mut self,
        allowlist: Option<std::collections::HashSet<crate::harness::StageKind>>,
    ) {
        self.stage_allowlist = allowlist;
    }

    /// Apply a user-confirmed cross-session adoption plan before a fresh
    /// operation starts. The plan is ignored by `resume()`, which always resumes
    /// the original operation checkpoint.
    pub fn set_continuity_adoption(
        &mut self,
        plan: Option<crate::harness::ContinuityAdoptionPlan>,
    ) {
        self.continuity_adoption = plan;
    }

    /// Prefer a deterministic `stage_run` dispatch for the next resumed stage
    /// when it is safe to do so. Callers should only set this for bare
    /// continuation prompts, not for "继续，但是..." steering text.
    pub fn set_force_stage_run_on_resume_once(&mut self, enabled: bool) {
        self.force_stage_run_on_resume_once = enabled;
    }

    /// Pin the next resume to the whole runtime-memory record selected by a
    /// trusted preflight. In particular, V2-preferred callers must pass either
    /// `V2` or `LegacyFallback`; the graph checkpointer will not reselect fields.
    pub fn set_resume_runtime_memory_source(
        &mut self,
        source: crate::db_traits::RuntimeMemoryRecordSource,
    ) {
        self.resume_runtime_memory_source = Some(source);
    }

    /// Mark the next resume as already durably claimed by its caller. This flag
    /// is one-shot and must only be set after a waiting->running CAS succeeds.
    pub fn set_resume_task_preclaimed(&mut self, preclaimed: bool) {
        self.resume_task_preclaimed = preclaimed;
    }

    /// Run a full Task mode execution.
    ///
    /// This is the top-level entry point, equivalent to PentAGI's
    /// `NewTaskWorker + tw.Run()`. The operation cursor starts at the DAG entry
    /// (`scoping`).
    pub async fn run(
        &mut self,
        task_input: &str,
        project_scope: ProjectScopeRegistration,
        executor: &dyn AgentExecutor,
    ) -> Result<String> {
        let entry_stage = if let Some(plan) = self.continuity_adoption.as_ref() {
            if self.stage_allowlist.is_none() {
                self.stage_allowlist = Some(plan.remaining_stages.iter().copied().collect());
            }
            plan.entry_stage
        } else {
            crate::harness::StageKind::Scoping
        };
        self.run_from_stage(task_input, project_scope, executor, entry_stage)
            .await
    }

    /// 方案 2 · headless single/range stage run: start a fresh operation whose
    /// cursor begins at `entry_stage` (instead of `scoping`). Pair with
    /// [`Self::set_stage_allowlist`] so the projected DAG's entry is `entry_stage`
    /// and its terminal is the slice's `to` (run finishes that slice, then stops).
    pub async fn run_stage(
        &mut self,
        entry_stage: crate::harness::StageKind,
        task_input: &str,
        project_scope: ProjectScopeRegistration,
        executor: &dyn AgentExecutor,
    ) -> Result<String> {
        self.run_from_stage(task_input, project_scope, executor, entry_stage)
            .await
    }

    /// Shared body for [`Self::run`] / [`Self::run_stage`]: create the task +
    /// `operation_state` (cursor at `entry_stage`) and drive the executor. The
    /// only difference between the two callers is the initial cursor stage.
    async fn run_from_stage(
        &mut self,
        task_input: &str,
        project_scope: ProjectScopeRegistration,
        executor: &dyn AgentExecutor,
        entry_stage: crate::harness::StageKind,
    ) -> Result<String> {
        // Phase C harness: 一个 Task = 一个 operation. 建 operation_state 游标,
        // 起点为 `entry_stage` (正常 run = DAG entry scoping; 方案 2 run_stage = 切片
        // 入口); gate 过后沿 profile 投影的 DAG 推进. profile 经 GOLISH_HARNESS_PROFILE
        // 选择, 未知 id 回退 assessment (typo 不 wedge 启动).
        //
        // Prefer the per-run picker override; fall back to the env default. Own the
        // string up front so no borrow of `self` lingers across the later
        // `&mut self` subtask loop.
        let configured: String = self
            .profile_override
            .clone()
            .unwrap_or_else(|| crate::harness::active_profile_id().to_string());
        let profile_id: String = match crate::harness::load_embedded_profile(&configured) {
            Ok(Some(_)) => configured,
            _ => {
                tracing::warn!(
                    target: "harness::hook",
                    configured = %configured,
                    "unknown harness profile, falling back to assessment"
                );
                "assessment".to_string()
            }
        };
        let operation_id = Uuid::new_v4();
        let initial_stage_execution_id = Uuid::new_v4();
        let expected_project_scope_id = project_scope.project_scope_id;
        let created = self
            .runtime_repo
            .create_runtime_operation(CreateRuntimeOperation {
                operation_id,
                initial_stage_execution_id,
                session_id: self.session_id,
                title: None,
                input: task_input.to_string(),
                profile: profile_id.clone(),
                entry_stage: entry_stage.as_str().to_string(),
                project_scope,
                cli_scope: self.cli_runtime_scope.take(),
            })
            .await
            .map_err(anyhow::Error::new)
            .context("Failed to create task and operation atomically")?;
        anyhow::ensure!(
            created.task.id == operation_id
                && created.operation.operation_id == operation_id
                && created.initial_stage_execution_id == initial_stage_execution_id
                && created.operation.project_scope_id == Some(expected_project_scope_id),
            "atomic runtime operation returned mismatched task/operation/project identity"
        );
        let initial_stage_execution_id = created.initial_stage_execution_id;
        let initial_operation_profile = created.operation.profile.clone();
        let initial_operation_stage = created.operation.current_stage.clone();
        let initial_runtime_memory_contract = created.operation.runtime_memory_contract;
        let task = created.task;

        tasks::update_status(&*self.repo, task.id, TaskStatus::Running).await?;

        self.emit(AiEvent::TaskProgress {
            task_id: task.id.to_string(),
            status: "running".to_string(),
            message: "Generating subtasks...".to_string(),
        });
        if let Some(plan) = self.continuity_adoption.as_ref() {
            self.emit(AiEvent::TaskProgress {
                task_id: task.id.to_string(),
                status: "running".to_string(),
                message: format!(
                    "Reusing DB-backed progress for {} stage(s); continuing at {}.",
                    plan.adopted_stages.len(),
                    plan.entry_stage.as_str()
                ),
            });
        }

        // A1 · lazy per-stage planning: the harness Executor plans each stage on
        // entry (see run_stage_subtasks), so do NOT pre-generate a flat whole-run
        // plan — start empty and let every projected stage plan itself.
        let mut generated: Vec<PlannedSubtask> = Vec::new();

        // S3 · the run() path never ran the deterministic stage-tag backfill (only
        // resume() did), so tags depended entirely on the Generator LLM. Backfill
        // here as a safety net, then drop any subtask tagged with a stage outside
        // the active profile's allowed set (forbidden / unreachable) so it is never
        // created-but-orphaned (e.g. a `vuln_triage` subtask under the assessment
        // profile).
        {
            crate::task_orchestrator::harness_backfill::backfill_harness_stage(&mut generated);
            let profile_id: String = self
                .profile_override
                .clone()
                .unwrap_or_else(|| crate::harness::active_profile_id().to_string());
            if let Ok(Some(p)) = crate::harness::load_embedded_profile(&profile_id) {
                let allowed = p.allowed_stage_set();
                generated.retain(|s| match s.harness_stage.as_ref() {
                    Some(h) if !allowed.contains(&h.stage_kind) => {
                        tracing::warn!(
                            target: "harness::hook",
                            subtask_title = %s.title,
                            stage = %h.stage_kind.as_str(),
                            profile = %profile_id,
                            "dag-strict: dropping planner subtask tagged with a stage outside the profile's allowed set"
                        );
                        false
                    }
                    _ => true,
                });
            }
        }

        let mut queue: Vec<PlannedSubtask> = Vec::new();
        for planned in &generated {
            let agent_type = parse_agent_type(&planned.agent);
            let subtask = match subtasks::create(
                &*self.repo,
                subtasks::NewSubtask {
                    task_id: task.id,
                    session_id: self.session_id,
                    title: Some(planned.title.clone()),
                    description: Some(planned.description.clone()),
                    agent: agent_type,
                },
            )
            .await
            {
                Ok(s) => s,
                Err(e) => {
                    // P1 · don't leak the task as `running` if subtask creation
                    // fails partway; finalize it before bubbling the error up.
                    self.fail_task_if_active(task.id, &e).await;
                    return Err(e.context("Failed to create subtask"));
                }
            };

            self.emit(AiEvent::SubtaskCreated {
                task_id: task.id.to_string(),
                subtask_id: subtask.id.to_string(),
                title: planned.title.clone(),
                agent: planned.agent.clone(),
            });

            queue.push(planned.clone());
        }

        self.emit(AiEvent::TaskProgress {
            task_id: task.id.to_string(),
            status: "running".to_string(),
            message: format!("Generated {} subtasks, starting execution...", queue.len()),
        });

        // Initial plan: all steps Pending. `current_index` must be 0 (not a
        // sentinel like usize::MAX) — emit_plan_update marks every `i <
        // current_index` Completed.
        self.emit_plan_update(&queue, 0, StepStatus::Pending);

        // P1 · compound operation creation already opened the exact initial
        // stage execution. The resume checkpoint must reference that returned
        // identity; opening a second random `stage_run` would immediately make
        // the operation ambiguous. Checkpoint failures are fatal because a run
        // that cannot durably resume must not begin stage work.
        let initial_checkpoint =
            if should_persist_initial_harness_checkpoint(initial_runtime_memory_contract) {
                async {
                    let rs = crate::task_orchestrator::harness_resume::HarnessResumeState {
                        profile: initial_operation_profile,
                        current_stage: initial_operation_stage,
                        current_stage_run_id: Some(initial_stage_execution_id),
                        queue_titles: queue.iter().map(|p| p.title.clone()).collect(),
                        completed_count: 0,
                        continuity_adoption: self.continuity_adoption.clone(),
                        schema_v: 1,
                    };
                    let checkpoint = serde_json::to_value(&rs)
                        .context("Failed to serialize initial harness checkpoint")?;
                    crate::db_shim::operation_state::write_state_blob(
                        &*self.repo,
                        task.id,
                        checkpoint,
                    )
                    .await
                    .context("Failed to persist initial harness checkpoint")
                }
                .await
            } else {
                Ok(())
            };
        if let Err(error) = initial_checkpoint {
            self.fail_task_if_active(task.id, &error).await;
            return Err(error);
        }

        // P2 方案 C · the metalcraft Executor drives the operation stage graph.
        // (`execute_subtask_loop` remains the resume path + the
        // run_executor_driven DAG-load-failure fallback.)
        let outcome = self
            .run_executor_driven(
                task.id,
                &queue,
                executor,
                false,
                None,
                Some(initial_stage_execution_id),
            )
            .await;

        // P1 · never leave the task zombied in `running`: any error from the
        // execution path finalizes the row as `failed` here (a process killed
        // mid-run is swept by the DB startup reaper instead). The happy path
        // already wrote `finished`, so `fail_task_if_active` skips it.
        if let Err(ref e) = outcome {
            self.fail_task_if_active(task.id, e).await;
        }
        outcome
    }

    /// Resume an interrupted/abandoned harness operation instead of starting a
    /// new one (Task 断线恢复 · L3).
    ///
    /// The entry point ([`crate::task_orchestrator`] caller in `chat.rs`) selects
    /// this when [`crate::db_shim`]'s `latest_resumable_by_session` finds a
    /// non-terminal task with a persisted `graph_flow` checkpoint for the chat
    /// session. Unlike [`Self::run`] this does NOT create a task / session /
    /// operation_state — they already exist; it re-drives the same operation from
    /// the checkpoint's `next_node` via `Executor::resume`.
    ///
    /// `user_message` is the request-local input that triggered the resume
    /// ("继续" / steering text / anything). A non-blank value is threaded into
    /// this execution's [`ExecutionContext`](crate::task_orchestrator::ExecutionContext)
    /// so the resumed primary/stage worker sees the current operator constraints.
    /// The durable task row keeps the original operation input unchanged; a blank
    /// resume message falls back to that original input. The message is not parsed
    /// as a routing keyword here — the caller already selected resume from DB state.
    pub async fn resume(
        &mut self,
        task_id: Uuid,
        user_message: &str,
        executor: &dyn AgentExecutor,
    ) -> Result<String> {
        // Re-activate the row: an abandoned op is `running` (zombie) or `waiting`
        // (paused); a resumed run is `running` again until it completes/pauses.
        if consume_resume_task_status_claim(&mut self.resume_task_preclaimed) {
            if let Err(e) = tasks::update_status(&*self.repo, task_id, TaskStatus::Running).await {
                tracing::warn!(
                    target: "harness::hook",
                    task_id = %task_id,
                    error = %e,
                    "resume: failed to set task running (continuing)"
                );
            }
        } else {
            tracing::debug!(
                target: "harness::hook",
                task_id = %task_id,
                "resume: exact task status was preclaimed by trusted caller"
            );
        }

        tracing::info!(
            target: "harness::hook",
            task_id = %task_id,
            trigger_len = user_message.len(),
            "resume: re-driving harness operation from checkpoint"
        );
        self.emit(AiEvent::TaskProgress {
            task_id: task_id.to_string(),
            status: "running".to_string(),
            message: "Resuming previous operation from checkpoint...".to_string(),
        });

        // Graph-flow uses lazy per-stage planning, so the queue is empty on the
        // run() path too; the per-stage roadmap is re-emitted from the projected
        // DAG inside run_executor_driven, which rehydrates the UI on resume.
        let queue: Vec<PlannedSubtask> = Vec::new();
        let outcome = self
            .run_executor_driven(task_id, &queue, executor, true, Some(user_message), None)
            .await;
        if let Err(ref e) = outcome {
            self.fail_task_if_active(task_id, e).await;
        }
        outcome
    }

    /// Finalize a task as `failed` unless it already reached a terminal status.
    ///
    /// P1 · before this, the only failure finalizer was the generator branch, so
    /// every other error path in [`Self::run`] left the row stuck in `running`
    /// (a zombie). Reads the current status first so a `finished` (or
    /// already-`failed`) result written by the normal path is never clobbered.
    /// Best-effort: a DB error while finalizing is swallowed (the task is
    /// already failing, and the startup reaper is the backstop).
    pub(super) async fn fail_task_if_active(&self, task_id: Uuid, err: &anyhow::Error) {
        match tasks::get(&*self.repo, task_id).await {
            Ok(Some(t))
                if matches!(
                    t.status,
                    TaskStatus::Created | TaskStatus::Running | TaskStatus::Waiting
                ) =>
            {
                let _ = tasks::set_result(
                    &*self.repo,
                    task_id,
                    &format!("Task failed: {err:#}"),
                    TaskStatus::Failed,
                )
                .await;
            }
            _ => {}
        }
    }

    pub(super) fn emit(&self, event: AiEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Emit a PlanUpdated event to synchronize the frontend Task Plan UI.
    ///
    /// The event `version` comes from a process-global monotonic counter
    /// ([`next_plan_version`]) so every emission is strictly newer than the
    /// last. Callers previously passed a hand-rolled version (`idx + 2`) that
    /// collided between a step's InProgress and Completed updates, so the
    /// frontend `setPlan` reducer — which drops same-version events — silently
    /// swallowed every "completed" transition.
    pub(super) fn emit_plan_update(
        &self,
        queue: &[PlannedSubtask],
        current_index: usize,
        current_status: StepStatus,
    ) {
        let steps: Vec<PlanStep> = queue
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let status = if i < current_index {
                    StepStatus::Completed
                } else if i == current_index {
                    current_status
                } else {
                    StepStatus::Pending
                };
                PlanStep {
                    id: Some(format!("task-step-{}", i + 1)),
                    step: s.title.clone(),
                    status,
                    failure_kind: None,
                }
            })
            .collect();
        let summary = PlanSummary::from_steps(&steps);
        self.emit(AiEvent::PlanUpdated {
            version: next_plan_version(),
            summary,
            steps,
            explanation: None,
            stage_id: None,
        });
    }
}

fn consume_resume_task_status_claim(preclaimed: &mut bool) -> bool {
    !std::mem::take(preclaimed)
}

/// Process-global monotonic version source for `PlanUpdated` events.
///
/// Returns a strictly increasing value on each call (starting at 1, never 0 —
/// the frontend treats `version == 0` as "no plan"). Used by
/// [`TaskOrchestrator::emit_plan_update`] so a step's InProgress and Completed
/// emissions get distinct, ordered versions and the frontend reducer stops
/// dropping the "completed" transition.
pub(super) fn next_plan_version() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static PLAN_VERSION: AtomicU32 = AtomicU32::new(1);
    PLAN_VERSION.fetch_add(1, Ordering::Relaxed)
}

/// V2-only operations reconstruct resume state from relational execution rows;
/// writing the flat `HarnessResumeState` would recreate a forbidden second
/// authority. Compatibility contracts retain their existing checkpoint path.
fn should_persist_initial_harness_checkpoint(
    contract: crate::runtime_memory::RuntimeMemoryContract,
) -> bool {
    contract != crate::runtime_memory::RuntimeMemoryContract::V2Only
}

#[cfg(test)]
mod plan_version_tests {
    use super::{
        consume_resume_task_status_claim, next_plan_version,
        should_persist_initial_harness_checkpoint,
    };

    /// Regression (P0 · plan version collision): `next_plan_version` must hand
    /// back strictly increasing, non-zero values so a step's InProgress and
    /// Completed `PlanUpdated` events never share a version. The frontend
    /// `setPlan` reducer drops same-version events, so the old hand-rolled
    /// `idx + 2` collided between those two emissions and silently swallowed
    /// every "completed" transition (plan card stuck at 0/N). Robust against
    /// parallel test interleaving: concurrent callers only widen the gap, they
    /// can never make a later call return a value <= an earlier one.
    #[test]
    fn next_plan_version_is_strictly_increasing_and_nonzero() {
        let v1 = next_plan_version();
        let v2 = next_plan_version();
        let v3 = next_plan_version();
        assert!(
            v1 >= 1,
            "version must never be 0 (frontend treats 0 as 'no plan')"
        );
        assert!(v2 > v1, "v2 ({v2}) must be strictly greater than v1 ({v1})");
        assert!(v3 > v2, "v3 ({v3}) must be strictly greater than v2 ({v2})");
    }

    #[test]
    fn initial_harness_checkpoint_is_never_written_for_v2_only() {
        use crate::runtime_memory::RuntimeMemoryContract::{
            DualWriteLegacyRead, DualWriteV2Preferred, LegacyV1, V2Only,
        };

        assert!(should_persist_initial_harness_checkpoint(LegacyV1));
        assert!(should_persist_initial_harness_checkpoint(
            DualWriteLegacyRead
        ));
        assert!(should_persist_initial_harness_checkpoint(
            DualWriteV2Preferred
        ));
        assert!(!should_persist_initial_harness_checkpoint(V2Only));
    }

    #[test]
    fn trusted_resume_preclaim_skips_exactly_one_generic_status_update() {
        let mut preclaimed = true;
        assert!(!consume_resume_task_status_claim(&mut preclaimed));
        assert!(!preclaimed, "the trusted preclaim must be consumed");
        assert!(consume_resume_task_status_claim(&mut preclaimed));
    }
}
