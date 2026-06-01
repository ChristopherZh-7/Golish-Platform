//! 嵌入式 harness 资源 registry (Phase B · Doc 3 §2 / §4).
//!
//! 所有 `resources/harness/**` 经此一处 `include_str!` 加载, 消灭散落在各
//! module / hook 里的 include_str! 字面路径. 按 `StageKind` / profile id 取
//! 对应 JSON 原文, 再交给 `stage_spec` / `profile` loader 解析.
//!
//! include_str! 相对路径以本文件 (`src/harness/resources.rs`) 为基准, 与
//! `stage_spec.rs` / `profile.rs` 的内联 fixture 同深度 (5 个 `../` 到 repo 根).

use super::profile::{load_profile_from_json, Profile, ProfileLoadError};
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

/// 按 stage kind 取嵌入的 stage spec JSON 原文.
///
/// 12 个 StageKind 全覆盖 (与 `resources/harness/stages/*.json` 一一对应).
pub fn stage_spec_json(kind: StageKind) -> &'static str {
    match kind {
        StageKind::Scoping => stage_json!("scoping.json"),
        StageKind::TargetIntel => stage_json!("target_intel.json"),
        StageKind::ExternalAttackSurface => stage_json!("external_attack_surface.json"),
        StageKind::Enumeration => stage_json!("enumeration.json"),
        StageKind::VulnTriage => stage_json!("vuln_triage.json"),
        StageKind::Verification => stage_json!("verification.json"),
        StageKind::AccessValidation => stage_json!("access_validation.json"),
        StageKind::InternalDiscovery => stage_json!("internal_discovery.json"),
        StageKind::ObjectivePathing => stage_json!("objective_pathing.json"),
        StageKind::ObjectiveSimulation => stage_json!("objective_simulation.json"),
        StageKind::Cleanup => stage_json!("cleanup.json"),
        StageKind::Reporting => stage_json!("reporting.json"),
    }
}

/// 按 profile id 取嵌入的 profile JSON 原文; 未知 id 返回 None.
pub fn profile_json(id: &str) -> Option<&'static str> {
    Some(match id {
        "assessment" => profile_json_raw!("assessment.json"),
        "pentest" => profile_json_raw!("pentest.json"),
        "red_team" => profile_json_raw!("red_team.json"),
        "bug_bounty" => profile_json_raw!("bug_bounty.json"),
        "cloud_assessment" => profile_json_raw!("cloud_assessment.json"),
        _ => return None,
    })
}

/// 按 kind 加载 + 解析 stage spec.
pub fn load_embedded_stage_spec(kind: StageKind) -> Result<StageSpec, StageSpecLoadError> {
    load_stage_spec_from_json(stage_spec_json(kind))
}

/// 按 id 加载 + 解析 profile; 未知 id 返回 `Ok(None)`.
pub fn load_embedded_profile(id: &str) -> Result<Option<Profile>, ProfileLoadError> {
    match profile_json(id) {
        Some(raw) => Ok(Some(load_profile_from_json(raw)?)),
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
    fn all_five_profiles_load() {
        for id in [
            "assessment",
            "pentest",
            "red_team",
            "bug_bounty",
            "cloud_assessment",
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
}
