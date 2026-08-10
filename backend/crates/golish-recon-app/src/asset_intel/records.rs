//! Record-level normalization: turn provider raw JSON into candidate records,
//! descriptor-driven candidates + profile entries, and lookup matches.
//!
//! Pure logic built on top of the [`super::normalize`] field/filter engine. No
//! DB / IO. Re-exported from the parent module so existing call sites keep
//! using the bare function names.

use std::collections::HashSet;

use serde_json::Value;

use crate::organizations::{
    OrganizationCandidate, OrganizationCandidateKind, OrganizationCandidates,
};

use super::{
    extract_profile_field_entries, filter_passes, now_millis, resolve_field_ref,
    select_json_values, AssetIntelProviderRecord, LookupCompanyMatch, ProfileFieldEntry,
};

pub(crate) fn normalize_provider_records(
    provider_id: &str,
    run_id: &str,
    fetched_at: u64,
    records: Vec<AssetIntelProviderRecord>,
) -> OrganizationCandidates {
    let mut candidates = OrganizationCandidates::default();
    for record in records {
        let candidate = OrganizationCandidate {
            id: format!(
                "{}:{}:{}",
                match record.kind {
                    OrganizationCandidateKind::Organization => "org",
                    OrganizationCandidateKind::Target => "target",
                },
                provider_id,
                record.value.trim()
            ),
            kind: record.kind,
            label: record.label,
            value: record.value,
            organization_id: None,
            ownership_percent: None,
            source: provider_id.to_string(),
            confidence: record.confidence,
            status: "needs_review".to_string(),
            evidence: serde_json::json!({
                "provider": provider_id,
                "runId": run_id,
                "raw": record.evidence,
            }),
            created_at: fetched_at,
        };
        match candidate.kind {
            OrganizationCandidateKind::Organization => candidates.organizations.push(candidate),
            OrganizationCandidateKind::Target => candidates.targets.push(candidate),
        }
    }
    candidates
}

/// Run the descriptor's candidate rules + profile_fields rules against a
/// single raw JSON document. Returns the candidate bucket (for the review
/// queue) plus the profile field entries (master record write). Callers
/// always get both — even when one or the other is empty — so call sites
/// don't have to remember to extract twice.
pub(crate) fn normalize_json_with_descriptor(
    provider_id: &str,
    run_id: &str,
    fetched_at: u64,
    normalize: &golish_pentest::models::AssetIntelNormalizeConfig,
    raw: &Value,
) -> (OrganizationCandidates, Vec<ProfileFieldEntry>) {
    fn collect_rule_records(
        kind: OrganizationCandidateKind,
        rules: &[golish_pentest::models::AssetIntelNormalizeRule],
        raw: &Value,
        out: &mut Vec<AssetIntelProviderRecord>,
    ) {
        for rule in rules {
            for item in select_json_values(raw, &rule.path) {
                // `when` clauses are AND'd; an empty when always keeps the
                // match (legacy behaviour). This is where descriptor-driven
                // filters like `invest.scale >= 51` cut down noise without
                // touching Rust.
                if !filter_passes(item, &rule.when) {
                    continue;
                }
                let Some(label) = resolve_field_ref(item, &rule.label) else {
                    continue;
                };
                let Some(value) = resolve_field_ref(item, &rule.value) else {
                    continue;
                };
                out.push(enscan_record(
                    kind.clone(),
                    &label,
                    &value,
                    rule.confidence,
                    item,
                ));
            }
        }
    }

    let mut records = Vec::new();
    collect_rule_records(
        OrganizationCandidateKind::Organization,
        &normalize.organization,
        raw,
        &mut records,
    );
    collect_rule_records(
        OrganizationCandidateKind::Target,
        &normalize.target,
        raw,
        &mut records,
    );

    let candidates = normalize_provider_records(provider_id, run_id, fetched_at, records);
    let profile_entries = extract_profile_field_entries(&normalize.profile_fields, raw);
    (candidates, profile_entries)
}

fn enscan_record(
    kind: OrganizationCandidateKind,
    label: &str,
    value: &str,
    confidence: f64,
    raw: &Value,
) -> AssetIntelProviderRecord {
    AssetIntelProviderRecord {
        kind,
        label: label.to_string(),
        value: value.to_string(),
        confidence,
        evidence: raw.clone(),
    }
}

/// Walk the descriptor's `lookup.normalize` mapping over a raw JSON
/// document and produce `LookupCompanyMatch` entries — one per item at
/// `normalize.path` that has a usable `name`. Missing optional fields stay
/// `None`; the static `default_confidence` is used unless `score` resolves
/// to a parseable f64.
pub(crate) fn extract_lookup_matches(
    provider_id: &str,
    config: &golish_pentest::models::AssetIntelLookupConfig,
    raw: &Value,
) -> Vec<LookupCompanyMatch> {
    let normalize = &config.normalize;
    let mut out = Vec::new();
    for item in select_json_values(raw, &normalize.path) {
        let Some(name) = resolve_field_ref(item, &normalize.name) else {
            continue;
        };
        let credit_code = normalize
            .credit_code
            .as_ref()
            .and_then(|field_ref| resolve_field_ref(item, field_ref));
        let industry = normalize
            .industry
            .as_ref()
            .and_then(|field_ref| resolve_field_ref(item, field_ref));
        let legal_representative = normalize
            .legal_representative
            .as_ref()
            .and_then(|field_ref| resolve_field_ref(item, field_ref));
        let address = normalize
            .address
            .as_ref()
            .and_then(|field_ref| resolve_field_ref(item, field_ref));
        let registered_at = normalize
            .registered_at
            .as_ref()
            .and_then(|field_ref| resolve_field_ref(item, field_ref));
        let confidence = normalize
            .score
            .as_ref()
            .and_then(|field_ref| resolve_field_ref(item, field_ref))
            .and_then(|raw_score| raw_score.parse::<f64>().ok())
            .unwrap_or(normalize.default_confidence);
        out.push(LookupCompanyMatch {
            provider_id: provider_id.to_string(),
            name,
            credit_code,
            industry,
            legal_representative,
            address,
            registered_at,
            confidence,
            evidence: item.clone(),
        });
    }
    out
}

pub(crate) fn normalize_json_document(
    provider_id: &str,
    run_id: &str,
    normalize: &golish_pentest::models::AssetIntelNormalizeConfig,
    raw: &str,
) -> Option<(OrganizationCandidates, Vec<ProfileFieldEntry>)> {
    let value = serde_json::from_str::<Value>(raw.trim()).ok()?;
    Some(normalize_json_with_descriptor(
        provider_id,
        run_id,
        now_millis(),
        normalize,
        &value,
    ))
}

/// Merge lookup matches that describe the same legal enterprise.
///
/// Credit code is authoritative when both candidates have one. A candidate
/// without a code may still merge with the same normalized legal name, but
/// equal names carrying two different non-empty codes remain distinct. The
/// representative is selected deterministically and every source/raw payload
/// remains in `evidence.sources`; provider iteration order never decides which
/// provenance survives.
pub(crate) fn dedupe_lookup_matches(input: Vec<LookupCompanyMatch>) -> Vec<LookupCompanyMatch> {
    let mut groups: Vec<Vec<LookupCompanyMatch>> = Vec::new();
    for candidate in input {
        if let Some(group) = groups.iter_mut().find(|group| {
            group
                .iter()
                .any(|member| same_lookup_identity(member, &candidate))
        }) {
            group.push(candidate);
        } else {
            groups.push(vec![candidate]);
        }
    }
    groups.into_iter().map(merge_lookup_group).collect()
}

fn same_lookup_identity(left: &LookupCompanyMatch, right: &LookupCompanyMatch) -> bool {
    let left_code = normalized_lookup_field(left.credit_code.as_deref());
    let right_code = normalized_lookup_field(right.credit_code.as_deref());
    if left_code.is_some() && left_code == right_code {
        return true;
    }

    let same_name =
        normalized_lookup_field(Some(&left.name)) == normalized_lookup_field(Some(&right.name));
    if !same_name {
        return false;
    }

    !matches!((left_code, right_code), (Some(left), Some(right)) if left != right)
}

fn normalized_lookup_field(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase)
}

fn lookup_completeness(candidate: &LookupCompanyMatch) -> usize {
    [
        candidate.credit_code.as_deref(),
        candidate.industry.as_deref(),
        candidate.legal_representative.as_deref(),
        candidate.address.as_deref(),
        candidate.registered_at.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter(|value| !value.trim().is_empty())
    .count()
}

fn merge_lookup_group(mut members: Vec<LookupCompanyMatch>) -> LookupCompanyMatch {
    members.sort_by(|left, right| {
        right
            .credit_code
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            .cmp(
                &left
                    .credit_code
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty()),
            )
            .then_with(|| lookup_completeness(right).cmp(&lookup_completeness(left)))
            .then_with(|| right.confidence.total_cmp(&left.confidence))
            .then_with(|| left.provider_id.cmp(&right.provider_id))
            .then_with(|| left.name.cmp(&right.name))
    });

    let mut merged = members
        .first()
        .cloned()
        .expect("lookup identity group is never empty");
    merged.confidence = members
        .iter()
        .map(|candidate| candidate.confidence)
        .max_by(f64::total_cmp)
        .unwrap_or(merged.confidence);
    merged.credit_code = preferred_lookup_field(
        members
            .iter()
            .map(|candidate| candidate.credit_code.as_deref()),
    );
    merged.industry = preferred_lookup_field(
        members
            .iter()
            .map(|candidate| candidate.industry.as_deref()),
    );
    merged.legal_representative = preferred_lookup_field(
        members
            .iter()
            .map(|candidate| candidate.legal_representative.as_deref()),
    );
    merged.address =
        preferred_lookup_field(members.iter().map(|candidate| candidate.address.as_deref()));
    merged.registered_at = preferred_lookup_field(
        members
            .iter()
            .map(|candidate| candidate.registered_at.as_deref()),
    );

    let mut seen_sources = HashSet::new();
    let mut provider_ids = Vec::new();
    let mut sources = Vec::new();
    for candidate in &members {
        if !provider_ids.contains(&candidate.provider_id) {
            provider_ids.push(candidate.provider_id.clone());
        }
        let fingerprint = serde_json::to_string(&serde_json::json!([
            candidate.provider_id,
            candidate.evidence
        ]))
        .unwrap_or_default();
        if seen_sources.insert(fingerprint) {
            sources.push(serde_json::json!({
                "provider_id": candidate.provider_id,
                "confidence": candidate.confidence,
                "raw": candidate.evidence,
            }));
        }
    }
    provider_ids.sort();
    merged.evidence = serde_json::json!({
        "schema": "company_lookup_provenance.v1",
        "primary_provider_id": merged.provider_id,
        "provider_ids": provider_ids,
        "sources": sources,
    });
    merged
}

fn preferred_lookup_field<'a>(values: impl Iterator<Item = Option<&'a str>>) -> Option<String> {
    values
        .filter_map(|value| value.map(str::trim).filter(|value| !value.is_empty()))
        .next()
        .map(str::to_string)
}

#[cfg(test)]
mod lookup_merge_tests {
    use super::*;

    fn candidate(
        provider_id: &str,
        name: &str,
        credit_code: Option<&str>,
        confidence: f64,
    ) -> LookupCompanyMatch {
        LookupCompanyMatch {
            provider_id: provider_id.into(),
            name: name.into(),
            credit_code: credit_code.map(str::to_string),
            industry: None,
            legal_representative: None,
            address: None,
            registered_at: None,
            confidence,
            evidence: serde_json::json!({"source": provider_id}),
        }
    }

    #[test]
    fn lookup_merge_preserves_all_provider_provenance() {
        let merged = dedupe_lookup_matches(vec![
            candidate("enterprise", "Acme Ltd", Some("CODE-1"), 0.68),
            candidate("0.zone", "ACME LTD", Some("code-1"), 0.64),
        ]);

        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0].evidence["provider_ids"],
            serde_json::json!(["0.zone", "enterprise"])
        );
        assert_eq!(
            merged[0].evidence["sources"].as_array().map(Vec::len),
            Some(2)
        );
    }

    #[test]
    fn lookup_merge_keeps_same_name_with_conflicting_codes_distinct() {
        let merged = dedupe_lookup_matches(vec![
            candidate("enterprise-a", "Acme Ltd", Some("CODE-1"), 0.68),
            candidate("enterprise-b", "acme ltd", Some("CODE-2"), 0.68),
        ]);

        assert_eq!(merged.len(), 2);
    }
}
