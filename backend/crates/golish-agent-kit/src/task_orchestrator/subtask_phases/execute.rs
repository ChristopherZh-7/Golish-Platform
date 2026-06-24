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

/// Outcome of asking the human to approve a 大阶段 crossing
/// ([`TaskOrchestrator::request_phase_approval`]).
enum PhaseApproval {
    /// Human approved — proceed across the boundary.
    Approved,
    /// Human declined. `Some(note)` carries a reviewer note to feed back to the
    /// agent as a rework directive; `None` means "just hold" (no note / Skip /
    /// timeout / no interactive channel).
    Declined(Option<String>),
}

/// What the servicer loop should do at a phase boundary
/// ([`TaskOrchestrator::two_level_phase_gate`]).
enum PhaseGateDecision {
    /// No approval needed, or the human approved — let the advance proceed.
    Allowed,
    /// Human held the crossing with no rework note — block (engine interrupts).
    Held,
    /// Human held the crossing and asked the agent to rework THIS stage first,
    /// using the carried reviewer note.
    Rework(String),
}

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

            // C2 · harness stage charter (该 subtask 归属某 stage 时, 前置注入
            // 允许/禁止工具面 + deliverable/gate 要求).
            {
                if let Some(hint) = planned.harness_stage.as_ref() {
                    if let Ok(spec) = crate::harness::load_embedded_stage_spec(hint.stage_kind) {
                        let scoping_policy = scoping_policy_for_ctx(exec_ctx);
                        desc.push_str(&super::super::prompts::stage_charter(
                            &spec,
                            &scoping_policy,
                        ));
                        // 阶段级方法论 playbook（设计 2026-06-11）：charter 之后注入
                        // 「这个阶段怎么高效做」的正向指导（推荐工具序列 / 效率红线 /
                        // 何时收口），补 charter 只讲约束、不讲方法论的缺口。没写
                        // playbook 的阶段返回空串。
                        //
                        // 例外（设计 2026-06-15）：有 `specialist` 的阶段（如 target_intel
                        // → recon）由 stage_run 把 specialist 按 org 扇出；真正干活 + 提交 +
                        // 过 gate 的是那个 worker 子 agent。recon「怎么做」的方法论因此注入
                        // 给 worker（见 stage_run 的 build_org_objective）。主 agent 这里只拿
                        // 一份精简编排提示（扇出 + gap 循环 + 收口），不再重复脏活方法论。
                        // 无 specialist 的阶段（主 agent 自己干）照旧注入完整 playbook。
                        if spec
                            .specialist
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .is_none()
                        {
                            desc.push_str(&super::super::prompts::stage_methodology(&spec));
                        } else {
                            desc.push_str(&super::super::prompts::stage_specialist_orchestration(
                                &spec,
                            ));
                        }
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
                        // Phase 2 ①③ seam (agent-facing): inject the authoritative
                        // in-scope asset list (recon-populated targets.scope='in',
                        // narrowed to the operation's org when bound) so the stage
                        // agent works the real assets. Empty (no recon yet) → no
                        // section, no behavior change.
                        let in_scope_assets = self
                            .repo
                            .in_scope_assets(self.harness_org_id)
                            .await
                            .unwrap_or_default();
                        // 设计 2026-06-13: scoping 是 ORG 层、不是 ASSET 层 → 不注入
                        // in-scope 资产（防上一轮/别 org 残留污染纠名）；其余阶段照旧。
                        desc.push_str(&super::super::prompts::render_in_scope_assets_for_stage(
                            hint.stage_kind,
                            &in_scope_assets,
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
        // 设计 2026-06-11 · when the BLOCK was "work done, only the submission is
        // missing" (targeted repair), the retry pass locks its tool_choice to
        // `submit_stage_deliverable` so a weak model can't drift into redoing the
        // stage — it can only submit.
        let mut pending_submit_only = false;

        for reflector_attempt in 0..=MAX_REFLECTOR_RETRIES {
            let exec_result = if let Some(correction) = pending_gate_correction.take() {
                let submit_only_pass = std::mem::take(&mut pending_submit_only);
                let augmented_desc = format!(
                    "{}\n\n## IMPORTANT CORRECTION\n\n{}",
                    augmented_description, correction
                );
                let mut attempt_ctx = exec_ctx.clone();
                attempt_ctx.harness_submit_only = submit_only_pass;
                executor
                    .execute_subtask(
                        &planned.title,
                        &augmented_desc,
                        &attempt_ctx,
                        Some(agent_type),
                    )
                    .await
            } else {
                // attempt 0，或上一轮没产生 pending 纠正（不可达于 BLOCK/text-only
                // 路径——两者都会填 pending_gate_correction）：按原任务描述执行。
                // 设计 2026-06-12-unified-refiner (PR-R4)：旧的 `executor.reflect()`
                // LLM 反思分支被删除——text-only 响应改由确定性 F 类模板纠正。
                executor
                    .execute_subtask(
                        &planned.title,
                        &augmented_description,
                        exec_ctx,
                        Some(agent_type),
                    )
                    .await
            };

            match exec_result {
                Ok(agent_result) => {
                    if reflector_attempt < MAX_REFLECTOR_RETRIES
                        && looks_like_text_only_response(&agent_result.content)
                    {
                        // 设计 2026-06-12-unified-refiner (PR-R4) · F 类：确定性
                        // 模板取代旧 LLM reflect()——直接灌下一轮纠正。
                        let decision =
                            crate::task_orchestrator::refiner::refine_text_only(&planned.title);
                        tracing::info!(
                            target: "harness::hook",
                            class = ?decision.class,
                            "[TaskMode/Refiner] Subtask '{}' returned text-only response ({} chars), \
                             feeding deterministic correction (attempt {})",
                            planned.title,
                            agent_result.content.len(),
                            reflector_attempt + 1,
                        );
                        pending_gate_correction = Some(decision.correction);
                        last_result = Some(agent_result);
                        continue;
                    }

                    let in_scope_assets = self.fetch_in_scope_assets_for_gate(planned).await;
                    let asset_types = self.fetch_in_scope_typed_assets_for_gate(planned).await;
                    let in_scope_target_types =
                        self.fetch_in_scope_target_types_for_gate(planned).await;
                    let evidence_facts = self
                        .fetch_evidence_facts_for_gate(planned, in_scope_assets.as_deref(), task_id)
                        .await;
                    let source_queries = self.fetch_source_queries_for_gate(planned).await;
                    // 设计 2026-06-12-unified-refiner · Refiner C 类诊断与 gate 用
                    // 同一份证据事实；hook move 走原值，这里留一份给渲染。
                    let refine_facts = evidence_facts.clone();
                    // Phase 1.5: fan-out 阶段收尾改判 stage_run pass_token（B-recompute），
                    // 跳过整阶段 coverage；非 fan-out / 不可解析交付物走常规 gate。
                    let mut specialist_gated = false;
                    let (gated_content, gate_outcome) = if let Some(res) = self
                        .try_specialist_stage_gate(planned, &agent_result.content)
                        .await
                    {
                        specialist_gated = true;
                        res
                    } else {
                        apply_harness_gate_hook(
                            planned,
                            exec_ctx,
                            agent_result.content,
                            in_scope_assets,
                            asset_types,
                            in_scope_target_types,
                            evidence_facts,
                            source_queries,
                            self.harness_subsidiary_policy.map(|p| p.threshold_pct),
                        )
                    };
                    if let Some(mut outcome) = gate_outcome {
                        // P0 · reject deliverables citing fabricated evidence ids
                        // (may flip PASS→BLOCK) before the retry decision below.
                        self.enforce_evidence_existence(&mut outcome).await;
                        self.enforce_evidence_kinds(&mut outcome).await;
                        self.enforce_evidence_freshness(&mut outcome).await;
                        // red_team scoping: verify the unit-candidate / org-creation
                        // flow actually ran (not just a claim) before allowing PASS.
                        self.enforce_scoping_red_team_flow(&mut outcome, exec_ctx)
                            .await;
                        // missing-deliverable 时补查账本真实 ids（A/B 类路由事实）。
                        self.gather_missing_deliverable_ids(&mut outcome).await;
                        // 设计 2026-06-12-unified-refiner · BLOCK 的全部事实汇入
                        // 唯一 Refiner：确定性分类 → 单模板纠正 → submit-only 锁。
                        if !outcome.gate_allowed {
                            // Phase 1.5: fan-out token BLOCK 不走 refiner——refiner 可能置
                            // submit_only_lock，会把主 agent 锁进「只能重交」而无法再调
                            // stage_run（死锁）。直接喂 gate 的「重跑 stage_run」事实、
                            // submit_only=false，让它能再扇出。
                            let (correction, submit_only) = if specialist_gated {
                                (outcome.gate_reasons.join("\n"), false)
                            } else {
                                let decision = crate::task_orchestrator::refiner::refine(
                                    &outcome.as_refine_input(refine_facts.as_deref()),
                                );
                                tracing::info!(
                                    target: "harness::hook",
                                    stage = %outcome.gated_stage.as_str(),
                                    class = ?decision.class,
                                    submit_only = decision.submit_only_lock,
                                    "refiner decision"
                                );
                                (decision.correction, decision.submit_only_lock)
                            };
                            outcome.repair_correction = Some(correction);
                            // C4 · gate BLOCK with retries left → feed the
                            // correction back into the loop; defer transition until
                            // the gate settles (PASS, or BLOCK with no retries left).
                            if reflector_attempt < MAX_REFLECTOR_RETRIES {
                                tracing::info!(
                                    "[TaskMode/Harness] Gate BLOCK on '{}' (attempt {}/{}), \
                                     feeding correction back (submit_only={})",
                                    planned.title,
                                    reflector_attempt + 1,
                                    MAX_REFLECTOR_RETRIES,
                                    submit_only,
                                );
                                pending_gate_correction = outcome.repair_correction.clone();
                                pending_submit_only = submit_only;
                                last_result = Some(AgentResult {
                                    content: gated_content,
                                    ..agent_result
                                });
                                continue;
                            }
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
        let in_scope_assets = self.fetch_in_scope_assets_for_gate(planned).await;
        let asset_types = self.fetch_in_scope_typed_assets_for_gate(planned).await;
        let in_scope_target_types = self.fetch_in_scope_target_types_for_gate(planned).await;
        let evidence_facts = self
            .fetch_evidence_facts_for_gate(planned, in_scope_assets.as_deref(), task_id)
            .await;
        let source_queries = self.fetch_source_queries_for_gate(planned).await;
        let refine_facts = evidence_facts.clone();
        // Phase 1.5: fan-out 阶段收尾改判 stage_run pass_token；非 fan-out 走常规 gate。
        let mut specialist_gated = false;
        let (out, gate_outcome) =
            if let Some(res) = self.try_specialist_stage_gate(planned, &fallback).await {
                specialist_gated = true;
                res
            } else {
                apply_harness_gate_hook(
                    planned,
                    exec_ctx,
                    fallback,
                    in_scope_assets,
                    asset_types,
                    in_scope_target_types,
                    evidence_facts,
                    source_queries,
                    self.harness_subsidiary_policy.map(|p| p.threshold_pct),
                )
            };
        if let Some(mut outcome) = gate_outcome {
            self.enforce_evidence_existence(&mut outcome).await;
            self.enforce_evidence_kinds(&mut outcome).await;
            self.enforce_evidence_freshness(&mut outcome).await;
            self.enforce_scoping_red_team_flow(&mut outcome, exec_ctx)
                .await;
            // No retry left here; gather the ledger facts + render the refiner
            // correction anyway so the HarnessTrace GateDecision carries the real
            // available ids and the final blocking reason.
            self.gather_missing_deliverable_ids(&mut outcome).await;
            if !outcome.gate_allowed {
                let correction = if specialist_gated {
                    outcome.gate_reasons.join("\n")
                } else {
                    let decision = crate::task_orchestrator::refiner::refine(
                        &outcome.as_refine_input(refine_facts.as_deref()),
                    );
                    tracing::info!(
                        target: "harness::hook",
                        stage = %outcome.gated_stage.as_str(),
                        class = ?decision.class,
                        submit_only = decision.submit_only_lock,
                        "refiner decision (retries exhausted — trace only)"
                    );
                    decision.correction
                };
                outcome.repair_correction = Some(correction);
            }
            self.consume_gate_outcome(task_id, outcome).await;
        }
        (out, None)
    }

    /// Post-gate handling for the Executor-driven stage loop, shared by both gate
    /// sites in [`Self::execute_single_subtask`].
    ///
    /// On PASS, record the stage's deliverable summary for cross-stage handoff,
    /// then accumulate the flow outcome (gate ANDed, progress ORed across the
    /// stage's subtasks) into [`Self::stage_outcome_acc`] for `run_stage_subtasks`
    /// to report to the graph (which owns the actual stage transition).
    async fn consume_gate_outcome(&mut self, task_id: Uuid, outcome: HarnessGateOutcome) {
        // G · observability: log every stage gate decision at the single chokepoint
        // both gate sites flow through (the loop only accumulates into
        // `stage_outcome_acc`, so without this its PASS/BLOCK decisions would be
        // invisible in the logs). Pure additive INFO — no behaviour change.
        tracing::info!(
            target: "harness::hook",
            task_id = %task_id,
            stage = %outcome.gated_stage.as_str(),
            gate = if outcome.gate_allowed { "PASS" } else { "BLOCK" },
            findings = outcome.findings_count,
            "gate decision"
        );
        // Observability (design 2026-06-05): the gate decision as a first-class
        // event so it lands in the transcript timeline next to the deliverable's
        // tool result (BLOCK was previously tracing-only → invisible to any AI
        // reconstructing the run). `agent_path = "main"`: the gate runs in the
        // orchestrator. `operation_id` = task id (the harness operation).
        let first_blocking_reason = if outcome.gate_allowed {
            None
        } else {
            outcome
                .repair_correction
                .as_deref()
                .map(|s| s.lines().next().unwrap_or(s).to_string())
        };
        self.emit(AiEvent::HarnessTrace {
            operation_id: task_id.to_string(),
            stage: outcome.gated_stage.as_str().to_string(),
            agent_path: "main".to_string(),
            trace: golish_core::events::HarnessTraceKind::GateDecision {
                gate: if outcome.gate_allowed {
                    "PASS"
                } else {
                    "BLOCK"
                }
                .to_string(),
                findings: outcome.findings_count as u32,
                fabricated_evidence_refs: outcome.fabricated_evidence_refs.clone(),
                available_real_ids: outcome.available_real_ids.clone(),
                first_blocking_reason,
            },
        });
        if outcome.gate_allowed {
            // Engagement-org isolation (设计 2026-06-15-engagement-org-isolation):
            // scoping confirmed the engagement's root org — bind it now + persist
            // to operation_state so every downstream stage's fan-out / in-scope
            // reads confine to its subtree (chat path; the CLI seed path bound it
            // up-front). Idempotent: re-binding the same id is harmless.
            if let Some(org) = outcome.engagement_org_id {
                self.harness_org_id = Some(org);
                let _ = crate::db_shim::operation_state::set_engagement_org(
                    &*self.repo,
                    task_id,
                    Some(org),
                )
                .await;
            }
            if let Some(summary) = outcome.evidence_summary.clone() {
                self.harness_evidence
                    .insert(outcome.gated_stage.as_str().to_string(), summary);
            }
            // Authoritative "stage passed its evidence gate" signal. The UI keys the
            // "Stage complete" milestone + per-stage card completion off THIS event
            // (not the `submit_stage_deliverable` preview, which is only structural),
            // so completion shows only after the deterministic evidence gate actually
            // accepts the stage. Reuses TaskProgress (message = stage id) to avoid a
            // new AiEvent variant + a long exhaustive-match churn.
            self.emit(AiEvent::TaskProgress {
                task_id: task_id.to_string(),
                status: "stage_passed".to_string(),
                message: outcome.gated_stage.as_str().to_string(),
            });
        }
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
    }

    /// P2 方案 C · Executor-driven run loop (stage_mode).
    ///
    /// The metalcraft `Executor` owns the top-level loop, driving the operation
    /// stage graph; a `ChannelStageRunner` turns each stage node into a request
    /// this method services with `&mut self` (running that stage's subtask group
    /// via `execute_single_subtask` with `graph_driven` on). Conditional bail,
    /// interrupt, and DB checkpoint come from the graph + executor. A profile/DAG
    /// load or graph-build failure errors the run (no legacy fallback — surface
    /// the failure instead of masking it).
    pub(crate) async fn run_executor_driven(
        &mut self,
        task_id: Uuid,
        queue: &[PlannedSubtask],
        executor: &dyn AgentExecutor,
        resume: bool,
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
        // Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): bind
        // this operation to its scoping-confirmed engagement root org. Prefer the
        // explicitly-set id (CLI seed path) and persist it to operation_state for
        // resume; otherwise recover the previously-persisted id (resume path).
        // `None` ⇒ no binding yet (legacy whole-DB axis; downstream fails open).
        self.harness_org_id = match self.harness_org_id {
            Some(id) => {
                let _ = crate::db_shim::operation_state::set_engagement_org(
                    &*self.repo,
                    task_id,
                    Some(id),
                )
                .await;
                Some(id)
            }
            None => crate::db_shim::operation_state::get(&*self.repo, task_id)
                .await
                .ok()
                .flatten()
                .and_then(|s| s.engagement_org_id),
        };
        let mut exec_ctx = ExecutionContext {
            operation_id: Some(task_id),
            completed_results: Vec::new(),
            task_input,
            current_subtask: None,
            planned_subtasks: Vec::new(),
            harness_stage: None,
            harness_authz: None,
            harness_profile_id: op_profile_id.clone(),
            harness_submit_only: false,
            harness_org_id: self.harness_org_id,
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
                //
                // 方案 2 · if a stage allowlist is set (headless single/range run),
                // intersect it so the executable DAG is just that slice; the
                // slice's terminal has no successors → run finishes it then stops.
                let mut allowed = p.allowed_stage_set();
                if let Some(ref allowlist) = self.stage_allowlist {
                    allowed = allowed.intersection(allowlist).copied().collect();
                    tracing::info!(
                        target: "harness::hook",
                        ?allowlist,
                        "graph-flow: stage allowlist active (headless slice run)"
                    );
                }
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
                tracing::error!(target: "harness::hook", "graph-flow: profile/DAG load failed");
                return Err(anyhow::anyhow!(
                    "harness graph-flow: profile/DAG load failed for task {task_id}"
                ));
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
                tracing::error!(target: "harness::hook", error = %e, "graph-flow: build_runner_graph failed");
                return Err(anyhow::anyhow!(
                    "harness graph-flow: build_runner_graph failed: {e}"
                ));
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
        // Fresh run starts at the DAG entry; resume continues from the persisted
        // checkpoint's `next_node` (Task 断线恢复 · L3). Both branches yield the
        // same future Output, boxed to one type so the select! loop can poll it.
        // `inject = None`: a plain resume re-enters the saved stage; steering text
        // is recorded separately (it is not a flow-routing FlowUpdate).
        let mut exec_fut: std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = crate::harness::graph_engine::Result<
                            crate::harness::graph_engine::RunOutcome<OperationFlowState>,
                        >,
                    > + Send
                    + '_,
            >,
        > = if resume {
            tracing::info!(
                target: "harness::hook",
                task_id = %task_id,
                "graph-flow: RESUMING operation from persisted checkpoint"
            );
            Box::pin(executor_obj.resume(&thread, None))
        } else {
            Box::pin(executor_obj.run(OperationFlowState::default(), &thread))
        };

        // Part 2 · per-stage roadmap (design 2026-06-04 · per-stage-plan-cards):
        // emit a `pending` seed plan for EVERY stage in the projected DAG up
        // front, in DAG node order, so the UI shows the full operation roadmap
        // (scoping → … → reporting) immediately — not-yet-run stages render as
        // greyed placeholders. When a stage actually runs, its stage-entry
        // `in_progress` seed (and then the agent's real `update_plan`) supersede
        // its placeholder in the frontend's per-stage bucket. Version 0 marks a
        // seed; the frontend always lets a newer seed/real update replace it.
        // The seed roadmap only renders each stage's title, which is policy-
        // independent — resolve the scoping policy once before the loop.
        let scoping_policy = scoping_policy_for_ctx(&exec_ctx);
        let intel_policy = intel_policy_for_ctx(&exec_ctx);
        for &stage in &dag.nodes {
            let seed_steps = vec![golish_core::plan::PlanStep {
                id: None,
                step: synthesize_stage_subtask(
                    stage,
                    &exec_ctx.task_input,
                    &scoping_policy,
                    &intel_policy,
                )
                .title,
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

        // The rx arm never breaks (it services stage requests); only the engine-
        // future arm breaks, carrying the run's terminal outcome out of the loop.
        let final_outcome = loop {
            tokio::select! {
                res = &mut exec_fut => {
                    match &res {
                        Ok(outcome) => tracing::info!(target: "harness::hook", ?outcome, "graph-flow: executor finished"),
                        Err(e) => tracing::warn!(target: "harness::hook", error = %e, "graph-flow: executor errored"),
                    }
                    break res;
                }
                Some(req) = rx.recv() => {
                    let indices = groups.get(&req.stage).cloned().unwrap_or_default();
                    tracing::info!(
                        target: "harness::hook",
                        stage = %req.stage.as_str(),
                        planner_subtasks = indices.len(),
                        "graph-flow: entering stage"
                    );
                    self.sync_operation_stage_on_entry(task_id, req.stage).await;
                    // Two-level model (flag on): run the stage, then hold for human
                    // approval before crossing a 大阶段 boundary. A decline that
                    // carries a reviewer note re-runs THIS stage with the note as a
                    // rework directive (bounded by MAX_HUMAN_REWORKS) so the agent
                    // backtracks per the human's reason; a bare hold (no note) →
                    // blocked so the engine Interrupts at this stage.
                    const MAX_HUMAN_REWORKS: u8 = 3;
                    let mut human_correction: Option<String> = None;
                    let mut human_reworks: u8 = 0;
                    let outcome = loop {
                        let outcome = self
                            .run_stage_subtasks(
                                req.stage, &indices, queue, &mut exec_ctx, op_max_authz,
                                executor, task_id, human_correction.take().as_deref(),
                            )
                            .await;
                        match self
                            .two_level_phase_gate(
                                task_id, req.stage, &outcome, &dag, op_profile.as_ref(),
                            )
                            .await
                        {
                            PhaseGateDecision::Allowed => break outcome,
                            PhaseGateDecision::Held => {
                                break crate::harness::operation_flow::StageFlowOutcome::blocked();
                            }
                            PhaseGateDecision::Rework(note) => {
                                if human_reworks >= MAX_HUMAN_REWORKS {
                                    self.emit(AiEvent::TaskProgress {
                                        task_id: task_id.to_string(),
                                        status: "waiting_approval".to_string(),
                                        message: format!(
                                            "Reached the human-rework limit ({}) at {}; holding.",
                                            MAX_HUMAN_REWORKS,
                                            req.stage.as_str()
                                        ),
                                    });
                                    break crate::harness::operation_flow::StageFlowOutcome::blocked(
                                    );
                                }
                                human_reworks += 1;
                                human_correction = Some(note);
                                // loop: re-run this stage with the reviewer's note
                            }
                        }
                    };
                    let _ = req.reply.send(outcome);
                }
            }
        };

        // L4a (Task 断线恢复): an engine Interrupt means the operation paused for
        // rework/approval and is RESUMABLE — it must NOT be marked Finished (that
        // previously made paused ops look complete and unresumable). Mark the task
        // `waiting` (paused) and return a short paused summary WITHOUT running the
        // reporter; the next user message resumes from the persisted checkpoint.
        if let Some(paused) = paused_disposition(&final_outcome) {
            if let Err(e) = crate::db_shim::tasks::update_status(
                &*self.repo,
                task_id,
                crate::db_traits::TaskStatus::Waiting,
            )
            .await
            {
                tracing::warn!(
                    target: "harness::hook",
                    error = %e,
                    "graph-flow: failed to mark task waiting on interrupt"
                );
            }
            self.emit(AiEvent::TaskProgress {
                task_id: task_id.to_string(),
                status: "waiting".to_string(),
                message: paused.clone(),
            });
            return Ok(paused);
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

    /// Refresh `exec_ctx.harness_org_id` from the orchestrator's currently-bound
    /// engagement org.
    ///
    /// `exec_ctx` is built ONCE at run start (`run_executor_driven`); on the chat
    /// path the engagement org is not known there yet — scoping binds it later via
    /// [`Self::consume_gate_outcome`]. Without re-syncing this snapshot at each
    /// stage entry it stays a stale `None`, and every post-scoping stage loses the
    /// org binding: `manage_targets` orphans discovered assets (`organization_id`
    /// NULL) and the submit gate skips its org-keyed DB-truth projection so every
    /// coverage cell reads "never attempted" and the stage dead-loops. Re-synced
    /// per stage so the bound org reaches that stage's tools + gate through the
    /// bridge side-channel.
    fn sync_engagement_org_into(&self, exec_ctx: &mut ExecutionContext) {
        exec_ctx.harness_org_id = self.harness_org_id;
    }

    /// Keep the operation-state cursor aligned with the stage currently being
    /// executed. `operation_state.advance_stage` also refreshes `stage_started_at`,
    /// so we only call it when the stage actually changes; a same-stage resume must
    /// preserve the original start time for freshness-window gates.
    async fn sync_operation_stage_on_entry(&self, task_id: Uuid, stage: crate::harness::StageKind) {
        let desired = stage.as_str();
        match crate::db_shim::operation_state::get(&*self.repo, task_id).await {
            Ok(Some(state)) if state.current_stage == desired => {
                tracing::debug!(
                    target: "harness::hook",
                    task_id = %task_id,
                    stage = %desired,
                    "graph-flow: operation_state cursor already at stage"
                );
            }
            Ok(Some(state)) => {
                match crate::db_shim::operation_state::advance_stage(&*self.repo, task_id, desired)
                    .await
                {
                    Ok(()) => tracing::info!(
                        target: "harness::hook",
                        task_id = %task_id,
                        previous_stage = %state.current_stage,
                        stage = %desired,
                        "graph-flow: operation_state cursor entered stage"
                    ),
                    Err(e) => tracing::warn!(
                        target: "harness::hook",
                        task_id = %task_id,
                        stage = %desired,
                        error = %e,
                        "graph-flow: operation_state stage-entry sync failed"
                    ),
                }
            }
            Ok(None) => tracing::warn!(
                target: "harness::hook",
                task_id = %task_id,
                stage = %desired,
                "graph-flow: operation_state missing during stage-entry sync"
            ),
            Err(e) => tracing::warn!(
                target: "harness::hook",
                task_id = %task_id,
                stage = %desired,
                error = %e,
                "graph-flow: operation_state stage-entry lookup failed"
            ),
        }
    }

    /// P2 方案 C · run one stage's subtask group under the Executor.
    ///
    /// `execute_single_subtask` accumulates the flow outcome (via
    /// `consume_gate_outcome`) into [`Self::stage_outcome_acc`]; this method reads
    /// it back and returns the merged [`StageFlowOutcome`] for the graph to route
    /// on (the graph owns the actual stage transition).
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
        human_correction: Option<&str>,
    ) -> crate::harness::operation_flow::StageFlowOutcome {
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

        let scoping_policy = scoping_policy_for_ctx(exec_ctx);
        let intel_policy = intel_policy_for_ctx(exec_ctx);
        let mut planned =
            synthesize_stage_subtask(stage, &exec_ctx.task_input, &scoping_policy, &intel_policy);
        // `synthesize_stage_subtask` already tags `harness_stage`; append the
        // agent-todo execution directive so the single loop self-plans + submits.
        planned.description = format!(
            "{}\n\n{}",
            planned.description,
            super::super::prompts::stage_execution_prompt(stage.as_str())
        );
        // Human-rejection rework (design 2026-06-05): a reviewer declined advancing
        // past this stage and supplied a note. Prepend it as a high-priority
        // directive so this re-run directly addresses the human's reason before the
        // phase transition is re-evaluated.
        if let Some(note) = human_correction {
            planned.description = format!(
                "## A human reviewer held this phase transition\n\
                 A human reviewer declined advancing past the **{}** stage and asked you to \
                 rework it first.\n\
                 Reviewer's note: \"{}\"\n\
                 Re-examine your work for this stage, directly address the reviewer's note, then \
                 submit an updated stage deliverable.\n\n{}",
                stage.as_str(),
                note,
                planned.description,
            );
        }

        exec_ctx.harness_stage = Some(stage);
        // Engagement-org isolation (设计 2026-06-15): `exec_ctx` was snapshotted at
        // run start before scoping bound the org; re-sync it here so this stage's
        // tools (manage_targets org backfill) + submit gate (org-keyed DB-truth
        // projection) see the bound org instead of a stale `None`.
        self.sync_engagement_org_into(exec_ctx);
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
    /// gate 没过 / 非跨界 / 无需审批 → 直接放行（返回 `true`）。在把某 stage 的 flow
    /// outcome 回给 metalcraft 引擎前调用：若这步推进会跨大阶段且需人工批准，
    /// 则发 `waiting_approval` 并**阻塞等用户回复**。`false`（未获批）时调用方把 outcome
    /// 降级为 `blocked`，使引擎在当前 stage Interrupt（暂停返工），不跨大阶段。
    async fn two_level_phase_gate(
        &mut self,
        task_id: Uuid,
        from_stage: crate::harness::StageKind,
        outcome: &crate::harness::operation_flow::StageFlowOutcome,
        dag: &crate::harness::AllowedDag,
        profile: Option<&crate::harness::Profile>,
    ) -> PhaseGateDecision {
        if !outcome.gate_allowed {
            return PhaseGateDecision::Allowed;
        }
        let Some(profile) = profile else {
            return PhaseGateDecision::Allowed;
        };
        // 引擎将走的下一 stage（与引擎条件边同源 branch_target 规则）。
        let Some(next) = crate::harness::operation_flow::branch_target(
            &dag.next_stages(from_stage),
            outcome.made_progress,
        ) else {
            return PhaseGateDecision::Allowed; // 终点，无跨界
        };
        let pm = match crate::harness::load_embedded_phase_map() {
            Ok(pm) => pm,
            Err(_) => return PhaseGateDecision::Allowed,
        };
        if !crate::harness::phase_flow::phase_crossing_requires_approval(
            &pm, from_stage, next, profile,
        ) {
            return PhaseGateDecision::Allowed; // 同大阶段内推进 / 无需审批 → 放行
        }
        tracing::info!(
            target: "harness::hook",
            task_id = %task_id,
            from = ?from_stage,
            to = ?next,
            "two-level phase boundary holds for human approval"
        );
        match self.request_phase_approval(task_id, from_stage, next).await {
            PhaseApproval::Approved => {
                self.emit(AiEvent::TaskProgress {
                    task_id: task_id.to_string(),
                    status: "running".to_string(),
                    message: format!("Approval granted; entering phase via {}.", next.as_str()),
                });
                PhaseGateDecision::Allowed
            }
            PhaseApproval::Declined(Some(note)) => {
                self.emit(AiEvent::TaskProgress {
                    task_id: task_id.to_string(),
                    status: "waiting_approval".to_string(),
                    message: format!(
                        "Held at {}; reworking this stage with your note before re-asking.",
                        from_stage.as_str()
                    ),
                });
                PhaseGateDecision::Rework(note)
            }
            PhaseApproval::Declined(None) => {
                self.emit(AiEvent::TaskProgress {
                    task_id: task_id.to_string(),
                    status: "waiting_approval".to_string(),
                    message: format!("Approval not granted; holding at {}.", from_stage.as_str()),
                });
                PhaseGateDecision::Held
            }
        }
    }

    /// Ask the human to approve crossing a 大阶段 boundary.
    ///
    /// **Preferred (interactive) path** — when a HITL coordinator is wired
    /// ([`TaskOrchestrator::set_approval_coordinator`]): emit an
    /// [`AiEvent::AskHumanRequest`] (`confirmation`) so the chat panel renders a
    /// Confirm/Skip card the user clicks **without** stopping the running task. On
    /// **Skip**, a second `freetext` card asks *why* — the note is returned as
    /// [`PhaseApproval::Declined`] so the caller can re-run this stage with it as a
    /// rework directive (the agent backtracks per the human's reason). The answers
    /// return over the same `respond_to_tool_approval` channel the `ask_human` tool
    /// uses; a 600s timeout (mirrors `ask_human`) keeps a forgotten card from
    /// wedging the run.
    ///
    /// **Fallback path** — no coordinator (e.g. unit tests): emit the legacy
    /// `waiting_approval` [`AiEvent::TaskProgress`] and block on `user_input_rx`,
    /// treating an affirmative reply ([`approval_reply_is_affirmative`]) as grant
    /// (no rework note on this path).
    async fn request_phase_approval(
        &mut self,
        task_id: Uuid,
        from_stage: crate::harness::StageKind,
        next: crate::harness::StageKind,
    ) -> PhaseApproval {
        const PHASE_APPROVAL_TIMEOUT_SECS: u64 = 600;
        if let Some(coordinator) = self.approval_coordinator.clone() {
            // Step 1 — Confirm/Skip the crossing.
            let request_id = Uuid::new_v4().to_string();
            let decision_rx = coordinator.register_approval(request_id.clone());
            self.emit(AiEvent::AskHumanRequest {
                request_id,
                question: format!(
                    "Approve entering the next phase (crossing {} → {})?",
                    from_stage.as_str(),
                    next.as_str()
                ),
                input_type: "confirmation".to_string(),
                options: Vec::new(),
                context: "Phase-boundary gate: Confirm to let the agent proceed, or \
                          Skip to hold — you'll then be asked what to rework."
                    .to_string(),
            });
            let approved = matches!(
                tokio::time::timeout(
                    std::time::Duration::from_secs(PHASE_APPROVAL_TIMEOUT_SECS),
                    decision_rx,
                )
                .await,
                Ok(Ok(decision)) if decision.approved
            );
            if approved {
                return PhaseApproval::Approved;
            }

            // Step 2 — declined: ask WHY so the agent can rework using the note.
            let reason_id = Uuid::new_v4().to_string();
            let reason_rx = coordinator.register_approval(reason_id.clone());
            self.emit(AiEvent::AskHumanRequest {
                request_id: reason_id,
                question: format!(
                    "You held the crossing out of {}. What should the agent reconsider \
                     or fix? It will rework this stage using your note.",
                    from_stage.as_str()
                ),
                input_type: "freetext".to_string(),
                options: Vec::new(),
                context: "Leave empty / Skip to just hold without reworking.".to_string(),
            });
            let note = match tokio::time::timeout(
                std::time::Duration::from_secs(PHASE_APPROVAL_TIMEOUT_SECS),
                reason_rx,
            )
            .await
            {
                Ok(Ok(decision)) if decision.approved => decision
                    .reason
                    .map(|r| r.trim().to_string())
                    .filter(|r| !r.is_empty()),
                _ => None,
            };
            return PhaseApproval::Declined(note);
        }

        // Fallback: legacy text channel (no interactive coordinator; unit tests).
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
        if reply
            .as_deref()
            .map(approval_reply_is_affirmative)
            .unwrap_or(false)
        {
            PhaseApproval::Approved
        } else {
            PhaseApproval::Declined(None)
        }
    }

    /// Phase 2 ①③ seam: fetch the authoritative in-scope asset set
    /// (recon-populated `targets.scope='in'`) for the harness coverage gate.
    /// Returns `None` when the subtask carries no harness stage, the DB has no
    /// in-scope assets, or the lookup errors — so `coverage_complete` keeps its
    /// self-reported fallback. An empty set must NEVER be injected (it would
    /// vacuously satisfy coverage), hence the explicit non-empty guard.
    async fn fetch_in_scope_assets_for_gate(
        &self,
        planned: &PlannedSubtask,
    ) -> Option<Vec<String>> {
        // Only stage-tagged subtasks run a gate; skip the DB hit otherwise.
        planned.harness_stage.as_ref()?;
        match self.repo.in_scope_assets(self.harness_org_id).await {
            Ok(v) if !v.is_empty() => {
                tracing::info!(
                    target: "harness::hook",
                    asset_count = v.len(),
                    org_id = ?self.harness_org_id,
                    "injecting authoritative in-scope assets into coverage gate"
                );
                Some(v)
            }
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(
                    target: "harness::hook",
                    error = %e,
                    "in-scope asset lookup failed; coverage gate falls back to self-reported"
                );
                None
            }
        }
    }

    /// 2c-1 (设计 2026-06-15-host-aware-coverage-2c §4.1): fetch authoritative
    /// `value -> targets.type` for the coverage gate's per-asset classification.
    /// `None` when the subtask carries no stage, the DB has no typed assets, or
    /// the lookup errors — `coverage_complete` then falls back to value inference
    /// (2a/2b), so this is purely additive (never relaxes the gate on failure).
    async fn fetch_in_scope_typed_assets_for_gate(
        &self,
        planned: &PlannedSubtask,
    ) -> Option<std::collections::HashMap<String, String>> {
        planned.harness_stage.as_ref()?;
        match self.repo.in_scope_typed_assets(self.harness_org_id).await {
            Ok(v) if !v.is_empty() => Some(v.into_iter().collect()),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(
                    target: "harness::hook",
                    error = %e,
                    "in-scope typed-asset lookup failed; coverage gate falls back to value-inferred class"
                );
                None
            }
        }
    }

    /// P3 ③ seam: fetch the distinct `targets.type` values of the in-scope assets
    /// so the coverage gate can derive **dynamic** expected techniques. Returns an
    /// empty vec when the subtask carries no stage, the DB has none, or the lookup
    /// errors — `gate_expected_techniques` then yields `None` and the gate keeps
    /// `spec.expected_techniques` (zero behavior change).
    async fn fetch_in_scope_target_types_for_gate(&self, planned: &PlannedSubtask) -> Vec<String> {
        if planned.harness_stage.is_none() {
            return vec![];
        }
        match self.repo.in_scope_target_types(self.harness_org_id).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    target: "harness::hook",
                    error = %e,
                    "in-scope target-type lookup failed; coverage gate uses static expected_techniques"
                );
                vec![]
            }
        }
    }

    /// Phase 1.5 · fan-out（specialist）阶段的**阶段收尾**不再跑整阶段 coverage gate（冗余——
    /// 每个 org 已在 Phase 1 各过各 per-org gate；且整库资产轴 org_id=None 会分母爆炸），改判
    /// stage_run 的 pass_token：收尾 gate 拿 per-org 完成账本**重算**令牌比对（B-recompute），
    /// 全 in-scope org 新鲜 PASS 且令牌对上才放行。返回 `None` = 非 fan-out 阶段 / 交付物不可
    /// 解析（交回常规 gate 处理：后者对缺交付物 fail-closed BLOCK）。
    async fn try_specialist_stage_gate(
        &self,
        planned: &PlannedSubtask,
        content: &str,
    ) -> Option<(String, Option<HarnessGateOutcome>)> {
        let stage = planned.harness_stage.as_ref()?.stage_kind;
        let is_fanout = crate::harness::load_embedded_stage_spec(stage)
            .map(|s| s.specialist.is_some())
            .unwrap_or(false);
        if !is_fanout {
            return None;
        }
        let deliverable = parse_deliverable_from_content(content)?;
        Some(
            self.verify_stage_run_pass_token(stage, content, &deliverable)
                .await,
        )
    }

    /// B-recompute 校验：核「全 in-scope org 都在 TTL 内 PASS」+「主 agent 带回的 pass_token
    /// == 收尾 gate 当场对账本重算的值」。令牌由 stage_run 确定性代码对 `org_stage_completions`
    /// 账本算出，agent 看不到也造不出 `passed_at` → 盖不了章。任一不满足 → BLOCK 并提示只重跑
    /// 缺口 org 的 stage_run（不绑 session：两路径 session 维度可能不一致，防伪靠账本真值）。
    async fn verify_stage_run_pass_token(
        &self,
        stage: crate::harness::StageKind,
        content: &str,
        deliverable: &crate::harness::StageDeliverable,
    ) -> (String, Option<HarnessGateOutcome>) {
        use crate::harness::org_gate::{
            completion_is_fresh, extract_pass_token, stage_pass_token, STAGE_COMPLETION_TTL_SECS,
        };
        // 整库口径（与 in_scope_assets 一致；chat 会话无 project key）。
        let org_ids = self.repo.in_scope_org_ids(None).await.unwrap_or_default();
        if org_ids.is_empty() {
            return render_specialist_gate(
                content,
                stage,
                false,
                vec![
                    "cannot verify stage completion: no in-scope organizations resolved — run \
                      scoping to build the org tree first"
                        .to_string(),
                ],
                deliverable,
            );
        }
        let now = chrono::Utc::now();
        let fresh: Vec<(uuid::Uuid, chrono::DateTime<chrono::Utc>)> = self
            .repo
            .org_stage_completions_get(stage.as_str(), &org_ids)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, at)| completion_is_fresh(*at, now, STAGE_COMPLETION_TTL_SECS))
            .collect();
        let have: std::collections::HashSet<uuid::Uuid> = fresh.iter().map(|(o, _)| *o).collect();
        let missing: Vec<uuid::Uuid> = org_ids
            .iter()
            .copied()
            .filter(|o| !have.contains(o))
            .collect();
        if !missing.is_empty() {
            return render_specialist_gate(
                content,
                stage,
                false,
                vec![format!(
                    "stage not complete: {} of {} in-scope orgs have not freshly passed this \
                     stage's per-org gate — re-run stage_run for the missing org(s): {:?}",
                    missing.len(),
                    org_ids.len(),
                    missing
                )],
                deliverable,
            );
        }
        let expected = stage_pass_token(stage, &fresh);
        let reasons = match extract_pass_token(deliverable) {
            Some(tok) if tok == expected => {
                return render_specialist_gate(content, stage, true, vec![], deliverable);
            }
            Some(_) => vec![
                "stage_run pass_token mismatch (stale or wrong stage) — re-run \
                             stage_run for this stage and submit the fresh pass_token it returns"
                    .to_string(),
            ],
            None => vec![
                "missing stage_run pass_token — call stage_run for this stage, then \
                          submit a claim {kind:\"stage_run_pass_token\", summary:<pass_token>} \
                          from its result"
                    .to_string(),
            ],
        };
        render_specialist_gate(content, stage, false, reasons, deliverable)
    }

    /// PR3 (设计 2026-06-11-coverage-auto-derive) · fetch the session's evidence
    /// facts (asset, technique, outcome, id) so `coverage_complete` can project
    /// ledger-proven cells instead of demanding a hand-written matrix. `None`
    /// when the subtask has no stage / no session / no facts / lookup error —
    /// every fallback is the projection-off legacy behavior (fail-closed: a
    /// missing fact never fills a cell).
    async fn fetch_evidence_facts_for_gate(
        &self,
        planned: &PlannedSubtask,
        in_scope_assets: Option<&[String]>,
        task_id: Uuid,
    ) -> Option<Vec<crate::harness::gate::rule_engine::EvidenceFact>> {
        use crate::harness::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
        let stage = planned.harness_stage.as_ref()?.stage_kind;
        let sid = self.chat_session_id.as_deref()?;

        // Per-dimension freshness window (design 2026-06-22 §3.2): when this stage's
        // spec opts in (`freshness_window`), anchor the DB-truth org-intel facts to
        // this stage-run start (`operation_state.stage_started_at`) so a stale row
        // from a previous run can't satisfy a cell this run. Spec off / unresolved /
        // missing operation_state ⇒ None = presence-only (gray-switch safe).
        let run_start = if crate::harness::load_embedded_stage_spec(stage)
            .map(|s| s.freshness_window)
            .unwrap_or(false)
        {
            crate::db_shim::operation_state::get(&*self.repo, task_id)
                .await
                .ok()
                .flatten()
                .map(|s| s.stage_started_at)
        } else {
            None
        };

        // ① 账本派生（现有路径）：audit_log 三列齐全的行 → EvidenceFact。
        let mut facts: Vec<EvidenceFact> = match self.repo.evidence_facts_for_session(sid).await {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|(asset, technique, outcome, evidence_id)| {
                    // 保守解析：未知 outcome 字符串 → 丢行（不投影），绝不猜。
                    let outcome = match outcome.as_str() {
                        "found" => EvidenceOutcome::Found,
                        "empty" => EvidenceOutcome::Empty,
                        // T2：失败检查（gray-switch GOLISH_FAILURE_OUTCOME_ERROR）记 error。
                        "error" => EvidenceOutcome::Error,
                        _ => return None,
                    };
                    Some(EvidenceFact {
                        asset,
                        technique,
                        outcome,
                        evidence_id,
                    })
                })
                .collect(),
            Err(e) => {
                tracing::warn!(
                    target: "harness::hook",
                    error = %e,
                    "evidence-facts lookup failed; coverage gate runs without ledger projection"
                );
                Vec::new()
            }
        };

        // ② DB 业务表真值派生（设计 2026-06-12 §5.3）：org 已隔离的 in-scope 资产集上，
        // 业务表真有数据的 (asset × technique) 作为 Found 合并（只产 Found，哨兵 id=0）。
        // in_scope_assets 缺失（GUI/chat 路径 org_id=None 且无注入）→ 跳过，退回纯账本
        // 投影（零回归）。
        if let Some(assets) = in_scope_assets {
            match self
                .repo
                .db_truth_facts(self.harness_org_id, assets, run_start)
                .await
            {
                Ok(truth) if !truth.is_empty() => {
                    let n = truth.len();
                    facts.extend(db_truth_facts_to_evidence(truth));
                    tracing::info!(
                        target: "harness::hook",
                        db_truth_facts = n,
                        org_id = ?self.harness_org_id,
                        "merged DB business-table truth facts into coverage gate (Found only)"
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(
                        target: "harness::hook",
                        error = %e,
                        "db-truth-facts lookup failed; coverage gate runs without DB projection"
                    );
                }
            }
        }

        // ③ Phase 2 (2026-06-12-redteam-phase2): GOLISH-INTEL-SUBSIDIARY 是 org 级
        // 维度——账本事实的 asset 是公司名 (recon_discover_subsidiaries 的 subject),
        // 不是 in-scope 主机。展开投影到每个 in-scope asset, 让 per-asset coverage
        // 格能消费 (与 db_truth_facts 的 has_subsidiary org 级投影同构)。Empty 的
        // 展开是 I8 的关键: 「跑了→0 合格子」才能走 checked_empty 而非 not_attempted。
        if let Some(assets) = in_scope_assets {
            project_org_level_subsidiary_facts(&mut facts, assets);
        }

        // #4/E3 (设计 2026-06-23-technique-outcomes-provenance): **始终**从
        // technique_outcomes 物化表投影 EvidenceFact 并 **union** 进 facts（dual-read：
        // 与现有 ledger + coverage_truth 并存；additive + fail-safe 到空，无灰度开关）。
        // run_id = chat session；org 绑定才读（表 org NOT NULL）。outcome `blocked`→Error
        // （与 error 同终态语义；gate 的 EvidenceOutcome 无 Blocked 变体）。
        if let Some(org_id) = self.harness_org_id {
            let projected: Vec<EvidenceFact> = self
                .repo
                .technique_outcome_facts(org_id, sid)
                .await
                .into_iter()
                .filter_map(|(asset, technique, outcome, evidence_id)| {
                    let outcome = match outcome.as_str() {
                        "found" => EvidenceOutcome::Found,
                        "empty" => EvidenceOutcome::Empty,
                        "error" | "blocked" => EvidenceOutcome::Error,
                        _ => return None,
                    };
                    Some(EvidenceFact {
                        asset,
                        technique,
                        outcome,
                        evidence_id,
                    })
                })
                .collect();
            if !projected.is_empty() {
                tracing::info!(
                    target: "harness::hook",
                    technique_outcome_facts = projected.len(),
                    "#4: merged technique_outcomes projection into coverage gate (dual-read union)"
                );
                facts.extend(projected);
            }
        }

        if facts.is_empty() {
            return None;
        }
        tracing::info!(
            target: "harness::hook",
            fact_count = facts.len(),
            "injecting merged ledger+DB evidence facts into coverage gate (projection)"
        );
        Some(facts)
    }

    async fn fetch_source_queries_for_gate(
        &self,
        planned: &PlannedSubtask,
    ) -> Option<Vec<crate::harness::SourceQueryFact>> {
        planned.harness_stage.as_ref()?;
        let org_id = self.harness_org_id?;
        let sid = self.chat_session_id.as_deref()?;
        let rows = self.repo.source_query_facts(org_id, sid).await;
        if rows.is_empty() {
            return None;
        }
        tracing::info!(
            target: "harness::hook",
            source_query_facts = rows.len(),
            "#5: merged source_query_log rows into gate context"
        );
        Some(rows)
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
        // P0+ · the deliverable cited ids that don't exist. The recurring failure
        // mode (esp. with weaker models) is the agent copying the template
        // placeholders 1/2/3 because it never learned the REAL ledger ids that
        // its (often backgrounded) scans produced. Look up this operation's real
        // evidence ids and name them in the correction so the retry can cite real
        // ones instead of guessing. Scoped by the chat-session string both
        // evidence write paths stamp on the ledger; infra failure / no session
        // just yields an empty hint (still BLOCKs, mirroring fail-open elsewhere).
        let available_real_ids = match self.chat_session_id.as_deref() {
            Some(sid) => self
                .repo
                .recent_evidence_ids(sid, 25)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        target: "harness::hook",
                        error = %e,
                        "real evidence-id lookup failed; correcting without an id hint"
                    );
                    Vec::new()
                }),
            None => Vec::new(),
        };
        tracing::warn!(
            target: "harness::hook",
            stage = %outcome.gated_stage.as_str(),
            fabricated = ?fabricated,
            available_real_ids = ?available_real_ids,
            "gate BLOCK: deliverable cites evidence ids absent from the ledger"
        );
        block_outcome_for_fabricated(outcome, &fabricated, &available_real_ids);
    }

    /// 设计 2026-06-12-unified-refiner · missing-deliverable BLOCK 时查账本真实
    /// ids + kind 标签，作为「事实」填进 outcome——Refiner 据此分类（账本非空 →
    /// A 类 submit-only 锁；空 → B 类重做）并渲染。查询失败 / 无 session / 账本
    /// 空 = 不填（B 类自然兜住，never imply work was done when it wasn't）。
    async fn gather_missing_deliverable_ids(&self, outcome: &mut HarnessGateOutcome) {
        if outcome.gate_allowed || !outcome.missing_deliverable {
            return;
        }
        let Some(sid) = self.chat_session_id.as_deref() else {
            return;
        };
        let ids = match self.repo.recent_evidence_ids(sid, 25).await {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!(
                    target: "harness::hook",
                    error = %e,
                    "refiner fact-gathering: evidence-id lookup failed; redo-class will apply"
                );
                return;
            }
        };
        if ids.is_empty() {
            return;
        }
        // Kind labels make the echo even easier for a weak model ("#1632 (dns_a)").
        // Lookup failure only drops the labels, not the submit-only routing.
        outcome.evidence_kind_labels = self.repo.evidence_kinds_for(&ids).await.unwrap_or_default();
        outcome.available_real_ids = ids;
        tracing::info!(
            target: "harness::hook",
            stage = %outcome.gated_stage.as_str(),
            evidence_ids = ?outcome.available_real_ids,
            "refiner fact-gathering: work already evidenced in the ledger (submit-only candidate)"
        );
    }

    /// Red_team scoping anti-shortcut gate (设计 2026-06-06-scoping-per-mode-gate-hitl
    /// §3.4 P1 强化). The deterministic gate only checks that a `scope_human_approved`
    /// claim EXISTS — which a weak model can fabricate without doing the work. For
    /// red_team profiles (`scoping_policy.require_unit_candidates`) cross-verify
    /// against this session's REAL `tool_calls` that the model actually invoked
    /// `ask_human(input_type="unit_review")` AND `manage_organizations(action="create")`.
    /// Missing either ⇒ flip PASS→BLOCK + corrective hint. Fails OPEN when the
    /// action set can't be verified (no tool_calls recorded), mirroring the
    /// evidence cross-checks (never block on infra absence).
    async fn enforce_scoping_red_team_flow(
        &self,
        outcome: &mut HarnessGateOutcome,
        exec_ctx: &ExecutionContext,
    ) {
        // Only a scoping stage that PASSed so far is in scope for this check.
        if outcome.gated_stage != crate::harness::StageKind::Scoping || !outcome.gate_allowed {
            return;
        }
        if !crate::harness::feature_flags::scoping_human_gate_enabled() {
            return;
        }
        // Only red_team-style profiles (require_unit_candidates) enforce the
        // unit-candidate + organization-creation flow.
        if !scoping_policy_for_ctx(exec_ctx).require_unit_candidates {
            return;
        }
        let seen = match self.repo.scoping_actions_for_session(self.session_id).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    target: "harness::hook",
                    error = %e,
                    "scoping red_team flow cross-check failed; not blocking on infra error"
                );
                return;
            }
        };
        if let Some(correction) = evaluate_red_team_scoping_flow(seen) {
            tracing::warn!(
                target: "harness::hook",
                stage = %outcome.gated_stage.as_str(),
                "gate BLOCK: red_team scoping skipped the unit-candidate / organization-creation flow"
            );
            // 设计 2026-06-12-unified-refiner · 只置事实标记，Refiner G 类透传该文本。
            outcome.gate_allowed = false;
            outcome.red_team_flow_correction = Some(correction);
        }
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
        // 设计 2026-06-12-unified-refiner · 只置事实标记，渲染交 Refiner（E 类）。
        outcome.gate_allowed = false;
        outcome.missing_kinds = missing;
    }

    /// P0 Task 6 · evidence「新鲜度」回查: 查 ledger 真实 age, 按 `evidence_kinds.json`
    /// max_age 拦截**硬过期**证据 (age ≥ 2×max → BLOCK; 软陈旧只 warn)。infra 查询失败
    /// 只 warn 不误伤; 无 evidence_refs 时整段跳过。
    async fn enforce_evidence_freshness(&self, outcome: &mut HarnessGateOutcome) {
        if outcome.evidence_refs.is_empty() {
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
        // 设计 2026-06-12-unified-refiner · 只置事实标记，渲染交 Refiner（E 类）。
        outcome.gate_allowed = false;
        outcome.expired = expired;
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

/// Pure: flip a gate outcome to BLOCK on fabricated evidence refs, recording
/// the facts (`fabricated_evidence_refs` + `available_real_ids`) for the
/// unified Refiner's D-class template（设计 2026-06-12-unified-refiner——渲染
/// 不再在此发生，HarnessTrace 观测字段不变）.
fn block_outcome_for_fabricated(
    outcome: &mut HarnessGateOutcome,
    fabricated: &[i64],
    available_real_ids: &[i64],
) {
    outcome.gate_allowed = false;
    outcome.fabricated_evidence_refs = fabricated.to_vec();
    outcome.available_real_ids = available_real_ids.to_vec();
}

// ── Harness gate hook (Phase C · Doc 3 §5.2 接入点) ─────────────────────────
//
// 仅当满足以下全部条件时, agent_result.content 末尾才会被追加 gate decision JSON:
//   1. `planned.harness_stage` 非 None
//   2. agent_result.content 含可解析的 StageDeliverable JSON
//      (整体即 JSON, 或 ```json fence 内的 JSON)
//
// Phase C: 支持**任意 stage** —— 按 `stage_hint.stage_kind` 从嵌入 registry 载对应
// StageSpec, 跑通用 gate (`validate_stage_gate`). 条件不满足时返回原 content.
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
    /// Observability (design 2026-06-05) · when an evidence-existence BLOCK fired,
    /// the cited ids absent from the ledger (fabricated) and the real ids that
    /// were available at decision time. Surfaced into the `HarnessTrace`
    /// GateDecision event so the timeline shows "cited placeholders while real
    /// ids existed". Empty unless `block_outcome_for_fabricated` ran.
    fabricated_evidence_refs: Vec<i64>,
    available_real_ids: Vec<i64>,
    /// 设计 2026-06-11 (weak-model-submit-channel) · `true` when this BLOCK was
    /// produced by the missing-deliverable path (stage-tagged subtask ended with
    /// no parseable `StageDeliverable`). Drives the targeted repair: the caller
    /// looks up the ledger's real evidence ids and, when work was actually done,
    /// rewrites the correction to "ONLY submit, do not redo" and locks the retry
    /// pass's tool_choice to `submit_stage_deliverable`.
    missing_deliverable: bool,
    // ── 设计 2026-06-12-unified-refiner · 以下为 Refiner 的「事实」输入 ──
    // gate 与 enforce_* 只置事实标记；纠正文本的渲染权全部上收
    // `task_orchestrator::refiner`（repair_correction 由它回填）。
    /// gate 原始拒绝理由（`decision.reasons` 克隆）。
    gate_reasons: Vec<String>,
    gate_recovery: Option<crate::harness::HarnessRecoveryActions>,
    /// enforce_evidence_kinds 置：stage 要求但 deliverable 证据缺失的 kinds。
    missing_kinds: Vec<String>,
    /// enforce_evidence_freshness 置：硬过期证据的描述行。
    expired: Vec<String>,
    /// enforce_scoping_red_team_flow 置：已渲染好的流程纠正（G 类透传）。
    red_team_flow_correction: Option<String>,
    /// `StageSpec.allowed_tool_types.is_empty()`（A 类 confirm-only 变体判定）。
    confirm_only_stage: bool,
    /// missing-deliverable 时账本真实 id → kind 标签（A 类模板）。
    evidence_kind_labels: std::collections::HashMap<i64, String>,
    /// Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): when a
    /// scoping deliverable confirmed the engagement subject org, its
    /// `organization_id` (parsed from the scope claim subject). On PASS the
    /// orchestrator binds it as `harness_org_id` + persists it to operation_state
    /// so downstream stages confine to that org's subtree. `None` = not scoping /
    /// no org id in the claim (fail-open).
    engagement_org_id: Option<uuid::Uuid>,
}

impl HarnessGateOutcome {
    /// Refiner 的纯输入视图（事实 → 分类/渲染，无 IO）。`facts` 为 stage-close
    /// hook 注入 coverage gate 的同一份证据事实（C 类诊断用）。
    fn as_refine_input<'a>(
        &'a self,
        facts: Option<&'a [crate::harness::gate::rule_engine::EvidenceFact]>,
    ) -> crate::task_orchestrator::refiner::RefineInput<'a> {
        crate::task_orchestrator::refiner::RefineInput {
            stage: self.gated_stage,
            gate_reasons: &self.gate_reasons,
            gate_recovery: self.gate_recovery.as_ref(),
            missing_deliverable: self.missing_deliverable,
            confirm_only_stage: self.confirm_only_stage,
            fabricated_ids: &self.fabricated_evidence_refs,
            available_real_ids: &self.available_real_ids,
            evidence_kind_labels: &self.evidence_kind_labels,
            missing_kinds: &self.missing_kinds,
            expired: &self.expired,
            red_team_flow_correction: self.red_team_flow_correction.as_deref(),
            evidence_facts: facts,
        }
    }
}

/// 返回 `(content, Option<outcome>)`: `None` 表示 hook 透传 (未跑 gate); `Some` 表示
/// 跑了 gate, 调用方据此驱动 stage 流转 (推进 operation_state 游标).
/// P3 ③ seam: map the in-scope assets' `targets.type` values onto this gate's
/// **dynamic** expected techniques. Returns `None` when there is no asset-type
/// information so `coverage_complete` falls back to `spec.expected_techniques`
/// (zero behavior change). Empty result (a stage with no coverage matrix, e.g.
/// scoping) is also `None`. Delegated to the pure `technique_resolver`.
fn gate_expected_techniques(
    stage: crate::harness::StageKind,
    target_types: &[String],
) -> Option<Vec<String>> {
    // 委托共享 helper（设计 2026-06-23-submit-preview-authoritative-context）：
    // stage-close 与 submit 预检共用同一派生，保证两路期望技术口径一致。
    crate::harness::sprint_contract::expected_techniques_for_target_types(stage, target_types)
}

/// Phase 2 (2026-06-12-redteam-phase2) · org 级 SUBSIDIARY 事实展开: 账本里
/// `GOLISH-INTEL-SUBSIDIARY` 的事实 asset 是公司名, 与 coverage 矩阵的 in-scope
/// asset 轴对不上——把每条 SUBSIDIARY 事实按 org 级语义复制到全部 in-scope
/// asset (去重)。纯函数便于单测。
fn project_org_level_subsidiary_facts(
    facts: &mut Vec<crate::harness::gate::rule_engine::EvidenceFact>,
    in_scope_assets: &[String],
) {
    use crate::harness::evidence_facts::TECH_SUBSIDIARY;
    let org_level: Vec<crate::harness::gate::rule_engine::EvidenceFact> = facts
        .iter()
        .filter(|f| f.technique == TECH_SUBSIDIARY)
        .cloned()
        .collect();
    for fact in org_level {
        for asset in in_scope_assets {
            let exists = facts.iter().any(|f| {
                f.asset == *asset && f.technique == TECH_SUBSIDIARY && f.outcome == fact.outcome
            });
            if !exists {
                facts.push(crate::harness::gate::rule_engine::EvidenceFact {
                    asset: asset.clone(),
                    technique: TECH_SUBSIDIARY.to_string(),
                    outcome: fact.outcome,
                    evidence_id: fact.evidence_id,
                });
            }
        }
    }
}

/// Phase 2 · scoping gate 的 SUBSIDIARY 期望技术注入: engagement 订阅了子公司
/// (`--include-subsidiaries`) 且当前 stage 是 scoping → 把 `GOLISH-INTEL-SUBSIDIARY`
/// 并入 gate 的 expected techniques (静态 spec 留空, 不带 flag 时 hook 不注入 →
/// coverage_complete no-op → 零回归)。纯函数便于单测。
fn inject_subsidiary_expected_technique(
    base: Option<Vec<String>>,
    stage: crate::harness::StageKind,
    require_subsidiary: bool,
) -> Option<Vec<String>> {
    use crate::harness::evidence_facts::TECH_SUBSIDIARY;
    if !require_subsidiary || stage != crate::harness::StageKind::Scoping {
        return base;
    }
    let mut techniques = base.unwrap_or_default();
    if !techniques.iter().any(|t| t == TECH_SUBSIDIARY) {
        techniques.push(TECH_SUBSIDIARY.to_string());
    }
    Some(techniques)
}

/// `subsidiary_threshold` (Phase 2): `Some(pct)` = the engagement opted into
/// subsidiary scoping (`--include-subsidiaries --subsidiary-threshold <pct>`) —
/// scoping's gate then requires the GOLISH-INTEL-SUBSIDIARY dimension. `None`
/// = legacy scoping (no injection, zero behaviour change).
fn is_confirm_only_stage(
    stage: crate::harness::StageKind,
    subsidiary_threshold: Option<u8>,
) -> bool {
    match stage {
        crate::harness::StageKind::Scoping => subsidiary_threshold.is_none(),
        crate::harness::StageKind::Reporting => true,
        _ => false,
    }
}

fn apply_harness_gate_hook(
    planned: &PlannedSubtask,
    exec_ctx: &ExecutionContext,
    content: String,
    in_scope_assets: Option<Vec<String>>,
    asset_types: Option<std::collections::HashMap<String, String>>,
    in_scope_target_types: Vec<String>,
    evidence_facts: Option<Vec<crate::harness::gate::rule_engine::EvidenceFact>>,
    source_queries: Option<Vec<crate::harness::SourceQueryFact>>,
    subsidiary_threshold: Option<u8>,
) -> (String, Option<HarnessGateOutcome>) {
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

    // C2d · per-target gate. Attach the profile's per-stage skeleton so the gate
    // also checks expected finding-count ranges + min tool invocations
    // (per-target, not just structural). No-op when the profile ships no skeleton
    // or the stage has no skeleton entry.
    let mut harness = match crate::harness::load_embedded_sprint_skeleton(profile_id) {
        Ok(Some(skeleton)) => match skeleton.for_stage(stage_hint.stage_kind) {
            Some(stage_skel) => {
                tracing::info!(
                    target: "harness::hook",
                    stage_kind = ?stage_hint.stage_kind,
                    profile_id = %profile_id,
                    "sprint-skeleton enforcement: gate will check per-target finding ranges"
                );
                harness.with_skeleton(Some(stage_skel.clone()))
            }
            None => harness,
        },
        _ => harness,
    };

    // scoping 人工确认硬门禁（设计 2026-06-06-scoping-per-mode-gate-hitl §3.4）：除 smoke 外
    // （profile.scoping_policy.require_human_scope_approval=true），scoping 通过前 deliverable
    // 必须带一条 kind="scope_human_approved" 的 claim，否则 gate Block、不许进 target_intel。
    // 灰度由 GOLISH_SCOPING_HUMAN_GATE 控制（默认开）；规则用现有 count_at_least 积木，不改引擎。
    if matches!(stage_hint.stage_kind, crate::harness::StageKind::Scoping)
        && crate::harness::feature_flags::scoping_human_gate_enabled()
        && harness.profile.scoping_policy.require_human_scope_approval
    {
        harness
            .stage_spec
            .gate_rules
            .push(crate::harness::gate::scoping_human_gate_rule());
        tracing::info!(
            target: "harness::hook",
            profile_id = %profile_id,
            "scoping human-approval hard gate injected (deliverable must carry a scope_human_approved claim)"
        );
    }

    // Confirm-only 是阶段语义，不再由 `allowed_tool_types.is_empty()` 推断：
    // target_intel / cleanup 可以禁止模型直接调外部工具，但仍有 substantive gate。
    // missing-deliverable 分支与 Refiner 的 A 类 confirm-only 变体共用此事实。
    // Phase 2 (redteam-phase2)：scoping 的静态白名单已含 recon/osint（子公司发现
    // 工具），但「要不要真跑工具」是 engagement 级决定——不带
    // `--include-subsidiaries` 时 scoping 仍是纯授权确认阶段（confirm-only，
    // 行为与白名单为空时逐字节一致，零回归）；带 flag 才是真工具阶段
    // （missing deliverable + 账本空 → B 类重做引导跑子公司发现）。
    let confirm_only = is_confirm_only_stage(stage_hint.stage_kind, subsidiary_threshold);

    let deliverable = match parse_deliverable_from_content(&content) {
        Some(d) => d,
        None => {
            // 设计 2026-06-12-unified-refiner (PR-R2/R3) · missing deliverable 一律
            // fail-closed BLOCK——后端不再代为合成（既不替 confirm-only 阶段填确认
            // claim，也不从账本投影）。Refiner 按账本事实路由纠正：confirm-only 或
            // 账本有真证据 → A 类 submit-only 锁；账本空 → B 类重做。deliverable
            // 永远出自主 agent 之手（红线，I7）。
            tracing::warn!(
                target: "harness::hook",
                stage_kind = ?stage_hint.stage_kind,
                subtask_title = %planned.title,
                content_len = content.len(),
                confirm_only,
                "harness gate: stage-tagged subtask produced no parseable StageDeliverable JSON block — BLOCK (fail-closed)"
            );
            return (
                content,
                missing_deliverable_gate_outcome(stage_hint.stage_kind, confirm_only),
            );
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

    // Phase 2 ①③ seam activation: inject the authoritative in-scope asset set
    // (recon-populated `targets.scope='in'`) so coverage_complete measures real
    // coverage. `None` (no recon assets yet) falls back to self-reported.
    // ③ (P3): dynamic expected techniques derived from the in-scope assets'
    // `targets.type` — `None` (no type info / non-coverage stage) falls back to
    // `spec.expected_techniques` (zero behavior change).
    // Phase 2 (redteam-phase2): when the engagement opted into subsidiary
    // scoping, scoping's coverage matrix additionally requires the org-tree
    // dimension (GOLISH-INTEL-SUBSIDIARY) — without the flag nothing is
    // injected and scoping's coverage_complete stays a no-op (zero回归).
    let expected_techniques = inject_subsidiary_expected_technique(
        gate_expected_techniques(stage_hint.stage_kind, &in_scope_target_types),
        stage_hint.stage_kind,
        subsidiary_threshold.is_some(),
    );
    if let (Some(pct), crate::harness::StageKind::Scoping) =
        (subsidiary_threshold, stage_hint.stage_kind)
    {
        tracing::info!(
            target: "harness::hook",
            subsidiary_threshold_pct = pct,
            "scoping subsidiary gate active: GOLISH-INTEL-SUBSIDIARY injected into expected techniques"
        );
    }
    // 统一组装入口（设计 2026-06-23-unified-gate-context-builder）：4 个值此前已是
    // Option（fetch helper 预归一）+ source rows → unwrap_or_default 喂 builder、build() 再归一，
    // 与手搓 GateContext{} 逐字节同构（行为保持）。
    let gate_ctx = crate::harness::GateContextBuilder::new()
        .in_scope_assets(in_scope_assets.unwrap_or_default())
        .asset_types_map(asset_types.unwrap_or_default())
        .extend_evidence_facts(evidence_facts.unwrap_or_default())
        .extend_source_queries(source_queries.unwrap_or_default())
        .expected_techniques(expected_techniques)
        .build();
    let decision = harness.validate_gate_with_context(&deliverable, None, &gate_ctx);

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

    // 设计 2026-06-12-unified-refiner · BLOCK 时不再在此渲染纠正文本——gate 只
    // 交「事实」（reasons / recovery），repair_correction 由调用方经 Refiner 回填。
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
            engagement_org_id: extract_engagement_org_if_scoping(
                stage_hint.stage_kind,
                &deliverable,
            ),
            repair_correction: None,
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
            fabricated_evidence_refs: Vec::new(),
            available_real_ids: Vec::new(),
            missing_deliverable: false,
            gate_reasons: decision.reasons.clone(),
            gate_recovery: decision.recovery_actions.clone(),
            missing_kinds: Vec::new(),
            expired: Vec::new(),
            red_team_flow_correction: None,
            confirm_only_stage: confirm_only,
            evidence_kind_labels: std::collections::HashMap::new(),
        }),
    )
}

/// Phase 1.5 · 构造 fan-out 阶段 token gate 的 outcome（PASS/BLOCK）。复刻 apply_harness_gate_hook
/// 的「## Harness Gate Decision」渲染 + HarnessGateOutcome 形状，但 `evidence_refs` /
/// `required_evidence_kinds` 留空——per-org 证据已在 Phase 1 各自过 gate，阶段收尾只认账本聚合
/// （令牌），不再要求主 agent 交付物带证据；空字段也让收尾的 evidence 强制（existence/kinds/
/// freshness）对本 outcome 一律 no-op。
fn render_specialist_gate(
    content: &str,
    stage: crate::harness::StageKind,
    allowed: bool,
    reasons: Vec<String>,
    deliverable: &crate::harness::StageDeliverable,
) -> (String, Option<HarnessGateOutcome>) {
    let decision = serde_json::json!({
        "allowed": allowed,
        "gate": "stage_run_pass_token",
        "reasons": reasons.clone(),
    });
    let decision_json = serde_json::to_string_pretty(&decision)
        .unwrap_or_else(|_| "{\"error\":\"failed to serialize gate decision\"}".to_string());
    let mut out = content.to_string();
    out.push_str("\n\n## Harness Gate Decision\n\n```json\n");
    out.push_str(&decision_json);
    out.push_str("\n```\n");
    (
        out,
        Some(HarnessGateOutcome {
            gated_stage: stage,
            gate_allowed: allowed,
            engagement_org_id: None,
            repair_correction: None,
            evidence_summary: Some(summarize_deliverable(deliverable)),
            evidence_refs: Vec::new(),
            required_evidence_kinds: Vec::new(),
            findings_count: deliverable.findings.len(),
            fabricated_evidence_refs: Vec::new(),
            available_real_ids: Vec::new(),
            missing_deliverable: false,
            gate_reasons: reasons,
            gate_recovery: None,
            missing_kinds: Vec::new(),
            expired: Vec::new(),
            red_team_flow_correction: None,
            confirm_only_stage: false,
            evidence_kind_labels: std::collections::HashMap::new(),
        }),
    )
}

/// C6 · render a compact, bounded summary of a gate-passed deliverable for the
/// cross-stage handoff store. Lists the first few claims + findings (kind +
/// subject) and the evidence-ref count so a downstream stage sees what upstream
/// actually produced without re-reading the full deliverable JSON.
/// Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): pull the
/// scoping-confirmed engagement root org id out of a scoping deliverable. The
/// agent sets the confirmed `organization_id` as the `subject` of its scope claim
/// (`scope_confirmed` / `scope_human_approved` / `engagement_org`); we take the
/// first such claim whose subject parses as a UUID. `None` (stage ≠ scoping, or no
/// UUID subject) ⇒ no binding (fail-open: legacy whole-DB behavior is unchanged).
fn extract_engagement_org_if_scoping(
    stage: crate::harness::StageKind,
    deliverable: &crate::harness::StageDeliverable,
) -> Option<uuid::Uuid> {
    if stage != crate::harness::StageKind::Scoping {
        return None;
    }
    deliverable
        .claims
        .iter()
        .filter(|c| {
            matches!(
                c.kind.as_str(),
                "scope_confirmed" | "scope_human_approved" | "engagement_org"
            )
        })
        .find_map(|c| uuid::Uuid::parse_str(c.subject.trim()).ok())
}

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
/// 时扫描**所有** fence、取**最后一个**能解析的 (PR1 设计 2026-06-11 · 运输鲁棒性:
/// submit 工具侧信道捕获的权威 deliverable 被 append 在 content 末尾, 不能被模型
/// 散文里更早出现的解释性/残缺 json 块遮蔽; 多份可解析时最后提交的覆盖早先草稿).
/// 都失败返 None → hook 按 stage-tagged 与否决定 BLOCK / skip.
fn parse_deliverable_from_content(content: &str) -> Option<crate::harness::StageDeliverable> {
    if let Ok(d) = serde_json::from_str::<crate::harness::StageDeliverable>(content.trim()) {
        return Some(d);
    }
    let mut last_parseable = None;
    let mut rest = content;
    while let Some(start) = rest.find("```json") {
        let after = &rest[start + "```json".len()..];
        let Some(end) = after.find("```") else {
            break;
        };
        if let Ok(d) = serde_json::from_str::<crate::harness::StageDeliverable>(after[..end].trim())
        {
            last_parseable = Some(d);
        }
        rest = &after[end + "```".len()..];
    }
    last_parseable
}

/// 设计 2026-06-12 §5.3 · 把 DB 业务表真值 `(asset, technique)` 转成 `Found`
/// EvidenceFact，供与账本 facts 合并注入 coverage gate。
///
/// 红线：
/// - outcome 恒 `Found` —— 业务表「有数据」即 Found；本函数永不产 `Empty`
///   （checked_empty 只能由账本「跑了→空」的真实 outcome 显式产生，I8）。
/// - `evidence_id` 用哨兵 [`DB_TRUTH_EVIDENCE_ID`]（0）标记「非账本来源」。
///   `coverage_complete` 投影只看 asset/technique/outcome（不看 id），哨兵无影响；
///   哨兵 fact 没有可引用的账本行，绝不进任何 deliverable 的 `evidence_refs` /
///   claims，fabricated-evidence 校验天然不误伤（设计 §4.1）。
const DB_TRUTH_EVIDENCE_ID: i64 = 0;

fn db_truth_facts_to_evidence(
    facts: Vec<(String, String)>,
) -> Vec<crate::harness::gate::rule_engine::EvidenceFact> {
    use crate::harness::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
    facts
        .into_iter()
        .map(|(asset, technique)| EvidenceFact {
            asset,
            technique,
            outcome: EvidenceOutcome::Found,
            evidence_id: DB_TRUTH_EVIDENCE_ID,
        })
        .collect()
}

/// S4 · the gate outcome when a stage-tagged subtask ends without a parseable
/// `StageDeliverable`: a fail-closed BLOCK. The correction text is rendered by
/// the unified Refiner（A 类 submit-only 或 B 类重做，由账本事实决定）——this
/// outcome only carries the facts (`missing_deliverable` + `confirm_only_stage`).
fn missing_deliverable_gate_outcome(
    stage: crate::harness::StageKind,
    confirm_only: bool,
) -> Option<HarnessGateOutcome> {
    Some(HarnessGateOutcome {
        gated_stage: stage,
        gate_allowed: false,
        engagement_org_id: None,
        repair_correction: None,
        evidence_summary: None,
        evidence_refs: Vec::new(),
        required_evidence_kinds: Vec::new(),
        findings_count: 0,
        fabricated_evidence_refs: Vec::new(),
        available_real_ids: Vec::new(),
        missing_deliverable: true,
        gate_reasons: Vec::new(),
        gate_recovery: None,
        missing_kinds: Vec::new(),
        expired: Vec::new(),
        red_team_flow_correction: None,
        confirm_only_stage: confirm_only,
        evidence_kind_labels: std::collections::HashMap::new(),
    })
}

/// S2 (DAG-strict) · synthesize one stage-scoped [`PlannedSubtask`] for a stage
/// the planner produced no subtask for, so the stage actually executes + gets
/// gated instead of being vacuously passed. The description is a per-stage
/// charter scoped to the operation target (`task_input`); `harness_stage` is set
/// so the gate hook runs against this stage.
/// Resolve the operation's scoping policy from the profile threaded via
/// `exec_ctx` (设计 2026-06-06-scoping-per-mode-gate-hitl §3.3). Falls back to the
/// conservative `ScopingPolicy::default()` (human gate ON) when no profile id is
/// set or it cannot be loaded — same fail-safe stance as `apply_harness_gate_hook`.
fn scoping_policy_for_ctx(exec_ctx: &ExecutionContext) -> crate::harness::profile::ScopingPolicy {
    exec_ctx
        .harness_profile_id
        .as_deref()
        .and_then(|id| crate::harness::load_embedded_profile(id).ok().flatten())
        .map(|p| p.scoping_policy)
        .unwrap_or_default()
}

/// Pure decision for [`TaskOrchestrator::enforce_scoping_red_team_flow`]: given the
/// cross-verified scoping actions, return `Some(correction)` when the red_team
/// unit-candidate / organization-creation flow was skipped (⇒ BLOCK), or `None`
/// to allow. `None` actions (unverifiable, e.g. no recorded tool_calls) allow
/// (fail open).
fn evaluate_red_team_scoping_flow(
    seen: Option<crate::db_traits::ScopingActionsSeen>,
) -> Option<String> {
    let seen = seen?; // unverifiable → fail open (allow)
    if seen.unit_review_invoked && seen.organization_created {
        return None;
    }
    let mut missing = Vec::new();
    if !seen.unit_review_invoked {
        missing.push(
            "ask_human(input_type=\"unit_review\") for the user to confirm/edit candidate units",
        );
    }
    if !seen.organization_created {
        missing.push(
            "a SUCCESSFUL manage_organizations(action=\"create\") that actually records the \
             organization (a create that returns an error — e.g. a duplicate name — does NOT \
             count; resolve it and retry)",
        );
    }
    Some(format!(
        "RED-TEAM SCOPING INCOMPLETE — a `scope_human_approved` claim is present but this run never \
         performed the required unit-candidate flow. Missing: {}. Before you submit the scoping \
         deliverable you MUST actually call manage_organizations(action=\"propose_candidates\"), then \
         ask_human(input_type=\"unit_review\") so the user judges the candidate units, then \
         manage_organizations(action=\"create\"). A claim alone is not sufficient.",
        missing.join(" and ")
    ))
}

/// Resolve the operation's `intel_policy` from the profile threaded via
/// `exec_ctx` (设计 2026-06-06-intel-stage-ai-driven-per-mode §3.5). Falls back
/// to the conservative `IntelPolicy::default()` (run passive intel) when no
/// profile id is set or it cannot be loaded.
fn intel_policy_for_ctx(exec_ctx: &ExecutionContext) -> crate::harness::profile::IntelPolicy {
    exec_ctx
        .harness_profile_id
        .as_deref()
        .and_then(|id| crate::harness::load_embedded_profile(id).ok().flatten())
        .map(|p| p.intel_policy)
        .unwrap_or_default()
}

fn synthesize_stage_subtask(
    stage: crate::harness::StageKind,
    task_input: &str,
    scoping_policy: &crate::harness::profile::ScopingPolicy,
    intel_policy: &crate::harness::profile::IntelPolicy,
) -> PlannedSubtask {
    use crate::harness::profile::{AssetConfirmation, PassiveIntelMode, SubjectKind};
    use crate::harness::StageKind as K;
    let target = task_input.trim();
    let (title, description, agent): (&str, String, &str) = match stage {
        K::Scoping => {
            // scoping prompt 按 profile 的 scoping_policy 分流 (设计 2026-06-06 §3.3):
            // 确认主体 → (红队) 列单位候选交人 → 列资产交人增删改 → 人确认后记
            // scope_human_approved claim. 每步由 policy 字段开关, smoke 全关 = 直接确认.
            let mut steps = String::new();
            if scoping_policy.require_subject {
                steps.push_str(match scoping_policy.subject_kind {
                    SubjectKind::Organization | SubjectKind::OrganizationOrFreetext => {
                        if scoping_policy.write_organizations
                            && !scoping_policy.require_unit_candidates
                        {
                            "1) Identify the engagement subject organization; create or select it via manage_organizations(action=\"create\"/\"list\") and CONFIRM it with the user (org-first: every target must link to this organization_id). "
                        } else {
                            "1) Identify and CONFIRM the engagement subject (the target organization). "
                        }
                    }
                    SubjectKind::CloudTenant => {
                        "1) Identify and CONFIRM the cloud tenant/account that is the engagement subject. "
                    }
                    SubjectKind::None | SubjectKind::Freetext => {
                        "1) State and CONFIRM the engagement subject. "
                    }
                });
            }
            if scoping_policy.require_unit_candidates {
                steps.push_str(
                    "2) Call manage_organizations(action=\"propose_candidates\") to list candidate unit/organization names (subsidiaries, aliases), then ask_human(input_type=\"unit_review\") so the user can judge/edit them; create confirmed orgs with manage_organizations(action=\"create\"). ",
                );
            }
            if matches!(
                scoping_policy.asset_confirmation,
                AssetConfirmation::Interactive
            ) {
                steps.push_str(
                    "3) Parse the user input into a candidate target list (mark in/out of scope), call ask_human(input_type=\"scope_review\") so the user can add/remove/edit, and ONLY AFTER approval write them via manage_targets(action=\"add\", with scope/organization_id). ",
                );
            }
            if scoping_policy.require_human_scope_approval {
                steps.push_str(
                    "4) After human approval, record a claim {kind:\"scope_human_approved\", subject:<engagement subject>} citing the ask_human request_id, then submit_stage_deliverable. ",
                );
            }
            steps.push_str("Do NOT perform any active scanning in this stage.");
            (
                "Scope & Authorization Confirmation",
                format!("Confirm and document the engagement scope for `{target}`. {steps}"),
                "pentester",
            )
        }
        K::TargetIntel => {
            // target_intel prompt 按 intel_policy 分流
            // (设计 2026-06-06-intel-stage-ai-driven-per-mode §3.5):
            // 渗透 skip 直接空跑; 红队/评估跑被动 (子公司发现 → 字段富化 → 引证 evidence).
            let mut steps = String::new();
            if matches!(intel_policy.passive_intel, PassiveIntelMode::Skip) {
                steps.push_str(
                    "Assets were already confirmed during scoping; this engagement SKIPS passive intel. Do NOT run passive providers. Mark each expected intel technique coverage cell as not_applicable with a short note (\"assets confirmed in scoping; passive intel skipped per mode\"), then submit_stage_deliverable.",
                );
            } else {
                let mut n = 1;
                steps.push_str(&format!(
                    "{n}) Call recon_list_providers first to see which passive providers have a configured credential; only invoke providers reported as available, and for any expected intel technique with no available provider record its coverage as blocked (no credential configured) — do NOT fabricate. ",
                ));
                n += 1;
                if intel_policy.discover_subsidiaries {
                    steps.push_str(&format!(
                        "{n}) Call recon_discover_subsidiaries(organization_id=<confirmed subject org>) to passively enumerate subsidiary/affiliate organizations via enterprise intel (ENScan); review the candidates it records. ",
                    ));
                    n += 1;
                }
                if intel_policy.enrich_assets {
                    steps.push_str(&format!(
                        "{n}) Call recon_map_assets(organization_id=<org>) to passively survey domains/IPs/DNS-adjacent asset facts/ASN/subdomains/certificates/ICP/apps/emails via intel providers (0.zone/quake/…), then recon_lookup_whois(organization_id=<org>) for WHOIS (RDAP, once per org). target_intel is provider/registry-tool backed; do not run scan-tool fallback here. ",
                    ));
                    n += 1;
                }
                steps.push_str(&format!(
                    "{n}) For each in-scope asset, give every expected intel technique (GOLISH-INTEL-DNS/WHOIS/ASN/CT/SUBDOMAIN/OSINT) a terminal coverage status, citing the evidence ids the tools recorded. If recon_map_assets/recon_lookup_whois cannot land a required technique, record it as blocked/checked_empty/not_applicable with note/evidence instead of switching tools. Then submit_stage_deliverable. Do NOT perform active scanning in this stage.",
                ));
            }
            (
                "Passive Target Intelligence",
                format!("Gather passive intelligence on `{target}` without touching the target. {steps}"),
                "pentester",
            )
        }
        K::ExternalAttackSurface => (
            "External Attack Surface Mapping",
            format!(
                "DEFINE the external attack surface of the hosts inherited from target_intel \
                 for `{target}`: PORT SCANNING, service/version fingerprinting, HTTP probing, \
                 and screenshots — establish host x port x service x live-web. Confirm liveness \
                 with httpx (it resolves + probes in one shot). Passive provider survey \
                 was ALREADY done upstream in target_intel — REUSE the inherited evidence \
                 and do NOT re-enumerate. JS/API extraction happens in the \
                 NEXT stage (enumeration) on the services you map here."
            ),
            "pentester",
        ),
        K::Enumeration => (
            "Content Enumeration",
            format!(
                "Enumerate the CONTENT of the services mapped by external_attack_surface for \
                 `{target}`: JS collection + API endpoint extraction, directory/path discovery, \
                 and parameter discovery. Ports/services were already mapped upstream — do NOT \
                 re-port-scan; the units (endpoints/paths/params) you record feed vuln triage."
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

/// L4a (Task 断线恢复): decide whether a terminal engine outcome means the
/// operation merely *paused* (and is resumable) rather than finished.
///
/// `Some(summary)` → the caller marks the task `Waiting` (paused) and returns
/// `summary` WITHOUT running the reporter, so the next user message resumes from
/// the persisted checkpoint. `None` → the normal terminal path (run the reporter,
/// mark `Finished`). Only `Interrupted` pauses; `Completed` / `Failed` / a
/// transport `Err` fall through (a hard failure is finalized elsewhere). This is
/// the fix for paused ops previously being marked Finished (and so unresumable).
fn paused_disposition(
    outcome: &crate::harness::graph_engine::Result<
        crate::harness::graph_engine::RunOutcome<
            crate::harness::operation_flow::OperationFlowState,
        >,
    >,
) -> Option<String> {
    match outcome {
        Ok(crate::harness::graph_engine::RunOutcome::Interrupted {
            reason,
            resume_from,
            ..
        }) => Some(format!(
            "Operation paused at stage '{resume_from}' ({reason}). Send a message to resume."
        )),
        _ => None,
    }
}

#[cfg(test)]
mod dag_driven_helper_tests {
    use super::*;
    use crate::harness::StageKind;

    #[test]
    fn missing_deliverable_outcome_carries_facts_for_the_refiner() {
        let o = missing_deliverable_gate_outcome(StageKind::Enumeration, false)
            .expect("missing deliverable must produce a BLOCK outcome");
        assert!(
            !o.gate_allowed,
            "missing-deliverable must BLOCK (fail-closed)"
        );
        assert_eq!(o.gated_stage, StageKind::Enumeration);
        // 设计 2026-06-12-unified-refiner · the outcome carries FACTS only; the
        // correction text is rendered by the refiner (A/B class by ledger state).
        assert!(
            o.repair_correction.is_none(),
            "rendering moved to the refiner — the outcome must not pre-render"
        );
        assert!(
            o.missing_deliverable,
            "missing-deliverable outcome must be marked so the refiner can route A/B"
        );
        assert!(!o.confirm_only_stage);
        let d = crate::task_orchestrator::refiner::refine(&o.as_refine_input(None));
        assert_eq!(
            d.class,
            crate::task_orchestrator::refiner::RefineClass::RedoStage,
            "empty ledger + substantive stage must route to the redo template"
        );
        assert!(d
            .correction
            .contains("did not include a parseable StageDeliverable"));
    }

    // 设计 2026-06-12-unified-refiner · missing + ledger has real ids ⇒ the
    // refiner routes to the submit-only template AND requests the tool lock
    // （投影兜底截胡 bug 的接线级回归锚点）.
    #[test]
    fn missing_with_ledger_ids_routes_to_submit_only_lock() {
        let mut o =
            missing_deliverable_gate_outcome(StageKind::TargetIntel, false).expect("BLOCK outcome");
        o.available_real_ids = vec![1634, 1632, 1700];
        o.evidence_kind_labels = std::collections::HashMap::from([
            (1632_i64, "dns_a".to_string()),
            (1634_i64, "http_probe".to_string()),
        ]);
        let d = crate::task_orchestrator::refiner::refine(&o.as_refine_input(None));
        assert_eq!(
            d.class,
            crate::task_orchestrator::refiner::RefineClass::SubmitOnly
        );
        assert!(d.submit_only_lock, "the retry pass must lock tool_choice");
        assert!(
            d.correction.contains("#1634 (http_probe)") && d.correction.contains("#1632 (dns_a)"),
            "ids must carry kind labels when known: {}",
            d.correction
        );
        assert!(
            d.correction.contains("#1700"),
            "ids without a known kind are still listed: {}",
            d.correction
        );
        assert!(
            d.correction.contains("Do NOT re-run any tools"),
            "must forbid redoing the stage work: {}",
            d.correction
        );
        assert!(
            d.correction.contains("target_intel"),
            "must name the stage being repaired: {}",
            d.correction
        );
    }

    #[test]
    fn synthesized_subtask_is_stage_tagged_and_targeted() {
        let policy = crate::harness::profile::ScopingPolicy::default();
        let intel = crate::harness::profile::IntelPolicy::default();
        let s = synthesize_stage_subtask(StageKind::Scoping, "example.com", &policy, &intel);
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
        let r = synthesize_stage_subtask(StageKind::Reporting, "example.com", &policy, &intel);
        assert_eq!(r.agent.as_deref(), Some("analyzer"));
    }

    /// T4 (设计 2026-06-06 §3.3): scoping 子任务描述随 profile 的 scoping_policy 分流——
    /// 红队 (require_unit_candidates + human gate) 出 unit_review + scope_review +
    /// scope_human_approved; smoke (gate off, asset_confirmation=none) 全不出现.
    #[test]
    fn scoping_subtask_prompt_varies_by_policy() {
        use crate::harness::profile::{AssetConfirmation, IntelPolicy, ScopingPolicy, SubjectKind};

        let red = ScopingPolicy {
            require_subject: true,
            subject_kind: SubjectKind::Organization,
            require_unit_candidates: true,
            asset_confirmation: AssetConfirmation::Interactive,
            require_human_scope_approval: true,
            write_organizations: true,
        };
        let intel = IntelPolicy::default();
        let s = synthesize_stage_subtask(StageKind::Scoping, "acme corp", &red, &intel);
        assert!(s.description.contains("unit_review"));
        assert!(s.description.contains("scope_review"));
        assert!(s.description.contains("scope_human_approved"));

        let smoke = ScopingPolicy {
            require_human_scope_approval: false,
            asset_confirmation: AssetConfirmation::None,
            ..ScopingPolicy::default()
        };
        let s2 = synthesize_stage_subtask(StageKind::Scoping, "x", &smoke, &intel);
        assert!(!s2.description.contains("scope_human_approved"));
        assert!(!s2.description.contains("unit_review"));
    }

    /// 红队 scoping 防偷懒硬门禁 (设计 2026-06-06 §3.4 P1 强化): 仅凭 claim 不够,
    /// 必须真的 invoke 了 unit_review + 建组织; 缺任一 → BLOCK; 无法核验 (None) → 放行.
    #[test]
    fn red_team_scoping_flow_blocks_when_steps_skipped() {
        use crate::db_traits::ScopingActionsSeen;

        // Both real steps performed → allow.
        assert!(evaluate_red_team_scoping_flow(Some(ScopingActionsSeen {
            unit_review_invoked: true,
            organization_created: true,
        }))
        .is_none());

        // Missing unit_review → BLOCK, correction names it.
        let c = evaluate_red_team_scoping_flow(Some(ScopingActionsSeen {
            unit_review_invoked: false,
            organization_created: true,
        }))
        .expect("missing unit_review must block");
        assert!(c.contains("unit_review"));

        // Missing organization create → BLOCK, correction names it.
        let c2 = evaluate_red_team_scoping_flow(Some(ScopingActionsSeen {
            unit_review_invoked: true,
            organization_created: false,
        }))
        .expect("missing org-create must block");
        assert!(c2.contains("manage_organizations(action=\"create\")"));

        // Both missing (the MiMo shortcut) → BLOCK, names both.
        let c3 = evaluate_red_team_scoping_flow(Some(ScopingActionsSeen::default()))
            .expect("both missing must block");
        assert!(c3.contains("unit_review") && c3.contains("create"));

        // Unverifiable (no recorded tool_calls) → fail open (allow).
        assert!(evaluate_red_team_scoping_flow(None).is_none());
    }

    /// T7 (设计 2026-06-06-intel-stage §3.5): target_intel 子任务描述随 intel_policy
    /// 分流——红队 (discover+enrich) 出 recon_discover_subsidiaries + recon_map_assets;
    /// 渗透 (passive_intel=skip) 出 not_applicable、不出现 recon 工具.
    #[test]
    fn target_intel_prompt_varies_by_intel_policy() {
        use crate::harness::profile::{IntelPolicy, PassiveIntelMode, ScopingPolicy};

        let scoping = ScopingPolicy::default();

        let red = IntelPolicy {
            passive_intel: PassiveIntelMode::Run,
            discover_subsidiaries: true,
            enrich_assets: true,
        };
        let s = synthesize_stage_subtask(StageKind::TargetIntel, "acme corp", &scoping, &red);
        assert!(s.description.contains("recon_list_providers"));
        assert!(s.description.contains("recon_discover_subsidiaries"));
        assert!(s.description.contains("recon_map_assets"));
        assert!(!s.description.contains("SKIPS passive intel"));
        assert!(!s.description.contains("subfinder"));
        assert!(!s.description.contains("dig"));

        let pentest = IntelPolicy {
            passive_intel: PassiveIntelMode::Skip,
            discover_subsidiaries: false,
            enrich_assets: false,
        };
        let s2 = synthesize_stage_subtask(StageKind::TargetIntel, "1.2.3.4", &scoping, &pentest);
        assert!(s2.description.contains("not_applicable"));
        assert!(!s2.description.contains("recon_list_providers"));
        assert!(!s2.description.contains("recon_discover_subsidiaries"));
        assert!(!s2.description.contains("recon_map_assets"));
    }

    // ── P3 ③ seam: dynamic expected_techniques in the gate hook ───────────────

    #[test]
    fn gate_expected_techniques_ip_only_enumeration_drops_param() {
        // IP-only scope → enumeration coverage matrix drops the web-only PARAM
        // technique (parameter discovery is meaningless without a web service).
        let t = super::gate_expected_techniques(StageKind::Enumeration, &["ip_address".into()])
            .expect("ip scope yields a technique set for enumeration");
        assert!(!t.contains(&"GOLISH-ENUM-PARAM".to_string()));
        assert!(t.contains(&"GOLISH-ENUM-DIR".to_string()));
    }

    #[test]
    fn gate_expected_techniques_none_when_no_target_types() {
        // No asset-type info → None → coverage_complete keeps spec.expected_techniques
        // (zero behavior change vs. the pre-P3 hardcoded None).
        assert!(super::gate_expected_techniques(StageKind::Enumeration, &[]).is_none());
    }

    #[test]
    fn gate_expected_techniques_none_for_non_coverage_stage() {
        // scoping declares no expected techniques → None even with asset types.
        assert!(super::gate_expected_techniques(StageKind::Scoping, &["domain".into()]).is_none());
    }

    // ── Phase 2 (2026-06-12-redteam-phase2): scoping SUBSIDIARY gate wiring ──

    #[test]
    fn subsidiary_injection_only_for_scoping_with_policy() {
        use crate::harness::evidence_facts::TECH_SUBSIDIARY;
        // scoping + policy → SUBSIDIARY 注入 (None base → Some([SUBSIDIARY])).
        let t = super::inject_subsidiary_expected_technique(None, StageKind::Scoping, true)
            .expect("scoping with policy injects");
        assert_eq!(t, vec![TECH_SUBSIDIARY.to_string()]);
        // 不带 flag (零回归): scoping 注入前后逐字节一致 — None 保持 None.
        assert!(
            super::inject_subsidiary_expected_technique(None, StageKind::Scoping, false).is_none()
        );
        // 非 scoping stage 不注入 (base 原样透传).
        let base = Some(vec!["GOLISH-INTEL-DNS".to_string()]);
        assert_eq!(
            super::inject_subsidiary_expected_technique(base.clone(), StageKind::TargetIntel, true),
            base
        );
        // base 已含 SUBSIDIARY → 不重复追加.
        let with = Some(vec![TECH_SUBSIDIARY.to_string()]);
        assert_eq!(
            super::inject_subsidiary_expected_technique(with.clone(), StageKind::Scoping, true),
            with
        );
    }

    #[test]
    fn org_level_subsidiary_facts_project_to_in_scope_assets() {
        use crate::harness::evidence_facts::TECH_SUBSIDIARY;
        use crate::harness::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
        // 账本事实的 asset 是公司名 ("默安科技"), in-scope 资产是域名 — 展开后
        // 每个 in-scope asset 拿到一条同 outcome 的 SUBSIDIARY 事实 (I8: Empty
        // 的展开让「跑了→0 合格子」能填 checked_empty 格).
        let mut facts = vec![EvidenceFact {
            asset: "默安科技".into(),
            technique: TECH_SUBSIDIARY.into(),
            outcome: EvidenceOutcome::Empty,
            evidence_id: 42,
        }];
        let assets = vec!["moresec.cn".to_string(), "moresec.com".to_string()];
        super::project_org_level_subsidiary_facts(&mut facts, &assets);
        for asset in &assets {
            assert!(
                facts.iter().any(|f| f.asset == *asset
                    && f.technique == TECH_SUBSIDIARY
                    && f.outcome == EvidenceOutcome::Empty
                    && f.evidence_id == 42),
                "expanded Empty fact missing for {asset}"
            );
        }
        // 幂等: 再跑一遍不重复.
        let n = facts.len();
        super::project_org_level_subsidiary_facts(&mut facts, &assets);
        assert_eq!(facts.len(), n, "projection must be idempotent");
        // 非 SUBSIDIARY 事实不被展开.
        let mut other = vec![EvidenceFact {
            asset: "默安科技".into(),
            technique: "GOLISH-INTEL-DNS".into(),
            outcome: EvidenceOutcome::Found,
            evidence_id: 7,
        }];
        super::project_org_level_subsidiary_facts(&mut other, &assets);
        assert_eq!(other.len(), 1, "non-subsidiary facts must not be projected");
    }

    /// L4a: only an engine `Interrupted` is a resumable pause (→ task `Waiting`,
    /// skip the reporter). `Completed`/`Failed` must fall through to the normal
    /// terminal path so a paused op is never wrongly marked Finished — the very
    /// bug that made断线后『继续』restart scoping instead of resuming.
    #[test]
    fn paused_disposition_pauses_on_interrupt_only() {
        use crate::harness::graph_engine::{Result as GraphResult, RunOutcome};
        use crate::harness::operation_flow::OperationFlowState;

        let interrupted: GraphResult<RunOutcome<OperationFlowState>> =
            Ok(RunOutcome::Interrupted {
                state: OperationFlowState::default(),
                reason: "gate blocked".to_string(),
                resume_from: "enumeration".to_string(),
            });
        let p = paused_disposition(&interrupted)
            .expect("interrupt must yield a paused (resumable) disposition");
        assert!(p.contains("paused"), "summary must say paused: {p}");
        assert!(
            p.contains("enumeration"),
            "summary must name the resume-from stage: {p}"
        );

        let completed: GraphResult<RunOutcome<OperationFlowState>> =
            Ok(RunOutcome::Completed(OperationFlowState::default()));
        assert!(
            paused_disposition(&completed).is_none(),
            "completed must NOT be treated as a pause"
        );

        let failed: GraphResult<RunOutcome<OperationFlowState>> = Ok(RunOutcome::Failed {
            state: OperationFlowState::default(),
            node: "enumeration".to_string(),
            error: "boom".to_string(),
        });
        assert!(
            paused_disposition(&failed).is_none(),
            "failed must NOT be treated as a resumable pause"
        );
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
            apply_harness_gate_hook(
                &p,
                &ctx,
                content.clone(),
                None,
                None,
                vec![],
                None,
                None,
                None,
            )
            .0,
            content
        );
    }

    #[test]
    fn no_harness_stage_skips_gate() {
        let p = planned_no_harness();
        let ctx = ExecutionContext::default();
        let content = "ignore me".to_string();
        assert_eq!(
            apply_harness_gate_hook(
                &p,
                &ctx,
                content.clone(),
                None,
                None,
                vec![],
                None,
                None,
                None,
            )
            .0,
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

    // PR1 (设计 2026-06-11-coverage-auto-derive §5.1 · 运输鲁棒性): the submit-tool
    // side-channel deliverable is appended at the END of the content. An earlier
    // explanatory / broken ```json block in the agent's prose must not shadow it —
    // the parser scans ALL fences, not just the first.
    #[test]
    fn parse_deliverable_skips_unparseable_fence_and_finds_later_one() {
        let deliverable = r#"{"stage_id":"target_intel","stage_run_id":"3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33","claims":[],"evidence_refs":[],"findings":[]}"#;
        let content = format!(
            "Analysis summary:\n```json\n{{\"note\": \"not a deliverable\"}}\n```\n\nsubmitted via tool:\n\n```json\n{deliverable}\n```"
        );
        let d = parse_deliverable_from_content(&content)
            .expect("trailing deliverable must be found despite an earlier non-deliverable fence");
        assert_eq!(d.stage_id, "target_intel");
    }

    // PR1 · multiple parseable deliverables → the LAST one wins (the side-channel
    // append happens last = the most recent submission supersedes earlier drafts).
    #[test]
    fn parse_deliverable_prefers_last_parseable_fence() {
        let d1 = r#"{"stage_id":"scoping","stage_run_id":"3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33","claims":[],"evidence_refs":[],"findings":[]}"#;
        let d2 = r#"{"stage_id":"target_intel","stage_run_id":"4b9b2c7e-2e5c-4f7b-8c3d-1a2e5f0c9d44","claims":[],"evidence_refs":[],"findings":[]}"#;
        let content = format!("```json\n{d1}\n```\ncorrected later:\n```json\n{d2}\n```");
        let d = parse_deliverable_from_content(&content).unwrap();
        assert_eq!(d.stage_id, "target_intel");
    }

    // 设计 2026-06-12-unified-refiner · 接线级：gate 的 reasons/recovery 事实经
    // outcome 进 Refiner 后，Generic 模板必须完整呈现（迁自旧 build_gate_correction 测试）。
    #[test]
    fn refiner_generic_correction_includes_reasons_and_recovery() {
        let mut o = missing_deliverable_gate_outcome(StageKind::Scoping, false).unwrap();
        o.missing_deliverable = false; // 模拟「交了但其它原因 BLOCK」
        o.gate_reasons = vec!["scope_status missing".to_string()];
        o.gate_recovery = Some(crate::harness::HarnessRecoveryActions {
            repair_tool_calls: vec!["dns_resolve".to_string()],
            missing_evidence_kinds: vec!["subdomain".to_string()],
            ..Default::default()
        });
        let d = crate::task_orchestrator::refiner::refine(&o.as_refine_input(None));
        assert!(d.correction.contains("REJECTED"));
        assert!(d.correction.contains("scope_status missing"));
        assert!(d.correction.contains("dns_resolve"));
        assert!(d.correction.contains("subdomain"));
    }

    #[test]
    fn command_hint_covers_passive_intel_provider_actions() {
        use crate::task_orchestrator::refiner::passive_intel_command_hint;
        assert!(passive_intel_command_hint("GOLISH-INTEL-DNS")
            .unwrap()
            .contains("recon_map_assets"));
        assert!(!passive_intel_command_hint("GOLISH-INTEL-DNS")
            .unwrap()
            .contains("dig"));
        let subdomain_hint = passive_intel_command_hint("GOLISH-INTEL-SUBDOMAIN").unwrap();
        assert!(subdomain_hint.contains("recon_map_assets"));
        assert!(!subdomain_hint.contains("subfinder"));
        assert!(passive_intel_command_hint("GOLISH-INTEL-WHOIS")
            .unwrap()
            .contains("recon_lookup_whois"));
        assert!(passive_intel_command_hint("GOLISH-INTEL-CT")
            .unwrap()
            .contains("recon_map_assets"));
        assert!(passive_intel_command_hint("GOLISH-INTEL-ASN").is_some());
        assert!(passive_intel_command_hint("GOLISH-INTEL-OSINT").is_some());
        assert!(
            passive_intel_command_hint("GOLISH-INTEL-BOGUS").is_none(),
            "unknown technique → None (no invented command)"
        );
    }

    #[test]
    fn db_truth_diagnosis_lists_found_only_and_none_when_empty() {
        use crate::harness::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
        use crate::task_orchestrator::refiner::build_db_truth_diagnosis;
        assert!(build_db_truth_diagnosis(&[]).is_none());
        let facts = vec![
            EvidenceFact {
                asset: "moresec.cn".to_string(),
                technique: "GOLISH-INTEL-DNS".to_string(),
                outcome: EvidenceOutcome::Found,
                evidence_id: 0,
            },
            EvidenceFact {
                asset: "moresec.cn".to_string(),
                technique: "GOLISH-INTEL-ASN".to_string(),
                outcome: EvidenceOutcome::Empty,
                evidence_id: 5,
            },
        ];
        let out = build_db_truth_diagnosis(&facts).unwrap();
        assert!(out.contains("moresec.cn") && out.contains("GOLISH-INTEL-DNS"));
        assert!(
            !out.contains("GOLISH-INTEL-ASN"),
            "Empty fact is not persisted data (I8)"
        );
    }

    #[test]
    fn refiner_coverage_block_appends_db_status_and_actions() {
        use crate::harness::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
        let mut o = missing_deliverable_gate_outcome(StageKind::TargetIntel, false).unwrap();
        o.missing_deliverable = false;
        o.gate_reasons = vec![
            "coverage incomplete: never attempted (moresec.cn × GOLISH-INTEL-DNS)".to_string(),
        ];
        let facts = vec![EvidenceFact {
            asset: "moresec.cn".to_string(),
            technique: "GOLISH-INTEL-SUBDOMAIN".to_string(),
            outcome: EvidenceOutcome::Found,
            evidence_id: 0,
        }];
        let d = crate::task_orchestrator::refiner::refine(&o.as_refine_input(Some(&facts)));
        assert!(d.correction.contains("DB truth status"), "DB 现状段");
        assert!(
            d.correction.contains("GOLISH-INTEL-SUBDOMAIN"),
            "列已 Found 的类"
        );
        assert!(
            d.correction.contains("Suggested next target_intel actions"),
            "下一步动作建议段"
        );
        assert!(d.correction.contains("recon_map_assets"));
        assert!(!d.correction.contains("dig"));
    }

    #[test]
    fn refiner_generic_block_has_no_diagnosis_sections() {
        let mut o = missing_deliverable_gate_outcome(StageKind::TargetIntel, false).unwrap();
        o.missing_deliverable = false;
        o.gate_reasons = vec!["finding count below minimum".to_string()];
        let d = crate::task_orchestrator::refiner::refine(&o.as_refine_input(None));
        assert!(!d.correction.contains("Suggested next target_intel actions"));
        assert!(!d.correction.contains("DB truth status"));
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
                technique: None,
            }],
            evidence_refs: vec![EvidenceAuditId::new(1), EvidenceAuditId::new(2)],
            skipped_checks: vec![],
            findings: vec![HarnessFinding {
                finding_id: uuid::Uuid::new_v4(),
                kind: "subdomain".to_string(),
                subject: "api.example.com".to_string(),
                severity: FindingSeverity::Info,
                evidence_refs: vec![EvidenceAuditId::new(1)],
                technique: None,
            }],
            required_checks_done: vec![],
            coverage: vec![],
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

    /// 测试用最小 outcome（设计 2026-06-12-unified-refiner 后 correction 由
    /// Refiner 渲染，构造处只填事实字段）。
    fn outcome_for_test(stage: StageKind, evidence_refs: Vec<i64>) -> HarnessGateOutcome {
        HarnessGateOutcome {
            gated_stage: stage,
            gate_allowed: true,
            engagement_org_id: None,
            repair_correction: None,
            evidence_summary: None,
            evidence_refs,
            required_evidence_kinds: Vec::new(),
            findings_count: 0,
            fabricated_evidence_refs: Vec::new(),
            available_real_ids: Vec::new(),
            missing_deliverable: false,
            gate_reasons: Vec::new(),
            gate_recovery: None,
            missing_kinds: Vec::new(),
            expired: Vec::new(),
            red_team_flow_correction: None,
            confirm_only_stage: false,
            evidence_kind_labels: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn block_outcome_for_fabricated_flips_pass_to_block() {
        // A PASS deliverable citing a fabricated id must flip to BLOCK; the
        // refiner's D-class template names the fabricated ids (plan acceptance
        // #2: fake refs get BLOCKed).
        let mut o = outcome_for_test(StageKind::ExternalAttackSurface, vec![1, 999]);
        block_outcome_for_fabricated(&mut o, &[999], &[]);
        assert!(!o.gate_allowed, "fabricated evidence must BLOCK the gate");
        let d = crate::task_orchestrator::refiner::refine(&o.as_refine_input(None));
        assert_eq!(
            d.class,
            crate::task_orchestrator::refiner::RefineClass::Fabricated
        );
        assert!(
            d.correction.contains("999"),
            "correction names the fabricated id"
        );
        assert!(d.correction.contains("do NOT exist"));
        // No real ids known → tell the agent to run the tools first.
        assert!(
            d.correction.contains("No real evidence ids exist"),
            "empty real-id set must instruct running tools first: {}",
            d.correction
        );
    }

    #[test]
    fn block_outcome_for_fabricated_names_real_ids_when_available() {
        // 甲 (root-cause fix): when the operation already has real evidence ids,
        // the correction must NAME them so the retry cites real ids instead of
        // re-copying the template placeholders.
        let mut o = outcome_for_test(StageKind::TargetIntel, vec![1, 2, 3]);
        block_outcome_for_fabricated(&mut o, &[1, 2, 3], &[86, 88, 90]);
        assert!(!o.gate_allowed);
        // Observability (design 2026-06-05): the fabricated/available ids are
        // captured onto the outcome so consume_gate_outcome can surface them in
        // the HarnessTrace GateDecision event.
        assert_eq!(o.fabricated_evidence_refs, vec![1, 2, 3]);
        assert_eq!(o.available_real_ids, vec![86, 88, 90]);
        let d = crate::task_orchestrator::refiner::refine(&o.as_refine_input(None));
        assert!(
            d.correction.contains("86")
                && d.correction.contains("88")
                && d.correction.contains("90"),
            "names real ids: {}",
            d.correction
        );
        assert!(
            d.correction.contains("REAL evidence ids"),
            "labels them as the real set: {}",
            d.correction
        );
        assert!(
            !d.correction.contains("No real evidence ids exist"),
            "must not also emit the empty-set instruction: {}",
            d.correction
        );
    }

    #[test]
    fn fabricated_takes_priority_over_other_block_reasons() {
        // 设计 2026-06-12-unified-refiner · 旧行为是链式 prepend（伪造通知 + 先前
        // 纠正叠加）；新行为是主因优先级——fabricated 压过其它原因，次因不再整段
        // 拼接（quality 类并存时由 secondary_note 一行附录兜住，见 refiner 单测）。
        let mut o = outcome_for_test(StageKind::ExternalAttackSurface, vec![5]);
        o.gate_allowed = false;
        o.gate_reasons = vec!["deliverable vacuous: no claims".to_string()];
        block_outcome_for_fabricated(&mut o, &[5], &[]);
        let d = crate::task_orchestrator::refiner::refine(&o.as_refine_input(None));
        assert_eq!(
            d.class,
            crate::task_orchestrator::refiner::RefineClass::Fabricated
        );
        assert!(
            d.correction.contains("do NOT exist"),
            "fabrication template wins as the primary correction"
        );
    }
}

// 设计 2026-06-12-unified-refiner (PR-R2/R3)：missing deliverable 一律 fail-closed
// BLOCK——投影兜底与 confirm-only 合成已删除，后端绝不代为合成 deliverable。
// DB 真值哨兵投影（PR-A，gate 判错侧）保留。
#[cfg(test)]
mod missing_deliverable_fail_closed_tests {
    use super::*;
    use crate::harness::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
    use crate::harness::{HarnessStageHint, StageKind};

    fn fact(asset: &str, technique: &str, outcome: EvidenceOutcome, id: i64) -> EvidenceFact {
        EvidenceFact {
            asset: asset.to_string(),
            technique: technique.to_string(),
            outcome,
            evidence_id: id,
        }
    }

    fn planned(stage_kind: StageKind) -> PlannedSubtask {
        PlannedSubtask {
            title: "t".to_string(),
            description: "d".to_string(),
            agent: None,
            harness_stage: Some(HarnessStageHint::new(stage_kind)),
            nl_slice: None,
            acceptance_criteria: vec![],
        }
    }

    // ── DB 业务表真值投影（设计 2026-06-12 §5.3，保留——gate 判错侧） ────────

    #[test]
    fn db_truth_to_evidence_maps_pairs_to_found_with_sentinel_id() {
        let pairs = vec![
            ("moresec.cn".to_string(), "GOLISH-INTEL-ASN".to_string()),
            (
                "moresec.cn".to_string(),
                "GOLISH-INTEL-SUBDOMAIN".to_string(),
            ),
        ];
        let facts = db_truth_facts_to_evidence(pairs);
        assert_eq!(facts.len(), 2);
        for f in &facts {
            assert_eq!(f.outcome, EvidenceOutcome::Found, "DB 投影只产 Found (I8)");
            assert_eq!(f.evidence_id, 0, "业务表 fact 用哨兵 id=0 (D2)");
        }
        assert_eq!(facts[0].asset, "moresec.cn");
        assert_eq!(facts[0].technique, "GOLISH-INTEL-ASN");
    }

    #[test]
    fn db_truth_to_evidence_empty_input_yields_empty() {
        assert!(db_truth_facts_to_evidence(vec![]).is_empty());
    }

    // ── missing deliverable = fail-closed BLOCK（一切 stage，无例外） ────────

    #[test]
    fn hook_blocks_missing_deliverable_even_with_ledger_facts() {
        // PR-R2 行为变化锚点：target_intel（旧投影兜底的灰度 stage）现在与其它
        // substantive stage 一致——账本有真证据也不投影，BLOCK 后由 Refiner 的
        // A 类 submit-only 锁驱动 agent 自己提交（live run 两连截胡的根治）。
        let ctx = ExecutionContext::default();
        for stage in [
            StageKind::TargetIntel,
            StageKind::ExternalAttackSurface,
            StageKind::VulnTriage,
        ] {
            let p = planned(stage);
            let facts = vec![fact("a", "GOLISH-INTEL-DNS", EvidenceOutcome::Found, 7)];
            let (out, outcome) = apply_harness_gate_hook(
                &p,
                &ctx,
                "prose".to_string(),
                None,
                None,
                vec![],
                Some(facts),
                None,
                None,
            );
            let o = outcome.expect("stage-tagged unparseable content must produce an outcome");
            assert!(!o.gate_allowed, "{stage:?} must fail-closed BLOCK");
            assert!(
                o.missing_deliverable,
                "{stage:?} must stay on the missing-deliverable path (refiner routes A/B)"
            );
            assert!(
                o.repair_correction.is_none(),
                "rendering belongs to the refiner, not the hook"
            );
            assert!(!o.confirm_only_stage, "{stage:?} is substantive");
            assert_eq!(out, "prose", "content passes through unchanged on BLOCK");
        }
    }

    #[test]
    fn hook_blocks_missing_deliverable_for_confirm_only_stage_too() {
        // PR-R3 行为变化锚点：scoping（confirm-only）不再由后端代填确认 claim——
        // BLOCK + confirm_only_stage 事实置位，Refiner 的 A 类 confirm-only 变体
        // 锁 submit 逼 agent 自己提交。
        let p = planned(StageKind::Scoping);
        let ctx = ExecutionContext::default();
        let (out, outcome) = apply_harness_gate_hook(
            &p,
            &ctx,
            "prose".to_string(),
            None,
            None,
            vec![],
            None,
            None,
            None,
        );
        let o = outcome.expect("stage-tagged unparseable content must produce an outcome");
        assert!(!o.gate_allowed, "confirm-only missing must BLOCK too");
        assert!(o.missing_deliverable);
        assert!(
            o.confirm_only_stage,
            "scoping must carry the confirm-only fact for the refiner's A-class variant"
        );
        let d = crate::task_orchestrator::refiner::refine(&o.as_refine_input(None));
        assert_eq!(
            d.class,
            crate::task_orchestrator::refiner::RefineClass::SubmitOnly,
            "confirm-only missing routes to submit-only"
        );
        assert!(
            d.submit_only_lock,
            "the retry must lock tool_choice to submit"
        );
        assert_eq!(out, "prose");
    }
}
