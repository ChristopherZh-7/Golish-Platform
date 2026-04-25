//! Subtask execution phases for [`TaskOrchestrator`]:
//! [`execute_subtask_loop`] (the orchestration core), `execute_single_subtask`
//! (one subtask with reflector retry + user-input pause), and
//! `refine_remaining` (the post-subtask refiner pass).
//!
//! Lives in a separate file because together these three methods are ~600
//! lines, and the surrounding code in [`super::orchestrator`] is much smaller
//! and easier to read on its own.

use anyhow::Result;
use uuid::Uuid;

use golish_core::events::AiEvent;
use golish_core::plan::{PlanStep, PlanSummary, StepStatus};
use golish_db::models::{SubtaskStatus, TaskStatus};
use golish_db::repo::{subtasks, tasks};

use super::helpers::{looks_like_text_only_response, parse_agent_type, truncate};
use super::types::{
    AgentExecutor, AgentResult, AgentTokenUsage, ExecutionContext, PlannedSubtask, SubtaskResult,
    TaskCostTracker, MAX_REFLECTOR_RETRIES, MAX_SUBTASKS,
};

use super::TaskOrchestrator;

impl TaskOrchestrator {
    /// Core subtask execution loop shared by `run` and `resume`.
    pub(super) async fn execute_subtask_loop(
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
}
