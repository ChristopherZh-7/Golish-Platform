//! Organizations Tauri commands.
//!
//! 组织 = 甲方资产情报库。§S3 起为多级树形结构（`parent_id` 自引用），
//! 2026-05-17 的 profile 升级（migration `_organizations_profile_fields`）
//! 在原 8 个基础字段之外加了 18 个 profile 字段，把组织从「一个名字」
//! 升级为 HVV 攻击方需要的资产情报库。
//!
//! 字段分组（与前端 5-tab UI 对应）：
//!   基础 tab : aliases / industry / tier / credit_code
//!   域名 tab : domains
//!   网络 tab : ip_ranges / asns / email_domains
//!   范围 tab : scope_rules
//!   其他 tab : intel / notes
//!   二期    : certificates / subsidiaries / business_systems / cloud_assets
//!            / github_orgs / social_accounts / historical_vulns / contacts
//!
//! 注意：`grp` 字段在 §S1 的字符串分级仍兼容保留作为回退；新建 target 可以
//! 直接关联 `organization_id`。

use crate::error::GolishError;
use crate::state::DbState;
use golish_db::repo::organizations::ProfilePatch;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::OnceLock;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: String,
    pub project_path: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub description: String,
    pub owner: String,
    pub sort_order: i32,
    // profile 字段（与 5-tab UI 对应）
    pub aliases: Vec<String>,
    pub industry: String,
    pub tier: String,
    pub credit_code: String,
    pub domains: serde_json::Value,
    pub ip_ranges: serde_json::Value,
    pub asns: serde_json::Value,
    pub email_domains: serde_json::Value,
    pub scope_rules: serde_json::Value,
    pub intel: serde_json::Value,
    pub notes: String,
    // 二期字段（schema 已就位，UI 后续 PR）
    pub certificates: serde_json::Value,
    pub subsidiaries: serde_json::Value,
    pub business_systems: serde_json::Value,
    pub cloud_assets: serde_json::Value,
    pub github_orgs: serde_json::Value,
    pub social_accounts: serde_json::Value,
    pub historical_vulns: serde_json::Value,
    pub contacts: serde_json::Value,
    pub created_at: u64,
    pub updated_at: u64,
}

fn to_org(o: golish_db::models::Organization) -> Organization {
    Organization {
        id: o.id.to_string(),
        project_path: o.project_path,
        name: o.name,
        parent_id: o.parent_id.map(|u| u.to_string()),
        description: o.description,
        owner: o.owner,
        sort_order: o.sort_order,
        aliases: o.aliases,
        industry: o.industry,
        tier: o.tier,
        credit_code: o.credit_code,
        domains: o.domains,
        ip_ranges: o.ip_ranges,
        asns: o.asns,
        email_domains: o.email_domains,
        scope_rules: o.scope_rules,
        intel: o.intel,
        notes: o.notes,
        certificates: o.certificates,
        subsidiaries: o.subsidiaries,
        business_systems: o.business_systems,
        cloud_assets: o.cloud_assets,
        github_orgs: o.github_orgs,
        social_accounts: o.social_accounts,
        historical_vulns: o.historical_vulns,
        contacts: o.contacts,
        created_at: o.created_at.timestamp() as u64,
        updated_at: o.updated_at.timestamp() as u64,
    }
}

#[tauri::command]
pub async fn organization_list(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<Vec<Organization>, GolishError> {
    let pool = state.pool_ready().await?;
    let pp = project_path.as_deref().unwrap_or("");
    let rows = golish_db::repo::organizations::list(pool, pp).await?;
    Ok(rows.into_iter().map(to_org).collect())
}

#[tauri::command]
pub async fn organization_get(
    state: tauri::State<'_, DbState>,
    id: String,
) -> Result<Organization, GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id.parse()?;
    let row = golish_db::repo::organizations::get_one(pool, uid)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {id}")))?;
    Ok(to_org(row))
}

#[tauri::command]
pub async fn organization_create(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
    name: String,
    parent_id: Option<String>,
    description: Option<String>,
    owner: Option<String>,
) -> Result<Organization, GolishError> {
    let pool = state.pool_ready().await?;
    let pp = project_path.as_deref().unwrap_or("");
    let pid: Option<Uuid> = parent_id.and_then(|s| s.parse().ok());
    let row = golish_db::repo::organizations::create(
        pool,
        pp,
        name.trim(),
        pid,
        description.as_deref().unwrap_or(""),
        owner.as_deref().unwrap_or(""),
    )
    .await?;
    Ok(to_org(row))
}

#[tauri::command]
pub async fn organization_update(
    state: tauri::State<'_, DbState>,
    id: String,
    name: Option<String>,
    description: Option<String>,
    owner: Option<String>,
    sort_order: Option<i32>,
) -> Result<Organization, GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id
        .parse()
        .map_err(|e: uuid::Error| GolishError::from(e.to_string()))?;
    let row = golish_db::repo::organizations::update(
        pool,
        uid,
        name.as_deref(),
        description.as_deref(),
        owner.as_deref(),
        sort_order,
    )
    .await?;
    Ok(to_org(row))
}

#[tauri::command]
pub async fn organization_move(
    state: tauri::State<'_, DbState>,
    id: String,
    new_parent_id: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id
        .parse()
        .map_err(|e: uuid::Error| GolishError::from(e.to_string()))?;
    let new_parent: Option<Uuid> = new_parent_id.and_then(|s| s.parse().ok());
    golish_db::repo::organizations::move_to(pool, uid, new_parent).await?;
    Ok(())
}

#[tauri::command]
pub async fn organization_delete(
    state: tauri::State<'_, DbState>,
    id: String,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id
        .parse()
        .map_err(|e: uuid::Error| GolishError::from(e.to_string()))?;
    golish_db::repo::organizations::delete(pool, uid).await?;
    Ok(())
}

// ── profile patch ────────────────────────────────────────────────────────────

/// 前端 PATCH 入参；每个字段 `Option` 表示「不传 = 不修改」。
///
/// 校验在 `validate_profile_patch` 一遍走完，发现任一字段不合法立刻 400，
/// 不写库——避免一半字段进去、一半 reject 的半成品状态。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationProfilePatch {
    pub aliases: Option<Vec<String>>,
    pub industry: Option<String>,
    pub tier: Option<String>,
    pub credit_code: Option<String>,
    pub domains: Option<serde_json::Value>,
    pub ip_ranges: Option<serde_json::Value>,
    pub asns: Option<serde_json::Value>,
    pub email_domains: Option<serde_json::Value>,
    pub scope_rules: Option<serde_json::Value>,
    pub intel: Option<serde_json::Value>,
    pub notes: Option<String>,
    pub certificates: Option<serde_json::Value>,
    pub subsidiaries: Option<serde_json::Value>,
    pub business_systems: Option<serde_json::Value>,
    pub cloud_assets: Option<serde_json::Value>,
    pub github_orgs: Option<serde_json::Value>,
    pub social_accounts: Option<serde_json::Value>,
    pub historical_vulns: Option<serde_json::Value>,
    pub contacts: Option<serde_json::Value>,
}

impl From<OrganizationProfilePatch> for ProfilePatch {
    fn from(p: OrganizationProfilePatch) -> Self {
        ProfilePatch {
            aliases: p.aliases,
            industry: p.industry,
            tier: p.tier,
            credit_code: p.credit_code,
            domains: p.domains,
            ip_ranges: p.ip_ranges,
            asns: p.asns,
            email_domains: p.email_domains,
            scope_rules: p.scope_rules,
            intel: p.intel,
            notes: p.notes,
            certificates: p.certificates,
            subsidiaries: p.subsidiaries,
            business_systems: p.business_systems,
            cloud_assets: p.cloud_assets,
            github_orgs: p.github_orgs,
            social_accounts: p.social_accounts,
            historical_vulns: p.historical_vulns,
            contacts: p.contacts,
        }
    }
}

// 注：tier 是 free-form text（前端 UI 限定 critical/high/medium/low，但 AI
// 注入时可能写别的；故后端不强卡，仅在 UI 校验）。

const ALLOWED_TIERS: &[&str] = &["", "critical", "high", "medium", "low"];

fn domain_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // RFC1035 简化版：label 由字母数字 hyphen 组成，首尾非 hyphen；
    // 顶级域至少 2 字符；允许 `*.` 通配前缀（domain wildcard）。
    R.get_or_init(|| {
        Regex::new(
            r"^(\*\.)?([a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z]{2,}$",
        )
        .expect("static regex must compile")
    })
}

fn asn_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^AS\d{1,10}$").expect("static regex must compile"))
}

/// Verify CIDR notation. Accepts `IP/PREFIX` where IP is IPv4 / IPv6 and
/// PREFIX is in `0..=32` (v4) or `0..=128` (v6).
fn is_valid_cidr(s: &str) -> bool {
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

fn is_valid_domain(s: &str) -> bool {
    !s.is_empty() && s.len() <= 253 && domain_regex().is_match(s)
}

fn is_valid_asn(s: &str) -> bool {
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
fn validate_profile_patch(p: &OrganizationProfilePatch) -> Vec<(String, String, String)> {
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
                    serde_json::Value::Object(map) => match map.get("domain").and_then(|v| v.as_str()) {
                        Some(d) => d.to_string(),
                        None => {
                            errs.push((
                                "domains".into(),
                                entry.to_string(),
                                "object missing required string field `domain`".into(),
                            ));
                            continue;
                        }
                    },
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

#[tauri::command]
pub async fn organization_update_profile(
    state: tauri::State<'_, DbState>,
    id: String,
    patch: OrganizationProfilePatch,
) -> Result<Organization, GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = id.parse()?;

    let errs = validate_profile_patch(&patch);
    if !errs.is_empty() {
        let summary: String = errs
            .iter()
            .map(|(f, v, r)| format!("{f}=`{v}` → {r}"))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(GolishError::Validation(summary));
    }

    let row = golish_db::repo::organizations::update_profile(pool, uid, &patch.into())
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {id}")))?;
    Ok(to_org(row))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_validation_accepts_ipv4_and_ipv6() {
        assert!(is_valid_cidr("10.0.0.0/8"));
        assert!(is_valid_cidr("192.168.1.0/24"));
        assert!(is_valid_cidr("0.0.0.0/0"));
        assert!(is_valid_cidr("2001:db8::/32"));
        assert!(is_valid_cidr("::/0"));
    }

    #[test]
    fn cidr_validation_rejects_garbage() {
        assert!(!is_valid_cidr(""));
        assert!(!is_valid_cidr("10.0.0.0"));
        assert!(!is_valid_cidr("10.0.0.0/"));
        assert!(!is_valid_cidr("10.0.0.0/33"));
        assert!(!is_valid_cidr("10.0.0.0/abc"));
        assert!(!is_valid_cidr("not-an-ip/24"));
        assert!(!is_valid_cidr("2001:db8::/129"));
    }

    #[test]
    fn domain_validation_accepts_normal_and_wildcard() {
        assert!(is_valid_domain("example.com"));
        assert!(is_valid_domain("a.b.example.com"));
        assert!(is_valid_domain("*.example.com"));
        assert!(is_valid_domain("xn--80akhbyknj4f.com"));
    }

    #[test]
    fn domain_validation_rejects_garbage() {
        assert!(!is_valid_domain(""));
        assert!(!is_valid_domain("example"));
        assert!(!is_valid_domain(".example.com"));
        assert!(!is_valid_domain("example..com"));
        assert!(!is_valid_domain("-bad.com"));
        assert!(!is_valid_domain("bad-.com"));
    }

    #[test]
    fn asn_validation() {
        assert!(is_valid_asn("AS1"));
        assert!(is_valid_asn("AS12345"));
        assert!(!is_valid_asn(""));
        assert!(!is_valid_asn("12345"));
        assert!(!is_valid_asn("as12345"));
        assert!(!is_valid_asn("AS"));
        assert!(!is_valid_asn("AS12345678901"));
    }

    #[test]
    fn validate_patch_collects_all_errors() {
        let p = OrganizationProfilePatch {
            tier: Some("supreme".into()),
            ip_ranges: Some(serde_json::json!(["10.0.0.0/24", "bad-ip"])),
            asns: Some(serde_json::json!(["AS1", "not-an-asn"])),
            domains: Some(serde_json::json!(["good.com", "bad..com"])),
            email_domains: Some(serde_json::json!(["pingan.com", "x x"])),
            ..Default::default()
        };
        let errs = validate_profile_patch(&p);
        let fields: Vec<&str> = errs.iter().map(|(f, _, _)| f.as_str()).collect();
        assert!(fields.contains(&"tier"));
        assert!(fields.contains(&"ip_ranges"));
        assert!(fields.contains(&"asns"));
        assert!(fields.contains(&"domains"));
        assert!(fields.contains(&"email_domains"));
    }

    #[test]
    fn validate_patch_accepts_clean_payload() {
        let p = OrganizationProfilePatch {
            tier: Some("critical".into()),
            ip_ranges: Some(serde_json::json!(["10.0.0.0/24", "2001:db8::/32"])),
            asns: Some(serde_json::json!(["AS12345"])),
            domains: Some(serde_json::json!(["pingan.com", "*.pingan.com"])),
            email_domains: Some(serde_json::json!(["pingan.com"])),
            ..Default::default()
        };
        assert!(validate_profile_patch(&p).is_empty());
    }
}
