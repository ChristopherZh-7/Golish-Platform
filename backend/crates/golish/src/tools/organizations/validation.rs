//! Server-side validation for the organization profile patch: domain /
//! CIDR / ASN syntax checks and the `validate_profile_patch` aggregator.

use regex::Regex;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::OnceLock;

use super::types::OrganizationProfilePatch;

// 注：tier 是 free-form text（前端 UI 限定 critical/high/medium/low，但 AI
// 注入时可能写别的；故后端不强卡，仅在 UI 校验）。
const ALLOWED_TIERS: &[&str] = &["", "critical", "high", "medium", "low"];

fn domain_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // RFC1035 简化版：label 由字母数字 hyphen 组成，首尾非 hyphen；
    // 顶级域至少 2 字符；允许 `*.` 通配前缀（domain wildcard）。
    R.get_or_init(|| {
        Regex::new(r"^(\*\.)?([a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z]{2,}$")
            .expect("static regex must compile")
    })
}

fn asn_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^AS\d{1,10}$").expect("static regex must compile"))
}

/// Verify CIDR notation. Accepts `IP/PREFIX` where IP is IPv4 / IPv6 and
/// PREFIX is in `0..=32` (v4) or `0..=128` (v6).
pub(super) fn is_valid_cidr(s: &str) -> bool {
    let mut parts = s.splitn(2, '/');
    let ip_part = match parts.next() {
        Some(p) if !p.is_empty() => p,
        _ => return false,
    };
    let prefix_part = match parts.next() {
        Some(p) => p,
        None => return false,
    };
    let prefix: u8 = match prefix_part.parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    match IpAddr::from_str(ip_part) {
        Ok(IpAddr::V4(_)) => prefix <= 32,
        Ok(IpAddr::V6(_)) => prefix <= 128,
        Err(_) => false,
    }
}

pub(super) fn is_valid_domain(s: &str) -> bool {
    !s.is_empty() && s.len() <= 253 && domain_regex().is_match(s)
}

pub(super) fn is_valid_asn(s: &str) -> bool {
    asn_regex().is_match(s)
}

fn iter_strings(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Returns a list of `(field, value, reason)` tuples describing every
/// invalid entry encountered in the patch. Empty Vec = OK.
pub(super) fn validate_profile_patch(
    p: &OrganizationProfilePatch,
) -> Vec<(String, String, String)> {
    let mut errs = Vec::new();

    if let Some(tier) = &p.tier {
        if !ALLOWED_TIERS.contains(&tier.as_str()) {
            errs.push((
                "tier".into(),
                tier.clone(),
                "expected one of: critical|high|medium|low (or empty)".into(),
            ));
        }
    }

    if let Some(domains) = &p.domains {
        if !domains.is_array() {
            errs.push((
                "domains".into(),
                domains.to_string(),
                "expected JSON array".into(),
            ));
        } else {
            for entry in domains.as_array().unwrap() {
                // 允许 {domain,wildcard,note} 或纯字符串
                let s = match entry {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Object(map) => {
                        match map.get("domain").and_then(|v| v.as_str()) {
                            Some(d) => d.to_string(),
                            None => {
                                errs.push((
                                    "domains".into(),
                                    entry.to_string(),
                                    "object missing required string field `domain`".into(),
                                ));
                                continue;
                            }
                        }
                    }
                    _ => {
                        errs.push((
                            "domains".into(),
                            entry.to_string(),
                            "expected string or object".into(),
                        ));
                        continue;
                    }
                };
                if !is_valid_domain(&s) {
                    errs.push(("domains".into(), s, "invalid domain syntax".into()));
                }
            }
        }
    }

    if let Some(ip_ranges) = &p.ip_ranges {
        if !ip_ranges.is_array() {
            errs.push((
                "ip_ranges".into(),
                ip_ranges.to_string(),
                "expected JSON array of CIDR strings".into(),
            ));
        } else {
            for v in iter_strings(ip_ranges) {
                if !is_valid_cidr(&v) {
                    errs.push(("ip_ranges".into(), v, "invalid CIDR".into()));
                }
            }
        }
    }

    if let Some(asns) = &p.asns {
        if !asns.is_array() {
            errs.push((
                "asns".into(),
                asns.to_string(),
                "expected JSON array of ASxxx strings".into(),
            ));
        } else {
            for v in iter_strings(asns) {
                if !is_valid_asn(&v) {
                    errs.push((
                        "asns".into(),
                        v,
                        "invalid ASN (expected `AS<digits>`)".into(),
                    ));
                }
            }
        }
    }

    if let Some(emails) = &p.email_domains {
        if !emails.is_array() {
            errs.push((
                "email_domains".into(),
                emails.to_string(),
                "expected JSON array of domain strings".into(),
            ));
        } else {
            for v in iter_strings(emails) {
                if !is_valid_domain(&v) {
                    errs.push(("email_domains".into(), v, "invalid domain".into()));
                }
            }
        }
    }

    errs
}
