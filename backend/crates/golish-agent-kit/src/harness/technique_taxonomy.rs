//! Technique taxonomy registry (设计 2026-06-05-vuln-triage-technique-matrix §3 + D2「挂标准」).
//!
//! Phase 2 ②。`coverage_complete` / `coverage_denominator` 对 technique id 形态不敏感
//! （D1）；本 registry 是「挂标准」的词典：stage spec 的 `expected_techniques` 必须在此
//! 登记，写错 id（typo / 未登记）会被 [`tests::all_embedded_expected_techniques_are_recognized`]
//! fail-closed 抓住，杜绝「拼错 WSTG id 造出一个永远覆盖不了的矩阵列」。
//!
//! 值约定：OWASP WSTG id（叶子 / 类别级）/ MITRE ATT&CK id（`T####`）/ `GOLISH-*` 自定义
//! 命名空间（WSTG 没有的类，如 n-day）。新增技术类 = 先在 `technique_taxonomy.json` 登记，
//! 再用于 `expected_techniques`。词典本身是 JSON 资源（git 版本化、不入 DB）。

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

/// 一个技术类的元数据（registry 值）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TechniqueMeta {
    /// 人类可读名（如 "SQL Injection"）。
    pub name: String,
    /// 标准来源（如 "OWASP WSTG" / "MITRE ATT&CK" / "GOLISH custom"）。
    pub standard: String,
}

const TECHNIQUE_TAXONOMY_JSON: &str =
    include_str!("../../../../../resources/harness/technique_taxonomy.json");

/// 解析内嵌的技术词典（跳过 `$schema` / `$comment` 等 `$`-前缀元字段）。
/// `BTreeMap` 保证确定性遍历（测试可重放）。
pub fn load_technique_taxonomy() -> Result<BTreeMap<String, TechniqueMeta>, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_str(TECHNIQUE_TAXONOMY_JSON)?;
    let serde_json::Value::Object(map) = value else {
        return serde_json::from_value::<BTreeMap<String, TechniqueMeta>>(value);
    };
    map.into_iter()
        .filter(|(k, _)| !k.starts_with('$'))
        .map(|(k, v)| serde_json::from_value::<TechniqueMeta>(v).map(|m| (k, m)))
        .collect()
}

/// 进程级缓存的词典（首次访问加载）。解析失败回退空表——
/// [`tests::taxonomy_loads_and_is_nonempty`] 保证内嵌资源真能解析且非空，
/// 故生产期不会静默退化为空。
fn taxonomy() -> &'static BTreeMap<String, TechniqueMeta> {
    static TAXONOMY: LazyLock<BTreeMap<String, TechniqueMeta>> =
        LazyLock::new(|| load_technique_taxonomy().unwrap_or_default());
    &TAXONOMY
}

/// 该 technique id 是否已在词典登记（D2「挂标准」校验用）。
pub fn is_recognized(technique_id: &str) -> bool {
    taxonomy().contains_key(technique_id)
}

/// 查 technique 元数据（charter / 报告展示可用）。
pub fn lookup(technique_id: &str) -> Option<&'static TechniqueMeta> {
    taxonomy().get(technique_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::resources::load_embedded_stage_spec;
    use crate::harness::types::StageKind;

    const ALL_KINDS: [StageKind; 13] = [
        StageKind::Scoping,
        StageKind::TargetIntel,
        StageKind::ExternalAttackSurface,
        StageKind::Enumeration,
        StageKind::VulnTriage,
        StageKind::AttackCandidate,
        StageKind::Verification,
        StageKind::AccessValidation,
        StageKind::InternalDiscovery,
        StageKind::ObjectivePathing,
        StageKind::ObjectiveSimulation,
        StageKind::Cleanup,
        StageKind::Reporting,
    ];

    #[test]
    fn taxonomy_loads_and_is_nonempty() {
        let t = load_technique_taxonomy().expect("technique_taxonomy.json parses");
        assert!(!t.is_empty(), "registry must not be empty");
        // sanity：几个关键 id 在册 + 元数据可读。
        assert_eq!(
            t.get("WSTG-INPV-05").map(|m| m.name.as_str()),
            Some("SQL Injection")
        );
        assert!(t.contains_key("GOLISH-NDAY"));
    }

    #[test]
    fn known_and_unknown_ids() {
        assert!(is_recognized("WSTG-INPV-05"));
        assert!(is_recognized("GOLISH-NDAY"));
        assert!(!is_recognized("WSTG-BOGUS-99"));
        assert!(!is_recognized(""));
        assert_eq!(
            lookup("WSTG-INPV-19").map(|m| m.standard.as_str()),
            Some("OWASP WSTG")
        );
        assert!(lookup("nope").is_none());
    }

    /// D2「挂标准」fail-closed 校验：每个 stage spec 声明的 `expected_techniques`
    /// 都必须在 `technique_taxonomy.json` 登记，杜绝 typo / 未登记 id 造出永远覆盖
    /// 不了的矩阵列。新增技术类时，先在词典登记再用于 `expected_techniques`。
    #[test]
    fn all_embedded_expected_techniques_are_recognized() {
        for kind in ALL_KINDS {
            let spec = load_embedded_stage_spec(kind)
                .unwrap_or_else(|e| panic!("load {kind:?} spec failed: {e}"));
            for tech in &spec.expected_techniques {
                assert!(
                    is_recognized(tech),
                    "stage {kind:?} expected_technique {tech:?} is not registered in \
                     technique_taxonomy.json (typo? or add it to the registry first)"
                );
            }
        }
    }

    // Attack-stage split (design 2026-07-02 §3.9): vuln_triage was narrowed from
    // the original 15 WSTG classes to the 10 FORMULAIC ones (tool+dictionary
    // batchable, relatively objective verdict). The 5 reasoning-heavy classes
    // (SSTI/SSRF/LFI/auth-bypass logic/business logic) moved to attack_candidate.
    #[test]
    fn vuln_triage_formulaic_techniques_all_recognized() {
        let spec = load_embedded_stage_spec(StageKind::VulnTriage).unwrap();
        assert_eq!(spec.expected_techniques.len(), 10);
        for tech in &spec.expected_techniques {
            assert!(
                is_recognized(tech),
                "{tech} must be registered in technique_taxonomy.json"
            );
        }
        // The formulaic n-day class stays; reasoning-heavy classes are gone.
        assert!(spec
            .expected_techniques
            .contains(&"GOLISH-NDAY".to_string()));
        for moved in [
            "WSTG-INPV-18", // SSTI
            "WSTG-INPV-19", // SSRF
            "WSTG-ATHZ-01", // path traversal / LFI
            "WSTG-ATHN-04", // auth-bypass logic
            "WSTG-BUSL",    // business logic
        ] {
            assert!(
                !spec.expected_techniques.contains(&moved.to_string()),
                "{moved} must move out of vuln_triage to attack_candidate"
            );
        }
    }
}
