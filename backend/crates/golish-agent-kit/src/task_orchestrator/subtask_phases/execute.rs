//! Single-subtask execution with enrichment, planning, and reflector retry.

use uuid::Uuid;

use golish_core::events::AiEvent;
use crate::db_traits::SubtaskStatus;
use crate::db_shim::subtasks;

use super::super::helpers::{looks_like_text_only_response, truncate};
use super::super::types::{
    AgentExecutor, AgentResult, AgentTokenUsage, ExecutionContext, PlannedSubtask,
    MAX_REFLECTOR_RETRIES,
};
use super::super::TaskOrchestrator;

impl TaskOrchestrator {
    /// Execute a single subtask with enrichment, planning, reflector retry,
    /// and optional user input pause.
    ///
    /// Follows PentAGI's full flow:
    /// 1. **Enrich**: Gather supplementary context from completed work
    /// 2. **Plan**: Generate an execution checklist via the Adviser
    /// 3. **Execute**: Run the subtask with reflector retry loop
    pub(super) async fn execute_single_subtask(
        &mut self,
        planned: &PlannedSubtask,
        exec_ctx: &ExecutionContext,
        executor: &dyn AgentExecutor,
        db_subtask: &Option<crate::db_traits::SubtaskView>,
        task_id: Uuid,
    ) -> (String, Option<AgentTokenUsage>) {
        let agent_type = planned.agent.as_deref().unwrap_or("primary");

        // Phase 1: Enrich — gather supplementary context
        let enrichment = match executor
            .enrich_subtask(&planned.title, &planned.description, exec_ctx, agent_type)
            .await
        {
            Ok(Some(ctx)) => {
                tracing::info!(
                    "[TaskMode] Enrichment added for '{}': {} chars",
                    planned.title,
                    ctx.len()
                );
                Some(ctx)
            }
            Ok(None) => None,
            Err(e) => {
                tracing::warn!("[TaskMode] Enrichment failed (continuing): {}", e);
                None
            }
        };

        // Phase 2: Plan — generate execution checklist
        let execution_plan = match executor
            .plan_subtask(&planned.title, &planned.description, exec_ctx, agent_type)
            .await
        {
            Ok(Some(plan)) => {
                tracing::info!(
                    "[TaskMode] Execution plan generated for '{}': {} chars",
                    planned.title,
                    plan.len()
                );
                Some(plan)
            }
            Ok(None) => None,
            Err(e) => {
                tracing::warn!("[TaskMode] Planning failed (continuing): {}", e);
                None
            }
        };

        let augmented_description = {
            let mut desc = String::new();

            if let Some(ref enrichment) = enrichment {
                desc.push_str("## SUPPLEMENTARY CONTEXT\n\n");
                desc.push_str(enrichment);
                desc.push_str("\n\n");
            }

            if let Some(ref plan) = execution_plan {
                desc.push_str(&super::super::prompts::wrap_task_with_plan(
                    &planned.description,
                    plan,
                ));
            } else {
                desc.push_str(&planned.description);
            }

            desc
        };

        // Phase 3: Execute with reflector retry loop
        let mut last_result: Option<AgentResult> = None;

        for reflector_attempt in 0..=MAX_REFLECTOR_RETRIES {
            let exec_result = if reflector_attempt == 0 {
                executor
                    .execute_subtask(&planned.title, &augmented_description, exec_ctx, Some(agent_type))
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

                    if agent_result.content.contains("[NEEDS_USER_INPUT]") {
                        let prompt = agent_result
                            .content
                            .replace("[NEEDS_USER_INPUT]", "")
                            .trim()
                            .to_string();

                        if let Some(ref st) = db_subtask {
                            let _ = subtasks::update_status(
                                &*self.repo,
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
                                        &*self.repo,
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
                                &*self.repo,
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
}
