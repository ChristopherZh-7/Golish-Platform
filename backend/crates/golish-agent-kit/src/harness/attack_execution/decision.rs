//! Pure Candidate V2 decision validation and immutable plan derivation.

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use super::classifier::{canonical_plan_hash, classify_candidate};
use super::state::AttackExecutionError;
use super::types::{
    AcceptedCandidateDecision, AcceptedNoCandidateDecision, CandidateAcceptance,
    CandidateClassificationInput, CandidateManifestSnapshot, CandidateManifestWorkItem,
    CandidateTargetClass, VerificationRiskClass,
};
use crate::harness::types::{
    CandidateDecisionDraft, CandidateDecisionKind, MAX_CANDIDATE_ACCEPTANCE_BYTES,
    MAX_CANDIDATE_DECISION_EVIDENCE_IDS, MAX_CANDIDATE_HYPOTHESIS_BYTES,
    MAX_CANDIDATE_MANIFEST_ITEMS, MAX_CANDIDATE_RATIONALE_BYTES, MAX_CANDIDATE_REASON_CODE_BYTES,
    MAX_CANDIDATE_TECHNIQUE_BYTES, MAX_CANDIDATE_WORK_ITEM_KEY_BYTES,
};

fn invalid(code: &'static str, message: impl Into<String>) -> AttackExecutionError {
    AttackExecutionError::new(code, message)
}

fn target_class(target_type: &str) -> CandidateTargetClass {
    match target_type.trim().to_ascii_lowercase().as_str() {
        "domain" | "wildcard" => CandidateTargetClass::Domain,
        "ip" => CandidateTargetClass::Ip,
        "url" => CandidateTargetClass::Url,
        "cidr" => CandidateTargetClass::Cidr,
        _ => CandidateTargetClass::Other,
    }
}

#[cfg(test)]
fn risk_label(risk: VerificationRiskClass) -> &'static str {
    match risk {
        VerificationRiskClass::DeterministicSafe => "deterministic_safe",
        VerificationRiskClass::ActiveSafe => "active_safe",
        VerificationRiskClass::Exploit => "exploit",
    }
}

fn priority_for(risk: VerificationRiskClass) -> &'static str {
    match risk {
        VerificationRiskClass::Exploit => "high",
        VerificationRiskClass::ActiveSafe => "medium",
        VerificationRiskClass::DeterministicSafe => "low",
    }
}

fn bounded_nonempty(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes
}

fn stable_reason_code(value: &str) -> bool {
    bounded_nonempty(value, MAX_CANDIDATE_REASON_CODE_BYTES)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn decision_shape_valid(draft: &CandidateDecisionDraft) -> bool {
    if !bounded_nonempty(&draft.work_item_key, MAX_CANDIDATE_WORK_ITEM_KEY_BYTES)
        || !bounded_nonempty(&draft.rationale, MAX_CANDIDATE_RATIONALE_BYTES)
        || draft.evidence_refs.is_empty()
        || draft.evidence_refs.len() > MAX_CANDIDATE_DECISION_EVIDENCE_IDS
        || draft.evidence_refs.iter().any(|id| *id <= 0)
    {
        return false;
    }
    let unique_evidence = draft.evidence_refs.iter().collect::<BTreeSet<_>>();
    if unique_evidence.len() != draft.evidence_refs.len() {
        return false;
    }
    match draft.decision {
        CandidateDecisionKind::Candidate => {
            draft
                .hypothesis
                .as_deref()
                .is_some_and(|value| bounded_nonempty(value, MAX_CANDIDATE_HYPOTHESIS_BYTES))
                && draft.no_candidate_reason_code.is_none()
                && draft
                    .technique
                    .as_deref()
                    .is_none_or(|value| bounded_nonempty(value, MAX_CANDIDATE_TECHNIQUE_BYTES))
        }
        CandidateDecisionKind::NoCandidate => {
            draft.hypothesis.is_none()
                && draft.technique.is_none()
                && draft
                    .no_candidate_reason_code
                    .as_deref()
                    .is_some_and(stable_reason_code)
        }
    }
}

fn grounded_evidence(
    item: &CandidateManifestWorkItem,
    draft: &CandidateDecisionDraft,
) -> Result<Vec<i64>, AttackExecutionError> {
    if draft.evidence_refs.is_empty() {
        return Err(invalid(
            "ATTACK_DECISION_EVIDENCE_REQUIRED",
            format!("work item {} has no decision evidence", item.work_item_key),
        ));
    }
    let available = item.evidence_ids.iter().copied().collect::<BTreeSet<_>>();
    let mut evidence = draft.evidence_refs.clone();
    evidence.sort_unstable();
    let before = evidence.len();
    evidence.dedup();
    if evidence.len() != before
        || evidence
            .iter()
            .any(|id| *id <= 0 || !available.contains(id))
    {
        return Err(invalid(
            "ATTACK_DECISION_EVIDENCE_UNGROUNDED",
            format!(
                "work item {} cites evidence outside its frozen manifest",
                item.work_item_key
            ),
        ));
    }
    Ok(evidence)
}

/// Validate one decision per exact work item and derive every executable field
/// from the immutable classifier registry. No model-owned identity survives.
pub fn build_candidate_acceptance(
    manifest: &CandidateManifestSnapshot,
    drafts: &[CandidateDecisionDraft],
) -> Result<CandidateAcceptance, AttackExecutionError> {
    if manifest.work_items.is_empty() || manifest.work_items.len() > MAX_CANDIDATE_MANIFEST_ITEMS {
        return Err(invalid(
            "ATTACK_CANDIDATE_MANIFEST_EMPTY",
            "Candidate reasoning manifest must be non-empty and policy-bounded",
        ));
    }
    if manifest.manifest_hash.trim().is_empty() {
        return Err(invalid(
            "ATTACK_CANDIDATE_MANIFEST_HASH_MISSING",
            "Candidate manifest requires a server-derived immutable hash",
        ));
    }
    let by_key = manifest
        .work_items
        .iter()
        .map(|item| (item.work_item_key.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    if by_key.len() != manifest.work_items.len()
        || manifest.work_items.iter().any(|item| {
            !bounded_nonempty(&item.work_item_key, MAX_CANDIDATE_WORK_ITEM_KEY_BYTES)
                || !bounded_nonempty(&item.technique, MAX_CANDIDATE_TECHNIQUE_BYTES)
                || item.evidence_ids.is_empty()
                || item.evidence_ids.len() > MAX_CANDIDATE_DECISION_EVIDENCE_IDS
        })
    {
        return Err(invalid(
            "ATTACK_CANDIDATE_MANIFEST_DUPLICATE",
            "Candidate manifest contains duplicate or unbounded work-item rows",
        ));
    }
    if drafts.len() > MAX_CANDIDATE_MANIFEST_ITEMS
        || drafts.iter().any(|draft| !decision_shape_valid(draft))
    {
        return Err(invalid(
            "ATTACK_CANDIDATE_DECISION_SHAPE_INVALID",
            "Candidate decisions exceed the bounded wire contract",
        ));
    }
    let mut decisions = BTreeMap::new();
    for draft in drafts {
        if draft.work_item_key.trim().is_empty()
            || decisions
                .insert(draft.work_item_key.as_str(), draft)
                .is_some()
        {
            return Err(invalid(
                "ATTACK_CANDIDATE_DECISION_DUPLICATE",
                "Candidate decisions must contain unique non-empty work-item keys",
            ));
        }
    }
    if decisions.keys().copied().collect::<Vec<_>>() != by_key.keys().copied().collect::<Vec<_>>() {
        return Err(invalid(
            "ATTACK_CANDIDATE_MANIFEST_INCOMPLETE",
            "every server-seeded work item must end as candidate or evidenced no_candidate",
        ));
    }

    let mut candidates = Vec::new();
    let mut no_candidate_decisions = Vec::new();
    for (work_item_key, item) in by_key {
        let draft = decisions[work_item_key];
        if draft.rationale.trim().is_empty() {
            return Err(invalid(
                "ATTACK_DECISION_RATIONALE_REQUIRED",
                format!("work item {work_item_key} has no rationale"),
            ));
        }
        let evidence_ids = grounded_evidence(item, draft)?;
        match draft.decision {
            CandidateDecisionKind::Candidate => {
                let hypothesis = draft
                    .hypothesis
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        invalid(
                            "ATTACK_CANDIDATE_HYPOTHESIS_REQUIRED",
                            format!("work item {work_item_key} has no hypothesis"),
                        )
                    })?;
                if draft.no_candidate_reason_code.is_some() {
                    return Err(invalid(
                        "ATTACK_CANDIDATE_DECISION_SHAPE_INVALID",
                        format!("candidate work item {work_item_key} carries no_candidate reason"),
                    ));
                }
                let technique = draft
                    .technique
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(item.technique.as_str());
                if technique != item.technique {
                    return Err(invalid(
                        "ATTACK_CANDIDATE_TECHNIQUE_DRIFT",
                        format!("work item {work_item_key} changed its frozen technique"),
                    ));
                }
                let identity = format!("{}:{}", manifest.operation_id, item.work_item_id);
                let candidate_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, identity.as_bytes());
                let execution_plan = classify_candidate(&CandidateClassificationInput {
                    candidate_id,
                    target_identity_hash: item.target_identity_hash.clone(),
                    target_class: target_class(&item.target_type_at_time),
                    target_value: item.target_value_at_time.clone(),
                    hypothesis: hypothesis.to_string(),
                    technique: technique.to_string(),
                    prior_refs: evidence_ids
                        .iter()
                        .map(|id| format!("audit:{id}"))
                        .collect(),
                })?;
                let candidate_plan_hash = canonical_plan_hash(&execution_plan)?;
                let risk_class = super::classifier::classifier_recipe_for(
                    technique,
                    target_class(&item.target_type_at_time),
                )
                .expect("classifier succeeded only with a registered recipe")
                .risk_class;
                let suggested_approach = execution_plan.actions[0].action_kind.clone();
                candidates.push(AcceptedCandidateDecision {
                    candidate_id,
                    work_item_id: item.work_item_id,
                    hypothesis: hypothesis.to_string(),
                    technique: Some(technique.to_string()),
                    rationale: draft.rationale.trim().to_string(),
                    prior_refs: evidence_ids
                        .iter()
                        .map(|id| format!("audit:{id}"))
                        .collect(),
                    suggested_approach,
                    priority: priority_for(risk_class).to_string(),
                    execution_plan,
                    candidate_plan_hash,
                    risk_class,
                    evidence_ids,
                });
            }
            CandidateDecisionKind::NoCandidate => {
                if draft.hypothesis.is_some() || draft.technique.is_some() {
                    return Err(invalid(
                        "ATTACK_NO_CANDIDATE_DECISION_SHAPE_INVALID",
                        format!("no_candidate work item {work_item_key} carries candidate fields"),
                    ));
                }
                let reason_code = draft
                    .no_candidate_reason_code
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        invalid(
                            "ATTACK_NO_CANDIDATE_REASON_REQUIRED",
                            format!("work item {work_item_key} has no stable reason code"),
                        )
                    })?;
                no_candidate_decisions.push(AcceptedNoCandidateDecision {
                    work_item_id: item.work_item_id,
                    reason_code: reason_code.to_string(),
                    detail: draft.rationale.trim().to_string(),
                    evidence_ids,
                });
            }
        }
    }
    let mut expected_work_item_ids = manifest
        .work_items
        .iter()
        .map(|item| item.work_item_id)
        .collect::<Vec<_>>();
    expected_work_item_ids.sort_unstable();
    let acceptance = CandidateAcceptance {
        wave_run_id: manifest.wave_run_id,
        wave_unit_id: manifest.wave_unit_id,
        manifest_hash: manifest.manifest_hash.clone(),
        expected_work_item_ids,
        candidates,
        no_candidate_decisions,
    };
    if serde_json::to_vec(&acceptance)
        .map_err(|_| {
            invalid(
                "ATTACK_CANDIDATE_ACCEPTANCE_INVALID",
                "Candidate acceptance is not serializable",
            )
        })?
        .len()
        > MAX_CANDIDATE_ACCEPTANCE_BYTES
    {
        return Err(invalid(
            "ATTACK_CANDIDATE_ACCEPTANCE_TOO_LARGE",
            "Candidate acceptance exceeds the bounded handoff payload",
        ));
    }
    Ok(acceptance)
}

/// Lightweight pure Gate projection. Exact identity/evidence grounding and plan
/// derivation are repeated by `build_candidate_acceptance` before final seal.
pub fn candidate_manifest_decisions_complete(
    expected_work_item_keys: &[String],
    drafts: &[CandidateDecisionDraft],
) -> bool {
    if expected_work_item_keys.is_empty()
        || drafts.is_empty()
        || expected_work_item_keys.len() > MAX_CANDIDATE_MANIFEST_ITEMS
        || drafts.len() > MAX_CANDIDATE_MANIFEST_ITEMS
        || expected_work_item_keys
            .iter()
            .any(|key| !bounded_nonempty(key, MAX_CANDIDATE_WORK_ITEM_KEY_BYTES))
    {
        return false;
    }
    let expected = expected_work_item_keys.iter().collect::<BTreeSet<_>>();
    let actual = drafts
        .iter()
        .map(|draft| &draft.work_item_key)
        .collect::<BTreeSet<_>>();
    expected.len() == expected_work_item_keys.len()
        && actual.len() == drafts.len()
        && expected == actual
        && drafts.iter().all(decision_shape_valid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, evidence_id: i64) -> CandidateManifestWorkItem {
        CandidateManifestWorkItem {
            work_item_id: Uuid::new_v4(),
            work_item_key: key.to_string(),
            target_live_id: None,
            target_type_at_time: "url".to_string(),
            target_value_at_time: "https://example.test/login".to_string(),
            target_identity_hash: "sha256:target".to_string(),
            technique: "WSTG-INPV-05".to_string(),
            evidence_ids: vec![evidence_id],
        }
    }

    #[test]
    fn empty_candidate_array_with_pending_work_item_blocks() {
        assert!(!candidate_manifest_decisions_complete(
            &["seed:one".to_string()],
            &[]
        ));
    }

    #[test]
    fn every_work_item_must_end_as_candidate_or_evidenced_no_candidate() {
        let manifest = CandidateManifestSnapshot {
            operation_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            wave_run_id: Uuid::new_v4(),
            wave_unit_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            manifest_hash: "sha256:fixture-manifest".to_string(),
            work_items: vec![item("candidate", 41), item("checked-empty", 42)],
        };
        let incomplete = vec![CandidateDecisionDraft {
            work_item_key: "candidate".to_string(),
            decision: CandidateDecisionKind::Candidate,
            hypothesis: Some("bounded SQL injection hypothesis".to_string()),
            rationale: "formulaic observation".to_string(),
            technique: None,
            evidence_refs: vec![41],
            no_candidate_reason_code: None,
        }];
        assert!(build_candidate_acceptance(&manifest, &incomplete).is_err());

        let mut complete = incomplete;
        complete.push(CandidateDecisionDraft {
            work_item_key: "checked-empty".to_string(),
            decision: CandidateDecisionKind::NoCandidate,
            hypothesis: None,
            rationale: "bounded check was empty".to_string(),
            technique: None,
            evidence_refs: vec![42],
            no_candidate_reason_code: Some("checked_empty".to_string()),
        });
        let accepted = build_candidate_acceptance(&manifest, &complete)
            .expect("complete exact manifest should classify");
        assert_eq!(accepted.candidates.len(), 1);
        assert_eq!(accepted.no_candidate_decisions.len(), 1);
        assert_eq!(risk_label(accepted.candidates[0].risk_class), "exploit");
    }

    #[test]
    fn oversized_or_unstable_candidate_decisions_fail_closed() {
        let manifest = CandidateManifestSnapshot {
            operation_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            wave_run_id: Uuid::new_v4(),
            wave_unit_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            manifest_hash: "sha256:fixture-manifest".to_string(),
            work_items: vec![item("checked-empty", 42)],
        };
        let mut draft = CandidateDecisionDraft {
            work_item_key: "checked-empty".to_string(),
            decision: CandidateDecisionKind::NoCandidate,
            hypothesis: None,
            rationale: "x".repeat(8193),
            technique: None,
            evidence_refs: vec![42],
            no_candidate_reason_code: Some("checked_empty".to_string()),
        };
        assert!(build_candidate_acceptance(&manifest, std::slice::from_ref(&draft)).is_err());
        assert!(!candidate_manifest_decisions_complete(
            &["checked-empty".to_string()],
            std::slice::from_ref(&draft),
        ));

        draft.rationale = "bounded".to_string();
        draft.no_candidate_reason_code = Some("Checked Empty".to_string());
        assert!(build_candidate_acceptance(&manifest, std::slice::from_ref(&draft)).is_err());
        assert!(!candidate_manifest_decisions_complete(
            &["checked-empty".to_string()],
            &[draft],
        ));
    }
}
