//! Gate validator (Doc 3 §8) 调度入口.
//!
//! Phase 1c.2 skeleton · 5 个 check (schema / scope / contract / vacuous /
//! freshness) 占位.
//!
//! Doc 4 (`docs/design/2026-05-26-harness-observability-plane.md`) 预留的
//! Observability ids 字段 (`gate_result_id` / `blocking_reason_id`) 已加入
//! [`GateResult`], Phase 2 完整 wiring 时填.

pub mod contract_check;
pub mod finding_verification_check;
pub mod freshness_check;
pub mod min_invocations_check;
pub mod rule_engine;
pub mod schema_check;
pub mod scope_check;
pub mod surface_coverage_check;
pub mod vacuous_check;

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
    let mut outcomes = vec![
        schema_check::run(deliverable, spec),
        contract_check::run_with_skeleton(deliverable, contract, skeleton),
        vacuous_check::run(deliverable, spec),
        freshness_check::run(deliverable, spec),
        // P2 · config-driven verification (no-op unless the stage spec declares
        // finding_verification / min_findings / min_claims).
        finding_verification_check::run(deliverable, spec),
    ];

    // 语义层 · 过关标准的**唯一入口**（gate-rules-migration 2026-06-05）。旧
    // `required_checks` 固定菜单 match（含 `_ => continue` 静默忽略）已删除：
    // scope / surface_coverage / min_invocations 现以 `named_check` 积木从
    // `gate_rules` 调用，简单标准（claim/finding 证据非空等）用数据积木声明。
    // 写错 op/check 名由 typed-enum 在 spec 加载期 fail-closed。空 gate_rules =
    // 仅跑上面 5 个结构层 check。
    outcomes.extend(rule_engine::eval(deliverable, spec, &spec.gate_rules));

    aggregate(outcomes)
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
            include_str!("../../../../../../resources/harness/stages/scoping.json");
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
        };
        let blocked = validate_stage_gate_with_skeleton(&deliverable, &spec, None, Some(&skeleton));
        assert!(!blocked.allowed);
        assert!(blocked
            .reasons
            .iter()
            .any(|r| r.contains("subdomain") && r.contains("below contract minimum")));
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
            }],
            evidence_refs: vec![eid],
            skipped_checks: vec![],
            findings: vec![HarnessFinding {
                finding_id: Uuid::new_v4(),
                kind: "rce".to_string(),
                subject: "api.example.com".to_string(),
                severity: FindingSeverity::Critical,
                evidence_refs: vec![eid],
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

    // ── coverage matrix 样例接入（设计 2026-06-05-coverage-matrix Task 6 + #4） ──
    // 用迁移后的 vuln_triage embedded spec（含 expected_techniques + coverage_complete
    // + found/checked_empty 证据规则）：完整覆盖 → Pass；删一格期望技术 → coverage_complete
    // Block；checked_empty 清空证据 → found/checked_empty 证据规则 Block（落地用户 #4）。

    #[test]
    fn vuln_triage_coverage_gate_blocks_on_gap_and_passes_when_complete() {
        use super::super::resources::load_embedded_stage_spec;
        use super::super::types::{
            CoverageCell, CoverageStatus, FindingSeverity, HarnessFinding, StageClaim, StageKind,
        };
        use golish_pentest::evidence_ledger::EvidenceAuditId;

        let spec = load_embedded_stage_spec(StageKind::VulnTriage).unwrap();
        // sanity：样例确实声明了 4 类期望技术（WSTG id）。
        assert_eq!(spec.expected_techniques.len(), 4);

        let eid = EvidenceAuditId::new(1);
        let asset = "api.example.com";
        // 资产对 4 类 WSTG 技术都给了终态（found/checked_empty/n_a），found 挂证据。
        let full_coverage = vec![
            CoverageCell {
                asset: asset.into(),
                technique: "WSTG-INPV-05".into(),
                status: CoverageStatus::Found,
                evidence_refs: vec![eid],
                note: None,
            },
            CoverageCell {
                asset: asset.into(),
                technique: "WSTG-INPV-01".into(),
                status: CoverageStatus::CheckedEmpty,
                evidence_refs: vec![eid],
                note: Some("no reflection observed".into()),
            },
            CoverageCell {
                asset: asset.into(),
                technique: "WSTG-ATHZ-04".into(),
                status: CoverageStatus::CheckedEmpty,
                evidence_refs: vec![eid],
                note: Some("object refs scoped to owner".into()),
            },
            CoverageCell {
                asset: asset.into(),
                technique: "WSTG-INPV-19".into(),
                status: CoverageStatus::NotApplicable,
                evidence_refs: vec![],
                note: Some("no outbound fetch surface".into()),
            },
        ];
        let pass_deliverable = StageDeliverable {
            stage_id: "vuln_triage".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![StageClaim {
                kind: "vuln".into(),
                subject: asset.into(),
                summary: "sqli confirmed".into(),
                evidence_ids: vec![eid],
            }],
            evidence_refs: vec![eid],
            skipped_checks: vec![],
            findings: vec![HarnessFinding {
                finding_id: Uuid::new_v4(),
                kind: "sqli".into(),
                subject: asset.into(),
                severity: FindingSeverity::High,
                evidence_refs: vec![eid],
            }],
            required_checks_done: vec![],
            coverage: full_coverage,
        };
        let pass = validate_stage_gate(&pass_deliverable, &spec, None);
        assert!(
            pass.allowed,
            "full coverage should pass: {:?}",
            pass.reasons
        );

        // 删掉 SSRF(WSTG-INPV-19) 那格 → coverage_complete 应 Block 且 reason 含缺口。
        let mut incomplete = pass_deliverable.clone();
        incomplete.coverage.pop();
        let blocked = validate_stage_gate(&incomplete, &spec, None);
        assert!(!blocked.allowed);
        assert!(
            blocked
                .reasons
                .iter()
                .any(|r| r.contains("coverage incomplete") && r.contains("WSTG-INPV-19")),
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
}
