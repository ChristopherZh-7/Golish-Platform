//! 嵌入式 harness 资源 registry (Phase B · Doc 3 §2 / §4).
//!
//! 所有 `resources/harness/**` 经此一处 `include_str!` 加载, 消灭散落在各
//! module / hook 里的 include_str! 字面路径. 按 `StageKind` / profile id 取
//! 对应 JSON 原文, 再交给 `stage_spec` / `profile` loader 解析.
//!
//! include_str! 相对路径以本文件 (`src/harness/resources.rs`) 为基准, 与
//! `stage_spec.rs` / `profile.rs` 的内联 fixture 同深度 (5 个 `../` 到 repo 根).

use super::phase::{load_phase_map_from_json, PhaseMap, PhaseMapError};
use super::profile::{load_profile_from_json, Profile, ProfileLoadError};
use super::sprint_contract::SprintSkeleton;
use super::stage_spec::{load_stage_spec_from_json, StageSpec, StageSpecLoadError};
use super::types::StageKind;

macro_rules! stage_json {
    ($p:literal) => {
        include_str!(concat!("../../../../../resources/harness/stages/", $p))
    };
}

macro_rules! profile_json_raw {
    ($p:literal) => {
        include_str!(concat!("../../../../../resources/harness/profiles/", $p))
    };
}

macro_rules! stage_methodology_raw {
    ($p:literal) => {
        include_str!(concat!("../../../../../resources/harness/stages/", $p))
    };
}

/// 按 stage kind 取嵌入的 stage spec JSON 原文.
///
/// 12 个 StageKind 全覆盖 (与 `resources/harness/stages/<stage>/spec.json` 一一对应).
pub fn stage_spec_json(kind: StageKind) -> &'static str {
    match kind {
        StageKind::Scoping => stage_json!("scoping/spec.json"),
        StageKind::TargetIntel => stage_json!("target_intel/spec.json"),
        StageKind::ExternalAttackSurface => stage_json!("external_attack_surface/spec.json"),
        StageKind::Enumeration => stage_json!("enumeration/spec.json"),
        StageKind::VulnTriage => stage_json!("vuln_triage/spec.json"),
        StageKind::Verification => stage_json!("verification/spec.json"),
        StageKind::AccessValidation => stage_json!("access_validation/spec.json"),
        StageKind::InternalDiscovery => stage_json!("internal_discovery/spec.json"),
        StageKind::ObjectivePathing => stage_json!("objective_pathing/spec.json"),
        StageKind::ObjectiveSimulation => stage_json!("objective_simulation/spec.json"),
        StageKind::Cleanup => stage_json!("cleanup/spec.json"),
        StageKind::Reporting => stage_json!("reporting/spec.json"),
    }
}

/// 所有内嵌 profile 的 id，单一来源（与 [`profile_json`] 的 match 臂一一对应）。
/// 顺序即 UI 列表呈现顺序：最安全的 assessment 在前，最激进的 red_team 在后。
/// 新增一个 profile JSON 时，在此数组与 [`profile_json`] 各加一行即可，前端零改动。
pub const EMBEDDED_PROFILE_IDS: &[&str] = &[
    "assessment",
    "pentest",
    "bug_bounty",
    "cloud_assessment",
    "red_team",
    // Minimal smoke-test flow (scoping -> target_intel). Listed last so it sits
    // at the bottom of the mode picker.
    "smoke",
];

/// 按 profile id 取嵌入的 profile JSON 原文; 未知 id 返回 None.
pub fn profile_json(id: &str) -> Option<&'static str> {
    Some(match id {
        "assessment" => profile_json_raw!("assessment.json"),
        "pentest" => profile_json_raw!("pentest.json"),
        "red_team" => profile_json_raw!("red_team.json"),
        "bug_bounty" => profile_json_raw!("bug_bounty.json"),
        "cloud_assessment" => profile_json_raw!("cloud_assessment.json"),
        "smoke" => profile_json_raw!("smoke.json"),
        _ => return None,
    })
}

/// 按 kind 加载 + 解析 stage spec.
pub fn load_embedded_stage_spec(kind: StageKind) -> Result<StageSpec, StageSpecLoadError> {
    load_stage_spec_from_json(stage_spec_json(kind))
}

/// 按 stage kind 取嵌入的「阶段方法论 playbook」原文 (`<stage>/methodology.md`).
///
/// 与 stage spec JSON 同目录、同 `include_str!` 深度，但与 gate 解耦：playbook 是
/// **正向方法论指导**（这个阶段怎么高效做、推荐工具序列、效率红线、何时收口），由
/// [`crate::task_orchestrator::prompts::stage_methodology`] 注入到 charter 之后。
/// 改它只影响 agent 看到的指导文本，0 Rust 改动、不参与确定性 gate。
///
/// 只有写了 playbook 的阶段返回 `Some`；其余返回 `None`（charter 不追加 playbook 段）。
pub fn stage_methodology_md(kind: StageKind) -> Option<&'static str> {
    Some(match kind {
        StageKind::Scoping => stage_methodology_raw!("scoping/methodology.md"),
        StageKind::TargetIntel => stage_methodology_raw!("target_intel/methodology.md"),
        StageKind::ExternalAttackSurface => {
            stage_methodology_raw!("external_attack_surface/methodology.md")
        }
        StageKind::Enumeration => stage_methodology_raw!("enumeration/methodology.md"),
        _ => return None,
    })
}

/// 加载 + 校验内嵌的大阶段分组表 (`resources/harness/graph/phases.json`).
///
/// 两级阶段模型（设计 2026-06-03）的 phase 拓扑单一来源；与 `operation_graph.json`
/// 同目录、同 `include_str!` 深度。
pub fn load_embedded_phase_map() -> Result<PhaseMap, PhaseMapError> {
    const PHASES_JSON: &str = include_str!("../../../../../resources/harness/graph/phases.json");
    load_phase_map_from_json(PHASES_JSON)
}

/// 按 id 加载 + 解析 profile; 未知 id 返回 `Ok(None)`.
pub fn load_embedded_profile(id: &str) -> Result<Option<Profile>, ProfileLoadError> {
    match profile_json(id) {
        Some(raw) => Ok(Some(load_profile_from_json(raw)?)),
        None => Ok(None),
    }
}

/// 按 profile id 取嵌入的 sprint skeleton JSON 原文; 没有 skeleton 文件的 profile
/// 返回 `None`。Skeleton 文件与 profile 同目录 (`<profile>.sprint_skeleton.json`)。
///
/// 当前仅 `assessment` 带 skeleton; 其余 profile 暂无 (gate 退回基础结构校验)。
pub fn sprint_skeleton_json(profile_id: &str) -> Option<&'static str> {
    Some(match profile_id {
        "assessment" => profile_json_raw!("assessment.sprint_skeleton.json"),
        _ => return None,
    })
}

/// 按 profile id 加载 + 解析 sprint skeleton; 无 skeleton 的 profile 返回 `Ok(None)`。
pub fn load_embedded_sprint_skeleton(
    profile_id: &str,
) -> Result<Option<SprintSkeleton>, serde_json::Error> {
    match sprint_skeleton_json(profile_id) {
        Some(raw) => Ok(Some(SprintSkeleton::from_json(raw)?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_twelve_stage_specs_load_and_kind_matches() {
        for kind in [
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
            StageKind::Cleanup,
            StageKind::Reporting,
        ] {
            let spec = load_embedded_stage_spec(kind)
                .unwrap_or_else(|e| panic!("load {:?} failed: {}", kind, e));
            assert_eq!(spec.kind, kind, "spec.kind must match requested kind");
        }
    }

    #[test]
    fn all_six_profiles_load() {
        for id in [
            "assessment",
            "pentest",
            "red_team",
            "bug_bounty",
            "cloud_assessment",
            "smoke",
        ] {
            let p =
                load_embedded_profile(id).unwrap_or_else(|e| panic!("load {} failed: {}", id, e));
            assert!(p.is_some(), "profile {} should load", id);
            assert_eq!(p.unwrap().id, id);
        }
    }

    #[test]
    fn unknown_profile_id_returns_none() {
        assert!(load_embedded_profile("does_not_exist").unwrap().is_none());
    }

    #[test]
    fn assessment_sprint_skeleton_loads_with_external_attack_surface() {
        let s = load_embedded_sprint_skeleton("assessment")
            .expect("parse ok")
            .expect("assessment has a skeleton");
        assert!(s.for_stage(StageKind::ExternalAttackSurface).is_some());
    }

    #[test]
    fn profile_without_skeleton_returns_none() {
        // Real profile with no skeleton file + an unknown id both yield None.
        assert!(load_embedded_sprint_skeleton("red_team").unwrap().is_none());
        assert!(load_embedded_sprint_skeleton("does_not_exist")
            .unwrap()
            .is_none());
    }

    #[test]
    fn stage_methodology_present_for_info_gathering_stages_and_absent_otherwise() {
        // The four info-gathering / active-mapping stages ship a methodology
        // playbook; each must be non-empty and carry its stage-specific red line.
        let scoping = stage_methodology_md(StageKind::Scoping).expect("scoping playbook");
        assert!(scoping.contains("do NOT probe") || scoping.contains("NO reconnaissance"));

        let ti = stage_methodology_md(StageKind::TargetIntel).expect("target_intel playbook");
        // The key fixes this stage encodes: no scan-tool fallback and no active
        // scan here.
        assert!(ti.contains("recon_map_assets"));
        assert!(ti.contains("recon_lookup_whois"));
        assert!(!ti.contains("subfinder"));
        assert!(!ti.contains("dig"));
        assert!(ti.to_lowercase().contains("once"));

        let eas = stage_methodology_md(StageKind::ExternalAttackSurface)
            .expect("external_attack_surface playbook");
        assert!(eas.contains("httpx"));
        assert!(eas.contains("skipped_checks"));

        let enumeration =
            stage_methodology_md(StageKind::Enumeration).expect("enumeration playbook");
        assert!(enumeration.contains("tested_units"));

        // Stages without a playbook return None (charter appends nothing).
        assert!(stage_methodology_md(StageKind::Cleanup).is_none());
        assert!(stage_methodology_md(StageKind::Reporting).is_none());
    }
}
