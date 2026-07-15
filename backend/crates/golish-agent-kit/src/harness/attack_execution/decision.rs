//! Pure Candidate V2 decision validation and immutable plan derivation.

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use super::classifier::{canonical_plan_hash, classify_candidate, supported_candidate_techniques};
use super::state::AttackExecutionError;
use super::types::{
    AcceptedCandidateDecision, AcceptedNoCandidateDecision, CandidateAcceptance,
    CandidateClassificationInput, CandidateManifestSnapshot, CandidateManifestWorkItem,
    CandidateTargetClass, VerificationRiskClass, CANDIDATE_SURFACE_ANALYSIS_TECHNIQUE,
};

const MAX_CANDIDATE_OBSERVATION_BYTES: usize = 64 * 1024;
const MAX_CANDIDATE_OBSERVATION_HASH_BYTES: usize = 128;
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

fn is_surface_analysis(item: &CandidateManifestWorkItem) -> bool {
    item.observation
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|schema| matches!(schema, "surface_analysis_v1" | "surface_analysis_v2"))
}

fn fact_delta_route_shape_valid(item: &CandidateManifestWorkItem) -> bool {
    let schema = item
        .observation
        .get("schema")
        .and_then(serde_json::Value::as_str);
    if schema != Some(item.observation_kind.as_str()) {
        return false;
    }
    match (item.source_fact_delta_id, item.delta_kind.as_deref()) {
        (None, None) => !item.enrichment_required && schema != Some("surface_analysis_v2"),
        (Some(fact_delta_id), Some(delta_kind)) => {
            matches!(delta_kind, "created" | "updated" | "new_surface")
                && item
                    .observation
                    .get("fact_delta_id")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    == Some(fact_delta_id)
                && item
                    .observation
                    .get("delta_kind")
                    .and_then(serde_json::Value::as_str)
                    == Some(delta_kind)
                && item
                    .observation
                    .get("observation_kind")
                    .and_then(serde_json::Value::as_str)
                    == Some(item.observation_kind.as_str())
                && item
                    .observation
                    .get("enrichment_required")
                    .and_then(serde_json::Value::as_bool)
                    == Some(item.enrichment_required)
                && item.observation.get("allowed_techniques")
                    == Some(&serde_json::json!(item.allowed_techniques))
                && (item.enrichment_required == (schema == Some("surface_analysis_v2")))
        }
        _ => false,
    }
}

fn manifest_item_shape_valid(item: &CandidateManifestWorkItem) -> bool {
    if item.work_item_id.is_nil()
        || item
            .target_live_id
            .is_some_and(|target_id| target_id.is_nil())
        || item.target_type_at_time.trim().is_empty()
        || item.target_value_at_time.trim().is_empty()
        || item.target_identity_hash.trim().is_empty()
        || !bounded_nonempty(&item.observation_kind, MAX_CANDIDATE_TECHNIQUE_BYTES)
        || !bounded_nonempty(&item.work_item_key, MAX_CANDIDATE_WORK_ITEM_KEY_BYTES)
        || !bounded_nonempty(&item.technique, MAX_CANDIDATE_TECHNIQUE_BYTES)
        || !bounded_nonempty(&item.observation_hash, MAX_CANDIDATE_OBSERVATION_HASH_BYTES)
        || !item.observation.is_object()
        || serde_json::to_vec(&item.observation)
            .map_or(true, |bytes| bytes.len() > MAX_CANDIDATE_OBSERVATION_BYTES)
        || item.evidence_ids.is_empty()
        || item.evidence_ids.len() > MAX_CANDIDATE_DECISION_EVIDENCE_IDS
        || item.allowed_techniques.is_empty()
        || item.allowed_techniques.len() > MAX_CANDIDATE_MANIFEST_ITEMS
        || item
            .allowed_techniques
            .iter()
            .any(|technique| !bounded_nonempty(technique, MAX_CANDIDATE_TECHNIQUE_BYTES))
        || item
            .evidence_ids
            .iter()
            .any(|evidence_id| *evidence_id <= 0)
    {
        return false;
    }
    if !fact_delta_route_shape_valid(item) {
        return false;
    }
    let unique_techniques = item.allowed_techniques.iter().collect::<BTreeSet<_>>();
    let unique_evidence = item.evidence_ids.iter().collect::<BTreeSet<_>>();
    if unique_techniques.len() != item.allowed_techniques.len()
        || unique_evidence.len() != item.evidence_ids.len()
    {
        return false;
    }
    if is_surface_analysis(item) {
        item.technique == CANDIDATE_SURFACE_ANALYSIS_TECHNIQUE
            && item.allowed_techniques
                == supported_candidate_techniques(target_class(&item.target_type_at_time))
                    .into_iter()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
    } else {
        item.technique != CANDIDATE_SURFACE_ANALYSIS_TECHNIQUE
            && item.allowed_techniques.len() == 1
            && item.allowed_techniques[0] == item.technique
    }
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
    if manifest.operation_id.is_nil()
        || manifest.scope_snapshot_id.is_nil()
        || manifest.wave_run_id.is_nil()
        || manifest.wave_unit_id.is_nil()
        || manifest.organization_id.is_nil()
        || manifest.work_items.is_empty()
        || manifest.work_items.len() > MAX_CANDIDATE_MANIFEST_ITEMS
    {
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
    let unique_work_item_ids = manifest
        .work_items
        .iter()
        .map(|item| item.work_item_id)
        .collect::<BTreeSet<_>>();
    if by_key.len() != manifest.work_items.len()
        || unique_work_item_ids.len() != manifest.work_items.len()
        || manifest
            .work_items
            .iter()
            .any(|item| !manifest_item_shape_valid(item))
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
                let technique = if is_surface_analysis(item) {
                    let selected = draft
                        .technique
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| {
                            invalid(
                                "ATTACK_SURFACE_TECHNIQUE_REQUIRED",
                                format!(
                                    "surface-analysis work item {work_item_key} requires an explicit technique"
                                ),
                            )
                        })?;
                    if !item
                        .allowed_techniques
                        .iter()
                        .any(|technique| technique == selected)
                    {
                        return Err(invalid(
                            "ATTACK_CAPABILITY_UNSUPPORTED",
                            format!(
                                "surface-analysis work item {work_item_key} selected a technique outside its frozen registry allowlist"
                            ),
                        ));
                    }
                    selected
                } else {
                    let selected = draft
                        .technique
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or(item.technique.as_str());
                    if selected != item.technique {
                        return Err(invalid(
                            "ATTACK_CANDIDATE_TECHNIQUE_DRIFT",
                            format!("work item {work_item_key} changed its frozen technique"),
                        ));
                    }
                    selected
                };
                let identity = format!("{}:{}", manifest.operation_id, item.work_item_id);
                let candidate_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, identity.as_bytes());
                let execution_plan = classify_candidate(&CandidateClassificationInput {
                    candidate_id,
                    target_live_id: item.target_live_id,
                    target_identity_hash: item.target_identity_hash.clone(),
                    target_class: target_class(&item.target_type_at_time),
                    target_value: item.target_value_at_time.clone(),
                    hypothesis: hypothesis.to_string(),
                    technique: technique.to_string(),
                    observation: item.observation.clone(),
                    observation_hash: item.observation_hash.clone(),
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
    use sha2::{Digest, Sha256};

    fn observation_hash(observation: &serde_json::Value) -> String {
        let digest =
            Sha256::digest(serde_json::to_vec(observation).expect("serialize observation"));
        let hex = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        format!("sha256:{hex}")
    }

    fn item(key: &str, evidence_id: i64) -> CandidateManifestWorkItem {
        let target_id = Uuid::new_v4();
        let observation = serde_json::json!({
            "schema": "nuclei_match_v1",
            "source_mode": "general",
            "target_id": target_id,
            "matched_url": "https://example.test/login",
            "template_id": "fixture",
            "technique": "WSTG-INPV-05",
        });
        CandidateManifestWorkItem {
            work_item_id: Uuid::new_v4(),
            work_item_key: key.to_string(),
            target_live_id: Some(target_id),
            target_type_at_time: "url".to_string(),
            target_value_at_time: "https://example.test:443".to_string(),
            target_identity_hash: "sha256:target".to_string(),
            technique: "WSTG-INPV-05".to_string(),
            source_fact_delta_id: None,
            delta_kind: None,
            observation_kind: "nuclei_match_v1".to_string(),
            allowed_techniques: vec!["WSTG-INPV-05".to_string()],
            enrichment_required: false,
            observation_hash: observation_hash(&observation),
            observation,
            evidence_ids: vec![evidence_id],
        }
    }

    fn surface_item(key: &str, evidence_id: i64) -> CandidateManifestWorkItem {
        let target_id = Uuid::new_v4();
        let observation = serde_json::json!({
            "schema": "surface_analysis_v1",
            "target_id": target_id,
            "target_identity": {
                "type": "url",
                "value": "https://example.test:443",
                "sha256": "sha256:surface-target",
            },
            "formulaic_coverage": [],
            "upstream_query_required": true,
        });
        CandidateManifestWorkItem {
            work_item_id: Uuid::new_v4(),
            work_item_key: key.to_string(),
            target_live_id: Some(target_id),
            target_type_at_time: "url".to_string(),
            target_value_at_time: "https://example.test:443".to_string(),
            target_identity_hash: "sha256:surface-target".to_string(),
            technique: CANDIDATE_SURFACE_ANALYSIS_TECHNIQUE.to_string(),
            source_fact_delta_id: None,
            delta_kind: None,
            observation_kind: "surface_analysis_v1".to_string(),
            allowed_techniques: supported_candidate_techniques(CandidateTargetClass::Url)
                .into_iter()
                .map(str::to_string)
                .collect(),
            enrichment_required: false,
            observation_hash: observation_hash(&observation),
            observation,
            evidence_ids: vec![evidence_id],
        }
    }

    fn directory_entry_item(key: &str, evidence_id: i64) -> CandidateManifestWorkItem {
        let target_id = Uuid::new_v4();
        let directory_entry_id = Uuid::new_v4();
        let row = serde_json::json!({
            "content_length": 74,
            "content_type": "",
            "id": directory_entry_id,
            "status_code": 200,
            "target_id": target_id,
            "tool": "route_probe",
            "url": "https://example.test/README.md",
        });
        let observation = serde_json::json!({
            "schema": "directory_entry_observation_v1",
            "target_id": target_id,
            "directory_entry_id": directory_entry_id,
            "directory_entry_row_sha256": observation_hash(&row),
            "url": "https://example.test/README.md",
            "method": "GET",
            "status_code": 200,
            "content_length": 74,
            "content_type": "",
            "source_tool": "route_probe",
            "source_evidence_id": evidence_id,
            "network_attempted": true,
            "authority_current_after": true,
        });
        CandidateManifestWorkItem {
            work_item_id: Uuid::new_v4(),
            work_item_key: key.to_string(),
            target_live_id: Some(target_id),
            target_type_at_time: "url".to_string(),
            target_value_at_time: "https://example.test:443".to_string(),
            target_identity_hash: "sha256:directory-target".to_string(),
            technique: "WSTG-INFO".to_string(),
            source_fact_delta_id: None,
            delta_kind: None,
            observation_kind: "directory_entry_observation_v1".to_string(),
            allowed_techniques: vec!["WSTG-INFO".to_string()],
            enrichment_required: false,
            observation_hash: observation_hash(&observation),
            observation,
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

    #[test]
    fn surface_analysis_requires_a_typed_v2_executor_after_technique_selection() {
        let manifest = CandidateManifestSnapshot {
            operation_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            wave_run_id: Uuid::new_v4(),
            wave_unit_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            manifest_hash: "sha256:surface-manifest".to_string(),
            work_items: vec![surface_item("surface-analysis", 51)],
        };
        let mut draft = CandidateDecisionDraft {
            work_item_key: "surface-analysis".to_string(),
            decision: CandidateDecisionKind::Candidate,
            hypothesis: Some("a reflected input may reach a dangerous sink".to_string()),
            rationale: "derived from the frozen target surface".to_string(),
            technique: None,
            evidence_refs: vec![51],
            no_candidate_reason_code: None,
        };

        assert!(build_candidate_acceptance(&manifest, std::slice::from_ref(&draft)).is_err());
        draft.technique = Some("WSTG-INPV-01".to_string());
        let error = build_candidate_acceptance(&manifest, std::slice::from_ref(&draft))
            .expect_err("a registry technique without a typed V2 executor must be quarantined");
        assert_eq!(error.code(), "ATTACK_EXECUTOR_CONTRACT_UNAVAILABLE");

        draft.technique = Some("WSTG-INFO".to_string());
        let error = build_candidate_acceptance(&manifest, std::slice::from_ref(&draft))
            .expect_err("WSTG-INFO cannot turn a generic surface into an exact replay");
        assert_eq!(error.code(), "ATTACK_EXECUTOR_CONTRACT_UNAVAILABLE");

        draft.technique = Some("WSTG-UNREGISTERED".to_string());
        assert!(build_candidate_acceptance(&manifest, &[draft]).is_err());
    }

    #[test]
    fn directory_entry_observation_accepts_only_its_exact_frozen_evidence() {
        let manifest = CandidateManifestSnapshot {
            operation_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            wave_run_id: Uuid::new_v4(),
            wave_unit_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            manifest_hash: "sha256:directory-manifest".to_string(),
            work_items: vec![directory_entry_item("readme", 20)],
        };
        let draft = CandidateDecisionDraft {
            work_item_key: "readme".to_string(),
            decision: CandidateDecisionKind::Candidate,
            hypothesis: Some("the exact README path may disclose deployment information".into()),
            rationale: "target-bound route observation returned 200".to_string(),
            technique: Some("WSTG-INFO".to_string()),
            evidence_refs: vec![20],
            no_candidate_reason_code: None,
        };
        let accepted = build_candidate_acceptance(&manifest, std::slice::from_ref(&draft))
            .expect("exact directory observation should classify");
        assert_eq!(accepted.candidates.len(), 1);
        assert_eq!(accepted.candidates[0].evidence_ids, vec![20]);
        assert_eq!(
            accepted.candidates[0].execution_plan.actions[0].capability_id,
            "verify.directory_entry_replay"
        );

        let mut foreign = draft;
        foreign.evidence_refs = vec![21];
        let error = build_candidate_acceptance(&manifest, &[foreign])
            .expect_err("same-owner but unlinked evidence must remain unavailable");
        assert_eq!(error.code(), "ATTACK_DECISION_EVIDENCE_UNGROUNDED");
    }

    #[test]
    fn concrete_scanner_observation_still_rejects_technique_drift() {
        let manifest = CandidateManifestSnapshot {
            operation_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            wave_run_id: Uuid::new_v4(),
            wave_unit_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            manifest_hash: "sha256:scanner-manifest".to_string(),
            work_items: vec![item("scanner-observation", 61)],
        };
        let draft = CandidateDecisionDraft {
            work_item_key: "scanner-observation".to_string(),
            decision: CandidateDecisionKind::Candidate,
            hypothesis: Some("the exact scanner match is exploitable".to_string()),
            rationale: "typed match".to_string(),
            technique: Some("WSTG-INPV-01".to_string()),
            evidence_refs: vec![61],
            no_candidate_reason_code: None,
        };

        assert!(build_candidate_acceptance(&manifest, &[draft]).is_err());
    }
}
