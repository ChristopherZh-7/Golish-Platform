//! Doc 3 §8.2 min_tool_invocations_per_check · spec.min_invocations 里声明的工具
//! 是否在 deliverable 中有体现.
//!
//! Phase B MVP 近似: 用 `deliverable.required_checks_done` 名单做包含匹配
//! (agent 在交付里登记跑过的 check / tool). Phase C 接真实 tool-call 痕迹后
//! 改为按 evidence / tool invocation log 精确计数.

use super::super::stage_spec::StageSpec;
use super::super::types::{HarnessRecoveryActions, StageDeliverable};
use super::GateCheckOutcome;

pub fn run(deliverable: &StageDeliverable, spec: &StageSpec) -> GateCheckOutcome {
    let mut missing: Vec<String> = Vec::new();

    for tool in spec.min_invocations.keys() {
        let satisfied = deliverable
            .required_checks_done
            .iter()
            .any(|done| done.contains(tool));
        if !satisfied {
            missing.push(tool.clone());
        }
    }

    if missing.is_empty() {
        GateCheckOutcome::Pass
    } else {
        let reasons = missing
            .iter()
            .map(|t| {
                format!(
                    "min tool invocations not satisfied for '{t}' (not in required_checks_done)"
                )
            })
            .collect();
        GateCheckOutcome::Block {
            reasons,
            recovery: HarnessRecoveryActions {
                repair_tool_calls: missing,
                ..Default::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::resources::load_embedded_stage_spec;
    use crate::harness::types::StageKind;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn deliverable(done: Vec<&str>) -> StageDeliverable {
        StageDeliverable {
            stage_id: "enumeration".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![],
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: done.into_iter().map(String::from).collect(),
        }
    }

    #[test]
    fn passes_when_no_min_invocations_required() {
        let mut spec = load_embedded_stage_spec(StageKind::Reporting).unwrap();
        spec.min_invocations = HashMap::new();
        assert!(matches!(
            run(&deliverable(vec![]), &spec),
            GateCheckOutcome::Pass
        ));
    }

    #[test]
    fn blocks_when_required_tool_not_done() {
        // enumeration spec requires http_probe >= 1
        let spec = load_embedded_stage_spec(StageKind::Enumeration).unwrap();
        let outcome = run(&deliverable(vec!["dns_resolve"]), &spec);
        assert!(matches!(outcome, GateCheckOutcome::Block { .. }));
    }

    #[test]
    fn passes_when_required_tool_present_in_done() {
        let spec = load_embedded_stage_spec(StageKind::Enumeration).unwrap();
        let outcome = run(&deliverable(vec!["http_probe done"]), &spec);
        assert!(matches!(outcome, GateCheckOutcome::Pass));
    }
}
