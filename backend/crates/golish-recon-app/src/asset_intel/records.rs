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

/// Dedupe lookup matches across providers. Key is `lower(credit_code)` when
/// present (most reliable), else `lower(trim(name))`. Keeps the first hit's
/// confidence + evidence; subsequent duplicates are silently dropped.
pub(crate) fn dedupe_lookup_matches(input: Vec<LookupCompanyMatch>) -> Vec<LookupCompanyMatch> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for m in input {
        let key = m
            .credit_code
            .as_deref()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| m.name.trim().to_lowercase());
        if seen.insert(key) {
            out.push(m);
        }
    }
    out
}
