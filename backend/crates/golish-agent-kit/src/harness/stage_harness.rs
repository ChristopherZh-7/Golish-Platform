//! StageHarness · 主 entry (Doc 3 §5.1 stage harness entry).
//!
//! Phase 1c.2 skeleton · 提供 `StageHarness::for_stage(kind)` + `validate_gate`
//! 主流程. Task 1c.6 在 task_orchestrator 接入.

use anyhow::{anyhow, Result};

use super::gate::{validate_stage_gate_with_skeleton, GateResult};
use super::profile::Profile;
use super::sprint_contract::{SprintContract, StageSkeleton};
use super::stage_spec::StageSpec;
use super::types::{StageDeliverable, StageKind};

/// Doc 3 §5 stage harness 顶层.
///
/// 持有 profile + stage_spec (+ 可选 per-stage sprint skeleton), 暴露 validate_gate
/// 给 task_orchestrator 末端 hook 使用.
pub struct StageHarness {
    pub profile: Profile,
    pub stage_spec: StageSpec,
    /// Optional per-stage sprint skeleton. When `Some`, [`Self::validate_gate`]
    /// enforces its `expected_count_range` / `min_tool_invocations` (per-target
    /// gate). `None` = baseline structural gate only (backward compatible).
    pub skeleton: Option<StageSkeleton>,
}

impl StageHarness {
    pub fn new(profile: Profile, stage_spec: StageSpec) -> Self {
        Self {
            profile,
            stage_spec,
            skeleton: None,
        }
    }

    /// Builder · attach a per-stage sprint skeleton to enable per-target gate
    /// enforcement. Returns `self` for chaining after `for_stage_embedded`.
    pub fn with_skeleton(mut self, skeleton: Option<StageSkeleton>) -> Self {
        self.skeleton = skeleton;
        self
    }

    /// 按 StageKind 构造 (Phase B: 解锁单 stage 硬锁, 支持全 12 stage).
    ///
    /// 唯一约束: `stage_spec.kind` 必须等于请求的 `stage_kind` (防止张冠李戴).
    pub fn for_stage(
        stage_kind: StageKind,
        profile: Profile,
        stage_spec: StageSpec,
    ) -> Result<Self> {
        if stage_spec.kind != stage_kind {
            return Err(anyhow!(
                "StageSpec.kind ({:?}) does not match requested stage_kind ({:?})",
                stage_spec.kind,
                stage_kind
            ));
        }
        Ok(Self::new(profile, stage_spec))
    }

    /// Phase B · 从嵌入 registry 按 kind 自动载 StageSpec, 免去调用方手动传 spec.
    pub fn for_stage_embedded(stage_kind: StageKind, profile: Profile) -> Result<Self> {
        let stage_spec = super::resources::load_embedded_stage_spec(stage_kind).map_err(|e| {
            anyhow!(
                "load embedded stage spec for {:?} failed: {}",
                stage_kind,
                e
            )
        })?;
        Self::for_stage(stage_kind, profile, stage_spec)
    }

    /// 验证 deliverable 是否通过 gate.
    ///
    /// 调用通用 `validate_stage_gate` (按 self.stage_spec 的 gate_rules 跑结构+语义 check),
    /// 返回 allowed / reasons / recovery.
    pub fn validate_gate(
        &self,
        deliverable: &StageDeliverable,
        sprint_contract: Option<&SprintContract>,
    ) -> GateResult {
        tracing::info!(
            target: "harness::stage_harness",
            stage_kind = ?self.stage_spec.kind,
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            has_contract = sprint_contract.is_some(),
            has_skeleton = self.skeleton.is_some(),
            "validate_gate entered (generic stage gate)"
        );
        let result = validate_stage_gate_with_skeleton(
            deliverable,
            &self.stage_spec,
            sprint_contract,
            self.skeleton.as_ref(),
        );
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
    fn for_stage_rejects_kind_mismatch_enumeration() {
        // 用 external_attack_surface spec 但请求 Enumeration → kind 不匹配 → Err.
        let p = load_profile_from_json(ASSESSMENT_JSON).unwrap();
        let s = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let h = StageHarness::for_stage(StageKind::Enumeration, p, s);
        assert!(h.is_err());
    }

    #[test]
    fn for_stage_embedded_loads_non_external_stages() {
        // Phase B: 单 stage 硬锁已解, for_stage_embedded 能载任意 stage.
        let p = load_profile_from_json(ASSESSMENT_JSON).unwrap();
        for kind in [
            StageKind::Scoping,
            StageKind::TargetIntel,
            StageKind::Enumeration,
            StageKind::VulnTriage,
            StageKind::Reporting,
        ] {
            assert!(
                StageHarness::for_stage_embedded(kind, p.clone()).is_ok(),
                "for_stage_embedded should load {:?}",
                kind
            );
        }
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
