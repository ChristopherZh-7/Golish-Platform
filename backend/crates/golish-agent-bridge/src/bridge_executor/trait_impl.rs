//! `AgentExecutor` trait implementation for `BridgeAgentExecutor`.

use anyhow::{Context, Result};
use tracing::Instrument;

use golish_agent_kit::harness::StageKind;
use golish_agent_kit::task_orchestrator::{
    prompts, AgentExecutor, AgentResult, AgentTokenUsage, ExecutionContext,
};
use golish_sub_agents::SubAgentContext;

use super::BridgeAgentExecutor;

/// PR1 (设计 2026-06-11-coverage-auto-derive §5.1) · whether the deliverable
/// captured in the side-channel belongs to `stage`. The reflector retry loop
/// re-enters `execute_subtask` once per gate attempt, so a per-call sink reset
/// would discard the previous attempt's authoritative submission — exactly the
/// run that ends `content_len=0` BLOCK after the model already submitted. Same
/// stage ⇒ keep the capture as a fallback; stage switch / no stage / unparseable
/// capture ⇒ reset (cross-stage pollution stays impossible).
#[allow(dead_code)]
fn captured_belongs_to_stage(captured: Option<&str>, stage: Option<StageKind>) -> bool {
    let (Some(json), Some(stage)) = (captured, stage) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| {
            v.get("stage_id")
                .and_then(|s| s.as_str())
                .map(|s| s == stage.as_str())
        })
        .unwrap_or(false)
}

/// PR1 · final content fed to the stage-close gate. A side-channel capture (the
/// submit tool's structured, serde-round-tripped submission) is authoritative:
/// it is appended as a trailing ```json fence, and the gate's parser takes the
/// LAST parseable fence — so it supersedes any draft in the agent's prose and
/// covers the `agent_text == ""` weak-model case. No capture ⇒ text unchanged
/// (stage-close stays fail-closed on prose-only output).
fn resolve_gate_content(agent_text: &str, side_channel: Option<&str>) -> String {
    match side_channel {
        Some(d) if !d.trim().is_empty() => {
            if d.contains("```json") {
                format!("{agent_text}\n\n{d}")
            } else {
                format!("{agent_text}\n\n```json\n{d}\n```")
            }
        }
        _ => agent_text.to_string(),
    }
}

/// The operation's `task_input` is the one common source for GUI Task requests
/// and headless CLI `-e`. Keep it out of the primary system prompt policy, but
/// carry it as data so `stage_run` can quote a bounded, lower-priority excerpt in
/// its specialist objective. This context is request-local and does not alter the
/// bridge's stage-run reentry guard or worker-chain persistence.
fn primary_loop_context(execution_context: &ExecutionContext) -> SubAgentContext {
    SubAgentContext {
        original_request: execution_context.task_input.clone(),
        depth: 0,
        ..Default::default()
    }
}

#[async_trait::async_trait]
impl AgentExecutor for BridgeAgentExecutor {
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
        // Scrub first as a same-request fallback for a previously dropped
        // subtask future; publication then validates before writing any handle.
        self.bridge.clear_active_subtask_context().await;
        self.bridge
            .publish_active_execution_context(execution_context)
            .await?;
        // C2c/PR1 · one capture belongs to one provider turn. The harness retry
        // loop re-enters this method after a Gate BLOCK, so retaining a
        // same-stage capture here would grade the previous rejected payload when
        // the model fails to submit a corrected one. Clear both sinks before the
        // turn; a successful submit during this turn repopulates them, while a
        // prose-only/invalid retry remains fail-closed as a missing deliverable.
        *self.bridge.harness_captured_submission.write().await = None;
        *self.bridge.harness_last_deliverable.write().await = None;

        // Poll the provider-heavy isolated turn as its own scheduled task. A
        // plain Box only moves Future state to the heap; it does not unwind the
        // caller's native poll frames before polling the child. The debug
        // provider dispatch plus TaskOrchestrator chain can therefore exceed
        // the production 32 MiB stage-run worker stack on its first poll.
        //
        // JoinSet is deliberate: dropping this parent future aborts the child,
        // unlike a detached JoinHandle, so Stop/cancellation cannot leave an
        // isolated agent continuing to execute tools in the background.
        let mut isolated_turn = tokio::task::JoinSet::new();
        let turn_bridge = self.bridge.clone();
        let turn_context = primary_loop_context(execution_context);
        isolated_turn.spawn(
            async move {
                turn_bridge
                    .execute_isolated_with_context(&prompt, turn_context)
                    .await
            }
            .in_current_span(),
        );
        let content_result = match isolated_turn.join_next().await {
            Some(Ok(result)) => result,
            Some(Err(join_error)) => Err(anyhow::anyhow!(
                "TaskMode isolated provider task failed: {join_error}"
            )),
            None => Err(anyhow::anyhow!(
                "TaskMode isolated provider task ended without a result"
            )),
        };
        // Every active authorization/runtime-identity handle is subtask-local.
        // Clear them before propagating an isolated-loop error so the next
        // subtask cannot inherit this stage, unit, organization, or worker lease.
        self.bridge.clear_active_subtask_context().await;
        let content = content_result?;

        // C2c/PR1 · The orchestrator often delegates the StageDeliverable to a
        // sub-agent / the submit tool and then narrates (or says nothing): the
        // gate parses only this content, so append the captured submission as a
        // trailing ```json fence. The gate's parser takes the LAST parseable
        // fence — the structured capture is authoritative over prose drafts.
        let trusted_capture = self.bridge.harness_captured_submission.read().await.clone();
        let content = if execution_context.harness_stage.is_some() {
            let legacy_capture = self.bridge.harness_last_deliverable.read().await.clone();
            let captured = trusted_capture
                .as_ref()
                .map(|capture| capture.canonical_deliverable_json.as_str())
                .or(legacy_capture.as_deref());
            if let Some(d) = captured {
                tracing::info!(
                    "[TaskMode] appending captured StageDeliverable to gate content ({} chars, agent_text {} chars)",
                    d.len(),
                    content.len()
                );
            }
            resolve_gate_content(&content, captured)
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
        )
        .with_captured_stage_submission(trusted_capture))
    }

    fn stage_run_retry_budget_exhausted(&self, stage: StageKind) -> bool {
        self.bridge.stage_run_reentry_guard.is_exhausted(stage)
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

#[cfg(test)]
mod tests {
    use super::*;

    const DELIVERABLE: &str = r#"{"stage_id":"target_intel","stage_run_id":"3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33","claims":[],"evidence_refs":[],"findings":[]}"#;

    // PR1 · the weak-model failure: empty agent text + a captured structured
    // submission must still reach the gate as a parseable ```json fence.
    #[test]
    fn empty_agent_text_falls_back_to_side_channel() {
        let out = resolve_gate_content("", Some(DELIVERABLE));
        assert!(out.contains("```json"));
        assert!(out.contains("target_intel"));
    }

    // PR1 · the capture is appended LAST so the gate's last-parseable-fence
    // parser treats the structured submission as authoritative over any draft
    // deliverable in the agent's prose.
    #[test]
    fn side_channel_is_appended_after_agent_text() {
        let prose = "draft:\n```json\n{\"stage_id\":\"scoping\"}\n```";
        let out = resolve_gate_content(prose, Some(DELIVERABLE));
        let draft_pos = out.find("scoping").unwrap();
        let capture_pos = out.find("target_intel").unwrap();
        assert!(
            capture_pos > draft_pos,
            "capture must trail the prose draft"
        );
    }

    // PR1 · no capture / blank capture → text unchanged (fail-closed at the gate).
    #[test]
    fn no_side_channel_returns_text_unchanged() {
        assert_eq!(resolve_gate_content("prose only", None), "prose only");
        assert_eq!(resolve_gate_content("prose only", Some("  ")), "prose only");
    }

    // PR1 · a capture already fenced is appended verbatim (no double fencing).
    #[test]
    fn already_fenced_capture_is_not_double_fenced() {
        let fenced = format!("```json\n{DELIVERABLE}\n```");
        let out = resolve_gate_content("", Some(&fenced));
        assert_eq!(out.matches("```json").count(), 1);
    }

    // PR1c · same-stage capture survives the per-attempt sink reset; anything
    // else (stage switch / no stage / unparseable) is discarded.
    #[test]
    fn capture_kept_only_for_matching_stage() {
        assert!(captured_belongs_to_stage(
            Some(DELIVERABLE),
            Some(StageKind::TargetIntel)
        ));
        assert!(!captured_belongs_to_stage(
            Some(DELIVERABLE),
            Some(StageKind::Scoping)
        ));
        assert!(!captured_belongs_to_stage(Some(DELIVERABLE), None));
        assert!(!captured_belongs_to_stage(
            Some("not json"),
            Some(StageKind::TargetIntel)
        ));
        assert!(!captured_belongs_to_stage(
            None,
            Some(StageKind::TargetIntel)
        ));
    }

    #[test]
    fn primary_loop_context_carries_cli_and_gui_top_level_requests() {
        for request in [
            "CLI -e: do not call producers for five unreachable exact origins",
            "GUI request: keep Enumeration read-only and process batches of at most 25",
        ] {
            let execution_context = ExecutionContext {
                task_input: request.to_string(),
                ..Default::default()
            };

            let context = primary_loop_context(&execution_context);

            assert_eq!(context.original_request, request);
            assert_eq!(context.depth, 0);
        }
    }
}
