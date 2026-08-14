//! Build the effective system prompt for a sub-agent invocation.
//!
//! Composition order (matches the legacy monolithic loop):
//! 1. Optionally generate an optimized prompt via a non-streaming completion
//!    when [`SubAgentDefinition::prompt_template`] is set (worker pattern).
//! 2. Append the orchestrator briefing (if any).
//! 3. Inject any skills matched against the task description.
//! 4. Append the barrier-tool completion-requirement instruction.

use rig::completion::{AssistantContent, CompletionModel as RigCompletionModel, Message};
use rig::message::{Text, UserContent};
use rig::one_or_many::OneOrMany;

use crate::definition::SubAgentDefinition;
use crate::executor_types::{
    BoundTerminalExecutionContract, SubAgentExecutorContext, BARRIER_TOOL_NAME,
    INVESTIGATION_ASSET_VERIFICATION_ACTOR_OBSERVATION_SCHEMA,
    INVESTIGATION_ASSET_VERIFICATION_PRIMARY_RESOLUTION_SCHEMA,
    INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_SCHEMA,
    INVESTIGATION_PRIMARY_SYNTHESIS_RESULT_SCHEMA, INVESTIGATION_REFINER_PATCH_RESULT_SCHEMA,
    INVESTIGATION_TASK_PLAN_RESULT_SCHEMA,
};
use golish_core::events::AiEvent;

/// Compose the final system prompt the sub-agent will run with.
pub(super) async fn assemble_effective_system_prompt<M>(
    agent_def: &SubAgentDefinition,
    task: &str,
    additional_context: &str,
    ctx: &SubAgentExecutorContext<'_>,
    parent_request_id: &str,
    model: &M,
) -> String
where
    M: RigCompletionModel + Sync,
{
    let agent_id = &agent_def.id;

    if let Some(contract) = ctx
        .bound_worker_chain
        .as_ref()
        .and_then(|bound| bound.terminal_execution.as_ref())
    {
        let mut effective =
            assemble_bound_terminal_prompt(agent_def, ctx.briefing.as_deref(), contract);
        if contract.inject_workspace_skills {
            inject_matched_skills(agent_id, task, ctx, &mut effective).await;
        }
        tracing::info!(
            target: "sub_agent::prompt_dump",
            agent_id = %agent_id,
            allowed_tools = ?agent_def.allowed_tools,
            prompt_len = effective.len(),
            terminal_only = true,
            "[sub-agent-prompt-dump] assembled terminal system prompt for '{agent_id}':\n{effective}"
        );
        return effective;
    }

    let mut effective = if let Some(ref template) = agent_def.prompt_template {
        generate_optimized_prompt(
            agent_id,
            template,
            task,
            additional_context,
            ctx,
            parent_request_id,
            model,
            &agent_def.system_prompt,
        )
        .await
    } else {
        agent_def.system_prompt.clone()
    };

    if let Some(ref briefing) = ctx.briefing {
        effective.push_str("\n\n");
        effective.push_str(briefing);
        tracing::info!(
            "[sub-agent:{}] Injected orchestrator briefing ({} chars)",
            agent_id,
            briefing.len()
        );
    }

    append_investigation_actor_contract(&mut effective, ctx);

    inject_matched_skills(agent_id, task, ctx, &mut effective).await;

    if let Some(bound) = ctx
        .bound_worker_chain
        .as_ref()
        .filter(|bound| bound.is_stage_team_child())
    {
        let output_schema = if matches!(
            bound.investigation_actor_contract.as_ref(),
            Some(crate::InvestigationActorContract::AssetVerificationPrimary(
                _
            ))
        ) {
            Some(INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_SCHEMA)
        } else {
            bound.stage_team_output_schema.as_deref()
        };
        effective.push_str(&stage_team_completion_requirement(output_schema));
    } else {
        effective.push_str(&format!(
            "\n\n## COMPLETION REQUIREMENT\n\n\
             When your task is complete, you MUST call the `{}` tool to submit your result. \
             Do NOT end with a plain text message — always use `{}` with:\n\
             - `result`: your full findings, outputs, or deliverables\n\
             - `success`: true if the task was completed, false if it failed\n\
             - `summary`: a one-line summary of what was accomplished",
            BARRIER_TOOL_NAME, BARRIER_TOOL_NAME
        ));
    }

    // Observability aid: a sub-agent's fully-assembled system prompt is otherwise
    // NOT captured anywhere (its per-sub-agent transcript logs only tool
    // calls/results, and backend.log never records prompts). Dump it here — with
    // the advertised tool list — so a run's backend.log / run.log shows exactly
    // what each sub-agent received. `target` is greppable: `sub_agent::prompt_dump`.
    tracing::info!(
        target: "sub_agent::prompt_dump",
        agent_id = %agent_id,
        allowed_tools = ?agent_def.allowed_tools,
        prompt_len = effective.len(),
        "[sub-agent-prompt-dump] assembled system prompt for '{agent_id}':\n{effective}"
    );

    effective
}

fn stage_team_completion_requirement(output_schema: Option<&str>) -> String {
    let result_contract = match output_schema {
        Some(INVESTIGATION_TASK_PLAN_RESULT_SCHEMA) => format!(
            "Its `result` argument MUST be the exact {INVESTIGATION_TASK_PLAN_RESULT_SCHEMA} object \
             shown by the tool schema: `schema_version`, `summary`, and 0-8 ordered `subtasks`. \
             Choose roles and ordering from the actual bounded need. Roles may repeat or be absent; \
             there is no fixed committee or role census. An empty subtask list means the same \
             Primary can proceed directly to its host-validated synthesis or zero-hypothesis result."
        ),
        Some(INVESTIGATION_REFINER_PATCH_RESULT_SCHEMA) => format!(
            "Its `result` argument MUST be the exact {INVESTIGATION_REFINER_PATCH_RESULT_SCHEMA} \
             object shown by the tool schema: preserve the frozen remaining identity set exactly."
        ),
        Some(INVESTIGATION_PRIMARY_SYNTHESIS_RESULT_SCHEMA) => format!(
            "Its `result` argument MUST be the exact \
             {INVESTIGATION_PRIMARY_SYNTHESIS_RESULT_SCHEMA} object shown by the tool schema, \
             including every accepted output hash exactly once."
        ),
        Some(INVESTIGATION_ASSET_VERIFICATION_ACTOR_OBSERVATION_SCHEMA) => format!(
            "Its `result` argument MUST be the exact \
             {INVESTIGATION_ASSET_VERIFICATION_ACTOR_OBSERVATION_SCHEMA} object shown by the tool \
             schema. Report only this dynamic actor call's observation for the exact session and \
             hypothesis revision, cite only durable evidence returned by tools, and \
             put distinct follow-up ideas in `new_hypothesis_proposals`."
        ),
        Some(INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_SCHEMA) => format!(
            "Its `result` argument MUST be the exact tagged \
             {INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_SCHEMA} object shown by the tool \
             schema. Use `decision=delegate` with 1-8 ordered exact-subject subtasks when more \
             work is needed; roles may repeat. Use `decision=resolve` with zero subtasks only \
             when the current hypothesis is terminally verified, refuted, or invalid. Evidence \
             citations are optional (0-N) but every supplied id must come from this session's \
             completed invocation audit evidence. Never invent or submit opaque authority ids."
        ),
        Some(INVESTIGATION_ASSET_VERIFICATION_PRIMARY_RESOLUTION_SCHEMA) => format!(
            "Its `result` argument MUST be the exact \
             {INVESTIGATION_ASSET_VERIFICATION_PRIMARY_RESOLUTION_SCHEMA} object shown by the \
             tool schema. Resolve the hypothesis only as verified/refuted/invalid from the \
             current revision and durable observations. The host derives all authority ids and hashes; \
             do not invent them."
        ),
        Some("investigation_cognitive_output.v1") =>
            "Its `result` argument MUST be the exact investigation_cognitive_output.v1 object \
             shown by the tool schema. Authority-bearing fact/evidence/checked-empty arrays must \
             remain empty; return only advisory proposal_signals, action_intents, and residuals."
                .to_string(),
        Some(output_schema) => format!(
            "Its `result` argument MUST be the exact {output_schema} object shown by the tool \
             schema, including `business_disposition`, `summary`, `fact_refs`, `evidence_ids`, \
             `checked_empty_units`, and `blocker_code`. Do not claim evidence that was not \
             durably booked."
        ),
        None => "Its `result` argument MUST match the exact object shown by the submit_result \
                 tool schema."
            .to_string(),
    };
    format!(
        "\n\n## COMPLETION REQUIREMENT\n\nWhen your task is complete, you MUST call the \
         `{BARRIER_TOOL_NAME}` tool. Do NOT end with a plain text message. {result_contract} Return \
         the object itself, never a JSON string, Markdown report, code fence, or prose wrapper. \
         Also set outer `success` and one-line outer `summary`."
    )
}

fn append_investigation_actor_contract(effective: &mut String, ctx: &SubAgentExecutorContext<'_>) {
    let Some(contract) = ctx
        .bound_worker_chain
        .as_ref()
        .and_then(|bound| bound.investigation_actor_contract.as_ref())
    else {
        return;
    };
    let directive = match contract {
        crate::InvestigationActorContract::AnalysisPrimary =>
            "You are the Analysis Primary. You may plan and use exact-scope read-only cognition, but may not perform target I/O or establish canonical truth.".to_string(),
        crate::InvestigationActorContract::AnalysisWorker =>
            "You are an Analysis reasoning worker. Remain read-only and return advisory material only.".to_string(),
        crate::InvestigationActorContract::AssetVerificationPrimary(binding) => {
            let identity = serde_json::json!({
                "asset_lane_id": binding.asset_lane_id,
                "hypothesis_revision_id": binding.hypothesis_revision_id,
                "message_chain_id": binding.message_chain_id,
                "session_id": binding.session_id,
                "target_id": binding.target_id,
                "work_item_id": binding.work_item_id,
                "worker_run_id": binding.worker_run_id,
            });
            format!(
                "You are the durable Asset Verification Primary for exactly one hypothesis round. This is a distinct Verification authority contract, never an Analysis Primary contract. Decide one turn at a time: delegate an ordered 1-8 actor batch, with any of the eight allowed roles repeatable, or resolve with zero actors. Every delegated subtask must carry exactly the current target and current hypothesis revision refs. Review only the host-projected actor completions and invocation authorities for this session. Supplied cited_evidence_ids must be fresh audit evidence from those invocations; zero citations is legal. Never manufacture authority ids, worker ids, hashes, tool receipts, or a foreign subject. A delegate turn may be followed by another Primary turn on this same durable chain.\nASSET VERIFICATION PRIMARY:\n{identity}"
            )
        }
        crate::InvestigationActorContract::AssetVerification(binding) => {
            let identity = serde_json::json!({
                "asset_lane_id": binding.asset_lane_id,
                "hypothesis_revision_id": binding.hypothesis_revision_id,
                "actor_call_id": binding.actor_call_id,
                "actor_ordinal": binding.actor_ordinal,
                "session_id": binding.session_id,
                "specialist_role": binding.specialist_role,
                "subtask_id": binding.subtask_id,
                "target_id": binding.target_id,
            });
            format!(
                "You are one {} call dynamically requested by the durable Asset Primary in an exact Verification session. You are not a mandatory roster member and you do not decide the final business resolution. Before the first managed skill/run/browser call in this round, call pentest_list_tools so the host freezes the current installed+enabled+ready Tool Manager inventory. You may then repeatedly inspect that inventory, read managed skills, run ready managed tools, and use the guarded Browser JS/API collector. Every target-I/O call is authorized, fenced, audited, and evidence-booked by the host. Never place session, lane, revision, worker-fence, credential, budget, or authorization fields in tool arguments; select only ordinary wrapper parameters. Stay on the exact target and hypothesis revision below, cite only fresh returned evidence, and finish this call with submit_result.\nASSET VERIFICATION ACTOR CALL:\n{identity}",
                binding.specialist_role,
            )
        }
    };
    effective.push_str("\n\n## INVESTIGATION ACTOR CONTRACT\n\n");
    effective.push_str(&directive);
}

fn assemble_bound_terminal_prompt(
    agent_def: &SubAgentDefinition,
    briefing: Option<&str>,
    contract: &BoundTerminalExecutionContract,
) -> String {
    let mut effective = agent_def.system_prompt.clone();
    if let Some(briefing) = briefing {
        effective.push_str("\n\n");
        effective.push_str(briefing);
    }
    effective.push_str("\n\n## TERMINAL COMPLETION CONTRACT\n\n");
    effective.push_str(&contract.completion_instruction);
    effective
}

/// Run the prompt-architect LLM call to translate a task description into a
/// fine-tuned worker system prompt. Falls back to the static
/// [`SubAgentDefinition::system_prompt`] on failure or empty response.
#[allow(clippy::too_many_arguments)]
async fn generate_optimized_prompt<M>(
    agent_id: &str,
    template: &str,
    task: &str,
    additional_context: &str,
    ctx: &SubAgentExecutorContext<'_>,
    parent_request_id: &str,
    model: &M,
    fallback_system_prompt: &str,
) -> String
where
    M: RigCompletionModel + Sync,
{
    let generation_input = if additional_context.is_empty() {
        format!("Task: {}", task)
    } else {
        format!("Task: {}\n\nContext: {}", task, additional_context)
    };

    tracing::info!(
        "[sub-agent:{}] Generating optimized system prompt via LLM call",
        agent_id
    );

    let _ = ctx.event_tx.send(AiEvent::PromptGenerationStarted {
        agent_id: agent_id.to_string(),
        parent_request_id: parent_request_id.to_string(),
        architect_system_prompt: template.to_string(),
        architect_user_message: generation_input.clone(),
    });

    let generation_start = std::time::Instant::now();

    let generation_request = rig::completion::CompletionRequest {
        preamble: Some(template.to_string()),
        chat_history: OneOrMany::one(Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: generation_input,
            })),
        }),
        documents: vec![],
        tools: vec![],
        temperature: Some(0.3),
        max_tokens: Some(2048),
        tool_choice: None,
        additional_params: None,
        model: None,
        output_schema: None,
    };

    match model.completion(generation_request).await {
        Ok(response) => {
            let generated = response
                .choice
                .iter()
                .filter_map(|c| {
                    if let AssistantContent::Text(t) = c {
                        Some(t.text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("");

            if generated.trim().is_empty() {
                tracing::warn!(
                    "[sub-agent:{}] Prompt generation returned empty response, using default",
                    agent_id
                );
                let _ = ctx.event_tx.send(AiEvent::PromptGenerationCompleted {
                    agent_id: agent_id.to_string(),
                    parent_request_id: parent_request_id.to_string(),
                    generated_prompt: None,
                    success: false,
                    duration_ms: generation_start.elapsed().as_millis() as u64,
                });
                fallback_system_prompt.to_string()
            } else {
                tracing::info!(
                    "[sub-agent:{}] Generated system prompt ({} chars)",
                    agent_id,
                    generated.len()
                );
                let _ = ctx.event_tx.send(AiEvent::PromptGenerationCompleted {
                    agent_id: agent_id.to_string(),
                    parent_request_id: parent_request_id.to_string(),
                    generated_prompt: Some(generated.clone()),
                    success: true,
                    duration_ms: generation_start.elapsed().as_millis() as u64,
                });
                generated
            }
        }
        Err(e) => {
            tracing::warn!(
                "[sub-agent:{}] Prompt generation failed: {}. Using default system prompt.",
                agent_id,
                e
            );
            let _ = ctx.event_tx.send(AiEvent::PromptGenerationCompleted {
                agent_id: agent_id.to_string(),
                parent_request_id: parent_request_id.to_string(),
                generated_prompt: None,
                success: false,
                duration_ms: generation_start.elapsed().as_millis() as u64,
            });
            fallback_system_prompt.to_string()
        }
    }
}

/// Discover skills under the active workspace, match them against the task
/// description, and append matched skills as `<skill name="...">…</skill>`
/// blocks to the system prompt.
async fn inject_matched_skills(
    agent_id: &str,
    task: &str,
    ctx: &SubAgentExecutorContext<'_>,
    effective_system_prompt: &mut String,
) {
    let workspace = ctx.workspace.read().await;
    let workspace_str = workspace.to_string_lossy();
    let skills = golish_skills::discover_skills(Some(&workspace_str));
    if skills.is_empty() {
        return;
    }

    let metadata: Vec<golish_skills::SkillMetadata> = skills
        .iter()
        .map(golish_skills::SkillMetadata::from)
        .collect();
    let matcher = golish_skills::SkillMatcher::default();
    let matches = matcher.match_skills(task, &metadata);
    for (matched_meta, _score, reason) in &matches {
        if let Ok(body) = golish_skills::load_skill_body(&matched_meta.path) {
            effective_system_prompt.push_str("\n\n<skill name=\"");
            effective_system_prompt.push_str(&matched_meta.name);
            effective_system_prompt.push_str("\">\n");
            effective_system_prompt.push_str(&body);
            effective_system_prompt.push_str("\n</skill>");
            tracing::info!(
                "[sub-agent:{}] Injected skill '{}' ({} chars, reason: {})",
                agent_id,
                matched_meta.name,
                body.len(),
                reason
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::stage_team_completion_requirement;
    use crate::executor_types::{
        INVESTIGATION_ASSET_VERIFICATION_ACTOR_OBSERVATION_SCHEMA,
        INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_SCHEMA,
        INVESTIGATION_PRIMARY_SYNTHESIS_RESULT_SCHEMA, INVESTIGATION_REFINER_PATCH_RESULT_SCHEMA,
        INVESTIGATION_TASK_PLAN_RESULT_SCHEMA,
    };

    #[test]
    fn investigation_primary_completion_prompts_never_require_worker_output_v1() {
        for schema in [
            INVESTIGATION_TASK_PLAN_RESULT_SCHEMA,
            INVESTIGATION_REFINER_PATCH_RESULT_SCHEMA,
            INVESTIGATION_PRIMARY_SYNTHESIS_RESULT_SCHEMA,
        ] {
            let prompt = stage_team_completion_requirement(Some(schema));
            assert!(prompt.contains(schema));
            assert!(prompt.contains("submit_result"));
            assert!(!prompt.contains("stage_worker_output.v1"));
        }
        let plan_prompt =
            stage_team_completion_requirement(Some(INVESTIGATION_TASK_PLAN_RESULT_SCHEMA));
        assert!(plan_prompt.contains("0-8 ordered `subtasks`"));
        assert!(plan_prompt.contains("Roles may repeat or be absent"));
        assert!(plan_prompt.contains("no fixed committee or role census"));
    }

    #[test]
    fn asset_verification_observation_completion_prompt_requires_typed_contract() {
        let prompt = stage_team_completion_requirement(Some(
            INVESTIGATION_ASSET_VERIFICATION_ACTOR_OBSERVATION_SCHEMA,
        ));
        assert!(prompt.contains(INVESTIGATION_ASSET_VERIFICATION_ACTOR_OBSERVATION_SCHEMA));
        assert!(prompt.contains("dynamic actor call's observation"));
        assert!(prompt.contains("cite only durable evidence"));
        assert!(prompt.contains("submit_result"));
        assert!(!prompt.contains("business_disposition"));
    }

    #[test]
    fn dynamic_verification_primary_completion_prompt_preserves_closed_turn_semantics() {
        let prompt = stage_team_completion_requirement(Some(
            INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_SCHEMA,
        ));
        assert!(prompt.contains(INVESTIGATION_DYNAMIC_VERIFICATION_PRIMARY_TURN_SCHEMA));
        assert!(prompt.contains("decision=delegate"));
        assert!(prompt.contains("1-8 ordered exact-subject subtasks"));
        assert!(prompt.contains("roles may repeat"));
        assert!(prompt.contains("decision=resolve"));
        assert!(prompt.contains("zero subtasks"));
        assert!(prompt.contains("0-N"));
        assert!(prompt.contains("Never invent or submit opaque authority ids"));
    }
}
