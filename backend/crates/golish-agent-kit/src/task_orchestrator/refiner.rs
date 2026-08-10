//! 统一 Refiner（设计 2026-06-12-unified-refiner）：gate 判错后的唯一纠错通道。
//!
//! 确定性分类（按危害优先级取主因）→ 每类一个独立 prompt 模板 → submit-only 锁
//! 决策。gate / enforce_* 只产「事实」（reasons、fabricated ids、missing kinds、
//! expired ids …），渲染权全部上收本模块；execute.rs 重试循环只消费
//! [`RefineDecision`]。
//!
//! 红线：本模块只产纠正文本，绝不合成 StageDeliverable——deliverable 永远出自
//! 主 agent 之手（submit-only 锁逼出来的也算：它经过 agent 的 LLM 决策与
//! submit 工具侧信道）。

use std::collections::HashMap;

use crate::harness::gate::rule_engine::EvidenceFact;
use crate::harness::StageKind;

/// 主因分类（HarnessTrace / 日志可观测）。优先级 = 枚举声明序（上 = 先判）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefineClass {
    /// D · 引用了账本不存在的 evidence id（伪造，最高危）。
    Fabricated,
    /// A · missing deliverable，但活已干（账本有真 ids）或属 confirm-only 阶段
    /// （无扫描工具，唯一动作就是 submit）→ 锁 tool_choice。
    SubmitOnly,
    /// B · missing deliverable 且账本空（活没干）→ 重做。
    RedoStage,
    /// C · 交了但 vacuous / coverage 缺口 → 诊断式（DB 真值现状 + 具体命令）。
    CoverageOrVacuous,
    /// E · 缺 required evidence kinds / 引用硬过期证据。
    EvidenceQuality,
    /// G · red_team scoping 流程缺失（enforce_scoping_red_team_flow 已产文本，透传）。
    ScopingFlow,
    /// F · gate 之前的检测：响应是纯散文、无工具调用（PR-R4 接线）。
    TextOnly,
    /// 兜底：其它 BLOCK 原因。
    Generic,
}

/// gate + enforce_* 产出的全部「事实」。分类与渲染的唯一输入，无 IO。
pub(crate) struct RefineInput<'a> {
    pub stage: StageKind,
    /// gate `decision.reasons`（BLOCK 原因原文）。
    pub gate_reasons: &'a [String],
    pub gate_recovery: Option<&'a crate::harness::HarnessRecoveryActions>,
    pub missing_deliverable: bool,
    /// Explicit stage semantic: scoping without subsidiary discovery, reporting, etc.
    /// This is not inferred from `allowed_tool_types` because provider-only stages can
    /// disallow direct tools while still requiring a substantive deliverable.
    pub confirm_only_stage: bool,
    pub fabricated_ids: &'a [i64],
    /// 账本真实 ids（newest first；missing 时由 gather_missing_deliverable_ids 填，
    /// fabricated 时由 enforce_evidence_existence 填）。
    pub available_real_ids: &'a [i64],
    /// id → evidence kind 标签（A 类模板 `#2247 (dns_a)` 用）。
    pub evidence_kind_labels: &'a HashMap<i64, String>,
    /// enforce_evidence_kinds 置：stage 要求但 deliverable 证据缺失的 kinds。
    pub missing_kinds: &'a [String],
    /// enforce_evidence_freshness 置：硬过期证据的描述行（`freshness_age_reasons` 原文）。
    pub expired: &'a [String],
    /// enforce_scoping_red_team_flow 置：已渲染好的流程纠正（G 类透传）。
    pub red_team_flow_correction: Option<&'a str>,
    /// C 类诊断用（与注入 coverage gate 的同一份，含 DB 真值哨兵 facts）。
    pub evidence_facts: Option<&'a [EvidenceFact]>,
}

pub(crate) struct RefineDecision {
    pub class: RefineClass,
    pub correction: String,
    /// `true` ⇒ 重试轮把 tool_choice 锁到 `submit_stage_deliverable`。
    pub submit_only_lock: bool,
}

/// 纯函数主入口：分类 → 渲染主因模板（+ 次因附录一行）→ 锁决策。
pub(crate) fn refine(input: &RefineInput<'_>) -> RefineDecision {
    let class = classify(input);
    let mut correction = match class {
        RefineClass::Fabricated => render_fabricated(input),
        RefineClass::SubmitOnly => render_submit_only(input),
        RefineClass::RedoStage => render_redo_stage(input),
        RefineClass::CoverageOrVacuous => render_coverage_or_vacuous(input),
        RefineClass::EvidenceQuality => render_evidence_quality(input),
        RefineClass::ScopingFlow => input
            .red_team_flow_correction
            .unwrap_or_default()
            .to_string(),
        // TextOnly 走独立入口 refine_text_only（gate 之前无 RefineInput），这里不可达；
        // 保守渲染成通用模板而非 panic。
        RefineClass::TextOnly | RefineClass::Generic => render_generic(input),
    };
    if let Some(note) = secondary_note(input, class) {
        correction.push_str(&note);
    }
    RefineDecision {
        class,
        correction,
        submit_only_lock: matches!(class, RefineClass::SubmitOnly),
    }
}

/// F · gate 之前的检测独立入口：响应是纯散文、无工具调用。
pub(crate) fn refine_text_only(subtask_title: &str) -> RefineDecision {
    RefineDecision {
        class: RefineClass::TextOnly,
        correction: format!(
            "Your previous response for subtask '{subtask_title}' was plain prose with no \
             tool calls — narration alone makes NO progress and cannot be verified. Take \
             concrete action now: run this stage's required tools to collect evidence, \
             then call `submit_stage_deliverable` citing the resulting evidence ids. Do \
             NOT restate plans or summaries; your next message must begin with a tool call."
        ),
        submit_only_lock: false,
    }
}

fn classify(input: &RefineInput<'_>) -> RefineClass {
    if !input.fabricated_ids.is_empty() {
        return RefineClass::Fabricated;
    }
    // A Scoping lifecycle blocker means the model still owes a typed host action
    // such as `ask_human(scope_review)` or the subsidiary-scope choice.  The
    // generic missing-deliverable rule must not turn that repair into a
    // submit-only retry: doing so removes the very tools required to satisfy the
    // deterministic gate and deadlocks the stage.
    if input.red_team_flow_correction.is_some() {
        return RefineClass::ScopingFlow;
    }
    if input.missing_deliverable {
        if input.confirm_only_stage || !input.available_real_ids.is_empty() {
            return RefineClass::SubmitOnly;
        }
        return RefineClass::RedoStage;
    }
    if reasons_hit_coverage_or_vacuous(input.gate_reasons) {
        return RefineClass::CoverageOrVacuous;
    }
    if !input.missing_kinds.is_empty() || !input.expired.is_empty() {
        return RefineClass::EvidenceQuality;
    }
    if input.red_team_flow_correction.is_some() {
        return RefineClass::ScopingFlow;
    }
    RefineClass::Generic
}

/// vacuous + coverage(complete / corroborated / denominator) 全走 C 类（设计 §5.1：
/// 诊断段从「仅 coverage BLOCK」扩展到 vacuous——live run 两连 BLOCK 缺的那块）。
fn reasons_hit_coverage_or_vacuous(reasons: &[String]) -> bool {
    reasons
        .iter()
        .any(|r| r.contains("vacuous") || r.contains("never attempted") || r.contains("corroborat"))
}

/// 主因之外的并存质量问题压成一行附录（防信号丢失，又不回到链式拼接的大杂烩）。
fn secondary_note(input: &RefineInput<'_>, class: RefineClass) -> Option<String> {
    if class == RefineClass::EvidenceQuality {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    if !input.missing_kinds.is_empty() {
        parts.push(format!("missing evidence kinds {:?}", input.missing_kinds));
    }
    if !input.expired.is_empty() {
        parts.push(format!(
            "hard-expired evidence {:?} (re-run those tools for fresh evidence)",
            input.expired
        ));
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("\n\nAlso fix: {}.", parts.join("; ")))
    }
}

// ── 模板（每类独立，禁止跨类拼接）────────────────────────────────────────────

/// D · 防伪造模板（迁自 execute.rs `block_outcome_for_fabricated`）。
fn render_fabricated(input: &RefineInput<'_>) -> String {
    let fabricated = input.fabricated_ids;
    let real_ids_hint = if input.available_real_ids.is_empty() {
        "Evidence ids are optional. Remove these fabricated id fields and resubmit, or \
         run the stage's required tools if the underlying DB truth is still missing."
            .to_string()
    } else {
        format!(
            "Real evidence ids already recorded for THIS operation (debug hint, newest first): {:?}. \
             Evidence ids are optional; remove fabricated ids unless you are certain a real id applies.",
            input.available_real_ids
        )
    };
    format!(
        "Your StageDeliverable cites evidence ids {fabricated:?} that do NOT exist in the \
         evidence ledger. Never substitute small guessed integers (1, 2, 3, …) for real ids. \
         {real_ids_hint} Do NOT guess, increment, or reuse placeholder ids. \
         Then resubmit a StageDeliverable with business facts; omit evidence id fields unless \
         they are real ledger ids."
    )
}

/// A · submit-only 模板（迁自 execute.rs `build_submit_only_correction`）+
/// confirm-only 无证据变体（设计 §5.1，PR-R3 起被 confirm-only missing 触达）。
fn render_submit_only(input: &RefineInput<'_>) -> String {
    if input.confirm_only_stage && input.available_real_ids.is_empty() {
        return format!(
            "The '{stage}' stage is confirm-only: it runs NO scan tools, so there is no \
             evidence to collect and nothing to re-do. Your ONLY remaining action is to call \
             the `submit_stage_deliverable` tool ONCE with a StageDeliverable containing a \
             single confirmation claim for this stage. Do NOT run tools, do NOT narrate — just submit.",
            stage = input.stage.as_str(),
        );
    }
    let id_list = input
        .available_real_ids
        .iter()
        .map(|id| match input.evidence_kind_labels.get(id) {
            Some(kind) => format!("#{id} ({kind})"),
            None => format!("#{id}"),
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Your '{stage}' stage run ALREADY did the scan work — its results are recorded in the \
         evidence ledger as: {id_list} (newest first). Do NOT re-run any tools and do NOT redo \
         the stage. Your ONLY remaining action is to call the `submit_stage_deliverable` tool \
         ONCE, with a StageDeliverable summarizing the business facts/coverage. You do NOT need \
         to copy these ids into the deliverable; the backend resolves ledger/DB truth. Do NOT \
         guess, increment, or invent ids.",
        stage = input.stage.as_str(),
    )
}

/// B · 重做模板（迁自 execute.rs `missing_deliverable_gate_outcome`）。
fn render_redo_stage(input: &RefineInput<'_>) -> String {
    format!(
        "Your output for the '{}' stage did not include a parseable StageDeliverable, \
         so the deterministic harness gate could not run. You MUST submit a StageDeliverable \
         — either by calling the submit_stage_deliverable tool, or by ending your next message \
         with a ```json fenced block containing a StageDeliverable (stage_id, claims, plus \
         optional findings/coverage). Re-do the stage work as needed and resubmit.",
        input.stage.as_str()
    )
}

/// C · 诊断式模板：gate 原因素体 + DB 真值现状。
/// vacuous 与 coverage BLOCK 都触发（设计 §5.1 对 PR-C 的扩展）。
fn render_coverage_or_vacuous(input: &RefineInput<'_>) -> String {
    let mut s = render_gate_reasons_body(input);
    if let Some(facts) = input.evidence_facts {
        if let Some(db_status) = build_db_truth_diagnosis(facts) {
            s.push_str(&db_status);
        }
    }
    s.push_str(
        "\nRepair the exact reported gap, re-collect evidence, and resubmit. The gate measures \
         persisted database truth rather than prose.\n",
    );
    s
}

/// E · 证据质量模板（迁自 enforce_evidence_kinds / enforce_evidence_freshness 文本）。
fn render_evidence_quality(input: &RefineInput<'_>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !input.missing_kinds.is_empty() {
        parts.push(format!(
            "This stage requires evidence of kinds {:?}, but the deliverable's evidence \
             includes none of them. Run the tools that produce these evidence kinds and resubmit \
             a StageDeliverable that cites them.",
            input.missing_kinds
        ));
    }
    if !input.expired.is_empty() {
        parts.push(format!(
            "Some cited evidence is hard-expired (older than 2x its max age): {:?}. \
             Re-run the relevant tools so the evidence is fresh, then resubmit a StageDeliverable \
             citing the new evidence ids.",
            input.expired
        ));
    }
    parts.join("\n\n")
}

/// 兜底 · 通用模板 = gate 原因素体（不附诊断段）。
fn render_generic(input: &RefineInput<'_>) -> String {
    render_gate_reasons_body(input)
}

/// gate 拒绝原因素体（迁自 execute.rs `build_gate_correction` 的非诊断部分）。
fn render_gate_reasons_body(input: &RefineInput<'_>) -> String {
    let mut s = String::from(
        "Your stage deliverable was REJECTED by the deterministic harness gate. \
         Fix the issues below and resubmit a corrected StageDeliverable — either by \
         calling the submit_stage_deliverable tool, or by ending your next message \
         with a corrected ```json StageDeliverable block.\n\n\
         ### Gate rejection reasons\n",
    );
    if input.gate_reasons.is_empty() {
        s.push_str("- (no specific reason reported)\n");
    } else {
        for r in input.gate_reasons {
            s.push_str(&format!("- {}\n", r));
        }
    }
    if let Some(rec) = input.gate_recovery {
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

// ── 诊断素材（迁自 execute.rs，全工作区唯一定义）────────────────────────────

/// 设计 2026-06-12 §5.4 · 被动情报 technique → 具体下一步命令建议（确定性表）。
/// `None` = 未知 technique（不臆造命令，保守）。`<asset>` 由模型按 in-scope 资产替换。
#[allow(dead_code)]
pub(crate) fn passive_intel_command_hint(technique: &str) -> Option<&'static str> {
    match technique {
        "GOLISH-INTEL-DNS" => Some(
            "run recon_map_assets(organization_id=<org>) and rely on provider DNS/host-IP landing; \
             the normal call auto-expands bounded owned apex domains; if no configured provider \
             can land DNS evidence for this asset, submit blocked or checked_empty with \
             evidence/note instead of running a scan-tool fallback",
        ),
        "GOLISH-INTEL-SUBDOMAIN" => Some(
            "recon_map_assets(organization_id=<org>) lands provider-backed subdomains to \
             target_assets and auto-expands bounded owned apex domains; if no provider can supply \
             this cell, record blocked/checked_empty instead of running CLI subdomain enumeration \
             in target_intel",
        ),
        "GOLISH-INTEL-WHOIS" => Some("run recon_lookup_whois(organization_id=<org>) once per org"),
        "GOLISH-INTEL-ASN" => Some(
            "run recon_map_assets(organization_id=<org>) and rely on provider ASN landing; if no \
             applicable provider exists, submit blocked/not_applicable with a note",
        ),
        "GOLISH-INTEL-CT" => Some(
            "run recon_map_assets(organization_id=<org>) and rely on provider certificate landing; \
             if the provider/source is unavailable, submit blocked with the source named",
        ),
        "GOLISH-INTEL-OSINT" => Some(
            "run recon_map_assets(organization_id=<org>) and rely on provider OSINT landing; if no \
             OSINT provider is configured, submit blocked with a note",
        ),
        _ => None,
    }
}

/// 被动情报 technique 全集（target_intel `expected_techniques` 镜像）。
#[allow(dead_code)]
pub(crate) const PASSIVE_INTEL_TECHNIQUES: &[&str] = &[
    "GOLISH-INTEL-DNS",
    "GOLISH-INTEL-WHOIS",
    "GOLISH-INTEL-ASN",
    "GOLISH-INTEL-CT",
    "GOLISH-INTEL-SUBDOMAIN",
    "GOLISH-INTEL-OSINT",
];

/// 设计 2026-06-12 §5.4 · 渲染「DB 真值现状」：该 run 已确认有数据的 (asset ×
/// technique)（仅 `Found`——`Empty` 是「跑了→空」不算「DB 已有数据」，I8）。
/// `None` = 无 Found 事实（不追加空段）。
pub(crate) fn build_db_truth_diagnosis(facts: &[EvidenceFact]) -> Option<String> {
    use crate::harness::gate::rule_engine::EvidenceOutcome;
    let mut found: Vec<String> = facts
        .iter()
        .filter(|f| f.outcome == EvidenceOutcome::Found)
        .map(|f| format!("- {} × {}", f.asset, f.technique))
        .collect();
    if found.is_empty() {
        return None;
    }
    found.sort();
    found.dedup();
    Some(format!(
        "\n### DB truth status (already persisted — do NOT re-run these)\n{}\n",
        found.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
    use std::collections::HashMap;

    fn base_input<'a>(reasons: &'a [String], kinds: &'a HashMap<i64, String>) -> RefineInput<'a> {
        RefineInput {
            stage: StageKind::Enumeration,
            gate_reasons: reasons,
            gate_recovery: None,
            missing_deliverable: false,
            confirm_only_stage: false,
            fabricated_ids: &[],
            available_real_ids: &[],
            evidence_kind_labels: kinds,
            missing_kinds: &[],
            expired: &[],
            red_team_flow_correction: None,
            evidence_facts: None,
        }
    }

    // ── 分类器（设计 §5.1 优先级表）─────────────────────────────────────────

    #[test]
    fn fabricated_wins_over_everything() {
        let reasons = vec!["deliverable vacuous: no claims".to_string()];
        let kinds = HashMap::new();
        let mut i = base_input(&reasons, &kinds);
        i.fabricated_ids = &[1, 2];
        i.missing_deliverable = true;
        let d = refine(&i);
        assert_eq!(d.class, RefineClass::Fabricated);
        assert!(!d.submit_only_lock);
    }

    #[test]
    fn missing_with_real_ids_locks_submit_only() {
        let kinds = HashMap::new();
        let mut i = base_input(&[], &kinds);
        i.missing_deliverable = true;
        i.available_real_ids = &[2247, 2245];
        let d = refine(&i);
        assert_eq!(d.class, RefineClass::SubmitOnly);
        assert!(
            d.submit_only_lock,
            "submit-only 锁必须触发（修复投影兜底截胡 bug 的回归锚点）"
        );
    }

    #[test]
    fn confirm_only_missing_locks_submit_only_without_ids() {
        let kinds = HashMap::new();
        let mut i = base_input(&[], &kinds);
        i.missing_deliverable = true;
        i.confirm_only_stage = true;
        let d = refine(&i);
        assert_eq!(d.class, RefineClass::SubmitOnly);
        assert!(d.submit_only_lock);
    }

    #[test]
    fn missing_with_empty_ledger_redoes_stage() {
        let kinds = HashMap::new();
        let mut i = base_input(&[], &kinds);
        i.missing_deliverable = true;
        let d = refine(&i);
        assert_eq!(d.class, RefineClass::RedoStage);
        assert!(!d.submit_only_lock);
    }

    #[test]
    fn vacuous_routes_to_coverage_class() {
        let reasons =
            vec!["deliverable vacuous: no claims, no findings, no skipped_checks".to_string()];
        let kinds = HashMap::new();
        let d = refine(&base_input(&reasons, &kinds));
        assert_eq!(d.class, RefineClass::CoverageOrVacuous);
    }

    #[test]
    fn coverage_never_attempted_routes_to_coverage_class() {
        let reasons = vec!["directory discovery on app.example.com never attempted".to_string()];
        let kinds = HashMap::new();
        let d = refine(&base_input(&reasons, &kinds));
        assert_eq!(d.class, RefineClass::CoverageOrVacuous);
    }

    #[test]
    fn quality_marks_route_to_evidence_quality() {
        let kinds = HashMap::new();
        let missing = vec!["dns_a".to_string()];
        let mut i = base_input(&[], &kinds);
        i.missing_kinds = &missing;
        let d = refine(&i);
        assert_eq!(d.class, RefineClass::EvidenceQuality);
        assert!(d.correction.contains("dns_a"));
    }

    #[test]
    fn red_team_flow_correction_is_passed_through() {
        let kinds = HashMap::new();
        let mut i = base_input(&[], &kinds);
        i.red_team_flow_correction = Some("RUN-THE-UNIT-REVIEW-FLOW");
        let d = refine(&i);
        assert_eq!(d.class, RefineClass::ScopingFlow);
        assert_eq!(d.correction, "RUN-THE-UNIT-REVIEW-FLOW");
    }

    #[test]
    fn missing_scoping_lifecycle_never_locks_required_human_tools() {
        let kinds = HashMap::new();
        let mut i = base_input(&[], &kinds);
        i.stage = StageKind::Scoping;
        i.missing_deliverable = true;
        i.confirm_only_stage = true;
        i.available_real_ids = &[2247];
        i.red_team_flow_correction = Some(
            "SCOPING TARGET REVIEW INCOMPLETE — exactly one scope_review is required for a non-empty trusted snapshot, but 0 were observed; no successful parseable scope_review was persisted.",
        );

        let d = refine(&i);

        assert_eq!(d.class, RefineClass::ScopingFlow);
        assert!(!d.submit_only_lock);
        assert!(d.correction.contains("scope_review"));
    }

    #[test]
    fn unmatched_block_falls_back_to_generic() {
        let reasons = vec!["finding count below skeleton minimum".to_string()];
        let kinds = HashMap::new();
        let d = refine(&base_input(&reasons, &kinds));
        assert_eq!(d.class, RefineClass::Generic);
        assert!(d
            .correction
            .contains("finding count below skeleton minimum"));
    }

    #[test]
    fn secondary_note_appends_when_quality_coexists() {
        let reasons = vec!["deliverable vacuous: no claims".to_string()];
        let kinds = HashMap::new();
        let expired = ["evidence #99 (dns_a) is hard-expired".to_string()];
        let mut i = base_input(&reasons, &kinds);
        i.expired = &expired;
        let d = refine(&i);
        assert_eq!(d.class, RefineClass::CoverageOrVacuous);
        assert!(d.correction.contains("Also fix:"));
        assert!(d.correction.contains("99"));
    }

    // ── 模板要素（设计 §5.2）────────────────────────────────────────────────

    #[test]
    fn submit_only_template_lists_real_ids_with_kind_labels() {
        let mut kinds = HashMap::new();
        kinds.insert(2247i64, "dns_a".to_string());
        let mut i = base_input(&[], &kinds);
        i.missing_deliverable = true;
        i.available_real_ids = &[2247];
        let d = refine(&i);
        assert!(d.correction.contains("#2247 (dns_a)"));
        assert!(d.correction.contains("submit_stage_deliverable"));
        assert!(d.correction.contains("Do NOT re-run"));
    }

    #[test]
    fn confirm_only_template_says_submit_without_evidence_ids() {
        let kinds = HashMap::new();
        let mut i = base_input(&[], &kinds);
        i.missing_deliverable = true;
        i.confirm_only_stage = true;
        let d = refine(&i);
        assert!(d.correction.contains("confirm-only"));
        assert!(d.correction.contains("single confirmation claim"));
        assert!(!d.correction.contains("evidence_ids may be empty"));
    }

    #[test]
    fn coverage_template_includes_generic_db_diagnosis_for_vacuous() {
        let reasons = vec!["deliverable vacuous: no claims".to_string()];
        let kinds = HashMap::new();
        let facts = vec![EvidenceFact {
            asset: "moresec.cn".into(),
            technique: "CONTENT-DISCOVERY".into(),
            outcome: EvidenceOutcome::Found,
            evidence_id: 0,
        }];
        let mut i = base_input(&reasons, &kinds);
        i.evidence_facts = Some(&facts);
        let d = refine(&i);
        assert!(d.correction.contains("DB truth status"));
        assert!(d.correction.contains("moresec.cn × CONTENT-DISCOVERY"));
        assert!(d.correction.contains("Repair the exact reported gap"));
    }

    #[test]
    fn coverage_template_omits_db_section_when_no_found_facts() {
        let reasons = vec!["directory discovery on x never attempted".to_string()];
        let kinds = HashMap::new();
        let d = refine(&base_input(&reasons, &kinds));
        assert!(!d.correction.contains("DB truth status"));
        assert!(d.correction.contains("Repair the exact reported gap"));
    }

    #[test]
    fn fabricated_template_names_fake_and_real_ids() {
        let kinds = HashMap::new();
        let mut i = base_input(&[], &kinds);
        i.fabricated_ids = &[1, 2, 3];
        i.available_real_ids = &[2247];
        let d = refine(&i);
        assert!(d.correction.contains("[1, 2, 3]"));
        assert!(d.correction.contains("2247"));
        assert!(d.correction.contains("do NOT exist in the"));
    }

    #[test]
    fn fabricated_template_without_real_ids_prefers_omission() {
        let kinds = HashMap::new();
        let mut i = base_input(&[], &kinds);
        i.fabricated_ids = &[7];
        let d = refine(&i);
        assert!(d.correction.contains("Evidence ids are optional"));
        assert!(d.correction.contains("Remove these fabricated id fields"));
    }

    #[test]
    fn redo_template_demands_a_deliverable() {
        let kinds = HashMap::new();
        let mut i = base_input(&[], &kinds);
        i.missing_deliverable = true;
        let d = refine(&i);
        assert!(d
            .correction
            .contains("did not include a parseable StageDeliverable"));
        assert!(d.correction.contains("Re-do the stage work"));
    }

    #[test]
    fn generic_body_renders_recovery_sections() {
        let reasons = vec!["finding count below skeleton minimum".to_string()];
        let kinds = HashMap::new();
        let rec = crate::harness::HarnessRecoveryActions {
            hints: vec!["hint-1".into()],
            repair_tool_calls: vec!["nuclei -u <asset>".into()],
            missing_evidence_kinds: vec!["http_probe".into()],
            ..Default::default()
        };
        let mut i = base_input(&reasons, &kinds);
        i.gate_recovery = Some(&rec);
        let d = refine(&i);
        assert!(d.correction.contains("Required tool calls"));
        assert!(d.correction.contains("nuclei -u <asset>"));
        assert!(d.correction.contains("Missing evidence to collect"));
        assert!(d.correction.contains("http_probe"));
        assert!(d.correction.contains("Hints"));
        assert!(d.correction.contains("hint-1"));
    }

    #[test]
    fn text_only_template_demands_tool_call_not_redo() {
        let d = refine_text_only("Passive Target Intelligence");
        assert_eq!(d.class, RefineClass::TextOnly);
        assert!(d.correction.contains("must begin with a tool call"));
        assert!(!d.correction.contains("re-do the stage"));
        assert!(!d.submit_only_lock);
    }
}
