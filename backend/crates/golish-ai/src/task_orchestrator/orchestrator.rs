//! [`TaskOrchestrator`] entry points (`new`, `user_input_sender`, `run`,
//! `resume`) and shared event-emission helpers.
//!
//! The actual subtask execution / refinement phases live in
//! [`super::subtask_phases`] as a separate `impl TaskOrchestrator` block.

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use uuid::Uuid;

use golish_core::events::AiEvent;
use golish_core::plan::{PlanStep, PlanSummary, StepStatus};
use golish_db::models::{SubtaskStatus, TaskStatus};
use golish_db::repo::{subtasks, tasks};

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
    pub(super) pool: Arc<sqlx::PgPool>,
    pub(super) session_id: Uuid,
    pub(super) event_tx: mpsc::UnboundedSender<AiEvent>,
    /// Channel for receiving user input when a subtask is paused.
    /// The orchestrator sends a `SubtaskWaitingForInput` event and blocks
    /// on this receiver until the user provides input.
    pub(super) user_input_rx: Option<mpsc::UnboundedReceiver<String>>,
    /// Sender side kept so callers can feed user input to the orchestrator.
    user_input_tx: mpsc::UnboundedSender<String>,
}

impl TaskOrchestrator {
    pub fn new(
        pool: Arc<sqlx::PgPool>,
        session_id: Uuid,
        event_tx: mpsc::UnboundedSender<AiEvent>,
    ) -> Self {
        let (user_input_tx, user_input_rx) = mpsc::unbounded_channel();
        Self {
            pool,
            session_id,
            event_tx,
            user_input_rx: Some(user_input_rx),
            user_input_tx,
        }
    }

    /// Returns a sender that can be used to provide user input to a waiting subtask.
    pub fn user_input_sender(&self) -> mpsc::UnboundedSender<String> {
        self.user_input_tx.clone()
    }

    /// Run a full Task mode execution.
    ///
    /// This is the top-level entry point, equivalent to PentAGI's
    /// `NewTaskWorker + tw.Run()`.
    pub async fn run(
        &mut self,
        task_input: &str,
        executor: &dyn AgentExecutor,
    ) -> Result<String> {
        let task = tasks::create(
            &self.pool,
            golish_db::models::NewTask {
                session_id: self.session_id,
                title: None,
                input: task_input.to_string(),
            },
        )
        .await
        .context("Failed to create task")?;

        tasks::update_status(&self.pool, task.id, TaskStatus::Running).await?;

        self.emit(AiEvent::TaskProgress {
            task_id: task.id.to_string(),
            status: "running".to_string(),
            message: "Generating subtasks...".to_string(),
        });

        let generator_output = match executor.generate_subtasks(task_input).await {
            Ok(output) => output,
            Err(e) => {
                tasks::set_result(
                    &self.pool,
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
            let subtask = subtasks::create(
                &self.pool,
                subtasks::NewSubtask {
                    task_id: task.id,
                    session_id: self.session_id,
                    title: Some(planned.title.clone()),
                    description: Some(planned.description.clone()),
                    agent: agent_type,
                },
            )
            .await?;

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

        // Emit initial plan with all steps pending
        self.emit_plan_update(&queue, usize::MAX, StepStatus::Pending, 1);

        self.execute_subtask_loop(task.id, &mut queue, 0, executor)
            .await
    }

    /// Resume a previously interrupted task from the last completed subtask.
    ///
    /// Reloads all completed subtask results from the DB and continues
    /// execution from the next pending subtask.
    pub async fn resume(
        &mut self,
        task_id: Uuid,
        executor: &dyn AgentExecutor,
    ) -> Result<String> {
        let task = tasks::get(&self.pool, task_id)
            .await?
            .context("Task not found")?;

        if task.status == TaskStatus::Finished {
            return Ok(task.result.unwrap_or_default());
        }

        tasks::update_status(&self.pool, task.id, TaskStatus::Running).await?;

        let db_subtasks = subtasks::list_by_task(&self.pool, task.id).await?;

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
            })
            .collect();

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
    pub(super) fn emit_plan_update(
        &self,
        queue: &[PlannedSubtask],
        current_index: usize,
        current_status: StepStatus,
        version: u32,
    ) {
        let steps: Vec<PlanStep> = queue
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let status = if i < current_index {
                    StepStatus::Completed
                } else if i == current_index {
                    current_status.clone()
                } else {
                    StepStatus::Pending
                };
                PlanStep {
                    id: Some(format!("task-step-{}", i + 1)),
                    step: s.title.clone(),
                    status,
                }
            })
            .collect();
        let summary = PlanSummary::from_steps(&steps);
        self.emit(AiEvent::PlanUpdated {
            version,
            summary,
            steps,
            explanation: None,
        });
    }
}
