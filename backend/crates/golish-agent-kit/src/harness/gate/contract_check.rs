//! Doc 3 §8.2 contract_check · findings 数量在 Sprint Contract range 内.
//!
//! Phase 1c.2 skeleton · contract = None 时直接 Pass; contract 存在时仅做
//! 简单 sanity (Task 1c.5 完整加入 expected_count_range 比对 + min_tool_invocations
//! 达标).

use super::super::sprint_contract::SprintContract;
use super::super::types::{ExternalAttackSurfaceDeliverable, HarnessRecoveryActions};
use super::GateCheckOutcome;

pub fn run(
    _deliverable: &ExternalAttackSurfaceDeliverable,
    contract: Option<&SprintContract>,
) -> GateCheckOutcome {
    // Phase 1c.2 skeleton: 没合同 → Pass (允许首跑无 contract 路径).
    // Task 1c.5 加 contract 范围检查 + min_tool_invocations 验证.
    match contract {
        None => GateCheckOutcome::Pass,
        Some(c) if c.status != "active" => {
            let mut recovery = HarnessRecoveryActions::default();
            recovery.hints.push(format!(
                "active sprint_contract required, found status={}", c.status
            ));
            GateCheckOutcome::Block {
                reasons: vec![format!(
                    "sprint_contract {} not active (status={})",
                    c.id, c.status
                )],
                recovery,
            }
        }
        Some(_) => GateCheckOutcome::Pass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::sprint_contract::SprintContract;
    use super::super::super::types::ExternalAttackSurfaceDeliverable;
    use uuid::Uuid;

    fn empty_deliverable() -> ExternalAttackSurfaceDeliverable {
        ExternalAttackSurfaceDeliverable {
            stage_id: "external_attack_surface".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![],
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: vec![],
        }
    }

    #[test]
    fn no_contract_passes() {
        assert!(matches!(run(&empty_deliverable(), None), GateCheckOutcome::Pass));
    }

    #[test]
    fn active_contract_passes() {
        let c = SprintContract::new_active(
            Uuid::new_v4(),
            "contract text".to_string(),
            "openai:gpt-4o".to_string(),
        );
        assert!(matches!(run(&empty_deliverable(), Some(&c)), GateCheckOutcome::Pass));
    }

    #[test]
    fn superseded_contract_blocks() {
        let mut c = SprintContract::new_active(
            Uuid::new_v4(),
            "contract text".to_string(),
            "openai:gpt-4o".to_string(),
        );
        c.status = "superseded".to_string();
        match run(&empty_deliverable(), Some(&c)) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("not active"));
            }
            _ => panic!("expected Block"),
        }
    }
}
