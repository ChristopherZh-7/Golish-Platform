//! surface_coverage_check (2026-06-01 re-anchor · design doc
//! `docs/design/2026-06-01-harness-rebuild.md` §D2).
//!
//! 把 stage "done" 判定锚到 Target Surface Workbench:
//!   - 硬要求 D2_REQUIRED_CATEGORIES (Surface + JsApi) 未覆盖 -> Block
//!   - 软要求 Sitemap 未覆盖且未在 skipped_checks 显式声明 -> 仅 hint (不 block)
//!
//! 空 deliverable 由 vacuous_check 先拦; 本 check 只在有内容时判覆盖度.

use super::super::surface_mapping::{
    missing_required_categories, SurfaceCategory, SurfaceCoverage,
};
use super::super::types::{ExternalAttackSurfaceDeliverable, HarnessRecoveryActions};
use super::GateCheckOutcome;

pub fn run(deliverable: &ExternalAttackSurfaceDeliverable) -> GateCheckOutcome {
    // 空 deliverable 留给 vacuous_check; 这里不重复拦.
    if deliverable.claims.is_empty() && deliverable.findings.is_empty() {
        return GateCheckOutcome::Pass;
    }

    let missing = missing_required_categories(deliverable);
    let sitemap_skipped = deliverable
        .skipped_checks
        .iter()
        .any(|s| s.check.to_lowercase().contains("sitemap"));
    let cov = SurfaceCoverage::from_deliverable(deliverable);
    let sitemap_soft_gap = !cov.covers(SurfaceCategory::Sitemap) && !sitemap_skipped;

    if missing.is_empty() {
        if sitemap_soft_gap {
            tracing::info!(
                target: "harness::gate::surface_coverage_check",
                stage_id = %deliverable.stage_id,
                "surface_coverage pass (sitemap soft-gap noted)"
            );
        }
        return GateCheckOutcome::Pass;
    }

    let reasons: Vec<String> = missing
        .iter()
        .map(|c| {
            format!(
                "surface coverage gap: required Surface Workbench category {:?} has no evidence-backed claim/finding",
                c
            )
        })
        .collect();

    let mut recovery = HarnessRecoveryActions::default();
    for c in &missing {
        match c {
            SurfaceCategory::Surface => {
                recovery.hints.push(
                    "run http_probe / fingerprint_target to produce Surface (ports/services/fingerprints) evidence".to_string(),
                );
                recovery.repair_tool_calls.push("http_probe".to_string());
                recovery
                    .missing_evidence_kinds
                    .push("fingerprint".to_string());
            }
            SurfaceCategory::JsApi => {
                recovery.hints.push(
                    "collect JS + extract API endpoints to produce JS/API evidence".to_string(),
                );
                recovery
                    .repair_tool_calls
                    .push("query_target_data".to_string());
                recovery
                    .missing_evidence_kinds
                    .push("api_endpoint".to_string());
            }
            _ => {}
        }
    }
    if sitemap_soft_gap {
        recovery.hints.push(
            "Sitemap tab empty: either crawl for sitemap evidence OR add an explicit skipped_checks entry (checked-empty != unchecked, AGENTS.md I8)".to_string(),
        );
    }

    tracing::warn!(
        target: "harness::gate::surface_coverage_check",
        stage_id = %deliverable.stage_id,
        missing = ?missing,
        "surface_coverage block"
    );
    GateCheckOutcome::Block { reasons, recovery }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::types::{FindingSeverity, HarnessFinding, SkippedCheckRecord};
    use golish_pentest::evidence_ledger::{EvidenceAuditId, SkipReason};
    use uuid::Uuid;

    fn finding(kind: &str) -> HarnessFinding {
        HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: kind.to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![EvidenceAuditId::new(1)],
        }
    }

    fn deliverable(findings: Vec<HarnessFinding>) -> ExternalAttackSurfaceDeliverable {
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
    fn empty_deliverable_passes_here() {
        let d = deliverable(vec![]);
        assert!(matches!(run(&d), GateCheckOutcome::Pass));
    }

    #[test]
    fn surface_plus_jsapi_passes() {
        let d = deliverable(vec![finding("http_service"), finding("api_endpoint")]);
        assert!(matches!(run(&d), GateCheckOutcome::Pass));
    }

    #[test]
    fn only_surface_passes_after_jsapi_moved_to_enumeration() {
        // 2026-06-09 阶段重排：EAS 只硬要求 Surface（JsApi 移交 enumeration 的
        // coverage_complete(GOLISH-ENUM-JSAPI)），故只有 Surface 也通过。
        let d = deliverable(vec![finding("http_service")]);
        assert!(matches!(run(&d), GateCheckOutcome::Pass));
    }

    #[test]
    fn sitemap_explicit_skip_does_not_block() {
        let mut d = deliverable(vec![finding("http_service"), finding("api_endpoint")]);
        d.skipped_checks.push(SkippedCheckRecord {
            check: "sitemap_crawl".to_string(),
            reason: SkipReason::Other {
                explanation: "no robots.txt / sitemap.xml present".to_string(),
                evidence_ref: EvidenceAuditId::new(1),
            },
        });
        assert!(matches!(run(&d), GateCheckOutcome::Pass));
    }
}
