//! Single-subtask execution with enrichment, planning, and reflector retry.

use anyhow::Context;
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

const SCOPING_SCOPE_SNAPSHOT_ID_V1: &[u8] = b"golish:scoping-scope-snapshot:v1";
const SCOPING_ROOT_UNIT_ID_V1: &[u8] = b"golish:scoping-root-unit:v1";

/// The generic reflector bound and the request-scoped `stage_run` circuit
/// breaker are independent limits. A gate BLOCK may open another automatic
/// repair turn only while both still permit it. A later explicit user
/// continuation gets a new top-level request lease, so its stage-run guard is
/// reset before this policy is evaluated.
fn should_retry_gate_block(
    reflector_attempt: usize,
    stage_run_retry_budget_exhausted: bool,
) -> bool {
    reflector_attempt < MAX_REFLECTOR_RETRIES && !stage_run_retry_budget_exhausted
}

fn exact_candidate_v2_contracts(
    runtime: crate::runtime_memory::RuntimeMemoryContract,
    attack: golish_core::AttackExecutionContract,
) -> bool {
    runtime == crate::runtime_memory::RuntimeMemoryContract::V2Only
        && attack == golish_core::AttackExecutionContract::V2Only
}

fn candidate_v2_synthesis_contracts(
    runtime: crate::runtime_memory::RuntimeMemoryContract,
    attack: golish_core::AttackExecutionContract,
) -> bool {
    runtime != crate::runtime_memory::RuntimeMemoryContract::LegacyV1 && attack.writes_v2()
}

/// Resolve the specialist visible to the depth-0 stage coordinator. Candidate
/// synthesis becomes relational as soon as runtime memory can write V2 and the
/// frozen attack contract writes V2. Verification has no legacy specialist and
/// remains disabled until both immutable contracts are exactly V2Only.
fn effective_stage_run_specialist(
    stage: crate::harness::StageKind,
    configured: Option<&str>,
    exact_candidate_v2: bool,
) -> Option<String> {
    let configured = configured
        .map(str::trim)
        .filter(|specialist| !specialist.is_empty())
        .map(ToOwned::to_owned);
    match stage {
        crate::harness::StageKind::Verification => {
            exact_candidate_v2.then(|| "candidate_verifier".to_string())
        }
        crate::harness::StageKind::AttackCandidate if exact_candidate_v2 => {
            Some("attack_analyst".to_string())
        }
        _ => configured,
    }
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
        let reporting_stage = planned
            .harness_stage
            .as_ref()
            .is_some_and(|hint| hint.stage_kind == crate::harness::StageKind::Reporting);

        // Phase 1: Enrich — gather supplementary context
        let enrichment = if reporting_stage {
            // Reporting authority is the server-built canonical read model. Do
            // not let generic memory/wiki enrichment become report truth.
            None
        } else {
            match executor
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
            }
        };

        // Phase 2: Plan — generate execution checklist
        let execution_plan = if reporting_stage {
            None
        } else {
            match executor
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
            }
        };

        let augmented_description = {
            let mut desc = String::new();

            // C2 · harness stage charter (该 subtask 归属某 stage 时, 前置注入
            // 允许/禁止工具面 + deliverable/gate 要求).
            {
                if let Some(hint) = planned.harness_stage.as_ref() {
                    if let Ok(mut spec) = crate::harness::load_embedded_stage_spec(hint.stage_kind)
                    {
                        let exact_candidate_v2 = self
                            .candidate_v2_specialist_for_operation(
                                task_id,
                                hint.stage_kind,
                                "fresh_stage_prompt",
                            )
                            .await;
                        spec.specialist = effective_stage_run_specialist(
                            hint.stage_kind,
                            spec.specialist.as_deref(),
                            exact_candidate_v2,
                        );
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
                        if hint.stage_kind != crate::harness::StageKind::Reporting {
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
                    let stage_run_retry_budget_exhausted = exec_ctx
                        .harness_stage
                        .is_some_and(|stage| executor.stage_run_retry_budget_exhausted(stage));
                    if should_retry_gate_block(reflector_attempt, stage_run_retry_budget_exhausted)
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

                    let in_scope_assets =
                        self.fetch_in_scope_assets_for_gate(planned, task_id).await;
                    let asset_types = self.fetch_in_scope_typed_assets_for_gate(planned).await;
                    let web_capable_assets = self
                        .fetch_web_capable_assets_for_gate(planned, task_id)
                        .await;
                    let not_applicable_coverage = self
                        .fetch_not_applicable_coverage_for_gate(planned, task_id)
                        .await;
                    let in_scope_target_types =
                        self.fetch_in_scope_target_types_for_gate(planned).await;
                    let evidence_facts = self
                        .fetch_evidence_facts_for_gate(planned, in_scope_assets.as_deref(), task_id)
                        .await;
                    let source_queries = self.fetch_source_queries_for_gate(planned).await;
                    let reporting_truth =
                        self.fetch_reporting_truth_for_gate(planned, task_id).await;
                    // 设计 2026-06-12-unified-refiner · Refiner C 类诊断与 gate 用
                    // 同一份证据事实；hook move 走原值，这里留一份给渲染。
                    let refine_facts = evidence_facts.clone();
                    // Phase 1.5: fan-out 阶段收尾改判 stage_run pass_token（B-recompute），
                    // 跳过整阶段 coverage；非 fan-out / 不可解析交付物走常规 gate。
                    let mut specialist_gated = false;
                    let trusted_submission = agent_result.captured_stage_submission.clone();
                    let (gated_content, gate_outcome) = if let Some(res) = self
                        .try_specialist_stage_gate(planned, &agent_result.content, task_id)
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
                            web_capable_assets,
                            not_applicable_coverage,
                            in_scope_target_types,
                            evidence_facts,
                            source_queries,
                            reporting_truth,
                            self.harness_subsidiary_policy.map(|p| p.threshold_pct),
                        )
                    };
                    if let Some(mut outcome) = gate_outcome {
                        outcome.trusted_submission = trusted_submission;
                        // Fan-out aggregate closeout is DB-authoritative: every
                        // org unit already owns an immutable worker submission,
                        // and try_specialist_stage_gate recomputes the token from
                        // current-operation completion rows. Requiring another
                        // unit-bound submission from the unit-less coordinator
                        // would reject the intended identity shape.
                        if !specialist_gated {
                            self.enforce_trusted_submission(&mut outcome, exec_ctx)
                                .await;
                        }
                        self.enforce_cleanup_closeout_gate(task_id, &mut outcome)
                            .await;
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
                            if should_retry_gate_block(
                                reflector_attempt,
                                stage_run_retry_budget_exhausted,
                            ) {
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
                            } else if stage_run_retry_budget_exhausted {
                                tracing::info!(
                                    target: "harness::hook",
                                    stage = %outcome.gated_stage.as_str(),
                                    "stage_run retry budget exhausted in this top-level request; stopping automatic gate-repair turns"
                                );
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

        let fallback_result = last_result.unwrap_or_else(|| {
            AgentResult::new("Subtask completed without tool usage.".to_string())
        });
        let trusted_submission = fallback_result.captured_stage_submission.clone();
        let fallback = fallback_result.content;
        // Loop exhausted: run the gate once on the fallback content (no further
        // retry possible) and drive the transition on whatever it decides.
        let in_scope_assets = self.fetch_in_scope_assets_for_gate(planned, task_id).await;
        let asset_types = self.fetch_in_scope_typed_assets_for_gate(planned).await;
        let web_capable_assets = self
            .fetch_web_capable_assets_for_gate(planned, task_id)
            .await;
        let not_applicable_coverage = self
            .fetch_not_applicable_coverage_for_gate(planned, task_id)
            .await;
        let in_scope_target_types = self.fetch_in_scope_target_types_for_gate(planned).await;
        let evidence_facts = self
            .fetch_evidence_facts_for_gate(planned, in_scope_assets.as_deref(), task_id)
            .await;
        let source_queries = self.fetch_source_queries_for_gate(planned).await;
        let reporting_truth = self.fetch_reporting_truth_for_gate(planned, task_id).await;
        let refine_facts = evidence_facts.clone();
        // Phase 1.5: fan-out 阶段收尾改判 stage_run pass_token；非 fan-out 走常规 gate。
        let mut specialist_gated = false;
        let (out, gate_outcome) = if let Some(res) = self
            .try_specialist_stage_gate(planned, &fallback, task_id)
            .await
        {
            specialist_gated = true;
            res
        } else {
            apply_harness_gate_hook(
                planned,
                exec_ctx,
                fallback,
                in_scope_assets,
                asset_types,
                web_capable_assets,
                not_applicable_coverage,
                in_scope_target_types,
                evidence_facts,
                source_queries,
                reporting_truth,
                self.harness_subsidiary_policy.map(|p| p.threshold_pct),
            )
        };
        if let Some(mut outcome) = gate_outcome {
            outcome.trusted_submission = trusted_submission;
            if !specialist_gated {
                self.enforce_trusted_submission(&mut outcome, exec_ctx)
                    .await;
            }
            self.enforce_cleanup_closeout_gate(task_id, &mut outcome)
                .await;
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

    /// V2 stage close is authorized by the immutable submission row, never by
    /// a parseable JSON fence alone. Legacy operations retain the compatibility
    /// path; every V2-writing contract must round-trip the bridge capture through
    /// the scoped repository before the gate can remain PASS.
    async fn enforce_trusted_submission(
        &self,
        outcome: &mut HarnessGateOutcome,
        exec_ctx: &ExecutionContext,
    ) {
        use sha2::{Digest, Sha256};

        let Some(operation_id) = exec_ctx.operation_id else {
            return;
        };
        let contract = match self
            .runtime_repo
            .runtime_memory_contract_for_operation(operation_id)
            .await
        {
            Ok(contract) => contract,
            Err(error) => {
                outcome.gate_allowed = false;
                outcome.gate_reasons.push(format!(
                    "trusted runtime contract lookup failed at stage close: {error}"
                ));
                return;
            }
        };
        if contract == crate::runtime_memory::RuntimeMemoryContract::LegacyV1 {
            return;
        }

        let Some(captured) = outcome.trusted_submission.as_ref() else {
            outcome.gate_allowed = false;
            outcome.gate_reasons.push(
                "V2 stage close requires a durable deliverable submission; a prose/legacy JSON capture is not authoritative."
                    .to_string(),
            );
            return;
        };
        if captured.operation_id != operation_id
            || Some(captured.stage_execution_id) != exec_ctx.stage_execution_id
            || captured.stage_run_unit_id != exec_ctx.stage_run_unit_id
        {
            outcome.gate_allowed = false;
            outcome.gate_reasons.push(
                "trusted deliverable submission does not belong to the active operation/execution/unit."
                    .to_string(),
            );
            return;
        }
        let actual_sha = Sha256::digest(captured.canonical_deliverable_json.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if actual_sha != captured.payload_sha256 {
            outcome.gate_allowed = false;
            outcome
                .gate_reasons
                .push("trusted deliverable capture hash mismatch.".to_string());
            return;
        }
        let canonical_payload =
            match serde_json::from_str::<serde_json::Value>(&captured.canonical_deliverable_json) {
                Ok(payload) => payload,
                Err(error) => {
                    outcome.gate_allowed = false;
                    outcome.gate_reasons.push(format!(
                        "trusted deliverable capture is invalid canonical JSON: {error}"
                    ));
                    return;
                }
            };
        let persisted = match self
            .runtime_repo
            .load_stage_deliverable_submission(
                captured.deliverable_submission_id,
                operation_id,
                captured.stage_execution_id,
            )
            .await
        {
            Ok(Some(persisted)) => persisted,
            Ok(None) => {
                outcome.gate_allowed = false;
                outcome
                    .gate_reasons
                    .push("trusted deliverable submission row is missing.".to_string());
                return;
            }
            Err(error) => {
                outcome.gate_allowed = false;
                outcome.gate_reasons.push(format!(
                    "trusted deliverable submission reload failed: {error}"
                ));
                return;
            }
        };
        if persisted.operation_id != operation_id
            || persisted.stage_execution_id != captured.stage_execution_id
            || persisted.stage_run_unit_id != captured.stage_run_unit_id
            || persisted.payload_sha256 != captured.payload_sha256
            || persisted.payload != canonical_payload
            || persisted.stage_kind != outcome.gated_stage.as_str()
        {
            outcome.gate_allowed = false;
            outcome.gate_reasons.push(
                "trusted deliverable submission changed or failed its scoped identity reload."
                    .to_string(),
            );
        }
    }

    /// Exact Verification execution and truth are enabled only when both
    /// operation-frozen rollout contracts have reached V2Only.
    async fn exact_candidate_v2_operation(&self, operation_id: Uuid) -> Result<bool, String> {
        let runtime = self
            .runtime_repo
            .runtime_memory_contract_for_operation(operation_id)
            .await
            .map_err(|error| format!("runtime contract lookup failed: {error}"))?;
        let attack = self
            .runtime_repo
            .attack_execution_contract_for_operation(operation_id)
            .await
            .map_err(|error| format!("attack contract lookup failed: {error}"))?;
        Ok(exact_candidate_v2_contracts(runtime, attack))
    }

    async fn candidate_v2_synthesis_operation(&self, operation_id: Uuid) -> Result<bool, String> {
        let runtime = self
            .runtime_repo
            .runtime_memory_contract_for_operation(operation_id)
            .await
            .map_err(|error| format!("runtime contract lookup failed: {error}"))?;
        let attack = self
            .runtime_repo
            .attack_execution_contract_for_operation(operation_id)
            .await
            .map_err(|error| format!("attack contract lookup failed: {error}"))?;
        Ok(candidate_v2_synthesis_contracts(runtime, attack))
    }

    /// Read the operation-frozen Candidate rollout pair for a coordinator seam.
    /// A lookup failure on a Candidate stage fails closed to the specialist-only
    /// route: `stage_run` will repeat the authoritative read and refuse dispatch
    /// if the contracts still cannot be loaded, instead of allowing direct work
    /// to bypass the immutable scheduler.
    async fn candidate_v2_specialist_for_operation(
        &self,
        operation_id: Uuid,
        stage: crate::harness::StageKind,
        seam: &'static str,
    ) -> bool {
        if !matches!(
            stage,
            crate::harness::StageKind::AttackCandidate | crate::harness::StageKind::Verification
        ) {
            return false;
        }
        let enabled = match stage {
            crate::harness::StageKind::AttackCandidate => {
                self.candidate_v2_synthesis_operation(operation_id).await
            }
            crate::harness::StageKind::Verification => {
                self.exact_candidate_v2_operation(operation_id).await
            }
            _ => return false,
        };
        match enabled {
            Ok(enabled) => enabled,
            Err(error) => {
                tracing::warn!(
                    target: "harness::hook",
                    operation_id = %operation_id,
                    stage = %stage.as_str(),
                    seam,
                    %error,
                    "Candidate contract lookup failed; keeping the primary on the specialist-only route"
                );
                true
            }
        }
    }

    async fn resolve_stage_flow_outcome(
        &mut self,
        task_id: Uuid,
        outcome: &HarnessGateOutcome,
    ) -> (
        crate::harness::operation_flow::StageFlowOutcome,
        Option<String>,
    ) {
        use crate::harness::operation_flow::StageFlowOutcome;

        let ordinary = || StageFlowOutcome {
            gate_allowed: outcome.gate_allowed,
            made_progress: gate_outcome_made_progress(outcome),
            reopen_wave: false,
            durable_wave_cursor: false,
        };
        if outcome.gated_stage == crate::harness::StageKind::Scoping && outcome.gate_allowed {
            if let Err(reason) = self
                .finalize_scoping_pass_if_v2_writing(task_id, outcome)
                .await
            {
                return (StageFlowOutcome::blocked(), Some(reason));
            }
        }
        if outcome.gated_stage != crate::harness::StageKind::Verification {
            return (ordinary(), None);
        }

        let exact_v2 = match self.exact_candidate_v2_operation(task_id).await {
            Ok(enabled) => enabled,
            Err(error) => {
                return (
                    StageFlowOutcome::blocked(),
                    Some(format!(
                        "Verification flow could not resolve persisted Candidate V2 contracts: {error}"
                    )),
                );
            }
        };
        if !exact_v2 {
            let mut flow = ordinary();
            if outcome.gate_allowed {
                match crate::harness::chain_wave::decide_chain_wave(
                    &outcome.spawned_candidates,
                    &self.chain_wave_seen,
                    self.chain_wave,
                    crate::harness::chain_wave::DEFAULT_MAX_WAVES,
                    crate::harness::chain_wave::DEFAULT_MAX_CHAIN_DEPTH,
                ) {
                    crate::harness::chain_wave::WaveDecision::OpenNextWave { next_wave } => {
                        for candidate in &outcome.spawned_candidates {
                            self.chain_wave_seen
                                .insert(crate::harness::chain_wave::candidate_dedup_key(candidate));
                        }
                        self.chain_wave = next_wave;
                        flow.reopen_wave = true;
                        tracing::info!(
                            target: "harness::hook",
                            task_id = %task_id,
                            wave = next_wave,
                            new_candidates = outcome.spawned_candidates.len(),
                            "legacy chain-wave opened next attack_candidate wave"
                        );
                    }
                    crate::harness::chain_wave::WaveDecision::Advance => {}
                }
            }
            return (flow, None);
        }
        if !outcome.gate_allowed {
            return (StageFlowOutcome::blocked(), None);
        }

        let truth = match self
            .repo
            .attack_v2_verification_truth_for_operation(task_id, None)
            .await
        {
            Ok(Some(truth)) => truth,
            Ok(None) => {
                return (
                    StageFlowOutcome::blocked(),
                    Some("V2Only Verification flow is missing exact DB truth".to_string()),
                );
            }
            Err(error) => {
                return (
                    StageFlowOutcome::blocked(),
                    Some(format!(
                        "Verification flow could not reload exact DB truth: {error}"
                    )),
                );
            }
        };
        if let Err(error) =
            crate::harness::attack_execution::validate_verification_truth_set(&truth)
        {
            return (
                StageFlowOutcome::blocked(),
                Some(format!(
                    "Verification flow rejected inconsistent exact DB truth: {error}"
                )),
            );
        }

        let consolidated = match self
            .repo
            .attack_v2_consolidate_wave(crate::db_traits::AttackV2ConsolidateWave {
                operation_id: task_id,
                scope_snapshot_id: truth.authority.scope_snapshot_id,
                source_wave_run_id: truth.authority.wave_run_id,
            })
            .await
        {
            Ok(result) => result,
            Err(error) => {
                return (
                    StageFlowOutcome::blocked(),
                    Some(format!("Verification Wave consolidation failed: {error}")),
                );
            }
        };
        if consolidated.operation_id != task_id
            || consolidated.scope_snapshot_id != truth.authority.scope_snapshot_id
            || consolidated.source_wave_run_id != truth.authority.wave_run_id
        {
            return (
                StageFlowOutcome::blocked(),
                Some("Verification Wave consolidation returned mismatched authority".to_string()),
            );
        }
        let exact = crate::harness::operation_flow::exact_verification_flow_outcome(&truth);
        let (reopen_wave, made_progress, pending_enrichment_block) =
            match consolidated.decision_kind.as_str() {
                "opened_next_wave"
                    if consolidated.target_wave_run_id.is_some()
                        && consolidated.accepted_fact_delta_count > 0
                        && consolidated.residual_risk_count == 0
                        && consolidated.pending_enrichment_count == 0 =>
                {
                    (true, true, false)
                }
                "closed_no_delta"
                    if consolidated.target_wave_run_id.is_none()
                        && consolidated.residual_risk_count == 0
                        && consolidated.pending_enrichment_count == 0 =>
                {
                    // Accepted refutations are real FactDelta truth but deliberately
                    // create no attack WorkItem, so the accepted count may be > 0.
                    (false, exact.made_progress, false)
                }
                "pending_enrichment"
                    if consolidated.target_wave_run_id.is_none()
                        && consolidated.pending_enrichment_count > 0
                        && consolidated.accepted_fact_delta_count
                            >= consolidated.pending_enrichment_count
                        && consolidated.residual_risk_count == 0 =>
                {
                    (false, false, true)
                }
                "exhausted"
                    if consolidated.target_wave_run_id.is_none()
                        && consolidated.pending_enrichment_count == 0 =>
                {
                    (false, false, false)
                }
                _ => {
                    return (
                        StageFlowOutcome::blocked(),
                        Some(format!(
                            "Verification Wave consolidation returned invalid decision '{}'",
                            consolidated.decision_kind
                        )),
                    );
                }
            };
        let accepted_fact_delta_count = match u32::try_from(consolidated.accepted_fact_delta_count)
        {
            Ok(count) => count,
            Err(_) => {
                return (
                    StageFlowOutcome::blocked(),
                    Some("Verification Wave consolidation count overflow".to_string()),
                );
            }
        };
        let rejected_fact_delta_count = match u32::try_from(consolidated.rejected_fact_delta_count)
        {
            Ok(count) => count,
            Err(_) => {
                return (
                    StageFlowOutcome::blocked(),
                    Some("Verification Wave consolidation count overflow".to_string()),
                );
            }
        };
        let residual_risk_count = match u32::try_from(consolidated.residual_risk_count) {
            Ok(count) => count,
            Err(_) => {
                return (
                    StageFlowOutcome::blocked(),
                    Some("Verification Wave consolidation count overflow".to_string()),
                );
            }
        };
        let pending_enrichment_count = match u32::try_from(consolidated.pending_enrichment_count) {
            Ok(count) => count,
            Err(_) => {
                return (
                    StageFlowOutcome::blocked(),
                    Some("Verification Wave consolidation count overflow".to_string()),
                );
            }
        };
        self.emit(AiEvent::HarnessTrace {
            operation_id: task_id.to_string(),
            stage: outcome.gated_stage.as_str().to_string(),
            agent_path: "main".to_string(),
            trace: golish_core::events::HarnessTraceKind::AttackWaveConsolidated {
                scope_snapshot_id: consolidated.scope_snapshot_id.to_string(),
                consolidation_id: consolidated.consolidation_id.to_string(),
                source_wave_run_id: consolidated.source_wave_run_id.to_string(),
                target_wave_run_id: consolidated.target_wave_run_id.map(|id| id.to_string()),
                decision_kind: consolidated.decision_kind,
                accepted_fact_delta_count,
                rejected_fact_delta_count,
                residual_risk_count,
                pending_enrichment_count,
                replayed: consolidated.replayed,
            },
        });

        if pending_enrichment_block {
            return (
                StageFlowOutcome::blocked(),
                Some(format!(
                    "ATTACK_FACT_DELTA_ENRICHMENT_REQUIRED: {pending_enrichment_count} accepted FactDelta item(s) still require a classifier-supported typed observation; the source Wave remains open"
                )),
            );
        }

        (
            StageFlowOutcome {
                gate_allowed: exact.gate_allowed,
                made_progress,
                reopen_wave,
                durable_wave_cursor: true,
            },
            None,
        )
    }

    /// A V2-writing Scoping PASS is not authoritative until the approved org
    /// scope, trusted submission binding, and synthetic root unit are frozen in
    /// one repository transaction. This runs before the GateDecision event and
    /// `stage_passed`, so any lookup, write, or identity mismatch turns the
    /// otherwise-positive gate into a deterministic BLOCK.
    async fn finalize_scoping_pass_if_v2_writing(
        &self,
        task_id: Uuid,
        outcome: &HarnessGateOutcome,
    ) -> Result<(), String> {
        use crate::runtime_memory::RuntimeMemoryWriteStrategy;

        let contract = self
            .runtime_repo
            .runtime_memory_contract_for_operation(task_id)
            .await
            .map_err(|error| {
                format!("Scoping finalization could not resolve the runtime contract: {error}")
            })?;
        if contract.policy().write == RuntimeMemoryWriteStrategy::LegacyOnly {
            return Ok(());
        }

        let submission = outcome.trusted_submission.as_ref().ok_or_else(|| {
            "V2-writing Scoping finalization requires the trusted deliverable submission."
                .to_string()
        })?;
        if submission.operation_id != task_id {
            return Err(
                "V2-writing Scoping finalization received a submission from another operation."
                    .to_string(),
            );
        }
        let root_organization_id = outcome.engagement_org_id.ok_or_else(|| {
            "V2-writing Scoping finalization requires the gate-approved root organization id."
                .to_string()
        })?;
        let operation = crate::db_shim::operation_state::get(&*self.repo, task_id)
            .await
            .map_err(|error| {
                format!("Scoping finalization could not load operation state: {error}")
            })?
            .ok_or_else(|| "Scoping finalization is missing operation state.".to_string())?;
        if operation.operation_id != task_id {
            return Err(
                "Scoping finalization loaded operation state with mismatched identity.".to_string(),
            );
        }
        if operation
            .engagement_org_id
            .is_some_and(|trusted_org| trusted_org != root_organization_id)
        {
            return Err(
                "V2-writing Scoping finalization root organization does not match the operation's trusted organization binding."
                    .to_string(),
            );
        }
        let project_scope_id = operation.project_scope_id.ok_or_else(|| {
            "V2-writing Scoping finalization requires a durable project scope id.".to_string()
        })?;
        let input = crate::db_traits::FinalizeScopingScope {
            operation_id: task_id,
            project_scope_id,
            stage_execution_id: submission.stage_execution_id,
            root_organization_id,
            deliverable_submission_id: submission.deliverable_submission_id,
            scope_snapshot_id: Uuid::new_v5(
                &submission.deliverable_submission_id,
                SCOPING_SCOPE_SNAPSHOT_ID_V1,
            ),
            scoping_root_unit_id: Uuid::new_v5(
                &submission.deliverable_submission_id,
                SCOPING_ROOT_UNIT_ID_V1,
            ),
        };
        let finalized = self
            .runtime_repo
            .finalize_scoping_scope(input.clone())
            .await
            .map_err(|error| format!("V2-writing Scoping finalization failed: {error}"))?;
        if finalized.operation_id != input.operation_id
            || finalized.project_scope_id != input.project_scope_id
            || finalized.stage_execution_id != input.stage_execution_id
            || finalized.root_organization_id != input.root_organization_id
            || finalized.deliverable_submission_id != input.deliverable_submission_id
            || finalized.scope_snapshot_id != input.scope_snapshot_id
            || finalized.scoping_root_unit_id != input.scoping_root_unit_id
        {
            return Err(
                "V2-writing Scoping finalization returned mismatched authority identity."
                    .to_string(),
            );
        }

        tracing::info!(
            target: "harness::hook",
            operation_id = %task_id,
            deliverable_submission_id = %submission.deliverable_submission_id,
            scope_snapshot_id = %finalized.scope_snapshot_id,
            scoping_root_unit_id = %finalized.scoping_root_unit_id,
            replayed = finalized.replayed,
            "V2-writing Scoping scope finalized before gate publication"
        );
        Ok(())
    }

    /// Post-gate handling for the Executor-driven stage loop, shared by both gate
    /// sites in [`Self::execute_single_subtask`].
    ///
    /// On PASS, record the stage's deliverable summary for cross-stage handoff,
    /// then accumulate the flow outcome (gate ANDed, progress ORed across the
    /// stage's subtasks) into [`Self::stage_outcome_acc`] for `run_stage_subtasks`
    /// to report to the graph (which owns the actual stage transition).
    async fn consume_gate_outcome(&mut self, task_id: Uuid, outcome: HarnessGateOutcome) {
        let (flow, post_gate_block_reason) =
            self.resolve_stage_flow_outcome(task_id, &outcome).await;
        // G · observability: log every stage gate decision at the single chokepoint
        // both gate sites flow through (the loop only accumulates into
        // `stage_outcome_acc`, so without this its PASS/BLOCK decisions would be
        // invisible in the logs). Pure additive INFO — no behaviour change.
        tracing::info!(
            target: "harness::hook",
            task_id = %task_id,
            stage = %outcome.gated_stage.as_str(),
            gate = if flow.gate_allowed { "PASS" } else { "BLOCK" },
            findings = outcome.findings_count,
            "gate decision"
        );
        // Observability (design 2026-06-05): the gate decision as a first-class
        // event so it lands in the transcript timeline next to the deliverable's
        // tool result (BLOCK was previously tracing-only → invisible to any AI
        // reconstructing the run). `agent_path = "main"`: the gate runs in the
        // orchestrator. `operation_id` = task id (the harness operation).
        let first_blocking_reason = if flow.gate_allowed {
            None
        } else {
            post_gate_block_reason.or_else(|| {
                outcome
                    .repair_correction
                    .as_deref()
                    .map(|s| s.lines().next().unwrap_or(s).to_string())
            })
        };
        self.emit(AiEvent::HarnessTrace {
            operation_id: task_id.to_string(),
            stage: outcome.gated_stage.as_str().to_string(),
            agent_path: "main".to_string(),
            trace: golish_core::events::HarnessTraceKind::GateDecision {
                gate: if flow.gate_allowed { "PASS" } else { "BLOCK" }.to_string(),
                findings: outcome.findings_count as u32,
                fabricated_evidence_refs: outcome.fabricated_evidence_refs.clone(),
                available_real_ids: outcome.available_real_ids.clone(),
                first_blocking_reason,
            },
        });
        if flow.gate_allowed {
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
        self.stage_outcome_acc = Some(match self.stage_outcome_acc.take() {
            Some(prev) => crate::harness::operation_flow::StageFlowOutcome {
                gate_allowed: prev.gate_allowed && flow.gate_allowed,
                made_progress: prev.made_progress || flow.made_progress,
                reopen_wave: prev.reopen_wave || flow.reopen_wave,
                durable_wave_cursor: prev.durable_wave_cursor || flow.durable_wave_cursor,
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
        request_input_override: Option<&str>,
        expected_initial_stage_execution_id: Option<Uuid>,
    ) -> anyhow::Result<String> {
        use crate::harness::operation_flow::{
            build_runner_graph, ChannelStageRunner, OperationFlowState, StageRunRequest,
        };

        let operation = crate::db_shim::operation_state::get(&*self.repo, task_id)
            .await
            .context("load runtime operation before executor-driven run")?
            .ok_or_else(|| anyhow::anyhow!("runtime operation {task_id} is missing"))?;
        let active_stage_execution = self
            .runtime_repo
            .active_stage_execution(task_id)
            .await
            .map_err(anyhow::Error::new)
            .context("load exact active stage execution before executor-driven run")?;
        anyhow::ensure!(
            active_stage_execution.operation_id == task_id,
            "active stage execution belongs to a different operation"
        );
        anyhow::ensure!(
            active_stage_execution.status
                == crate::task_orchestrator::stage_execution::StageExecutionStatus::Started,
            "active stage execution is not started"
        );
        anyhow::ensure!(
            active_stage_execution.stage.as_str() == operation.current_stage,
            "operation cursor and active stage execution disagree"
        );
        if let Some(expected_id) = expected_initial_stage_execution_id {
            anyhow::ensure!(
                active_stage_execution.id == expected_id,
                "atomic create returned an initial stage execution that is not active"
            );
        }
        let (op_max_authz, op_profile_id) =
            match crate::harness::load_embedded_profile(&operation.profile) {
                Ok(Some(p)) => (Some(p.max_authorization), Some(operation.profile.clone())),
                _ => (None, None),
            };

        let durable_task_input = crate::db_shim::tasks::get(&*self.repo, task_id)
            .await
            .ok()
            .flatten()
            .map(|t| t.input)
            .unwrap_or_default();
        // Keep the task row as the durable original operation objective, but let
        // a resumed top-level request steer this one execution. This is the seam
        // that later becomes Bridge `SubAgentContext.original_request` and the
        // bounded operator-constraint block in each stage_run worker objective.
        // Blank continuation input deliberately preserves the historical
        // behavior by falling back to the durable original.
        let task_input =
            resolve_request_local_task_input(durable_task_input, request_input_override);
        // Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): bind
        // this operation to its scoping-confirmed engagement root org. Prefer the
        // explicitly-set id (CLI seed path) and persist it to operation_state for
        // resume; otherwise recover the previously-persisted id (resume path).
        // `None` means no binding yet. Org-keyed reads and active-recon entry
        // both fail closed until Scoping establishes the engagement root.
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
            None => operation.engagement_org_id,
        };
        let mut exec_ctx = ExecutionContext {
            operation_id: Some(task_id),
            stage_execution_id: Some(active_stage_execution.id),
            stage_run_unit_id: None,
            worker_lease: None,
            completed_results: Vec::new(),
            task_input,
            current_subtask: None,
            planned_subtasks: Vec::new(),
            harness_stage: None,
            harness_authz: None,
            harness_profile_id: op_profile_id.clone(),
            harness_submit_only: false,
            harness_forced_tool: None,
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
        let mut checkpointer = crate::task_orchestrator::stage_execution::DbFlowCheckpointer::new(
            self.repo.clone(),
            task_id,
        );
        if let Some(source) = self.resume_runtime_memory_source {
            checkpointer = checkpointer.with_selected_resume_source(source);
        }
        let checkpointer = std::sync::Arc::new(checkpointer);
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
                    // Defense in depth for a direct CLI stage slice (or any
                    // restored cursor) whose entry node is already EAS. The
                    // normal full-profile path checks the same invariant before
                    // TargetIntel crosses into EAS; checking again here closes
                    // the direct-entry bypass and the TOCTOU gap. Return a
                    // blocked flow before rotating stage identity or invoking
                    // the executor so the graph persists a resumable Interrupt.
                    if req.stage == crate::harness::StageKind::ExternalAttackSurface
                        && !self.active_recon_trusted_target_ready(task_id).await
                    {
                        let _ = req.reply.send(
                            crate::harness::operation_flow::StageFlowOutcome::blocked(),
                        );
                        continue;
                    }
                    // Stage work is permitted only after the exact durable
                    // execution identity has been loaded or atomically rotated.
                    // Propagating this error drops the request without invoking
                    // the stage body, so a failed transition cannot execute under
                    // the previous stage's identity.
                    self.sync_stage_execution_on_entry(&mut exec_ctx, task_id, req.stage).await?;
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

        match &final_outcome {
            Ok(crate::harness::graph_engine::RunOutcome::Completed(_)) => {}
            Ok(crate::harness::graph_engine::RunOutcome::Failed { node, error, .. }) => {
                anyhow::bail!("graph-flow failed at {node}: {error}");
            }
            Ok(crate::harness::graph_engine::RunOutcome::Interrupted { .. }) => {
                unreachable!("interrupted graph-flow returned before terminal completion")
            }
            Err(error) => anyhow::bail!("graph-flow executor failed: {error}"),
        }

        let report = match executor.generate_report(&exec_ctx).await {
            Ok(r) => r.content,
            Err(e) => {
                tracing::warn!("graph-flow reporter failed, using summary: {}", e);
                exec_ctx.summary()
            }
        };
        let terminal_stage = exec_ctx
            .harness_stage
            .context("completed graph-flow has no terminal stage identity")?;
        let terminal_stage_execution_id = exec_ctx
            .stage_execution_id
            .context("completed graph-flow has no terminal stage execution identity")?;
        let completed = self
            .runtime_repo
            .complete_terminal_stage_execution(
                crate::task_orchestrator::stage_execution::CompleteTerminalStageExecution {
                    operation_id: task_id,
                    current_stage_execution_id: terminal_stage_execution_id,
                    terminal_stage,
                    task_result: report.clone(),
                },
            )
            .await
            .map_err(anyhow::Error::new)
            .context("atomically complete terminal stage execution and task")?;
        anyhow::ensure!(
            completed.id == terminal_stage_execution_id
                && completed.operation_id == task_id
                && completed.stage == terminal_stage
                && completed.status
                    == crate::task_orchestrator::stage_execution::StageExecutionStatus::Completed,
            "repository returned an invalid terminal stage completion"
        );
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

    /// Bind stage entry to the exact durable execution identity. A same-stage
    /// resume retains the active id and freshness anchor; a real stage change is
    /// one atomic close/open/cursor transition. Context changes happen only after
    /// the repository returns and validates the complete transition.
    async fn sync_stage_execution_on_entry(
        &self,
        exec_ctx: &mut ExecutionContext,
        operation_id: Uuid,
        stage: crate::harness::StageKind,
    ) -> anyhow::Result<()> {
        use crate::task_orchestrator::stage_execution::{
            StageExecutionStatus, TransitionStageExecution,
        };

        anyhow::ensure!(
            exec_ctx.operation_id == Some(operation_id),
            "execution context operation identity does not match stage entry"
        );
        let active = self
            .runtime_repo
            .active_stage_execution(operation_id)
            .await
            .map_err(anyhow::Error::new)
            .context("load exact active stage execution on stage entry")?;
        anyhow::ensure!(
            active.operation_id == operation_id && active.status == StageExecutionStatus::Started,
            "repository returned an invalid active stage execution"
        );
        anyhow::ensure!(
            exec_ctx.stage_execution_id == Some(active.id),
            "execution context stage identity is stale"
        );

        if active.stage == stage {
            tracing::debug!(
                target: "harness::hook",
                operation_id = %operation_id,
                stage_execution_id = %active.id,
                stage = %stage.as_str(),
                "graph-flow: retaining same-stage execution identity"
            );
            return Ok(());
        }

        let next_stage_execution_id = Uuid::new_v4();
        let transitioned = self
            .runtime_repo
            .transition_stage_execution(TransitionStageExecution {
                operation_id,
                current_stage_execution_id: active.id,
                next_stage_execution_id,
                next_stage: stage,
            })
            .await
            .map_err(anyhow::Error::new)
            .context("atomically transition stage execution on stage entry")?;
        anyhow::ensure!(
            transitioned.previous.id == active.id
                && transitioned.previous.operation_id == operation_id
                && transitioned.previous.stage == active.stage
                && transitioned.previous.status == StageExecutionStatus::Completed,
            "repository returned an invalid previous stage execution"
        );
        anyhow::ensure!(
            transitioned.current.id == next_stage_execution_id
                && transitioned.current.operation_id == operation_id
                && transitioned.current.stage == stage
                && transitioned.current.status == StageExecutionStatus::Started,
            "repository returned an invalid current stage execution"
        );

        exec_ctx.stage_execution_id = Some(transitioned.current.id);
        exec_ctx.stage_run_unit_id = None;
        exec_ctx.worker_lease = None;
        tracing::info!(
            target: "harness::hook",
            operation_id = %operation_id,
            previous_stage_execution_id = %transitioned.previous.id,
            stage_execution_id = %transitioned.current.id,
            previous_stage = %transitioned.previous.stage.as_str(),
            stage = %stage.as_str(),
            "graph-flow: atomically entered stage execution"
        );
        Ok(())
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

        // Reporting is prepared entirely from canonical DB truth before any
        // agent turn. This seam can build/validate a revision, but exposes no
        // artifact or finalization operation; publication remains an explicit
        // local-operator command after stage completion.
        if stage == crate::harness::StageKind::Reporting {
            let preparation = self
                .repo
                .reporting_build_validated_revision(task_id)
                .await
                .and_then(|truth| {
                    if truth.operation_id != task_id {
                        anyhow::bail!("REPORT_OPERATION_MISMATCH");
                    }
                    crate::harness::validate_reporting_gate_truth(&truth)
                        .map_err(anyhow::Error::new)?;
                    Ok(truth)
                });
            match preparation {
                Ok(truth) => {
                    tracing::info!(
                        target: "harness::hook",
                        operation_id = %task_id,
                        report_id = %truth.report_id,
                        revision_id = %truth.revision_id,
                        publication_status = %truth.publication_status,
                        "reporting canonical revision prepared and validated"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        target: "harness::hook",
                        operation_id = %task_id,
                        error = %error,
                        "reporting canonical revision preparation failed; stage blocked"
                    );
                    self.emit(AiEvent::TaskProgress {
                        task_id: task_id.to_string(),
                        status: "blocked".to_string(),
                        message: format!("Reporting canonical read model is not ready: {error}"),
                    });
                    return crate::harness::operation_flow::StageFlowOutcome::blocked();
                }
            }
        }

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
        let force_stage_run = if std::mem::take(&mut self.force_stage_run_on_resume_once) {
            let exact_candidate_v2 = self
                .candidate_v2_specialist_for_operation(task_id, stage, "fast_resume")
                .await;
            let can_force = exec_ctx.harness_org_id.is_some()
                && stage_has_stage_run_specialist(stage, exact_candidate_v2);
            if !can_force {
                tracing::info!(
                    target: "harness::hook",
                    stage = %stage.as_str(),
                    has_engagement_org = exec_ctx.harness_org_id.is_some(),
                    "fast resume did not force stage_run for this stage"
                );
            }
            can_force
        } else {
            false
        };
        exec_ctx.harness_forced_tool = force_stage_run.then(|| "stage_run".to_string());
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
        if force_stage_run {
            self.emit(AiEvent::TaskProgress {
                task_id: task_id.to_string(),
                status: "running".to_string(),
                message: format!(
                    "Fast resume: dispatching stage_run directly for '{}'.",
                    stage.as_str()
                ),
            });
        }

        let (result_text, _usage) = self
            .execute_single_subtask(&planned, exec_ctx, executor, &None, task_id)
            .await;
        exec_ctx.harness_forced_tool = None;

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
    /// Candidate review 是另一条 DB barrier：只在 runtime/attack contract 都为
    /// `V2Only` 时读取并阻塞；dual-write 的 V2 mirror 永远不是 live authority。
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
        // Resolve the projected successor before consulting any crossing-only
        // barrier. A CLI/GUI stage slice whose current stage is the projected
        // terminal has nothing to cross into; in particular, Candidate review
        // protects entry into Verification and must not hold `--to
        // attack_candidate` after the Candidate Gate has already passed.
        let Some(next) = crate::harness::operation_flow::branch_target(
            &dag.next_stages(from_stage),
            outcome.made_progress,
        ) else {
            return PhaseGateDecision::Allowed;
        };
        let exact_candidate_v2 = if from_stage == crate::harness::StageKind::AttackCandidate {
            match self.exact_candidate_v2_operation(task_id).await {
                Ok(enabled) => enabled,
                Err(error) => {
                    tracing::warn!(
                        target: "harness::hook",
                        task_id = %task_id,
                        error = %error,
                        "Candidate contract lookup failed; holding phase boundary"
                    );
                    return PhaseGateDecision::Held;
                }
            }
        } else {
            false
        };
        if exact_candidate_v2 {
            let barrier = match self
                .repo
                .attack_v2_review_barrier_for_operation(task_id)
                .await
            {
                Ok(barrier) => barrier,
                Err(error) => {
                    tracing::warn!(
                        target: "harness::hook",
                        task_id = %task_id,
                        error = %error,
                        "Candidate review barrier DB read failed; holding stage"
                    );
                    self.emit(AiEvent::TaskProgress {
                        task_id: task_id.to_string(),
                        status: "waiting_approval".to_string(),
                        message: "Candidate review is unavailable; holding before verification."
                            .to_string(),
                    });
                    return PhaseGateDecision::Held;
                }
            };
            let snapshot = crate::harness::attack_execution::ReviewBarrierSnapshot {
                wave_unit_count: barrier.wave_unit_count,
                review_closed_unit_count: barrier.review_closed_unit_count,
                candidate_count: barrier.candidate_count,
                proposed_candidate_count: barrier.proposed_candidate_count,
                durable_status: barrier.status.clone(),
                dispatch_is_stale: barrier.dispatch_is_stale,
            };
            let action = match crate::harness::attack_execution::decide_review_barrier(&snapshot) {
                Ok(action) => action,
                Err(error) => {
                    tracing::warn!(
                        target: "harness::hook",
                        task_id = %task_id,
                        error = %error,
                        "Candidate review barrier snapshot is inconsistent; holding stage"
                    );
                    return PhaseGateDecision::Held;
                }
            };
            if matches!(
                action,
                crate::harness::attack_execution::ReviewBarrierAction::Resumed
                    | crate::harness::attack_execution::ReviewBarrierAction::Terminal
            ) {
                return PhaseGateDecision::Allowed;
            }
            self.emit(AiEvent::TaskProgress {
                task_id: task_id.to_string(),
                status: "waiting_approval".to_string(),
                message: format!(
                    "Candidate review required for durable wave {} before verification.",
                    barrier.wave_run_id
                ),
            });
            self.emit(AiEvent::HarnessTrace {
                operation_id: task_id.to_string(),
                stage: from_stage.as_str().to_string(),
                agent_path: "main".to_string(),
                trace: golish_core::events::HarnessTraceKind::CandidateReviewRequired {
                    wave_run_id: barrier.wave_run_id.to_string(),
                    status: barrier.status,
                    resume_version: barrier.resume_version,
                    candidate_count: i64::try_from(barrier.candidate_count).unwrap_or(i64::MAX),
                    proposed_candidate_count: i64::try_from(barrier.proposed_candidate_count)
                        .unwrap_or(i64::MAX),
                },
            });
            return PhaseGateDecision::Held;
        }
        if from_stage == crate::harness::StageKind::TargetIntel
            && next == crate::harness::StageKind::ExternalAttackSurface
        {
            return if self.ensure_active_recon_target_scope(task_id).await {
                // The exact target review is the authorization boundary. Do not
                // ask for a second generic `before_active_scan` approval.
                PhaseGateDecision::Allowed
            } else {
                PhaseGateDecision::Held
            };
        }
        let Some(profile) = profile else {
            return PhaseGateDecision::Allowed;
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
    /// Returns `None` when the subtask carries no harness stage, the lookup
    /// errors, or no authoritative Enumeration coverage snapshot exists. A
    /// successful Enumeration snapshot preserves `Some([])`: it proves the
    /// stage denominator is genuinely empty and therefore vacuously complete.
    /// VulnTriage is intentionally absent: only its stage_run specialist gate
    /// may consume the operation-scoped final-sealed Enumeration surface.
    async fn fetch_in_scope_assets_for_gate(
        &self,
        planned: &PlannedSubtask,
        task_id: Uuid,
    ) -> Option<Vec<String>> {
        // Only stage-tagged subtasks run a gate; skip the DB hit otherwise.
        let stage = planned.harness_stage.as_ref()?.stage_kind;
        let stage_started_at = self
            .active_stage_started_at_for_gate(planned, task_id)
            .await;
        let wave_cutoff = self.asset_wave_cutoff_for_gate(planned, task_id).await;
        let asset_axis_cutoff =
            crate::harness::org_gate::stage_asset_axis_cutoff(stage, stage_started_at, wave_cutoff);
        let result = match asset_axis_cutoff {
            Some(cutoff) => {
                self.repo
                    .in_scope_assets_created_before(self.harness_org_id, cutoff)
                    .await
            }
            None => self.repo.in_scope_assets(self.harness_org_id).await,
        };
        match result {
            Ok(v) => {
                let v = self.exclude_dead_assets_if_opted_in(planned, v).await;
                if stage == crate::harness::StageKind::Enumeration {
                    if let (Some(org_id), Some(session_id)) =
                        (self.harness_org_id, self.chat_session_id.as_deref())
                    {
                        if let Ok(snapshot) = self
                            .repo
                            .stage_asset_coverage_for_operation(
                                Some(task_id),
                                org_id,
                                stage.as_str(),
                                Some(session_id),
                                stage_started_at,
                                None,
                                None,
                            )
                            .await
                        {
                            if let Ok((origins, _)) = crate::harness::org_gate::validated_exact_web_origin_axis_from_coverage_snapshot(
                                &snapshot,
                                stage,
                                org_id,
                                Some(session_id),
                            ) {
                                tracing::info!(
                                    target: "harness::hook",
                                    asset_count = origins.len(),
                                    org_id = %org_id,
                                    stage = stage.as_str(),
                                    "injecting exact Web Origin axis into stage gate"
                                );
                                return Some(origins);
                            }
                        }
                    }
                }
                tracing::info!(
                    target: "harness::hook",
                    asset_count = v.len(),
                    org_id = ?self.harness_org_id,
                    asset_axis_cutoff = ?asset_axis_cutoff,
                    "injecting authoritative in-scope assets into coverage gate"
                );
                Some(v)
            }
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

    /// Dead-asset denominator exclusion (design 2026-07-02-dead-asset-liveness-
    /// state §5.2), subtask-gate path (mirrors `org_gate`). When the subtask's
    /// stage spec opts in via `skip_dead_assets` (enumeration onward — never EAS),
    /// drop assets EAS confirmed dead so a dead host no longer forces a probe /
    /// `checked_empty`. All-dead is an authoritative zero denominator, distinct
    /// from a failed asset lookup. Only `'dead'` is dropped (`'unreachable'` may
    /// be transient).
    async fn exclude_dead_assets_if_opted_in(
        &self,
        planned: &PlannedSubtask,
        assets: Vec<String>,
    ) -> Vec<String> {
        let Some(stage) = planned.harness_stage.as_ref().map(|s| s.stage_kind) else {
            return assets;
        };
        let opted_in = crate::harness::load_embedded_stage_spec(stage)
            .map(|spec| spec.skip_dead_assets)
            .unwrap_or(false);
        if !opted_in {
            return assets;
        }
        let dead: std::collections::HashSet<String> = self
            .repo
            .dead_asset_values(self.harness_org_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
        if dead.is_empty() {
            return assets;
        }
        let survivors: Vec<String> = assets
            .iter()
            .filter(|a| !dead.contains(*a))
            .cloned()
            .collect();
        if survivors.len() == assets.len() {
            return assets;
        }
        tracing::info!(
            target: "harness::hook",
            stage = stage.as_str(),
            org_id = ?self.harness_org_id,
            removed = assets.len() - survivors.len(),
            "excluded confirmed-dead assets from coverage denominator"
        );
        survivors
    }

    async fn asset_wave_cutoff_for_gate(
        &self,
        planned: &PlannedSubtask,
        task_id: Uuid,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        let stage = planned.harness_stage.as_ref()?.stage_kind;
        let spec = crate::harness::load_embedded_stage_spec(stage).ok()?;
        if !spec.asset_wave_barrier {
            return None;
        }
        self.active_stage_started_at_for_gate(planned, task_id)
            .await
    }

    async fn active_stage_started_at_for_gate(
        &self,
        planned: &PlannedSubtask,
        task_id: Uuid,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        let stage = planned.harness_stage.as_ref()?.stage_kind;
        let state = crate::db_shim::operation_state::get(&*self.repo, task_id)
            .await
            .ok()
            .flatten()?;
        (crate::harness::StageKind::try_parse(&state.current_stage) == Some(stage))
            .then_some(state.stage_started_at)
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

    /// Web-capable seam: for EAS, fetch assets that now require WhatWeb
    /// web-stack coverage; for Enumeration, fetch IP/CIDR web roots proven by
    /// EAS/httpx. Empty/failed lookup keeps the previous denominator behavior.
    async fn fetch_web_capable_assets_for_gate(
        &self,
        planned: &PlannedSubtask,
        task_id: Uuid,
    ) -> Option<Vec<String>> {
        let stage = planned.harness_stage.as_ref()?.stage_kind;
        let spec = crate::harness::load_embedded_stage_spec(stage).ok()?;
        match stage {
            crate::harness::StageKind::Enumeration if spec.enum_ip_web_coverage => match self
                .repo
                .enumeration_web_capable_assets(self.harness_org_id)
                .await
            {
                Ok(v) if !v.is_empty() => Some(v),
                Ok(_) => None,
                Err(e) => {
                    tracing::warn!(
                        target: "harness::hook",
                        error = %e,
                        "web-capable IP lookup failed; enumeration gate keeps bare-IP exclusion"
                    );
                    None
                }
            },
            crate::harness::StageKind::ExternalAttackSurface => {
                let run_start = if spec.freshness_window {
                    crate::db_shim::operation_state::get(&*self.repo, task_id)
                        .await
                        .ok()
                        .flatten()
                        .map(|s| s.stage_started_at)
                } else {
                    None
                };
                match self
                    .repo
                    .eas_web_capable_assets(self.harness_org_id, run_start)
                    .await
                {
                    Ok(v) if !v.is_empty() => Some(v),
                    Ok(_) => None,
                    Err(e) => {
                        tracing::warn!(
                            target: "harness::hook",
                            error = %e,
                            "EAS web-capable lookup failed; web fingerprint coverage stays evidence-gated"
                        );
                        None
                    }
                }
            }
            _ => None,
        }
    }

    async fn fetch_not_applicable_coverage_for_gate(
        &self,
        planned: &PlannedSubtask,
        task_id: Uuid,
    ) -> Option<Vec<(String, String)>> {
        let stage = planned.harness_stage.as_ref()?.stage_kind;
        let org_id = self.harness_org_id?;
        match stage {
            crate::harness::StageKind::ExternalAttackSurface => {
                let sid = self.chat_session_id.as_deref()?;
                let spec = crate::harness::load_embedded_stage_spec(stage).ok()?;
                let run_start = if spec.freshness_window {
                    crate::db_shim::operation_state::get(&*self.repo, task_id)
                        .await
                        .ok()
                        .flatten()
                        .map(|s| s.stage_started_at)
                } else {
                    None
                };
                let rows = self
                    .repo
                    .technique_outcome_facts_fresh(org_id, sid, run_start)
                    .await;
                let pairs =
                    crate::harness::org_gate::eas_service_not_applicable_from_port_outcomes(&rows);
                (!pairs.is_empty()).then_some(pairs)
            }
            crate::harness::StageKind::VulnTriage => {
                let sid = self.chat_session_id.as_deref()?;
                let run_start = self
                    .active_stage_started_at_for_gate(planned, task_id)
                    .await;
                let rows = self
                    .repo
                    .technique_outcome_facts_fresh_with_evidence_session(
                        org_id,
                        &task_id.to_string(),
                        sid,
                        run_start,
                    )
                    .await;
                let pairs = crate::harness::org_gate::vuln_not_applicable_from_outcomes(&rows);
                (!pairs.is_empty()).then_some(pairs)
            }
            _ => None,
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
        task_id: Uuid,
    ) -> Option<(String, Option<HarnessGateOutcome>)> {
        let stage = planned.harness_stage.as_ref()?.stage_kind;
        let configured_fanout = crate::harness::load_embedded_stage_spec(stage)
            .map(|s| s.specialist.is_some())
            .unwrap_or(false);
        let exact_v2_verification = if stage == crate::harness::StageKind::Verification {
            match self.exact_candidate_v2_operation(task_id).await {
                Ok(enabled) => enabled,
                Err(error) => {
                    let deliverable = parse_deliverable_from_content(content)?;
                    return Some(render_named_specialist_gate(
                        content,
                        stage,
                        "verification_exact_db_truth",
                        false,
                        vec![format!(
                            "Verification Candidate V2 contract lookup failed: {error}"
                        )],
                        &deliverable,
                    ));
                }
            }
        } else {
            false
        };
        let is_fanout = configured_fanout || exact_v2_verification;
        if !is_fanout {
            return None;
        }
        let deliverable = parse_deliverable_from_content(content)?;
        if exact_v2_verification {
            match self
                .repo
                .attack_v2_verification_truth_for_operation(task_id, None)
                .await
            {
                Ok(Some(truth)) => {
                    let error =
                        crate::harness::attack_execution::validate_verification_truth_set(&truth)
                            .err()
                            .map(|error| format!("Verification exact DB truth blocked: {error}"));
                    return Some(render_named_specialist_gate(
                        content,
                        stage,
                        "verification_exact_db_truth",
                        error.is_none(),
                        error.into_iter().collect(),
                        &deliverable,
                    ));
                }
                Ok(None) => {
                    return Some(render_named_specialist_gate(
                        content,
                        stage,
                        "verification_exact_db_truth",
                        false,
                        vec!["V2Only Verification is missing exact DB truth".to_string()],
                        &deliverable,
                    ));
                }
                Err(error) => {
                    return Some(render_named_specialist_gate(
                        content,
                        stage,
                        "verification_exact_db_truth",
                        false,
                        vec![format!("Verification DB truth load failed: {error}")],
                        &deliverable,
                    ));
                }
            }
        }
        Some(
            self.verify_stage_run_pass_token(stage, content, &deliverable, task_id)
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
        task_id: Uuid,
    ) -> (String, Option<HarnessGateOutcome>) {
        use crate::harness::org_gate::{
            completion_is_fresh_for_stage, extract_pass_token, fanout_completion_scope_ids,
            stage_pass_token, STAGE_COMPLETION_TTL_SECS,
        };
        let engagement_subtree_ids = if let Some(root) = self.harness_org_id {
            match self.repo.org_subtree_ids(root).await {
                Ok(ids) if !ids.is_empty() => Some(ids),
                Ok(_) => {
                    tracing::warn!(
                        target: "harness::hook",
                        root_org = %root,
                        "fan-out closeout could not resolve engagement org subtree"
                    );
                    Some(vec![])
                }
                Err(error) => {
                    tracing::warn!(
                        target: "harness::hook",
                        root_org = %root,
                        error = %error,
                        "fan-out closeout org-subtree lookup failed"
                    );
                    Some(vec![])
                }
            }
        } else {
            None
        };
        let legacy_org_ids = if self.harness_org_id.is_none() {
            self.repo.in_scope_org_ids(None).await.unwrap_or_default()
        } else {
            vec![]
        };
        let org_ids = fanout_completion_scope_ids(
            self.harness_org_id,
            engagement_subtree_ids,
            legacy_org_ids,
        );
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
        let completion_not_before = crate::db_shim::operation_state::get(&*self.repo, task_id)
            .await
            .ok()
            .flatten()
            .and_then(|state| {
                (crate::harness::StageKind::try_parse(&state.current_stage) == Some(stage))
                    .then_some(state.stage_started_at)
            });
        if let Some(floor) = completion_not_before {
            tracing::info!(
                target: "harness::hook",
                stage = %stage.as_str(),
                stage_started_at = %floor,
                "fan-out closeout constrained pass-token completions to current active stage"
            );
        }
        let now = chrono::Utc::now();
        let expected_stage_run_id = task_id.to_string();
        let fresh: Vec<(uuid::Uuid, chrono::DateTime<chrono::Utc>)> = self
            .repo
            .org_stage_completions_get_with_run_id(stage.as_str(), &org_ids)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(organization_id, passed_at, row_stage_run_id)| {
                (completion_row_belongs_to_task(
                    row_stage_run_id.as_deref(),
                    &expected_stage_run_id,
                ) && completion_is_fresh_for_stage(
                    passed_at,
                    now,
                    STAGE_COMPLETION_TTL_SECS,
                    completion_not_before,
                ))
                .then_some((organization_id, passed_at))
            })
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
                     stage's per-org gate — re-run stage_run for the missing org(s) only while \
                     its prior result has retry_budget_exhausted=false. If it returned true, stop \
                     this request BLOCKED and resume from a separate user continuation. Missing: {:?}",
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
                             stage_run for this stage and submit the fresh pass_token it returns; \
                             if its prior result had retry_budget_exhausted=true, stop this request \
                             BLOCKED instead of re-entering it"
                    .to_string(),
            ],
            None => vec![
                "missing stage_run pass_token — call stage_run for this stage, then \
                          submit a claim {kind:\"stage_run_pass_token\", summary:<pass_token>} \
                          from its result. If stage_run already returned \
                          retry_budget_exhausted=true in this request, stop BLOCKED instead"
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
        // from a previous run can't satisfy a cell this run. For Enumeration, a
        // missing cutoff later disables outcome projection entirely; other stages
        // retain their historical presence-only fallback.
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
        let ledger_rows = if stage == crate::harness::StageKind::ExternalAttackSurface {
            match (self.harness_org_id, run_start) {
                (Some(organization_id), Some(since)) => {
                    self.repo
                        .eas_evidence_facts_for_session_org_fresh(sid, organization_id, since)
                        .await
                }
                _ => Ok(Vec::new()),
            }
        } else {
            self.repo.evidence_facts_for_session(sid).await
        };
        let mut facts: Vec<EvidenceFact> = match ledger_rows {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|(asset, technique, outcome, evidence_id)| {
                    // 保守解析：未知 outcome 字符串 → 丢行（不投影），绝不猜。
                    let outcome = match outcome.as_str() {
                        "found" => EvidenceOutcome::Found,
                        "empty" => EvidenceOutcome::Empty,
                        "blocked" => EvidenceOutcome::Blocked,
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
        // Enumeration 的四个内容轴会在 apply_technique_outcome_rows 中先清掉这些兼容
        // facts，只认当前 exact-origin outcome；其他阶段保持原有 DB truth 语义。
        // in_scope_assets 缺失（GUI/chat 路径 org_id=None 且无注入）→ 跳过。
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

        // #4/E3 (设计 2026-06-23-technique-outcomes-provenance): 从
        // technique_outcomes 物化表投影 EvidenceFact。普通 stage 仍与 ledger/DB facts
        // additive union；Enumeration 会先移除四个内容轴的兼容 facts，再以当前 exact-origin
        // rows 覆盖。run_id = chat session；org 绑定才读（表 org NOT NULL）。
        //
        // 护栏 4 (2026-07-02-gate-capability-ledger Phase 1)：套 `run_start` freshness
        // cutoff（与上面 db_truth_facts 同源），避免同 session 旧 stage-run 采集的
        // technique_outcomes 行泄漏进本 stage-run 的 coverage 判定。Enumeration 缺
        // run_start 时 fail-closed，不读旧行；未开启 freshness 的其他 stage 仍 presence-only。
        let outcome_rows = match self.harness_org_id {
            Some(org_id)
                if crate::harness::org_gate::stage_accepts_outcome_projection(
                    stage,
                    run_start.is_some(),
                ) =>
            {
                if stage == crate::harness::StageKind::VulnTriage {
                    self.repo
                        .technique_outcome_facts_fresh_with_evidence_session(
                            org_id,
                            &task_id.to_string(),
                            sid,
                            run_start,
                        )
                        .await
                } else {
                    self.repo
                        .technique_outcome_facts_fresh(org_id, sid, run_start)
                        .await
                }
            }
            _ => Vec::new(),
        };
        if !outcome_rows.is_empty() {
            tracing::info!(
                target: "harness::hook",
                technique_outcome_facts = outcome_rows.len(),
                "#4: applying current technique_outcomes projection to coverage gate"
            );
        }
        crate::harness::org_gate::apply_technique_outcome_rows(stage, &mut facts, &outcome_rows);

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
        let stage = planned.harness_stage.as_ref()?.stage_kind;
        if !crate::harness::org_gate::stage_accepts_source_query_completion(stage) {
            return None;
        }
        let org_id = self.harness_org_id?;
        let sid = self.chat_session_id.as_deref()?;
        let rows = match self.repo.source_query_facts(org_id, sid).await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(
                    target: "harness::hook",
                    %error,
                    "source_query_facts read failed; specialist org gate will fail closed"
                );
                return None;
            }
        };
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

    async fn fetch_reporting_truth_for_gate(
        &self,
        planned: &PlannedSubtask,
        operation_id: Uuid,
    ) -> Option<crate::harness::ReportingGateTruth> {
        let stage = planned.harness_stage.as_ref()?.stage_kind;
        if stage != crate::harness::StageKind::Reporting {
            return None;
        }
        match self.repo.reporting_gate_truth(operation_id).await {
            Ok(Some(truth)) if truth.operation_id == operation_id => Some(truth),
            Ok(Some(truth)) => {
                tracing::warn!(
                    target: "harness::hook",
                    expected_operation_id = %operation_id,
                    actual_operation_id = %truth.operation_id,
                    "reporting truth adapter returned a foreign operation; gate will block"
                );
                None
            }
            Ok(None) => {
                tracing::warn!(
                    target: "harness::hook",
                    operation_id = %operation_id,
                    "reporting current revision is missing; gate will block"
                );
                None
            }
            Err(error) => {
                tracing::warn!(
                    target: "harness::hook",
                    operation_id = %operation_id,
                    %error,
                    "reporting truth reload failed; gate will block"
                );
                None
            }
        }
    }

    /// Cleanup is never graded from model claims. The main-agent stage-close
    /// path re-reads the exact operation/org obligation and residual counts;
    /// missing identity or repository failure is a deterministic BLOCK.
    async fn enforce_cleanup_closeout_gate(
        &self,
        operation_id: Uuid,
        outcome: &mut HarnessGateOutcome,
    ) {
        if outcome.gated_stage != crate::harness::StageKind::Cleanup {
            return;
        }
        let snapshot = match self.harness_org_id {
            Some(organization_id) => self
                .repo
                .cleanup_closeout_gate(operation_id, organization_id)
                .await
                .map_err(|error| format!("cleanup authoritative closeout query failed: {error}")),
            None => Err("cleanup gate requires exact organization identity".to_string()),
        };
        let reason = match snapshot {
            Ok(snapshot) if snapshot.allows_closeout() => return,
            Ok(snapshot) => format!(
                "cleanup closeout blocked by DB truth: missing_obligations={}, nonterminal_obligations={}, undisclosed_residuals={}, invalid_terminal_truth={}",
                snapshot.missing_obligation_count,
                snapshot.nonterminal_obligation_count,
                snapshot.undisclosed_residual_count,
                snapshot.invalid_terminal_truth_count,
            ),
            Err(reason) => reason,
        };
        outcome.gate_allowed = false;
        outcome.gate_reasons.push(reason);
        let recovery = outcome.gate_recovery.get_or_insert_with(Default::default);
        recovery.repair_tool_calls.extend([
            "cleanup_inspect_obligation".to_string(),
            "cleanup_execute_obligation".to_string(),
            "cleanup_verify_absence".to_string(),
        ]);
        recovery.hints.push(
            "retry exact cleanup/absence verification, or ask the local operator for a residual waiver; do not change deliverable prose"
                .to_string(),
        );
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
    /// against this session's REAL `tool_calls` that the human either explicitly
    /// chose root-only scope, or completed a same-root candidate proposal followed
    /// by `ask_human(input_type="unit_review")`. Earlier versions also forced a
    /// `manage_organizations(action="create")`, but that made REUSE mode unsafe:
    /// an existing root org/tree would be re-created or expanded just to appease
    /// the gate. Creation is still fine when the org is missing or the user
    /// explicitly added units, but a human-confirmed existing tree is already a
    /// persisted record. An incomplete selected branch flips PASS→BLOCK with a
    /// corrective hint.
    /// The current operation's persisted tool lifecycle is authoritative. A
    /// required unit review or a non-empty target review fails closed when that
    /// lifecycle is missing; an organization-only flow with no target snapshot
    /// does not manufacture an empty target-table review.
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
        let policy = scoping_policy_for_ctx(exec_ctx);
        let scope_alignment_enabled = policy.require_human_scope_approval
            && matches!(
                policy.asset_confirmation,
                crate::harness::profile::AssetConfirmation::Interactive
            );
        if !policy.require_unit_candidates && !scope_alignment_enabled {
            return;
        }
        let trusted_bound_org = self.harness_org_id.or(exec_ctx.harness_org_id);
        let review_organization_id =
            match resolve_scoping_review_org(trusted_bound_org, outcome.engagement_org_id) {
                Ok(organization_id) => organization_id,
                Err(correction) => {
                    outcome.gate_allowed = false;
                    outcome.red_team_flow_correction = Some(correction);
                    return;
                }
            };
        let trusted_snapshot = if scope_alignment_enabled {
            match self
                .repo
                .scoping_target_snapshot(review_organization_id)
                .await
            {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    outcome.gate_allowed = false;
                    outcome.red_team_flow_correction = Some(format!(
                        "SCOPING TARGET REVIEW INCOMPLETE — trusted target snapshot read failed: {error}"
                    ));
                    return;
                }
            }
        } else {
            Vec::new()
        };
        let requires_scope_review = scope_alignment_enabled && !trusted_snapshot.is_empty();
        let requires_lifecycle = policy.require_unit_candidates || requires_scope_review;

        // For an empty snapshot, an absent scope_review is correct. We still
        // inspect an existing lifecycle when available so a model-authored
        // non-empty proposal cannot masquerade as trusted scope.
        let inspect_optional_empty_review = scope_alignment_enabled && trusted_snapshot.is_empty();
        if !requires_lifecycle && !inspect_optional_empty_review {
            return;
        }
        let review_not_before = match exec_ctx.operation_id {
            Some(operation_id) => {
                match crate::db_shim::operation_state::get(&*self.repo, operation_id).await {
                    Ok(Some(state))
                        if crate::harness::StageKind::try_parse(&state.current_stage)
                            == Some(crate::harness::StageKind::Scoping) =>
                    {
                        state.stage_started_at
                    }
                    Ok(Some(_)) => {
                        if requires_lifecycle {
                            outcome.gate_allowed = false;
                            outcome.red_team_flow_correction = Some(
                                "SCOPING HUMAN REVIEW INCOMPLETE — the operation is not durably bound to the current Scoping stage; refusing to reuse session-level approval history."
                                    .to_string(),
                            );
                        }
                        return;
                    }
                    Ok(None) | Err(_) => {
                        if requires_lifecycle {
                            outcome.gate_allowed = false;
                            outcome.red_team_flow_correction = Some(
                                "SCOPING HUMAN REVIEW INCOMPLETE — the current operation's Scoping start time is unavailable; refusing to reuse session-level approval history."
                                    .to_string(),
                            );
                        }
                        return;
                    }
                }
            }
            None => {
                if requires_lifecycle {
                    outcome.gate_allowed = false;
                    outcome.red_team_flow_correction = Some(
                        "SCOPING HUMAN REVIEW INCOMPLETE — no current operation id is available; refusing to reuse session-level approval history."
                            .to_string(),
                    );
                }
                return;
            }
        };
        let seen = match self
            .repo
            .scoping_actions_for_session(self.session_id, review_organization_id, review_not_before)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    target: "harness::hook",
                    error = %e,
                    "scoping red_team flow cross-check failed"
                );
                if requires_lifecycle {
                    outcome.gate_allowed = false;
                    outcome.red_team_flow_correction = Some(format!(
                        "SCOPING HUMAN REVIEW INCOMPLETE — approval verification failed: {e}. A scope_human_approved claim alone cannot prove the required review."
                    ));
                }
                return;
            }
        };
        let Some(seen) = seen else {
            if requires_lifecycle {
                outcome.gate_allowed = false;
                outcome.red_team_flow_correction = Some(
                    "SCOPING HUMAN REVIEW INCOMPLETE — approval verification is unavailable because no persisted tool-call lifecycle exists for this durable session; a scope_human_approved claim alone cannot prove the required review."
                        .to_string(),
                );
            }
            return;
        };
        let mut corrections = Vec::new();
        if policy.require_unit_candidates {
            if let Some(correction) = evaluate_red_team_scoping_flow(Some(seen.clone())) {
                corrections.push(correction);
            }
        }
        if scope_alignment_enabled
            && (requires_scope_review || !seen.scope_review_targets.is_empty())
        {
            if let Some(correction) = evaluate_scope_review_alignment(&seen, &trusted_snapshot) {
                corrections.push(correction);
            }
        }
        if !corrections.is_empty() {
            tracing::warn!(
                target: "harness::hook",
                stage = %outcome.gated_stage.as_str(),
                "gate BLOCK: red_team scoping review is not aligned with trusted scope state"
            );
            // 设计 2026-06-12-unified-refiner · 只置事实标记，Refiner G 类透传该文本。
            outcome.gate_allowed = false;
            outcome.red_team_flow_correction = Some(corrections.join("\n"));
        }
    }

    /// P2 · evidence-kind 回查: stage spec 声明的 `required_evidence_kinds` 必须真的
    /// 出现在交付物引用的证据里 (查 ledger 的 `detail->>'kind'`). 缺 → BLOCK + 纠正。
    /// infra 查询失败只 warn, 不误伤合法 stage。
    async fn enforce_evidence_kinds(&self, outcome: &mut HarnessGateOutcome) {
        if outcome.required_evidence_kinds.is_empty() {
            return;
        }
        if outcome.evidence_refs.is_empty() {
            tracing::debug!(
                target: "harness::hook",
                stage = %outcome.gated_stage.as_str(),
                required = ?outcome.required_evidence_kinds,
                "evidence kind check skipped: model-authored evidence ids are optional"
            );
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

fn completion_row_belongs_to_task(row_stage_run_id: Option<&str>, task_id: &str) -> bool {
    row_stage_run_id == Some(task_id)
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

fn collect_deliverable_evidence_ids(
    deliverable: &crate::harness::types::StageDeliverable,
) -> Vec<i64> {
    let mut seen = std::collections::HashSet::new();
    let mut ids = Vec::new();
    let mut push = |id: i64| {
        if seen.insert(id) {
            ids.push(id);
        }
    };
    for id in &deliverable.evidence_refs {
        push(id.as_i64());
    }
    for claim in &deliverable.claims {
        for id in &claim.evidence_ids {
            push(id.as_i64());
        }
    }
    for finding in &deliverable.findings {
        for id in &finding.evidence_refs {
            push(id.as_i64());
        }
    }
    for cell in &deliverable.coverage {
        for id in &cell.evidence_refs {
            push(id.as_i64());
        }
    }
    ids
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
    /// Exact DB-backed submission whose canonical payload was graded. Required
    /// for every V2-writing operation; legacy operations may leave it absent.
    trusted_submission: Option<crate::db_traits::CapturedStageSubmission>,
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
    /// P2 (graph flow) · how many findings the deliverable carried. Vulnerability
    /// stages use this as their progress signal; recon/info stages can suppress
    /// findings and still make progress through evidence/coverage handoff.
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
    /// Wave loop (设计 2026-07-02-attack-stage §3.5): the attack candidates this
    /// stage's deliverable carried. On a `verification` PASS, `consume_gate_outcome`
    /// feeds these to `decide_chain_wave` (with the run's cross-wave dedupe set +
    /// wave counter) to decide whether to overwrite the cursor back to
    /// attack_candidate for another wave. Empty for stages that carry no candidates.
    spawned_candidates: Vec<crate::harness::AttackCandidate>,
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

fn gate_outcome_made_progress(outcome: &HarnessGateOutcome) -> bool {
    if !outcome.gate_allowed {
        return false;
    }
    if outcome.findings_count > 0 {
        return true;
    }

    let findings_suppressed = crate::harness::load_embedded_stage_spec(outcome.gated_stage)
        .map(|spec| !spec.findings_allowed)
        .unwrap_or(false);
    if !findings_suppressed {
        return false;
    }

    outcome.engagement_org_id.is_some()
        || !outcome.evidence_refs.is_empty()
        || outcome
            .evidence_summary
            .as_deref()
            .is_some_and(stage_summary_indicates_progress)
}

fn stage_summary_indicates_progress(summary: &str) -> bool {
    summary.lines().any(|line| {
        let line = line.trim();
        if line.starts_with("- claims:") || line.starts_with("- findings:") {
            return true;
        }
        line.strip_prefix("- evidence refs:")
            .and_then(|count| count.trim().parse::<usize>().ok())
            .is_some_and(|count| count > 0)
    })
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
    crate::harness::org_gate::stage_gate_expected_techniques(stage, target_types)
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
    web_capable_assets: Option<Vec<String>>,
    not_applicable_coverage: Option<Vec<(String, String)>>,
    in_scope_target_types: Vec<String>,
    evidence_facts: Option<Vec<crate::harness::gate::rule_engine::EvidenceFact>>,
    source_queries: Option<Vec<crate::harness::SourceQueryFact>>,
    reporting_truth: Option<crate::harness::ReportingGateTruth>,
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

    // VulnTriage no longer has a generic/manual close path. Its two Nuclei
    // wrappers, operation-scoped evidence projection, and final-sealed
    // Enumeration denominator are assembled only by stage_run's per-org
    // specialist gate. Reaching this hook means that authoritative path was not
    // used, so fail closed before parsing any model-authored deliverable.
    if stage_hint.stage_kind == crate::harness::StageKind::VulnTriage {
        return (content, specialist_only_vuln_gate_outcome());
    }

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
        .authoritative_in_scope_assets(in_scope_assets)
        .asset_types_map(asset_types.unwrap_or_default())
        .web_capable_assets(web_capable_assets.unwrap_or_default())
        .not_applicable_coverage(not_applicable_coverage.unwrap_or_default())
        .extend_evidence_facts(evidence_facts.unwrap_or_default())
        .extend_source_queries(source_queries.unwrap_or_default())
        .expected_techniques(expected_techniques)
        .reporting_truth(reporting_truth)
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
            trusted_submission: None,
            engagement_org_id: extract_engagement_org_if_scoping(
                stage_hint.stage_kind,
                &deliverable,
            ),
            repair_correction: None,
            evidence_summary: Some(summarize_deliverable(&deliverable)),
            evidence_refs: collect_deliverable_evidence_ids(&deliverable),
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
            spawned_candidates: deliverable.candidates.clone(),
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
    render_named_specialist_gate(
        content,
        stage,
        "stage_run_pass_token",
        allowed,
        reasons,
        deliverable,
    )
}

fn render_named_specialist_gate(
    content: &str,
    stage: crate::harness::StageKind,
    gate_name: &str,
    allowed: bool,
    reasons: Vec<String>,
    deliverable: &crate::harness::StageDeliverable,
) -> (String, Option<HarnessGateOutcome>) {
    let decision = serde_json::json!({
        "allowed": allowed,
        "gate": gate_name,
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
            trusted_submission: None,
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
            spawned_candidates: deliverable.candidates.clone(),
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
        trusted_submission: None,
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
        spawned_candidates: Vec::new(),
    })
}

fn specialist_only_vuln_gate_outcome() -> Option<HarnessGateOutcome> {
    let mut outcome =
        missing_deliverable_gate_outcome(crate::harness::StageKind::VulnTriage, false)?;
    outcome.missing_deliverable = false;
    outcome.gate_reasons = vec![
        "vuln_triage must close through stage_run's vuln_scanner specialist and its authoritative operation-scoped Nuclei gate; generic/manual deliverables are unsupported"
            .to_string(),
    ];
    outcome.repair_correction = Some(
        "Run vuln_triage through stage_run so vuln_scanner can execute the two guarded Nuclei wrappers and submit DB-derived coverage=[]"
            .to_string(),
    );
    Some(outcome)
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
/// unit-review flow was skipped (⇒ BLOCK), or `None` to allow. This pure helper
/// treats `None` as no correction; the outer gate separately fails closed when
/// a required lifecycle is unavailable.
fn evaluate_red_team_scoping_flow(
    seen: Option<crate::db_traits::ScopingActionsSeen>,
) -> Option<String> {
    let seen = seen?;
    if seen.subsidiaries_excluded {
        return None;
    }
    if seen.unit_candidates_proposed && seen.unit_review_invoked {
        return None;
    }
    let mut missing = Vec::new();
    if !seen.unit_candidates_proposed {
        missing.push("manage_organizations(action=\"propose_candidates\")");
    }
    if !seen.unit_review_invoked {
        missing.push(
            "ask_human(input_type=\"unit_review\") for the user to confirm/edit candidate units",
        );
    }
    Some(format!(
        "RED-TEAM SCOPING INCOMPLETE — a `scope_human_approved` claim is present but this run never \
         completed the required subsidiary-scope branch. Missing: {}. If the human explicitly chooses \
         parent/root-only scope in ask_human(input_type=\"choice\", context containing \
         decision=\"subsidiary_scope\"), do not manufacture a candidate or unit-review table. Otherwise, \
         before submit you MUST call manage_organizations(action=\"propose_candidates\"), then \
         ask_human(input_type=\"unit_review\") so the user judges candidate units. If the root org/tree \
         already exists, DO NOT call create/create_batch just to satisfy the gate; only create a missing \
         root or units the user explicitly added/confirmed. A claim alone is not sufficient.",
        missing.join(" and ")
    ))
}

fn resolve_scoping_review_org(
    trusted_bound_org: Option<uuid::Uuid>,
    claimed_org: Option<uuid::Uuid>,
) -> Result<uuid::Uuid, String> {
    match (trusted_bound_org, claimed_org) {
        (Some(bound), Some(claimed)) if bound != claimed => Err(format!(
            "SCOPING TARGET REVIEW INCOMPLETE — scope claim organization {claimed} does not match the trusted operation organization {bound}."
        )),
        (Some(bound), _) => Ok(bound),
        (None, Some(claimed)) => Ok(claimed),
        (None, None) => Err(
            "SCOPING TARGET REVIEW INCOMPLETE — the approved scope cannot be matched to a trusted organization because the scope claim subject is not an organization UUID."
                .to_string(),
        ),
    }
}

#[cfg(test)]
use crate::task_orchestrator::active_recon_scope::canonical_scoping_cidr;
use crate::task_orchestrator::active_recon_scope::canonical_scoping_target;

fn evaluate_scope_review_alignment(
    seen: &crate::db_traits::ScopingActionsSeen,
    persisted: &[crate::db_traits::ScopingReviewedTarget],
) -> Option<String> {
    // Company-name / organization-only Scoping has no concrete trusted target
    // snapshot to approve. Do not manufacture an empty target-table review. A
    // non-empty human proposal against an empty store still falls through to the
    // mismatch check below and cannot expand authorization.
    if persisted.is_empty() && seen.scope_review_targets.is_empty() {
        return None;
    }
    if seen.scope_review_attempts != 1 {
        let detail = if seen.scope_review_attempts == 0 {
            "no successful parseable scope_review was persisted"
        } else {
            "multiple scope_review lifecycles were persisted; an earlier human edit/rejection cannot be replaced by a later confirmation"
        };
        return Some(format!(
            "SCOPING TARGET REVIEW INCOMPLETE — exactly one scope_review is required for a non-empty trusted snapshot, but {} were observed; {detail}.",
            seen.scope_review_attempts
        ));
    }
    if !seen.scope_review_approved {
        return Some(
            "SCOPING TARGET REVIEW INCOMPLETE — this run has no successful parseable ask_human(input_type=\"scope_review\") response; a claim alone cannot approve scope."
                .to_string(),
        );
    }
    let reviewed: std::collections::BTreeSet<String> = seen
        .scope_review_targets
        .iter()
        .filter_map(canonical_scoping_target)
        .collect();
    let stored: std::collections::BTreeSet<String> = persisted
        .iter()
        .filter_map(canonical_scoping_target)
        .collect();
    if reviewed.len() != seen.scope_review_targets.len() || stored.len() != persisted.len() {
        return Some(
            "SCOPING TARGET REVIEW INCOMPLETE — the reviewed or stored target list contains an invalid type/value/scope row."
                .to_string(),
        );
    }
    if reviewed.is_empty() {
        return Some(
            "SCOPING TARGET REVIEW INCOMPLETE — no concrete target seed was approved.".to_string(),
        );
    }
    if reviewed == stored {
        return None;
    }
    let missing_from_store = reviewed.difference(&stored).cloned().collect::<Vec<_>>();
    let not_reviewed = stored.difference(&reviewed).cloned().collect::<Vec<_>>();
    Some(format!(
        "SCOPING TARGET REVIEW INCOMPLETE — the human-edited list does not match the trusted pre-stage target snapshot. missing_from_store={missing_from_store:?}; not_reviewed={not_reviewed:?}. Update scope through the trusted UI/CLI ingestion path, then rerun Scoping; do not call manage_targets inside the stage."
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

fn stage_has_stage_run_specialist(
    stage: crate::harness::StageKind,
    exact_candidate_v2: bool,
) -> bool {
    crate::harness::load_embedded_stage_spec(stage)
        .ok()
        .and_then(|spec| {
            effective_stage_run_specialist(stage, spec.specialist.as_deref(), exact_candidate_v2)
        })
        .is_some()
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
                    "2) Ask the human whether subsidiaries/branches are in scope using ask_human(input_type=\"choice\", context=\"{\\\"decision\\\":\\\"subsidiary_scope\\\",\\\"organization_id\\\":\\\"<root-id>\\\"}\"). If they explicitly choose parent/root-only scope, persist that decision and skip discovery, propose_candidates, and unit_review. Only when subsidiaries may be included, call manage_organizations(action=\"propose_candidates\") and then ask_human(input_type=\"unit_review\") so the user can judge/edit candidates. If the engagement root/tree already exists, reuse it and do not create more orgs; only call manage_organizations(action=\"create\"/\"create_batch\") for a missing root or units the user explicitly added/confirmed. ",
                );
            }
            if matches!(
                scoping_policy.asset_confirmation,
                AssetConfirmation::Interactive
            ) {
                steps.push_str(
                    "3) Inspect the concrete domain/IP/CIDR/URL seeds already ingested by the trusted UI/CLI before this stage. Only when that trusted snapshot is NON-EMPTY, call ask_human(input_type=\"scope_review\") EXACTLY ONCE so the user can confirm or reject the exact list; after an edit/rejection, stop instead of opening a second review. For company/organization-only input with an EMPTY snapshot, do not ask for an empty target review; the applicable organization/unit confirmation is sufficient. Do NOT call manage_targets or create assets from organization OSINT; if a proposed approved seed is absent from the scoped target store, stop with a concrete ingestion blocker instead of inventing it. ",
                );
            }
            if scoping_policy.require_human_scope_approval {
                steps.push_str(
                    "4) After the applicable human approval, record a claim {kind:\"scope_human_approved\", subject:<engagement subject>} citing the applicable ask_human request_id (the `subsidiary_scope` choice for parent-only scope, `unit_review` when subsidiaries were reviewed, or `scope_review` for a non-empty concrete snapshot), then submit_stage_deliverable. ",
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
                steps.push_str(
                    "This stage is per-org specialist work. Do NOT call recon_list_providers, recon_discover_subsidiaries, recon_map_assets, recon_lookup_whois, or any sub_agent_* directly from the primary stage agent. Instead call stage_run with the confirmed root org plus subsidiaries from scoping; the recon worker receives the provider-survey methodology and submits each per-org deliverable. Re-run stage_run only for blocked orgs while retry_budget_exhausted=false. If retry_budget_exhausted=true, stop this request BLOCKED; a separate user continuation may resume the saved worker with a fresh bounded budget. After all orgs pass, submit the stage_run pass token via submit_stage_deliverable to close target_intel.",
                );
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
                 for `{target}` through stage_run/prober per-org workers: PORT SCANNING, \
                 service/version fingerprinting, HTTP probing, and screenshots — establish \
                 host x port x service x live-web. Prober should batch httpx/naabu/nmap \
                 over list/stdin inputs where possible instead of one foreground call per \
                 asset. Passive provider survey was ALREADY done upstream in target_intel — \
                 REUSE the inherited evidence and do NOT re-enumerate. JS/API extraction \
                 happens in the NEXT stage (enumeration) on the services you map here."
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
            "Validated Report Read Model",
            format!(
                "The server has already built or reused the current canonical, cited report \
                 revision for `{target}` and validated its complete source manifest, citations, \
                 redaction boundary, and cleanup closeout. Do not scan, retrieve RAG/KG/wiki \
                 context, invent narrative facts, render artifacts, or finalize publication. \
                 Submit only the minimal Reporting StageDeliverable acknowledging that the \
                 deterministic read model is ready; Gate PASS does not mean final publication."
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

/// Resolve the input exposed to agents for this top-level execution without
/// mutating the task's durable original input.
///
/// Fresh runs pass no override and therefore use `durable_task_input`. Resume
/// runs pass the current user message; a non-blank message becomes the
/// request-local operator input, while an empty/whitespace-only continuation
/// explicitly falls back to the durable original.
fn resolve_request_local_task_input(
    durable_task_input: String,
    request_input_override: Option<&str>,
) -> String {
    request_input_override
        .filter(|input| !input.trim().is_empty())
        .map(str::to_string)
        .unwrap_or(durable_task_input)
}

#[cfg(test)]
mod dag_driven_helper_tests {
    use super::*;
    use crate::harness::StageKind;

    #[test]
    fn candidate_v2_truth_requires_both_exact_immutable_contracts() {
        use crate::runtime_memory::RuntimeMemoryContract;
        use golish_core::AttackExecutionContract;

        assert!(exact_candidate_v2_contracts(
            RuntimeMemoryContract::V2Only,
            AttackExecutionContract::V2Only,
        ));
        for runtime in RuntimeMemoryContract::ALL {
            for attack in AttackExecutionContract::ALL {
                if (runtime, attack)
                    != (
                        RuntimeMemoryContract::V2Only,
                        AttackExecutionContract::V2Only,
                    )
                {
                    assert!(!exact_candidate_v2_contracts(runtime, attack));
                }
            }
        }

        for runtime in [
            RuntimeMemoryContract::DualWriteLegacyRead,
            RuntimeMemoryContract::DualWriteV2Preferred,
            RuntimeMemoryContract::V2Only,
        ] {
            for attack in [
                AttackExecutionContract::DualWriteReadLegacy,
                AttackExecutionContract::DualWriteReadV2Fallback,
                AttackExecutionContract::V2Only,
            ] {
                assert!(candidate_v2_synthesis_contracts(runtime, attack));
            }
        }
        assert!(!candidate_v2_synthesis_contracts(
            RuntimeMemoryContract::LegacyV1,
            AttackExecutionContract::DualWriteReadLegacy,
        ));
        assert!(!candidate_v2_synthesis_contracts(
            RuntimeMemoryContract::V2Only,
            AttackExecutionContract::Legacy,
        ));
    }

    #[test]
    fn candidate_v2_fresh_and_resume_use_effective_specialists_only_after_double_cutover() {
        assert_eq!(
            effective_stage_run_specialist(StageKind::Verification, None, true).as_deref(),
            Some("candidate_verifier"),
            "fresh Verification must receive the stage_run coordinator prompt in exact V2",
        );
        assert_eq!(
            effective_stage_run_specialist(StageKind::AttackCandidate, Some("analyst"), true)
                .as_deref(),
            Some("attack_analyst"),
        );
        assert!(stage_has_stage_run_specialist(
            StageKind::Verification,
            true
        ));

        assert_eq!(
            effective_stage_run_specialist(StageKind::Verification, None, false),
            None,
            "legacy/dual Verification must not acquire a V2-only specialist",
        );
        assert!(!stage_has_stage_run_specialist(
            StageKind::Verification,
            false
        ));
    }

    #[test]
    fn final_fanout_gate_rejects_another_operations_completion() {
        assert!(completion_row_belongs_to_task(
            Some("operation-b"),
            "operation-b"
        ));
        assert!(
            !completion_row_belongs_to_task(Some("operation-a"), "operation-b"),
            "a fresh timestamp from a concurrent operation must not satisfy this task"
        );
        assert!(
            !completion_row_belongs_to_task(None, "operation-b"),
            "legacy unbound completion rows fail closed at the operation final gate"
        );
    }

    #[test]
    fn request_local_resume_input_overrides_durable_original_without_merging() {
        let durable = "A: run Enumeration over the original exact-origin set".to_string();
        let resumed = "B: do not call producers for the five unreachable exact origins";

        let resolved = resolve_request_local_task_input(durable, Some(resumed));

        assert_eq!(resolved, resumed);
        assert!(
            !resolved.contains("A:"),
            "the worker must see current request B, not stale A"
        );
    }

    #[test]
    fn request_local_resume_input_fresh_and_blank_fall_back_to_durable_original() {
        let durable = "A: original operation objective".to_string();

        assert_eq!(
            resolve_request_local_task_input(durable.clone(), None),
            durable
        );
        assert_eq!(
            resolve_request_local_task_input(durable.clone(), Some("  \n\t")),
            durable
        );
    }

    #[test]
    fn same_request_stage_run_exhaustion_stops_automatic_gate_repair_only() {
        assert!(
            should_retry_gate_block(0, false),
            "a request whose bounded stage_run budget is still open may use the normal repair loop"
        );
        assert!(
            !should_retry_gate_block(0, true),
            "once stage_run exhausts this top-level request's budget, the orchestrator must not open another automatic repair turn"
        );
        assert!(
            !should_retry_gate_block(MAX_REFLECTOR_RETRIES, false),
            "the ordinary reflector bound remains authoritative"
        );
    }

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
    /// 红队 (require_unit_candidates + human gate) 出 unit_review +
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
        assert!(s.description.contains("subsidiary_scope"));
        assert!(s.description.contains("parent/root-only"));
        assert!(s.description.contains("propose_candidates"));
        assert!(s.description.contains("unit_review"));
        assert!(s.description.contains("scope_review"));
        assert!(s.description.contains("scope_human_approved"));
        assert!(s.description.contains("EXACTLY ONCE"));
        assert!(s.description.contains("Do NOT call manage_targets"));

        let smoke = ScopingPolicy {
            require_human_scope_approval: false,
            asset_confirmation: AssetConfirmation::None,
            ..ScopingPolicy::default()
        };
        let s2 = synthesize_stage_subtask(StageKind::Scoping, "x", &smoke, &intel);
        assert!(!s2.description.contains("scope_human_approved"));
        assert!(!s2.description.contains("unit_review"));
    }

    /// 红队 scoping 防偷懒硬门禁 (设计 2026-06-06 §3.4 P1 强化): 仅凭 claim 不够。
    /// 明确 root-only 的持久化 choice 可完成分支；纳入子公司时必须真实完成
    /// propose_candidates + unit_review。已有树可复用，不再强制建组织。
    /// pure helper 的 None 不产生 correction；outer gate 负责 required lifecycle fail-closed.
    #[test]
    fn red_team_scoping_flow_blocks_when_steps_skipped() {
        use crate::db_traits::ScopingActionsSeen;

        // Unit review plus creation → allow.
        assert!(evaluate_red_team_scoping_flow(Some(ScopingActionsSeen {
            unit_candidates_proposed: true,
            unit_review_invoked: true,
            organization_created: true,
            ..Default::default()
        }))
        .is_none());

        // REUSE mode: the existing org tree was human-reviewed, so a fresh create
        // is not required. This prevents runaway create_batch expansion.
        assert!(evaluate_red_team_scoping_flow(Some(ScopingActionsSeen {
            unit_candidates_proposed: true,
            unit_review_invoked: true,
            organization_created: false,
            ..Default::default()
        }))
        .is_none());

        // Missing unit_review → BLOCK, correction names it.
        let c = evaluate_red_team_scoping_flow(Some(ScopingActionsSeen {
            unit_candidates_proposed: true,
            unit_review_invoked: false,
            organization_created: true,
            ..Default::default()
        }))
        .expect("missing unit_review must block");
        assert!(c.contains("unit_review"));

        let c = evaluate_red_team_scoping_flow(Some(ScopingActionsSeen {
            unit_review_invoked: true,
            ..Default::default()
        }))
        .expect("unit review without a candidate proposal must block");
        assert!(c.contains("propose_candidates"));

        // Both missing (the shortcut) → BLOCK, names unit_review but does not
        // instruct the model to create more orgs.
        let c3 = evaluate_red_team_scoping_flow(Some(ScopingActionsSeen::default()))
            .expect("both missing must block");
        assert!(c3.contains("unit_review"));
        assert!(!c3.contains("manage_organizations(action=\"create\")"));

        // Unverifiable (no recorded tool_calls) → fail open (allow).
        assert!(evaluate_red_team_scoping_flow(None).is_none());
    }

    #[test]
    fn explicit_subsidiary_exclusion_needs_no_empty_unit_review() {
        let seen = crate::db_traits::ScopingActionsSeen {
            subsidiaries_excluded: true,
            ..Default::default()
        };
        assert!(
            evaluate_red_team_scoping_flow(Some(seen)).is_none(),
            "a persisted parent-only decision must not manufacture an empty unit review"
        );
    }

    #[test]
    fn scope_review_must_exactly_match_trusted_seed_snapshot() {
        use crate::db_traits::{ScopingActionsSeen, ScopingReviewedTarget};

        let seed = ScopingReviewedTarget {
            value: "MoreSec.CN.".to_string(),
            target_type: "domain".to_string(),
            scope: "in".to_string(),
        };
        let approved = ScopingActionsSeen {
            scope_review_approved: true,
            scope_review_attempts: 1,
            scope_review_targets: vec![ScopingReviewedTarget {
                value: "moresec.cn".to_string(),
                target_type: "domain".to_string(),
                scope: "in".to_string(),
            }],
            ..Default::default()
        };
        assert!(evaluate_scope_review_alignment(&approved, std::slice::from_ref(&seed)).is_none());

        let edited = ScopingActionsSeen {
            scope_review_targets: vec![ScopingReviewedTarget {
                value: "vendor.example".to_string(),
                target_type: "domain".to_string(),
                scope: "in".to_string(),
            }],
            ..approved.clone()
        };
        let correction = evaluate_scope_review_alignment(&edited, &[seed])
            .expect("human edits absent from trusted ingestion must block");
        assert!(correction.contains("missing_from_store"));
        assert!(correction.contains("trusted UI/CLI"));
    }

    #[test]
    fn scope_review_skip_or_free_text_cannot_be_replaced_by_claim() {
        let seed = crate::db_traits::ScopingReviewedTarget {
            value: "moresec.cn".to_string(),
            target_type: "domain".to_string(),
            scope: "in".to_string(),
        };
        let correction = evaluate_scope_review_alignment(
            &crate::db_traits::ScopingActionsSeen::default(),
            &[seed],
        )
        .expect("missing approved review must block when trusted targets exist");
        assert!(correction.contains("no successful parseable"));
    }

    #[test]
    fn repeated_scope_review_cannot_replace_an_edited_response() {
        let seed = crate::db_traits::ScopingReviewedTarget {
            value: "moresec.cn".to_string(),
            target_type: "domain".to_string(),
            scope: "in".to_string(),
        };
        let repeated = crate::db_traits::ScopingActionsSeen {
            scope_review_approved: true,
            scope_review_attempts: 2,
            scope_review_targets: vec![seed.clone()],
            ..Default::default()
        };
        let correction = evaluate_scope_review_alignment(&repeated, &[seed])
            .expect("a second review must not wash away the first human edit");
        assert!(correction.contains("exactly one"));
    }

    #[test]
    fn organization_only_scope_does_not_require_an_empty_target_review() {
        assert!(evaluate_scope_review_alignment(
            &crate::db_traits::ScopingActionsSeen::default(),
            &[]
        )
        .is_none());
    }

    #[test]
    fn scope_claim_cannot_override_prebound_operation_org() {
        let bound = uuid::Uuid::new_v4();
        let sibling = uuid::Uuid::new_v4();
        assert_eq!(
            resolve_scoping_review_org(Some(bound), Some(bound)),
            Ok(bound)
        );
        assert_eq!(resolve_scoping_review_org(Some(bound), None), Ok(bound));
        assert!(resolve_scoping_review_org(Some(bound), Some(sibling))
            .expect_err("claim must not override trusted operation org")
            .contains("does not match"));
        assert_eq!(resolve_scoping_review_org(None, Some(sibling)), Ok(sibling));
    }

    #[test]
    fn scope_review_cidr_identity_masks_host_bits_and_rejects_invalid_prefix() {
        assert_eq!(
            canonical_scoping_cidr("203.0.113.7/24").as_deref(),
            Some("203.0.113.0/24")
        );
        assert_eq!(
            canonical_scoping_cidr("2001:db8::1234/64").as_deref(),
            Some("2001:db8::/64")
        );
        assert!(canonical_scoping_cidr("203.0.113.7/99").is_none());
    }

    /// target_intel is a specialist stage: the primary prompt must route through
    /// `stage_run`, while the recon worker receives the provider methodology.
    /// Skip mode remains a direct not_applicable closeout.
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
        assert!(s.description.contains("stage_run"));
        assert!(s.description.contains("per-org specialist work"));
        assert!(s.description.contains("Do NOT call recon_list_providers"));
        assert!(s.description.contains("recon_map_assets"));
        assert!(s.description.contains("submit the stage_run pass token"));
        assert!(s.description.contains("recon worker"));
        assert!(s.description.contains("retry_budget_exhausted=true"));
        assert!(s.description.contains("separate user continuation"));
        assert!(!s
            .description
            .contains("Call recon_map_assets(organization_id=<org>)"));
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
    fn gate_expected_techniques_ip_only_enumeration_keeps_exact_origin_four_axes() {
        let t = super::gate_expected_techniques(StageKind::Enumeration, &["ip_address".into()])
            .expect("ip scope yields a technique set for enumeration");
        assert_eq!(
            t,
            vec![
                "GOLISH-ENUM-JS".to_string(),
                "GOLISH-ENUM-DIR".to_string(),
                "GOLISH-ENUM-PARAM".to_string(),
                "GOLISH-ENUM-JSAPI".to_string(),
            ]
        );
    }

    #[test]
    fn gate_expected_techniques_enumeration_stays_four_axes_without_target_types() {
        assert_eq!(
            super::gate_expected_techniques(StageKind::Enumeration, &[]).unwrap(),
            vec![
                "GOLISH-ENUM-JS".to_string(),
                "GOLISH-ENUM-DIR".to_string(),
                "GOLISH-ENUM-PARAM".to_string(),
                "GOLISH-ENUM-JSAPI".to_string(),
            ]
        );
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
                None,
                None,
                vec![],
                None,
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
                None,
                None,
                vec![],
                None,
                None,
                None,
                None,
            )
            .0,
            content
        );
    }

    #[test]
    fn generic_vuln_triage_gate_is_rejected_in_favor_of_stage_run_specialist() {
        let p = planned_with_harness(StageKind::VulnTriage);
        let ctx = ExecutionContext::default();
        let content =
            r#"{"stage_id":"vuln_triage","claims":[],"findings":[],"coverage":[]}"#.to_string();
        let (_, outcome) = apply_harness_gate_hook(
            &p,
            &ctx,
            content,
            Some(Vec::new()),
            None,
            None,
            None,
            vec![],
            None,
            None,
            None,
            None,
        );
        let outcome = outcome.expect("generic VulnTriage must return an explicit BLOCK");

        assert!(!outcome.gate_allowed);
        assert!(
            !outcome.missing_deliverable,
            "the unsupported generic path must not be misclassified as a submit-only missing deliverable"
        );
        assert!(outcome
            .gate_reasons
            .iter()
            .any(|reason| reason.contains("generic/manual deliverables are unsupported")));
        assert!(outcome
            .repair_correction
            .as_deref()
            .is_some_and(|correction| correction.contains("stage_run")));
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
            candidates: vec![],
            candidate_decisions: vec![],
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
            trusted_submission: None,
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
            spawned_candidates: Vec::new(),
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
        // No real ids known → tell the agent to remove fabricated ids or run
        // tools if DB truth is still missing.
        assert!(
            d.correction.contains("Evidence ids are optional"),
            "empty real-id set must not force id-filling: {}",
            d.correction
        );
    }

    #[test]
    fn block_outcome_for_fabricated_names_real_ids_when_available() {
        // When the operation already has real evidence ids, the correction may
        // name them as debug context but must not require the model to copy ids
        // into the deliverable.
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
            d.correction.contains("Evidence ids are optional"),
            "does not force id-filling: {}",
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

    // ── generic deliverable stages: missing deliverable = fail-closed BLOCK ──

    #[test]
    fn hook_blocks_missing_deliverable_even_with_ledger_facts() {
        // PR-R2 行为变化锚点：target_intel（旧投影兜底的灰度 stage）现在与其它
        // substantive stage 一致——账本有真证据也不投影，BLOCK 后由 Refiner 的
        // A 类 submit-only 锁驱动 agent 自己提交（live run 两连截胡的根治）。
        let ctx = ExecutionContext::default();
        for stage in [StageKind::TargetIntel, StageKind::ExternalAttackSurface] {
            let p = planned(stage);
            let facts = vec![fact("a", "GOLISH-INTEL-DNS", EvidenceOutcome::Found, 7)];
            let (out, outcome) = apply_harness_gate_hook(
                &p,
                &ctx,
                "prose".to_string(),
                None,
                None,
                None,
                None,
                vec![],
                Some(facts),
                None,
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
            None,
            None,
            vec![],
            None,
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

    #[test]
    fn specialist_stages_are_fast_resume_stage_run_candidates() {
        assert!(stage_has_stage_run_specialist(
            StageKind::TargetIntel,
            false
        ));
        assert!(stage_has_stage_run_specialist(
            StageKind::ExternalAttackSurface,
            false
        ));
        assert!(stage_has_stage_run_specialist(
            StageKind::Enumeration,
            false
        ));
    }

    #[test]
    fn non_specialist_stages_do_not_force_stage_run_on_resume() {
        assert!(!stage_has_stage_run_specialist(StageKind::Scoping, false));
        assert!(!stage_has_stage_run_specialist(StageKind::Reporting, false));
    }
}
