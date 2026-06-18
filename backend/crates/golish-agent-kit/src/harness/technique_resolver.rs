//! 按 stage + in-scope 资产类型动态产出 coverage_complete 的期望技术清单
//! （设计 2026-06-05-coverage-matrix §6.5 ③ seam 的动态生成器）。纯函数、无 IO。
//! 输出 id 与 resources/harness/stages/*.json 的 expected_techniques 同命名空间。

use crate::harness::types::StageKind;

// AssetClass 的单一来源已迁到 golish-pentest-domain（design 2026-06-18 D1）；
// 此处重导出，保持 `technique_resolver::AssetClass` 既有引用零改动。
pub use golish_pentest_domain::AssetClass;

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

/// Host-aware coverage (design 2026-06-15 §3): whether `tech` (one of `stage`'s
/// baseline techniques) applies to a single asset of `class`. Differentiates
/// `TargetIntel` (2a), `ExternalAttackSurface` + `Enumeration` (2b); any other
/// stage returns `true` (no-op). `Other` keeps every technique (fail-safe: an
/// unclassified asset is never under-checked).
pub fn technique_applies(stage: StageKind, class: AssetClass, tech: &str) -> bool {
    use AssetClass::*;
    if matches!(class, Other) {
        return true;
    }
    match stage {
        StageKind::TargetIntel => match tech {
            // Subdomain enumeration only makes sense for a domain.
            "GOLISH-INTEL-SUBDOMAIN" => matches!(class, Domain),
            // Forward DNS + cert transparency are domain/host concepts; a bare
            // IP/CIDR has neither a self-keyed forward A record nor a CT log.
            "GOLISH-INTEL-DNS" | "GOLISH-INTEL-CT" => matches!(class, Domain | Url),
            // WHOIS / ASN / OSINT apply to every class (org/netblock-wide).
            _ => true,
        },
        // Host-aware coverage 2b (design 2026-06-15 §3.2): EAS is host-level.
        // LIVENESS applies to anything with a host; PORT / SERVICE-FINGERPRINT
        // are host-level too, but a single URL endpoint is not itself a
        // port-scan / service-fingerprint target (its host is covered by the
        // host/IP asset). Domain/IP/CIDR keep all three.
        StageKind::ExternalAttackSurface => match tech {
            "GOLISH-EAS-PORT" | "GOLISH-EAS-SERVICE-FINGERPRINT" => !matches!(class, Url),
            _ => true,
        },
        // Host-aware coverage 2b (design 2026-06-15 §3.3): content enumeration
        // (DIR / PARAM / JSAPI) is web-level — only a web-capable asset (domain
        // or URL) is a content-enumeration target; a bare IP/CIDR is not (the
        // per-asset form of the existing scope-level no-web PARAM drop).
        StageKind::Enumeration => matches!(class, Domain | Url),
        _ => true,
    }
}

/// Convenience: the subset of `stage`'s baseline that applies to `class`
/// (= [`technique_applies`] over [`stage_baseline`]). For tests/diagnostics.
pub fn techniques_for(stage: StageKind, class: AssetClass) -> Vec<String> {
    stage_baseline(stage)
        .into_iter()
        .filter(|t| technique_applies(stage, class, t))
        .map(String::from)
        .collect()
}

/// Value-aware extension of [`technique_applies`]: whether `tech` applies to a
/// specific in-scope `value` (not just its `class`).
///
/// TargetIntel SUBDOMAIN is a **registrable-apex** concern: you enumerate the
/// subdomains of a root (`niuza.com`), not of a leaf subdomain (`s.niuza.com`).
/// The passive-intel auto-landing registers every discovered subdomain as its own
/// `scope='in'` target, and most of their roots are NOT in-scope, so
/// `coverage_anchor_only` (which only collapses subdomains of an *in-scope* parent)
/// cannot fold them — leaving one unsatisfiable SUBDOMAIN cell per leaf (the
/// treadmill that pinned every engagement at its iteration cap → gate dead-loop).
/// Exempting non-apex domains from SUBDOMAIN removes that per-leaf requirement;
/// apex roots still require it. Only relaxes (never adds) a requirement.
pub fn technique_applies_to_value(
    stage: StageKind,
    class: AssetClass,
    value: &str,
    tech: &str,
) -> bool {
    if !technique_applies(stage, class, tech) {
        return false;
    }
    if matches!(stage, StageKind::TargetIntel)
        && tech == "GOLISH-INTEL-SUBDOMAIN"
        && matches!(class, AssetClass::Domain)
        && !is_registrable_apex(value)
    {
        return false;
    }
    true
}

/// True when `value`'s host is its own registrable apex (`niuza.com`,
/// `pingan.com.cn`) rather than a leaf subdomain (`s.niuza.com`,
/// `a.pingan.com.cn`). Best-effort 2-level-TLD heuristic mirroring the recon
/// `registrable_domain` (no public-suffix list); `www.` is treated as the apex.
fn is_registrable_apex(value: &str) -> bool {
    const SECOND_LEVEL: &[&str] = &["com", "net", "org", "gov", "edu", "co", "ac"];
    let host = value.trim().trim_end_matches('.').to_ascii_lowercase();
    let host = host.trim_start_matches("www.");
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    let len = labels.len();
    let apex_len =
        if len >= 3 && labels[len - 1].len() == 2 && SECOND_LEVEL.contains(&labels[len - 2]) {
            3
        } else {
            2
        };
    len <= apex_len
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

    // ── Host-aware coverage (design 2026-06-15 §3, Phase 2a) ──────────────────

    #[test]
    fn url_wrapped_ip_is_ip_not_url() {
        // A URL whose host is a raw IP is an IP asset (was wrongly Url before the
        // fix, which forced CT/DNS/SUBDOMAIN on an address that can satisfy none).
        assert!(AssetClass::is_url_wrapped_ip("http://124.196.77.48"));
        assert!(AssetClass::is_url_wrapped_ip("https://124.196.77.48:8443/x"));
        assert!(AssetClass::is_url_wrapped_ip("https://[2606:4700::1111]:443/"));
        assert!(!AssetClass::is_url_wrapped_ip("https://a.com/x"));
        assert!(!AssetClass::is_url_wrapped_ip("124.196.77.48")); // bare IP, no scheme
        assert_eq!(AssetClass::from_value("http://124.196.77.48"), AssetClass::Ip);
        assert_eq!(AssetClass::from_value("https://a.com"), AssetClass::Url);
        // host-aware: a url-wrapped IP drops the domain-only intel techniques.
        let t = techniques_for(
            StageKind::TargetIntel,
            AssetClass::from_value("http://124.196.77.48"),
        );
        assert!(!t.contains(&"GOLISH-INTEL-CT".to_string()));
        assert!(!t.contains(&"GOLISH-INTEL-SUBDOMAIN".to_string()));
        assert!(!t.contains(&"GOLISH-INTEL-DNS".to_string()));
    }

    #[test]
    fn classify_overrides_mistyped_url_wrapped_ip_only() {
        // The 22 live URL-wrapped IPs are stored as targets.type='domain'; classify
        // must override that to Ip (cert transparency / subdomain don't apply) …
        assert_eq!(
            AssetClass::classify(Some("domain"), "http://124.196.77.48"),
            AssetClass::Ip
        );
        // … but a bare IP-looking value with an authoritative domain type still
        // trusts the type (preserves host_aware_uses_authoritative_type_over_value).
        assert_eq!(
            AssetClass::classify(Some("domain"), "1.2.3.4"),
            AssetClass::Domain
        );
        // None type falls back to value inference.
        assert_eq!(AssetClass::classify(None, "a.com"), AssetClass::Domain);
        assert_eq!(AssetClass::classify(Some("ip"), "9.9.9.9"), AssetClass::Ip);
    }

    #[test]
    fn target_intel_drops_domain_only_techniques_for_ip() {
        let ip = techniques_for(StageKind::TargetIntel, AssetClass::Ip);
        assert!(!ip.contains(&"GOLISH-INTEL-SUBDOMAIN".to_string()));
        assert!(!ip.contains(&"GOLISH-INTEL-DNS".to_string()));
        assert!(!ip.contains(&"GOLISH-INTEL-CT".to_string()));
        assert!(ip.contains(&"GOLISH-INTEL-WHOIS".to_string()));
        assert!(ip.contains(&"GOLISH-INTEL-ASN".to_string()));
        assert!(ip.contains(&"GOLISH-INTEL-OSINT".to_string()));
        // CIDR matches IP.
        assert_eq!(
            techniques_for(StageKind::TargetIntel, AssetClass::Cidr),
            techniques_for(StageKind::TargetIntel, AssetClass::Ip)
        );
        // Domain keeps all 6.
        assert_eq!(techniques_for(StageKind::TargetIntel, AssetClass::Domain).len(), 6);
        // URL keeps host intel (DNS/CT) but not subdomain enumeration.
        let url = techniques_for(StageKind::TargetIntel, AssetClass::Url);
        assert!(!url.contains(&"GOLISH-INTEL-SUBDOMAIN".to_string()));
        assert!(url.contains(&"GOLISH-INTEL-DNS".to_string()));
        assert!(url.contains(&"GOLISH-INTEL-CT".to_string()));
    }

    #[test]
    fn other_class_keeps_full_set_failsafe() {
        assert_eq!(techniques_for(StageKind::TargetIntel, AssetClass::Other).len(), 6);
    }

    #[test]
    fn is_registrable_apex_distinguishes_roots_from_leaves() {
        assert!(is_registrable_apex("niuza.com"));
        assert!(is_registrable_apex("pingan.com.cn"));
        assert!(is_registrable_apex("www.niuza.com")); // www treated as the apex
        assert!(!is_registrable_apex("s.niuza.com"));
        assert!(!is_registrable_apex("icorepnbs.pingan-property.com.cn"));
        assert!(!is_registrable_apex("a.b.pingan.com.cn"));
    }

    #[test]
    fn subdomain_required_only_on_registrable_apex() {
        use StageKind::TargetIntel as Ti;
        // Apex roots still require subdomain enumeration …
        assert!(technique_applies_to_value(
            Ti,
            AssetClass::Domain,
            "niuza.com",
            "GOLISH-INTEL-SUBDOMAIN"
        ));
        assert!(technique_applies_to_value(
            Ti,
            AssetClass::Domain,
            "pingan.com.cn",
            "GOLISH-INTEL-SUBDOMAIN"
        ));
        // … leaf subdomains (passively discovered + landed in-scope) do NOT —
        // killing the per-leaf SUBDOMAIN treadmill that dead-looped the gate.
        assert!(!technique_applies_to_value(
            Ti,
            AssetClass::Domain,
            "s.niuza.com",
            "GOLISH-INTEL-SUBDOMAIN"
        ));
        assert!(!technique_applies_to_value(
            Ti,
            AssetClass::Domain,
            "icorepnbs.pingan-property.com.cn",
            "GOLISH-INTEL-SUBDOMAIN"
        ));
        // DNS/CT still apply to a leaf domain (only SUBDOMAIN is apex-scoped).
        assert!(technique_applies_to_value(
            Ti,
            AssetClass::Domain,
            "s.niuza.com",
            "GOLISH-INTEL-DNS"
        ));
        assert!(technique_applies_to_value(
            Ti,
            AssetClass::Domain,
            "s.niuza.com",
            "GOLISH-INTEL-CT"
        ));
        // IP class never gets SUBDOMAIN regardless (delegates to technique_applies).
        assert!(!technique_applies_to_value(
            Ti,
            AssetClass::Ip,
            "1.2.3.4",
            "GOLISH-INTEL-SUBDOMAIN"
        ));
        // WHOIS/ASN/OSINT apply to every asset including a leaf (org-level).
        assert!(technique_applies_to_value(
            Ti,
            AssetClass::Domain,
            "s.niuza.com",
            "GOLISH-INTEL-WHOIS"
        ));
    }

    #[test]
    fn eas_does_not_differentiate_ip_from_domain() {
        // EAS is host-level: IP and domain both keep the full host set (only a
        // bare URL endpoint drops PORT / SERVICE-FINGERPRINT — see the 2b test).
        assert_eq!(
            techniques_for(StageKind::ExternalAttackSurface, AssetClass::Ip).len(),
            techniques_for(StageKind::ExternalAttackSurface, AssetClass::Domain).len()
        );
    }

    // ── Host-aware coverage 2b: EAS + enumeration matrices (design §3.2/§3.3) ──

    #[test]
    fn eas_drops_port_and_service_fp_for_url_only() {
        use StageKind::ExternalAttackSurface as Eas;
        // A bare URL endpoint is not itself a port-scan / service-fingerprint
        // target (its host is covered via the host/IP asset); it keeps LIVENESS.
        let url = techniques_for(Eas, AssetClass::Url);
        assert!(url.contains(&"GOLISH-EAS-LIVENESS".to_string()));
        assert!(!url.contains(&"GOLISH-EAS-PORT".to_string()));
        assert!(!url.contains(&"GOLISH-EAS-SERVICE-FINGERPRINT".to_string()));
        // domain / ip / cidr are host-level → keep all 3.
        assert_eq!(techniques_for(Eas, AssetClass::Domain).len(), 3);
        assert_eq!(techniques_for(Eas, AssetClass::Ip).len(), 3);
        assert_eq!(techniques_for(Eas, AssetClass::Cidr).len(), 3);
        // fail-safe: unknown keeps the full set.
        assert_eq!(techniques_for(Eas, AssetClass::Other).len(), 3);
    }

    #[test]
    fn enumeration_is_web_only_per_asset() {
        use StageKind::Enumeration as Enum;
        // Content enumeration (DIR/PARAM/JSAPI) is web-level: domain + URL get
        // the full set; a bare IP/CIDR is not a content-enumeration target.
        assert_eq!(techniques_for(Enum, AssetClass::Domain).len(), 3);
        assert_eq!(techniques_for(Enum, AssetClass::Url).len(), 3);
        assert!(techniques_for(Enum, AssetClass::Ip).is_empty());
        assert!(techniques_for(Enum, AssetClass::Cidr).is_empty());
        // fail-safe: unknown keeps the full set (never under-checked).
        assert_eq!(techniques_for(Enum, AssetClass::Other).len(), 3);
    }
}
