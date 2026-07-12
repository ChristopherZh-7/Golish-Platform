//! 按 stage + in-scope 资产类型动态产出 coverage_complete 的期望技术清单
//! （设计 2026-06-05-coverage-matrix §6.5 ③ seam 的动态生成器）。纯函数、无 IO。
//! 输出 id 与 resources/harness/stages/*.json 的 expected_techniques 同命名空间。

use crate::harness::types::StageKind;

// AssetClass 的单一来源已迁到 golish-pentest-domain（design 2026-06-18 D1）；
// 此处重导出，保持 `technique_resolver::AssetClass` 既有引用零改动。
pub use golish_pentest_domain::AssetClass;

/// Classify one coverage row using the stage's authorization boundary.
///
/// EAS PORT/SERVICE work is permitted only for an explicitly typed IP/CIDR
/// target. Domain/URL rows remain vhost/origin scoped even when their value is an
/// HTTP URL whose host is an IP literal; stripping the scheme there would
/// silently promote an exact-origin seed into authorization for its bare host.
/// Other stages retain the shared host-identity classification contract.
pub fn classify_stage_asset(
    stage: StageKind,
    target_type: Option<&str>,
    value: &str,
) -> AssetClass {
    if stage == StageKind::ExternalAttackSurface {
        match target_type.map(AssetClass::from_target_type) {
            Some(class @ (AssetClass::Domain | AssetClass::Url)) => return class,
            Some(class @ (AssetClass::Ip | AssetClass::Cidr)) => return class,
            Some(AssetClass::Other) | None => {
                if golish_pentest_domain::canonical_web_origin(value).is_some() {
                    return AssetClass::Url;
                }
            }
        }
    }
    AssetClass::classify(target_type, value)
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
            "GOLISH-EAS-WEB-FINGERPRINT",
        ],
        StageKind::Enumeration => vec![
            "GOLISH-ENUM-JS",
            "GOLISH-ENUM-DIR",
            "GOLISH-ENUM-PARAM",
            "GOLISH-ENUM-JSAPI",
        ],
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
        // Host-aware coverage 2b (design 2026-06-15 §3.2): EAS splits vhost
        // liveness from host/IP mapping. Domain/URL assets can prove LIVENESS,
        // but PORT / SERVICE-FINGERPRINT only apply to concrete IP/CIDR host
        // assets. WEB-FINGERPRINT is evidence-aware and only applies after this
        // asset has a confirmed HTTP(S) surface; see `technique_applies_web_aware`.
        // If target_intel has not registered a concrete IP, EAS must not turn
        // the domain string into a port-scan target.
        StageKind::ExternalAttackSurface => match tech {
            "GOLISH-EAS-PORT" => matches!(class, Ip | Cidr),
            // A CIDR discovery wave records range-level LIVENESS/PORT and
            // materialises concrete child IP targets. Service/version work is
            // intentionally owned by those IPs in the supplemental wave;
            // `eas_fingerprint_services` never accepts a range directly.
            "GOLISH-EAS-SERVICE-FINGERPRINT" => matches!(class, Ip),
            "GOLISH-EAS-WEB-FINGERPRINT" => false,
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
    // A wildcard target is an authorization pattern, never an executable host.
    // Target Intel nevertheless keeps one passive SUBDOMAIN responsibility so a
    // failed/empty expansion remains distinguishable from "not applicable".
    // Concrete children materialized as domain targets own every later stage.
    if wildcard_scope_pattern(value) {
        return match stage {
            StageKind::TargetIntel => tech == "GOLISH-INTEL-SUBDOMAIN",
            StageKind::ExternalAttackSurface | StageKind::Enumeration => false,
            _ => technique_applies(stage, class, tech),
        };
    }
    if !technique_applies(stage, class, tech) {
        return false;
    }
    if matches!(stage, StageKind::TargetIntel)
        && tech == "GOLISH-INTEL-SUBDOMAIN"
        && matches!(class, AssetClass::Domain)
        && (!is_registrable_apex(value) || is_www_prefixed_host(value))
    {
        return false;
    }
    true
}

fn wildcard_scope_pattern(value: &str) -> bool {
    let Some(base) = value.trim().trim_end_matches('.').strip_prefix("*.") else {
        return false;
    };
    let Some(key) = golish_pentest_domain::canonical_asset_key(base) else {
        return false;
    };
    key.class == golish_pentest_domain::AssetClass::Domain && key.key.contains('.')
}

/// Whether `candidate` is a concrete domain child whose Target Intel work is
/// represented by `anchor`'s passive SUBDOMAIN responsibility.
///
/// Both exact domain roots and wildcard authorization patterns may anchor strict
/// children. A wildcard never covers its apex, and wildcard candidates are kept
/// as independent authorization patterns. URL/IP/CIDR/organization display
/// strings are rejected so suffix-shaped non-domain identities cannot collapse
/// the denominator.
pub fn target_intel_anchor_covers_child(anchor: &str, candidate: &str) -> bool {
    let anchor = anchor.trim().trim_end_matches('.').to_ascii_lowercase();
    let candidate = candidate.trim().trim_end_matches('.').to_ascii_lowercase();
    if anchor.is_empty()
        || candidate.is_empty()
        || candidate.starts_with("*.")
        || anchor.contains("://")
        || candidate.contains("://")
    {
        return false;
    }
    let anchor_base = anchor.strip_prefix("*.").unwrap_or(&anchor);
    let Some(anchor_key) = golish_pentest_domain::canonical_asset_key(anchor_base) else {
        return false;
    };
    let Some(candidate_key) = golish_pentest_domain::canonical_asset_key(&candidate) else {
        return false;
    };
    if anchor_key.class != golish_pentest_domain::AssetClass::Domain
        || candidate_key.class != golish_pentest_domain::AssetClass::Domain
    {
        return false;
    }
    candidate_key.key != anchor_key.key
        && candidate_key.key.ends_with(&format!(".{}", anchor_key.key))
}

/// Evidence-aware extension of [`technique_applies_to_value`]. Only
/// `Enumeration + Ip/Cidr` changes: an exact normalized HTTP(S) origin is
/// intrinsically web-capable, while a bare IP/CIDR still requires EAS/httpx
/// proof before it participates in the full content-enumeration axis.
pub fn technique_applies_web_aware(
    stage: StageKind,
    class: AssetClass,
    value: &str,
    tech: &str,
    web_capable: bool,
) -> bool {
    // The evidence-aware branches below intentionally bypass the static
    // technique matrix for WEB work, so repeat the value-level authorization
    // guard before either special case. Target Intel keeps only its passive
    // child-expansion responsibility; evidence can never turn a wildcard into a
    // Web Origin or any Enumeration/EAS work item.
    if wildcard_scope_pattern(value) {
        return match stage {
            StageKind::TargetIntel => tech == "GOLISH-INTEL-SUBDOMAIN",
            StageKind::ExternalAttackSurface | StageKind::Enumeration => false,
            _ => technique_applies_to_value(stage, class, value, tech),
        };
    }
    if matches!(stage, StageKind::ExternalAttackSurface) && tech == "GOLISH-EAS-WEB-FINGERPRINT" {
        return web_capable
            && matches!(
                class,
                AssetClass::Domain
                    | AssetClass::Url
                    | AssetClass::Ip
                    | AssetClass::Cidr
                    | AssetClass::Other
            );
    }
    if matches!(stage, StageKind::Enumeration) && matches!(class, AssetClass::Ip | AssetClass::Cidr)
    {
        // `AssetClass::classify` intentionally treats `https://1.2.3.4:443`
        // as an IP asset even when the authoritative axis labels the row
        // `url`. Enumeration's authoritative denominator is now exact Web
        // Origins, so requiring the expanded origin string to also appear in
        // the raw-IP `web_capable_assets` set would vacuously drop all four
        // axes. A canonical origin already carries the missing web capability;
        // only bare IP/CIDR values need the separate EAS proof bit.
        let exact_web_origin = golish_pentest_domain::canonical_web_origin(value).is_some();
        return (web_capable || exact_web_origin)
            && matches!(
                tech,
                "GOLISH-ENUM-JS" | "GOLISH-ENUM-DIR" | "GOLISH-ENUM-PARAM" | "GOLISH-ENUM-JSAPI"
            );
    }
    technique_applies_to_value(stage, class, value, tech)
}

/// True when `value`'s host is its own registrable apex (`niuza.com`,
/// `pingan.com.cn`) rather than a leaf subdomain (`s.niuza.com`,
/// `a.pingan.com.cn`). The shared domain helper still treats `www.` as apex for
/// compatibility, so `technique_applies_to_value` adds the target_intel-specific
/// `www.*` exemption above. Delegates to the single source
/// [`golish_pentest_domain::is_registrable_apex`], which recon's
/// `registrable_domain` also uses — keeping the two-level-TLD table in one place
/// so the gate and recon can never drift (the duplicated table previously missed
/// ccTLD second levels like `.ne.jp`, mis-classifying real apexes as leaves).
fn is_registrable_apex(value: &str) -> bool {
    golish_pentest_domain::is_registrable_apex(value)
}

fn is_www_prefixed_host(value: &str) -> bool {
    let host = value
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .unwrap_or(value)
        .split('@')
        .next_back()
        .unwrap_or(value)
        .split(':')
        .next()
        .unwrap_or(value)
        .trim()
        .trim_matches('.')
        .to_ascii_lowercase();
    host.strip_prefix("www.")
        .is_some_and(|rest| rest.contains('.'))
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
        assert!(t.contains(&"GOLISH-EAS-WEB-FINGERPRINT".to_string()));
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
        assert!(AssetClass::is_url_wrapped_ip(
            "https://124.196.77.48:8443/x"
        ));
        assert!(AssetClass::is_url_wrapped_ip(
            "https://[2606:4700::1111]:443/"
        ));
        assert!(!AssetClass::is_url_wrapped_ip("https://a.com/x"));
        assert!(!AssetClass::is_url_wrapped_ip("124.196.77.48")); // bare IP, no scheme
        assert_eq!(
            AssetClass::from_value("http://124.196.77.48"),
            AssetClass::Ip
        );
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
    fn classify_overrides_mistyped_url_values() {
        // Live URL-shaped assets may be stored as targets.type='domain'; classify
        // must trust URL syntax so URL-only values do not pick up SUBDOMAIN.
        assert_eq!(
            AssetClass::classify(Some("domain"), "http://124.196.77.48"),
            AssetClass::Ip
        );
        assert_eq!(
            AssetClass::classify(Some("domain"), "https://a.com/login"),
            AssetClass::Url
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
    fn eas_classification_never_promotes_ip_origin_url_to_bare_ip() {
        let origin = "http://127.0.0.1:54537";
        assert_eq!(
            classify_stage_asset(StageKind::ExternalAttackSurface, Some("url"), origin),
            AssetClass::Url
        );
        assert_eq!(
            classify_stage_asset(StageKind::ExternalAttackSurface, Some("domain"), origin),
            AssetClass::Domain
        );
        assert_eq!(
            classify_stage_asset(StageKind::ExternalAttackSurface, None, origin),
            AssetClass::Url
        );
        assert_eq!(
            classify_stage_asset(StageKind::ExternalAttackSurface, Some("ip_address"), origin),
            AssetClass::Ip
        );
        assert_eq!(
            classify_stage_asset(StageKind::Enumeration, Some("url"), origin),
            AssetClass::Ip,
            "Enumeration retains its exact-origin host classification contract"
        );
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
        assert_eq!(
            techniques_for(StageKind::TargetIntel, AssetClass::Domain).len(),
            6
        );
        // URL keeps host intel (DNS/CT) but not subdomain enumeration.
        let url = techniques_for(StageKind::TargetIntel, AssetClass::Url);
        assert!(!url.contains(&"GOLISH-INTEL-SUBDOMAIN".to_string()));
        assert!(url.contains(&"GOLISH-INTEL-DNS".to_string()));
        assert!(url.contains(&"GOLISH-INTEL-CT".to_string()));
    }

    #[test]
    fn other_class_keeps_full_set_failsafe() {
        assert_eq!(
            techniques_for(StageKind::TargetIntel, AssetClass::Other).len(),
            6
        );
    }

    #[test]
    fn is_registrable_apex_distinguishes_roots_from_leaves() {
        assert!(is_registrable_apex("niuza.com"));
        assert!(is_registrable_apex("pingan.com.cn"));
        assert!(is_registrable_apex("www.niuza.com")); // shared domain helper still treats www as apex
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
        assert!(!technique_applies_to_value(
            Ti,
            AssetClass::Domain,
            "www.niuza.com",
            "GOLISH-INTEL-SUBDOMAIN"
        ));
        assert!(!technique_applies_to_value(
            Ti,
            AssetClass::Domain,
            "https://www.niuza.com/login",
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
    fn subdomain_required_on_cctld_second_level_apex() {
        // ③ 修复回归：`.ne.jp`（日本组织类二级域）下的 apex 必须被要求做 SUBDOMAIN
        // 枚举（旧表漏 "ne" 时它被误判成叶子 → 漏枚举该根域的子域）。其子域仍免。
        use StageKind::TargetIntel as Ti;
        assert!(technique_applies_to_value(
            Ti,
            AssetClass::Domain,
            "example.ne.jp",
            "GOLISH-INTEL-SUBDOMAIN"
        ));
        assert!(!technique_applies_to_value(
            Ti,
            AssetClass::Domain,
            "s.example.ne.jp",
            "GOLISH-INTEL-SUBDOMAIN"
        ));
    }

    #[test]
    fn eas_port_applies_to_ip_or_cidr_but_service_requires_concrete_ip() {
        let domain = techniques_for(StageKind::ExternalAttackSurface, AssetClass::Domain);
        assert_eq!(domain, vec!["GOLISH-EAS-LIVENESS".to_string()]);

        let ip = techniques_for(StageKind::ExternalAttackSurface, AssetClass::Ip);
        assert!(ip.contains(&"GOLISH-EAS-PORT".to_string()));
        assert!(ip.contains(&"GOLISH-EAS-SERVICE-FINGERPRINT".to_string()));

        let cidr = techniques_for(StageKind::ExternalAttackSurface, AssetClass::Cidr);
        assert!(cidr.contains(&"GOLISH-EAS-PORT".to_string()));
        assert!(!cidr.contains(&"GOLISH-EAS-SERVICE-FINGERPRINT".to_string()));
    }

    // ── Host-aware coverage 2b: EAS + enumeration matrices (design §3.2/§3.3) ──

    #[test]
    fn eas_drops_port_and_service_fp_for_non_ip_hosts() {
        use StageKind::ExternalAttackSurface as Eas;
        // A bare URL endpoint is not itself a port-scan / service-fingerprint
        // target (its host is covered via the host/IP asset); it keeps LIVENESS.
        let url = techniques_for(Eas, AssetClass::Url);
        assert!(url.contains(&"GOLISH-EAS-LIVENESS".to_string()));
        assert!(!url.contains(&"GOLISH-EAS-PORT".to_string()));
        assert!(!url.contains(&"GOLISH-EAS-SERVICE-FINGERPRINT".to_string()));
        // Domains are vhost/liveness only; host-level PORT/SERVICE belongs to a
        // concrete IP/CIDR target registered by target_intel.
        let domain = techniques_for(Eas, AssetClass::Domain);
        assert_eq!(domain, vec!["GOLISH-EAS-LIVENESS".to_string()]);
        // Concrete IPs keep liveness/port/service. CIDRs keep range-level
        // liveness/port; discovered child IPs own service in the next wave.
        // Web-fingerprint is evidence-aware and appears only after a web surface.
        assert_eq!(techniques_for(Eas, AssetClass::Ip).len(), 3);
        assert_eq!(techniques_for(Eas, AssetClass::Cidr).len(), 2);
        // fail-safe: unknown keeps the full set.
        assert_eq!(techniques_for(Eas, AssetClass::Other).len(), 4);
    }

    #[test]
    fn eas_web_fingerprint_only_applies_after_web_surface_is_confirmed() {
        use StageKind::ExternalAttackSurface as Eas;
        assert!(!technique_applies_web_aware(
            Eas,
            AssetClass::Domain,
            "app.example.com",
            "GOLISH-EAS-WEB-FINGERPRINT",
            false
        ));
        assert!(technique_applies_web_aware(
            Eas,
            AssetClass::Domain,
            "app.example.com",
            "GOLISH-EAS-WEB-FINGERPRINT",
            true
        ));
        assert!(technique_applies_web_aware(
            Eas,
            AssetClass::Ip,
            "203.0.113.10",
            "GOLISH-EAS-WEB-FINGERPRINT",
            true
        ));
        assert!(!technique_applies_web_aware(
            Eas,
            AssetClass::Ip,
            "203.0.113.10",
            "GOLISH-EAS-WEB-FINGERPRINT",
            false
        ));
    }

    #[test]
    fn enumeration_is_web_only_per_asset() {
        use StageKind::Enumeration as Enum;
        // Content enumeration (JS/DIR/PARAM/JSAPI, design 2026-07-01 §4) is web-level:
        // domain + URL get the full four; a bare IP/CIDR is not a content-enumeration
        // target here (PR-2 web-aware relaxation is a separate seam).
        assert_eq!(techniques_for(Enum, AssetClass::Domain).len(), 4);
        assert_eq!(techniques_for(Enum, AssetClass::Url).len(), 4);
        assert!(techniques_for(Enum, AssetClass::Ip).is_empty());
        assert!(techniques_for(Enum, AssetClass::Cidr).is_empty());
        // fail-safe: unknown keeps the full set (never under-checked).
        assert_eq!(techniques_for(Enum, AssetClass::Other).len(), 4);
    }

    #[test]
    fn enumeration_ip_web_capable_gets_all_four() {
        use StageKind::Enumeration as Enum;
        for tech in [
            "GOLISH-ENUM-JS",
            "GOLISH-ENUM-DIR",
            "GOLISH-ENUM-PARAM",
            "GOLISH-ENUM-JSAPI",
        ] {
            assert!(
                technique_applies_web_aware(Enum, AssetClass::Ip, "1.2.3.4", tech, true),
                "web-capable IP should require {tech}"
            );
            assert!(
                !technique_applies_web_aware(Enum, AssetClass::Ip, "1.2.3.4", tech, false),
                "non-web IP should not require {tech}"
            );
            assert!(
                technique_applies_web_aware(Enum, AssetClass::Domain, "a.com", tech, false),
                "domain parity should not depend on web_capable"
            );
            assert!(
                technique_applies_web_aware(
                    Enum,
                    AssetClass::Ip,
                    "https://1.2.3.4:443",
                    tech,
                    false,
                ),
                "an exact IP Web Origin is intrinsically web-capable for {tech}"
            );
        }
        assert!(technique_applies_web_aware(
            StageKind::TargetIntel,
            AssetClass::Domain,
            "a.com",
            "GOLISH-INTEL-WHOIS",
            true
        ));
    }

    #[test]
    fn wildcard_scope_pattern_only_owns_passive_intel_child_expansion() {
        for technique in stage_baseline(StageKind::TargetIntel) {
            assert_eq!(
                technique_applies_web_aware(
                    StageKind::TargetIntel,
                    AssetClass::Other,
                    "*.moresec.cn",
                    technique,
                    true,
                ),
                technique == "GOLISH-INTEL-SUBDOMAIN",
                "wildcard Target Intel responsibility drifted for {technique}"
            );
        }
        for stage in [StageKind::ExternalAttackSurface, StageKind::Enumeration] {
            for technique in stage_baseline(stage) {
                assert!(
                    !technique_applies_web_aware(
                        stage,
                        AssetClass::Other,
                        "*.moresec.cn",
                        technique,
                        true,
                    ),
                    "wildcard pattern must not own executable {stage:?}/{technique} work"
                );
            }
        }
    }

    #[test]
    fn target_intel_anchor_handles_wildcard_strict_children_without_aliasing_apex() {
        assert!(target_intel_anchor_covers_child(
            "*.moresec.cn",
            "app.moresec.cn"
        ));
        assert!(target_intel_anchor_covers_child(
            "moresec.cn",
            "app.moresec.cn"
        ));
        assert!(!target_intel_anchor_covers_child(
            "*.moresec.cn",
            "moresec.cn"
        ));
        assert!(!target_intel_anchor_covers_child(
            "moresec.cn",
            "*.moresec.cn"
        ));
        assert!(!target_intel_anchor_covers_child(
            "organization:moresec.cn",
            "app.moresec.cn"
        ));
        assert!(!target_intel_anchor_covers_child(
            "https://moresec.cn",
            "app.moresec.cn"
        ));
        assert!(!target_intel_anchor_covers_child(
            "*.moresec.cn",
            "app.vendor.net"
        ));
    }
}
