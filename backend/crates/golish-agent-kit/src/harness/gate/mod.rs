//! Gate validator (Doc 3 §8) 调度入口.
//!
//! Phase 1c.2 skeleton · 5 个 check (schema / scope / contract / vacuous /
//! freshness) 占位.
//!
//! Doc 4 (`docs/design/2026-05-26-harness-observability-plane.md`) 预留的
//! Observability ids 字段 (`gate_result_id` / `blocking_reason_id`) 已加入
//! [`GateResult`], Phase 2 完整 wiring 时填.

pub mod contract_check;
pub mod freshness_check;
pub mod min_invocations_check;
pub mod schema_check;
pub mod scope_check;
pub mod surface_coverage_check;
pub mod vacuous_check;

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::sprint_contract::SprintContract;
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
/// **结构性 check** (schema / contract / vacuous / freshness) 永远跑: 与 stage
/// 语义无关, 只看 deliverable 形状 / 契约 / 时效.
///
/// **语义 check** (scope / surface_coverage / min_invocations) 由
/// `spec.required_checks` 命名选跑; 多个 required_checks 名映射到同一 check 时
/// 去重只跑一次. `evidence_non_empty` / `unchecked_distinct_from_checked_empty`
/// 已被 schema / vacuous 覆盖, 不单独再跑.
pub fn validate_stage_gate(
    deliverable: &StageDeliverable,
    spec: &StageSpec,
    contract: Option<&SprintContract>,
) -> GateResult {
    let mut outcomes = vec![
        schema_check::run(deliverable, spec),
        contract_check::run(deliverable, contract),
        vacuous_check::run(deliverable, spec),
        freshness_check::run(deliverable, spec),
    ];

    let mut ran: HashSet<&'static str> = HashSet::new();
    for name in &spec.required_checks {
        let check_id = match name.as_str() {
            "scope_status_present" | "out_of_scope_targets_excluded" => "scope",
            "surface_workbench_coverage" => "surface_coverage",
            "min_tool_invocations_per_check" => "min_invocations",
            _ => continue,
        };
        if !ran.insert(check_id) {
            continue;
        }
        outcomes.push(match check_id {
            "scope" => scope_check::run(deliverable),
            "surface_coverage" => surface_coverage_check::run(deliverable),
            "min_invocations" => min_invocations_check::run(deliverable, spec),
            _ => unreachable!(),
        });
    }

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
}
