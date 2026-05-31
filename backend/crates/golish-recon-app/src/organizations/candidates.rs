//! Engagement-candidate read/upsert helpers, persisted under
//! `organizations.intel.engagement.candidates`.

use golish_db::repo::organizations::ProfilePatch;
use serde_json::{json, Value};
use uuid::Uuid;

use golish_app_core::GolishError;
use golish_core::time::now_ms;

use super::types::{OrganizationCandidate, OrganizationCandidateKind, OrganizationCandidates};

pub(super) fn normalize_candidate(mut candidate: OrganizationCandidate) -> OrganizationCandidate {
    if candidate.id.trim().is_empty() {
        candidate.id = format!(
            "{}:{}:{}",
            match candidate.kind {
                OrganizationCandidateKind::Organization => "org",
                OrganizationCandidateKind::Target => "target",
            },
            candidate.source.trim(),
            candidate.value.trim()
        );
    }
    if candidate.status.trim().is_empty() {
        candidate.status = "needs_review".to_string();
    }
    if candidate.source.trim().is_empty() {
        candidate.source = "manual".to_string();
    }
    if candidate.created_at == 0 {
        candidate.created_at = now_ms();
    }
    candidate
}

pub(super) fn read_candidates_from_intel(intel: &Value) -> OrganizationCandidates {
    let candidates = intel
        .get("engagement")
        .and_then(|v| v.get("candidates"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    serde_json::from_value(candidates).unwrap_or_default()
}

pub(super) fn upsert_candidates_into_intel(
    mut intel: Value,
    incoming: Vec<OrganizationCandidate>,
) -> Result<Value, GolishError> {
    if !intel.is_object() {
        intel = json!({});
    }
    let mut store = read_candidates_from_intel(&intel);
    for candidate in incoming.into_iter().map(normalize_candidate) {
        let bucket = match candidate.kind {
            OrganizationCandidateKind::Organization => &mut store.organizations,
            OrganizationCandidateKind::Target => &mut store.targets,
        };
        if let Some(existing) = bucket.iter_mut().find(|item| item.id == candidate.id) {
            *existing = candidate;
        } else {
            bucket.push(candidate);
        }
    }

    let root = intel.as_object_mut().ok_or_else(|| {
        GolishError::Internal("organization intel must be a JSON object".to_string())
    })?;
    let engagement = root.entry("engagement").or_insert_with(|| json!({}));
    if !engagement.is_object() {
        *engagement = json!({});
    }
    if let Some(map) = engagement.as_object_mut() {
        map.insert("candidates".to_string(), serde_json::to_value(store)?);
    }
    Ok(intel)
}

pub async fn upsert_organization_candidates_for_org(
    pool: &sqlx::PgPool,
    id: Uuid,
    candidates: Vec<OrganizationCandidate>,
) -> Result<OrganizationCandidates, GolishError> {
    let row = golish_db::repo::organizations::get_one(pool, id)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {id}")))?;
    let intel = upsert_candidates_into_intel(row.intel, candidates)?;
    let patch = ProfilePatch {
        intel: Some(intel.clone()),
        ..Default::default()
    };
    golish_db::repo::organizations::update_profile(pool, id, &patch)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {id}")))?;
    Ok(read_candidates_from_intel(&intel))
}
