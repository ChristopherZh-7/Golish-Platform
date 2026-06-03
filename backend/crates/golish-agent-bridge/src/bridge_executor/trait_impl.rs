//! `AgentExecutor` trait implementation for `BridgeAgentExecutor`.

use anyhow::{Context, Result};

use golish_agent_kit::task_orchestrator::{
    backfill_harness_stage, prompts, AgentExecutor, AgentResult, AgentTokenUsage, ExecutionContext,
    GeneratorOutput, PlannedSubtask, RefinerOutput,
};

use super::{describe_plan_parse_failure, extract_json_from_response, BridgeAgentExecutor};

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
        // A refusal / clarifying question / non-JSON prose response must NOT be
        // surfaced as a cryptic `expected value at line 1 column 1` serde error.
        // `describe_plan_parse_failure` splits "model declined" (clean message,
        // no serde noise) from genuinely malformed JSON (keep the serde detail).
        let mut output: GeneratorOutput = serde_json::from_str(json_str).map_err(|e| {
            anyhow::anyhow!(describe_plan_parse_failure(
                "task planner",
                &response,
                json_str,
                &e,
            ))
        })?;

        // Phase 1 MVP · Operation Harness:
        // LLM may omit `harness_stage` even when subtask matches a known stage.
        // Run deterministic keyword-based backfill as a safety net. LLM-supplied
        // values are preserved (backfill only fills `None` slots).
        let _filled = backfill_harness_stage(&mut output.subtasks);

        Ok(output)
    }

    async fn generate_stage_plan(
        &self,
        stage: golish_agent_kit::harness::StageKind,
        task_input: &str,
        upstream_evidence: &str,
    ) -> Result<Vec<PlannedSubtask>> {
        let stage_id = stage.as_str();
        tracing::info!("[TaskMode/StagePlan] Lazily planning stage '{stage_id}'");
        let user = format!(
            "Target: {task_input}\n\nUpstream evidence so far:\n{upstream_evidence}\n\n\
             Plan ONLY the current stage: `{stage_id}`. Produce 2-4 concrete, executable \
             subtasks that advance THIS stage. The FINAL subtask MUST be \
             \"Submit & verify the {stage_id} StageDeliverable\" and instruct the agent to call \
             the submit_stage_deliverable tool. Do not plan other stages."
        );
        let response = match self
            .simple_completion_for_phase(
                prompts::generator_prompt(),
                &user,
                Some("pipeline_generator"),
            )
            .await
        {
            Ok(r) => r,
            // LLM/transport failure → empty so the orchestrator falls back to the
            // single-subtask synthesizer (never spin, never vacuous pass).
            Err(e) => {
                tracing::warn!("[TaskMode/StagePlan] LLM failed (fallback to synth): {e}");
                return Ok(Vec::new());
            }
        };
        let json_str = extract_json_from_response(&response);
        // Refusal / non-JSON → empty → caller falls back to the synth single step.
        let mut output: GeneratorOutput = match serde_json::from_str(json_str) {
            Ok(o) => o,
            Err(_) => return Ok(Vec::new()),
        };
        if output.subtasks.is_empty() {
            return Ok(Vec::new());
        }
        // Deterministically tag every step with this stage + guarantee a submit
        // terminal (the model may forget the closing submit step).
        let agent = if matches!(stage, golish_agent_kit::harness::StageKind::Reporting) {
            "analyzer"
        } else {
            "pentester"
        };
        let hint = golish_agent_kit::harness::HarnessStageHint::new(stage);
        for st in output.subtasks.iter_mut() {
            st.harness_stage = Some(hint.clone());
        }
        ensure_submit_terminal(&mut output.subtasks, stage_id, agent);
        Ok(output.subtasks)
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

        // C2b · Stage discipline. When this subtask belongs to a harness stage,
        // append a final high-salience directive so the agent (1) ends with the
        // StageDeliverable JSON the deterministic gate parses, and (2) stops +
        // reports instead of rabbit-holing when a tool is unavailable. Appended
        // last = most recent → outranks the base orchestrator prompt's generic
        // prose-completion instructions. No-op for non-stage subtasks.
        let prompt = if execution_context.harness_stage.is_some() {
            format!("{}\n\n{}", prompt, prompts::stage_discipline_reminder())
        } else {
            prompt
        };

        // C3 · publish this subtask's harness stage + authorization context to the
        // bridge side-channel so the agentic loop's per-tool gate can enforce the
        // stage forbidden-tool barrier (stage) and the full pre-action authorizer
        // (authz). `None` when stage_mode is off or the subtask has no stage.
        *self.bridge.harness_active_stage.write().await = execution_context.harness_stage;
        *self.bridge.harness_active_authz.write().await = execution_context.harness_authz;
        // C2c · reset the per-subtask deliverable-capture sink before running.
        *self.bridge.harness_last_deliverable.write().await = None;

        let content = self.bridge.execute_isolated(&prompt).await?;

        // C2c · The Primary orchestrator often delegates the StageDeliverable to
        // `sub_agent_reporter` and then narrates instead of inlining the JSON, so
        // the gate (which parses only this content) would skip with "no parseable
        // StageDeliverable". If this is a stage subtask and the content has no
        // deliverable signature, recover the one a sub-agent produced and append
        // it as a fenced ```json block so the gate can parse it.
        let content = if execution_context.harness_stage.is_some() {
            match self.bridge.harness_last_deliverable.read().await.clone() {
                Some(deliverable) => {
                    tracing::info!(
                        "[TaskMode] Orchestrator omitted StageDeliverable; appending one captured from a sub-agent ({} chars)",
                        deliverable.len()
                    );
                    if deliverable.contains("```json") {
                        format!("{content}\n\n{deliverable}")
                    } else {
                        format!("{content}\n\n```json\n{deliverable}\n```")
                    }
                }
                None => content,
            }
        } else {
            content
        };

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
        // Same refusal-vs-malformed split as the generator (see generate_subtasks).
        serde_json::from_str::<RefinerOutput>(json_str).map_err(|e| {
            anyhow::anyhow!(describe_plan_parse_failure(
                "plan refiner",
                &response,
                json_str,
                &e,
            ))
        })
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

/// Ensure a lazily-generated stage plan ends with a submit-and-verify step
/// (deterministic — the model may forget the closing submit). No-op when the
/// last step already references a submit.
fn ensure_submit_terminal(subtasks: &mut Vec<PlannedSubtask>, stage_id: &str, agent: &str) {
    let last_is_submit = subtasks.last().is_some_and(|s| {
        s.title.to_lowercase().contains("submit")
            || s.description.contains("submit_stage_deliverable")
    });
    if last_is_submit {
        return;
    }
    subtasks.push(PlannedSubtask {
        title: format!("Submit & verify the {stage_id} StageDeliverable"),
        description: format!(
            "Compile this stage's findings and call the submit_stage_deliverable tool with the \
             structured StageDeliverable (stage_id, stage_run_id, claims, evidence_refs, findings) \
             for the '{stage_id}' stage. Cite only real evidence-ledger ids produced this run."
        ),
        agent: Some(agent.to_string()),
        harness_stage: None,
        nl_slice: None,
        acceptance_criteria: Vec::new(),
    });
}

#[cfg(test)]
mod stage_plan_tests {
    use super::ensure_submit_terminal;
    use golish_agent_kit::task_orchestrator::PlannedSubtask;

    fn pt(title: &str) -> PlannedSubtask {
        PlannedSubtask {
            title: title.to_string(),
            description: title.to_string(),
            agent: Some("pentester".to_string()),
            harness_stage: None,
            nl_slice: None,
            acceptance_criteria: Vec::new(),
        }
    }

    #[test]
    fn appends_submit_when_missing() {
        let mut v = vec![pt("Enumerate subdomains")];
        ensure_submit_terminal(&mut v, "enumeration", "pentester");
        assert_eq!(v.len(), 2);
        let last = v.last().unwrap();
        assert!(last.title.to_lowercase().contains("submit"));
        assert!(last.description.contains("submit_stage_deliverable"));
    }

    #[test]
    fn keeps_existing_submit_terminal() {
        let mut v = vec![
            pt("recon"),
            pt("Submit & verify the enumeration StageDeliverable"),
        ];
        ensure_submit_terminal(&mut v, "enumeration", "pentester");
        assert_eq!(v.len(), 2, "must not double-append a submit step");
    }
}
