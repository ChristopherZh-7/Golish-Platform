//! Deterministic backfill of `PlannedSubtask.harness_stage` after Generator LLM
//! returns subtasks.
//!
//! Phase 1 MVP rationale:
//!
//! - The Generator prompt now asks the LLM to fill `harness_stage` when a
//!   subtask matches a known stage (`external_attack_surface`).
//! - LLM compliance is unreliable (especially across vendors and Chinese
//!   prompts); this backfill is a deterministic safety net.
//! - We only **set** `harness_stage` when it is `None`. We never overwrite
//!   an explicit LLM-supplied value (LLM > backfill).
//! - Phase 1 supports only one stage; matchers are conservative — false
//!   negative (skip backfill, hook silently skips) is preferred over false
//!   positive (force the gate on an unrelated subtask).
//!
//! When Phase 2 adds more stages, the matcher fan-out should move into
//! `harness::IntentClassifier` (which already understands intent axes) or a
//! dedicated `stage_keyword_router`.

use crate::harness::{HarnessStageHint, StageKind};

use super::types::PlannedSubtask;

/// Trigger keywords (case-insensitive) for `StageKind::ExternalAttackSurface`.
///
/// Mix of English + Simplified Chinese. Anchored on **whole-word** lowercase
/// matches via [`contains`] (no fuzzy / regex; Phase 2 can swap in a smarter
/// matcher if false positives appear).
///
/// Bias toward recall in scoping vocabulary; explicit anti-patterns below.
const EXTERNAL_ATTACK_SURFACE_KEYWORDS: &[&str] = &[
    // English (passive/external recon vocabulary)
    "external attack surface",
    "attack surface",
    "external recon",
    "passive recon",
    "subdomain enum",
    "subdomain enumeration",
    "subfinder",
    "dns enum",
    "dns resolution",
    "ct log",
    "certificate transparency",
    "asn discovery",
    "external surface",
    // Simplified Chinese (passive/external recon vocabulary)
    "资产测绘",
    "攻击面",
    "外部侦察",
    "外部资产",
    "子域名",
    "外部 attack surface",
    "外部 recon",
    "被动侦察",
    "被动 recon",
];

/// Anti-triggers · if the text contains any of these, we do NOT backfill
/// `ExternalAttackSurface` even if a positive keyword hits. These suggest the
/// subtask is downstream of the surface mapping (exploit / pivot / report /
/// internal) and should not be gated by `external_attack_surface`.
const EXTERNAL_ATTACK_SURFACE_ANTI_TRIGGERS: &[&str] = &[
    "exploit",
    "exploitation",
    "metasploit",
    "sqlmap",
    "internal pivot",
    "lateral movement",
    "post-exploitation",
    "post exploitation",
    "report generation",
    "final report",
    "remediation plan",
    "漏洞利用",
    "横向移动",
    "后渗透",
    "最终报告",
    "修复建议",
];

/// Infer `HarnessStageHint` from arbitrary text (typically a subtask
/// `title + " " + description`).
///
/// Returns `Some(ExternalAttackSurface)` iff the text matches an
/// external-surface keyword AND no anti-trigger fires. Returns `None`
/// otherwise.
pub fn infer_harness_stage(text: &str) -> Option<HarnessStageHint> {
    let lower = text.to_lowercase();

    let has_positive = EXTERNAL_ATTACK_SURFACE_KEYWORDS
        .iter()
        .any(|kw| lower.contains(&kw.to_lowercase()));

    if !has_positive {
        return None;
    }

    let has_anti = EXTERNAL_ATTACK_SURFACE_ANTI_TRIGGERS
        .iter()
        .any(|kw| lower.contains(&kw.to_lowercase()));

    if has_anti {
        return None;
    }

    Some(HarnessStageHint::new(StageKind::ExternalAttackSurface))
}

/// Backfill `harness_stage` on every subtask whose field is currently `None`.
///
/// **Never overwrites** an LLM-supplied value (`Some(_)` → left as-is). Only
/// fills `None` slots using `infer_harness_stage`.
///
/// Returns the number of subtasks that had `harness_stage` set by this pass
/// (for `tracing` / metrics).
pub fn backfill_harness_stage(subtasks: &mut [PlannedSubtask]) -> usize {
    let mut filled = 0;
    for subtask in subtasks.iter_mut() {
        if subtask.harness_stage.is_some() {
            continue;
        }
        let text = format!("{} {}", subtask.title, subtask.description);
        if let Some(hint) = infer_harness_stage(&text) {
            tracing::info!(
                target: "harness::backfill",
                subtask_title = %subtask.title,
                stage_kind = ?hint.stage_kind,
                "harness_stage backfilled by keyword matcher"
            );
            subtask.harness_stage = Some(hint);
            filled += 1;
        }
    }
    if filled > 0 {
        tracing::info!(
            target: "harness::backfill",
            backfilled = filled,
            total = subtasks.len(),
            "harness_stage backfill pass completed"
        );
    }
    filled
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subtask(title: &str, description: &str) -> PlannedSubtask {
        PlannedSubtask {
            title: title.to_string(),
            description: description.to_string(),
            agent: None,
            harness_stage: None,
            nl_slice: None,
            acceptance_criteria: Vec::new(),
        }
    }

    #[test]
    fn english_attack_surface_phrase_triggers_external_attack_surface() {
        let hint = infer_harness_stage("Map the external attack surface of example.com");
        assert!(matches!(
            hint,
            Some(HarnessStageHint {
                stage_kind: StageKind::ExternalAttackSurface
            })
        ));
    }

    #[test]
    fn chinese_attack_surface_triggers_external_attack_surface() {
        let hint = infer_harness_stage("评估 example.com 的攻击面，列出子域名");
        assert!(matches!(
            hint,
            Some(HarnessStageHint {
                stage_kind: StageKind::ExternalAttackSurface
            })
        ));
    }

    #[test]
    fn subdomain_enumeration_triggers_external_attack_surface() {
        let hint = infer_harness_stage("Run subfinder for subdomain enumeration on target.com");
        assert!(matches!(
            hint,
            Some(HarnessStageHint {
                stage_kind: StageKind::ExternalAttackSurface
            })
        ));
    }

    #[test]
    fn unrelated_text_returns_none() {
        let hint = infer_harness_stage("Update README and refactor billing module");
        assert!(hint.is_none());
    }

    #[test]
    fn exploit_anti_trigger_suppresses_match() {
        let hint = infer_harness_stage(
            "Run sqlmap exploit against the subdomain admin panel after enumeration",
        );
        assert!(
            hint.is_none(),
            "anti-trigger 'sqlmap'/'exploit' should suppress despite 'subdomain' positive hit"
        );
    }

    #[test]
    fn final_report_anti_trigger_suppresses_match() {
        let hint = infer_harness_stage(
            "Generate the final report summarizing the external attack surface findings",
        );
        assert!(
            hint.is_none(),
            "final report subtask should not be tagged as external_attack_surface"
        );
    }

    #[test]
    fn backfill_sets_field_for_matching_subtask() {
        let mut subtasks = vec![
            subtask(
                "Recon Phase",
                "Enumerate subdomains via passive recon and CT logs",
            ),
            subtask("Refactor billing", "Move BillingService to new module"),
        ];
        let filled = backfill_harness_stage(&mut subtasks);
        assert_eq!(filled, 1);
        assert!(matches!(
            subtasks[0].harness_stage,
            Some(HarnessStageHint {
                stage_kind: StageKind::ExternalAttackSurface
            })
        ));
        assert!(subtasks[1].harness_stage.is_none());
    }

    #[test]
    fn backfill_preserves_existing_llm_supplied_value() {
        let mut existing = subtask("Map surface", "Subdomain enum + DNS resolution + ct log");
        existing.harness_stage = Some(HarnessStageHint::new(StageKind::ExternalAttackSurface));
        let mut subtasks = vec![existing];
        let filled = backfill_harness_stage(&mut subtasks);
        assert_eq!(filled, 0, "should not overwrite existing harness_stage");
        assert!(subtasks[0].harness_stage.is_some());
    }

    #[test]
    fn backfill_empty_slice_is_noop() {
        let mut subtasks: Vec<PlannedSubtask> = Vec::new();
        let filled = backfill_harness_stage(&mut subtasks);
        assert_eq!(filled, 0);
    }

    #[test]
    fn case_insensitive_match() {
        let hint = infer_harness_stage("SUBDOMAIN ENUMERATION via SubFinder");
        assert!(hint.is_some());
    }

    #[test]
    fn whole_phrase_chinese_external_recon_triggers() {
        let hint = infer_harness_stage("先做外部侦察，列出 example.com 的公开子域名与历史 DNS");
        assert!(matches!(
            hint,
            Some(HarnessStageHint {
                stage_kind: StageKind::ExternalAttackSurface
            })
        ));
    }
}
