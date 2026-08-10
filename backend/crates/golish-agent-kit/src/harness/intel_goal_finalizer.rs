//! I/O-free deterministic Target Intel finalizer.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelGoalFinalizerMaterial {
    pub review_id: Uuid,
    pub operation_contract_sha256: String,
    pub review_bundle_sha256: String,
    pub verdict_sha256: String,
    pub operation_contract_valid: bool,
    pub review_is_fresh_pass: bool,
    pub all_four_sections_read: bool,
    pub material_revision_matches: bool,
    pub active_authoritative_workers: usize,
    pub active_authoritative_tools: usize,
    pub current_run_terminal_receipt_count: usize,
    pub valid_evidence_artifact_closure_count: usize,
    pub pending_or_retryable_frontier_count: usize,
    pub unwaived_blocked_or_unsupported_count: usize,
    pub unresolved_material_contradiction_count: usize,
    pub open_material_finding_count: usize,
    pub unauthorized_scope_promotion_count: usize,
    pub needs_human_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum IntelGoalFinalizerDecision {
    Pass {
        review_id: Uuid,
        review_bundle_sha256: String,
        verdict_sha256: String,
    },
    Block {
        code: String,
        reason: String,
        finding_refs: Vec<Uuid>,
    },
}

pub fn evaluate_intel_goal_finalizer(
    material: &IntelGoalFinalizerMaterial,
) -> IntelGoalFinalizerDecision {
    macro_rules! block {
        ($code:literal, $reason:literal) => {
            return IntelGoalFinalizerDecision::Block {
                code: $code.to_string(),
                reason: $reason.to_string(),
                finding_refs: Vec::new(),
            }
        };
    }
    if material.review_id.is_nil()
        || !material.operation_contract_valid
        || !is_sha256(&material.operation_contract_sha256)
        || !is_sha256(&material.review_bundle_sha256)
        || !is_sha256(&material.verdict_sha256)
    {
        block!(
            "INTEL_GOAL_OPERATION_CONTRACT_INVALID",
            "missing or invalid immutable operation contract"
        );
    }
    if !material.review_is_fresh_pass || !material.all_four_sections_read {
        block!(
            "INTEL_GOAL_REVIEW_NOT_FRESH_PASS",
            "finalization requires a fresh PASS after all four ordered sections"
        );
    }
    if !material.material_revision_matches {
        block!(
            "INTEL_GOAL_MATERIAL_DRIFT",
            "material state or action revision changed after review freeze"
        );
    }
    if material.active_authoritative_workers > 0 || material.active_authoritative_tools > 0 {
        block!(
            "INTEL_GOAL_ACTIVE_WORK_REMAINS",
            "authoritative workers or tools are still active"
        );
    }
    if material.current_run_terminal_receipt_count == 0
        || material.valid_evidence_artifact_closure_count
            != material.current_run_terminal_receipt_count
    {
        block!(
            "INTEL_GOAL_NON_VACUITY_FAILED",
            "current-run terminal receipts and evidence/artifact closure are required"
        );
    }
    if material.pending_or_retryable_frontier_count > 0 {
        block!(
            "INTEL_GOAL_FRONTIER_OPEN",
            "pending or retryable material frontier remains"
        );
    }
    if material.unwaived_blocked_or_unsupported_count > 0 {
        block!(
            "INTEL_GOAL_CAPABILITY_GAP_UNRESOLVED",
            "blocked or unsupported material capability lacks policy waiver and alternative evidence"
        );
    }
    if material.unresolved_material_contradiction_count > 0
        || material.open_material_finding_count > 0
        || material.needs_human_count > 0
    {
        block!(
            "INTEL_GOAL_MATERIAL_REVIEW_OPEN",
            "material contradiction, finding, or human requirement remains open"
        );
    }
    if material.unauthorized_scope_promotion_count > 0 {
        block!(
            "INTEL_GOAL_SCOPE_PROMOTION_VIOLATION",
            "candidate attribution was promoted without server-owned authorization"
        );
    }
    IntelGoalFinalizerDecision::Pass {
        review_id: material.review_id,
        review_bundle_sha256: material.review_bundle_sha256.clone(),
        verdict_sha256: material.verdict_sha256.clone(),
    }
}

fn is_sha256(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> IntelGoalFinalizerMaterial {
        IntelGoalFinalizerMaterial {
            review_id: Uuid::from_u128(1),
            operation_contract_sha256: format!("sha256:{}", "a".repeat(64)),
            review_bundle_sha256: format!("sha256:{}", "b".repeat(64)),
            verdict_sha256: format!("sha256:{}", "c".repeat(64)),
            operation_contract_valid: true,
            review_is_fresh_pass: true,
            all_four_sections_read: true,
            material_revision_matches: true,
            active_authoritative_workers: 0,
            active_authoritative_tools: 0,
            current_run_terminal_receipt_count: 1,
            valid_evidence_artifact_closure_count: 1,
            pending_or_retryable_frontier_count: 0,
            unwaived_blocked_or_unsupported_count: 0,
            unresolved_material_contradiction_count: 0,
            open_material_finding_count: 0,
            unauthorized_scope_promotion_count: 0,
            needs_human_count: 0,
        }
    }

    #[test]
    fn finalizer_passes_fresh_non_vacuous_exact_material() {
        assert!(matches!(
            evaluate_intel_goal_finalizer(&valid()),
            IntelGoalFinalizerDecision::Pass { .. }
        ));
    }

    #[test]
    fn finalizer_rejects_six_axis_style_vacuity_and_scope_promotion() {
        let mut material = valid();
        material.current_run_terminal_receipt_count = 0;
        assert!(matches!(
            evaluate_intel_goal_finalizer(&material),
            IntelGoalFinalizerDecision::Block { ref code, .. }
                if code == "INTEL_GOAL_NON_VACUITY_FAILED"
        ));
        let mut material = valid();
        material.unauthorized_scope_promotion_count = 1;
        assert!(matches!(
            evaluate_intel_goal_finalizer(&material),
            IntelGoalFinalizerDecision::Block { ref code, .. }
                if code == "INTEL_GOAL_SCOPE_PROMOTION_VIOLATION"
        ));
    }

    #[test]
    fn finalizer_requires_closure_for_every_terminal_receipt() {
        let mut material = valid();
        material.current_run_terminal_receipt_count = 2;
        assert!(matches!(
            evaluate_intel_goal_finalizer(&material),
            IntelGoalFinalizerDecision::Block { ref code, .. }
                if code == "INTEL_GOAL_NON_VACUITY_FAILED"
        ));
    }
}
