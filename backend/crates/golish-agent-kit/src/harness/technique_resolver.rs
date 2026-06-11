//! 按 stage + in-scope 资产类型动态产出 coverage_complete 的期望技术清单
//! （设计 2026-06-05-coverage-matrix §6.5 ③ seam 的动态生成器）。纯函数、无 IO。
//! 输出 id 与 resources/harness/stages/*.json 的 expected_techniques 同命名空间。

use crate::harness::types::StageKind;

/// in-scope 资产的粗分类（来自 `targets.type`），决定哪些技术适用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetClass {
    Domain,
    Ip,
    Url,
    Cidr,
    /// 未知/其它：保守当作"可能含 web"，不缩小技术集。
    Other,
}

impl AssetClass {
    /// 映射 `targets.type` 字符串（与 golish-pentest-domain TargetType 对齐）。
    pub fn from_target_type(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "domain" | "subdomain" | "host" => Self::Domain,
            "ip" | "ip_address" | "ipv4" | "ipv6" => Self::Ip,
            "url" | "endpoint" | "web" => Self::Url,
            "cidr" | "range" | "netblock" => Self::Cidr,
            _ => Self::Other,
        }
    }

    /// 该资产是否可能承载 web 服务（决定 PARAM / JSAPI / DIR 等 web 技术是否要求）。
    fn maybe_web(self) -> bool {
        matches!(self, Self::Domain | Self::Url | Self::Other)
    }
}

/// 该 stage 的完整静态技术集（与 stage JSON 的 expected_techniques 保持一致；
/// 这是回退基线，动态逻辑只在此之上"按资产类型裁剪"，绝不新增 stage 未声明的技术）。
fn stage_baseline(stage: StageKind) -> Vec<&'static str> {
    match stage {
        StageKind::TargetIntel => vec![
            "GOLISH-INTEL-DNS",
            "GOLISH-INTEL-WHOIS",
            "GOLISH-INTEL-ASN",
            "GOLISH-INTEL-CT",
            "GOLISH-INTEL-SUBDOMAIN",
            "GOLISH-INTEL-OSINT",
        ],
        StageKind::ExternalAttackSurface => vec![
            "GOLISH-EAS-LIVENESS",
            "GOLISH-EAS-PORT",
            "GOLISH-EAS-SERVICE-FINGERPRINT",
        ],
        StageKind::Enumeration => vec!["GOLISH-ENUM-DIR", "GOLISH-ENUM-PARAM", "GOLISH-ENUM-JSAPI"],
        // vuln_triage 的 15 类 WSTG 与具体服务强相关，Phase A 不裁剪（保持静态全集，
        // 走 spec 回退）；本 resolver 对它返回空 = 用 spec 静态值。
        _ => vec![],
    }
}

/// 主入口：按 stage + 资产类型集产出期望技术清单。
///
/// 规则（Phase A）：
///   - baseline 为空（如 scoping / vuln_triage）→ 返回空（gate 用 spec 静态值或 no-op）。
///   - 资产集为空 → 返回完整 baseline（不擅自缩小）。
///   - 否则按资产类型裁剪 web-only 技术：无任何 web 资产时去掉 GOLISH-ENUM-PARAM。
pub fn resolve_expected_techniques(stage: StageKind, assets: &[AssetClass]) -> Vec<String> {
    let baseline = stage_baseline(stage);
    if baseline.is_empty() {
        return vec![];
    }
    let any_web = assets.iter().any(|a| a.maybe_web());
    let has_assets = !assets.is_empty();
    baseline
        .into_iter()
        .filter(|t| {
            // 唯一的 Phase A 裁剪规则：有资产信息且全非 web 时，PARAM 不适用。
            !(has_assets && !any_web && *t == "GOLISH-ENUM-PARAM")
        })
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::types::StageKind;

    #[test]
    fn target_intel_returns_all_intel_techniques() {
        // 任意资产类型，target_intel 都核全部 6 类被动情报技术。
        let t = resolve_expected_techniques(StageKind::TargetIntel, &[AssetClass::Domain]);
        assert!(t.contains(&"GOLISH-INTEL-DNS".to_string()));
        assert!(t.contains(&"GOLISH-INTEL-WHOIS".to_string()));
        assert_eq!(t.len(), 6);
    }

    #[test]
    fn enumeration_drops_param_when_no_web_asset() {
        // 纯 IP（无 web）资产：内容枚举不要求 PARAM（参数发现对非 web 无意义）。
        let ip_only = resolve_expected_techniques(StageKind::Enumeration, &[AssetClass::Ip]);
        assert!(!ip_only.contains(&"GOLISH-ENUM-PARAM".to_string()));
        // 有 web 资产时 PARAM 回来。
        let web = resolve_expected_techniques(StageKind::Enumeration, &[AssetClass::Url]);
        assert!(web.contains(&"GOLISH-ENUM-PARAM".to_string()));
    }

    #[test]
    fn empty_asset_set_falls_back_to_stage_default() {
        // 无资产信息 → 回退该 stage 的完整静态集（不擅自缩小）。
        let t = resolve_expected_techniques(StageKind::ExternalAttackSurface, &[]);
        assert!(t.contains(&"GOLISH-EAS-LIVENESS".to_string()));
        assert!(t.contains(&"GOLISH-EAS-PORT".to_string()));
        assert!(t.contains(&"GOLISH-EAS-SERVICE-FINGERPRINT".to_string()));
    }

    #[test]
    fn stage_without_coverage_returns_empty() {
        // scoping / reporting 不做覆盖矩阵 → 空（coverage_complete no-op）。
        assert!(resolve_expected_techniques(StageKind::Scoping, &[AssetClass::Domain]).is_empty());
    }

    #[test]
    fn asset_class_parses_target_type_strings() {
        assert_eq!(AssetClass::from_target_type("domain"), AssetClass::Domain);
        assert_eq!(AssetClass::from_target_type("ip_address"), AssetClass::Ip);
        assert_eq!(AssetClass::from_target_type("url"), AssetClass::Url);
        // 未知 → Other（保守：当作可能有 web，不缩小技术集）
        assert_eq!(AssetClass::from_target_type("weird"), AssetClass::Other);
    }
}
