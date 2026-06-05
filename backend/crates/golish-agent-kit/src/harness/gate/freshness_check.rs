//! Doc 3 §8.2 freshness_check · evidence as_of_timestamp + max_age 比较.
//!
//! Phase 1c.5 完整版:
//!   1. Sanity: claim/finding 引用的 evidence_id 必须在 deliverable.evidence_refs
//!   2. Freshness: 调用方可传 `evidence_ages` 映射 (eid → age via Utc::now -
//!      as_of_timestamp); freshness_check 用 `EvidenceKindRegistry` 配 `evidence_kinds`
//!      Doc 1 §6.1 默认 + 7 days fallback 比较.
//!
//! Phase 1 MVP `run()` 单参版仍保留 sanity-only path; `run_with_freshness()`
//! 接受额外 `evidence_kinds + evidence_ages` 启用真 max_age 比较.

use std::collections::HashMap;
use std::time::Duration as StdDuration;

use chrono::Duration;
use golish_pentest::evidence_kinds::EvidenceKindRegistry;
use golish_pentest::evidence_ledger::EvidenceAuditId;

use super::super::stage_spec::StageSpec;
use super::super::types::{ExternalAttackSurfaceDeliverable, HarnessRecoveryActions};
use super::GateCheckOutcome;

pub fn run(deliverable: &ExternalAttackSurfaceDeliverable, _spec: &StageSpec) -> GateCheckOutcome {
    // Phase 1c.2 skeleton: 不做真 freshness 查 (需要 EvidenceLedger). 只做
    // sanity: evidence_refs 数量 vs claims/findings 引用 evidence_ids 一致性.
    let referenced_eids: std::collections::HashSet<_> = deliverable
        .claims
        .iter()
        .flat_map(|c| c.evidence_ids.iter().copied())
        .chain(
            deliverable
                .findings
                .iter()
                .flat_map(|f| f.evidence_refs.iter().copied()),
        )
        .collect();

    let registered_eids: std::collections::HashSet<_> =
        deliverable.evidence_refs.iter().copied().collect();

    let mut reasons = Vec::new();
    for eid in &referenced_eids {
        if !registered_eids.contains(eid) {
            reasons.push(format!(
                "evidence_audit_id={} referenced by claim/finding but not declared in deliverable.evidence_refs",
                eid.as_i64()
            ));
        }
    }

    if reasons.is_empty() {
        tracing::info!(
            target: "harness::gate::freshness_check",
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            referenced_eids = referenced_eids.len(),
            registered_eids = registered_eids.len(),
            outcome = "pass",
            "freshness_check sanity pass"
        );
        GateCheckOutcome::Pass
    } else {
        tracing::info!(
            target: "harness::gate::freshness_check",
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            outcome = "block",
            reasons_count = reasons.len(),
            first_reason = %reasons[0],
            "freshness_check sanity block"
        );
        let mut recovery = HarnessRecoveryActions::default();
        recovery.hints.push(
            "add all claim/finding-referenced evidence ids to deliverable.evidence_refs"
                .to_string(),
        );
        GateCheckOutcome::Block { reasons, recovery }
    }
}

/// 完整版本: 额外接受 `evidence_kinds + evidence_ages` 启用 max_age 比较.
///
/// `evidence_kinds[eid]` 给 evidence kind 字符串 (用于查 EvidenceKindRegistry);
/// `evidence_ages[eid]` 给 evidence 已存在多久 (Utc::now - as_of_timestamp).
/// stage_spec 不提供 override; 用 evidence_kinds.json default + 7 days fallback.
pub fn run_with_freshness(
    deliverable: &ExternalAttackSurfaceDeliverable,
    spec: &StageSpec,
    evidence_kinds: &HashMap<EvidenceAuditId, String>,
    evidence_ages: &HashMap<EvidenceAuditId, StdDuration>,
) -> GateCheckOutcome {
    // 1. 先跑 sanity (eid 引用一致性)
    let sanity = run(deliverable, spec);
    let registry = EvidenceKindRegistry::instance();

    let mut reasons = Vec::new();
    let mut recovery = HarnessRecoveryActions::default();

    if let GateCheckOutcome::Block {
        reasons: r,
        recovery: rec,
    } = sanity
    {
        reasons.extend(r);
        recovery.hints.extend(rec.hints);
        recovery.repair_tool_calls.extend(rec.repair_tool_calls);
        recovery
            .missing_evidence_kinds
            .extend(rec.missing_evidence_kinds);
    }

    // 2. 对 deliverable.evidence_refs 中每条 eid 做 freshness 检查
    for eid in &deliverable.evidence_refs {
        let kind = match evidence_kinds.get(eid) {
            Some(k) => k.as_str(),
            None => continue, // 未提供 kind 信息 → 跳过 (gate 仍能放过)
        };
        let age = match evidence_ages.get(eid).copied() {
            Some(a) => a,
            None => continue,
        };
        let max_age_std = registry.max_age_with_default(kind);
        let max_age = Duration::from_std(max_age_std).unwrap_or_else(|_| Duration::days(7));
        let age_chrono = Duration::from_std(age).unwrap_or_else(|_| Duration::seconds(0));
        // age >= 2 * max → hard expired (block); age >= max → soft stale (block 但软)
        if age_chrono >= max_age * 2 {
            reasons.push(format!(
                "evidence eid={} kind={} hard-expired (age={}s, max={}s)",
                eid.as_i64(),
                kind,
                age.as_secs(),
                max_age_std.as_secs()
            ));
            recovery.missing_evidence_kinds.push(kind.to_string());
            recovery
                .repair_tool_calls
                .push(format!("re-acquire fresh {} evidence", kind));
        } else if age_chrono >= max_age {
            reasons.push(format!(
                "evidence eid={} kind={} stale (age={}s, max={}s)",
                eid.as_i64(),
                kind,
                age.as_secs(),
                max_age_std.as_secs()
            ));
            recovery
                .hints
                .push(format!("consider re-checking stale {} evidence", kind));
        }
    }

    if reasons.is_empty() {
        tracing::info!(
            target: "harness::gate::freshness_check",
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            evidence_refs = deliverable.evidence_refs.len(),
            evidence_kinds_known = evidence_kinds.len(),
            evidence_ages_known = evidence_ages.len(),
            outcome = "pass",
            "freshness_check (with max_age) pass"
        );
        GateCheckOutcome::Pass
    } else {
        tracing::info!(
            target: "harness::gate::freshness_check",
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            evidence_refs = deliverable.evidence_refs.len(),
            outcome = "block",
            reasons_count = reasons.len(),
            first_reason = %reasons[0],
            "freshness_check (with max_age) block"
        );
        GateCheckOutcome::Block { reasons, recovery }
    }
}

/// P0 Task 6 · 纯函数: 给一组 evidence id + 它们的 kind/age, 按 `evidence_kinds.json`
/// 的 max_age 判定「过期」(age ≥ 2×max) 与「陈旧」(max ≤ age < 2×max), 返回
/// `(expired_reasons, stale_reasons)`. 缺 kind 或 age 信息的 id 跳过 (gate 放行)。
///
/// 与 [`run_with_freshness`] 同阈值, 但是一个独立、无 DB 的纯函数, 供
/// `task_orchestrator` 收口阶段查 ledger 拿到真实 age 后做 post-gate 回查
/// (刻意不改 `run_with_freshness`, 避免动到已被单测覆盖的 gate 主路径)。
pub fn freshness_age_reasons(
    ids: &[EvidenceAuditId],
    kinds: &HashMap<EvidenceAuditId, String>,
    ages: &HashMap<EvidenceAuditId, StdDuration>,
) -> (Vec<String>, Vec<String>) {
    let registry = EvidenceKindRegistry::instance();
    let mut expired = Vec::new();
    let mut stale = Vec::new();
    for id in ids {
        let (Some(kind), Some(age)) = (kinds.get(id), ages.get(id).copied()) else {
            continue;
        };
        let max = registry.max_age_with_default(kind).as_secs();
        if max == 0 {
            continue;
        }
        let a = age.as_secs();
        if a >= max.saturating_mul(2) {
            expired.push(format!(
                "evidence id={} kind={} hard-expired (age={}s, max={}s)",
                id.as_i64(),
                kind,
                a,
                max
            ));
        } else if a >= max {
            stale.push(format!(
                "evidence id={} kind={} stale (age={}s, max={}s)",
                id.as_i64(),
                kind,
                a,
                max
            ));
        }
    }
    (expired, stale)
}

#[cfg(test)]
mod tests {
    use super::super::super::stage_spec::load_stage_spec_from_json;
    use super::super::super::types::{
        ExternalAttackSurfaceDeliverable, FindingSeverity, HarnessFinding, StageClaim,
    };
    use super::*;
    use golish_pentest::evidence_ledger::EvidenceAuditId;
    use uuid::Uuid;

    const STAGE_JSON: &str =
        include_str!("../../../../../../resources/harness/stages/external_attack_surface.json");

    fn empty_deliverable() -> ExternalAttackSurfaceDeliverable {
        ExternalAttackSurfaceDeliverable {
            stage_id: "external_attack_surface".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![],
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: vec![],
            coverage: vec![],
        }
    }

    #[test]
    fn empty_deliverable_passes_freshness_sanity() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let d = empty_deliverable();
        assert!(matches!(run(&d, &spec), GateCheckOutcome::Pass));
    }

    #[test]
    fn evidence_refs_complete_passes() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        let eid = EvidenceAuditId::new(7);
        d.evidence_refs = vec![eid];
        d.findings.push(HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![eid],
        });
        assert!(matches!(run(&d, &spec), GateCheckOutcome::Pass));
    }

    #[test]
    fn finding_evidence_not_in_deliverable_blocks() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        // 故意不把 eid=42 加到 deliverable.evidence_refs
        d.evidence_refs = vec![];
        d.findings.push(HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![EvidenceAuditId::new(42)],
        });
        match run(&d, &spec) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("evidence_audit_id=42"));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn claim_evidence_not_in_deliverable_blocks() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        d.evidence_refs = vec![];
        d.claims.push(StageClaim {
            kind: "http_service_observed".to_string(),
            subject: "x.example.com".to_string(),
            summary: "200 OK".to_string(),
            evidence_ids: vec![EvidenceAuditId::new(99)],
        });
        match run(&d, &spec) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("evidence_audit_id=99"));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn run_with_freshness_pass_when_evidence_fresh() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        let eid = EvidenceAuditId::new(1);
        d.evidence_refs = vec![eid];
        d.findings.push(HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![eid],
        });
        let mut kinds = std::collections::HashMap::new();
        kinds.insert(eid, "dns_a".to_string()); // 86400s max
        let mut ages = std::collections::HashMap::new();
        ages.insert(eid, std::time::Duration::from_secs(60)); // 1 min - 极新
        assert!(matches!(
            run_with_freshness(&d, &spec, &kinds, &ages),
            GateCheckOutcome::Pass
        ));
    }

    #[test]
    fn run_with_freshness_stale_blocks_softly() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        let eid = EvidenceAuditId::new(1);
        d.evidence_refs = vec![eid];
        d.findings.push(HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![eid],
        });
        let mut kinds = std::collections::HashMap::new();
        kinds.insert(eid, "http_probe".to_string()); // 21600s max (6h)
        let mut ages = std::collections::HashMap::new();
        // age = 8h (between 6h and 12h) → Stale (block)
        ages.insert(eid, std::time::Duration::from_secs(8 * 3600));
        match run_with_freshness(&d, &spec, &kinds, &ages) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons.iter().any(|r| r.contains("stale")));
            }
            _ => panic!("expected Block (stale)"),
        }
    }

    #[test]
    fn run_with_freshness_hard_expired_blocks_with_repair() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        let eid = EvidenceAuditId::new(1);
        d.evidence_refs = vec![eid];
        d.findings.push(HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![eid],
        });
        let mut kinds = std::collections::HashMap::new();
        kinds.insert(eid, "http_probe".to_string());
        let mut ages = std::collections::HashMap::new();
        // age = 24h (> 2 * 6h) → hard expired (block + repair tool call)
        ages.insert(eid, std::time::Duration::from_secs(24 * 3600));
        match run_with_freshness(&d, &spec, &kinds, &ages) {
            GateCheckOutcome::Block { reasons, recovery } => {
                assert!(reasons.iter().any(|r| r.contains("hard-expired")));
                assert!(recovery
                    .repair_tool_calls
                    .iter()
                    .any(|c| c.contains("re-acquire fresh http_probe")));
            }
            _ => panic!("expected Block (hard-expired)"),
        }
    }

    #[test]
    fn run_with_freshness_missing_kind_skipped() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        let eid = EvidenceAuditId::new(1);
        d.evidence_refs = vec![eid];
        d.findings.push(HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![eid],
        });
        // 不提供 kind / age → 跳过 freshness 检查仅做 sanity → Pass
        let kinds = std::collections::HashMap::new();
        let ages = std::collections::HashMap::new();
        assert!(matches!(
            run_with_freshness(&d, &spec, &kinds, &ages),
            GateCheckOutcome::Pass
        ));
    }

    #[test]
    fn freshness_age_reasons_classifies_fresh_stale_expired() {
        let fresh = EvidenceAuditId::new(1);
        let stale = EvidenceAuditId::new(2);
        let expired = EvidenceAuditId::new(3);
        let mut kinds = std::collections::HashMap::new();
        kinds.insert(fresh, "http_probe".to_string()); // max 21600s (6h)
        kinds.insert(stale, "http_probe".to_string());
        kinds.insert(expired, "http_probe".to_string());
        let mut ages = std::collections::HashMap::new();
        ages.insert(fresh, std::time::Duration::from_secs(60)); // 1m → fresh
        ages.insert(stale, std::time::Duration::from_secs(8 * 3600)); // 8h → stale (6h–12h)
        ages.insert(expired, std::time::Duration::from_secs(24 * 3600)); // 24h → expired (>12h)
        let (exp, stl) = freshness_age_reasons(&[fresh, stale, expired], &kinds, &ages);
        assert!(exp
            .iter()
            .any(|r| r.contains("hard-expired") && r.contains("id=3")));
        assert!(stl
            .iter()
            .any(|r| r.contains("stale") && r.contains("id=2")));
        assert!(!exp.iter().chain(stl.iter()).any(|r| r.contains("id=1")));
    }

    #[test]
    fn freshness_age_reasons_skips_missing_kind_or_age() {
        let id = EvidenceAuditId::new(5);
        let kinds = std::collections::HashMap::new(); // no kind for the id
        let mut ages = std::collections::HashMap::new();
        ages.insert(id, std::time::Duration::from_secs(999_999));
        let (exp, stl) = freshness_age_reasons(&[id], &kinds, &ages);
        assert!(exp.is_empty() && stl.is_empty());
    }
}
