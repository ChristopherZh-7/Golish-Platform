//! Single-subtask execution with enrichment, planning, and reflector retry.

use uuid::Uuid;

use crate::db_shim::subtasks;
use crate::db_traits::SubtaskStatus;
use golish_core::events::AiEvent;
use golish_core::plan::StepStatus;

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

            // C2 · harness stage charter (stage_mode 开 + 该 subtask 归属某 stage 时,
            // 前置注入允许/禁止工具面 + deliverable/gate 要求).
            if crate::harness::stage_mode_enabled() {
                if let Some(hint) = planned.harness_stage.as_ref() {
                    if let Ok(spec) = crate::harness::load_embedded_stage_spec(hint.stage_kind) {
                        desc.push_str(&super::super::prompts::stage_charter(&spec));
                        // C6 · handoff: inject which evidence kinds this stage
                        // inherits from upstream stages (consumes the otherwise
                        // runtime-dead `inherits_evidence_from`).
                        desc.push_str(&super::super::prompts::stage_inherited_evidence(&spec));
                        // C6 · real handoff: inject the actual gate-passed
                        // deliverable summaries from upstream stages recorded
                        // earlier in this operation.
                        desc.push_str(&super::super::prompts::render_inherited_handoff(
                            &spec,
                            &self.harness_evidence,
                        ));
                        // P3 · RAG prior: retrieve relevant prior writeups/PoCs
                        // from the wiki KB and inject them so the agent consults
                        // known exploits/findings before testing. (Graph prior
                        // needs a graph handle the orchestrator doesn't hold yet
                        // → wiki-only here; graph wiring is a follow-up.)
                        let pk = crate::harness::rag_prior::retrieve_wiki_prior(
                            &*self.repo,
                            &planned.title,
                            5,
                        )
                        .await;
                        desc.push_str(&crate::harness::rag_prior::render_prior_knowledge(&pk));
                    }
                }
            }

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
        // C4 · pending gate-repair correction. When the harness gate BLOCKs, the
        // recovery message is stashed here and injected on the next iteration so
        // the agent re-does the subtask (distinct from the text-only reflector).
        let mut pending_gate_correction: Option<String> = None;

        for reflector_attempt in 0..=MAX_REFLECTOR_RETRIES {
            let exec_result = if let Some(correction) = pending_gate_correction.take() {
                let augmented_desc = format!(
                    "{}\n\n## IMPORTANT CORRECTION\n\n{}",
                    augmented_description, correction
                );
                executor
                    .execute_subtask(&planned.title, &augmented_desc, exec_ctx, Some(agent_type))
                    .await
            } else if reflector_attempt == 0 {
                executor
                    .execute_subtask(
                        &planned.title,
                        &augmented_description,
                        exec_ctx,
                        Some(agent_type),
                    )
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
                            .execute_subtask(
                                &planned.title,
                                &augmented_desc,
                                exec_ctx,
                                Some(agent_type),
                            )
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

                    let (gated_content, gate_outcome) =
                        apply_harness_gate_hook(planned, exec_ctx, agent_result.content);
                    if let Some(mut outcome) = gate_outcome {
                        // P0 · reject deliverables citing fabricated evidence ids
                        // (may flip PASS→BLOCK + attach a correction) before the
                        // retry decision below.
                        self.enforce_evidence_existence(&mut outcome).await;
                        self.enforce_evidence_kinds(&mut outcome).await;
                        self.enforce_evidence_freshness(&mut outcome).await;
                        // C4 · gate BLOCK with retries left → feed the recovery
                        // correction back into the loop (re-do the subtask) instead
                        // of accepting the blocked result; defer transition until
                        // the gate settles (PASS, or BLOCK with no retries left).
                        if !outcome.gate_allowed
                            && reflector_attempt < MAX_REFLECTOR_RETRIES
                            && outcome.repair_correction.is_some()
                        {
                            tracing::info!(
                                "[TaskMode/Harness] Gate BLOCK on '{}' (attempt {}/{}), \
                                 feeding repair correction back to reflector",
                                planned.title,
                                reflector_attempt + 1,
                                MAX_REFLECTOR_RETRIES,
                            );
                            pending_gate_correction = outcome.repair_correction.clone();
                            last_result = Some(AgentResult {
                                content: gated_content,
                                ..agent_result
                            });
                            continue;
                        }
                        self.consume_gate_outcome(task_id, outcome).await;
                    }
                    let agent_result = AgentResult {
                        content: gated_content,
                        ..agent_result
                    };

                    if agent_result.content.contains("[NEEDS_USER_INPUT]") {
                        let prompt = agent_result
                            .content
                            .replace("[NEEDS_USER_INPUT]", "")
                            .trim()
                            .to_string();

                        if let Some(ref st) = db_subtask {
                            let _ =
                                subtasks::update_status(&*self.repo, st.id, SubtaskStatus::Waiting)
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
                                        return (final_result.content, final_result.token_usage);
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
        // Loop exhausted: run the gate once on the fallback content (no further
        // retry possible) and drive the transition on whatever it decides.
        let (out, gate_outcome) = apply_harness_gate_hook(planned, exec_ctx, fallback);
        if let Some(mut outcome) = gate_outcome {
            self.enforce_evidence_existence(&mut outcome).await;
            self.enforce_evidence_kinds(&mut outcome).await;
            self.enforce_evidence_freshness(&mut outcome).await;
            self.consume_gate_outcome(task_id, outcome).await;
        }
        (out, None)
    }

    /// Post-gate handling shared by both gate sites in [`Self::execute_single_subtask`].
    ///
    /// On PASS, record the stage's deliverable summary for cross-stage handoff.
    /// Then either (legacy / `graph_driven == false`) drive the per-subtask cursor
    /// transition, or (P2 方案 C / `graph_driven == true`) accumulate the flow
    /// outcome for the Executor-driven loop and leave transitions to the graph.
    async fn consume_gate_outcome(&mut self, task_id: Uuid, outcome: HarnessGateOutcome) {
        // G · observability: log every stage gate decision at the single chokepoint
        // both gate sites flow through. The graph-driven path only accumulates into
        // `stage_outcome_acc`, so without this its PASS/BLOCK decisions were invisible
        // in the logs (only the legacy `drive_stage_transition` path logged cursor
        // moves). Pure additive INFO — no behaviour change.
        tracing::info!(
            target: "harness::hook",
            task_id = %task_id,
            stage = %outcome.gated_stage.as_str(),
            gate = if outcome.gate_allowed { "PASS" } else { "BLOCK" },
            findings = outcome.findings_count,
            graph_driven = self.graph_driven,
            "gate decision"
        );
        if outcome.gate_allowed {
            if let Some(summary) = outcome.evidence_summary.clone() {
                self.harness_evidence
                    .insert(outcome.gated_stage.as_str().to_string(), summary);
            }
        }
        if self.graph_driven {
            let flow = crate::harness::operation_flow::StageFlowOutcome {
                gate_allowed: outcome.gate_allowed,
                made_progress: outcome.findings_count > 0,
            };
            self.stage_outcome_acc = Some(match self.stage_outcome_acc.take() {
                Some(prev) => crate::harness::operation_flow::StageFlowOutcome {
                    gate_allowed: prev.gate_allowed && flow.gate_allowed,
                    made_progress: prev.made_progress || flow.made_progress,
                },
                None => flow,
            });
        } else {
            self.drive_stage_transition(task_id, outcome).await;
        }
    }

    /// P2 方案 C · Executor-driven run loop (flag-gated, opt-in).
    ///
    /// The metalcraft `Executor` owns the top-level loop, driving the operation
    /// stage graph; a `ChannelStageRunner` turns each stage node into a request
    /// this method services with `&mut self` (running that stage's subtask group
    /// via `execute_single_subtask` with `graph_driven` on). Conditional bail,
    /// interrupt, and DB checkpoint come from the graph + executor. `run()` only
    /// takes this path when `operation_flow::graph_flow_enabled()`; otherwise the
    /// legacy `execute_subtask_loop` runs unchanged (rollback = flag off).
    pub(crate) async fn run_executor_driven(
        &mut self,
        task_id: Uuid,
        queue: &[PlannedSubtask],
        executor: &dyn AgentExecutor,
    ) -> anyhow::Result<String> {
        use crate::harness::operation_flow::{
            build_runner_graph, ChannelStageRunner, OperationFlowState, StageRunRequest,
        };

        let (op_max_authz, op_profile_id) =
            match crate::db_shim::operation_state::get(&*self.repo, task_id).await {
                Ok(Some(state)) => match crate::harness::load_embedded_profile(&state.profile) {
                    Ok(Some(p)) => (Some(p.max_authorization), Some(state.profile)),
                    _ => (None, None),
                },
                _ => (None, None),
            };

        let task_input = crate::db_shim::tasks::get(&*self.repo, task_id)
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
            harness_profile_id: op_profile_id.clone(),
        };

        let groups: std::collections::HashMap<crate::harness::StageKind, Vec<usize>> =
            crate::task_orchestrator::stage_execution::group_subtasks_by_stage(queue)
                .into_iter()
                .collect();

        let profile_id = op_profile_id.as_deref().unwrap_or("assessment");
        let dag = match (
            crate::harness::load_embedded_profile(profile_id),
            crate::harness::base_operation_graph(),
        ) {
            (Ok(Some(p)), Ok(g)) => {
                // S0 · observability: which profile drove this run + which stages
                // the DAG was projected to + per-stage planner subtask counts.
                // Previously nothing logged the profile/projection, so a run that
                // skipped scoping/target_intel (because the planner produced no
                // subtask for them → vacuous pass) was invisible.
                let allowed = p.allowed_stage_set();
                tracing::info!(
                    target: "harness::hook",
                    profile = %profile_id,
                    ?allowed,
                    "graph-flow: profile/DAG projected (DAG-driven execution)"
                );
                for (stage, idxs) in &groups {
                    tracing::info!(
                        target: "harness::hook",
                        stage = %stage.as_str(),
                        planner_subtasks = idxs.len(),
                        in_dag = allowed.contains(stage),
                        "graph-flow: stage→planner-subtask mapping"
                    );
                }
                g.project(&allowed)
            }
            _ => {
                tracing::warn!(target: "harness::hook", "graph-flow: profile/DAG load failed; falling back to legacy loop");
                return self
                    .execute_subtask_loop(task_id, &mut queue.to_vec(), 0, executor)
                    .await;
            }
        };

        // Two-level model: the Profile object (for phase-boundary approval policy).
        // Loaded once; only consulted when GOLISH_HARNESS_TWO_LEVEL is on.
        let op_profile = crate::harness::load_embedded_profile(profile_id)
            .ok()
            .flatten();

        let (tx, mut rx) = tokio::sync::mpsc::channel::<StageRunRequest>(8);
        let runner = std::sync::Arc::new(ChannelStageRunner::new(tx));
        let graph = match build_runner_graph(&dag, runner) {
            Ok(g) => g,
            Err(e) => {
                tracing::warn!(target: "harness::hook", error = %e, "graph-flow: build_runner_graph failed; falling back to legacy loop");
                return self
                    .execute_subtask_loop(task_id, &mut queue.to_vec(), 0, executor)
                    .await;
            }
        };
        let checkpointer = std::sync::Arc::new(
            crate::task_orchestrator::stage_execution::DbFlowCheckpointer::new(
                self.repo.clone(),
                task_id,
            ),
        );
        let executor_obj =
            crate::harness::graph_engine::Executor::new(graph).with_checkpointer(checkpointer);
        let thread = task_id.to_string();
        let mut exec_fut = Box::pin(executor_obj.run(OperationFlowState::default(), &thread));

        // Part 2 · per-stage roadmap (design 2026-06-04 · per-stage-plan-cards):
        // emit a `pending` seed plan for EVERY stage in the projected DAG up
        // front, in DAG node order, so the UI shows the full operation roadmap
        // (scoping → … → reporting) immediately — not-yet-run stages render as
        // greyed placeholders. When a stage actually runs, its stage-entry
        // `in_progress` seed (and then the agent's real `update_plan`) supersede
        // its placeholder in the frontend's per-stage bucket. Version 0 marks a
        // seed; the frontend always lets a newer seed/real update replace it.
        for &stage in &dag.nodes {
            let seed_steps = vec![golish_core::plan::PlanStep {
                id: None,
                step: synthesize_stage_subtask(stage, &exec_ctx.task_input).title,
                status: StepStatus::Pending,
                failure_kind: None,
            }];
            self.emit(AiEvent::PlanUpdated {
                version: 0,
                summary: golish_core::plan::PlanSummary::from_steps(&seed_steps),
                steps: seed_steps,
                explanation: None,
                stage_id: Some(stage.as_str().to_string()),
            });
        }

        self.emit(AiEvent::TaskProgress {
            task_id: task_id.to_string(),
            status: "running".to_string(),
            message: "Executor-driven harness flow started".to_string(),
        });

        loop {
            tokio::select! {
                res = &mut exec_fut => {
                    match res {
                        Ok(outcome) => tracing::info!(target: "harness::hook", ?outcome, "graph-flow: executor finished"),
                        Err(e) => tracing::warn!(target: "harness::hook", error = %e, "graph-flow: executor errored"),
                    }
                    break;
                }
                Some(req) = rx.recv() => {
                    let indices = groups.get(&req.stage).cloned().unwrap_or_default();
                    tracing::info!(
                        target: "harness::hook",
                        stage = %req.stage.as_str(),
                        planner_subtasks = indices.len(),
                        "graph-flow: entering stage"
                    );
                    let mut outcome = self
                        .run_stage_subtasks(
                            req.stage, &indices, queue, &mut exec_ctx, op_max_authz, executor, task_id,
                        )
                        .await;
                    // Two-level model (flag on): hold for human approval before
                    // crossing a大阶段 boundary. Withheld → downgrade to blocked so
                    // the engine Interrupts at this stage (no cross-phase advance).
                    if !self
                        .two_level_phase_gate(task_id, req.stage, outcome, &dag, op_profile.as_ref())
                        .await
                    {
                        outcome = crate::harness::operation_flow::StageFlowOutcome::blocked();
                    }
                    // G · observability: the graph-driven path advanced the cursor
                    // silently; mirror the legacy path's "cursor advanced" log so a
                    // run's stage progression is visible end-to-end.
                    match crate::db_shim::operation_state::advance_stage(
                        &*self.repo, task_id, req.stage.as_str(),
                    )
                    .await
                    {
                        Ok(()) => tracing::info!(
                            target: "harness::hook",
                            task_id = %task_id,
                            stage = %req.stage.as_str(),
                            "graph-flow: operation_state cursor advanced past stage"
                        ),
                        Err(e) => tracing::warn!(
                            target: "harness::hook",
                            error = %e,
                            "graph-flow: advance_stage failed"
                        ),
                    }
                    let _ = req.reply.send(outcome);
                }
            }
        }

        let report = match executor.generate_report(&exec_ctx).await {
            Ok(r) => r.content,
            Err(e) => {
                tracing::warn!("graph-flow reporter failed, using summary: {}", e);
                exec_ctx.summary()
            }
        };
        crate::db_shim::tasks::set_result(
            &*self.repo,
            task_id,
            &report,
            crate::db_traits::TaskStatus::Finished,
        )
        .await?;
        self.emit_plan_update(queue, queue.len(), StepStatus::Completed);
        self.emit(AiEvent::TaskProgress {
            task_id: task_id.to_string(),
            status: "finished".to_string(),
            message: "Executor-driven harness flow complete".to_string(),
        });
        Ok(report)
    }

    /// P2 方案 C · run one stage's subtask group under the Executor.
    ///
    /// Sets `graph_driven` so `execute_single_subtask` accumulates the flow
    /// outcome (via `consume_gate_outcome`) instead of driving the cursor, then
    /// returns the merged [`StageFlowOutcome`] for the graph to route on.
    #[allow(clippy::too_many_arguments)]
    async fn run_stage_subtasks(
        &mut self,
        stage: crate::harness::StageKind,
        indices: &[usize],
        queue: &[PlannedSubtask],
        exec_ctx: &mut ExecutionContext,
        op_max_authz: Option<crate::harness::AuthorizationLevel>,
        executor: &dyn AgentExecutor,
        task_id: Uuid,
    ) -> crate::harness::operation_flow::StageFlowOutcome {
        self.graph_driven = true;
        self.stage_outcome_acc = None;

        // Agent-driven stage body (设计 2026-06-04 · D1=B / 阶段内 todo).
        //
        // Instead of asking the generator for JSON subtasks and looping them, run
        // ONE stage-scoped agentic loop: the depth-0 primary self-manages this
        // stage's todos via `update_plan`, dispatches `sub_agent_*` specialists per
        // item, and submits the StageDeliverable. `execute_single_subtask` already
        // injects the stage charter, runs the reflector + gate + retry loop, and
        // consumes the gate outcome into `stage_outcome_acc`; we just hand it one
        // stage-scoped unit of work whose description carries the stage-execution
        // directive. `indices` / `queue` are unused on this lazy path (the queue is
        // empty under graph-flow lazy-per-stage planning).
        let _ = (indices, queue);

        let mut planned = synthesize_stage_subtask(stage, &exec_ctx.task_input);
        // `synthesize_stage_subtask` already tags `harness_stage`; append the
        // agent-todo execution directive so the single loop self-plans + submits.
        planned.description = format!(
            "{}\n\n{}",
            planned.description,
            super::super::prompts::stage_execution_prompt(stage.as_str())
        );

        exec_ctx.harness_stage = Some(stage);
        exec_ctx.harness_authz = op_max_authz.map(|max_authorization| {
            let intent = crate::harness::IntentClassifier::with_default_keywords()
                .classify(&planned.description, stage);
            crate::harness::HarnessAuthz {
                max_authorization,
                intent,
            }
        });
        exec_ctx.current_subtask = Some(super::super::types::CurrentSubtask {
            title: planned.title.clone(),
            description: planned.description.clone(),
            agent: planned.agent.clone(),
        });

        // Per-stage plan card seed (design 2026-06-04 · per-stage-plan-cards):
        // emit a deterministic, stage-tagged plan the moment we enter the stage
        // so the UI shows a card for THIS stage immediately — even confirm-only
        // stages (scoping) that may never call `update_plan` themselves. The
        // agent's own stage-scoped `update_plan` calls (tagged with the same
        // stage) then refine/replace this seed in the frontend's per-stage
        // bucket. Version 0 is a sentinel below the PlanManager's monotonic
        // counter so a real update always supersedes the seed.
        let seed_steps = vec![golish_core::plan::PlanStep {
            id: None,
            step: planned.title.clone(),
            status: StepStatus::InProgress,
            failure_kind: None,
        }];
        self.emit(AiEvent::PlanUpdated {
            version: 0,
            summary: golish_core::plan::PlanSummary::from_steps(&seed_steps),
            steps: seed_steps,
            explanation: None,
            stage_id: Some(stage.as_str().to_string()),
        });

        self.emit(AiEvent::TaskProgress {
            task_id: task_id.to_string(),
            status: "running".to_string(),
            message: format!("Stage '{}' executing (agent-driven).", stage.as_str()),
        });

        let (result_text, _usage) = self
            .execute_single_subtask(&planned, exec_ctx, executor, &None, task_id)
            .await;

        self.emit(AiEvent::SubtaskCompleted {
            task_id: task_id.to_string(),
            subtask_id: String::new(),
            title: planned.title.clone(),
            result: truncate(&result_text, 500),
            stage_kind: Some(stage.as_str().to_string()),
        });
        exec_ctx
            .completed_results
            .push(super::super::types::SubtaskResult {
                title: planned.title.clone(),
                result: result_text,
                token_usage: None,
            });

        self.graph_driven = false;
        // S1 · a projected stage that produced no gated deliverable must not
        // vacuously PASS — default to BLOCK so the engine interrupts here instead
        // of advancing on nothing (the old vacuous `pass_with_progress` is exactly
        // what let scoping/target_intel slip through). After synthesis + the
        // fail-closed gate this is only a defensive fallback (acc is normally Some).
        self.stage_outcome_acc
            .take()
            .unwrap_or_else(crate::harness::operation_flow::StageFlowOutcome::blocked)
    }

    /// 两级模型 · graph-flow 路径的「跨大阶段审批」闸（设计 2026-06-03）。
    ///
    /// flag off / gate 没过 / 非跨界 / 无需审批 → 直接放行（返回 `true`）。在把某 stage
    /// 的 flow outcome 回给 metalcraft 引擎前调用：若这步推进会跨大阶段且需人工批准，
    /// 则发 `waiting_approval` 并**阻塞等用户回复**。`false`（未获批）时调用方把 outcome
    /// 降级为 `blocked`，使引擎在当前 stage Interrupt（暂停返工），不跨大阶段。
    async fn two_level_phase_gate(
        &mut self,
        task_id: Uuid,
        from_stage: crate::harness::StageKind,
        outcome: crate::harness::operation_flow::StageFlowOutcome,
        dag: &crate::harness::AllowedDag,
        profile: Option<&crate::harness::Profile>,
    ) -> bool {
        if !crate::harness::two_level_enabled() || !outcome.gate_allowed {
            return true;
        }
        let Some(profile) = profile else {
            return true;
        };
        // 引擎将走的下一 stage（与引擎条件边同源 branch_target 规则）。
        let Some(next) = crate::harness::operation_flow::branch_target(
            &dag.next_stages(from_stage),
            outcome.made_progress,
        ) else {
            return true; // 终点，无跨界
        };
        let pm = match crate::harness::load_embedded_phase_map() {
            Ok(pm) => pm,
            Err(_) => return true,
        };
        if !crate::harness::phase_flow::phase_crossing_requires_approval(
            &pm, from_stage, next, profile,
        ) {
            return true; // 同大阶段内推进 / 无需审批 → 放行
        }
        tracing::info!(
            target: "harness::hook",
            task_id = %task_id,
            from = ?from_stage,
            to = ?next,
            "two-level phase boundary holds for human approval"
        );
        self.emit(AiEvent::TaskProgress {
            task_id: task_id.to_string(),
            status: "waiting_approval".to_string(),
            message: format!(
                "Phase boundary {} → {} requires human approval. Reply to approve \
                 (yes / approve / 继续) or anything else to hold.",
                from_stage.as_str(),
                next.as_str()
            ),
        });
        let reply = match self.user_input_rx.as_mut() {
            Some(rx) => rx.recv().await,
            None => None,
        };
        let approved = reply
            .as_deref()
            .map(approval_reply_is_affirmative)
            .unwrap_or(false);
        if approved {
            self.emit(AiEvent::TaskProgress {
                task_id: task_id.to_string(),
                status: "running".to_string(),
                message: format!("Approval granted; entering phase via {}.", next.as_str()),
            });
        } else {
            self.emit(AiEvent::TaskProgress {
                task_id: task_id.to_string(),
                status: "waiting_approval".to_string(),
                message: format!("Approval not granted; holding at {}.", from_stage.as_str()),
            });
        }
        approved
    }

    /// Phase 2/C: gate 通过后按 Operation DAG 推进 operation_state 游标 (Doc 3 §6.2).
    ///
    /// `operation_id == task_id` (一个 Task = 一个 operation). 读 `operation_state`
    /// 拿**真实 profile + 当前 stage** (不再硬编码 assessment), 投影 DAG, 用
    /// [`crate::harness::decide_transition`] 选下一 stage. C5: 若下一 stage 需人工
    /// 批准则 hold 并发 `waiting_approval` 事件, 不自动推进. Hold / Complete (无下
    /// 一格) 同样不写.
    async fn drive_stage_transition(&mut self, operation_id: Uuid, outcome: HarnessGateOutcome) {
        let state = match crate::db_shim::operation_state::get(&*self.repo, operation_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::warn!(target: "harness::hook", operation_id = %operation_id, "no operation_state row; skip transition");
                return;
            }
            Err(e) => {
                tracing::warn!(target: "harness::hook", error = %e, "operation_state get failed");
                return;
            }
        };
        let profile = match crate::harness::load_embedded_profile(&state.profile) {
            Ok(Some(p)) => p,
            Ok(None) => {
                tracing::warn!(target: "harness::hook", profile = %state.profile, "unknown profile in operation_state; skip transition");
                return;
            }
            Err(e) => {
                tracing::warn!(target: "harness::hook", error = %e, "profile load failed in transition");
                return;
            }
        };
        let graph = match crate::harness::base_operation_graph() {
            Ok(g) => g,
            Err(e) => {
                tracing::warn!(target: "harness::hook", error = %e, "base operation graph load failed");
                return;
            }
        };
        let dag = graph.project(&profile.allowed_stage_set());
        let decision =
            crate::harness::decide_transition(outcome.gated_stage, outcome.gate_allowed, &dag);
        // P2 (graph flow) · pick the next stage. With the metalcraft graph-flow
        // flag ON, a branch routes by progress (no findings → bail to reporting)
        // instead of always taking the first candidate; flag OFF preserves the
        // legacy `advance_target` (first-candidate) behaviour exactly.
        let Some(next) = crate::harness::operation_flow::chosen_next_stage(
            &decision,
            outcome.findings_count > 0,
            crate::harness::operation_flow::graph_flow_enabled(),
        ) else {
            tracing::info!(
                target: "harness::hook",
                operation_id = %operation_id,
                gated_stage = ?outcome.gated_stage,
                gate_allowed = outcome.gate_allowed,
                decision = ?decision,
                "no stage advance (hold/complete)"
            );
            return;
        };

        // C5 · approval 闸: 下一 stage 需人工批准 + profile policy 打开 → hold,
        // 发 waiting_approval 事件并**阻塞等用户回复** (user_input_rx). 肯定回复才
        // 推进游标 (resume); 否定 / 通道关闭 / 无交互通道时保持 hold 不推进.
        // Two-level model (flag on): approval fires only when crossing a大阶段
        // boundary (de-dup per crossing); legacy (flag off): per-stage approval.
        let needs_approval = if crate::harness::two_level_enabled() {
            crate::harness::load_embedded_phase_map()
                .map(|pm| {
                    crate::harness::phase_flow::phase_crossing_requires_approval(
                        &pm,
                        outcome.gated_stage,
                        next,
                        &profile,
                    )
                })
                .unwrap_or(false)
        } else {
            crate::harness::load_embedded_stage_spec(next)
                .map(|next_spec| {
                    crate::harness::stage_entry_requires_approval(&next_spec, &profile)
                })
                .unwrap_or(false)
        };
        {
            if needs_approval {
                tracing::info!(
                    target: "harness::hook",
                    operation_id = %operation_id,
                    from = ?outcome.gated_stage,
                    to = ?next,
                    "transition holds for human approval"
                );
                self.emit(AiEvent::TaskProgress {
                    task_id: operation_id.to_string(),
                    status: "waiting_approval".to_string(),
                    message: format!(
                        "Stage {} → {} requires human approval. Reply to approve \
                         (yes / approve / 继续) or anything else to hold.",
                        outcome.gated_stage.as_str(),
                        next.as_str()
                    ),
                });

                // Scope the rx borrow to just the recv so we can emit afterwards.
                let reply = match self.user_input_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => None,
                };
                let approved = reply
                    .as_deref()
                    .map(approval_reply_is_affirmative)
                    .unwrap_or(false);
                if approved {
                    self.emit(AiEvent::TaskProgress {
                        task_id: operation_id.to_string(),
                        status: "running".to_string(),
                        message: format!("Approval granted; advancing to {}.", next.as_str()),
                    });
                } else {
                    tracing::info!(
                        target: "harness::hook",
                        operation_id = %operation_id,
                        to = ?next,
                        "approval not granted; cursor held"
                    );
                    self.emit(AiEvent::TaskProgress {
                        task_id: operation_id.to_string(),
                        status: "waiting_approval".to_string(),
                        message: format!(
                            "Approval not granted; staying at {}.",
                            outcome.gated_stage.as_str()
                        ),
                    });
                    return;
                }
            }
        }

        match crate::db_shim::operation_state::advance_stage(
            &*self.repo,
            operation_id,
            next.as_str(),
        )
        .await
        {
            Ok(()) => tracing::info!(
                target: "harness::hook",
                operation_id = %operation_id,
                from = ?outcome.gated_stage,
                to = ?next,
                "operation_state cursor advanced"
            ),
            Err(e) => tracing::warn!(target: "harness::hook", error = %e, "advance_stage failed"),
        }

        // P1 · roll the stage_run + resume checkpoint forward: close the prior
        // stage's run, open one for `next`, and rewrite state_blob so a process
        // kill resumes at `next`. Best-effort (warn-free): failures don't block
        // the transition the cursor already made.
        let prior: crate::task_orchestrator::harness_resume::HarnessResumeState =
            serde_json::from_value(state.state_blob.clone()).unwrap_or_default();
        if let Some(prev_run) = prior.current_stage_run_id {
            let _ =
                crate::db_shim::stage_runs::mark_terminal(&*self.repo, prev_run, "completed").await;
        }
        let new_run = Uuid::new_v4();
        let _ =
            crate::db_shim::stage_runs::insert(&*self.repo, new_run, operation_id, next.as_str())
                .await;
        let rs = crate::task_orchestrator::harness_resume::HarnessResumeState {
            current_stage: next.as_str().to_string(),
            current_stage_run_id: Some(new_run),
            completed_count: prior.completed_count + 1,
            ..prior
        };
        let _ = crate::db_shim::operation_state::write_state_blob(
            &*self.repo,
            operation_id,
            serde_json::to_value(&rs).unwrap_or_default(),
        )
        .await;
    }

    /// P0 · gate evidence 回查: 把交付物里引用、但 ledger 中**不存在**的 evidence
    /// id 当作伪造 → 翻 BLOCK + 追加纠正喂回 reflector. infra 查询失败只 warn,
    /// 不误伤合法 stage (放行), 避免 DB 抖动卡死流程.
    async fn enforce_evidence_existence(&self, outcome: &mut HarnessGateOutcome) {
        if outcome.evidence_refs.is_empty() {
            return;
        }
        // Scoping is an L0 authorization-confirmation stage ("L0-L1 only, no
        // probing"): its scope claim is backed by the authorization framework,
        // not a tool run, so there are no real evidence-ledger ids to cite.
        // Exempt it from the fabricated-evidence cross-check — otherwise scoping
        // can never pass (no scan tool ⇒ no ledger evidence ⇒ infinite BLOCK).
        if outcome.gated_stage == crate::harness::StageKind::Scoping {
            tracing::debug!(
                target: "harness::hook",
                stage = %outcome.gated_stage.as_str(),
                "evidence-existence check skipped for scoping (authz-confirmation stage)"
            );
            return;
        }
        let existing = match self
            .repo
            .evidence_existing_ids(&outcome.evidence_refs)
            .await
        {
            Ok(set) => set,
            Err(e) => {
                tracing::warn!(
                    target: "harness::hook",
                    error = %e,
                    "evidence existence check failed; not blocking on infra error"
                );
                return;
            }
        };
        let fabricated = fabricated_evidence_ids(&outcome.evidence_refs, &existing);
        if fabricated.is_empty() {
            return;
        }
        tracing::warn!(
            target: "harness::hook",
            stage = %outcome.gated_stage.as_str(),
            fabricated = ?fabricated,
            "gate BLOCK: deliverable cites evidence ids absent from the ledger"
        );
        block_outcome_for_fabricated(outcome, &fabricated);
    }

    /// P2 · evidence-kind 回查: stage spec 声明的 `required_evidence_kinds` 必须真的
    /// 出现在交付物引用的证据里 (查 ledger 的 `detail->>'kind'`). 缺 → BLOCK + 纠正。
    /// infra 查询失败只 warn, 不误伤合法 stage。
    async fn enforce_evidence_kinds(&self, outcome: &mut HarnessGateOutcome) {
        if outcome.required_evidence_kinds.is_empty() {
            return;
        }
        let present: std::collections::HashSet<String> =
            match self.repo.evidence_kinds_for(&outcome.evidence_refs).await {
                Ok(map) => map.into_values().collect(),
                Err(e) => {
                    tracing::warn!(
                        target: "harness::hook",
                        error = %e,
                        "evidence kind check failed; not blocking on infra error"
                    );
                    return;
                }
            };
        let missing: Vec<String> = outcome
            .required_evidence_kinds
            .iter()
            .filter(|k| !present.contains(*k))
            .cloned()
            .collect();
        if missing.is_empty() {
            return;
        }
        tracing::warn!(
            target: "harness::hook",
            stage = %outcome.gated_stage.as_str(),
            missing = ?missing,
            "gate BLOCK: stage requires evidence kinds absent from the deliverable"
        );
        outcome.gate_allowed = false;
        let correction = format!(
            "This stage requires evidence of kinds {missing:?}, but the deliverable's evidence \
             includes none of them. Run the tools that produce these evidence kinds and resubmit \
             a StageDeliverable that cites them."
        );
        outcome.repair_correction = Some(match outcome.repair_correction.take() {
            Some(prev) => format!("{correction}\n\n{prev}"),
            None => correction,
        });
    }

    /// P0 Task 6 · evidence「新鲜度」回查 (flag-gated, 默认 OFF): 查 ledger 真实 age,
    /// 按 `evidence_kinds.json` max_age 拦截**硬过期**证据 (age ≥ 2×max → BLOCK; 软
    /// 陈旧只 warn)。infra 查询失败只 warn 不误伤; flag 关时整段跳过 (零行为变化)。
    async fn enforce_evidence_freshness(&self, outcome: &mut HarnessGateOutcome) {
        if !crate::harness::evidence_freshness_enforcement_enabled()
            || outcome.evidence_refs.is_empty()
        {
            return;
        }
        let kinds_raw = match self.repo.evidence_kinds_for(&outcome.evidence_refs).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(target: "harness::hook", error = %e, "evidence freshness: kind lookup failed; not blocking");
                return;
            }
        };
        let ages_raw = match self.repo.evidence_ages_for(&outcome.evidence_refs).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(target: "harness::hook", error = %e, "evidence freshness: age lookup failed; not blocking");
                return;
            }
        };
        type Eid = golish_pentest::evidence_ledger::EvidenceAuditId;
        let kinds: std::collections::HashMap<Eid, String> = kinds_raw
            .into_iter()
            .map(|(id, k)| (Eid::new(id), k))
            .collect();
        let ages: std::collections::HashMap<Eid, std::time::Duration> = ages_raw
            .into_iter()
            .map(|(id, d)| (Eid::new(id), d))
            .collect();
        let ids: Vec<Eid> = outcome.evidence_refs.iter().map(|&i| Eid::new(i)).collect();
        let (expired, stale) = crate::harness::freshness_age_reasons(&ids, &kinds, &ages);
        if !stale.is_empty() {
            tracing::warn!(
                target: "harness::hook",
                stage = %outcome.gated_stage.as_str(),
                stale = ?stale,
                "evidence freshness: stale evidence (soft, not blocking)"
            );
        }
        if expired.is_empty() {
            return;
        }
        tracing::warn!(
            target: "harness::hook",
            stage = %outcome.gated_stage.as_str(),
            expired = ?expired,
            "gate BLOCK: deliverable cites hard-expired evidence"
        );
        outcome.gate_allowed = false;
        let correction = format!(
            "Some cited evidence is hard-expired (older than 2x its max age): {expired:?}. \
             Re-run the relevant tools so the evidence is fresh, then resubmit a StageDeliverable \
             citing the new evidence ids."
        );
        outcome.repair_correction = Some(match outcome.repair_correction.take() {
            Some(prev) => format!("{correction}\n\n{prev}"),
            None => correction,
        });
    }
}

/// Pure core of the evidence-existence gate: the cited ids absent from the
/// ledger (`existing`). Non-empty ⇒ the deliverable fabricated evidence refs ⇒
/// the gate must BLOCK. Order follows `cited` for stable messages.
fn fabricated_evidence_ids(cited: &[i64], existing: &std::collections::HashSet<i64>) -> Vec<i64> {
    cited
        .iter()
        .copied()
        .filter(|id| !existing.contains(id))
        .collect()
}

/// Pure: flip a gate outcome to BLOCK with an anti-fabrication correction,
/// preserving any prior correction by prepending the fabrication notice.
fn block_outcome_for_fabricated(outcome: &mut HarnessGateOutcome, fabricated: &[i64]) {
    outcome.gate_allowed = false;
    let correction = format!(
        "Your StageDeliverable cites evidence ids {fabricated:?} that do NOT exist in the \
         evidence ledger. You may only reference evidence produced by real tool runs in this \
         operation. Re-run the required tools so their output is recorded, then resubmit a \
         StageDeliverable whose evidence_refs are all real ledger ids."
    );
    outcome.repair_correction = Some(match outcome.repair_correction.take() {
        Some(prev) => format!("{correction}\n\n{prev}"),
        None => correction,
    });
}

// ── Harness gate hook (Phase C · Doc 3 §5.2 接入点) ─────────────────────────
//
// 仅当满足以下全部条件时, agent_result.content 末尾才会被追加 gate decision JSON:
//   1. `harness::stage_mode_enabled()` 返回 true (默认 on; 显式 =false 才关)
//   2. `planned.harness_stage` 非 None
//   3. agent_result.content 含可解析的 StageDeliverable JSON
//      (整体即 JSON, 或 ```json fence 内的 JSON)
//
// Phase C: 支持**任意 stage** —— 按 `stage_hint.stage_kind` 从嵌入 registry 载对应
// StageSpec, 跑通用 gate (`validate_stage_gate`). 任一条件不满足时返回原 content
// (不破坏旧路径行为).
//
// **execute_single_subtask 2 元组返回签名保持不变**: gate decision 文本化嵌入
// content 末尾兼容; 第二元素 `Option<HarnessGateOutcome>` 驱动 stage 流转.

/// gate hook 跑完后回传给流转驱动的最小信息 (Doc 3 §6.2).
struct HarnessGateOutcome {
    gated_stage: crate::harness::StageKind,
    gate_allowed: bool,
    /// C4 · when the gate BLOCKs, a correction message (reasons + recovery
    /// actions) the caller can feed back into the reflector retry loop so the
    /// agent re-does the subtask and resubmits a fixed deliverable. `None` when
    /// the gate passed.
    repair_correction: Option<String>,
    /// C6 · compact summary of the parsed deliverable (claims/findings/evidence
    /// counts). Recorded into the orchestrator's handoff store on PASS so
    /// downstream inheriting stages get the real upstream results. `None` when no
    /// deliverable was parsed.
    evidence_summary: Option<String>,
    /// P0 · the deliverable's cited evidence ids (as `audit_log.id`) so the
    /// caller can verify each exists in the ledger (anti-fabrication) before
    /// honoring a PASS. Empty when no deliverable / no refs.
    evidence_refs: Vec<i64>,
    /// P2 · evidence kinds this stage requires (from
    /// `StageSpec.required_evidence_kinds`); the caller cross-checks them against
    /// the ledger before honoring a PASS. Empty = stage declares no requirement.
    required_evidence_kinds: Vec<String>,
    /// P2 (graph flow) · how many findings the deliverable carried. Used as the
    /// "made progress" signal for conditional branch routing: a stage that
    /// passes its gate but surfaces NO findings can bail to reporting instead of
    /// descending into enumeration/triage (see `operation_flow::chosen_next_stage`).
    findings_count: usize,
}

/// 返回 `(content, Option<outcome>)`: `None` 表示 hook 透传 (未跑 gate); `Some` 表示
/// 跑了 gate, 调用方据此驱动 stage 流转 (推进 operation_state 游标).
fn apply_harness_gate_hook(
    planned: &PlannedSubtask,
    exec_ctx: &ExecutionContext,
    content: String,
) -> (String, Option<HarnessGateOutcome>) {
    if !crate::harness::stage_mode_enabled() {
        return (content, None);
    }
    let Some(stage_hint) = planned.harness_stage.as_ref() else {
        // Observability (2026-06-01): previously DEBUG, so a stage-less subtask
        // produced ZERO signal at default log level — the gate "silently
        // skipped" and the DAG cursor never moved without any trace. INFO so
        // `golish=info` runs can see that this subtask carried no stage.
        tracing::info!(
            target: "harness::hook",
            subtask_title = %planned.title,
            "harness gate skipped: subtask has no harness_stage (not tagged by Generator \
             or keyword backfill) — no gate / no cursor advance for it"
        );
        return (content, None);
    };

    tracing::info!(
        target: "harness::hook",
        stage_kind = ?stage_hint.stage_kind,
        subtask_title = %planned.title,
        content_len = content.len(),
        "harness gate hook entered"
    );

    // C1 · construct StageHarness with the operation's real profile (threaded via
    // exec_ctx from operation_state), falling back to "assessment" when absent.
    // gate validation only reads stage_spec; the profile id also keeps logs and
    // future profile-sensitive checks honest.
    let profile_id = exec_ctx
        .harness_profile_id
        .as_deref()
        .unwrap_or("assessment");
    let profile = match crate::harness::load_embedded_profile(profile_id) {
        Ok(Some(p)) => p,
        _ => {
            tracing::warn!(target: "harness::hook", profile_id = %profile_id, "[harness] failed to load operation profile");
            return (content, None);
        }
    };
    // 按 stage_hint.stage_kind 从嵌入 registry 载对应 StageSpec (支持任意 stage).
    let harness = match crate::harness::StageHarness::for_stage_embedded(
        stage_hint.stage_kind,
        profile,
    ) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(target: "harness::hook", error = %e, "[harness] StageHarness::for_stage_embedded failed");
            return (content, None);
        }
    };

    // C2d · per-target gate. When sprint-skeleton enforcement is enabled, attach
    // the profile's per-stage skeleton so the gate also checks expected
    // finding-count ranges + min tool invocations (per-target, not just
    // structural). Flag-gated (default OFF) because it can newly BLOCK runs whose
    // finding kinds/counts don't match the skeleton; no-op when the profile ships
    // no skeleton or the stage has no skeleton entry.
    let harness = if crate::harness::sprint_skeleton_enforcement_enabled() {
        match crate::harness::load_embedded_sprint_skeleton(profile_id) {
            Ok(Some(skeleton)) => match skeleton.for_stage(stage_hint.stage_kind) {
                Some(stage_skel) => {
                    tracing::info!(
                        target: "harness::hook",
                        stage_kind = ?stage_hint.stage_kind,
                        profile_id = %profile_id,
                        "sprint-skeleton enforcement ON: gate will check per-target finding ranges"
                    );
                    harness.with_skeleton(Some(stage_skel.clone()))
                }
                None => harness,
            },
            _ => harness,
        }
    } else {
        harness
    };

    let deliverable = match parse_deliverable_from_content(&content) {
        Some(d) => d,
        None => {
            // D2 hybrid (设计 2026-06-04): a confirm-only stage (empty
            // `allowed_tool_types`, e.g. scoping / reporting) has a deliverable
            // derivable from known operation state, so synthesize a minimal,
            // gate-passing one instead of dead-locking when a weak model returns
            // no parseable StageDeliverable (the `content_len=0` deadlock).
            // Substantive scan / finding-producing stages KEEP the fail-closed
            // BLOCK — their findings must never be fabricated.
            let confirm_only = crate::harness::load_embedded_stage_spec(stage_hint.stage_kind)
                .map(|s| s.allowed_tool_types.is_empty())
                .unwrap_or(false);
            if confirm_only {
                tracing::warn!(
                    target: "harness::hook",
                    stage_kind = ?stage_hint.stage_kind,
                    subtask_title = %planned.title,
                    content_len = content.len(),
                    "confirm-only stage produced no parseable StageDeliverable — synthesizing a minimal one (D2 fallback, no deadlock)"
                );
                synthesize_confirm_only_deliverable(stage_hint.stage_kind, exec_ctx)
            } else {
                // Only reachable for a stage-TAGGED substantive subtask, so this is
                // a genuine contract miss (agent didn't end with a ```json
                // StageDeliverable). S4: fail-closed — BLOCK + repair correction so
                // the reflector retry loop pushes the agent to actually submit one.
                tracing::warn!(
                    target: "harness::hook",
                    stage_kind = ?stage_hint.stage_kind,
                    subtask_title = %planned.title,
                    content_len = content.len(),
                    "harness gate: stage-tagged subtask produced no parseable StageDeliverable JSON block — BLOCK (fail-closed)"
                );
                return (
                    content,
                    missing_deliverable_gate_outcome(stage_hint.stage_kind),
                );
            }
        }
    };

    tracing::info!(
        target: "harness::hook",
        stage_id = %deliverable.stage_id,
        stage_run_id = %deliverable.stage_run_id,
        claims = deliverable.claims.len(),
        findings = deliverable.findings.len(),
        skipped_checks = deliverable.skipped_checks.len(),
        evidence_refs = deliverable.evidence_refs.len(),
        "deliverable parsed, running gate validation"
    );

    let decision = harness.validate_gate(&deliverable, None);

    // P2-c · doer eval: score this deliverable's quality (gate outcome +
    // evidence backing + finding verification) and log it for ranking doer runs.
    let scorecard = crate::harness::eval::score_deliverable(
        &deliverable,
        &decision,
        &crate::harness::eval::default_scorers(),
    );
    tracing::info!(
        target: "harness::eval",
        stage_id = %deliverable.stage_id,
        doer_score = scorecard.overall,
        "doer quality scorecard computed"
    );

    if decision.allowed {
        tracing::info!(
            target: "harness::hook",
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            allowed = true,
            reasons_count = decision.reasons.len(),
            "gate decision: PASS"
        );
    } else {
        let recovery = decision.recovery_actions.as_ref();
        tracing::warn!(
            target: "harness::hook",
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            allowed = false,
            reasons_count = decision.reasons.len(),
            first_reason = decision.reasons.first().map(|s| s.as_str()).unwrap_or("<none>"),
            recovery_hints = recovery.map(|r| r.hints.len()).unwrap_or(0),
            recovery_repair_tool_calls = recovery.map(|r| r.repair_tool_calls.len()).unwrap_or(0),
            recovery_missing_evidence_kinds = recovery.map(|r| r.missing_evidence_kinds.len()).unwrap_or(0),
            "gate decision: BLOCK"
        );
    }

    // C4 · on BLOCK, render a correction the caller can feed back into the
    // reflector retry loop (gate→repair). `None` on PASS.
    let repair_correction = if decision.allowed {
        None
    } else {
        Some(build_gate_correction(&decision))
    };

    let decision_json = serde_json::to_string_pretty(&decision)
        .unwrap_or_else(|_| "{\"error\":\"failed to serialize gate decision\"}".to_string());

    let mut out = content;
    out.push_str("\n\n## Harness Gate Decision\n\n```json\n");
    out.push_str(&decision_json);
    out.push_str("\n```\n");
    (
        out,
        Some(HarnessGateOutcome {
            gated_stage: stage_hint.stage_kind,
            gate_allowed: decision.allowed,
            repair_correction,
            evidence_summary: Some(summarize_deliverable(&deliverable)),
            evidence_refs: deliverable
                .evidence_refs
                .iter()
                .map(|e| e.as_i64())
                .collect(),
            required_evidence_kinds: crate::harness::load_embedded_stage_spec(
                stage_hint.stage_kind,
            )
            .map(|s| s.required_evidence_kinds)
            .unwrap_or_default(),
            findings_count: deliverable.findings.len(),
        }),
    )
}

/// C6 · render a compact, bounded summary of a gate-passed deliverable for the
/// cross-stage handoff store. Lists the first few claims + findings (kind +
/// subject) and the evidence-ref count so a downstream stage sees what upstream
/// actually produced without re-reading the full deliverable JSON.
fn summarize_deliverable(d: &crate::harness::StageDeliverable) -> String {
    const MAX_ITEMS: usize = 6;
    let mut s = String::new();
    if !d.claims.is_empty() {
        s.push_str("- claims: ");
        let parts: Vec<String> = d
            .claims
            .iter()
            .take(MAX_ITEMS)
            .map(|c| format!("{} ({})", c.kind, c.subject))
            .collect();
        s.push_str(&parts.join("; "));
        if d.claims.len() > MAX_ITEMS {
            s.push_str(&format!(" … (+{} more)", d.claims.len() - MAX_ITEMS));
        }
        s.push('\n');
    }
    if !d.findings.is_empty() {
        s.push_str("- findings: ");
        let parts: Vec<String> = d
            .findings
            .iter()
            .take(MAX_ITEMS)
            .map(|f| format!("{} ({})", f.kind, f.subject))
            .collect();
        s.push_str(&parts.join("; "));
        if d.findings.len() > MAX_ITEMS {
            s.push_str(&format!(" … (+{} more)", d.findings.len() - MAX_ITEMS));
        }
        s.push('\n');
    }
    s.push_str(&format!("- evidence refs: {}", d.evidence_refs.len()));
    s
}

/// C4 · render a harness gate BLOCK decision into a correction message for the
/// reflector retry loop: the agent re-does the subtask addressing the gate's
/// reasons + recovery actions and resubmits a fixed `StageDeliverable`.
fn build_gate_correction(decision: &crate::harness::GateResult) -> String {
    let mut s = String::from(
        "Your stage deliverable was REJECTED by the deterministic harness gate. \
         Fix the issues below and resubmit a corrected StageDeliverable — either by \
         calling the submit_stage_deliverable tool, or by ending your next message \
         with a corrected ```json StageDeliverable block.\n\n\
         ### Gate rejection reasons\n",
    );
    if decision.reasons.is_empty() {
        s.push_str("- (no specific reason reported)\n");
    } else {
        for r in &decision.reasons {
            s.push_str(&format!("- {}\n", r));
        }
    }
    if let Some(rec) = decision.recovery_actions.as_ref() {
        if !rec.repair_tool_calls.is_empty() {
            s.push_str("\n### Required tool calls (run these, then re-collect evidence)\n");
            for t in &rec.repair_tool_calls {
                s.push_str(&format!("- {}\n", t));
            }
        }
        if !rec.missing_evidence_kinds.is_empty() {
            s.push_str("\n### Missing evidence to collect\n");
            for k in &rec.missing_evidence_kinds {
                s.push_str(&format!("- {}\n", k));
            }
        }
        if !rec.hints.is_empty() {
            s.push_str("\n### Hints\n");
            for h in &rec.hints {
                s.push_str(&format!("- {}\n", h));
            }
        }
    }
    s
}

/// C5 · whether a user's approval reply grants the held stage transition.
/// Affirmative on common yes-tokens (en + zh); anything else holds the cursor.
fn approval_reply_is_affirmative(reply: &str) -> bool {
    let r = reply.trim().to_lowercase();
    if r.is_empty() {
        return false;
    }
    const YES: &[&str] = &[
        "approve", "approved", "approval", "yes", "ok", "okay", "proceed", "continue", "confirm",
        "批准", "同意", "通过", "继续", "确认", "可以",
    ];
    if r == "y" || r == "go" {
        return true;
    }
    YES.iter().any(|t| r.contains(t))
}

/// 把 content 解析为 [`crate::harness::StageDeliverable`].
///
/// 两条路径: ① content 整体 (trim 后) 即 JSON; ② content 含 ```json ... ``` fence
/// 时抽取 fence 内 JSON. 都失败返 None → hook skip (不强制 block).
fn parse_deliverable_from_content(content: &str) -> Option<crate::harness::StageDeliverable> {
    if let Ok(d) = serde_json::from_str::<crate::harness::StageDeliverable>(content.trim()) {
        return Some(d);
    }
    if let Some(start) = content.find("```json") {
        let after = &content[start + "```json".len()..];
        if let Some(end) = after.find("```") {
            if let Ok(d) =
                serde_json::from_str::<crate::harness::StageDeliverable>(after[..end].trim())
            {
                return Some(d);
            }
        }
    }
    None
}

/// D2 fallback (设计 2026-06-04): build a minimal, gate-passing `StageDeliverable`
/// for a **confirm-only** stage (empty `allowed_tool_types`, e.g. scoping /
/// reporting) when the agent produced none. Such stages run no scan tools, so
/// there is no tool evidence to fabricate — their deliverable is a single
/// confirmation claim derivable from the operation's known scope/target. A scoping
/// claim with empty `evidence_ids` passes the gate (scope_check treats scoping
/// evidence as optional; vacuous_check is satisfied by the one claim; the stage's
/// `min_invocations` is empty so FakePattern does not fire). This kills the
/// `content_len=0` deadlock without ever inventing findings for a scanning stage.
fn synthesize_confirm_only_deliverable(
    stage: crate::harness::StageKind,
    exec_ctx: &ExecutionContext,
) -> crate::harness::StageDeliverable {
    let subject = {
        let t = exec_ctx.task_input.trim();
        if t.is_empty() {
            "operation scope".to_string()
        } else {
            t.to_string()
        }
    };
    let kind = if matches!(stage, crate::harness::StageKind::Scoping) {
        "scope_confirmed".to_string()
    } else {
        format!("{}_completed", stage.as_str())
    };
    crate::harness::StageDeliverable {
        stage_id: stage.as_str().to_string(),
        stage_run_id: uuid::Uuid::new_v4(),
        claims: vec![crate::harness::StageClaim {
            kind,
            subject,
            summary: "Backend-synthesized confirmation for a confirm-only stage \
                      (no scan tools); the agent submitted no parseable StageDeliverable."
                .to_string(),
            evidence_ids: vec![],
        }],
        evidence_refs: vec![],
        skipped_checks: vec![],
        findings: vec![],
        required_checks_done: vec![],
    }
}

/// S4 · the gate outcome when a stage-tagged subtask ends without a parseable
/// `StageDeliverable`: a BLOCK + repair correction so the reflector retry loop
/// pushes the agent to actually submit one (fail-closed). Replaces the old
/// fail-open skip that let the cursor advance on plain narration.
fn missing_deliverable_gate_outcome(
    stage: crate::harness::StageKind,
) -> Option<HarnessGateOutcome> {
    let correction = format!(
        "Your output for the '{}' stage did not include a parseable StageDeliverable, \
         so the deterministic harness gate could not run. You MUST submit a StageDeliverable \
         — either by calling the submit_stage_deliverable tool, or by ending your next message \
         with a ```json fenced block containing a StageDeliverable (stage_id, stage_run_id, \
         claims, findings, evidence_refs). Re-do the stage work as needed and resubmit.",
        stage.as_str()
    );
    Some(HarnessGateOutcome {
        gated_stage: stage,
        gate_allowed: false,
        repair_correction: Some(correction),
        evidence_summary: None,
        evidence_refs: Vec::new(),
        required_evidence_kinds: Vec::new(),
        findings_count: 0,
    })
}

/// S2 (DAG-strict) · synthesize one stage-scoped [`PlannedSubtask`] for a stage
/// the planner produced no subtask for, so the stage actually executes + gets
/// gated instead of being vacuously passed. The description is a per-stage
/// charter scoped to the operation target (`task_input`); `harness_stage` is set
/// so the gate hook runs against this stage.
fn synthesize_stage_subtask(stage: crate::harness::StageKind, task_input: &str) -> PlannedSubtask {
    use crate::harness::StageKind as K;
    let target = task_input.trim();
    let (title, description, agent): (&str, String, &str) = match stage {
        K::Scoping => (
            "Scope & Authorization Confirmation",
            format!(
                "Confirm and document the engagement scope for `{target}`: in-scope \
                 targets/domains/IPs, explicit out-of-scope items, the authorization basis, \
                 and rules of engagement. Do NOT perform any active scanning in this stage."
            ),
            "pentester",
        ),
        K::TargetIntel => (
            "Passive Target Intelligence",
            format!(
                "Gather passive intelligence on `{target}` (WHOIS, ASN/netblocks, registrant, \
                 public DNS records, org footprint) without touching the target. Summarize what \
                 is known before any active recon."
            ),
            "pentester",
        ),
        K::ExternalAttackSurface => (
            "External Attack Surface Mapping",
            format!(
                "Map the external attack surface of `{target}`: passive subdomain enumeration, \
                 DNS resolution, and exposed hosts. Passive / low-touch only."
            ),
            "pentester",
        ),
        K::Enumeration => (
            "Service Enumeration",
            format!(
                "Enumerate services on the in-scope hosts of `{target}`: port scan, \
                 service/version fingerprinting, and surface the key endpoints."
            ),
            "pentester",
        ),
        K::VulnTriage => (
            "Vulnerability Triage",
            format!(
                "Triage likely vulnerabilities across the enumerated surface of `{target}`, \
                 prioritizing by severity and exploitability."
            ),
            "pentester",
        ),
        K::Verification => (
            "Finding Verification",
            format!(
                "Verify the highest-priority findings for `{target}` with controlled, authorized \
                 checks and record proof."
            ),
            "pentester",
        ),
        K::Reporting => (
            "Final Report Compilation",
            format!(
                "Compile the final report for `{target}`: scope, methodology, findings with \
                 evidence, and remediation guidance."
            ),
            "analyzer",
        ),
        other => (
            "Stage Execution",
            format!(
                "Execute the `{}` stage for `{target}` per the harness stage charter, then submit \
                 a StageDeliverable.",
                other.as_str()
            ),
            "pentester",
        ),
    };
    PlannedSubtask {
        title: title.to_string(),
        description,
        agent: Some(agent.to_string()),
        harness_stage: Some(crate::harness::HarnessStageHint::new(stage)),
        nl_slice: None,
        acceptance_criteria: Vec::new(),
    }
}

#[cfg(test)]
mod dag_driven_helper_tests {
    use super::*;
    use crate::harness::StageKind;

    #[test]
    fn missing_deliverable_outcome_blocks_with_correction() {
        let o = missing_deliverable_gate_outcome(StageKind::Enumeration)
            .expect("missing deliverable must produce a BLOCK outcome");
        assert!(
            !o.gate_allowed,
            "missing-deliverable must BLOCK (fail-closed)"
        );
        assert_eq!(o.gated_stage, StageKind::Enumeration);
        assert!(
            o.repair_correction.is_some(),
            "BLOCK must carry a repair correction to drive the reflector retry"
        );
    }

    #[test]
    fn synthesized_subtask_is_stage_tagged_and_targeted() {
        let s = synthesize_stage_subtask(StageKind::Scoping, "example.com");
        assert_eq!(
            s.harness_stage.as_ref().map(|h| h.stage_kind),
            Some(StageKind::Scoping),
            "synthesized subtask must be tagged with its stage so the gate runs"
        );
        assert!(
            s.description.contains("example.com"),
            "synthesized description must be scoped to the target"
        );
        assert!(s.agent.is_some());
        // Reporting routes to the analyzer specialist, not the pentester.
        let r = synthesize_stage_subtask(StageKind::Reporting, "example.com");
        assert_eq!(r.agent.as_deref(), Some("analyzer"));
    }
}

#[cfg(test)]
#[path = "execute_harness_loop_tests.rs"]
mod execute_harness_loop_tests;

#[cfg(test)]
mod harness_gate_hook_tests {
    use super::*;
    use crate::harness::{HarnessStageHint, StageKind};

    fn planned_with_harness(stage_kind: StageKind) -> PlannedSubtask {
        PlannedSubtask {
            title: "t".to_string(),
            description: "d".to_string(),
            agent: None,
            harness_stage: Some(HarnessStageHint::new(stage_kind)),
            nl_slice: None,
            acceptance_criteria: vec![],
        }
    }

    fn planned_no_harness() -> PlannedSubtask {
        PlannedSubtask {
            title: "t".to_string(),
            description: "d".to_string(),
            agent: None,
            harness_stage: None,
            nl_slice: None,
            acceptance_criteria: vec![],
        }
    }

    #[test]
    fn hook_passes_through_unparseable_content() {
        // content 不是可解析的 StageDeliverable → hook 透传原文.
        // (flag off 时早退透传; flag on 时解析失败也透传 — 两种默认都成立.)
        let p = planned_with_harness(StageKind::ExternalAttackSurface);
        let ctx = ExecutionContext::default();
        let content = "anything".to_string();
        assert_eq!(
            apply_harness_gate_hook(&p, &ctx, content.clone()).0,
            content
        );
    }

    #[test]
    fn no_harness_stage_skips_gate() {
        let p = planned_no_harness();
        let ctx = ExecutionContext::default();
        let content = "ignore me".to_string();
        assert_eq!(
            apply_harness_gate_hook(&p, &ctx, content.clone()).0,
            content
        );
    }

    #[test]
    fn embedded_registry_loads_external_stage_and_assessment_profile() {
        // 替代旧的 const include_str! 测试: 经嵌入 registry 加载.
        assert!(crate::harness::load_embedded_profile("assessment")
            .unwrap()
            .is_some());
        assert!(crate::harness::load_embedded_stage_spec(StageKind::ExternalAttackSurface).is_ok());
    }

    #[test]
    fn parse_deliverable_returns_none_on_non_json_content() {
        assert!(parse_deliverable_from_content("not json").is_none());
        assert!(parse_deliverable_from_content("# markdown header\n\nsome text").is_none());
    }

    #[test]
    fn build_gate_correction_includes_reasons_and_recovery() {
        // C4 · the correction fed back to the reflector must surface the gate's
        // reasons + required tool calls + missing evidence so the agent can fix it.
        let decision = crate::harness::GateResult::block(
            vec!["scope_status missing".to_string()],
            crate::harness::HarnessRecoveryActions {
                repair_tool_calls: vec!["dns_resolve".to_string()],
                missing_evidence_kinds: vec!["subdomain".to_string()],
                ..Default::default()
            },
        );
        let c = build_gate_correction(&decision);
        assert!(c.contains("REJECTED"));
        assert!(c.contains("scope_status missing"));
        assert!(c.contains("dns_resolve"));
        assert!(c.contains("subdomain"));
    }

    #[test]
    fn stage_inherited_evidence_renders_for_inheriting_stage_and_empty_otherwise() {
        // C6 · enumeration inherits from external_attack_surface; scoping inherits
        // from nothing → empty section.
        let enumeration = crate::harness::load_embedded_stage_spec(StageKind::Enumeration).unwrap();
        let rendered = crate::task_orchestrator::prompts::stage_inherited_evidence(&enumeration);
        assert!(rendered.contains("INHERITED EVIDENCE"));
        assert!(rendered.contains("external_attack_surface"));

        let scoping = crate::harness::load_embedded_stage_spec(StageKind::Scoping).unwrap();
        assert!(crate::task_orchestrator::prompts::stage_inherited_evidence(&scoping).is_empty());
    }

    #[test]
    fn approval_reply_affirmative_and_negative() {
        // C5 · only explicit yes-tokens (en + zh) grant the held transition.
        for yes in [
            "yes",
            "Y",
            "approve",
            "  Approved ",
            "继续",
            "ok proceed",
            "go",
        ] {
            assert!(approval_reply_is_affirmative(yes), "'{yes}' should approve");
        }
        for no in ["", "no", "wait", "hold on", "stop", "n"] {
            assert!(
                !approval_reply_is_affirmative(no),
                "'{no}' should NOT approve"
            );
        }
    }

    #[test]
    fn summarize_deliverable_lists_claims_findings_and_evidence() {
        // C6 · the handoff summary surfaces upstream claims/findings + ev count.
        use crate::harness::types::{FindingSeverity, HarnessFinding, StageClaim};
        use golish_pentest::evidence_ledger::EvidenceAuditId;
        let d = crate::harness::StageDeliverable {
            stage_id: "external_attack_surface".to_string(),
            stage_run_id: uuid::Uuid::new_v4(),
            claims: vec![StageClaim {
                kind: "http_service_observed".to_string(),
                subject: "api.example.com".to_string(),
                summary: "200 OK".to_string(),
                evidence_ids: vec![EvidenceAuditId::new(1)],
            }],
            evidence_refs: vec![EvidenceAuditId::new(1), EvidenceAuditId::new(2)],
            skipped_checks: vec![],
            findings: vec![HarnessFinding {
                finding_id: uuid::Uuid::new_v4(),
                kind: "subdomain".to_string(),
                subject: "api.example.com".to_string(),
                severity: FindingSeverity::Info,
                evidence_refs: vec![EvidenceAuditId::new(1)],
            }],
            required_checks_done: vec![],
        };
        let s = summarize_deliverable(&d);
        assert!(s.contains("http_service_observed"));
        assert!(s.contains("subdomain"));
        assert!(s.contains("evidence refs: 2"));
    }

    #[test]
    fn render_inherited_handoff_injects_recorded_upstream() {
        // C6 · enumeration inherits from external_attack_surface; with a recorded
        // upstream summary the real-handoff section is emitted, else empty.
        let enumeration = crate::harness::load_embedded_stage_spec(StageKind::Enumeration).unwrap();
        let mut recorded = std::collections::HashMap::new();
        assert!(
            crate::task_orchestrator::prompts::render_inherited_handoff(&enumeration, &recorded)
                .is_empty(),
            "no recorded upstream → empty"
        );
        recorded.insert(
            "external_attack_surface".to_string(),
            "- findings: subdomain (api.example.com)".to_string(),
        );
        let rendered =
            crate::task_orchestrator::prompts::render_inherited_handoff(&enumeration, &recorded);
        assert!(rendered.contains("ACTUAL UPSTREAM RESULTS"));
        assert!(rendered.contains("external_attack_surface"));
        assert!(rendered.contains("subdomain (api.example.com)"));
    }

    #[test]
    fn fabricated_evidence_ids_flags_only_absent_refs() {
        let existing: std::collections::HashSet<i64> = [1, 2, 3].into_iter().collect();
        // all cited exist → nothing fabricated
        assert!(fabricated_evidence_ids(&[1, 2, 3], &existing).is_empty());
        // mix: only the absent ids are flagged, in cited order
        assert_eq!(
            fabricated_evidence_ids(&[1, 999, 2, 42], &existing),
            vec![999, 42]
        );
        // all absent
        assert_eq!(fabricated_evidence_ids(&[7, 8], &existing), vec![7, 8]);
        // empty cited → empty
        assert!(fabricated_evidence_ids(&[], &existing).is_empty());
    }

    #[test]
    fn block_outcome_for_fabricated_flips_pass_to_block() {
        // A PASS deliverable citing a fabricated id must flip to BLOCK with a
        // correction naming the fabricated ids (plan acceptance #2: fake refs
        // get BLOCKed).
        let mut o = HarnessGateOutcome {
            gated_stage: StageKind::ExternalAttackSurface,
            gate_allowed: true,
            repair_correction: None,
            evidence_summary: None,
            evidence_refs: vec![1, 999],
            required_evidence_kinds: Vec::new(),
            findings_count: 0,
        };
        block_outcome_for_fabricated(&mut o, &[999]);
        assert!(!o.gate_allowed, "fabricated evidence must BLOCK the gate");
        let c = o.repair_correction.expect("correction set on block");
        assert!(c.contains("999"), "correction names the fabricated id");
        assert!(c.contains("do NOT exist"));
    }

    #[test]
    fn block_outcome_for_fabricated_prepends_to_existing_correction() {
        let mut o = HarnessGateOutcome {
            gated_stage: StageKind::ExternalAttackSurface,
            gate_allowed: false,
            repair_correction: Some("PRIOR-GATE-REASON".to_string()),
            evidence_summary: None,
            evidence_refs: vec![5],
            required_evidence_kinds: Vec::new(),
            findings_count: 0,
        };
        block_outcome_for_fabricated(&mut o, &[5]);
        let c = o.repair_correction.unwrap();
        assert!(
            c.contains("PRIOR-GATE-REASON"),
            "must keep prior correction"
        );
        let fab_pos = c.find("do NOT exist").expect("fabrication notice present");
        let prior_pos = c.find("PRIOR-GATE-REASON").unwrap();
        assert!(fab_pos < prior_pos, "fabrication notice is prepended");
    }
}
