//! Per-org 权威 gate 评估器（chat `stage_run` 扇出用）。
//!
//! `stage_run` 串行对每个 org 跑完专家后，用本模块对**该 org 自己的** StageDeliverable
//! 跑一次注入了该 org DB 真值的 gate（与 orchestrator stage-close gate 同一套
//! `validate_stage_gate_with_context` + 同一批 repo 查询），用 PASS/BLOCK 决定该 org
//! 是否算通过——取代旧的「子 agent 跑完即通过」。纯函数部分单测覆盖。

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
use super::gate::{validate_stage_gate_with_context, GateContextBuilder, GateResult};
use super::stage_spec::StageSpec;
use super::types::{HarnessRecoveryActions, StageDeliverable};
use super::{load_embedded_stage_spec, StageKind};
use crate::db_traits::DbRepoProvider;

/// 一个 org 在某 stage 的裁决。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrgVerdict {
    /// gate 通过：可计入 passed_count + 写完成账本。
    Pass,
    /// gate 未过：进 gaps，**不**写账本；`reasons` 供汇报，
    /// `recovery_actions` 供 StageRefiner 结构化闭环回灌。
    Block {
        reasons: Vec<String>,
        recovery_actions: HarnessRecoveryActions,
    },
}

/// `GateResult` → `OrgVerdict`（纯函数，单测）。
pub fn decide_org_verdict(gate: &GateResult) -> OrgVerdict {
    if gate.allowed {
        OrgVerdict::Pass
    } else {
        OrgVerdict::Block {
            reasons: gate.reasons.clone(),
            recovery_actions: gate.recovery_actions.clone().unwrap_or_default(),
        }
    }
}

/// `evidence_facts_for_session` 的 `(asset, technique, outcome, id)` 行 →
/// `EvidenceFact`（纯函数，单测）。`outcome` 文本：`"found"` → Found，其余 → Empty
/// （I8：只有显式 found 才算 Found，绝不把别的当 Found）。
pub fn facts_from_rows(rows: Vec<(String, String, String, i64)>) -> Vec<EvidenceFact> {
    rows.into_iter()
        .map(|(asset, technique, outcome, id)| EvidenceFact {
            asset,
            technique,
            outcome: if outcome.eq_ignore_ascii_case("found") {
                EvidenceOutcome::Found
            } else if outcome.eq_ignore_ascii_case("error") {
                // T2：失败检查（gray-switch）记 error，≠ checked_empty。
                EvidenceOutcome::Error
            } else {
                EvidenceOutcome::Empty
            },
            evidence_id: id,
        })
        .collect()
}

/// db_truth `(asset, technique)` → Found `EvidenceFact`（哨兵 id=0，与 execute.rs
/// `DB_TRUTH_EVIDENCE_ID` 同义：投影只看 asset/technique/outcome，与 id 无关）。
fn db_truth_to_facts(rows: Vec<(String, String)>) -> Vec<EvidenceFact> {
    rows.into_iter()
        .map(|(asset, technique)| EvidenceFact {
            asset,
            technique,
            outcome: EvidenceOutcome::Found,
            evidence_id: 0,
        })
        .collect()
}

// ── Phase 1.5：fan-out 阶段过门令牌（hash） ─────────────────────────────
//
// specialist（fan-out）阶段的**阶段收尾**不再让主 agent 重交一份整阶段 deliverable、
// 再跑一遍整阶段 coverage（冗余 + 单槽存不下 N 个 org + 整库资产轴 org_id=None 分母
// 爆炸）。改为：stage_run 全过时对 per-org 完成账本算一个确定性 hash 令牌，主 agent 带
// 回；收尾 gate 拿**同一张账本重算**比对（B-recompute），全 org 新鲜 PASS 且令牌对上才
// 放行，否则 BLOCK 提示「只重跑缺口 org」。令牌**不绑 session**：两条路径（runtime
// stage_run / orchestrator gate）的 session id 维度可能不一致 → 会造成「永远对不上」死锁；
// 防伪真正靠 agent 看不到也造不出的 `passed_at`（只有真过 gate 才由 stage_run 写库）+
// 新鲜度窗口。

/// 主 agent 收尾时承载 pass_token 的保留 claim kind。
pub const STAGE_RUN_PASS_TOKEN_KIND: &str = "stage_run_pass_token";

/// 一个 org 的 stage 完成账本行在 TTL 窗口内才算「新鲜通过」。stage_run（发令牌）与收尾
/// gate（验令牌）**必须共用**此常量与 [`completion_is_fresh`]，否则一边发了令牌、另一边按
/// 不同 TTL 判过期 → 假 BLOCK。（值与历史 stage_run resume-skip 一致：7 天。）
pub const STAGE_COMPLETION_TTL_SECS: i64 = 7 * 24 * 3600;

/// 纯 TTL 判定（IO-free，可单测）：`passed_at` 是否在 `now` 前 `ttl_secs` 内？未来时间戳
/// （时钟偏移）一律当新鲜，绝不提前重跑。
pub fn completion_is_fresh(passed_at: DateTime<Utc>, now: DateTime<Utc>, ttl_secs: i64) -> bool {
    now.signed_duration_since(passed_at).num_seconds() <= ttl_secs
}

/// A stage completion is usable for the current operation only if it is fresh
/// within the TTL and, when the operation has a stage-start anchor, was written
/// after that stage began. This prevents a new operation from short-circuiting
/// its workers using an older run's `org_stage_completions` rows while the
/// current gate still expects current-run evidence/source rows.
pub fn completion_is_fresh_for_stage(
    passed_at: DateTime<Utc>,
    now: DateTime<Utc>,
    ttl_secs: i64,
    not_before: Option<DateTime<Utc>>,
) -> bool {
    if let Some(floor) = not_before {
        if passed_at < floor {
            return false;
        }
    }
    completion_is_fresh(passed_at, now, ttl_secs)
}

/// Pick the org axis used by fan-out pass-token generation and closeout
/// verification. Once scoping binds an engagement root, only that root's
/// subtree belongs to this operation; sibling orgs elsewhere in the DB must not
/// participate in token generation or block closeout.
///
/// `engagement_subtree_ids = None` is only used when there is no bound root; a
/// bound root with an unreadable/empty subtree remains empty so callers
/// fail-closed instead of falling back to the whole DB.
pub fn fanout_completion_scope_ids(
    engagement_root: Option<Uuid>,
    engagement_subtree_ids: Option<Vec<Uuid>>,
    legacy_in_scope_ids: Vec<Uuid>,
) -> Vec<Uuid> {
    if engagement_root.is_some() {
        engagement_subtree_ids.unwrap_or_default()
    } else {
        legacy_in_scope_ids
    }
}

/// 对 (stage, 全 org 的 PASS 行) 算确定性摘要令牌。**规范化**：org 行按 org_id 升序、用账本
/// 里的 `passed_at` 微秒时间戳入摘要，保证 stage_run 端与 gate 端对同一张账本态算出同一串。
/// 空行集 → 空串（调用方按「无 PASS」处理）。
pub fn stage_pass_token(stage: StageKind, rows: &[(Uuid, DateTime<Utc>)]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<&(Uuid, DateTime<Utc>)> = rows.iter().collect();
    sorted.sort_by_key(|t| t.0);
    let mut h = Sha256::new();
    h.update(stage.as_str().as_bytes());
    for (org, at) in sorted {
        h.update(b"|");
        h.update(org.as_bytes());
        h.update(b"@");
        h.update(at.timestamp_micros().to_le_bytes());
    }
    h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

/// 从 deliverable 抽主 agent 带回的 stage_run pass_token（保留 claim 的 summary，trim 后非空）。
pub fn extract_pass_token(deliverable: &StageDeliverable) -> Option<String> {
    deliverable
        .claims
        .iter()
        .find(|c| c.kind == STAGE_RUN_PASS_TOKEN_KIND)
        .map(|c| c.summary.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 对 `org_id` 的某 stage 交付跑一次注入了该 org DB 真值的权威 gate。
///
/// 复用 orchestrator stage-close 的同一批 repo 查询（`in_scope_assets` /
/// `in_scope_typed_assets` / `evidence_facts_for_session` / `db_truth_facts`）+ 同一个
/// `validate_stage_gate_with_context`，把判定按 org 隔离。先做一次 fabricated-ref 存在性
/// 兜底（与 execute.rs `enforce_evidence_existence` 同义；scoping 例外——它不产账本证据）。
///
/// 失败回退：spec 加载失败 → 直接 Block（fail-closed，配置坏不该放行）。repo 缺失/
/// DB 错由调用方（stage_run）决定回退策略，本函数要求传入可用 repo。
pub async fn evaluate_org_stage_gate(
    repo: &dyn DbRepoProvider,
    org_id: Option<Uuid>,
    session_id: &str,
    stage: StageKind,
    deliverable: &StageDeliverable,
) -> GateResult {
    let spec: StageSpec = match load_embedded_stage_spec(stage) {
        Ok(s) => s,
        Err(e) => {
            return GateResult::block(
                vec![format!(
                    "could not load stage spec for {}: {e}",
                    stage.as_str()
                )],
                Default::default(),
            )
        }
    };

    // 1) fabricated-ref 兜底（scoping 不要求账本证据，跳过）。
    if stage != StageKind::Scoping {
        let cited: Vec<i64> = deliverable
            .evidence_refs
            .iter()
            .map(|e| e.as_i64())
            .collect();
        if !cited.is_empty() {
            if let Ok(existing) = repo.evidence_existing_ids(&cited).await {
                let fabricated: Vec<i64> = cited
                    .into_iter()
                    .filter(|id| !existing.contains(id))
                    .collect();
                if !fabricated.is_empty() {
                    return GateResult::block(
                        vec![format!(
                            "cited evidence ids {fabricated:?} do not exist in the evidence ledger"
                        )],
                        Default::default(),
                    );
                }
            }
            // infra error → 不在这兜底 BLOCK（与 execute.rs fail-open 一致），交给覆盖 gate。
        }
    }

    // 2) 资产轴 + 类型（org 隔离）。空资产集 → 不注入（gate 回退自报，coverage_complete
    //    自带「空矩阵但声明了期望技术 → BLOCK」保护）。
    let in_scope_assets = repo.in_scope_assets(org_id).await.unwrap_or_default();
    let typed_assets = repo.in_scope_typed_assets(org_id).await.unwrap_or_default();

    // 3) 证据事实：账本投影 + DB 业务表真值（Found）合并。
    let mut facts: Vec<EvidenceFact> = repo
        .evidence_facts_for_session(session_id)
        .await
        .map(facts_from_rows)
        .unwrap_or_default();
    if !in_scope_assets.is_empty() {
        // Per-org fan-out scoping gate stays presence-only (run_start=None): the
        // per-dimension freshness window (design 2026-06-22) is wired on the
        // single-org target_intel execute.rs path; threading the stage-run anchor
        // through the multi-org fan-out gate is separate, deferred work.
        if let Ok(truth) = repo.db_truth_facts(org_id, &in_scope_assets, None).await {
            facts.extend(db_truth_to_facts(truth));
        }
    }

    // #4/E3 (设计 2026-06-23-technique-outcomes-provenance)：**始终**从
    // technique_outcomes union 进 facts（per-org fan-out gate；与 execute.rs 主路径同
    // 源）。org=None = 跳过（additive + fail-safe，无灰度开关）。outcome blocked→Error。
    if let Some(oid) = org_id {
        let projected: Vec<EvidenceFact> = repo
            .technique_outcome_facts(oid, session_id)
            .await
            .into_iter()
            .filter_map(|(asset, technique, outcome, evidence_id)| {
                let outcome = match outcome.as_str() {
                    "found" => EvidenceOutcome::Found,
                    "empty" => EvidenceOutcome::Empty,
                    "error" | "blocked" => EvidenceOutcome::Error,
                    _ => return None,
                };
                Some(EvidenceFact {
                    asset,
                    technique,
                    outcome,
                    evidence_id,
                })
            })
            .collect();
        facts.extend(projected);
    }

    let source_queries = match org_id {
        Some(oid) => repo.source_query_facts(oid, session_id).await,
        None => Vec::new(),
    };

    // 统一组装入口（设计 2026-06-23-unified-gate-context-builder）：归一/合并收口到
    // GateContextBuilder。expected_techniques=None ⇒ 回退 spec.expected_techniques
    // （target_intel 已声明）。
    let ctx = GateContextBuilder::new()
        .in_scope_assets(in_scope_assets)
        .typed_assets(typed_assets)
        .extend_evidence_facts(facts)
        .extend_source_queries(source_queries)
        .expected_techniques(None)
        .build();

    validate_stage_gate_with_context(deliverable, &spec, None, None, &ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::types::{CoverageGapAction, HarnessRecoveryActions};

    #[test]
    fn verdict_pass_on_allowed() {
        assert_eq!(decide_org_verdict(&GateResult::pass()), OrgVerdict::Pass);
    }

    #[test]
    fn verdict_block_carries_reasons() {
        let g = GateResult::block(
            vec!["coverage incomplete".to_string()],
            HarnessRecoveryActions::default(),
        );
        assert_eq!(
            decide_org_verdict(&g),
            OrgVerdict::Block {
                reasons: vec!["coverage incomplete".to_string()],
                recovery_actions: HarnessRecoveryActions::default()
            }
        );
    }

    #[test]
    fn verdict_block_carries_recovery_actions() {
        let recovery = HarnessRecoveryActions {
            coverage_gap_actions: vec![CoverageGapAction {
                asset: "a.com".to_string(),
                technique: "GOLISH-EAS-LIVENESS".to_string(),
                reason: "missing liveness".to_string(),
                suggested_tools: vec!["httpx".to_string()],
            }],
            ..Default::default()
        };
        let g = GateResult::block(vec!["coverage incomplete".to_string()], recovery.clone());

        assert_eq!(
            decide_org_verdict(&g),
            OrgVerdict::Block {
                reasons: vec!["coverage incomplete".to_string()],
                recovery_actions: recovery
            }
        );
    }

    #[test]
    fn facts_only_found_outcome_maps_found() {
        let f = facts_from_rows(vec![
            ("a.com".into(), "GOLISH-INTEL-DNS".into(), "found".into(), 7),
            (
                "a.com".into(),
                "GOLISH-INTEL-WHOIS".into(),
                "empty".into(),
                8,
            ),
        ]);
        assert_eq!(f[0].outcome, EvidenceOutcome::Found);
        assert_eq!(f[1].outcome, EvidenceOutcome::Empty);
        assert_eq!(f[0].evidence_id, 7);
    }

    #[test]
    fn db_truth_rows_are_found_sentinel() {
        let f = db_truth_to_facts(vec![("a.com".into(), "GOLISH-INTEL-ASN".into())]);
        assert_eq!(f[0].outcome, EvidenceOutcome::Found);
        assert_eq!(f[0].evidence_id, 0);
    }

    // ── Phase 1.5 token / freshness ──────────────────────────────────────

    #[test]
    fn token_is_order_independent_and_binds_stage_and_org_set() {
        let now = Utc::now();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let t1 = stage_pass_token(StageKind::TargetIntel, &[(a, now), (b, now)]);
        let t2 = stage_pass_token(StageKind::TargetIntel, &[(b, now), (a, now)]);
        assert_eq!(t1, t2, "org 顺序不影响令牌");
        assert_ne!(
            t1,
            stage_pass_token(StageKind::Enumeration, &[(a, now), (b, now)]),
            "stage 变 → 令牌变"
        );
        assert_ne!(
            t1,
            stage_pass_token(StageKind::TargetIntel, &[(a, now)]),
            "少一个 org → 令牌变"
        );
        assert!(stage_pass_token(StageKind::TargetIntel, &[]).is_empty());
    }

    #[test]
    fn token_changes_with_passed_at() {
        let a = Uuid::from_u128(1);
        let t0 = Utc::now();
        let t1 = t0 + chrono::Duration::seconds(5);
        assert_ne!(
            stage_pass_token(StageKind::TargetIntel, &[(a, t0)]),
            stage_pass_token(StageKind::TargetIntel, &[(a, t1)]),
            "passed_at 变 → 令牌变（旧账本态蒙混不了）"
        );
    }

    #[test]
    fn fanout_completion_scope_uses_engagement_subtree_when_bound() {
        let root = Uuid::from_u128(1);
        let child = Uuid::from_u128(2);
        let sibling = Uuid::from_u128(3);

        assert_eq!(
            fanout_completion_scope_ids(
                Some(root),
                Some(vec![root, child]),
                vec![root, child, sibling]
            ),
            vec![root, child],
            "bound operations must not let sibling orgs from the whole DB block closeout"
        );
    }

    #[test]
    fn fanout_completion_scope_is_fail_closed_when_bound_subtree_missing() {
        let root = Uuid::from_u128(1);
        let sibling = Uuid::from_u128(3);

        assert!(fanout_completion_scope_ids(Some(root), None, vec![root, sibling]).is_empty());
        assert!(
            fanout_completion_scope_ids(Some(root), Some(vec![]), vec![root, sibling]).is_empty()
        );
    }

    #[test]
    fn fanout_completion_scope_keeps_legacy_axis_without_root_binding() {
        let root = Uuid::from_u128(1);
        let sibling = Uuid::from_u128(3);

        assert_eq!(
            fanout_completion_scope_ids(None, Some(vec![root]), vec![root, sibling]),
            vec![root, sibling]
        );
    }

    #[test]
    fn extract_pass_token_reads_reserved_claim_trimmed() {
        use crate::harness::types::StageClaim;
        let d = StageDeliverable {
            stage_id: "target_intel".into(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![StageClaim {
                kind: STAGE_RUN_PASS_TOKEN_KIND.into(),
                subject: "target_intel".into(),
                summary: "  deadbeef  ".into(),
                evidence_ids: vec![],
                technique: None,
            }],
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: vec![],
            coverage: vec![],
        };
        assert_eq!(extract_pass_token(&d).as_deref(), Some("deadbeef"));
    }

    #[test]
    fn extract_pass_token_none_when_absent_or_blank() {
        use crate::harness::types::StageClaim;
        let mut d = StageDeliverable {
            stage_id: "target_intel".into(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![],
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: vec![],
            coverage: vec![],
        };
        assert_eq!(extract_pass_token(&d), None);
        d.claims.push(StageClaim {
            kind: STAGE_RUN_PASS_TOKEN_KIND.into(),
            subject: "x".into(),
            summary: "   ".into(),
            evidence_ids: vec![],
            technique: None,
        });
        assert_eq!(extract_pass_token(&d), None, "空白 summary → None");
    }

    #[test]
    fn completion_freshness_ttl_boundaries() {
        let now = Utc::now();
        let ttl = STAGE_COMPLETION_TTL_SECS;
        assert!(completion_is_fresh(now, now, ttl));
        assert!(completion_is_fresh(
            now - chrono::Duration::days(1),
            now,
            ttl
        ));
        assert!(completion_is_fresh(
            now - chrono::Duration::seconds(ttl),
            now,
            ttl
        ));
        assert!(!completion_is_fresh(
            now - chrono::Duration::days(8),
            now,
            ttl
        ));
        assert!(completion_is_fresh(
            now + chrono::Duration::hours(1),
            now,
            ttl
        ));
    }

    #[test]
    fn completion_fresh_for_stage_respects_stage_start_floor() {
        let now = Utc::now();
        let floor = now - chrono::Duration::minutes(10);
        let ttl = STAGE_COMPLETION_TTL_SECS;

        assert!(!completion_is_fresh_for_stage(
            now - chrono::Duration::hours(1),
            now,
            ttl,
            Some(floor)
        ));
        assert!(completion_is_fresh_for_stage(
            now - chrono::Duration::minutes(5),
            now,
            ttl,
            Some(floor)
        ));
        assert!(completion_is_fresh_for_stage(
            now - chrono::Duration::hours(1),
            now,
            ttl,
            None
        ));
    }
}
