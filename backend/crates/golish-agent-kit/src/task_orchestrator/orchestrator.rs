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
use crate::db_traits::{DbRepoProvider, NewTask, SubtaskStatus, TaskStatus};
use golish_core::events::AiEvent;
use golish_core::plan::{PlanStep, PlanSummary, StepStatus};

use super::helpers::parse_agent_type;
use super::types::{AgentExecutor, PlannedSubtask};

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
    /// P2 方案 C · when true, the metalcraft Executor owns the stage loop, so
    /// `execute_single_subtask` must NOT drive the per-subtask cursor transition
    /// (the graph drives transitions); it accumulates the flow outcome into
    /// [`Self::stage_outcome_acc`] instead. Default false = legacy per-subtask
    /// transition (`run_executor_driven` sets/clears it around each stage).
    pub(super) graph_driven: bool,
    /// P2 方案 C · accumulated [`StageFlowOutcome`] for the stage currently being
    /// run under the Executor (merged across that stage's subtasks: gate ANDed,
    /// progress ORed). Read + cleared by `run_stage_subtasks`.
    pub(super) stage_outcome_acc: Option<crate::harness::operation_flow::StageFlowOutcome>,
}

impl TaskOrchestrator {
    pub fn new(
        repo: Arc<dyn DbRepoProvider>,
        session_id: Uuid,
        event_tx: mpsc::UnboundedSender<AiEvent>,
    ) -> Self {
        let (user_input_tx, user_input_rx) = mpsc::unbounded_channel();
        Self {
            repo,
            session_id,
            event_tx,
            user_input_rx: Some(user_input_rx),
            user_input_tx,
            harness_evidence: std::collections::HashMap::new(),
            profile_override: None,
            graph_driven: false,
            stage_outcome_acc: None,
        }
    }

    /// Returns a sender that can be used to provide user input to a waiting subtask.
    pub fn user_input_sender(&self) -> mpsc::UnboundedSender<String> {
        self.user_input_tx.clone()
    }

    /// Override the operation profile for this run (set from the chat-panel mode
    /// picker). `None` keeps the `GOLISH_HARNESS_PROFILE` env default.
    pub fn set_profile_override(&mut self, profile: Option<String>) {
        self.profile_override = profile;
    }

    /// Run a full Task mode execution.
    ///
    /// This is the top-level entry point, equivalent to PentAGI's
    /// `NewTaskWorker + tw.Run()`.
    pub async fn run(&mut self, task_input: &str, executor: &dyn AgentExecutor) -> Result<String> {
        let task = tasks::create(
            &*self.repo,
            NewTask {
                session_id: self.session_id,
                title: None,
                input: task_input.to_string(),
            },
        )
        .await
        .context("Failed to create task")?;

        tasks::update_status(&*self.repo, task.id, TaskStatus::Running).await?;

        // Phase C harness: 一个 Task = 一个 operation. stage_mode 开启时建
        // operation_state 游标, 起点为 DAG entry (scoping); gate 过后由
        // drive_stage_transition 沿 profile 投影的 DAG 推进. profile 经
        // GOLISH_HARNESS_PROFILE 选择, 未知 id 回退 assessment (typo 不 wedge 启动).
        // flag OFF 时完全不触碰 DB, 旧路径零影响.
        if crate::harness::stage_mode_enabled() {
            // Prefer the per-run picker override; fall back to the env default.
            // Own the string up front so no borrow of `self` lingers across the
            // later `&mut self` subtask loop.
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
            if let Err(e) = crate::db_shim::operation_state::insert(
                &*self.repo,
                task.id,
                &profile_id,
                crate::harness::StageKind::Scoping.as_str(),
            )
            .await
            {
                tracing::warn!(
                    target: "harness::hook",
                    error = %e,
                    "operation_state insert failed (continuing)"
                );
            }
        }

        self.emit(AiEvent::TaskProgress {
            task_id: task.id.to_string(),
            status: "running".to_string(),
            message: "Generating subtasks...".to_string(),
        });

        let generator_output = match executor.generate_subtasks(task_input).await {
            Ok(output) => output,
            Err(e) => {
                tasks::set_result(
                    &*self.repo,
                    task.id,
                    &format!("Generator failed: {}", e),
                    TaskStatus::Failed,
                )
                .await?;
                return Err(e.context("Generator failed"));
            }
        };

        let mut queue: Vec<PlannedSubtask> = Vec::new();
        for planned in &generator_output.subtasks {
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

        // P1 · write the initial harness checkpoint (open a stage_run for the
        // entry stage + persist a resume state_blob) so a kill before the first
        // stage transition can still resume from the right place.
        if crate::harness::stage_mode_enabled() {
            if let Ok(Some(os)) = crate::db_shim::operation_state::get(&*self.repo, task.id).await {
                let run_id = uuid::Uuid::new_v4();
                let _ = crate::db_shim::stage_runs::insert(
                    &*self.repo,
                    run_id,
                    task.id,
                    &os.current_stage,
                )
                .await;
                let rs = crate::task_orchestrator::harness_resume::HarnessResumeState {
                    profile: os.profile.clone(),
                    current_stage: os.current_stage.clone(),
                    current_stage_run_id: Some(run_id),
                    queue_titles: queue.iter().map(|p| p.title.clone()).collect(),
                    completed_count: 0,
                    schema_v: 1,
                };
                let _ = crate::db_shim::operation_state::write_state_blob(
                    &*self.repo,
                    task.id,
                    serde_json::to_value(&rs).unwrap_or_default(),
                )
                .await;
            }
        }

        // P2 方案 C · when the graph-flow flag is on (and stage_mode), let the
        // metalcraft Executor drive the operation stage graph; otherwise the
        // legacy flat subtask loop runs unchanged (default / rollback).
        let outcome = if crate::harness::stage_mode_enabled()
            && crate::harness::operation_flow::graph_flow_enabled()
        {
            self.run_executor_driven(task.id, &queue, executor).await
        } else {
            self.execute_subtask_loop(task.id, &mut queue, 0, executor)
                .await
        };

        // P1 · never leave the task zombied in `running`: any error from the
        // execution path finalizes the row as `failed` here (a process killed
        // mid-run is swept by the DB startup reaper instead). The happy path
        // already wrote `finished`, so `fail_task_if_active` skips it.
        if let Err(ref e) = outcome {
            self.fail_task_if_active(task.id, e).await;
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

    /// Resume a previously interrupted task from the last completed subtask.
    ///
    /// Reloads all completed subtask results from the DB and continues
    /// execution from the next pending subtask.
    pub async fn resume(&mut self, task_id: Uuid, executor: &dyn AgentExecutor) -> Result<String> {
        let task = tasks::get(&*self.repo, task_id)
            .await?
            .context("Task not found")?;

        if task.status == TaskStatus::Finished {
            return Ok(task.result.unwrap_or_default());
        }

        tasks::update_status(&*self.repo, task.id, TaskStatus::Running).await?;

        let db_subtasks = subtasks::list_by_task(&*self.repo, task.id).await?;

        let completed_count = db_subtasks
            .iter()
            .filter(|s| s.status == SubtaskStatus::Finished)
            .count();

        let mut queue: Vec<PlannedSubtask> = db_subtasks
            .iter()
            .map(|s| PlannedSubtask {
                title: s.title.clone().unwrap_or_default(),
                description: s.description.clone().unwrap_or_default(),
                agent: s.agent.map(|a| format!("{:?}", a).to_lowercase()),
                // Phase 1 MVP: 旧路径恢复时不带 harness 信息. harness_stage=None
                // 等同于走旧 task_orchestrator 行为 (feature flag OFF).
                harness_stage: None,
                nl_slice: None,
                acceptance_criteria: Vec::new(),
            })
            .collect();

        // P1 · restore harness context on resume: the queue was rebuilt from DB
        // rows that don't carry harness_stage, so re-infer each subtask's stage
        // when this is a harness operation. Without this, a resumed run silently
        // falls back to the non-harness path (no gate / no cursor advance).
        if crate::harness::stage_mode_enabled()
            && crate::db_shim::operation_state::get(&*self.repo, task.id)
                .await
                .ok()
                .flatten()
                .is_some()
        {
            crate::task_orchestrator::harness_backfill::backfill_harness_stage(&mut queue);
        }

        self.emit(AiEvent::TaskResumed {
            task_id: task.id.to_string(),
            subtask_index: completed_count,
            total_subtasks: queue.len(),
        });

        self.execute_subtask_loop(task.id, &mut queue, completed_count, executor)
            .await
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
        });
    }
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

#[cfg(test)]
mod plan_version_tests {
    use super::next_plan_version;

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
        assert!(v1 >= 1, "version must never be 0 (frontend treats 0 as 'no plan')");
        assert!(v2 > v1, "v2 ({v2}) must be strictly greater than v1 ({v1})");
        assert!(v3 > v2, "v3 ({v3}) must be strictly greater than v2 ({v2})");
    }
}
