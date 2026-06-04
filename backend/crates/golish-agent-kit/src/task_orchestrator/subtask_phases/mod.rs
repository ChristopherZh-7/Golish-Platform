//! Subtask execution phases for [`TaskOrchestrator`]:
//! [`execute_subtask_loop`] (the orchestration core) and
//! `refine_remaining` (the post-subtask refiner pass).
//!
//! The single-subtask execution logic (enrichment, planning, reflector retry)
//! lives in [`execute`].

mod execute;

use anyhow::Result;
use uuid::Uuid;

use crate::db_shim::{message_chains, subtasks, tasks};
use crate::db_traits::{SubtaskStatus, TaskStatus};
use golish_core::events::AiEvent;
use golish_core::plan::{PlanStep, PlanSummary, StepStatus};

use super::helpers::{parse_agent_type, truncate};
use super::types::{
    AgentExecutor, CurrentSubtask, ExecutionContext, PlannedSubtask, PlannedSubtaskInfo,
    SubtaskResult, TaskCostTracker, MAX_SUBTASKS,
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
        let task_input = tasks::get(&*self.repo, task_id)
            .await
            .ok()
            .flatten()
            .map(|t| t.input)
            .unwrap_or_default();

        let mut exec_ctx = ExecutionContext {
            completed_results: Vec::new(),
            task_input,
            current_subtask: None,
            planned_subtasks: Vec::new(),
            harness_stage: None,
            harness_authz: None,
            harness_profile_id: None,
        };

        // C3/C1 · resolve the operation's profile once.
        // `operation_id == task_id`; read the real profile from `operation_state`
        // so (C3) per-tool dispatch can reject intents above the profile's
        // `max_authorization`, and (C1) the gate hook constructs StageHarness with
        // the real profile id instead of a placeholder. `None` → gate degrades to
        // forbidden-only / falls back to "assessment".
        let (op_max_authz, op_profile_id): (
            Option<crate::harness::AuthorizationLevel>,
            Option<String>,
        ) = match crate::db_shim::operation_state::get(&*self.repo, task_id).await {
            Ok(Some(state)) => match crate::harness::load_embedded_profile(&state.profile) {
                Ok(Some(p)) => (Some(p.max_authorization), Some(state.profile)),
                _ => (None, None),
            },
            _ => (None, None),
        };
        exec_ctx.harness_profile_id = op_profile_id;

        if start_index > 0 {
            let db_subtasks = subtasks::list_by_task(&*self.repo, task_id).await?;
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

            let db_subtask = subtasks::next_pending(&*self.repo, task_id).await?;
            if let Some(ref st) = db_subtask {
                subtasks::update_status(&*self.repo, st.id, SubtaskStatus::Running).await?;
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

            self.emit_plan_update(queue, subtask_index, StepStatus::InProgress);

            let chain_id = if let Some(ref st) = db_subtask {
                let agent_type = parse_agent_type(&planned.agent)
                    .unwrap_or(crate::db_traits::AgentType::Primary);
                match message_chains::create(
                    &*self.repo,
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

            exec_ctx.current_subtask = Some(CurrentSubtask {
                title: planned.title.clone(),
                description: planned.description.clone(),
                agent: planned.agent.clone(),
            });
            // C3 · thread this subtask's harness stage so the bridge/agentic loop
            // can enforce the stage forbidden-tool barrier.
            exec_ctx.harness_stage = planned.harness_stage.as_ref().map(|h| h.stage_kind);
            // C3 · bundle the authorization context: classify this subtask's
            // intent (deterministic keyword classifier over its description) and
            // pair it with the operation ceiling. Only set when a stage is active
            // and the ceiling resolved → per-tool dispatch runs the full
            // pre-action authorizer on real executor tools.
            exec_ctx.harness_authz = match (exec_ctx.harness_stage, op_max_authz) {
                (Some(stage_kind), Some(max_authorization)) => {
                    let intent = crate::harness::IntentClassifier::with_default_keywords()
                        .classify(&planned.description, stage_kind);
                    Some(crate::harness::HarnessAuthz {
                        max_authorization,
                        intent,
                    })
                }
                _ => None,
            };
            exec_ctx.planned_subtasks = queue[subtask_index + 1..]
                .iter()
                .map(|p| PlannedSubtaskInfo {
                    title: p.title.clone(),
                    description: p.description.clone(),
                })
                .collect();

            let (result_text, subtask_usage) = self
                .execute_single_subtask(planned, &exec_ctx, executor, &db_subtask, task_id)
                .await;

            if let Some(cid) = chain_id {
                if let Some(chain_json) = executor.current_message_chain() {
                    let _ = message_chains::update_chain(&*self.repo, cid, &chain_json).await;
                }
                if let Some(ref usage) = subtask_usage {
                    let _ = message_chains::update_usage(
                        &*self.repo,
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
                subtasks::set_result(&*self.repo, st.id, &result_text, SubtaskStatus::Finished)
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
                stage_kind: exec_ctx.harness_stage.map(|s| s.as_str().to_string()),
            });

            self.emit_plan_update(queue, subtask_index, StepStatus::Completed);

            subtask_index += 1;

            if subtask_index < queue.len() {
                self.refine_remaining(task_id, queue, subtask_index, &exec_ctx, executor)
                    .await;
            }
        }

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

        tasks::set_result(&*self.repo, task_id, &report, TaskStatus::Finished).await?;

        tracing::info!(
            "[TaskMode] Task completed. Total tokens: {} in / {} out, {} agent calls, {:.1}s",
            cost_tracker.total_input_tokens(),
            cost_tracker.total_output_tokens(),
            cost_tracker.entries.len(),
            cost_tracker.total_duration_ms() as f64 / 1000.0,
        );

        let final_steps: Vec<PlanStep> = queue
            .iter()
            .enumerate()
            .map(|(i, s)| PlanStep {
                id: Some(format!("task-step-{}", i + 1)),
                step: s.title.clone(),
                status: StepStatus::Completed,
                failure_kind: None,
            })
            .collect();
        let final_summary = PlanSummary::from_steps(&final_steps);
        // P2 · use the same process-global monotonic version source as
        // `emit_plan_update` so this final "all completed" plan can never collide
        // with (or sort before) the per-step updates emitted above. The previous
        // hard-coded `queue.len() + 10` could land below a recent step version
        // (which the frontend reducer would then mis-order/keep stale).
        self.emit(AiEvent::PlanUpdated {
            version: super::orchestrator::next_plan_version(),
            summary: final_summary,
            steps: final_steps,
            explanation: Some("Task completed".to_string()),
            stage_id: None,
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
                    let _ = subtasks::delete_pending(&*self.repo, task_id).await;
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
                        tracing::info!("Refiner reordered {} remaining subtasks", remaining_len);
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
                        &*self.repo,
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
