//! Single-subtask execution with enrichment, planning, and reflector retry.

use uuid::Uuid;

use crate::db_shim::subtasks;
use crate::db_traits::SubtaskStatus;
use golish_core::events::AiEvent;

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

        for reflector_attempt in 0..=MAX_REFLECTOR_RETRIES {
            let exec_result = if reflector_attempt == 0 {
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
                        apply_harness_gate_hook(planned, agent_result.content);
                    if let Some(outcome) = gate_outcome {
                        self.drive_stage_transition(task_id, outcome).await;
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
        let (out, gate_outcome) = apply_harness_gate_hook(planned, fallback);
        if let Some(outcome) = gate_outcome {
            self.drive_stage_transition(task_id, outcome).await;
        }
        (out, None)
    }

    /// Phase 2/C: gate 通过后按 Operation DAG 推进 operation_state 游标 (Doc 3 §6.2).
    ///
    /// `operation_id == task_id` (一个 Task = 一个 operation). 读 `operation_state`
    /// 拿**真实 profile + 当前 stage** (不再硬编码 assessment), 投影 DAG, 用
    /// [`crate::harness::decide_transition`] 选下一 stage. C5: 若下一 stage 需人工
    /// 批准则 hold 并发 `waiting_approval` 事件, 不自动推进. Hold / Complete (无下
    /// 一格) 同样不写.
    async fn drive_stage_transition(&self, operation_id: Uuid, outcome: HarnessGateOutcome) {
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
        let Some(next) = decision.advance_target() else {
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
        // 发 waiting_approval 事件请求确认, 不自动推进游标.
        if let Ok(next_spec) = crate::harness::load_embedded_stage_spec(next) {
            if crate::harness::stage_entry_requires_approval(&next_spec, &profile) {
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
                        "Stage {} → {} requires human approval before proceeding.",
                        outcome.gated_stage.as_str(),
                        next.as_str()
                    ),
                });
                return;
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
    }
}

// ── Harness gate hook (Phase C · Doc 3 §5.2 接入点) ─────────────────────────
//
// 仅当满足以下全部条件时, agent_result.content 末尾才会被追加 gate decision JSON:
//   1. `harness::stage_mode_enabled()` 返回 true (默认 false)
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
}

/// 返回 `(content, Option<outcome>)`: `None` 表示 hook 透传 (未跑 gate); `Some` 表示
/// 跑了 gate, 调用方据此驱动 stage 流转 (推进 operation_state 游标).
fn apply_harness_gate_hook(
    planned: &PlannedSubtask,
    content: String,
) -> (String, Option<HarnessGateOutcome>) {
    if !crate::harness::stage_mode_enabled() {
        return (content, None);
    }
    let Some(stage_hint) = planned.harness_stage.as_ref() else {
        tracing::debug!(
            target: "harness::hook",
            "skip: stage_mode enabled but planned.harness_stage is None"
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

    // profile 仅用于构造 StageHarness (gate 校验只读 stage_spec); 用 assessment 占位.
    // 真实 profile 由 drive_stage_transition 按 operation_state 读取并影响流转/审批.
    let profile = match crate::harness::load_embedded_profile("assessment") {
        Ok(Some(p)) => p,
        _ => {
            tracing::warn!(target: "harness::hook", "[harness] failed to load assessment profile");
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

    let Some(deliverable) = parse_deliverable_from_content(&content) else {
        // 找不到 deliverable 时不强制 block; debug-log 留痕.
        tracing::debug!(
            target: "harness::hook",
            content_len = content.len(),
            "no StageDeliverable JSON found in agent content, skipping gate"
        );
        return (content, None);
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
        }),
    )
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
    fn feature_flag_off_skips_gate_unconditionally() {
        // crate::harness::stage_mode_enabled() 默认 false → hook 必然透传
        let p = planned_with_harness(StageKind::ExternalAttackSurface);
        let content = "anything".to_string();
        assert_eq!(apply_harness_gate_hook(&p, content.clone()).0, content);
    }

    #[test]
    fn no_harness_stage_skips_gate() {
        let p = planned_no_harness();
        let content = "ignore me".to_string();
        assert_eq!(apply_harness_gate_hook(&p, content.clone()).0, content);
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
}
