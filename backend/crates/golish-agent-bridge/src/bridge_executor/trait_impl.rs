//! `AgentExecutor` trait implementation for `BridgeAgentExecutor`.

use anyhow::{Context, Result};

use golish_agent_kit::task_orchestrator::{
    backfill_harness_stage, prompts, AgentExecutor, AgentResult, AgentTokenUsage, ExecutionContext,
    GeneratorOutput, PlannedSubtask, RefinerOutput,
};

use super::{extract_json_from_response, truncate_to_char_boundary, BridgeAgentExecutor};

#[async_trait::async_trait]
impl AgentExecutor for BridgeAgentExecutor {
    async fn generate_subtasks(&self, task_input: &str) -> Result<GeneratorOutput> {
        tracing::info!("[TaskMode/Generator] Decomposing task into subtasks");
        let response = self
            .simple_completion_for_phase(
                prompts::generator_prompt(),
                task_input,
                Some("pipeline_generator"),
            )
            .await
            .context("Generator LLM call failed")?;

        let json_str = extract_json_from_response(&response);
        let mut output: GeneratorOutput = serde_json::from_str(json_str).context(format!(
            "Failed to parse generator JSON. Raw response:\n{}",
            truncate_to_char_boundary(&response, 500)
        ))?;

        // Phase 1 MVP · Operation Harness:
        // LLM may omit `harness_stage` even when subtask matches a known stage.
        // Run deterministic keyword-based backfill as a safety net. LLM-supplied
        // values are preserved (backfill only fills `None` slots).
        let _filled = backfill_harness_stage(&mut output.subtasks);

        Ok(output)
    }

    async fn execute_subtask(
        &self,
        subtask_title: &str,
        subtask_description: &str,
        execution_context: &ExecutionContext,
        agent_type: Option<&str>,
    ) -> Result<AgentResult> {
        let agent_label = agent_type.unwrap_or("primary");
        tracing::info!(
            "[TaskMode] Executing subtask: {} (suggested agent: {})",
            subtask_title,
            agent_label,
        );
        let start = std::time::Instant::now();

        let prompt = {
            let registry = self.bridge.sub_agent_registry();
            let reg = registry.read().await;
            let orchestrator_prompt = reg.get("orchestrator").map(|def| {
                def.system_prompt
                    .replace("{{execution_context}}", &execution_context.render_xml())
            });
            drop(reg);

            if let Some(base_prompt) = orchestrator_prompt {
                format!(
                    "{}\n\n## CURRENT SUBTASK\n\nTitle: {}\nDescription: {}\n{}",
                    base_prompt,
                    subtask_title,
                    subtask_description,
                    agent_type
                        .map(|a| format!("Suggested specialist: {}", a))
                        .unwrap_or_default(),
                )
            } else {
                tracing::warn!(
                    "[TaskMode] Orchestrator agent not found in registry, using fallback prompt"
                );
                prompts::primary_agent_subtask_prompt_with_agent(
                    subtask_title,
                    subtask_description,
                    &execution_context.summary(),
                    agent_type,
                )
            }
        };

        let content = self.bridge.execute_isolated(&prompt).await?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(AgentResult::with_usage(
            content,
            AgentTokenUsage {
                input_tokens: 0,
                output_tokens: 0,
                duration_ms,
                phase: format!("primary_subtask:{}", agent_label),
            },
        ))
    }

    async fn refine_plan(
        &self,
        execution_context: &ExecutionContext,
        remaining_subtasks: &[PlannedSubtask],
    ) -> Result<RefinerOutput> {
        tracing::info!(
            "[TaskMode/Refiner] Refining plan ({} remaining subtasks)",
            remaining_subtasks.len()
        );
        let remaining_json = serde_json::to_string_pretty(remaining_subtasks)?;
        let system = prompts::refiner_prompt(&execution_context.summary(), &remaining_json);

        let response = self
            .simple_completion_for_phase(
                &system,
                "Analyze completed work and adjust the remaining plan.",
                Some("pipeline_refiner"),
            )
            .await
            .context("Refiner LLM call failed")?;

        let json_str = extract_json_from_response(&response);
        serde_json::from_str::<RefinerOutput>(json_str).context(format!(
            "Failed to parse refiner JSON. Raw response:\n{}",
            truncate_to_char_boundary(&response, 500)
        ))
    }

    async fn generate_report(&self, execution_context: &ExecutionContext) -> Result<AgentResult> {
        tracing::info!("[TaskMode/Reporter] Generating final report");
        let start = std::time::Instant::now();
        let system = prompts::reporter_prompt(&execution_context.summary());
        let content = self
            .simple_completion_for_phase(
                &system,
                "Generate the final task report based on all completed subtask results.",
                Some("pipeline_reporter"),
            )
            .await
            .context("Reporter LLM call failed")?;
        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(AgentResult::with_usage(
            content,
            AgentTokenUsage {
                input_tokens: 0,
                output_tokens: 0,
                duration_ms,
                phase: "reporter".to_string(),
            },
        ))
    }

    async fn reflect(&self, subtask_title: &str, agent_response: &str) -> Result<String> {
        tracing::info!(
            "[TaskMode/Reflector] Agent returned text for '{}', redirecting to tool usage",
            subtask_title
        );
        let system = prompts::reflector_system_prompt();
        let user = prompts::reflector_user_prompt(subtask_title, agent_response);
        self.simple_completion_for_phase(system, &user, Some("pipeline_reflector"))
            .await
            .context("Reflector LLM call failed")
    }

    async fn enrich_subtask(
        &self,
        subtask_title: &str,
        subtask_description: &str,
        execution_context: &ExecutionContext,
        agent_type: &str,
    ) -> Result<Option<String>> {
        if execution_context.completed_results.is_empty() {
            return Ok(None);
        }

        tracing::info!(
            "[TaskMode/Enricher] Enriching subtask '{}' for {} agent",
            subtask_title,
            agent_type
        );

        let user_msg = prompts::enricher_user_prompt(
            subtask_title,
            subtask_description,
            agent_type,
            &execution_context.summary(),
        );

        match self
            .simple_completion_for_phase(
                prompts::enricher_system_prompt(),
                &user_msg,
                Some("pipeline_enricher"),
            )
            .await
        {
            Ok(enrichment) => {
                let trimmed = enrichment.trim();
                if trimmed.is_empty() || trimmed.to_lowercase().contains("no additional context") {
                    tracing::debug!("[TaskMode/Enricher] No enrichment needed");
                    Ok(None)
                } else {
                    tracing::info!("[TaskMode/Enricher] Enrichment: {} chars", trimmed.len());
                    Ok(Some(trimmed.to_string()))
                }
            }
            Err(e) => {
                tracing::warn!("[TaskMode/Enricher] Failed (non-fatal): {}", e);
                Ok(None)
            }
        }
    }

    async fn plan_subtask(
        &self,
        subtask_title: &str,
        subtask_description: &str,
        execution_context: &ExecutionContext,
        agent_type: &str,
    ) -> Result<Option<String>> {
        tracing::info!(
            "[TaskMode/Planner] Generating execution plan for '{}'",
            subtask_title
        );

        let user_msg = prompts::task_planner_user_prompt(
            agent_type,
            subtask_title,
            subtask_description,
            &execution_context.summary(),
        );

        match self
            .simple_completion_for_phase(
                prompts::task_planner_system_prompt(),
                &user_msg,
                Some("pipeline_planner"),
            )
            .await
        {
            Ok(plan) => {
                let trimmed = plan.trim();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    tracing::info!("[TaskMode/Planner] Plan generated: {} chars", trimmed.len());
                    Ok(Some(trimmed.to_string()))
                }
            }
            Err(e) => {
                tracing::warn!("[TaskMode/Planner] Failed (non-fatal): {}", e);
                Ok(None)
            }
        }
    }

    async fn monitor_execution(
        &self,
        subtask_description: &str,
        repeated_tool: &str,
        repeat_count: usize,
        recent_tool_calls: &str,
    ) -> Result<Option<String>> {
        tracing::info!(
            "[TaskMode/Monitor] Tool '{}' called {} times, requesting advice",
            repeated_tool,
            repeat_count
        );

        let user_msg = prompts::mentor_user_prompt(
            subtask_description,
            repeated_tool,
            repeat_count,
            recent_tool_calls,
        );

        match self
            .simple_completion_for_phase(
                prompts::mentor_system_prompt(),
                &user_msg,
                Some("pipeline_monitor"),
            )
            .await
        {
            Ok(advice) => {
                let trimmed = advice.trim();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    tracing::info!(
                        "[TaskMode/Monitor] Advice generated: {} chars",
                        trimmed.len()
                    );
                    Ok(Some(trimmed.to_string()))
                }
            }
            Err(e) => {
                tracing::warn!("[TaskMode/Monitor] Failed (non-fatal): {}", e);
                Ok(None)
            }
        }
    }
}
