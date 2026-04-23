//! Task Orchestrator — PentAGI-style automated task execution.
//!
//! Implements the full Task mode state machine:
//! 1. **Generator**: Decomposes user input into ordered subtasks
//! 2. **Primary Agent Loop**: Executes each subtask with delegation
//! 3. **Refiner**: After each subtask, adjusts remaining plan
//! 4. **Reporter**: Generates a final task report
//!
//! This module operates at a level above the `AgentBridge`, calling
//! into it for each agent invocation while managing the overall
//! task lifecycle and DB persistence.

pub mod bridge_executor;
pub(crate) mod prompts;

use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

use golish_core::events::AiEvent;
use golish_core::plan::{PlanStep, PlanSummary, StepStatus};
use golish_db::models::{SubtaskStatus, TaskStatus};
use golish_db::repo::{subtasks, tasks};

/// Maximum number of subtasks per task (safety limit matching PentAGI's TasksNumberLimit+3).
const MAX_SUBTASKS: usize = 13;
/// Maximum reflector attempts before giving up (matches PentAGI's maxReflectorCallsPerChain).
const MAX_REFLECTOR_RETRIES: usize = 3;

/// A planned subtask from the Generator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedSubtask {
    pub title: String,
    pub description: String,
    /// Which specialist should handle this (e.g. "pentester", "coder").
    /// The primary agent uses this as guidance, not a hard constraint.
    pub agent: Option<String>,
}

/// The generator's response — a list of subtasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorOutput {
    pub subtasks: Vec<PlannedSubtask>,
}

/// The refiner's response — structured patch operations on the remaining plan.
///
/// Mirrors PentAGI's `SubtaskPatch` pattern: instead of regenerating the entire plan,
/// the refiner applies surgical operations (add/remove/modify/reorder) which is
/// more token-efficient and preserves context better.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefinerOutput {
    /// Subtasks to add to the queue.
    #[serde(default)]
    pub add: Vec<PlannedSubtask>,
    /// Indices (0-based, relative to remaining queue) to remove.
    #[serde(default)]
    pub remove: Vec<usize>,
    /// Modifications to apply to existing subtasks (by 0-based index in remaining queue).
    #[serde(default)]
    pub modify: Vec<SubtaskModification>,
    /// New ordering of remaining subtasks (0-based indices). If provided, subtasks
    /// are reordered accordingly before add/remove operations.
    #[serde(default)]
    pub reorder: Option<Vec<usize>>,
    /// Whether the task is considered complete (skip remaining subtasks).
    #[serde(default)]
    pub complete: bool,
}

/// A modification to an existing subtask in the remaining queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskModification {
    /// 0-based index in the remaining queue.
    pub index: usize,
    /// New title (if changed).
    #[serde(default)]
    pub title: Option<String>,
    /// New description (if changed).
    #[serde(default)]
    pub description: Option<String>,
    /// New agent assignment (if changed).
    #[serde(default)]
    pub agent: Option<String>,
}

/// Token usage statistics for a single agent call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Agent phase that consumed these tokens (e.g. "generator", "primary_agent", "refiner", "reporter").
    pub phase: String,
}

/// Accumulated cost tracking across all agent calls in a task.
#[derive(Debug, Clone, Default)]
pub struct TaskCostTracker {
    pub entries: Vec<AgentTokenUsage>,
}

impl TaskCostTracker {
    pub fn record(&mut self, entry: AgentTokenUsage) {
        self.entries.push(entry);
    }

    pub fn total_input_tokens(&self) -> u64 {
        self.entries.iter().map(|e| e.input_tokens).sum()
    }

    pub fn total_output_tokens(&self) -> u64 {
        self.entries.iter().map(|e| e.output_tokens).sum()
    }

    pub fn total_duration_ms(&self) -> u64 {
        self.entries.iter().map(|e| e.duration_ms).sum()
    }
}

/// Context accumulated during task execution, passed between agents.
#[derive(Debug, Clone, Default)]
pub struct ExecutionContext {
    /// Accumulated results from completed subtasks.
    pub completed_results: Vec<SubtaskResult>,
    /// The original user input.
    pub task_input: String,
    /// Additional context gathered by the Enricher after subtask completions.
    pub enrichment_context: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SubtaskResult {
    pub title: String,
    pub result: String,
    /// Token usage for executing this subtask (if tracked).
    pub token_usage: Option<AgentTokenUsage>,
}

impl ExecutionContext {
    pub fn summary(&self) -> String {
        if self.completed_results.is_empty() {
            return "No subtasks completed yet.".to_string();
        }
        let mut s = String::new();
        for (i, r) in self.completed_results.iter().enumerate() {
            s.push_str(&format!(
                "### Subtask {} — {}\n{}\n\n",
                i + 1,
                r.title,
                r.result
            ));
        }
        if !self.enrichment_context.is_empty() {
            s.push_str("### Additional Context (Enricher)\n");
            for ctx in &self.enrichment_context {
                s.push_str(&format!("- {}\n", ctx));
            }
            s.push('\n');
        }
        s
    }
}

/// Result from an agent execution that includes token tracking.
#[derive(Debug, Clone)]
pub struct AgentResult {
    pub content: String,
    pub token_usage: Option<AgentTokenUsage>,
}

impl AgentResult {
    pub fn new(content: String) -> Self {
        Self {
            content,
            token_usage: None,
        }
    }

    pub fn with_usage(content: String, usage: AgentTokenUsage) -> Self {
        Self {
            content,
            token_usage: Some(usage),
        }
    }
}

/// Callback trait for the orchestrator to invoke LLM agents.
///
/// This decouples the orchestrator from `AgentBridge` directly,
/// making it testable and allowing different execution strategies.
///
/// All methods return `AgentResult` to enable per-call token tracking
/// (PentAGI-style per-chain cost accounting).
#[async_trait::async_trait]
pub trait AgentExecutor: Send + Sync {
    /// Run the generator to decompose the task into subtasks.
    async fn generate_subtasks(
        &self,
        task_input: &str,
    ) -> Result<GeneratorOutput>;

    /// Execute a single subtask as the primary agent.
    /// Returns the result text and optional token usage.
    /// `agent_type` is the specialist type assigned by the Generator (e.g., "pentester", "coder").
    async fn execute_subtask(
        &self,
        subtask_title: &str,
        subtask_description: &str,
        execution_context: &ExecutionContext,
        agent_type: Option<&str>,
    ) -> Result<AgentResult>;

    /// Run the refiner to adjust the remaining plan.
    async fn refine_plan(
        &self,
        execution_context: &ExecutionContext,
        remaining_subtasks: &[PlannedSubtask],
    ) -> Result<RefinerOutput>;

    /// Run the reporter to generate the final summary.
    async fn generate_report(
        &self,
        execution_context: &ExecutionContext,
    ) -> Result<AgentResult>;

    /// Run the reflector to redirect an agent that returned plain text.
    ///
    /// Returns a corrective message that should be injected as a user message
    /// before retrying the subtask. The reflector acts as a "proxy user" that
    /// guides the agent back to tool usage (PentAGI's Reflector pattern).
    async fn reflect(
        &self,
        subtask_title: &str,
        agent_response: &str,
    ) -> Result<String>;

    /// Generate a pre-execution plan for a subtask (PentAGI's Task Planner pattern).
    ///
    /// Before a specialist agent starts working, the Adviser creates a structured
    /// checklist (3-7 steps) that is wrapped around the original subtask description.
    /// This prevents the agent from acting blindly and improves efficiency.
    ///
    /// Default implementation returns `None` (no pre-planning).
    async fn plan_subtask(
        &self,
        subtask_title: &str,
        subtask_description: &str,
        agent_type: &str,
        execution_context: &ExecutionContext,
    ) -> Result<Option<String>> {
        let _ = (subtask_title, subtask_description, agent_type, execution_context);
        Ok(None)
    }

    /// Enrich context after a subtask completes.
    ///
    /// Searches memories, knowledge bases, or external sources for context
    /// relevant to the subtask result. Returns enrichment text to inject
    /// into subsequent subtask prompts (PentAGI's Enricher pattern).
    ///
    /// Default implementation returns `None` (no enrichment).
    async fn enrich(
        &self,
        subtask_title: &str,
        subtask_result: &str,
        execution_context: &ExecutionContext,
    ) -> Result<Option<String>> {
        let _ = (subtask_title, subtask_result, execution_context);
        Ok(None)
    }

    /// Serialize the current message chain for persistence.
    ///
    /// Returns the conversation messages as JSON for storage in the
    /// `message_chains` table. Default returns `None` (no persistence).
    fn current_message_chain(&self) -> Option<serde_json::Value> {
        None
    }
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
    pool: Arc<sqlx::PgPool>,
    session_id: Uuid,
    event_tx: mpsc::UnboundedSender<AiEvent>,
    /// Channel for receiving user input when a subtask is paused.
    /// The orchestrator sends a `SubtaskWaitingForInput` event and blocks
    /// on this receiver until the user provides input.
    user_input_rx: Option<mpsc::UnboundedReceiver<String>>,
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

    /// Core subtask execution loop shared by `run` and `resume`.
    async fn execute_subtask_loop(
        &mut self,
        task_id: Uuid,
        queue: &mut Vec<PlannedSubtask>,
        start_index: usize,
        executor: &dyn AgentExecutor,
    ) -> Result<String> {
        let mut exec_ctx = ExecutionContext {
            completed_results: Vec::new(),
            task_input: String::new(),
            enrichment_context: Vec::new(),
        };

        if start_index > 0 {
            let db_subtasks = subtasks::list_by_task(&self.pool, task_id).await?;
            for st in db_subtasks.iter().take(start_index) {
                exec_ctx.completed_results.push(SubtaskResult {
                    title: st.title.clone().unwrap_or_default(),
                    result: st.result.clone().unwrap_or_default(),
                    token_usage: None,
                });
            }
        }

        let mut cost_tracker = TaskCostTracker::default();
        let mut subtask_index = start_index;

        while subtask_index < queue.len() && subtask_index < MAX_SUBTASKS {
            let planned = &queue[subtask_index];

            let db_subtask = subtasks::next_pending(&self.pool, task_id).await?;
            if let Some(ref st) = db_subtask {
                subtasks::update_status(&self.pool, st.id, SubtaskStatus::Running).await?;
            }

            self.emit(AiEvent::TaskProgress {
                task_id: task_id.to_string(),
                status: "running".to_string(),
                message: format!(
                    "Executing subtask {}/{}: {}",
                    subtask_index + 1,
                    queue.len(),
                    planned.title
                ),
            });

            // Mark current subtask as in_progress in the plan UI
            self.emit_plan_update(
                queue,
                subtask_index,
                StepStatus::InProgress,
                subtask_index as u32 + 2,
            );

            // Create message chain record for this subtask
            let chain_id = if let Some(ref st) = db_subtask {
                let agent_type = parse_agent_type(&planned.agent)
                    .unwrap_or(golish_db::models::AgentType::Primary);
                match golish_db::repo::message_chains::create(
                    &self.pool,
                    self.session_id,
                    Some(task_id),
                    Some(st.id),
                    agent_type,
                    None,
                    None,
                )
                .await
                {
                    Ok(chain) => Some(chain.id),
                    Err(e) => {
                        tracing::warn!("Failed to create message chain: {}", e);
                        None
                    }
                }
            } else {
                None
            };

            // Execute subtask with reflector retry
            let (result_text, subtask_usage) = self
                .execute_single_subtask(planned, &exec_ctx, executor, &db_subtask, task_id)
                .await;

            // Persist message chain content
            if let Some(cid) = chain_id {
                if let Some(chain_json) = executor.current_message_chain() {
                    let _ = golish_db::repo::message_chains::update_chain(
                        &self.pool, cid, &chain_json,
                    )
                    .await;
                }
                if let Some(ref usage) = subtask_usage {
                    let _ = golish_db::repo::message_chains::update_usage(
                        &self.pool,
                        cid,
                        usage.input_tokens as i32,
                        usage.output_tokens as i32,
                        0,
                        0.0,
                        0.0,
                        usage.duration_ms as i32,
                    )
                    .await;
                }
            }

            if let Some(ref usage) = subtask_usage {
                cost_tracker.record(usage.clone());
            }

            if let Some(ref st) = db_subtask {
                subtasks::set_result(&self.pool, st.id, &result_text, SubtaskStatus::Finished)
                    .await?;
            }

            exec_ctx.completed_results.push(SubtaskResult {
                title: planned.title.clone(),
                result: result_text.clone(),
                token_usage: subtask_usage,
            });

            self.emit(AiEvent::SubtaskCompleted {
                task_id: task_id.to_string(),
                subtask_id: db_subtask
                    .as_ref()
                    .map(|s| s.id.to_string())
                    .unwrap_or_default(),
                title: planned.title.clone(),
                result: truncate(&result_text, 500),
            });

            // Mark subtask as completed in the plan UI
            self.emit_plan_update(
                queue,
                subtask_index,
                StepStatus::Completed,
                subtask_index as u32 + 2,
            );

            // Enricher: search for additional context after subtask completion
            match executor
                .enrich(&planned.title, &result_text, &exec_ctx)
                .await
            {
                Ok(Some(enrichment)) => {
                    tracing::info!(
                        "[Enricher] Added {} chars of context after '{}'",
                        enrichment.len(),
                        planned.title
                    );
                    if let Some(ref st) = db_subtask {
                        let _ =
                            subtasks::set_context(&self.pool, st.id, &enrichment).await;
                        self.emit(AiEvent::EnricherResult {
                            task_id: task_id.to_string(),
                            subtask_id: st.id.to_string(),
                            context_added: truncate(&enrichment, 300),
                        });
                    }
                    exec_ctx.enrichment_context.push(enrichment);
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("[Enricher] Failed after '{}': {}", planned.title, e);
                }
            }

            subtask_index += 1;

            // Refine remaining plan (unless this was the last subtask)
            if subtask_index < queue.len() {
                self.refine_remaining(task_id, queue, subtask_index, &exec_ctx, executor)
                    .await;
            }
        }

        // Generate final report
        self.emit(AiEvent::TaskProgress {
            task_id: task_id.to_string(),
            status: "reporting".to_string(),
            message: "Generating final report...".to_string(),
        });

        let report = match executor.generate_report(&exec_ctx).await {
            Ok(agent_result) => {
                if let Some(ref usage) = agent_result.token_usage {
                    cost_tracker.record(usage.clone());
                }
                agent_result.content
            }
            Err(e) => {
                tracing::warn!("Reporter failed, using summary: {}", e);
                exec_ctx.summary()
            }
        };

        tasks::set_result(&self.pool, task_id, &report, TaskStatus::Finished).await?;

        tracing::info!(
            "[TaskMode] Task completed. Total tokens: {} in / {} out, {} agent calls, {:.1}s",
            cost_tracker.total_input_tokens(),
            cost_tracker.total_output_tokens(),
            cost_tracker.entries.len(),
            cost_tracker.total_duration_ms() as f64 / 1000.0,
        );

        // Emit final plan update — all steps completed
        let final_steps: Vec<PlanStep> = queue
            .iter()
            .enumerate()
            .map(|(i, s)| PlanStep {
                id: Some(format!("task-step-{}", i + 1)),
                step: s.title.clone(),
                status: StepStatus::Completed,
            })
            .collect();
        let final_summary = PlanSummary::from_steps(&final_steps);
        self.emit(AiEvent::PlanUpdated {
            version: (queue.len() as u32 + 10),
            summary: final_summary,
            steps: final_steps,
            explanation: Some("Task completed".to_string()),
        });

        self.emit(AiEvent::TaskProgress {
            task_id: task_id.to_string(),
            status: "finished".to_string(),
            message: format!(
                "Task completed. Tokens: {} in / {} out across {} agent calls.",
                cost_tracker.total_input_tokens(),
                cost_tracker.total_output_tokens(),
                cost_tracker.entries.len(),
            ),
        });

        Ok(report)
    }

    /// Execute a single subtask with reflector retry and optional user input pause.
    async fn execute_single_subtask(
        &mut self,
        planned: &PlannedSubtask,
        exec_ctx: &ExecutionContext,
        executor: &dyn AgentExecutor,
        db_subtask: &Option<golish_db::models::Subtask>,
        task_id: Uuid,
    ) -> (String, Option<AgentTokenUsage>) {
        // Pre-fetch: search knowledge base for relevant guides/memories BEFORE execution.
        // PentAGI's agents always search their guide store before acting.
        let pre_knowledge = match executor
            .enrich(&planned.title, &planned.description, exec_ctx)
            .await
        {
            Ok(Some(knowledge)) => {
                tracing::info!(
                    "[PreFetch] Found {} chars of prior knowledge for '{}'",
                    knowledge.len(),
                    planned.title
                );
                Some(knowledge)
            }
            Ok(None) => None,
            Err(e) => {
                tracing::debug!("[PreFetch] Knowledge search failed for '{}': {}", planned.title, e);
                None
            }
        };

        // Task Planner: generate an execution plan before the agent starts.
        let agent_type = planned.agent.as_deref().unwrap_or("primary");
        let description_with_knowledge = if let Some(ref knowledge) = pre_knowledge {
            format!(
                "{}\n\n## PRIOR KNOWLEDGE\n\n\
                 The following relevant information was found in the knowledge base:\n\n{}",
                planned.description, knowledge
            )
        } else {
            planned.description.clone()
        };

        let effective_description = match executor
            .plan_subtask(&planned.title, &description_with_knowledge, agent_type, exec_ctx)
            .await
        {
            Ok(Some(plan)) => {
                tracing::info!(
                    "[TaskPlanner] Generated {} char plan for '{}'",
                    plan.len(),
                    planned.title
                );
                format!(
                    "<task_assignment>\n\
                     <original_request>\n{}\n</original_request>\n\n\
                     <execution_plan>\n{}\n</execution_plan>\n\n\
                     <hint>\n\
                     The original_request is the primary objective.\n\
                     The execution_plan was prepared by analyzing the broader context.\n\
                     Use this plan as guidance, but adapt to actual circumstances.\n\
                     </hint>\n\
                     </task_assignment>",
                    description_with_knowledge, plan
                )
            }
            Ok(None) => description_with_knowledge,
            Err(e) => {
                tracing::warn!("[TaskPlanner] Plan generation failed: {}", e);
                description_with_knowledge
            }
        };

        let mut last_result: Option<AgentResult> = None;

        for reflector_attempt in 0..=MAX_REFLECTOR_RETRIES {
            let exec_result = if reflector_attempt == 0 {
                executor
                    .execute_subtask(&planned.title, &effective_description, exec_ctx, Some(agent_type))
                    .await
            } else {
                let prev_response = last_result
                    .as_ref()
                    .map(|r| r.content.as_str())
                    .unwrap_or("");
                match executor.reflect(&planned.title, prev_response).await {
                    Ok(correction) => {
                        tracing::info!(
                            "[TaskMode/Reflector] Retry {}/{} for '{}': {}",
                            reflector_attempt,
                            MAX_REFLECTOR_RETRIES,
                            planned.title,
                            truncate(&correction, 200)
                        );
                        let augmented_desc = format!(
                            "{}\n\n## IMPORTANT CORRECTION\n\n{}",
                            planned.description, correction
                        );
                        executor
                            .execute_subtask(&planned.title, &augmented_desc, exec_ctx, Some(agent_type))
                            .await
                    }
                    Err(e) => {
                        tracing::warn!("Reflector failed: {}", e);
                        break;
                    }
                }
            };

            match exec_result {
                Ok(agent_result) => {
                    // Detect text-only responses that lack evidence of real tool usage.
                    // If the agent just described what it would do instead of doing it,
                    // treat it like a soft failure and invoke the reflector.
                    if reflector_attempt < MAX_REFLECTOR_RETRIES
                        && looks_like_text_only_response(&agent_result.content)
                    {
                        tracing::info!(
                            "[TaskMode/Reflector] Subtask '{}' returned text-only response ({} chars), \
                             triggering reflector (attempt {})",
                            planned.title,
                            agent_result.content.len(),
                            reflector_attempt + 1,
                        );
                        last_result = Some(agent_result);
                        continue;
                    }

                    // Check if the agent is requesting user input
                    if agent_result.content.contains("[NEEDS_USER_INPUT]") {
                        let prompt = agent_result
                            .content
                            .replace("[NEEDS_USER_INPUT]", "")
                            .trim()
                            .to_string();

                        if let Some(ref st) = db_subtask {
                            let _ = subtasks::update_status(
                                &self.pool,
                                st.id,
                                SubtaskStatus::Waiting,
                            )
                            .await;
                        }

                        self.emit(AiEvent::SubtaskWaitingForInput {
                            task_id: task_id.to_string(),
                            subtask_id: db_subtask
                                .as_ref()
                                .map(|s| s.id.to_string())
                                .unwrap_or_default(),
                            title: planned.title.clone(),
                            prompt: prompt.clone(),
                        });

                        // Wait for user input
                        if let Some(ref mut rx) = self.user_input_rx {
                            tracing::info!(
                                "[TaskMode] Subtask '{}' waiting for user input",
                                planned.title
                            );
                            if let Some(user_input) = rx.recv().await {
                                self.emit(AiEvent::SubtaskUserInput {
                                    task_id: task_id.to_string(),
                                    subtask_id: db_subtask
                                        .as_ref()
                                        .map(|s| s.id.to_string())
                                        .unwrap_or_default(),
                                    input: truncate(&user_input, 200),
                                });

                                if let Some(ref st) = db_subtask {
                                    let _ = subtasks::update_status(
                                        &self.pool,
                                        st.id,
                                        SubtaskStatus::Running,
                                    )
                                    .await;
                                }

                                let augmented_desc = format!(
                                    "{}\n\n## USER INPUT\n\n{}",
                                    planned.description, user_input
                                );
                                match executor
                                    .execute_subtask(
                                        &planned.title,
                                        &augmented_desc,
                                        exec_ctx,
                                        Some(agent_type),
                                    )
                                    .await
                                {
                                    Ok(final_result) => {
                                        return (
                                            final_result.content,
                                            final_result.token_usage,
                                        );
                                    }
                                    Err(e) => {
                                        return (format!("Error after user input: {}", e), None);
                                    }
                                }
                            }
                        }
                    }

                    return (agent_result.content, agent_result.token_usage);
                }
                Err(e) => {
                    if reflector_attempt == MAX_REFLECTOR_RETRIES {
                        let err_msg = format!(
                            "Subtask failed after {} reflector retries: {}",
                            MAX_REFLECTOR_RETRIES, e
                        );
                        if let Some(ref st) = db_subtask {
                            let _ = subtasks::set_result(
                                &self.pool,
                                st.id,
                                &err_msg,
                                SubtaskStatus::Failed,
                            )
                            .await;
                        }
                        tracing::warn!("Subtask '{}' failed: {}", planned.title, e);
                        return (err_msg, None);
                    }
                    last_result = Some(AgentResult::new(format!("Error: {}", e)));
                }
            }
        }

        let fallback = last_result
            .map(|r| r.content)
            .unwrap_or_else(|| "Subtask completed without tool usage.".to_string());
        (fallback, None)
    }

    /// Apply refinement to the remaining subtask queue.
    async fn refine_remaining(
        &self,
        task_id: Uuid,
        queue: &mut Vec<PlannedSubtask>,
        subtask_index: usize,
        exec_ctx: &ExecutionContext,
        executor: &dyn AgentExecutor,
    ) {
        let remaining = &queue[subtask_index..];
        match executor.refine_plan(exec_ctx, remaining).await {
            Ok(refinement) => {
                if refinement.complete {
                    tracing::info!("Refiner says task is complete, skipping remaining");
                    let _ = subtasks::delete_pending(&self.pool, task_id).await;
                    queue.truncate(subtask_index);
                    return;
                }

                if let Some(ref new_order) = refinement.reorder {
                    let remaining_len = queue.len() - subtask_index;
                    if new_order.len() == remaining_len
                        && new_order.iter().all(|&i| i < remaining_len)
                    {
                        let remaining: Vec<PlannedSubtask> = queue[subtask_index..].to_vec();
                        for (dst, &src) in new_order.iter().enumerate() {
                            queue[subtask_index + dst] = remaining[src].clone();
                        }
                        tracing::info!(
                            "Refiner reordered {} remaining subtasks",
                            remaining_len
                        );
                    }
                }

                for m in &refinement.modify {
                    let absolute_idx = subtask_index + m.index;
                    if absolute_idx < queue.len() {
                        let subtask = &mut queue[absolute_idx];
                        if let Some(ref title) = m.title {
                            subtask.title = title.clone();
                        }
                        if let Some(ref desc) = m.description {
                            subtask.description = desc.clone();
                        }
                        if m.agent.is_some() {
                            subtask.agent = m.agent.clone();
                        }
                    }
                }

                let mut to_remove = refinement.remove.clone();
                to_remove.sort_unstable();
                to_remove.dedup();
                for &idx in to_remove.iter().rev() {
                    let absolute_idx = subtask_index + idx;
                    if absolute_idx < queue.len() {
                        queue.remove(absolute_idx);
                    }
                }

                for added in &refinement.add {
                    let agent_type = parse_agent_type(&added.agent);
                    match subtasks::create(
                        &self.pool,
                        subtasks::NewSubtask {
                            task_id,
                            session_id: self.session_id,
                            title: Some(added.title.clone()),
                            description: Some(added.description.clone()),
                            agent: agent_type,
                        },
                    )
                    .await
                    {
                        Ok(st) => {
                            self.emit(AiEvent::SubtaskCreated {
                                task_id: task_id.to_string(),
                                subtask_id: st.id.to_string(),
                                title: added.title.clone(),
                                agent: added.agent.clone(),
                            });
                            queue.push(added.clone());
                        }
                        Err(e) => {
                            tracing::warn!("Failed to create refined subtask: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Refiner failed, continuing without refinement: {}", e);
            }
        }
    }

    fn emit(&self, event: AiEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Emit a PlanUpdated event to synchronize the frontend Task Plan UI.
    fn emit_plan_update(
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

fn parse_agent_type(agent: &Option<String>) -> Option<golish_db::models::AgentType> {
    agent.as_ref().and_then(|a| match a.as_str() {
        "pentester" => Some(golish_db::models::AgentType::Pentester),
        "coder" => Some(golish_db::models::AgentType::Coder),
        "searcher" | "researcher" => Some(golish_db::models::AgentType::Searcher),
        "memorist" => Some(golish_db::models::AgentType::Memorist),
        "reporter" => Some(golish_db::models::AgentType::Reporter),
        "adviser" => Some(golish_db::models::AgentType::Adviser),
        _ => Some(golish_db::models::AgentType::Primary),
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// Heuristic to detect responses that are purely descriptive text without
/// evidence of actual tool execution. PentAGI uses barrier functions to
/// enforce structured output; this is a lighter alternative.
fn looks_like_text_only_response(response: &str) -> bool {
    let trimmed = response.trim();
    if trimmed.len() < 50 {
        return false;
    }

    // Markers that indicate real tool work was performed
    let tool_evidence = [
        "```",           // code blocks from tool output
        "scan result",
        "output:",
        "found ",
        "discovered ",
        "vulnerable",
        "port ",
        "service ",
        "HTTP/",
        "200 OK",
        "404",
        "nmap",
        "subfinder",
        "httpx",
        "nuclei",
        ".golish/",
        "successfully",
        "executed",
        "Error:",
    ];

    let lower = trimmed.to_lowercase();
    let has_evidence = tool_evidence
        .iter()
        .any(|marker| lower.contains(&marker.to_lowercase()));

    // Phrases that indicate the agent is describing rather than doing
    let description_phrases = [
        "i would",
        "i will",
        "i can",
        "let me",
        "we should",
        "we could",
        "the next step",
        "here's my plan",
        "i recommend",
    ];
    let has_description = description_phrases
        .iter()
        .any(|phrase| lower.contains(phrase));

    !has_evidence && has_description
}
