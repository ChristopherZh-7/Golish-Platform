//! Candidate dedup + merge helpers for asset-intel provider output.
//!
//! Pure logic: collapses duplicate organization/target candidates across
//! providers (case-insensitive on the trimmed value) and unions their evidence
//! `sources`. No DB / Tauri / IO. Re-exported from the parent module so
//! existing call sites keep using the bare function names.

use std::collections::HashSet;

use serde_json::Value;

use crate::tools::organizations::{OrganizationCandidate, OrganizationCandidates};

fn merge_candidate_evidence(
    existing: &mut OrganizationCandidate,
    incoming: &OrganizationCandidate,
) {
    fn evidence_sources(evidence: &Value) -> Vec<Value> {
        evidence
            .get("sources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![evidence.clone()])
    }

    let mut sources = evidence_sources(&existing.evidence);
    for source in evidence_sources(&incoming.evidence) {
        if !sources.iter().any(|item| item == &source) {
            sources.push(source);
        }
    }

    if incoming.confidence > existing.confidence {
        existing.confidence = incoming.confidence;
    }
    if let Some(obj) = existing.evidence.as_object_mut() {
        obj.insert("sources".into(), Value::Array(sources));
    } else {
        existing.evidence = serde_json::json!({
            "primary": existing.evidence,
            "sources": sources,
        });
    }
}

fn dedupe_candidates(candidates: OrganizationCandidates) -> OrganizationCandidates {
    fn dedupe_bucket(items: Vec<OrganizationCandidate>) -> Vec<OrganizationCandidate> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for item in items {
            let key = item.value.trim().to_lowercase();
            if seen.insert(key) {
                out.push(item);
            } else if let Some(existing) = out.iter_mut().find(|existing| {
                existing
                    .value
                    .trim()
                    .eq_ignore_ascii_case(item.value.trim())
            }) {
                merge_candidate_evidence(existing, &item);
            }
        }
        out
    }

    OrganizationCandidates {
        organizations: dedupe_bucket(candidates.organizations),
        targets: dedupe_bucket(candidates.targets),
    }
}

pub(crate) fn merge_candidates(target: &mut OrganizationCandidates, next: OrganizationCandidates) {
    target.organizations.extend(next.organizations);
    target.targets.extend(next.targets);
    let deduped = dedupe_candidates(std::mem::take(target));
    *target = deduped;
}

pub(crate) fn flatten_candidates(
    candidates: &OrganizationCandidates,
) -> Vec<OrganizationCandidate> {
    candidates
        .organizations
        .iter()
        .cloned()
        .chain(candidates.targets.iter().cloned())
        .collect()
}
