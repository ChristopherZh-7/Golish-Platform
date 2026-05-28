//! StageHarness · 主 entry (Doc 3 §5.1 stage harness entry).
//!
//! Phase 1c.2 skeleton · 提供 `StageHarness::for_stage(kind)` + `validate_gate`
//! 主流程. Task 1c.6 在 task_orchestrator 接入.

use anyhow::{anyhow, Result};

use super::gate::{validate_external_attack_surface_gate, GateResult};
use super::profile::Profile;
use super::sprint_contract::SprintContract;
use super::stage_spec::StageSpec;
use super::types::{ExternalAttackSurfaceDeliverable, StageKind};

/// Doc 3 §5 stage harness 顶层.
///
/// 持有 profile + stage_spec + 当前 sprint_contract, 暴露 validate_gate 给
/// task_orchestrator 末端 hook 使用.
pub struct StageHarness {
    pub profile: Profile,
    pub stage_spec: StageSpec,
}

impl StageHarness {
    pub fn new(profile: Profile, stage_spec: StageSpec) -> Self {
        Self {
            profile,
            stage_spec,
        }
    }

    /// 按 StageKind 选 stage_spec.
    ///
    /// Phase 1c.2 skeleton 仅支持 `ExternalAttackSurface`; 其它 stage 返 Err.
    /// 实际加载在 Task 1c.6 阶段做 (from disk · resources/harness/...).
    pub fn for_stage(
        stage_kind: StageKind,
        profile: Profile,
        stage_spec: StageSpec,
    ) -> Result<Self> {
        if stage_kind != StageKind::ExternalAttackSurface {
            return Err(anyhow!(
                "StageHarness Phase 1 MVP supports only ExternalAttackSurface, got {:?}",
                stage_kind
            ));
        }
        if stage_spec.kind != stage_kind {
            return Err(anyhow!(
                "StageSpec.kind ({:?}) does not match requested stage_kind ({:?})",
                stage_spec.kind,
                stage_kind
            ));
        }
        Ok(Self::new(profile, stage_spec))
    }

    /// 验证 deliverable 是否通过 gate.
    ///
    /// Task 1c.5 实施各 check 后, 本方法返回真实 allowed / reasons / recovery.
    /// Phase 1c.2 skeleton 直接调用 `validate_external_attack_surface_gate`.
    pub fn validate_gate(
        &self,
        deliverable: &ExternalAttackSurfaceDeliverable,
        sprint_contract: Option<&SprintContract>,
    ) -> GateResult {
        tracing::info!(
            target: "harness::stage_harness",
            stage_kind = ?self.stage_spec.kind,
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            has_contract = sprint_contract.is_some(),
            "validate_gate entered (5-check pipeline)"
        );
        let result =
            validate_external_attack_surface_gate(deliverable, &self.stage_spec, sprint_contract);
        tracing::info!(
            target: "harness::stage_harness",
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            allowed = result.allowed,
            reasons_count = result.reasons.len(),
            "validate_gate completed"
        );
        result
    }
}

#[cfg(test)]
mod tests {
    use super::super::profile::load_profile_from_json;
    use super::super::stage_spec::load_stage_spec_from_json;
    use super::*;

    const ASSESSMENT_JSON: &str =
        include_str!("../../../../../resources/harness/profiles/assessment.json");
    const STAGE_JSON: &str =
        include_str!("../../../../../resources/harness/stages/external_attack_surface.json");

    #[test]
    fn for_stage_accepts_external_attack_surface() {
        let p = load_profile_from_json(ASSESSMENT_JSON).unwrap();
        let s = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let h = StageHarness::for_stage(StageKind::ExternalAttackSurface, p, s);
        assert!(h.is_ok());
    }

    #[test]
    fn for_stage_rejects_unsupported_stage_in_phase1() {
        let p = load_profile_from_json(ASSESSMENT_JSON).unwrap();
        let s = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let h = StageHarness::for_stage(StageKind::Enumeration, p, s);
        assert!(h.is_err());
    }

    #[test]
    fn for_stage_rejects_kind_mismatch() {
        let p = load_profile_from_json(ASSESSMENT_JSON).unwrap();
        let mut s = load_stage_spec_from_json(STAGE_JSON).unwrap();
        s.kind = StageKind::Scoping;
        let h = StageHarness::for_stage(StageKind::ExternalAttackSurface, p, s);
        assert!(h.is_err());
    }
}
