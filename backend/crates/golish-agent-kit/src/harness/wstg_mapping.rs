//! Nuclei canonical tag ↔ 注册 WSTG technique id 确定性映射
//! （设计 `docs/superpowers/plans/2026-07-02-gate-capability-ledger.md` Phase 2 /
//! Task 2.3）。
//!
//! vuln_triage 阶段的 coverage crediting 必须由**确定性 handler**（Rust 代码）从
//! 工具真实输出推导「本次覆盖了哪个 WSTG 类」，绝不让模型自报（护栏 1）。这张纯函数
//! 映射表是那条推导链的第一环：把 Nuclei 给出的 tag / classification 归一成
//! `technique_taxonomy.json` 里由 Nuclei 负责的 8 个通用类与 n-day 类。匿名访问
//! (`WSTG-ATHN-04`) 由独立包装器落库，不允许伪装成 Nuclei tag。
//!
//! 只收录**无歧义**映射；未知 tag → `None`（fail-closed，不 upsert，保持
//! `not_attempted`，绝不凭一个陌生 tag 谎报覆盖）。返回值必须与
//! `resources/harness/technique_taxonomy.json` + `vuln_triage/spec.json`
//! `expected_techniques` 完全对齐，否则 gate join 漂移。

/// vuln_triage 公式化扫描 10 类 technique id（对齐 `vuln_triage/spec.json`
/// `expected_techniques` + `technique_taxonomy.json`）。
pub const WSTG_SQLI: &str = "WSTG-INPV-05";
pub const WSTG_XSS: &str = "WSTG-INPV-01";
pub const WSTG_CMD_INJECTION: &str = "WSTG-INPV-12";
/// Legacy Candidate/Verification taxonomy id. It is deliberately absent from
/// the formulaic VulnTriage denominator because real IDOR/BOLA needs role and
/// object comparison rather than a tag-driven scanner result.
pub const WSTG_IDOR: &str = "WSTG-ATHZ-04";
pub const WSTG_ANONYMOUS_ACCESS: &str = "WSTG-ATHN-04";
pub const WSTG_WEAK_CREDS: &str = "WSTG-ATHN-02";
pub const WSTG_SESSION_CSRF: &str = "WSTG-SESS-02";
pub const WSTG_EXPOSURE_CONFIG: &str = "WSTG-CONF-05";
pub const WSTG_TLS: &str = "WSTG-CRYP-03";
pub const WSTG_INFO_DISCLOSURE: &str = "WSTG-INFO";
pub const GOLISH_NDAY: &str = "GOLISH-NDAY";

/// Complete formulaic vuln-triage taxonomy and the general-Nuclei subset.
/// Scanner adapters import these constants instead of carrying private copies.
pub const FORMULAIC_TECHNIQUES: &[&str] = &[
    WSTG_SQLI,
    WSTG_XSS,
    WSTG_CMD_INJECTION,
    WSTG_ANONYMOUS_ACCESS,
    WSTG_WEAK_CREDS,
    WSTG_SESSION_CSRF,
    WSTG_EXPOSURE_CONFIG,
    WSTG_TLS,
    WSTG_INFO_DISCLOSURE,
    GOLISH_NDAY,
];

pub const GENERAL_NUCLEI_TECHNIQUES: &[&str] = &[
    WSTG_SQLI,
    WSTG_XSS,
    WSTG_CMD_INJECTION,
    WSTG_WEAK_CREDS,
    WSTG_SESSION_CSRF,
    WSTG_EXPOSURE_CONFIG,
    WSTG_TLS,
    WSTG_INFO_DISCLOSURE,
];

/// 把一个扫描器 tag / classification 归一成注册的 vuln_triage WSTG technique id。
///
/// - 大小写不敏感（内部 `to_ascii_lowercase`）。
/// - 只映射无歧义 tag；未知 → `None`（fail-closed：不算覆盖、不 upsert）。
/// - 返回的是 `&'static str`（注册 id），调用方据此 upsert `technique_outcomes`。
pub fn wstg_technique_for_tag(tag: &str) -> Option<&'static str> {
    match tag.trim().to_ascii_lowercase().as_str() {
        "sqli" | "sql-injection" => Some(WSTG_SQLI),
        "xss" => Some(WSTG_XSS),
        "rce" | "cmd-injection" | "command-injection" => Some(WSTG_CMD_INJECTION),
        "default-login" | "weak-password" | "brute" => Some(WSTG_WEAK_CREDS),
        "csrf" | "session" => Some(WSTG_SESSION_CSRF),
        "exposure" | "config" | "misconfig" => Some(WSTG_EXPOSURE_CONFIG),
        "ssl" | "tls" => Some(WSTG_TLS),
        "disclosure" | "info-leak" | "info" => Some(WSTG_INFO_DISCLOSURE),
        "cve" | "nday" => Some(GOLISH_NDAY),
        _ => None,
    }
}

/// Canonical Nuclei tag selected by the server for one registered technique.
/// This is deliberately one-to-one; alias handling lives only in the parser
/// direction above, while command construction always emits one stable tag.
pub fn nuclei_tag_for_technique(technique: &str) -> Option<&'static str> {
    match technique {
        WSTG_SQLI => Some("sqli"),
        WSTG_XSS => Some("xss"),
        WSTG_CMD_INJECTION => Some("rce"),
        WSTG_WEAK_CREDS => Some("default-login"),
        WSTG_SESSION_CSRF => Some("csrf"),
        WSTG_EXPOSURE_CONFIG => Some("exposure"),
        WSTG_TLS => Some("ssl"),
        WSTG_INFO_DISCLOSURE => Some("disclosure"),
        GOLISH_NDAY => Some("cve"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_registered_tag() {
        assert_eq!(wstg_technique_for_tag("sqli"), Some(WSTG_SQLI));
        assert_eq!(wstg_technique_for_tag("sql-injection"), Some(WSTG_SQLI));
        assert_eq!(wstg_technique_for_tag("xss"), Some(WSTG_XSS));
        assert_eq!(wstg_technique_for_tag("rce"), Some(WSTG_CMD_INJECTION));
        assert_eq!(
            wstg_technique_for_tag("cmd-injection"),
            Some(WSTG_CMD_INJECTION)
        );
        assert_eq!(
            wstg_technique_for_tag("command-injection"),
            Some(WSTG_CMD_INJECTION)
        );
        assert_eq!(
            wstg_technique_for_tag("default-login"),
            Some(WSTG_WEAK_CREDS)
        );
        assert_eq!(
            wstg_technique_for_tag("weak-password"),
            Some(WSTG_WEAK_CREDS)
        );
        assert_eq!(wstg_technique_for_tag("brute"), Some(WSTG_WEAK_CREDS));
        assert_eq!(wstg_technique_for_tag("csrf"), Some(WSTG_SESSION_CSRF));
        assert_eq!(wstg_technique_for_tag("session"), Some(WSTG_SESSION_CSRF));
        assert_eq!(
            wstg_technique_for_tag("exposure"),
            Some(WSTG_EXPOSURE_CONFIG)
        );
        assert_eq!(wstg_technique_for_tag("config"), Some(WSTG_EXPOSURE_CONFIG));
        assert_eq!(
            wstg_technique_for_tag("misconfig"),
            Some(WSTG_EXPOSURE_CONFIG)
        );
        assert_eq!(wstg_technique_for_tag("ssl"), Some(WSTG_TLS));
        assert_eq!(wstg_technique_for_tag("tls"), Some(WSTG_TLS));
        assert_eq!(
            wstg_technique_for_tag("disclosure"),
            Some(WSTG_INFO_DISCLOSURE)
        );
        assert_eq!(
            wstg_technique_for_tag("info-leak"),
            Some(WSTG_INFO_DISCLOSURE)
        );
        assert_eq!(wstg_technique_for_tag("info"), Some(WSTG_INFO_DISCLOSURE));
        assert_eq!(wstg_technique_for_tag("cve"), Some(GOLISH_NDAY));
        assert_eq!(wstg_technique_for_tag("nday"), Some(GOLISH_NDAY));
    }

    #[test]
    fn is_case_insensitive_and_trims() {
        assert_eq!(wstg_technique_for_tag("SQLi"), Some(WSTG_SQLI));
        assert_eq!(wstg_technique_for_tag("  XSS  "), Some(WSTG_XSS));
        assert_eq!(wstg_technique_for_tag("CVE"), Some(GOLISH_NDAY));
        assert_eq!(
            wstg_technique_for_tag("Misconfig"),
            Some(WSTG_EXPOSURE_CONFIG)
        );
    }

    #[test]
    fn unknown_tag_is_fail_closed_none() {
        // fail-closed：陌生 tag 绝不算覆盖（否则会谎报 not_attempted → found/empty）。
        assert_eq!(wstg_technique_for_tag("network"), None);
        assert_eq!(wstg_technique_for_tag("tech"), None);
        assert_eq!(wstg_technique_for_tag("wordpress"), None);
        assert_eq!(wstg_technique_for_tag("ssti"), None); // 归 attack_candidate，非本阶段
        assert_eq!(wstg_technique_for_tag("ssrf"), None); // 归 attack_candidate
        assert_eq!(wstg_technique_for_tag("lfi"), None); // 归 attack_candidate
        assert_eq!(wstg_technique_for_tag(""), None);
        assert_eq!(wstg_technique_for_tag("   "), None);
    }

    #[test]
    fn canonical_nuclei_inverse_covers_the_taxonomy_without_alias_drift() {
        assert_eq!(FORMULAIC_TECHNIQUES.len(), 10);
        assert!(FORMULAIC_TECHNIQUES.contains(&"WSTG-ATHN-04"));
        assert!(!FORMULAIC_TECHNIQUES.contains(&WSTG_IDOR));
        assert_eq!(GENERAL_NUCLEI_TECHNIQUES.len(), 8);
        assert!(!GENERAL_NUCLEI_TECHNIQUES.contains(&"WSTG-ATHN-04"));
        assert!(!GENERAL_NUCLEI_TECHNIQUES.contains(&WSTG_IDOR));
        assert!(!GENERAL_NUCLEI_TECHNIQUES.contains(&GOLISH_NDAY));
        for technique in GENERAL_NUCLEI_TECHNIQUES
            .iter()
            .copied()
            .chain(std::iter::once(GOLISH_NDAY))
        {
            let tag = nuclei_tag_for_technique(technique).expect("canonical Nuclei tag");
            assert_eq!(wstg_technique_for_tag(tag), Some(technique));
        }
        assert_eq!(nuclei_tag_for_technique("WSTG-ATHN-04"), None);
        assert_eq!(nuclei_tag_for_technique("WSTG-UNKNOWN"), None);
    }

    #[test]
    fn every_mapped_id_is_in_vuln_triage_expected_set() {
        // 守卫：映射产出的每个 id 必须落在 spec.json vuln_triage expected_techniques
        // 的 10 类内（与 technique_taxonomy.json 对齐），否则 gate 投影对不上。
        let expected: std::collections::HashSet<&'static str> = [
            "WSTG-INPV-05",
            "WSTG-INPV-01",
            "WSTG-INPV-12",
            "WSTG-ATHN-04",
            "WSTG-ATHN-02",
            "WSTG-SESS-02",
            "WSTG-CONF-05",
            "WSTG-CRYP-03",
            "WSTG-INFO",
            "GOLISH-NDAY",
        ]
        .into_iter()
        .collect();
        for tag in [
            "sqli",
            "xss",
            "rce",
            "default-login",
            "csrf",
            "exposure",
            "ssl",
            "disclosure",
            "cve",
        ] {
            let id = wstg_technique_for_tag(tag).expect("known tag maps");
            assert!(
                expected.contains(id),
                "{id} must be a vuln_triage expected technique"
            );
        }
        assert_eq!(wstg_technique_for_tag("idor"), None);
        assert_eq!(wstg_technique_for_tag("bola"), None);
    }
}
