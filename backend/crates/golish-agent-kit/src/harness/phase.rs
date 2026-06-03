//! Phase grouping DTO + loader (设计 2026-06-03 两级阶段模型).
//!
//! Phase = 大阶段，是 12 个 [`StageKind`] 之上的编排薄层。成员是 StageKind 列表，
//! 每个 phase 可声明跨入它之前的 `entry_approval`（human_approval key）。
//! 与 `operation_graph.json` 一样：静态 JSON 加载 + 校验 + 按 profile 投影。
//!
//! 本模块**只**管分组拓扑：加载 / 校验「每个 stage 恰好属于一个 phase」/ 按
//! profile 投影 / 查 stage 所属 phase。不碰 DB、不判审批触发（那是
//! `phase_flow` + 运行时的关注点）。

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::profile::Profile;
use super::types::StageKind;

/// `phases.json` 中 `phases[*]` 元素：一个大阶段.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Phase {
    pub id: String,
    pub stages: Vec<StageKind>,
    /// 跨入本 phase 前需要的人工审批动作 key（如 `"active_scan"`）。
    /// None = 无 phase 级入口审批。
    #[serde(default)]
    pub entry_approval: Option<String>,
}

impl Phase {
    /// 该 phase 是否包含某 stage.
    pub fn contains(&self, stage: StageKind) -> bool {
        self.stages.contains(&stage)
    }
}

/// `phases.json` 根：大阶段有序列表.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseMap {
    pub phases: Vec<Phase>,
}

/// PhaseMap 加载 / 校验错误.
#[derive(Debug, Error)]
pub enum PhaseMapError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("stage {0:?} appears in more than one phase")]
    DuplicateStage(StageKind),
    #[error("stage {0:?} is not assigned to any phase")]
    UnassignedStage(StageKind),
}

/// 全部 12 个 StageKind（校验「不漏不重」用）.
const ALL_STAGES: [StageKind; 12] = [
    StageKind::Scoping,
    StageKind::TargetIntel,
    StageKind::ExternalAttackSurface,
    StageKind::Enumeration,
    StageKind::VulnTriage,
    StageKind::Verification,
    StageKind::AccessValidation,
    StageKind::InternalDiscovery,
    StageKind::ObjectivePathing,
    StageKind::ObjectiveSimulation,
    StageKind::Reporting,
    StageKind::Cleanup,
];

/// 静态 JSON 字符串 → [`PhaseMap`]，并校验（每个 stage 恰好属于一个 phase）.
///
/// 真正从 disk 读由调用方做；单测可无 IO 直接 fixture 验证.
pub fn load_phase_map_from_json(raw: &str) -> Result<PhaseMap, PhaseMapError> {
    let map: PhaseMap = serde_json::from_str(raw)?;
    map.validate()?;
    Ok(map)
}

impl PhaseMap {
    /// 校验：每个 StageKind 恰好属于一个 phase（不漏不重）.
    pub fn validate(&self) -> Result<(), PhaseMapError> {
        let mut seen: Vec<StageKind> = Vec::new();
        for p in &self.phases {
            for &s in &p.stages {
                if seen.contains(&s) {
                    return Err(PhaseMapError::DuplicateStage(s));
                }
                seen.push(s);
            }
        }
        for s in ALL_STAGES {
            if !seen.contains(&s) {
                return Err(PhaseMapError::UnassignedStage(s));
            }
        }
        Ok(())
    }

    /// stage 所属 phase（按定义顺序找第一个含它的）.
    pub fn phase_of(&self, stage: StageKind) -> Option<&Phase> {
        self.phases.iter().find(|p| p.contains(stage))
    }

    /// 按 profile 投影：每个 phase 只保留 `allowed` 内的成员；成员清空的 phase
    /// 整体剔除。复用 [`Profile::allowed_stage_set`]（与 operation_graph 投影同源逻辑）.
    pub fn project(&self, profile: &Profile) -> PhaseMap {
        let allowed = profile.allowed_stage_set();
        let phases = self
            .phases
            .iter()
            .filter_map(|p| {
                let stages: Vec<StageKind> = p
                    .stages
                    .iter()
                    .copied()
                    .filter(|s| allowed.contains(s))
                    .collect();
                if stages.is_empty() {
                    None
                } else {
                    Some(Phase {
                        id: p.id.clone(),
                        stages,
                        entry_approval: p.entry_approval.clone(),
                    })
                }
            })
            .collect();
        PhaseMap { phases }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::profile::load_profile_from_json;

    const PHASES_JSON: &str = include_str!("../../../../../resources/harness/graph/phases.json");
    const ASSESSMENT_JSON: &str =
        include_str!("../../../../../resources/harness/profiles/assessment.json");

    fn map() -> PhaseMap {
        load_phase_map_from_json(PHASES_JSON).expect("phases.json parses + validates")
    }

    #[test]
    fn phases_cover_all_12_stages_exactly_once() {
        let m = map();
        assert_eq!(m.phases.len(), 5);
        m.validate().expect("every stage assigned exactly once");
    }

    #[test]
    fn phase_of_known_stages() {
        let m = map();
        assert_eq!(m.phase_of(StageKind::Scoping).unwrap().id, "prep");
        assert_eq!(m.phase_of(StageKind::TargetIntel).unwrap().id, "prep");
        assert_eq!(
            m.phase_of(StageKind::ExternalAttackSurface).unwrap().id,
            "active_recon"
        );
        assert_eq!(
            m.phase_of(StageKind::Enumeration).unwrap().id,
            "active_recon"
        );
        assert_eq!(m.phase_of(StageKind::Verification).unwrap().id, "vuln");
        assert_eq!(m.phase_of(StageKind::Cleanup).unwrap().id, "closeout");
    }

    #[test]
    fn entry_approvals_on_active_recon_and_vuln_only() {
        let m = map();
        let ar = m.phases.iter().find(|p| p.id == "active_recon").unwrap();
        assert_eq!(ar.entry_approval.as_deref(), Some("active_scan"));
        let vuln = m.phases.iter().find(|p| p.id == "vuln").unwrap();
        assert_eq!(vuln.entry_approval.as_deref(), Some("exploit_validation"));
        let prep = m.phases.iter().find(|p| p.id == "prep").unwrap();
        assert_eq!(prep.entry_approval, None);
    }

    #[test]
    fn assessment_projection_drops_vuln_and_post_exploit() {
        // assessment forbids vuln_triage/verification/access_validation/cleanup.
        let profile = load_profile_from_json(ASSESSMENT_JSON).expect("profile");
        let projected = map().project(&profile);
        let ids: Vec<&str> = projected.phases.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"prep"));
        assert!(ids.contains(&"active_recon"));
        assert!(ids.contains(&"closeout")); // reporting 在 closeout，assessment 允许
        assert!(!ids.contains(&"vuln"));
        assert!(!ids.contains(&"post_exploit"));
        // closeout 在 assessment 只剩 reporting（cleanup 被 forbidden）
        let closeout = projected
            .phases
            .iter()
            .find(|p| p.id == "closeout")
            .unwrap();
        assert_eq!(closeout.stages, vec![StageKind::Reporting]);
    }

    #[test]
    fn validate_rejects_duplicate_stage() {
        let raw = r#"{"phases":[
            {"id":"a","stages":["scoping","scoping","target_intel","external_attack_surface","enumeration","vuln_triage","verification","access_validation","internal_discovery","objective_pathing","objective_simulation","reporting","cleanup"]}
        ]}"#;
        assert!(matches!(
            load_phase_map_from_json(raw),
            Err(PhaseMapError::DuplicateStage(StageKind::Scoping))
        ));
    }

    #[test]
    fn validate_rejects_unassigned_stage() {
        // 缺 cleanup → UnassignedStage.
        let raw = r#"{"phases":[
            {"id":"a","stages":["scoping","target_intel","external_attack_surface","enumeration","vuln_triage","verification","access_validation","internal_discovery","objective_pathing","objective_simulation","reporting"]}
        ]}"#;
        assert!(matches!(
            load_phase_map_from_json(raw),
            Err(PhaseMapError::UnassignedStage(StageKind::Cleanup))
        ));
    }
}
