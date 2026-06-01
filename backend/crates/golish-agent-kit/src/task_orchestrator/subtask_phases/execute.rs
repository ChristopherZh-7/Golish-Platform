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

    /// Phase 2: gate 通过后按 Operation DAG 推进 operation_state 游标 (Doc 3 §6.2).
    ///
    /// `operation_id == task_id` (Phase 2 决定: 一个 Task = 一个 operation). 投影
    /// assessment DAG, 用 [`crate::harness::decide_transition`] 选下一 stage, 写入
    /// `operation_state.current_stage`. Hold / Complete (无下一格) 则不写.
    async fn drive_stage_transition(&self, operation_id: Uuid, outcome: HarnessGateOutcome) {
        let graph = match crate::harness::base_operation_graph() {
            Ok(g) => g,
            Err(e) => {
                tracing::warn!(target: "harness::hook", error = %e, "base operation graph load failed");
                return;
            }
        };
        let profile = match crate::harness::load_profile_from_json(ASSESSMENT_PROFILE_JSON) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(target: "harness::hook", error = %e, "profile load failed in transition");
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

// ── Harness gate hook (Phase 1c.6 · Doc 3 §5.2 接入点) ─────────────────────────
//
// 仅当满足以下全部条件时, agent_result.content 末尾才会被追加 gate decision JSON:
//   1. `harness::stage_mode_enabled()` 返回 true (Phase 1 默认 false, 见
//      Task 1c.7 settings.toml 接入)
//   2. `planned.harness_stage` 非 None
//   3. `harness_stage.stage_kind == StageKind::ExternalAttackSurface`
//      (Phase 1 MVP 仅支持此 stage; 其它 stage 推 Phase 2)
//   4. agent_result.content 含可解析的 ExternalAttackSurfaceDeliverable JSON
//      (Phase 1 MVP 简化路径: 整个 content 必须是 JSON)
//
// 任一条件不满足时返回原 content (不破坏旧路径行为).
//
// **现有 execute_single_subtask 2 元组返回签名保持不变** (plan §5 Task 1c.6
// hook 代码用 3 元组返回值, 实际项目签名是 2 元组; 通过把 gate decision 文本
// 化嵌入 content 末尾来兼容).

const ASSESSMENT_PROFILE_JSON: &str =
    include_str!("../../../../../../resources/harness/profiles/assessment.json");

const EXTERNAL_ATTACK_SURFACE_SPEC_JSON: &str =
    include_str!("../../../../../../resources/harness/stages/external_attack_surface.json");

/// gate hook 跑完后回传给流转驱动的最小信息 (Phase 2 · Doc 3 §6.2).
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
    // Phase 1 MVP 仅支持 ExternalAttackSurface
    if stage_hint.stage_kind != crate::harness::StageKind::ExternalAttackSurface {
        tracing::debug!(
            target: "harness::hook",
            stage_kind = ?stage_hint.stage_kind,
            "skip: Phase 1 MVP supports only ExternalAttackSurface"
        );
        return (content, None);
    }

    tracing::info!(
        target: "harness::hook",
        stage_kind = ?stage_hint.stage_kind,
        subtask_title = %planned.title,
        content_len = content.len(),
        "harness gate hook entered"
    );

    let profile = match crate::harness::load_profile_from_json(ASSESSMENT_PROFILE_JSON) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(target: "harness::hook", error = %e, "[harness] failed to load assessment profile JSON");
            return (content, None);
        }
    };
    let spec = match crate::harness::load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_SPEC_JSON) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(target: "harness::hook", error = %e, "[harness] failed to load external_attack_surface spec JSON");
            return (content, None);
        }
    };
    let harness = match crate::harness::StageHarness::for_stage(
        stage_hint.stage_kind,
        profile,
        spec,
    ) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(target: "harness::hook", error = %e, "[harness] StageHarness::for_stage failed");
            return (content, None);
        }
    };

    let Some(deliverable) = parse_deliverable_from_content(&content) else {
        // Phase 1 MVP: 找不到 deliverable 时不强制 block; debug-log 留痕.
        tracing::debug!(
            target: "harness::hook",
            content_len = content.len(),
            "no ExternalAttackSurfaceDeliverable JSON found in agent content, skipping gate"
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

/// Phase 1 MVP: 尝试把 content 整体 (trim 后) 解析为 ExternalAttackSurfaceDeliverable.
///
/// 若 content 是混合文本 (含 prose + JSON code block), Phase 2 加 JSON code
/// fence 抽取; 当前简化版返回 None → hook skip.
fn parse_deliverable_from_content(
    content: &str,
) -> Option<crate::harness::ExternalAttackSurfaceDeliverable> {
    serde_json::from_str(content.trim()).ok()
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
    fn embedded_resource_jsons_parse_at_compile_time() {
        // 测的不是逻辑而是 const include_str! 路径正确, 嵌入的 JSON 合法.
        assert!(!ASSESSMENT_PROFILE_JSON.is_empty());
        assert!(!EXTERNAL_ATTACK_SURFACE_SPEC_JSON.is_empty());
        assert!(crate::harness::load_profile_from_json(ASSESSMENT_PROFILE_JSON).is_ok());
        assert!(
            crate::harness::load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_SPEC_JSON).is_ok()
        );
    }

    #[test]
    fn parse_deliverable_returns_none_on_non_json_content() {
        assert!(parse_deliverable_from_content("not json").is_none());
        assert!(parse_deliverable_from_content("# markdown header\n\nsome text").is_none());
    }
}
