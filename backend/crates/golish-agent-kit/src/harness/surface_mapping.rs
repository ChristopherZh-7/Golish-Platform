//! Surface Workbench category 映射 (2026-06-01 re-anchor).
//!
//! 把 external_attack_surface stage 的 "done" 判定 (design doc
//! `docs/design/2026-06-01-harness-rebuild.md` §D2) 重新锚定到 pivot 后的
//! Target Surface Workbench 数据模型
//! (`docs/design/2026-05-28-target-surface-workbench.md`): deliverable 的
//! claim/finding kind 关键词映射到 Surface Workbench tab, gate 据此推理覆盖度,
//! 而非已删除的 SecurityView 模型.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::types::ExternalAttackSurfaceDeliverable;

/// Target Surface Workbench tab 分类 (external_attack_surface stage 可产证据的子集).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceCategory {
    /// `Identity` tab: host/IP/DNS/ASN/CDN 解析.
    Identity,
    /// `Surface` tab: 端口/服务/HTTP 探测/指纹.
    Surface,
    /// `Sitemap` tab: robots/sitemap.xml/爬虫路径.
    Sitemap,
    /// `JS / API` tab: JS 文件, source map, 抽取的 API 端点.
    JsApi,
    /// `Sensitive` tab: 密钥/泄漏暴露.
    Sensitive,
}

impl SurfaceCategory {
    /// 把 claim/finding `kind` 字符串 (小写关键词匹配) 映射到 Surface Workbench
    /// category. 落在 surface 分类法之外的 kind 返回 None.
    ///
    /// 顺序敏感: 更专的 category (Sitemap/JsApi/Sensitive) 先判, 避免 generic
    /// 关键词误吞.
    pub fn from_kind(kind: &str) -> Option<Self> {
        let k = kind.to_lowercase();
        if contains_any(
            &k,
            &["sitemap", "robots", "crawl", "site_path", "path_discovery"],
        ) {
            return Some(Self::Sitemap);
        }
        if contains_any(
            &k,
            &[
                "js_",
                "javascript",
                "api_endpoint",
                "endpoint",
                "api_route",
                "source_map",
                "sourcemap",
            ],
        ) {
            return Some(Self::JsApi);
        }
        if contains_any(
            &k,
            &[
                "secret",
                "leak",
                "sensitive",
                "exposure",
                "credential",
                "token_exposure",
            ],
        ) {
            return Some(Self::Sensitive);
        }
        if contains_any(
            &k,
            &[
                "port",
                "service",
                "http",
                "fingerprint",
                "tech_stack",
                "banner",
                "tls",
                "subdomain",
            ],
        ) {
            return Some(Self::Surface);
        }
        if contains_any(
            &k,
            &["dns", "asn", "cdn", "whois", "ip_resolution", "identity"],
        ) {
            return Some(Self::Identity);
        }
        None
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// deliverable 的 claim+finding 触及了哪些 Surface Workbench category.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SurfaceCoverage {
    pub categories: BTreeSet<SurfaceCategory>,
}

impl SurfaceCoverage {
    pub fn from_deliverable(d: &ExternalAttackSurfaceDeliverable) -> Self {
        let mut categories = BTreeSet::new();
        for c in &d.claims {
            if let Some(cat) = SurfaceCategory::from_kind(&c.kind) {
                categories.insert(cat);
            }
        }
        for f in &d.findings {
            if let Some(cat) = SurfaceCategory::from_kind(&f.kind) {
                categories.insert(cat);
            }
        }
        Self { categories }
    }

    pub fn covers(&self, cat: SurfaceCategory) -> bool {
        self.categories.contains(&cat)
    }
}

/// D2 硬要求 category（2026-06-09 阶段重排：端口/服务前移到 EAS、JS/API 移交
/// enumeration，详见 `docs/design/2026-06-09-active-stage-verify-first.md`）：
/// external_attack_surface 现在只负责「定义攻击面」(Surface = 端口/服务/HTTP/指纹)，
/// 故 EAS `surface_coverage` 只硬要求 **Surface**。JsApi 的把关由 enumeration 的
/// `coverage_complete`(GOLISH-ENUM-JSAPI) 承担，不再压在 EAS。
pub const D2_REQUIRED_CATEGORIES: &[SurfaceCategory] = &[SurfaceCategory::Surface];

/// D2 软要求 category (属 D2 意图但当前无保证后端数据源, 允许 honest-empty).
pub const D2_SOFT_CATEGORIES: &[SurfaceCategory] = &[SurfaceCategory::Sitemap];

/// 返回 deliverable 未覆盖的硬要求 category 列表 (空 = 满足 D2 硬门槛).
pub fn missing_required_categories(d: &ExternalAttackSurfaceDeliverable) -> Vec<SurfaceCategory> {
    let cov = SurfaceCoverage::from_deliverable(d);
    D2_REQUIRED_CATEGORIES
        .iter()
        .copied()
        .filter(|c| !cov.covers(*c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{missing_required_categories, SurfaceCategory, SurfaceCoverage};
    use crate::harness::types::{
        ExternalAttackSurfaceDeliverable, FindingSeverity, HarnessFinding, StageClaim,
    };
    use golish_pentest::evidence_ledger::EvidenceAuditId;
    use uuid::Uuid;

    #[test]
    fn from_kind_maps_known_kinds() {
        assert_eq!(
            SurfaceCategory::from_kind("http_service"),
            Some(SurfaceCategory::Surface)
        );
        assert_eq!(
            SurfaceCategory::from_kind("subdomain"),
            Some(SurfaceCategory::Surface)
        );
        assert_eq!(
            SurfaceCategory::from_kind("fingerprint"),
            Some(SurfaceCategory::Surface)
        );
        assert_eq!(
            SurfaceCategory::from_kind("api_endpoint"),
            Some(SurfaceCategory::JsApi)
        );
        assert_eq!(
            SurfaceCategory::from_kind("js_secret"),
            Some(SurfaceCategory::JsApi)
        );
        assert_eq!(
            SurfaceCategory::from_kind("sitemap_path"),
            Some(SurfaceCategory::Sitemap)
        );
        assert_eq!(
            SurfaceCategory::from_kind("sensitive_exposure"),
            Some(SurfaceCategory::Sensitive)
        );
        assert_eq!(
            SurfaceCategory::from_kind("dns_a"),
            Some(SurfaceCategory::Identity)
        );
    }

    #[test]
    fn from_kind_unknown_returns_none() {
        assert_eq!(SurfaceCategory::from_kind("billing_refactor"), None);
        assert_eq!(SurfaceCategory::from_kind(""), None);
    }

    fn finding(kind: &str) -> HarnessFinding {
        HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: kind.to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![EvidenceAuditId::new(1)],
            technique: None,
        }
    }

    fn deliverable_with(findings: Vec<HarnessFinding>) -> ExternalAttackSurfaceDeliverable {
        ExternalAttackSurfaceDeliverable {
            stage_id: "external_attack_surface".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![],
            evidence_refs: vec![EvidenceAuditId::new(1)],
            skipped_checks: vec![],
            findings,
            required_checks_done: vec![],
            coverage: vec![],
        }
    }

    #[test]
    fn coverage_collects_distinct_categories() {
        let d = deliverable_with(vec![finding("http_service"), finding("api_endpoint")]);
        let cov = SurfaceCoverage::from_deliverable(&d);
        assert!(cov.covers(SurfaceCategory::Surface));
        assert!(cov.covers(SurfaceCategory::JsApi));
        assert!(!cov.covers(SurfaceCategory::Sitemap));
    }

    #[test]
    fn only_surface_satisfies_required_after_jsapi_moved_to_enumeration() {
        // 2026-06-09 阶段重排：EAS 只硬要求 Surface；JsApi 移交 enumeration。
        let d = deliverable_with(vec![finding("http_service")]);
        assert!(missing_required_categories(&d).is_empty());
    }

    #[test]
    fn no_missing_when_surface_and_jsapi_present() {
        let mut d = deliverable_with(vec![finding("http_service")]);
        d.claims.push(StageClaim {
            kind: "api_endpoint_observed".to_string(),
            subject: "api.example.com/v1".to_string(),
            summary: "GET 200".to_string(),
            evidence_ids: vec![EvidenceAuditId::new(1)],
            technique: None,
        });
        assert!(missing_required_categories(&d).is_empty());
    }
}
