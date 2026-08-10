//! Per-org 权威 gate 评估器（chat `stage_run` 扇出用）。
//!
//! `stage_run` 串行对每个 org 跑完专家后，用本模块对**该 org 自己的** StageDeliverable
//! 跑一次注入了该 org DB 真值的 gate（与 orchestrator stage-close gate 同一套
//! `validate_stage_gate_with_context` + 同一批 repo 查询），用 PASS/BLOCK 决定该 org
//! 是否算通过——取代旧的「子 agent 跑完即通过」。纯函数部分单测覆盖。

use chrono::{DateTime, Utc};
use golish_pentest_domain::tool_truth::ToolTruthContract;
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::collections::{BTreeSet, HashMap};
use uuid::Uuid;

use super::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
use super::gate::{
    completed_from_guarded_outcomes, validate_stage_gate_with_context, GateContextBuilder,
    GateResult,
};
use super::stage_spec::StageSpec;
#[cfg(test)]
use super::technique_resolver::AssetClass;
use super::types::{HarnessRecoveryActions, StageDeliverable};
use super::{load_embedded_stage_spec, StageKind};
use crate::db_traits::{
    DbRepoProvider, RecordToolTruthShadowAssessment, StageAssetWaveView, TechniqueOutcomeFact,
};

const TECH_INTEL_DNS: &str = "GOLISH-INTEL-DNS";
const TECH_INTEL_CT: &str = "GOLISH-INTEL-CT";
const TECH_INTEL_SUBDOMAIN: &str = "GOLISH-INTEL-SUBDOMAIN";
#[allow(dead_code)]
const TECH_INTEL_WHOIS: &str = "GOLISH-INTEL-WHOIS";
#[allow(dead_code)]
const TECH_INTEL_ASN: &str = "GOLISH-INTEL-ASN";
#[allow(dead_code)]
const TECH_INTEL_OSINT: &str = "GOLISH-INTEL-OSINT";
const TECH_EAS_LIVENESS: &str = "GOLISH-EAS-LIVENESS";
const TECH_EAS_PORT: &str = "GOLISH-EAS-PORT";
const TECH_EAS_SERVICE_FP: &str = "GOLISH-EAS-SERVICE-FINGERPRINT";
const TECH_EAS_WEB_FP: &str = "GOLISH-EAS-WEB-FINGERPRINT";
const EAS_TECHNIQUES: [&str; 4] = [
    TECH_EAS_LIVENESS,
    TECH_EAS_PORT,
    TECH_EAS_SERVICE_FP,
    TECH_EAS_WEB_FP,
];
/// Content-enumeration axes (design 2026-07-03): an IP proven DNS/53-only with
/// no web surface is not_applicable for all four.
const ENUM_CONTENT_TECHNIQUES: [&str; 4] = [
    "GOLISH-ENUM-JS",
    "GOLISH-ENUM-DIR",
    "GOLISH-ENUM-PARAM",
    "GOLISH-ENUM-JSAPI",
];
const VULN_TRIAGE_TECHNIQUES: [&str; 10] = [
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
];
const TRUSTED_ENUM_BLOCKED_SOURCE: &str = "enum_preflight_web_origins";
const TRUSTED_ENUM_ROUTE_RECOVERY_BLOCKED_SOURCE: &str = "route_probe_paths";
const TRUSTED_ENUM_COLLECTION_RECOVERY_BLOCKED_SOURCE: &str = "browser_collect_js_api";
const TRUSTED_EAS_WEB_BLOCKED_SOURCE: &str = "eas_fingerprint_web_stack";
const TRUSTED_VULN_ANONYMOUS_ACCESS_SOURCE: &str = "vuln_probe_anonymous_access";

/// Collision-proof Target Intel organization coverage identity shared by the
/// submit preview and the final per-org gate.
///
/// Organization profile work owns WHOIS/ASN/OSINT. DNS/CT/SUBDOMAIN belong to
/// executable target rows, so those three cells are deterministic N/A on this
/// synthetic, non-executable row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetIntelOrganizationContext {
    asset_key: String,
}

impl TargetIntelOrganizationContext {
    pub fn new(organization_id: Uuid) -> Self {
        Self {
            asset_key: format!("organization:{organization_id}"),
        }
    }

    pub fn asset_key(&self) -> &str {
        &self.asset_key
    }

    pub fn inject_asset(&self, in_scope_assets: &mut Vec<String>) {
        if !in_scope_assets.contains(&self.asset_key) {
            in_scope_assets.push(self.asset_key.clone());
        }
    }

    pub fn inject_type(&self, typed_assets: &mut Vec<(String, String)>) {
        match typed_assets
            .iter_mut()
            .find(|(asset, _)| asset == &self.asset_key)
        {
            Some((_, target_type)) => *target_type = "organization".to_string(),
            None => typed_assets.push((self.asset_key.clone(), "organization".to_string())),
        }
    }

    pub fn inject_axis(
        &self,
        in_scope_assets: &mut Vec<String>,
        typed_assets: &mut Vec<(String, String)>,
    ) {
        self.inject_asset(in_scope_assets);
        self.inject_type(typed_assets);
    }

    pub fn not_applicable_coverage(&self) -> Vec<(String, String)> {
        [TECH_INTEL_DNS, TECH_INTEL_CT, TECH_INTEL_SUBDOMAIN]
            .into_iter()
            .map(|technique| (self.asset_key.clone(), technique.to_string()))
            .collect()
    }
}

/// Resolve the collision-proof identity for the synthetic Target Intel
/// organization row exposed by the coverage read model. Callers must still
/// scope this to Target Intel; the target type and UUID check prevent a domain
/// whose display value resembles an organization name from becoming an alias.
pub fn target_intel_organization_asset_key(
    target_type: Option<&str>,
    target_id: Option<&str>,
) -> Option<String> {
    if target_type != Some("organization") {
        return None;
    }
    let organization_id = target_id.and_then(|value| Uuid::parse_str(value).ok())?;
    Some(format!("organization:{organization_id}"))
}

#[allow(dead_code)]
fn target_intel_deliverable_with_organization_aliases(
    deliverable: &StageDeliverable,
    organization_key: &str,
    organization_display_name: &str,
) -> StageDeliverable {
    let mut normalized = deliverable.clone();
    let aliases = normalized
        .coverage
        .iter()
        .filter(|cell| {
            cell.asset == organization_display_name
                && matches!(
                    cell.technique.as_str(),
                    TECH_INTEL_WHOIS | TECH_INTEL_ASN | TECH_INTEL_OSINT
                )
        })
        .cloned()
        .map(|mut cell| {
            cell.asset = organization_key.to_string();
            cell
        })
        .collect::<Vec<_>>();
    normalized.coverage.extend(aliases);
    normalized
}

fn trusted_enumeration_blocked_source(
    contract: ToolTruthContract,
    technique: &str,
    source: Option<&str>,
) -> bool {
    match source {
        Some(TRUSTED_ENUM_BLOCKED_SOURCE) => {
            !contract.enforces_fail_safe_projection()
                && ENUM_CONTENT_TECHNIQUES.contains(&technique)
        }
        Some(TRUSTED_ENUM_ROUTE_RECOVERY_BLOCKED_SOURCE) => technique == "GOLISH-ENUM-DIR",
        Some(TRUSTED_ENUM_COLLECTION_RECOVERY_BLOCKED_SOURCE) => matches!(
            technique,
            "GOLISH-ENUM-JS" | "GOLISH-ENUM-PARAM" | "GOLISH-ENUM-JSAPI"
        ),
        _ => false,
    }
}

pub type ExactWebOriginCoverageAxis = (Vec<String>, Vec<(String, String)>);
pub type EnumerationCoverageAxis = ExactWebOriginCoverageAxis;

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
/// `EvidenceFact`（纯函数，单测）。`partial` 是非终态，不能伪装成 checked-empty；
/// `found` / `blocked` / `error` 保留各自语义，其余历史终态兼容映射为 Empty（I8）。
pub fn facts_from_rows(rows: Vec<(String, String, String, i64)>) -> Vec<EvidenceFact> {
    rows.into_iter()
        .filter_map(|(asset, technique, outcome, id)| {
            if outcome.eq_ignore_ascii_case("partial") {
                return None;
            }
            Some(EvidenceFact {
                asset,
                technique,
                outcome: if outcome.eq_ignore_ascii_case("found") {
                    EvidenceOutcome::Found
                } else if outcome.eq_ignore_ascii_case("blocked") {
                    EvidenceOutcome::Blocked
                } else if outcome.eq_ignore_ascii_case("error") {
                    // T2：失败检查（gray-switch）记 error，≠ checked_empty。
                    EvidenceOutcome::Error
                } else {
                    EvidenceOutcome::Empty
                },
                evidence_id: id,
            })
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

fn technique_outcome_to_fact(
    asset: String,
    technique: String,
    outcome: String,
    evidence_id: i64,
) -> Option<EvidenceFact> {
    let outcome = match outcome.as_str() {
        "found" => EvidenceOutcome::Found,
        "empty" => EvidenceOutcome::Empty,
        // Enumeration projects `partial` as a non-terminal marker. Its spec sets
        // `error_is_terminal=false`, so the marker can veto model-side terminal
        // assertions without accidentally closing the cell.
        "error" => EvidenceOutcome::Error,
        "blocked" => EvidenceOutcome::Blocked,
        _ => return None,
    };
    Some(EvidenceFact {
        asset,
        technique,
        outcome,
        evidence_id,
    })
}

fn eas_fact_asset_key(asset: &str, technique: &str) -> String {
    if technique == TECH_EAS_LIVENESS {
        return super::evidence_facts::eas_liveness_asset_key(asset)
            .unwrap_or_else(|| asset.trim().to_ascii_lowercase());
    }
    golish_pentest_domain::canonical_asset_key(asset)
        .map(|key| key.key)
        .unwrap_or_else(|| asset.trim().to_ascii_lowercase())
}

fn eas_guarded_fact_asset_key(asset: &str, technique: &str) -> String {
    if technique == TECH_EAS_WEB_FP {
        return golish_pentest_domain::canonical_web_origin(asset)
            .map(|origin| origin.key)
            .unwrap_or_else(|| asset.trim().to_ascii_lowercase());
    }
    eas_fact_asset_key(asset, technique)
}

fn target_intel_fact_asset_key(asset: &str) -> String {
    asset.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// Merge the current run's provenance rows into gate facts. For Enumeration and
/// Vuln Triage, current exact-origin `technique_outcomes` are the sole completion
/// truth: legacy ledger/DB facts are removed first, then this run's rows are
/// projected. Partial reuses the Error sentinel, while both stage specs make
/// Error non-terminal; missing/marker rows therefore cannot inherit a PASS from
/// stale evidence.
pub fn apply_technique_outcome_rows(
    stage: StageKind,
    facts: &mut Vec<EvidenceFact>,
    rows: &[TechniqueOutcomeFact],
) {
    apply_technique_outcome_rows_for_contract(ToolTruthContract::LegacyV1, stage, facts, rows);
}

pub fn apply_technique_outcome_rows_for_contract(
    contract: ToolTruthContract,
    stage: StageKind,
    facts: &mut Vec<EvidenceFact>,
    rows: &[TechniqueOutcomeFact],
) {
    let enumeration_blocked_evidence: std::collections::HashSet<(String, String, i64)> =
        if stage == StageKind::Enumeration {
            facts
                .iter()
                .filter(|fact| {
                    fact.outcome == EvidenceOutcome::Blocked
                        && fact.evidence_id > 0
                        && ENUM_CONTENT_TECHNIQUES.contains(&fact.technique.as_str())
                })
                .filter_map(|fact| {
                    Some((
                        golish_pentest_domain::canonical_web_origin(&fact.asset)?.key,
                        fact.technique.clone(),
                        fact.evidence_id,
                    ))
                })
                .collect()
        } else {
            std::collections::HashSet::new()
        };
    let eas_business_found: std::collections::HashSet<(String, String)> =
        if stage == StageKind::ExternalAttackSurface {
            facts
                .iter()
                .filter(|fact| {
                    fact.outcome == EvidenceOutcome::Found
                        && fact.evidence_id == 0
                        && EAS_TECHNIQUES.contains(&fact.technique.as_str())
                })
                .map(|fact| {
                    (
                        eas_fact_asset_key(&fact.asset, &fact.technique),
                        fact.technique.clone(),
                    )
                })
                .collect()
        } else {
            std::collections::HashSet::new()
        };
    let target_intel_business_found: std::collections::HashSet<(String, String)> =
        if stage == StageKind::TargetIntel {
            facts
                .iter()
                .filter(|fact| fact.outcome == EvidenceOutcome::Found && fact.evidence_id == 0)
                .map(|fact| {
                    (
                        target_intel_fact_asset_key(&fact.asset),
                        fact.technique.clone(),
                    )
                })
                .collect()
        } else {
            std::collections::HashSet::new()
        };
    let eas_guarded_evidence: std::collections::HashSet<(String, String, String, i64)> =
        if stage == StageKind::ExternalAttackSurface {
            facts
                .iter()
                .filter(|fact| {
                    fact.evidence_id > 0 && EAS_TECHNIQUES.contains(&fact.technique.as_str())
                })
                .map(|fact| {
                    (
                        eas_guarded_fact_asset_key(&fact.asset, &fact.technique),
                        fact.technique.clone(),
                        match fact.outcome {
                            EvidenceOutcome::Found => "found",
                            EvidenceOutcome::Empty => "empty",
                            EvidenceOutcome::Error => "error",
                            EvidenceOutcome::Blocked => "blocked",
                        }
                        .to_string(),
                        fact.evidence_id,
                    )
                })
                .collect()
        } else {
            std::collections::HashSet::new()
        };
    if stage == StageKind::Enumeration {
        facts.retain(|fact| !ENUM_CONTENT_TECHNIQUES.contains(&fact.technique.as_str()));
    } else if stage == StageKind::VulnTriage {
        facts.retain(|fact| !VULN_TRIAGE_TECHNIQUES.contains(&fact.technique.as_str()));
    } else if stage == StageKind::ExternalAttackSurface {
        // Raw ledger and business-table EAS facts are corroboration inputs only.
        // A terminal cell is re-added below solely from a fresh org/current-owner
        // technique_outcome whose positive evidence id matched guarded audit.
        facts.retain(|fact| !EAS_TECHNIQUES.contains(&fact.technique.as_str()));
    }

    facts.extend(rows.iter().filter_map(|row| {
        let TechniqueOutcomeFact {
            asset,
            technique,
            outcome,
            evidence_id,
            source,
        } = row;
        if stage == StageKind::Enumeration
            && ENUM_CONTENT_TECHNIQUES.contains(&technique.as_str())
            && matches!(outcome.as_str(), "found" | "empty" | "blocked")
            && *evidence_id <= 0
        {
            return None;
        }
        if stage == StageKind::VulnTriage {
            if !VULN_TRIAGE_TECHNIQUES.contains(&technique.as_str())
                || !vuln_outcome_source_is_trusted(technique, source.as_deref())
            {
                return None;
            }
            if matches!(
                outcome.as_str(),
                "found" | "empty" | "blocked" | "not_applicable"
            ) && *evidence_id <= 0
            {
                return None;
            }
        }
        if stage == StageKind::Enumeration
            && outcome == "blocked"
            && !trusted_enumeration_blocked_source(contract, technique, source.as_deref())
        {
            return None;
        }
        if stage == StageKind::Enumeration
            && outcome == "blocked"
            && !golish_pentest_domain::canonical_web_origin(asset).is_some_and(|origin| {
                enumeration_blocked_evidence.contains(&(
                    origin.key,
                    technique.clone(),
                    *evidence_id,
                ))
            })
        {
            return None;
        }
        if stage == StageKind::ExternalAttackSurface && EAS_TECHNIQUES.contains(&technique.as_str())
        {
            let trusted_web_blocked = technique == TECH_EAS_WEB_FP
                && outcome == "blocked"
                && source.as_deref() == Some(TRUSTED_EAS_WEB_BLOCKED_SOURCE)
                && golish_pentest_domain::canonical_web_origin(asset).is_some();
            if *evidence_id <= 0
                || (!matches!(outcome.as_str(), "found" | "empty") && !trusted_web_blocked)
            {
                return None;
            }
            if !eas_guarded_evidence.contains(&(
                eas_guarded_fact_asset_key(asset, technique),
                technique.clone(),
                outcome.clone(),
                *evidence_id,
            )) {
                return None;
            }
            if outcome == "found"
                && !eas_business_found
                    .contains(&(eas_fact_asset_key(asset, technique), technique.clone()))
                && !eas_cidr_range_outcome_is_self_corroborating(
                    asset,
                    technique,
                    source.as_deref(),
                )
            {
                return None;
            }
        }
        if stage == StageKind::TargetIntel
            && outcome == "found"
            && !target_intel_business_found
                .contains(&(target_intel_fact_asset_key(asset), technique.clone()))
        {
            return None;
        }
        let asset = if (stage == StageKind::Enumeration
            && ENUM_CONTENT_TECHNIQUES.contains(&technique.as_str()))
            || (stage == StageKind::VulnTriage
                && VULN_TRIAGE_TECHNIQUES.contains(&technique.as_str()))
        {
            golish_pentest_domain::canonical_web_origin(asset)
                .map(|origin| origin.key)
                .unwrap_or_else(|| asset.clone())
        } else {
            asset.clone()
        };
        let outcome = if outcome == "partial" {
            "error".to_string()
        } else {
            outcome.clone()
        };
        technique_outcome_to_fact(asset, technique.clone(), outcome, *evidence_id)
    }));
}

pub fn eas_cidr_range_outcome_is_self_corroborating(
    asset: &str,
    technique: &str,
    source: Option<&str>,
) -> bool {
    matches!(
        technique,
        golish_db::repo::coverage_truth::TECH_EAS_LIVENESS
            | golish_db::repo::coverage_truth::TECH_EAS_PORT
    ) && source == Some("eas_discover_ports")
        && golish_pentest_domain::canonical_asset_key(asset)
            .is_some_and(|key| key.class == golish_pentest_domain::AssetClass::Cidr)
}

/// Enumeration and Vuln Triage completion is origin-keyed and comes only from
/// current, org-bound technique outcome rows; provider/source terminal rows are
/// host/source-level compatibility data and cannot close their cells.
pub fn stage_accepts_source_query_completion(stage: StageKind) -> bool {
    !matches!(
        stage,
        StageKind::TargetIntel | StageKind::Enumeration | StageKind::VulnTriage
    )
}

/// EAS, Enumeration, and Vuln Triage have strict freshness contracts: without
/// a concrete stage start, presence-only rows from an earlier attempt in the
/// same chat session must not be projected. Other stages retain their fallback.
pub fn stage_accepts_outcome_projection(stage: StageKind, has_freshness_cutoff: bool) -> bool {
    if stage == StageKind::TargetIntel {
        return false;
    }
    !matches!(
        stage,
        StageKind::ExternalAttackSurface | StageKind::Enumeration | StageKind::VulnTriage
    ) || has_freshness_cutoff
}

/// Gate-wide expected-technique contract. Exact Enumeration origins always carry
/// all four content axes, even when their owning target row is typed as IP/CIDR.
pub fn stage_gate_expected_techniques(
    stage: StageKind,
    target_types: &[String],
) -> Option<Vec<String>> {
    if stage == StageKind::TargetIntel {
        return Some(Vec::new());
    }
    if stage == StageKind::Enumeration {
        return Some(ENUM_CONTENT_TECHNIQUES.map(str::to_string).to_vec());
    }
    super::expected_techniques_for_target_types(stage, target_types)
}

pub fn eas_service_not_applicable_from_port_outcomes(
    rows: &[TechniqueOutcomeFact],
) -> Vec<(String, String)> {
    rows.iter()
        .filter(|row| {
            row.technique == TECH_EAS_PORT
                && matches!(row.outcome.as_str(), "empty" | "not_applicable")
        })
        .map(|row| (row.asset.clone(), TECH_EAS_SERVICE_FP.to_string()))
        .collect()
}

fn vuln_outcome_source_is_trusted(technique: &str, source: Option<&str>) -> bool {
    if technique == "GOLISH-NDAY" {
        source == Some("vuln_nuclei_fingerprint_targeted")
    } else if technique == "WSTG-ATHN-04" {
        source == Some(TRUSTED_VULN_ANONYMOUS_ACCESS_SOURCE)
    } else if VULN_TRIAGE_TECHNIQUES.contains(&technique) {
        source == Some("vuln_nuclei_general")
    } else {
        false
    }
}

pub fn vuln_not_applicable_from_outcomes(rows: &[TechniqueOutcomeFact]) -> Vec<(String, String)> {
    rows.iter()
        .filter(|row| {
            row.outcome == "not_applicable"
                && row.evidence_id > 0
                && vuln_outcome_source_is_trusted(&row.technique, row.source.as_deref())
        })
        .filter_map(|row| {
            let origin = golish_pentest_domain::canonical_web_origin(&row.asset)?;
            Some((origin.key, row.technique.clone()))
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

/// Select the immutable organization denominator used by specialist closeout.
///
/// Investigation is special: its closure publication is produced from the
/// operation-frozen StageUnit set and is therefore the only admissible scope
/// authority. The engagement organization subtree is a mutable catalog view
/// and may include subsidiaries that a root-only operation excluded. Other
/// specialist stages retain the legacy engagement-subtree behavior until they
/// expose an equivalent immutable publication authority.
pub fn specialist_completion_scope_ids(
    stage: StageKind,
    investigation_completion_authority: Option<&[(Uuid, DateTime<Utc>)]>,
    engagement_root: Option<Uuid>,
    engagement_subtree_ids: Option<Vec<Uuid>>,
    legacy_in_scope_ids: Vec<Uuid>,
) -> Vec<Uuid> {
    if stage == StageKind::Investigation {
        return investigation_completion_authority
            .unwrap_or_default()
            .iter()
            .map(|(organization_id, _)| *organization_id)
            .collect();
    }
    fanout_completion_scope_ids(engagement_root, engagement_subtree_ids, legacy_in_scope_ids)
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

fn current_wave_gate_error(
    current_wave: Option<&StageAssetWaveView>,
    org_id: Option<Uuid>,
    stage: StageKind,
) -> Option<String> {
    let wave = current_wave?;
    if let Err(error) = wave.validate_membership() {
        return Some(format!("invalid current asset wave: {error}"));
    }
    if wave.stage_kind != stage.as_str() {
        return Some(format!(
            "current asset wave stage '{}' does not match gate stage '{}'",
            wave.stage_kind,
            stage.as_str()
        ));
    }
    if org_id.is_some_and(|org_id| wave.organization_id != org_id) {
        return Some(format!(
            "current asset wave organization {} does not match gate organization {:?}",
            wave.organization_id, org_id
        ));
    }
    None
}

/// Coverage-axis cutoff for legacy matrix stages. Target Intel no longer owns a
/// coverage denominator and is finalized exclusively by IntelGoalV1 review.
pub fn stage_asset_axis_cutoff(
    _stage: StageKind,
    _stage_started_at: Option<DateTime<Utc>>,
    wave_cutoff: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    wave_cutoff
}

fn stage_outcome_run_id(
    stage: StageKind,
    operation_id: Option<Uuid>,
    session_id: &str,
) -> Option<String> {
    if stage == StageKind::VulnTriage {
        operation_id.map(|operation_id| operation_id.to_string())
    } else {
        Some(session_id.to_string())
    }
}

/// 对 `org_id` 的某 stage 交付跑一次注入了该 org DB 真值的权威 gate。
///
/// 复用 orchestrator stage-close 的同一批 repo 查询（`in_scope_assets` /
/// `in_scope_typed_assets` / `evidence_facts_for_session` / `db_truth_facts`）+ 同一个
/// `validate_stage_gate_with_context`，把判定按 org 隔离。先做一次 fabricated-ref 存在性
/// 兜底（与 execute.rs `enforce_evidence_existence` 同义）。Scoping 的企业身份确认同样
/// 必须引用真实账本证据，不能绕过这条通用真实性边界。
///
/// 失败回退：spec 加载失败 → 直接 Block（fail-closed，配置坏不该放行）。repo 缺失/
/// DB 错由调用方（stage_run）决定回退策略，本函数要求传入可用 repo。
pub async fn evaluate_org_stage_gate(
    repo: &dyn DbRepoProvider,
    operation_id: Option<Uuid>,
    org_id: Option<Uuid>,
    session_id: &str,
    stage: StageKind,
    deliverable: &StageDeliverable,
    wave_cutoff: Option<DateTime<Utc>>,
    current_wave: Option<&StageAssetWaveView>,
) -> GateResult {
    if stage == StageKind::TargetIntel {
        return GateResult::block(
            vec![
                "Target Intel legacy coverage Gate is retired; IntelGoalV1 independent review and DB finalization are required"
                    .to_string(),
            ],
            Default::default(),
        );
    }
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
    let tool_truth_contract = match operation_id {
        Some(operation_id) => match repo.tool_truth_contract(operation_id).await {
            Ok(contract) => contract,
            Err(error) => {
                return GateResult::block(
                    vec![format!("Tool Truth contract query failed: {error}")],
                    Default::default(),
                )
            }
        },
        None => ToolTruthContract::LegacyV1,
    };
    if let Some(error) = current_wave_gate_error(current_wave, org_id, stage) {
        return GateResult::block(vec![error], Default::default());
    }
    if stage == StageKind::Cleanup {
        let (Some(operation_id), Some(organization_id)) = (operation_id, org_id) else {
            return GateResult::block(
                vec!["cleanup gate requires exact operation and organization identity".to_string()],
                Default::default(),
            );
        };
        let cleanup = match repo
            .cleanup_closeout_gate(operation_id, organization_id)
            .await
        {
            Ok(cleanup) => cleanup,
            Err(error) => {
                return GateResult::block(
                    vec![format!(
                        "cleanup authoritative closeout query failed: {error}"
                    )],
                    Default::default(),
                )
            }
        };
        if !cleanup.allows_closeout() {
            return GateResult::block(
                vec![format!(
                    "cleanup closeout blocked by DB truth: missing_obligations={}, nonterminal_obligations={}, undisclosed_residuals={}, invalid_terminal_truth={}",
                    cleanup.missing_obligation_count,
                    cleanup.nonterminal_obligation_count,
                    cleanup.undisclosed_residual_count,
                    cleanup.invalid_terminal_truth_count,
                )],
                Default::default(),
            );
        }
    }
    let effective_cutoff = current_wave.map(|wave| wave.started_at).or(wave_cutoff);
    let effective_wave_cutoff = (spec.asset_wave_barrier || current_wave.is_some())
        .then_some(effective_cutoff)
        .flatten();
    let asset_axis_cutoff = stage_asset_axis_cutoff(stage, effective_cutoff, effective_wave_cutoff);
    let freshness_cutoff = spec.freshness_window.then_some(effective_cutoff).flatten();
    if matches!(stage, StageKind::Enumeration | StageKind::VulnTriage) {
        if session_id.trim().is_empty() {
            return GateResult::block(
                vec![format!(
                    "{} gate requires a non-empty current run/session id",
                    stage.as_str()
                )],
                Default::default(),
            );
        }
        if stage == StageKind::VulnTriage && operation_id.is_none() {
            return GateResult::block(
                vec![
                    "vuln_triage gate requires a trusted operation id for outcome identity"
                        .to_string(),
                ],
                Default::default(),
            );
        }
        if freshness_cutoff.is_none() {
            return GateResult::block(
                vec![format!(
                    "{} gate requires the current stage_started_at freshness cutoff; refusing an unscoped or stale denominator",
                    stage.as_str()
                )],
                Default::default(),
            );
        }
    }
    if stage == StageKind::Enumeration {
        if let (Some(operation_id), Some(organization_id)) = (operation_id, org_id) {
            let lifecycle = match repo
                .enumeration_occurrence_gate_snapshot(operation_id, organization_id)
                .await
            {
                Ok(lifecycle) => lifecycle,
                Err(error) => {
                    return GateResult::block(
                        vec![format!(
                            "enumeration occurrence lifecycle query failed: {error}"
                        )],
                        Default::default(),
                    )
                }
            };
            if let Some(lifecycle) = lifecycle {
                if lifecycle.enforces_closeout && !lifecycle.allows_closeout() {
                    return GateResult::block(
                        vec![format!(
                            "enumeration v2 exact-subject closure is nonterminal: frozen_subjects={}, coverage_receipts={}, missing_coverage_receipts={}, invalid_coverage_receipts={}, closure_graph_drift={} (terminal_residual_occurrences={}, stage_execution_id={}, stage_run_unit_id={})",
                            lifecycle.frozen_subject_count,
                            lifecycle.coverage_receipt_count,
                            lifecycle.missing_coverage_receipt_count,
                            lifecycle.invalid_coverage_receipt_count,
                            lifecycle.closure_graph_drift_count,
                            lifecycle.residual_occurrence_count,
                            lifecycle.stage_execution_id,
                            lifecycle.stage_run_unit_id,
                        )],
                        Default::default(),
                    );
                }
                tracing::info!(
                    target: "harness::hook",
                    operation_id = %operation_id,
                    organization_id = %organization_id,
                    stage_execution_id = %lifecycle.stage_execution_id,
                    stage_run_unit_id = %lifecycle.stage_run_unit_id,
                    enforces_closeout = lifecycle.enforces_closeout,
                    frozen_subjects = lifecycle.frozen_subject_count,
                    coverage_receipts = lifecycle.coverage_receipt_count,
                    missing_coverage_receipts = lifecycle.missing_coverage_receipt_count,
                    invalid_coverage_receipts = lifecycle.invalid_coverage_receipt_count,
                    closure_graph_drift = lifecycle.closure_graph_drift_count,
                    terminal_residual_occurrences = lifecycle.residual_occurrence_count,
                    "evaluated DB-authoritative Enumeration exact-subject closure"
                );
            }
        }
    }
    let wave_asset_override = current_wave.map(|wave| wave.asset_values.clone());
    let wave_target_id_override = current_wave.map(|wave| wave.target_ids.clone());
    // Once an engagement org is bound, EAS coverage is a DB-truth contract. A
    // failed read must never collapse to an empty Vec and silently re-enable a
    // model-authored axis. Target Intel has no coverage denominator here.
    let authoritative_db_gate = org_id.is_some() && stage == StageKind::ExternalAttackSurface;

    // 1) fabricated-ref 兜底。包括 Scoping 在内的所有阶段都必须引用真实账本证据。
    let cited: Vec<i64> = deliverable
        .evidence_refs
        .iter()
        .map(|e| e.as_i64())
        .collect();
    if !cited.is_empty() {
        if let Ok(existing) = repo.evidence_existing_ids(&cited).await {
            let fabricated = fabricated_evidence_ids(&cited, &existing);
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

    // 2) 资产轴 + 类型（org 隔离）。空资产集 → 不注入（gate 回退自报，coverage_complete
    //    自带「空矩阵但声明了期望技术 → BLOCK」保护）。
    let mut in_scope_assets = match wave_asset_override.clone() {
        Some(assets) => assets,
        None => match asset_axis_cutoff {
            Some(cutoff) => match repo.in_scope_assets_created_before(org_id, cutoff).await {
                Ok(assets) => assets,
                Err(error) if authoritative_db_gate => {
                    return GateResult::block(
                        vec![format!(
                            "{} authoritative asset-axis query failed: {error}",
                            stage.as_str()
                        )],
                        Default::default(),
                    )
                }
                Err(_) => Vec::new(),
            },
            None => match repo.in_scope_assets(org_id).await {
                Ok(assets) => assets,
                Err(error) if authoritative_db_gate => {
                    return GateResult::block(
                        vec![format!(
                            "{} authoritative asset-axis query failed: {error}",
                            stage.as_str()
                        )],
                        Default::default(),
                    )
                }
                Err(_) => Vec::new(),
            },
        },
    };
    let mut typed_assets = match repo.in_scope_typed_assets(org_id).await {
        Ok(assets) => assets,
        Err(error) if authoritative_db_gate => {
            return GateResult::block(
                vec![format!(
                    "{} authoritative typed-asset query failed: {error}",
                    stage.as_str()
                )],
                Default::default(),
            )
        }
        Err(_) => Vec::new(),
    };
    if !in_scope_assets.is_empty() {
        let current_axis: std::collections::HashSet<&str> =
            in_scope_assets.iter().map(String::as_str).collect();
        typed_assets.retain(|(asset, _)| current_axis.contains(asset.as_str()));
    }
    // Dead-asset denominator exclusion (design 2026-07-02-dead-asset-liveness-
    // state §5.2): a stage that opts in (`skip_dead_assets`, enumeration onward —
    // never EAS) drops assets EAS confirmed dead so a dead host no longer forces a
    // probe / `checked_empty`. An all-dead set is a real authoritative zero
    // denominator; `authoritative_in_scope_assets(Some([]))` preserves that
    // distinction from a failed/missing asset lookup. Only `'dead'` is dropped
    // (`'unreachable'` may be transient — see `dead_asset_values`).
    if spec.skip_dead_assets && !in_scope_assets.is_empty() {
        let dead: std::collections::HashSet<String> = repo
            .dead_asset_values(org_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
        if !dead.is_empty() {
            let survivors: Vec<String> = in_scope_assets
                .iter()
                .filter(|asset| !dead.contains(*asset))
                .cloned()
                .collect();
            let removed = in_scope_assets.len() - survivors.len();
            if removed > 0 {
                tracing::info!(
                    target: "harness::hook",
                    stage = stage.as_str(),
                    org_id = ?org_id,
                    removed,
                    "excluded confirmed-dead assets from coverage denominator"
                );
            }
            in_scope_assets = survivors;
            let alive: std::collections::HashSet<&str> =
                in_scope_assets.iter().map(String::as_str).collect();
            typed_assets.retain(|(asset, _)| alive.contains(asset.as_str()));
        }
    }
    let web_capable_assets: Vec<String> = match stage {
        StageKind::Enumeration if spec.enum_ip_web_coverage => repo
            .enumeration_web_capable_assets(org_id)
            .await
            .unwrap_or_default(),
        StageKind::ExternalAttackSurface => {
            match repo
                .eas_web_capable_assets(org_id, freshness_cutoff)
                .await
            {
                Ok(assets) => assets,
                Err(error) => {
                    return GateResult::block(
                        vec![format!(
                            "external_attack_surface authoritative web-capability query failed: {error}"
                        )],
                        Default::default(),
                    )
                }
            }
        }
        _ => Vec::new(),
    };
    // EAS PORT-empty outcomes may deterministically close SERVICE below. The
    // Enumeration/Vuln denominator is already exact HTTP(S) origins, so raw-host
    // context must never synthesize origin-level not_applicable cells.
    let mut not_applicable_coverage: Vec<(String, String)> = Vec::new();
    let mut authoritative_coverage_axis = false;
    if matches!(stage, StageKind::Enumeration | StageKind::VulnTriage) {
        let Some(oid) = org_id else {
            return GateResult::block(
                vec![format!(
                    "{} gate requires an organization-bound exact-origin coverage snapshot",
                    stage.as_str()
                )],
                Default::default(),
            );
        };
        let snapshot = match repo
            .stage_asset_coverage_for_operation(
                operation_id,
                oid,
                stage.as_str(),
                Some(session_id),
                freshness_cutoff,
                wave_target_id_override.clone(),
                wave_asset_override.clone(),
            )
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return GateResult::block(
                    vec![format!(
                        "{} exact-origin coverage snapshot failed: {error}",
                        stage.as_str()
                    )],
                    Default::default(),
                )
            }
        };
        match validated_exact_web_origin_axis_from_coverage_snapshot(
            &snapshot,
            stage,
            oid,
            Some(session_id),
        ) {
            Ok((assets, typed)) => {
                if stage == StageKind::VulnTriage {
                    match trusted_vuln_surface_not_applicable_from_snapshot(&snapshot) {
                        Ok(cells) => not_applicable_coverage.extend(cells),
                        Err(error) => {
                            return GateResult::block(
                                vec![format!(
                                    "vuln_triage surface applicability snapshot is invalid: {error}"
                                )],
                                Default::default(),
                            )
                        }
                    }
                }
                authoritative_coverage_axis = true;
                in_scope_assets = assets;
                typed_assets = typed;
            }
            Err(error) => {
                return GateResult::block(
                    vec![format!(
                        "{} exact-origin coverage snapshot is invalid: {error}",
                        stage.as_str()
                    )],
                    Default::default(),
                )
            }
        }
    }

    // 3) 证据事实：账本投影 + DB 业务表真值（Found）合并。
    let mut facts: Vec<EvidenceFact> = if stage == StageKind::ExternalAttackSurface {
        match (org_id, freshness_cutoff) {
            (Some(organization_id), Some(since)) => match repo
                .eas_evidence_facts_for_session_org_fresh(session_id, organization_id, since)
                .await
            {
                Ok(rows) => facts_from_rows(rows),
                Err(error) => {
                    return GateResult::block(
                        vec![format!(
                            "external_attack_surface authoritative evidence query failed: {error}"
                        )],
                        Default::default(),
                    )
                }
            },
            _ => Vec::new(),
        }
    } else if stage == StageKind::VulnTriage {
        // The Nuclei wrappers self-land fresh, org-bound, run-scoped technique
        // outcomes. Session-wide legacy shell evidence must not close an exact
        // origin cell for a sibling organization or a previous attempt.
        Vec::new()
    } else {
        match repo.evidence_facts_for_session(session_id).await {
            Ok(rows) => facts_from_rows(rows),
            Err(error) if authoritative_db_gate => {
                return GateResult::block(
                    vec![format!(
                        "{} authoritative evidence query failed: {error}",
                        stage.as_str()
                    )],
                    Default::default(),
                )
            }
            Err(_) => Vec::new(),
        }
    };
    if !in_scope_assets.is_empty() && stage != StageKind::VulnTriage {
        match repo
            .db_truth_facts(org_id, &in_scope_assets, freshness_cutoff)
            .await
        {
            Ok(truth) => facts.extend(db_truth_to_facts(truth)),
            Err(error) if authoritative_db_gate => {
                return GateResult::block(
                    vec![format!(
                        "{} authoritative business-truth query failed: {error}",
                        stage.as_str()
                    )],
                    Default::default(),
                )
            }
            Err(_) => {}
        }
    }

    // #4/E3 (设计 2026-06-23-technique-outcomes-provenance)：从当前
    // technique_outcomes 投影 facts（per-org fan-out gate；与 execute.rs 主路径同源）。
    // Enumeration/Vuln 会先清掉兼容 facts，且缺 freshness cutoff 时
    // fail-closed；其他 stage 保持 additive union。org=None = 跳过。
    let outcome_run_id = stage_outcome_run_id(stage, operation_id, session_id);
    let outcome_rows = match (org_id, outcome_run_id.as_deref()) {
        (Some(oid), Some(outcome_run_id))
            if stage_accepts_outcome_projection(stage, freshness_cutoff.is_some()) =>
        {
            if stage == StageKind::VulnTriage {
                repo.technique_outcome_facts_fresh_with_evidence_session(
                    oid,
                    outcome_run_id,
                    session_id,
                    freshness_cutoff,
                )
                .await
            } else {
                repo.technique_outcome_facts_fresh(oid, outcome_run_id, freshness_cutoff)
                    .await
            }
        }
        _ => Vec::new(),
    };
    let eas_origin_barrier = if stage == StageKind::ExternalAttackSurface {
        let Some(organization_id) = org_id else {
            return GateResult::block(
                vec![
                    "external_attack_surface gate requires an organization-bound exact-origin denominator"
                        .to_string(),
                ],
                Default::default(),
            );
        };
        let Some(since) = freshness_cutoff else {
            return GateResult::block(
                vec![
                    "external_attack_surface gate requires the current stage_started_at freshness cutoff for exact Web Origins"
                        .to_string(),
                ],
                Default::default(),
            );
        };
        let required = match repo
            .eas_required_web_origins(organization_id, since, wave_target_id_override.clone())
            .await
        {
            Ok(required) => required,
            Err(error) => {
                return GateResult::block(
                    vec![format!(
                        "external_attack_surface exact-origin denominator query failed: {error}"
                    )],
                    Default::default(),
                )
            }
        };
        let completed = completed_from_guarded_outcomes(&outcome_rows, &facts);
        Some((required, completed))
    } else {
        None
    };
    if stage == StageKind::ExternalAttackSurface {
        not_applicable_coverage
            .extend(eas_service_not_applicable_from_port_outcomes(&outcome_rows));
    } else if stage == StageKind::VulnTriage {
        not_applicable_coverage.extend(vuln_not_applicable_from_outcomes(&outcome_rows));
    }
    apply_technique_outcome_rows_for_contract(
        tool_truth_contract,
        stage,
        &mut facts,
        &outcome_rows,
    );

    let source_queries = match (stage_accepts_source_query_completion(stage), org_id) {
        (true, Some(oid)) => match repo.source_query_facts(oid, session_id).await {
            Ok(rows) => rows,
            Err(error) if authoritative_db_gate => {
                return GateResult::block(
                    vec![format!(
                        "{} authoritative source-query read failed: {error}",
                        stage.as_str()
                    )],
                    Default::default(),
                )
            }
            Err(_) => Vec::new(),
        },
        _ => Vec::new(),
    };

    // 统一组装入口（设计 2026-06-23-unified-gate-context-builder）：归一/合并收口到
    // GateContextBuilder。expected_techniques=None ⇒ 回退 spec.expected_techniques。
    let mut ctx_builder = GateContextBuilder::new()
        .typed_assets(typed_assets)
        .web_capable_assets(web_capable_assets)
        .not_applicable_coverage(not_applicable_coverage)
        .extend_evidence_facts(facts)
        .extend_source_queries(source_queries)
        .expected_techniques(stage_gate_expected_techniques(stage, &[]));
    if let Some((required, completed)) = eas_origin_barrier {
        ctx_builder = ctx_builder.eas_web_origin_barrier(required, completed);
    }
    let ctx = if authoritative_coverage_axis || authoritative_db_gate {
        ctx_builder.authoritative_in_scope_assets(Some(in_scope_assets))
    } else {
        ctx_builder.in_scope_assets(in_scope_assets)
    }
    .build();

    let legacy_result = validate_stage_gate_with_context(deliverable, &spec, None, None, &ctx);
    if tool_truth_contract.writes_receipts() {
        if let (Some(operation_id), Some(organization_id)) = (operation_id, org_id) {
            if let Err(error) = repo
                .tool_truth_record_shadow_assessment(RecordToolTruthShadowAssessment {
                    operation_id,
                    organization_id,
                    stage_kind: stage.as_str().to_string(),
                    stage_asset_wave_id: current_wave.map(|wave| wave.id),
                    legacy_allowed: legacy_result.allowed,
                })
                .await
            {
                tracing::error!(
                    operation_id = %operation_id,
                    organization_id = %organization_id,
                    stage = stage.as_str(),
                    error = %error,
                    "tool-truth shadow assessment failed after legacy Gate decision"
                );
            }
        }
    }
    legacy_result
}

fn fabricated_evidence_ids(cited: &[i64], existing: &std::collections::HashSet<i64>) -> Vec<i64> {
    cited
        .iter()
        .copied()
        .filter(|id| !existing.contains(id))
        .collect()
}

#[cfg(test)]
fn enumeration_eas_live_web_worklist(
    assets: &[String],
    typed_assets: &[(String, String)],
    truth_rows: &[(String, String)],
    web_capable_assets: &[String],
) -> Option<BTreeSet<String>> {
    let live_liveness_keys: BTreeSet<String> = truth_rows
        .iter()
        .filter(|(_, technique)| technique == TECH_EAS_LIVENESS)
        .map(|(asset, _)| eas_liveness_lookup_key(asset))
        .collect();
    let web_capable_assets: BTreeSet<&str> =
        web_capable_assets.iter().map(String::as_str).collect();
    if live_liveness_keys.is_empty() && web_capable_assets.is_empty() {
        return None;
    }

    let type_by_asset: HashMap<&str, &str> = typed_assets
        .iter()
        .map(|(asset, target_type)| (asset.as_str(), target_type.as_str()))
        .collect();
    let worklist: BTreeSet<String> = assets
        .iter()
        .filter(|asset| {
            let class = AssetClass::classify(type_by_asset.get(asset.as_str()).copied(), asset);
            (matches!(class, AssetClass::Domain | AssetClass::Url)
                && live_liveness_keys.contains(&eas_liveness_lookup_key(asset)))
                || (matches!(class, AssetClass::Ip | AssetClass::Cidr)
                    && web_capable_assets.contains(asset.as_str()))
        })
        .cloned()
        .collect();

    (!worklist.is_empty()).then_some(worklist)
}

pub fn exact_web_origin_axis_from_coverage_snapshot(
    snapshot: &serde_json::Value,
) -> ExactWebOriginCoverageAxis {
    let mut assets = Vec::new();
    let mut typed_assets = Vec::new();
    for row in snapshot
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        // The read model intentionally keeps supplemental-wave rows visible as
        // `next_wave_pending`, but the current gate/submit/pass-token denominator
        // must contain only the active wave. Otherwise preflight can say ready
        // while the close gate immediately re-adds the deferred origins.
        if row
            .get("coverage")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|cells| {
                cells.iter().any(|cell| {
                    cell.get("state").and_then(serde_json::Value::as_str)
                        == Some("next_wave_pending")
                })
            })
        {
            continue;
        }
        if row
            .get("exact_web_origin")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        {
            continue;
        }
        let value = row
            .get("value")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim();
        let Some(origin) = golish_pentest_domain::canonical_web_origin(value) else {
            continue;
        };
        if assets.iter().any(|asset| asset == &origin.key) {
            continue;
        }
        assets.push(origin.key.clone());
        typed_assets.push((origin.key, "url".to_string()));
    }
    (assets, typed_assets)
}

/// Backward-compatible Enumeration name for callers that consume the same
/// shared exact-origin axis outside the final gate.
pub fn enumeration_axis_from_coverage_snapshot(
    snapshot: &serde_json::Value,
) -> EnumerationCoverageAxis {
    exact_web_origin_axis_from_coverage_snapshot(snapshot)
}

/// Validate a trusted exact-origin stage snapshot before deriving its axis.
/// Worklist preview and final per-org gate must reject the same stage/org/run
/// mismatch or malformed assets payload.
pub fn validated_exact_web_origin_axis_from_coverage_snapshot(
    snapshot: &serde_json::Value,
    expected_stage: StageKind,
    expected_org_id: Uuid,
    expected_session_id: Option<&str>,
) -> Result<ExactWebOriginCoverageAxis, &'static str> {
    if snapshot.get("stage").and_then(serde_json::Value::as_str) != Some(expected_stage.as_str()) {
        return Err("stage does not match the requested exact-origin stage");
    }
    if snapshot
        .get("organization_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        != Some(expected_org_id)
    {
        return Err("organization_id does not match the gate organization");
    }
    if snapshot.get("session_id").is_none()
        || snapshot
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            != expected_session_id
    {
        return Err("session_id does not match the current gate run");
    }
    if !snapshot
        .get("assets")
        .is_some_and(serde_json::Value::is_array)
    {
        return Err("assets is not an authoritative array");
    }
    Ok(exact_web_origin_axis_from_coverage_snapshot(snapshot))
}

/// Extract only server-generated deterministic N/A cells from the trusted Vuln
/// coverage snapshot. Model-authored deliverables and ordinary outcome rows do
/// not carry this source+details authority pair.
pub fn trusted_vuln_surface_not_applicable_from_snapshot(
    snapshot: &serde_json::Value,
) -> Result<Vec<(String, String)>, &'static str> {
    let mut cells = Vec::new();
    for row in snapshot
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        if row
            .get("exact_web_origin")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        {
            continue;
        }
        let value = row
            .get("value")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let Some(origin) = golish_pentest_domain::canonical_web_origin(value) else {
            return Err("surface applicability row is not a canonical exact origin");
        };
        for cell in row
            .get("coverage")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            if cell.get("state").and_then(serde_json::Value::as_str) != Some("not_applicable")
                || cell.get("source").and_then(serde_json::Value::as_str)
                    != Some("enumeration_surface_manifest")
            {
                continue;
            }
            let technique = cell
                .get("technique")
                .and_then(serde_json::Value::as_str)
                .ok_or("surface applicability cell has no technique")?;
            if !matches!(
                technique,
                "WSTG-INPV-05" | "WSTG-INPV-01" | "WSTG-INPV-12" | "WSTG-ATHN-04" | "GOLISH-NDAY"
            ) {
                return Err("surface applicability source used for an unsupported technique");
            }
            if cell
                .get("details")
                .and_then(|details| details.get("authority"))
                .and_then(serde_json::Value::as_str)
                != Some("enumeration_surface_manifest")
            {
                return Err("surface applicability cell is missing backend authority details");
            }
            cells.push((origin.key.clone(), technique.to_string()));
        }
    }
    cells.sort();
    cells.dedup();
    Ok(cells)
}

/// Validate the trusted Enumeration snapshot envelope before deriving its exact
/// origin axis. Submit preview and final per-org gate must reject the same
/// stage/org/session mismatch or malformed assets payload.
pub fn validated_enumeration_axis_from_coverage_snapshot(
    snapshot: &serde_json::Value,
    expected_org_id: Uuid,
    expected_session_id: Option<&str>,
) -> Result<EnumerationCoverageAxis, &'static str> {
    validated_exact_web_origin_axis_from_coverage_snapshot(
        snapshot,
        StageKind::Enumeration,
        expected_org_id,
        expected_session_id,
    )
}

#[cfg(test)]
fn eas_liveness_lookup_key(asset: &str) -> String {
    super::evidence_facts::eas_liveness_asset_key(asset).unwrap_or_else(|| asset.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::types::{CoverageGapAction, HarnessRecoveryActions};

    #[test]
    fn scoping_cited_evidence_has_no_fabricated_ref_exemption() {
        let existing = std::collections::HashSet::from([11_i64, 13_i64]);

        assert_eq!(
            fabricated_evidence_ids(&[11, 12, 13, 14], &existing),
            vec![12, 14],
            "Scoping uses this same stage-agnostic filter before any stage-specific gate rules",
        );
    }

    #[test]
    fn target_intel_has_no_legacy_asset_axis_cutoff() {
        let stage_started_at = chrono::Utc::now();
        let wave_started_at = stage_started_at + chrono::Duration::minutes(1);

        assert_eq!(
            stage_asset_axis_cutoff(StageKind::TargetIntel, Some(stage_started_at), None,),
            None
        );
        assert_eq!(
            stage_asset_axis_cutoff(
                StageKind::ExternalAttackSurface,
                Some(stage_started_at),
                Some(wave_started_at),
            ),
            Some(wave_started_at)
        );
    }

    fn outcome_fact(
        asset: &str,
        technique: &str,
        state: &str,
        evidence_id: i64,
    ) -> TechniqueOutcomeFact {
        TechniqueOutcomeFact::new(asset, technique, state, evidence_id, None)
    }

    #[test]
    fn enumeration_gate_axis_uses_origin_rows_from_coverage_snapshot() {
        let snapshot = serde_json::json!({
            "stage": "enumeration",
            "assets": [
                {"value": "http://app.example.com:80/path", "target_type": "url", "exact_web_origin": true},
                {"value": "HTTPS://APP.EXAMPLE.COM:443/login", "target_type": "url", "exact_web_origin": true},
                {"value": "alive-but-bare.example.com", "target_type": "domain", "exact_web_origin": false},
                {"value": "222.186.129.58", "target_type": "ip", "exact_web_origin": false},
                {"value": "not a url", "target_type": "url", "exact_web_origin": true}
            ]
        });

        let (assets, typed_assets) = enumeration_axis_from_coverage_snapshot(&snapshot);

        assert_eq!(
            assets,
            vec![
                "http://app.example.com:80".to_string(),
                "https://app.example.com:443".to_string(),
            ]
        );
        assert_eq!(
            typed_assets,
            vec![
                ("http://app.example.com:80".to_string(), "url".to_string()),
                ("https://app.example.com:443".to_string(), "url".to_string()),
            ]
        );
    }

    #[test]
    fn enumeration_org_gate_axis_keeps_exact_ip_origin_four_axis_applicable() {
        let origin = "https://203.0.113.10:443";
        let snapshot = serde_json::json!({
            "stage": "enumeration",
            "assets": [{
                "value": origin,
                "target_type": "url",
                "exact_web_origin": true,
                "coverage": []
            }]
        });

        let (assets, typed_assets) = enumeration_axis_from_coverage_snapshot(&snapshot);

        assert_eq!(assets, vec![origin.to_string()]);
        let class = AssetClass::classify(Some(&typed_assets[0].1), &assets[0]);
        assert_eq!(class, AssetClass::Ip, "URL-wrapped IP keeps host class");
        for technique in ENUM_CONTENT_TECHNIQUES {
            assert!(
                crate::harness::technique_resolver::technique_applies_web_aware(
                    StageKind::Enumeration,
                    class,
                    &assets[0],
                    technique,
                    false,
                ),
                "org gate exact-origin axis must retain {technique}"
            );
        }
    }

    #[test]
    fn enumeration_gate_axis_rejects_malformed_snapshot_instead_of_falling_back() {
        let org_id = Uuid::new_v4();
        let session_id = "run-current";
        let malformed = serde_json::json!({
            "stage": "enumeration",
            "organization_id": org_id,
            "session_id": session_id,
            "assets": null
        });
        assert!(validated_enumeration_axis_from_coverage_snapshot(
            &malformed,
            org_id,
            Some(session_id),
        )
        .is_err());

        let authoritative_empty = serde_json::json!({
            "stage": "enumeration",
            "organization_id": org_id,
            "session_id": session_id,
            "assets": []
        });
        assert_eq!(
            validated_enumeration_axis_from_coverage_snapshot(
                &authoritative_empty,
                org_id,
                Some(session_id)
            )
            .unwrap(),
            (Vec::new(), Vec::new())
        );

        let foreign_org = serde_json::json!({
            "stage": "enumeration",
            "organization_id": Uuid::new_v4(),
            "session_id": session_id,
            "assets": []
        });
        assert!(validated_enumeration_axis_from_coverage_snapshot(
            &foreign_org,
            org_id,
            Some(session_id)
        )
        .is_err());

        let stale_session = serde_json::json!({
            "stage": "enumeration",
            "organization_id": org_id,
            "session_id": "run-old",
            "assets": []
        });
        assert!(validated_enumeration_axis_from_coverage_snapshot(
            &stale_session,
            org_id,
            Some(session_id)
        )
        .is_err());
    }

    #[test]
    fn enumeration_gate_axis_excludes_next_wave_pending_origin_rows() {
        let snapshot = serde_json::json!({
            "stage": "enumeration",
            "assets": [
                {
                    "value": "https://current.example.com:443",
                    "target_type": "url",
                    "exact_web_origin": true,
                    "coverage": [
                        {"technique": "GOLISH-ENUM-JS", "state": "found"},
                        {"technique": "GOLISH-ENUM-DIR", "state": "empty"}
                    ]
                },
                {
                    "value": "https://next.example.com:443",
                    "target_type": "url",
                    "exact_web_origin": true,
                    "coverage": [
                        {"technique": "GOLISH-ENUM-JS", "state": "next_wave_pending"},
                        {"technique": "GOLISH-ENUM-DIR", "state": "next_wave_pending"}
                    ]
                }
            ]
        });

        let (assets, typed_assets) = enumeration_axis_from_coverage_snapshot(&snapshot);

        assert_eq!(assets, vec!["https://current.example.com:443".to_string()]);
        assert_eq!(
            typed_assets,
            vec![(
                "https://current.example.com:443".to_string(),
                "url".to_string()
            )]
        );
    }

    #[test]
    fn vuln_triage_final_gate_uses_exact_origin_snapshot_axis() {
        use golish_pentest::evidence_ledger::EvidenceAuditId;

        let org_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        let session_id = "vuln-run-current";
        let exact_origin = "https://app.example.com:443";
        let snapshot = serde_json::json!({
            "stage": "vuln_triage",
            "organization_id": org_id,
            "session_id": session_id,
            "assets": [{
                "target_id": target_id,
                "value": exact_origin,
                "target_type": "url",
                "exact_web_origin": true,
                "coverage": []
            }]
        });
        let (assets, typed_assets) = validated_exact_web_origin_axis_from_coverage_snapshot(
            &snapshot,
            StageKind::VulnTriage,
            org_id,
            Some(session_id),
        )
        .expect("trusted Vuln snapshot");
        assert_eq!(assets, vec![exact_origin.to_string()]);
        assert_eq!(
            typed_assets,
            vec![(exact_origin.to_string(), "url".to_string())]
        );

        let outcome_rows = VULN_TRIAGE_TECHNIQUES
            .iter()
            .enumerate()
            .map(|(index, technique)| {
                TechniqueOutcomeFact::new(
                    "HTTPS://APP.EXAMPLE.COM/login",
                    *technique,
                    if matches!(*technique, "GOLISH-NDAY" | "WSTG-ATHN-04") {
                        "not_applicable"
                    } else {
                        "empty"
                    },
                    100 + index as i64,
                    Some(
                        match *technique {
                            "GOLISH-NDAY" => "vuln_nuclei_fingerprint_targeted",
                            "WSTG-ATHN-04" => TRUSTED_VULN_ANONYMOUS_ACCESS_SOURCE,
                            _ => "vuln_nuclei_general",
                        }
                        .to_string(),
                    ),
                )
            })
            .collect::<Vec<_>>();
        let mut facts = vec![EvidenceFact {
            asset: "app.example.com".to_string(),
            technique: "WSTG-CONF-05".to_string(),
            outcome: EvidenceOutcome::Found,
            evidence_id: 7,
        }];
        let not_applicable_coverage = vuln_not_applicable_from_outcomes(&outcome_rows);
        apply_technique_outcome_rows(StageKind::VulnTriage, &mut facts, &outcome_rows);
        assert_eq!(facts.len(), VULN_TRIAGE_TECHNIQUES.len() - 2);
        assert!(facts.iter().all(|fact| fact.asset == exact_origin));
        assert!(facts.iter().all(|fact| fact.evidence_id >= 100));
        assert_eq!(
            not_applicable_coverage,
            vec![
                (exact_origin.to_string(), "WSTG-ATHN-04".to_string()),
                (exact_origin.to_string(), "GOLISH-NDAY".to_string()),
            ]
        );

        let evidence_refs = (100..110).map(EvidenceAuditId::new).collect::<Vec<_>>();
        let deliverable = StageDeliverable {
            stage_id: StageKind::VulnTriage.as_str().to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: Vec::new(),
            evidence_refs: evidence_refs.clone(),
            skipped_checks: Vec::new(),
            findings: Vec::new(),
            required_checks_done: Vec::new(),
            coverage: Vec::new(),
            candidates: Vec::new(),
            candidate_decisions: Vec::new(),
        };
        let context = GateContextBuilder::new()
            .typed_assets(typed_assets)
            .extend_evidence_facts(facts)
            .not_applicable_coverage(not_applicable_coverage)
            .expected_techniques(Some(VULN_TRIAGE_TECHNIQUES.map(str::to_string).to_vec()))
            .authoritative_in_scope_assets(Some(assets))
            .build();
        let spec = load_embedded_stage_spec(StageKind::VulnTriage).unwrap();

        let result = validate_stage_gate_with_context(&deliverable, &spec, None, None, &context);

        assert!(
            result.allowed,
            "exact-origin Nuclei outcomes should close the final Vuln gate: {:?}",
            result.reasons
        );
    }

    #[test]
    fn vuln_surface_snapshot_only_trusts_backend_applicability_cells() {
        let snapshot = serde_json::json!({
            "assets": [{
                "value": "https://app.example.com:443",
                "exact_web_origin": true,
                "coverage": [{
                    "technique": "WSTG-INPV-05",
                    "state": "not_applicable",
                    "source": "enumeration_surface_manifest",
                    "details": {"authority": "enumeration_surface_manifest"}
                }, {
                    "technique": "WSTG-INPV-01",
                    "state": "not_applicable",
                    "source": "model_claim",
                    "details": {"authority": "enumeration_surface_manifest"}
                }]
            }]
        });

        assert_eq!(
            trusted_vuln_surface_not_applicable_from_snapshot(&snapshot)
                .expect("trusted backend cell"),
            vec![(
                "https://app.example.com:443".to_string(),
                "WSTG-INPV-05".to_string()
            )]
        );

        let forged = serde_json::json!({
            "assets": [{
                "value": "https://app.example.com:443",
                "exact_web_origin": true,
                "coverage": [{
                    "technique": "WSTG-INPV-05",
                    "state": "not_applicable",
                    "source": "enumeration_surface_manifest",
                    "details": {}
                }]
            }]
        });
        assert!(trusted_vuln_surface_not_applicable_from_snapshot(&forged).is_err());
    }

    #[test]
    fn vuln_triage_outcome_run_identity_uses_operation_not_chat_session() {
        let operation_id = Uuid::new_v4();
        let chat_session_id = "chat-session-is-not-the-operation";
        let operation_run_id = operation_id.to_string();

        assert_eq!(
            stage_outcome_run_id(StageKind::VulnTriage, Some(operation_id), chat_session_id,)
                .as_deref(),
            Some(operation_run_id.as_str())
        );
        assert_eq!(
            stage_outcome_run_id(StageKind::Enumeration, Some(operation_id), chat_session_id)
                .as_deref(),
            Some(chat_session_id)
        );
        assert_eq!(
            stage_outcome_run_id(StageKind::VulnTriage, None, chat_session_id),
            None,
            "Vuln must fail closed without a trusted operation identity"
        );
    }

    #[test]
    fn vuln_not_applicable_requires_exact_origin_real_evidence_and_targeted_source() {
        let valid = TechniqueOutcomeFact::new(
            "https://app.example.com/login",
            "GOLISH-NDAY",
            "not_applicable",
            91,
            Some("vuln_nuclei_fingerprint_targeted".to_string()),
        );
        assert_eq!(
            vuln_not_applicable_from_outcomes(std::slice::from_ref(&valid)),
            vec![(
                "https://app.example.com:443".to_string(),
                "GOLISH-NDAY".to_string(),
            )]
        );

        for forged in [
            TechniqueOutcomeFact::new(
                &valid.asset,
                &valid.technique,
                &valid.outcome,
                0,
                valid.source.clone(),
            ),
            TechniqueOutcomeFact::new(
                &valid.asset,
                &valid.technique,
                &valid.outcome,
                valid.evidence_id,
                Some("vuln_nuclei_general".to_string()),
            ),
            TechniqueOutcomeFact::new(
                "app.example.com",
                &valid.technique,
                &valid.outcome,
                valid.evidence_id,
                valid.source.clone(),
            ),
        ] {
            assert!(vuln_not_applicable_from_outcomes(&[forged]).is_empty());
        }
    }

    #[test]
    fn vuln_anonymous_access_not_applicable_requires_its_dedicated_wrapper() {
        let exact_origin = "https://app.example.com:443";
        let valid = TechniqueOutcomeFact::new(
            exact_origin,
            "WSTG-ATHN-04",
            "not_applicable",
            92,
            Some("vuln_probe_anonymous_access".to_string()),
        );

        assert_eq!(
            vuln_not_applicable_from_outcomes(std::slice::from_ref(&valid)),
            vec![(
                exact_origin.to_string(),
                "WSTG-ATHN-04".to_string(),
            )],
            "the final gate must consume the same dedicated-wrapper truth as the worklist read model"
        );
        assert!(
            vuln_not_applicable_from_outcomes(&[TechniqueOutcomeFact::new(
                exact_origin,
                "WSTG-ATHN-04",
                "not_applicable",
                92,
                Some("vuln_nuclei_general".to_string()),
            )])
            .is_empty()
        );
    }

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
                suggested_capabilities: Vec::new(),
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
            ("a.com".into(), "GOLISH-ENUM-JS".into(), "partial".into(), 9),
        ]);
        assert_eq!(f.len(), 2, "partial evidence is never a terminal fact");
        assert_eq!(f[0].outcome, EvidenceOutcome::Found);
        assert_eq!(f[1].outcome, EvidenceOutcome::Empty);
        assert_eq!(f[0].evidence_id, 7);
    }

    #[test]
    fn enumeration_nonterminal_outcome_overrides_historical_found_fact() {
        let mut facts = vec![EvidenceFact {
            asset: "https://app.example.com:443".to_string(),
            technique: "GOLISH-ENUM-JS".to_string(),
            outcome: EvidenceOutcome::Found,
            evidence_id: 41,
        }];
        let rows = vec![outcome_fact(
            "https://app.example.com:443",
            "GOLISH-ENUM-JS",
            "partial",
            0,
        )];

        apply_technique_outcome_rows(StageKind::Enumeration, &mut facts, &rows);

        assert_eq!(facts.len(), 1, "partial must leave a non-terminal marker");
        assert_eq!(facts[0].outcome, EvidenceOutcome::Error);

        facts.push(EvidenceFact {
            asset: "https://app.example.com:443".to_string(),
            technique: "GOLISH-ENUM-JS".to_string(),
            outcome: EvidenceOutcome::Found,
            evidence_id: 42,
        });
        let rows = vec![outcome_fact(
            "https://app.example.com:443/ignored/path",
            "GOLISH-ENUM-JS",
            "error",
            0,
        )];

        apply_technique_outcome_rows(StageKind::Enumeration, &mut facts, &rows);

        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].outcome, EvidenceOutcome::Error);
    }

    #[test]
    fn enumeration_missing_current_outcome_does_not_inherit_legacy_found_fact() {
        let mut facts = vec![EvidenceFact {
            asset: "https://app.example.com:443".to_string(),
            technique: "GOLISH-ENUM-DIR".to_string(),
            outcome: EvidenceOutcome::Found,
            evidence_id: 77,
        }];

        apply_technique_outcome_rows(StageKind::Enumeration, &mut facts, &[]);

        assert!(facts.is_empty());
    }

    #[test]
    fn enumeration_terminal_outcome_requires_real_evidence_id() {
        for outcome in ["found", "empty", "blocked"] {
            let mut facts = Vec::new();
            apply_technique_outcome_rows(
                StageKind::Enumeration,
                &mut facts,
                &[TechniqueOutcomeFact::new(
                    "https://app.example.com:443",
                    "GOLISH-ENUM-DIR",
                    outcome,
                    0,
                    Some(TRUSTED_ENUM_BLOCKED_SOURCE.to_string()),
                )],
            );
            assert!(
                facts.is_empty(),
                "{outcome} without evidence must not close a cell"
            );
        }
    }

    #[test]
    fn enumeration_blocked_requires_matching_current_evidence_fact() {
        let asset = "https://app.example.com:443";
        let technique = "GOLISH-ENUM-DIR";
        let outcome = TechniqueOutcomeFact::new(
            asset,
            technique,
            "blocked",
            61,
            Some(TRUSTED_ENUM_BLOCKED_SOURCE.to_string()),
        );

        for mut facts in [
            Vec::new(),
            vec![EvidenceFact {
                asset: asset.to_string(),
                technique: technique.to_string(),
                outcome: EvidenceOutcome::Blocked,
                evidence_id: 62,
            }],
            vec![EvidenceFact {
                asset: asset.to_string(),
                technique: technique.to_string(),
                outcome: EvidenceOutcome::Error,
                evidence_id: 61,
            }],
        ] {
            apply_technique_outcome_rows(
                StageKind::Enumeration,
                &mut facts,
                std::slice::from_ref(&outcome),
            );
            assert!(
                facts.is_empty(),
                "unmatched blocked evidence must fail closed"
            );
        }

        let trusted_evidence = EvidenceFact {
            asset: asset.to_string(),
            technique: technique.to_string(),
            outcome: EvidenceOutcome::Blocked,
            evidence_id: 61,
        };
        let mut facts = vec![trusted_evidence.clone()];
        let mut forged_source = outcome.clone();
        forged_source.source = Some("untrusted_probe".to_string());
        apply_technique_outcome_rows(
            StageKind::Enumeration,
            &mut facts,
            std::slice::from_ref(&forged_source),
        );
        assert!(
            facts.is_empty(),
            "final and submit gates must reject blocked from an untrusted outcome source"
        );

        let mut facts = vec![trusted_evidence.clone()];
        let mut route_source = outcome.clone();
        route_source.source = Some("route_probe_paths".to_string());
        apply_technique_outcome_rows(
            StageKind::Enumeration,
            &mut facts,
            std::slice::from_ref(&route_source),
        );
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].outcome, EvidenceOutcome::Blocked);

        let mut wrong_axis_facts = vec![EvidenceFact {
            asset: asset.to_string(),
            technique: "GOLISH-ENUM-JS".to_string(),
            outcome: EvidenceOutcome::Blocked,
            evidence_id: 63,
        }];
        apply_technique_outcome_rows(
            StageKind::Enumeration,
            &mut wrong_axis_facts,
            &[TechniqueOutcomeFact::new(
                asset,
                "GOLISH-ENUM-JS",
                "blocked",
                63,
                Some("route_probe_paths".to_string()),
            )],
        );
        assert!(
            wrong_axis_facts.is_empty(),
            "route recovery exhaustion must own only DIR blocked"
        );

        let mut facts = vec![trusted_evidence];
        apply_technique_outcome_rows(
            StageKind::Enumeration,
            &mut facts,
            std::slice::from_ref(&outcome),
        );
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].outcome, EvidenceOutcome::Blocked);
        assert_eq!(facts[0].evidence_id, 61);
    }

    #[test]
    fn preflight_blocked_is_authoritative_for_all_enumeration_axes() {
        let asset = "https://app.example.com:443";
        for technique in ENUM_CONTENT_TECHNIQUES {
            let mut facts = vec![EvidenceFact {
                asset: asset.to_string(),
                technique: technique.to_string(),
                outcome: EvidenceOutcome::Blocked,
                evidence_id: 70,
            }];
            apply_technique_outcome_rows(
                StageKind::Enumeration,
                &mut facts,
                &[TechniqueOutcomeFact::new(
                    asset,
                    technique,
                    "blocked",
                    70,
                    Some(TRUSTED_ENUM_BLOCKED_SOURCE.to_string()),
                )],
            );
            assert_eq!(facts.len(), 1, "preflight should own blocked {technique}");
        }
    }

    #[test]
    fn preflight_blocked_is_prerequisite_gap_not_content_coverage() {
        let mut facts = vec![EvidenceFact {
            asset: "https://app.example.test".to_string(),
            technique: "GOLISH-ENUM-JS".to_string(),
            outcome: EvidenceOutcome::Blocked,
            evidence_id: 41,
        }];
        apply_technique_outcome_rows_for_contract(
            golish_pentest_domain::tool_truth::ToolTruthContract::ReceiptV1,
            StageKind::Enumeration,
            &mut facts,
            &[TechniqueOutcomeFact::new(
                "https://app.example.test",
                "GOLISH-ENUM-JS",
                "blocked",
                41,
                Some(TRUSTED_ENUM_BLOCKED_SOURCE.to_string()),
            )],
        );
        assert!(facts.is_empty());
    }

    #[test]
    fn legacy_preflight_rows_keep_frozen_compatibility() {
        let mut facts = vec![EvidenceFact {
            asset: "https://legacy.example.test".to_string(),
            technique: "GOLISH-ENUM-JS".to_string(),
            outcome: EvidenceOutcome::Blocked,
            evidence_id: 42,
        }];
        apply_technique_outcome_rows_for_contract(
            golish_pentest_domain::tool_truth::ToolTruthContract::LegacyV1,
            StageKind::Enumeration,
            &mut facts,
            &[TechniqueOutcomeFact::new(
                "https://legacy.example.test",
                "GOLISH-ENUM-JS",
                "blocked",
                42,
                Some(TRUSTED_ENUM_BLOCKED_SOURCE.to_string()),
            )],
        );
        assert!(facts
            .iter()
            .any(|fact| fact.outcome == EvidenceOutcome::Blocked));
    }

    #[test]
    fn browser_recovery_blocked_is_authoritative_only_for_browser_axes() {
        let asset = "https://app.example.com:443";
        for technique in ["GOLISH-ENUM-JS", "GOLISH-ENUM-JSAPI", "GOLISH-ENUM-PARAM"] {
            let mut facts = vec![EvidenceFact {
                asset: asset.to_string(),
                technique: technique.to_string(),
                outcome: EvidenceOutcome::Blocked,
                evidence_id: 71,
            }];
            apply_technique_outcome_rows(
                StageKind::Enumeration,
                &mut facts,
                &[TechniqueOutcomeFact::new(
                    asset,
                    technique,
                    "blocked",
                    71,
                    Some("browser_collect_js_api".to_string()),
                )],
            );
            assert_eq!(facts.len(), 1, "browser should own blocked {technique}");
        }

        let mut dir = vec![EvidenceFact {
            asset: asset.to_string(),
            technique: "GOLISH-ENUM-DIR".to_string(),
            outcome: EvidenceOutcome::Blocked,
            evidence_id: 72,
        }];
        apply_technique_outcome_rows(
            StageKind::Enumeration,
            &mut dir,
            &[TechniqueOutcomeFact::new(
                asset,
                "GOLISH-ENUM-DIR",
                "blocked",
                72,
                Some("browser_collect_js_api".to_string()),
            )],
        );
        assert!(dir.is_empty(), "browser must never own DIR blocked");
    }

    #[test]
    fn eas_found_requires_business_truth_guarded_evidence_and_matching_outcome() {
        let asset = "192.0.2.10";
        let technique = TECH_EAS_PORT;
        let business = EvidenceFact {
            asset: asset.to_string(),
            technique: technique.to_string(),
            outcome: EvidenceOutcome::Found,
            evidence_id: 0,
        };
        let guarded = EvidenceFact {
            asset: asset.to_string(),
            technique: technique.to_string(),
            outcome: EvidenceOutcome::Found,
            evidence_id: 41,
        };
        let outcome = outcome_fact(asset, technique, "found", 41);

        for (mut facts, rows) in [
            (vec![business.clone(), guarded.clone()], Vec::new()),
            (vec![guarded.clone()], vec![outcome.clone()]),
            (vec![business.clone()], vec![outcome.clone()]),
            (
                vec![business.clone(), guarded.clone()],
                vec![outcome_fact(asset, technique, "found", 42)],
            ),
        ] {
            apply_technique_outcome_rows(StageKind::ExternalAttackSurface, &mut facts, &rows);
            assert!(
                facts.is_empty(),
                "incomplete EAS intersection must fail closed"
            );
        }

        let mut complete = vec![business, guarded];
        apply_technique_outcome_rows(StageKind::ExternalAttackSurface, &mut complete, &[outcome]);
        assert_eq!(complete.len(), 1);
        assert_eq!(complete[0].outcome, EvidenceOutcome::Found);
        assert_eq!(complete[0].evidence_id, 41);
    }

    #[test]
    fn eas_empty_requires_matching_guarded_outcome_but_not_business_found() {
        let mut facts = vec![EvidenceFact {
            asset: "192.0.2.10".to_string(),
            technique: TECH_EAS_PORT.to_string(),
            outcome: EvidenceOutcome::Empty,
            evidence_id: 51,
        }];
        apply_technique_outcome_rows(
            StageKind::ExternalAttackSurface,
            &mut facts,
            &[outcome_fact("192.0.2.10", TECH_EAS_PORT, "empty", 51)],
        );

        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].outcome, EvidenceOutcome::Empty);
    }

    #[test]
    fn eas_web_blocked_requires_trusted_producer_and_exact_guarded_evidence() {
        let asset = "https://app.example.com:443";
        let guarded = EvidenceFact {
            asset: asset.to_string(),
            technique: TECH_EAS_WEB_FP.to_string(),
            outcome: EvidenceOutcome::Blocked,
            evidence_id: 52,
        };
        let trusted = TechniqueOutcomeFact::new(
            asset,
            TECH_EAS_WEB_FP,
            "blocked",
            52,
            Some("eas_fingerprint_web_stack".to_string()),
        );

        let mut facts = vec![guarded.clone()];
        apply_technique_outcome_rows(
            StageKind::ExternalAttackSurface,
            &mut facts,
            std::slice::from_ref(&trusted),
        );
        assert_eq!(facts, vec![guarded.clone()]);

        for row in [
            TechniqueOutcomeFact::new(
                asset,
                TECH_EAS_WEB_FP,
                "blocked",
                52,
                Some("model_authored".to_string()),
            ),
            TechniqueOutcomeFact::new(
                "http://app.example.com:80",
                TECH_EAS_WEB_FP,
                "blocked",
                52,
                Some("eas_fingerprint_web_stack".to_string()),
            ),
            TechniqueOutcomeFact::new(
                asset,
                TECH_EAS_WEB_FP,
                "blocked",
                53,
                Some("eas_fingerprint_web_stack".to_string()),
            ),
            TechniqueOutcomeFact::new(
                asset,
                TECH_EAS_PORT,
                "blocked",
                52,
                Some("eas_fingerprint_web_stack".to_string()),
            ),
            TechniqueOutcomeFact::new(
                "app.example.com",
                TECH_EAS_WEB_FP,
                "blocked",
                52,
                Some("eas_fingerprint_web_stack".to_string()),
            ),
        ] {
            let mut facts = vec![guarded.clone()];
            apply_technique_outcome_rows(
                StageKind::ExternalAttackSurface,
                &mut facts,
                std::slice::from_ref(&row),
            );
            assert!(
                facts.is_empty(),
                "untrusted EAS blocked row leaked: {row:?}"
            );
        }
    }

    #[test]
    fn guarded_cidr_discovery_outcome_closes_only_its_own_range_without_ip_projection() {
        let asset = "192.0.2.0/24";
        let evidence_id = 61;
        let mut facts = vec![EvidenceFact {
            asset: asset.to_string(),
            technique: TECH_EAS_PORT.to_string(),
            outcome: EvidenceOutcome::Found,
            evidence_id,
        }];
        let outcome = TechniqueOutcomeFact::new(
            asset,
            TECH_EAS_PORT,
            "found",
            evidence_id,
            Some("eas_discover_ports".to_string()),
        );

        apply_technique_outcome_rows(StageKind::ExternalAttackSurface, &mut facts, &[outcome]);

        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].asset, asset);
        assert_eq!(facts[0].outcome, EvidenceOutcome::Found);

        let mut wrong_source = vec![EvidenceFact {
            asset: asset.to_string(),
            technique: TECH_EAS_PORT.to_string(),
            outcome: EvidenceOutcome::Found,
            evidence_id: 62,
        }];
        apply_technique_outcome_rows(
            StageKind::ExternalAttackSurface,
            &mut wrong_source,
            &[TechniqueOutcomeFact::new(
                asset,
                TECH_EAS_PORT,
                "found",
                62,
                Some("naabu".to_string()),
            )],
        );
        assert!(wrong_source.is_empty());
    }

    #[test]
    fn enumeration_outcome_projection_requires_freshness_cutoff() {
        assert!(!stage_accepts_outcome_projection(
            StageKind::Enumeration,
            false
        ));
        assert!(stage_accepts_outcome_projection(
            StageKind::Enumeration,
            true
        ));
        assert!(!stage_accepts_outcome_projection(
            StageKind::ExternalAttackSurface,
            false
        ));
        assert!(stage_accepts_outcome_projection(
            StageKind::ExternalAttackSurface,
            true
        ));
        assert!(!stage_accepts_outcome_projection(
            StageKind::TargetIntel,
            false
        ));
    }

    #[test]
    fn enumeration_gate_expected_techniques_are_always_four_axes() {
        assert_eq!(
            stage_gate_expected_techniques(StageKind::Enumeration, &["ip_address".to_string()])
                .unwrap(),
            ENUM_CONTENT_TECHNIQUES.map(str::to_string).to_vec()
        );
        assert_eq!(
            stage_gate_expected_techniques(StageKind::Enumeration, &[]).unwrap(),
            ENUM_CONTENT_TECHNIQUES.map(str::to_string).to_vec()
        );
        assert_eq!(
            stage_gate_expected_techniques(
                StageKind::TargetIntel,
                &["domain".to_string(), "ip".to_string()]
            )
            .unwrap(),
            Vec::<String>::new(),
            "IntelGoalV1 has no legacy six-axis denominator",
        );
    }

    #[test]
    fn enumeration_does_not_accept_source_query_completion() {
        assert!(!stage_accepts_source_query_completion(
            StageKind::Enumeration
        ));
        assert!(!stage_accepts_source_query_completion(
            StageKind::TargetIntel
        ));
    }

    #[test]
    fn eas_service_outcome_mapping_is_filtered_by_the_outer_intersection() {
        assert_eq!(
            technique_outcome_to_fact(
                "115.175.6.207".to_string(),
                TECH_EAS_SERVICE_FP.to_string(),
                "found".to_string(),
                9,
            )
            .unwrap()
            .outcome,
            EvidenceOutcome::Found
        );
        assert_eq!(
            technique_outcome_to_fact(
                "115.175.6.207".to_string(),
                TECH_EAS_SERVICE_FP.to_string(),
                "empty".to_string(),
                10,
            )
            .unwrap()
            .outcome,
            EvidenceOutcome::Empty
        );
    }

    #[test]
    fn eas_port_empty_outcome_makes_service_not_applicable() {
        let rows = vec![
            outcome_fact("101.132.155.91", TECH_EAS_PORT, "empty", 7),
            outcome_fact("115.175.6.207", TECH_EAS_PORT, "found", 8),
            outcome_fact("example.com", TECH_EAS_LIVENESS, "empty", 9),
        ];

        assert_eq!(
            eas_service_not_applicable_from_port_outcomes(&rows),
            vec![(
                "101.132.155.91".to_string(),
                TECH_EAS_SERVICE_FP.to_string()
            )]
        );
    }

    #[test]
    fn db_truth_rows_are_found_sentinel() {
        let f = db_truth_to_facts(vec![("a.com".into(), "GOLISH-INTEL-ASN".into())]);
        assert_eq!(f[0].outcome, EvidenceOutcome::Found);
        assert_eq!(f[0].evidence_id, 0);
    }

    #[test]
    fn enumeration_worklist_keeps_only_eas_live_web_assets_when_present() {
        let assets = vec![
            "app.example.com".to_string(),
            "dead.example.com".to_string(),
            "https://portal.example.com/login".to_string(),
            "203.0.113.10".to_string(),
        ];
        let typed_assets = vec![
            ("app.example.com".to_string(), "domain".to_string()),
            ("dead.example.com".to_string(), "domain".to_string()),
            (
                "https://portal.example.com/login".to_string(),
                "url".to_string(),
            ),
            ("203.0.113.10".to_string(), "ip".to_string()),
        ];
        let truth = vec![
            ("app.example.com".to_string(), TECH_EAS_LIVENESS.to_string()),
            (
                "portal.example.com/login".to_string(),
                TECH_EAS_LIVENESS.to_string(),
            ),
            ("203.0.113.10".to_string(), TECH_EAS_LIVENESS.to_string()),
        ];

        let worklist = enumeration_eas_live_web_worklist(&assets, &typed_assets, &truth, &[])
            .expect("live web roots narrow enumeration");

        assert_eq!(
            worklist,
            BTreeSet::from([
                "app.example.com".to_string(),
                "https://portal.example.com/login".to_string()
            ])
        );
    }

    #[test]
    fn enumeration_worklist_is_fail_safe_when_eas_has_no_live_truth() {
        let assets = vec!["app.example.com".to_string(), "203.0.113.10".to_string()];
        let typed_assets = vec![
            ("app.example.com".to_string(), "domain".to_string()),
            ("203.0.113.10".to_string(), "ip".to_string()),
        ];

        assert!(
            enumeration_eas_live_web_worklist(&assets, &typed_assets, &[], &[]).is_none(),
            "no inherited EAS liveness truth must not collapse the gate denominator to empty"
        );
    }

    #[test]
    fn enumeration_worklist_includes_http_proven_ip_web_assets() {
        let assets = vec![
            "app.example.com".to_string(),
            "203.0.113.10".to_string(),
            "203.0.113.11".to_string(),
        ];
        let typed_assets = vec![
            ("app.example.com".to_string(), "domain".to_string()),
            ("203.0.113.10".to_string(), "ip".to_string()),
            ("203.0.113.11".to_string(), "ip".to_string()),
        ];
        let truth = vec![("app.example.com".to_string(), TECH_EAS_LIVENESS.to_string())];
        let web_capable = vec!["203.0.113.10".to_string()];

        let worklist =
            enumeration_eas_live_web_worklist(&assets, &typed_assets, &truth, &web_capable)
                .expect("domain liveness + IP-web roots narrow enumeration");

        assert_eq!(
            worklist,
            BTreeSet::from(["app.example.com".to_string(), "203.0.113.10".to_string()])
        );
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
    fn investigation_completion_scope_uses_immutable_root_only_publication() {
        let root = Uuid::from_u128(1);
        let excluded_child = Uuid::from_u128(2);
        let passed_at = Utc::now();
        let publication_authority = vec![(root, passed_at)];

        assert_eq!(
            specialist_completion_scope_ids(
                StageKind::Investigation,
                Some(&publication_authority),
                Some(root),
                Some(vec![root, excluded_child]),
                vec![root, excluded_child],
            ),
            vec![root],
            "the mutable engagement subtree must not enlarge a root-only frozen Investigation scope",
        );
        assert!(
            specialist_completion_scope_ids(
                StageKind::Investigation,
                None,
                Some(root),
                Some(vec![root, excluded_child]),
                vec![root, excluded_child],
            )
            .is_empty(),
            "missing immutable publication authority must fail closed",
        );
    }

    #[test]
    fn non_investigation_completion_scope_keeps_engagement_subtree_behavior() {
        let root = Uuid::from_u128(1);
        let child = Uuid::from_u128(2);

        assert_eq!(
            specialist_completion_scope_ids(
                StageKind::Enumeration,
                None,
                Some(root),
                Some(vec![root, child]),
                vec![root],
            ),
            vec![root, child],
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
            candidates: vec![],
            candidate_decisions: vec![],
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
            candidates: vec![],
            candidate_decisions: vec![],
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

    #[test]
    fn final_org_gate_rejects_present_empty_wave_before_cutoff_fallback() {
        let org_id = Uuid::from_u128(7);
        let wave = StageAssetWaveView {
            id: Uuid::from_u128(1),
            operation_id: Uuid::from_u128(2),
            organization_id: org_id,
            stage_kind: StageKind::Enumeration.as_str().to_string(),
            wave_index: 0,
            started_at: Utc::now(),
            parent_wave_id: None,
            asset_hash: "empty".to_string(),
            target_ids: Vec::new(),
            asset_values: Vec::new(),
        };

        let error = current_wave_gate_error(Some(&wave), Some(org_id), StageKind::Enumeration)
            .expect("a present empty wave must block instead of becoming NoWave");

        assert!(error.contains("has no items"));
    }
}
