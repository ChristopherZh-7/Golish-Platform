//! Gate validator (Doc 3 §8) 调度入口.
//!
//! Phase 1c.2 skeleton · 5 个 check (schema / scope / contract / vacuous /
//! freshness) 占位.
//!
//! Doc 4 (`docs/design/2026-05-26-harness-observability-plane.md`) 预留的
//! Observability ids 字段 (`gate_result_id` / `blocking_reason_id`) 已加入
//! [`GateResult`], Phase 2 完整 wiring 时填.

pub mod context_builder;
pub mod contract_check;
pub mod finding_verification_check;
pub mod freshness_check;
pub mod min_invocations_check;
pub mod rule_engine;
pub mod schema_check;
pub mod scope_check;
pub mod surface_coverage_check;
pub mod vacuous_check;

pub use context_builder::GateContextBuilder;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::sprint_contract::{SprintContract, StageSkeleton};
use super::stage_spec::StageSpec;
use super::types::{HarnessRecoveryActions, StageDeliverable};

/// 单个 check 的结果 (gate/mod 聚合用).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GateCheckOutcome {
    Pass,
    Block {
        reasons: Vec<String>,
        recovery: HarnessRecoveryActions,
    },
}

impl GateCheckOutcome {
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }
}

/// Doc 3 §8 GateResult · 5 check 聚合结果.
///
/// Doc 4 §6 raw event refs (gate_result_id / blocking_reason_id) 占位字段
/// 留 Option<Uuid>, Phase 1 不填; 推 Phase 2 落 Observability Plane 时 fill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateResult {
    pub allowed: bool,
    pub reasons: Vec<String>,
    pub recovery_actions: Option<HarnessRecoveryActions>,
    /// Doc 4 §6 reserved · Phase 1 = None.
    #[serde(default)]
    pub gate_result_id: Option<Uuid>,
    /// Doc 4 §6 reserved · Phase 1 = None.
    #[serde(default)]
    pub blocking_reason_id: Option<Uuid>,
}

impl GateResult {
    pub fn pass() -> Self {
        Self {
            allowed: true,
            reasons: vec![],
            recovery_actions: None,
            gate_result_id: None,
            blocking_reason_id: None,
        }
    }

    pub fn block(reasons: Vec<String>, recovery: HarnessRecoveryActions) -> Self {
        Self {
            allowed: false,
            reasons,
            recovery_actions: Some(recovery),
            gate_result_id: None,
            blocking_reason_id: None,
        }
    }
}

/// Doc 3 §8.1 通用 gate 入口 (Phase B) · 按 StageSpec 跑结构性 check + spec
/// 选择的语义 check, 适用任意 stage.
///
/// **结构性 check** (schema / contract / vacuous / freshness / finding_verification)
/// 永远跑: 与 stage 语义无关, 只看 deliverable 形状 / 契约 / 时效.
///
/// **语义层** 由 `spec.gate_rules` 单一声明（gate-rules-migration 2026-06-05）:
/// 简单标准用数据积木 (count_at_least / for_all), 领域/遗留逻辑 (scope /
/// surface_coverage / min_invocations) 用 `named_check` 积木按名调用。旧
/// `required_checks` 固定菜单已删除。
pub fn validate_stage_gate(
    deliverable: &StageDeliverable,
    spec: &StageSpec,
    contract: Option<&SprintContract>,
) -> GateResult {
    validate_stage_gate_with_skeleton(deliverable, spec, contract, None)
}

/// 同 [`validate_stage_gate`], 但额外接受一个 per-stage [`StageSkeleton`] 以启用
/// **per-target 强校验**: contract_check 会按 skeleton 的 `expected_count_range`
/// (每类 finding 数量区间) 与 `min_tool_invocations` 比对 deliverable.
///
/// `skeleton = None` 时与旧 [`validate_stage_gate`] 完全等价 (向后兼容); 现网灰度
/// 由 `sprint_skeleton_enforcement_enabled()` 控制是否在 gate hook 传入 skeleton.
pub fn validate_stage_gate_with_skeleton(
    deliverable: &StageDeliverable,
    spec: &StageSpec,
    contract: Option<&SprintContract>,
    skeleton: Option<&StageSkeleton>,
) -> GateResult {
    validate_stage_gate_with_context(
        deliverable,
        spec,
        contract,
        skeleton,
        &rule_engine::GateContext::default(),
    )
}

/// 同 [`validate_stage_gate_with_skeleton`]，但额外接受 [`rule_engine::GateContext`] 注入
/// **权威 in-scope 资产集**（①）与/或**动态期望技术**（③）——Phase 2 seam（设计 §6.5）。
///
/// 有效上下文合并规则：`ctx` 字段优先；当 `ctx.expected_techniques` 为 `None` 时，回退用
/// `skeleton.expected_techniques`（③ 动态生成产物，非空才用），再 `None` 由
/// `coverage_complete` 回退 `spec.expected_techniques`（静态）。资产维度仅取 `ctx.in_scope_assets`
/// （① 外层查库后注入；活体接线待资产库 + DB §2.7）。gate 仍纯函数 / DB-free。
pub fn validate_stage_gate_with_context(
    deliverable: &StageDeliverable,
    spec: &StageSpec,
    contract: Option<&SprintContract>,
    skeleton: Option<&StageSkeleton>,
    ctx: &rule_engine::GateContext,
) -> GateResult {
    let mut outcomes = vec![
        schema_check::run(deliverable, spec),
        contract_check::run_with_skeleton(deliverable, contract, skeleton),
        vacuous_check::run(deliverable, spec, ctx.evidence_facts.as_deref()),
        freshness_check::run(deliverable, spec),
        // P2 · config-driven verification (no-op unless the stage spec declares
        // finding_verification / min_findings / min_claims).
        finding_verification_check::run(deliverable, spec),
    ];

    // ③ seam：skeleton 动态生成的 expected_techniques 在 ctx 未显式指定时作为有效期望技术
    // 注入 gate（覆盖 spec 静态值）；ctx 显式指定则尊重 ctx。资产维度透传 ctx（①）。
    let effective_ctx = rule_engine::GateContext {
        in_scope_assets: ctx.in_scope_assets.clone(),
        // Host-aware coverage 2c：权威资产类型原样透传给规则求值（None = 按值推断）。
        asset_types: ctx.asset_types.clone(),
        expected_techniques: ctx.expected_techniques.clone().or_else(|| {
            skeleton
                .map(|s| s.expected_techniques.clone())
                .filter(|t| !t.is_empty())
        }),
        // PR3 · 账本投影事实原样透传（None = 不启用投影）。
        evidence_facts: ctx.evidence_facts.clone(),
        // Source-query terminal facts原样透传（None = source_coverage 不消费）。
        source_queries: ctx.source_queries.clone(),
    };

    // 语义层 · 过关标准的**唯一入口**（gate-rules-migration 2026-06-05）。旧
    // `required_checks` 固定菜单 match（含 `_ => continue` 静默忽略）已删除：
    // scope / surface_coverage / min_invocations 现以 `named_check` 积木从
    // `gate_rules` 调用，简单标准（claim/finding 证据非空等）用数据积木声明。
    // 写错 op/check 名由 typed-enum 在 spec 加载期 fail-closed。空 gate_rules =
    // 仅跑上面 5 个结构层 check。
    outcomes.extend(rule_engine::eval_with_context(
        deliverable,
        spec,
        &spec.gate_rules,
        &effective_ctx,
    ));

    aggregate(outcomes)
}

/// scoping 人工确认硬门禁规则（设计 2026-06-06-scoping-per-mode-gate-hitl §3.4）。
///
/// 由 gate hook 按 `profile.scoping_policy.require_human_scope_approval` 注入（per-profile
/// 启用，smoke 不注入）：要求 deliverable 至少 1 条 `kind="scope_human_approved"` 的 claim，
/// 否则 Block，不许离开 scoping。用现有 `count_at_least` 数据积木声明，无需改 gate 引擎。
pub fn scoping_human_gate_rule() -> rule_engine::GateRule {
    rule_engine::GateRule::CountAtLeast {
        over: rule_engine::Collection::Claims,
        filter: Some(rule_engine::Pred::Eq {
            field: rule_engine::ItemField::Kind,
            value: "scope_human_approved".to_string(),
        }),
        min: 1,
        on_fail: rule_engine::OnFail {
            reason: "scope must be human-confirmed before leaving scoping".to_string(),
            hints: vec![
                "call ask_human(input_type=\"scope_review\") and let the user confirm/edit the target list".to_string(),
                "after the user approves, add a claim {kind:\"scope_human_approved\", subject:<engagement subject>} that cites the ask_human request_id".to_string(),
            ],
            repair_tool_calls: vec!["ask_human".to_string()],
            missing_evidence_kinds: vec![],
        },
    }
}

/// 把多个 check outcome 聚合为单个 GateResult (合并 reasons + recovery).
fn aggregate(outcomes: Vec<GateCheckOutcome>) -> GateResult {
    let mut reasons = Vec::new();
    let mut recovery = HarnessRecoveryActions::default();

    for outcome in outcomes {
        if let GateCheckOutcome::Block {
            reasons: r,
            recovery: rec,
        } = outcome
        {
            reasons.extend(r);
            recovery.hints.extend(rec.hints);
            recovery.repair_tool_calls.extend(rec.repair_tool_calls);
            recovery
                .missing_evidence_kinds
                .extend(rec.missing_evidence_kinds);
        }
    }

    if reasons.is_empty() {
        GateResult::pass()
    } else {
        GateResult::block(reasons, recovery)
    }
}

/// 薄包装 · 保留旧调用方与 e2e 单测 (= 跑 external_attack_surface spec 的通用 gate).
pub fn validate_external_attack_surface_gate(
    deliverable: &StageDeliverable,
    spec: &StageSpec,
    contract: Option<&SprintContract>,
) -> GateResult {
    validate_stage_gate(deliverable, spec, contract)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_result_pass_constructor() {
        let r = GateResult::pass();
        assert!(r.allowed);
        assert!(r.reasons.is_empty());
        assert!(r.recovery_actions.is_none());
        assert!(r.gate_result_id.is_none());
        assert!(r.blocking_reason_id.is_none());
    }

    #[test]
    fn gate_result_block_constructor() {
        let r = GateResult::block(
            vec!["missing dns_a".to_string()],
            HarnessRecoveryActions::default(),
        );
        assert!(!r.allowed);
        assert_eq!(r.reasons, vec!["missing dns_a".to_string()]);
        assert!(r.recovery_actions.is_some());
    }

    #[test]
    fn gate_check_outcome_is_pass_predicate() {
        assert!(GateCheckOutcome::Pass.is_pass());
        let block = GateCheckOutcome::Block {
            reasons: vec!["x".to_string()],
            recovery: HarnessRecoveryActions::default(),
        };
        assert!(!block.is_pass());
    }

    #[test]
    fn skeleton_enforces_per_target_finding_range() {
        use super::super::sprint_contract::{ExpectedFinding, StageSkeleton};
        use super::super::stage_spec::load_stage_spec_from_json;
        use super::super::types::StageClaim;
        use golish_pentest::evidence_ledger::EvidenceAuditId;
        use std::collections::HashMap;

        const SCOPING_JSON: &str =
            include_str!("../../../../../../resources/harness/stages/scoping/spec.json");
        let spec = load_stage_spec_from_json(SCOPING_JSON).unwrap();

        // A scoping deliverable that PASSES the baseline gate: one evidence-backed
        // claim (non-vacuous); scoping has no surface/min-invocation checks.
        let deliverable = StageDeliverable {
            stage_id: "scoping".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![StageClaim {
                kind: "scope_confirmed".to_string(),
                subject: "example.com".to_string(),
                summary: "in scope".to_string(),
                evidence_ids: vec![EvidenceAuditId::new(1)],
                technique: None,
            }],
            evidence_refs: vec![EvidenceAuditId::new(1)],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: vec![],
            coverage: vec![],
        };

        // Baseline (skeleton = None) passes.
        assert!(validate_stage_gate(&deliverable, &spec, None).allowed);

        // A per-target skeleton requiring >=1 `subdomain` finding turns the same
        // (0-subdomain) deliverable into a BLOCK — proving the skeleton is wired
        // through validate_stage_gate_with_skeleton into contract_check.
        let skeleton = StageSkeleton {
            expected_findings: vec![ExpectedFinding {
                kind: "subdomain".to_string(),
                expected_count_range: [1, 9],
                required_evidence_kinds: vec![],
            }],
            time_budget_minutes: 10,
            min_tool_invocations: HashMap::new(),
            expected_techniques: vec![],
        };
        let blocked = validate_stage_gate_with_skeleton(&deliverable, &spec, None, Some(&skeleton));
        assert!(!blocked.allowed);
        assert!(blocked
            .reasons
            .iter()
            .any(|r| r.contains("subdomain") && r.contains("below contract minimum")));
    }

    #[test]
    fn scoping_human_gate_rule_blocks_without_approval_and_passes_with() {
        use super::super::stage_spec::load_stage_spec_from_json;
        use super::super::types::StageClaim;
        use golish_pentest::evidence_ledger::EvidenceAuditId;

        const SCOPING_JSON: &str =
            include_str!("../../../../../../resources/harness/stages/scoping/spec.json");
        let mut spec = load_stage_spec_from_json(SCOPING_JSON).unwrap();
        // What the gate hook injects for a profile with require_human_scope_approval.
        spec.gate_rules.push(scoping_human_gate_rule());

        // scope_confirmed only (evidence-backed so the baseline gate passes) — the
        // injected rule must still BLOCK because there is no human-approval claim.
        let mut d = StageDeliverable {
            stage_id: "scoping".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![StageClaim {
                kind: "scope_confirmed".to_string(),
                subject: "example.com".to_string(),
                summary: "in scope".to_string(),
                evidence_ids: vec![EvidenceAuditId::new(1)],
                technique: None,
            }],
            evidence_refs: vec![EvidenceAuditId::new(1)],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: vec![],
            coverage: vec![],
        };
        assert!(
            !validate_stage_gate(&d, &spec, None).allowed,
            "scoping must BLOCK without a scope_human_approved claim"
        );

        // Add the human-approval claim → PASS.
        d.claims.push(StageClaim {
            kind: "scope_human_approved".to_string(),
            subject: "example.com".to_string(),
            summary: "user approved 3 targets".to_string(),
            evidence_ids: vec![EvidenceAuditId::new(1)],
            technique: None,
        });
        assert!(
            validate_stage_gate(&d, &spec, None).allowed,
            "scoping must PASS once the user has approved the scope"
        );
    }

    #[test]
    fn gate_rules_block_propagates_through_aggregate() {
        use super::super::stage_spec::load_stage_spec_from_json;
        use super::super::types::{FindingSeverity, HarnessFinding, StageClaim};
        use golish_pentest::evidence_ledger::EvidenceAuditId;

        // spec：一条 gate_rule 要求每个 high+ finding 挂证据；无 required_checks
        // (隔离 gate_rule 行为：scope_check 不会跑)，其余字段走 serde default。
        let spec_json = r#"{
            "id":"verification","kind":"verification","risk_level":"critical",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
            "gate_rules":[
              { "op":"for_all","over":"findings",
                "where":{"pred":"severity_at_least","min":"high"},
                "require":{"pred":"non_empty","field":"evidence_refs"},
                "on_fail":{"reason":"GATE_RULE: high+ finding needs evidence"} }
            ]
        }"#;
        let spec = load_stage_spec_from_json(spec_json).unwrap();

        // 基线：1 个挂证据的 critical finding + 1 个挂证据的 claim → 过所有结构 check
        // 且过 gate_rule。
        let eid = EvidenceAuditId::new(1);
        let deliverable = StageDeliverable {
            stage_id: "verification".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![StageClaim {
                kind: "exploit".to_string(),
                subject: "api.example.com".to_string(),
                summary: "verified".to_string(),
                evidence_ids: vec![eid],
                technique: None,
            }],
            evidence_refs: vec![eid],
            skipped_checks: vec![],
            findings: vec![HarnessFinding {
                finding_id: Uuid::new_v4(),
                kind: "rce".to_string(),
                subject: "api.example.com".to_string(),
                severity: FindingSeverity::Critical,
                evidence_refs: vec![eid],
                technique: None,
            }],
            required_checks_done: vec![],
            coverage: vec![],
        };
        let base = validate_stage_gate(&deliverable, &spec, None);
        assert!(base.allowed, "baseline should pass: {:?}", base.reasons);

        // 追加一个不挂证据的 critical finding → gate_rule 触发 → 整体 Block。
        let mut bad = deliverable.clone();
        bad.findings.push(HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: "rce".to_string(),
            subject: "db.example.com".to_string(),
            severity: FindingSeverity::Critical,
            evidence_refs: vec![],
            technique: None,
        });
        let blocked = validate_stage_gate(&bad, &spec, None);
        assert!(!blocked.allowed);
        assert!(blocked.reasons.iter().any(|r| r.contains("GATE_RULE")));
    }

    // ── gate-rules-migration (2026-06-05) 等价性回归 ──────────────────────────
    // 证明删 required_checks 后，迁移到 gate_rules 的内嵌 spec 仍接住旧 scope /
    // min_invocations 语义（行为零变更，设计 §7）。

    #[test]
    fn migrated_eas_scope_rule_blocks_claim_without_evidence() {
        use super::super::resources::load_embedded_stage_spec;
        use super::super::types::{StageClaim, StageKind};

        let spec = load_embedded_stage_spec(StageKind::ExternalAttackSurface).unwrap();
        // 一个缺 evidence_ids 的 claim → 迁移后的 scope×2 数据规则应 Block。
        let d = StageDeliverable {
            stage_id: "external_attack_surface".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![StageClaim {
                kind: "http_service".to_string(),
                subject: "api.example.com".to_string(),
                summary: "200".to_string(),
                evidence_ids: vec![],
                technique: None,
            }],
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: vec![],
            coverage: vec![],
        };
        let result = validate_stage_gate(&d, &spec, None);
        assert!(!result.allowed);
        assert!(
            result
                .reasons
                .iter()
                .any(|r| r.contains("must cite evidence")),
            "scope×2 gate_rule should fire: {:?}",
            result.reasons
        );
    }

    #[test]
    fn migrated_enumeration_named_min_invocations_blocks_when_tool_absent() {
        use super::super::resources::load_embedded_stage_spec;
        use super::super::types::{FindingSeverity, HarnessFinding, StageKind};
        use golish_pentest::evidence_ledger::EvidenceAuditId;

        let spec = load_embedded_stage_spec(StageKind::Enumeration).unwrap();
        // 非 vacuous + 证据足够过 scope/vacuous，但 required_checks_done 不含 http_probe
        // → 迁移后的 named_check:min_invocations 应 Block（reason 含 min tool invocations）。
        let d = StageDeliverable {
            stage_id: "enumeration".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![],
            evidence_refs: vec![EvidenceAuditId::new(1)],
            skipped_checks: vec![],
            findings: vec![HarnessFinding {
                finding_id: Uuid::new_v4(),
                kind: "open_port".to_string(),
                subject: "api.example.com:443".to_string(),
                severity: FindingSeverity::Info,
                evidence_refs: vec![EvidenceAuditId::new(1)],
                technique: None,
            }],
            required_checks_done: vec![],
            coverage: vec![],
        };
        let result = validate_stage_gate(&d, &spec, None);
        assert!(!result.allowed);
        assert!(
            result
                .reasons
                .iter()
                .any(|r| r.contains("min tool invocations")),
            "named_check:min_invocations should fire: {:?}",
            result.reasons
        );
    }

    // ── coverage matrix / technique-matrix 样例接入 ───────────────────────────
    // coverage-matrix Task 6 + #4 + vuln-triage-technique-matrix T6：用迁移后的
    // vuln_triage embedded spec（15 expected_techniques + coverage_complete +
    // found/checked_empty 证据规则 + coverage_denominator）：完整覆盖 → Pass；删一格
    // 期望技术 → coverage_complete Block；checked_empty 清空证据 → 证据规则 Block；
    // 某格 tested<total 无 rationale → coverage_denominator Block。

    /// 覆盖全部 15 类期望技术的 coverage（found/checked_empty 挂证据 + tested==total
    /// 满足分母全覆盖；not_applicable 免分母免证据）。供下面两个集成测试复用。
    fn full_vuln_triage_coverage(
        asset: &str,
        eid: golish_pentest::evidence_ledger::EvidenceAuditId,
    ) -> Vec<super::super::types::CoverageCell> {
        use super::super::types::{CoverageCell, CoverageStatus};
        let ce = |tech: &str| CoverageCell {
            asset: asset.into(),
            technique: tech.into(),
            status: CoverageStatus::CheckedEmpty,
            evidence_refs: vec![eid],
            note: Some("scanned, no finding".into()),
            reason_kind: None,
            tested_units: 1,
            total_units: 1,
            sampling_rationale: None,
        };
        vec![
            CoverageCell {
                asset: asset.into(),
                technique: "WSTG-INPV-05".into(),
                status: CoverageStatus::Found,
                evidence_refs: vec![eid],
                note: None,
                reason_kind: None,
                tested_units: 1,
                total_units: 1,
                sampling_rationale: None,
            },
            ce("WSTG-INPV-01"),
            ce("WSTG-INPV-12"),
            ce("WSTG-INPV-18"),
            CoverageCell {
                asset: asset.into(),
                technique: "WSTG-INPV-19".into(),
                status: CoverageStatus::NotApplicable,
                evidence_refs: vec![],
                note: Some("no outbound fetch surface".into()),
                reason_kind: None,
                tested_units: 0,
                total_units: 0,
                sampling_rationale: None,
            },
            ce("WSTG-ATHZ-04"),
            ce("WSTG-ATHZ-01"),
            ce("WSTG-ATHN-04"),
            ce("WSTG-ATHN-02"),
            ce("WSTG-SESS-02"),
            ce("WSTG-CONF-05"),
            ce("WSTG-CRYP-03"),
            ce("WSTG-BUSL"),
            ce("WSTG-INFO"),
            ce("GOLISH-NDAY"),
        ]
    }

    /// vuln_triage 的「全过关」deliverable（1 claim + 1 finding 挂证据 + 15 格全覆盖）。
    fn vuln_triage_pass_deliverable(
        asset: &str,
        eid: golish_pentest::evidence_ledger::EvidenceAuditId,
    ) -> StageDeliverable {
        use super::super::types::{FindingSeverity, HarnessFinding, StageClaim};
        StageDeliverable {
            stage_id: "vuln_triage".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![StageClaim {
                kind: "vuln".into(),
                subject: asset.into(),
                summary: "sqli confirmed".into(),
                evidence_ids: vec![eid],
                technique: None,
            }],
            evidence_refs: vec![eid],
            skipped_checks: vec![],
            findings: vec![HarnessFinding {
                finding_id: Uuid::new_v4(),
                kind: "sqli".into(),
                subject: asset.into(),
                severity: FindingSeverity::High,
                evidence_refs: vec![eid],
                technique: None,
            }],
            required_checks_done: vec![],
            coverage: full_vuln_triage_coverage(asset, eid),
        }
    }

    #[test]
    fn vuln_triage_coverage_gate_blocks_on_gap_and_passes_when_complete() {
        use super::super::resources::load_embedded_stage_spec;
        use super::super::types::{CoverageStatus, StageKind};
        use golish_pentest::evidence_ledger::EvidenceAuditId;

        let spec = load_embedded_stage_spec(StageKind::VulnTriage).unwrap();
        // sanity：样例声明 15 类期望技术（技术矩阵 §3）。
        assert_eq!(spec.expected_techniques.len(), 15);

        let eid = EvidenceAuditId::new(1);
        let asset = "api.example.com";
        let pass_deliverable = vuln_triage_pass_deliverable(asset, eid);
        let pass = validate_stage_gate(&pass_deliverable, &spec, None);
        assert!(
            pass.allowed,
            "full coverage should pass: {:?}",
            pass.reasons
        );

        // 删掉 GOLISH-NDAY 那格 → coverage_complete 应 Block 且 reason 含缺口。
        let mut incomplete = pass_deliverable.clone();
        incomplete.coverage.retain(|c| c.technique != "GOLISH-NDAY");
        let blocked = validate_stage_gate(&incomplete, &spec, None);
        assert!(!blocked.allowed);
        assert!(
            blocked
                .reasons
                .iter()
                .any(|r| r.contains("coverage incomplete") && r.contains("GOLISH-NDAY")),
            "coverage_complete should fire naming the gap: {:?}",
            blocked.reasons
        );

        // #4（用户拍板「checked_empty 也要证据」）：把一个 checked_empty 格的证据清空 →
        // vuln_triage 的 checked_empty 证据规则应 Block（I8：已检查为空 ≠ 未检查）。
        let mut empty_ce = pass_deliverable.clone();
        let cleared = empty_ce
            .coverage
            .iter_mut()
            .find(|c| c.status == CoverageStatus::CheckedEmpty)
            .map(|c| c.evidence_refs.clear());
        assert!(
            cleared.is_some(),
            "fixture must contain a checked_empty cell"
        );
        let blocked_ce = validate_stage_gate(&empty_ce, &spec, None);
        assert!(
            !blocked_ce.allowed,
            "checked_empty without evidence must block"
        );
        assert!(
            blocked_ce
                .reasons
                .iter()
                .any(|r| r.contains("checked_empty") && r.contains("must cite evidence")),
            "checked_empty evidence rule should fire: {:?}",
            blocked_ce.reasons
        );
    }

    #[test]
    fn vuln_triage_denominator_blocks_partial_and_passes_when_full() {
        use super::super::resources::load_embedded_stage_spec;
        use super::super::types::{CoverageStatus, StageKind};
        use golish_pentest::evidence_ledger::EvidenceAuditId;

        let spec = load_embedded_stage_spec(StageKind::VulnTriage).unwrap();
        let eid = EvidenceAuditId::new(1);
        let asset = "api.example.com";

        // 基线全覆盖（每格 tested==total）→ Pass。
        let base = vuln_triage_pass_deliverable(asset, eid);
        assert!(
            validate_stage_gate(&base, &spec, None).allowed,
            "full-denominator coverage should pass"
        );

        // 把一个 checked_empty 格改成 tested 3 / total 5000 且无 rationale →
        // coverage_denominator 应 Block，reason 含 "3/5000"。
        let mut partial = base.clone();
        if let Some(c) = partial
            .coverage
            .iter_mut()
            .find(|c| c.status == CoverageStatus::CheckedEmpty)
        {
            c.tested_units = 3;
            c.total_units = 5000;
            c.sampling_rationale = None;
        }
        let blocked = validate_stage_gate(&partial, &spec, None);
        assert!(!blocked.allowed);
        assert!(
            blocked.reasons.iter().any(|r| r.contains("3/5000")),
            "coverage_denominator should fire with N/M: {:?}",
            blocked.reasons
        );

        // 把该格补成全覆盖（tested==total）→ 重新 Pass（embedded min_sample_ratio_pct=100）。
        let mut fixed = partial.clone();
        if let Some(c) = fixed
            .coverage
            .iter_mut()
            .find(|c| c.tested_units == 3 && c.total_units == 5000)
        {
            c.tested_units = 5000;
        }
        assert!(
            validate_stage_gate(&fixed, &spec, None).allowed,
            "tested==total should clear the denominator gate"
        );
    }

    // ── Phase 2 ③ seam（设计 §6.5）：skeleton 动态 expected_techniques 驱动 coverage_complete ──

    #[test]
    fn skeleton_expected_techniques_drive_coverage_complete() {
        // spec.expected_techniques 空（coverage_complete 本应 no-op），但 skeleton 动态产出
        // ["WSTG-INPV-05"]，经 validate_stage_gate_with_skeleton → GateContext 注入 →
        // 资产缺该技术 → Block。证明 skeleton.expected_techniques 能驱动完整性闸（③ 预埋）。
        use super::super::sprint_contract::StageSkeleton;
        use super::super::stage_spec::load_stage_spec_from_json;
        use super::super::types::{CoverageCell, CoverageStatus, StageClaim};
        use golish_pentest::evidence_ledger::EvidenceAuditId;
        use std::collections::HashMap;

        // 内联 spec：只挂 coverage_complete 规则、expected_techniques 空。
        let spec = load_stage_spec_from_json(
            r#"{"id":"vuln_triage","kind":"vuln_triage","risk_level":"high",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
                "gate_rules":[{"op":"coverage_complete","on_fail":{"reason":"coverage incomplete"}}]}"#,
        )
        .unwrap();
        let eid = EvidenceAuditId::new(1);
        let asset = "api.example.com";
        // 覆盖 WSTG-INPV-01（≠ 期望的 WSTG-INPV-05），非 vacuous（1 claim 挂证据）。
        let d = StageDeliverable {
            stage_id: "vuln_triage".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![StageClaim {
                kind: "vuln".into(),
                subject: asset.into(),
                summary: "checked".into(),
                evidence_ids: vec![eid],
                technique: None,
            }],
            evidence_refs: vec![eid],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: vec![],
            coverage: vec![CoverageCell {
                asset: asset.into(),
                technique: "WSTG-INPV-01".into(),
                status: CoverageStatus::CheckedEmpty,
                evidence_refs: vec![eid],
                note: Some("scanned".into()),
                reason_kind: None,
                tested_units: 1,
                total_units: 1,
                sampling_rationale: None,
            }],
        };

        // 无 skeleton：spec.expected_techniques 空 → coverage_complete no-op → allowed。
        assert!(
            validate_stage_gate(&d, &spec, None).allowed,
            "empty expected_techniques should be a no-op"
        );

        // skeleton 动态产 ["WSTG-INPV-05"]：经 ctx 注入 → 资产缺该技术 → Block 含缺口。
        let skeleton = StageSkeleton {
            expected_findings: vec![],
            time_budget_minutes: 5,
            min_tool_invocations: HashMap::new(),
            expected_techniques: vec!["WSTG-INPV-05".to_string()],
        };
        let blocked = validate_stage_gate_with_skeleton(&d, &spec, None, Some(&skeleton));
        assert!(!blocked.allowed);
        assert!(
            blocked
                .reasons
                .iter()
                .any(|r| r.contains("coverage incomplete") && r.contains("WSTG-INPV-05")),
            "skeleton-driven coverage_complete should fire: {:?}",
            blocked.reasons
        );
    }
}
