//! Immutable projection-source boundary for Hypothesis Registry writes.
//!
//! Canonical writers may append one complete, typed source batch and advance
//! only the source head.  Materialized entity/legacy rows and the projection
//! head remain exclusively owned by the whole-batch projector.

use chrono::{DateTime, Utc};
use golish_core::hypothesis_semantic_key::CanonicalJsonObject;
use golish_core::investigation_comparison::{
    ComparisonAuthorityBasisInputV1, ComparisonHypothesisDispositionV1,
    ComparisonHypothesisReadinessV1, GenerationComparisonV1, InvestigationComparisonRecordInputV1,
    InvestigationComparisonRecordV1, PlanCComparisonAuthorityInputV1,
};
use golish_core::investigation_projection::{
    projection_timeline_event_kind, HypothesisProjectionRecordV1, LegacyAttemptProjectionRecordV1,
    ProjectionChangeKind, ProjectionInvalidationReason, ProjectionSourceSnapshotV1,
    ProjectionSourceTimeStatusV1,
};
use golish_core::{InvestigationRolloutMode, LegacyProjectionPolicy};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{DbError, Result};

const SOURCE_BATCH_REPLAY_DRIFT: &str = "INVESTIGATION_SOURCE_BATCH_REPLAY_DRIFT";
const SOURCE_BATCH_EMPTY: &str = "INVESTIGATION_SOURCE_BATCH_EMPTY";
const SOURCE_SNAPSHOT_ID_INVALID: &str = "INVESTIGATION_SOURCE_SNAPSHOT_ID_INVALID";
const SOURCE_TIMELINE_ROUTE_INVALID: &str = "INVESTIGATION_SOURCE_TIMELINE_ROUTE_INVALID";
const SOURCE_COMPARISON_RECORD_INVALID: &str = "INVESTIGATION_SOURCE_COMPARISON_RECORD_INVALID";
const LEGACY_REGISTRY_SHADOW_ADAPTER_V1: &str = "grandfathered_legacy_registry_shadow_adapter.v1";

fn conflict(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!("sha256:{hex}")
}

fn comparison_hash(domain: &'static str, value: Value) -> Result<String> {
    let value = CanonicalJsonObject::try_from_value(json!({
        "domain": domain,
        "value": value,
    }))
    .map_err(|_| conflict(SOURCE_COMPARISON_RECORD_INVALID))?;
    let bytes = serde_json::to_vec(value.as_value())
        .map_err(|_| conflict(SOURCE_COMPARISON_RECORD_INVALID))?;
    Ok(sha256_bytes(&bytes))
}

fn normalized_hash(domain: &'static str, value: &str) -> Result<String> {
    if value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Ok(value.to_owned());
    }
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Ok(format!("sha256:{value}"));
    }
    comparison_hash(domain, json!(value))
}

#[derive(Debug)]
struct GrandfatheredComparisonParts {
    semantic_key_hash: String,
    revision_ingredients_hash: String,
    generation: GenerationComparisonV1,
    disposition: ComparisonHypothesisDispositionV1,
    readiness: ComparisonHypothesisReadinessV1,
    tool_truth_member_hashes: Vec<String>,
    candidate_plan_member_hashes: Vec<String>,
    legacy_coverage_member_hashes: Vec<String>,
    finding_lineage_member_hashes: Vec<String>,
    refutation_lineage_member_hashes: Vec<String>,
    residual_member_hashes: Vec<String>,
}

impl GrandfatheredComparisonParts {
    fn into_input(self) -> Result<InvestigationComparisonRecordInputV1> {
        let adapter_contract_hash = comparison_hash(
            "comparison_record.grandfathered_legacy.adapter_contract.v1",
            json!(LEGACY_REGISTRY_SHADOW_ADAPTER_V1),
        )?;
        Ok(InvestigationComparisonRecordInputV1 {
            semantic_key_hash: self.semantic_key_hash,
            revision_ingredients_hash: self.revision_ingredients_hash,
            authority_basis: ComparisonAuthorityBasisInputV1::GrandfatheredLegacy {
                adapter_contract_hash,
                tool_truth_member_hashes: self.tool_truth_member_hashes,
                candidate_plan_member_hashes: self.candidate_plan_member_hashes,
                coverage_member_hashes: self.legacy_coverage_member_hashes.clone(),
            },
            generation: self.generation,
            disposition: self.disposition,
            readiness: self.readiness,
            plan_c: PlanCComparisonAuthorityInputV1::not_available_plan_c(),
            finding_lineage_member_hashes: self.finding_lineage_member_hashes,
            refutation_lineage_member_hashes: self.refutation_lineage_member_hashes,
            residual_member_hashes: self.residual_member_hashes,
            coverage_member_hashes: self.legacy_coverage_member_hashes,
        })
    }
}

fn candidate_disposition(
    value: &str,
) -> (
    ComparisonHypothesisDispositionV1,
    ComparisonHypothesisReadinessV1,
) {
    match value {
        "verified" => (
            ComparisonHypothesisDispositionV1::Supported,
            ComparisonHypothesisReadinessV1::ReportingOnlyPlanCUnavailable,
        ),
        "refuted" => (
            ComparisonHypothesisDispositionV1::Contested,
            ComparisonHypothesisReadinessV1::ReportingOnlyPlanCUnavailable,
        ),
        "rejected" | "blocked" | "no_candidate" => (
            ComparisonHypothesisDispositionV1::Inconclusive,
            ComparisonHypothesisReadinessV1::Blocked,
        ),
        _ => (
            ComparisonHypothesisDispositionV1::Proposed,
            ComparisonHypothesisReadinessV1::PlanningReady,
        ),
    }
}

fn legacy_candidate_comparison_input(
    source: &LegacyCandidateShadowSourceV1,
    tool_truth_contract: &str,
) -> Result<InvestigationComparisonRecordInputV1> {
    let semantic_seed = json!({
        "target_type_at_time": source.target_type_at_time,
        "target_identity_hash": source.target_identity_hash,
        "hypothesis_hash": source.hypothesis_hash,
        "technique": source.technique,
    });
    let revision_seed = json!({
        "semantic": semantic_seed,
        "entity_version": source.entity_version,
        "candidate_plan_hash": source.candidate_plan_hash,
        "disposition": source.disposition,
    });
    let generation_seed = json!({
        "revision": revision_seed,
        "source_contract": "legacy_candidate_shadow.v1",
    });
    let (disposition, readiness) = candidate_disposition(&source.disposition);
    let plan_hashes = source
        .candidate_plan_hash
        .as_deref()
        .map(|value| normalized_hash("legacy_candidate.plan.v1", value))
        .transpose()?
        .into_iter()
        .collect::<Vec<_>>();
    let coverage_hash = comparison_hash(
        "legacy_candidate.coverage.v1",
        json!({
            "hypothesis_hash": source.hypothesis_hash,
            "target_identity_hash": source.target_identity_hash,
        }),
    )?;
    let result_hash = comparison_hash(
        "legacy_candidate.disposition.v1",
        json!({"disposition": source.disposition, "revision": revision_seed}),
    )?;
    GrandfatheredComparisonParts {
        semantic_key_hash: comparison_hash("legacy_candidate.semantic_key.v1", semantic_seed)?,
        revision_ingredients_hash: comparison_hash(
            "legacy_candidate.revision_ingredients.v1",
            revision_seed,
        )?,
        generation: GenerationComparisonV1 {
            generation_ordinal: u32::try_from(source.entity_version.saturating_sub(1))
                .map_err(|_| conflict(SOURCE_COMPARISON_RECORD_INVALID))?,
            generation_seal_hash: comparison_hash(
                "legacy_candidate.generation_seal.v1",
                generation_seed.clone(),
            )?,
            generation_member_set_hash: comparison_hash(
                "legacy_candidate.generation_members.v1",
                generation_seed.clone(),
            )?,
            generation_event_set_hash: comparison_hash(
                "legacy_candidate.generation_events.v1",
                generation_seed.clone(),
            )?,
            open_obligation_set_hash: comparison_hash(
                "legacy_candidate.open_obligations.v1",
                json!({"generation": generation_seed, "disposition": source.disposition}),
            )?,
        },
        disposition,
        readiness,
        tool_truth_member_hashes: vec![comparison_hash(
            "legacy_candidate.tool_truth_contract.v1",
            json!(tool_truth_contract),
        )?],
        candidate_plan_member_hashes: plan_hashes,
        legacy_coverage_member_hashes: vec![coverage_hash],
        finding_lineage_member_hashes: (source.disposition == "verified")
            .then_some(result_hash.clone())
            .into_iter()
            .collect(),
        refutation_lineage_member_hashes: (source.disposition == "refuted")
            .then_some(result_hash.clone())
            .into_iter()
            .collect(),
        residual_member_hashes: matches!(
            source.disposition.as_str(),
            "rejected" | "blocked" | "no_candidate"
        )
        .then_some(result_hash)
        .into_iter()
        .collect(),
    }
    .into_input()
}

#[derive(Debug, Deserialize)]
struct FrozenLegacyCandidateShadowV1 {
    entity_version: i64,
    tool_truth_contract: String,
    target_type_at_time: String,
    target_identity_hash: String,
    hypothesis_hash: Option<String>,
    technique: Option<String>,
    candidate_plan_hash: Option<String>,
    disposition: String,
}

struct RegistryShadowAdapterV1;

impl RegistryShadowAdapterV1 {
    fn reduce_candidate(base_body: &Value) -> Result<InvestigationComparisonRecordInputV1> {
        let frozen: FrozenLegacyCandidateShadowV1 = serde_json::from_value(base_body.clone())
            .map_err(|_| conflict(SOURCE_COMPARISON_RECORD_INVALID))?;
        let semantic_seed = json!({
            "target_type_at_time": frozen.target_type_at_time,
            "target_identity_hash": frozen.target_identity_hash,
            "hypothesis_hash": frozen.hypothesis_hash,
            "technique": frozen.technique,
        });
        let revision_seed = json!({
            "semantic": semantic_seed,
            "entity_version": frozen.entity_version,
            "candidate_plan_hash": frozen.candidate_plan_hash,
            "disposition": frozen.disposition,
        });
        let generation_seed = json!({
            "revision": revision_seed,
            "source_contract": "legacy_candidate_shadow.v1",
        });
        let (disposition, readiness) = candidate_disposition(&frozen.disposition);
        let plan_hashes = frozen
            .candidate_plan_hash
            .as_deref()
            .map(|value| normalized_hash("legacy_candidate.plan.v1", value))
            .transpose()?
            .into_iter()
            .collect::<Vec<_>>();
        let coverage_hash = comparison_hash(
            "legacy_candidate.coverage.v1",
            json!({
                "hypothesis_hash": frozen.hypothesis_hash,
                "target_identity_hash": frozen.target_identity_hash,
            }),
        )?;
        let result_hash = comparison_hash(
            "legacy_candidate.disposition.v1",
            json!({"disposition": frozen.disposition, "revision": revision_seed}),
        )?;
        GrandfatheredComparisonParts {
            semantic_key_hash: comparison_hash("legacy_candidate.semantic_key.v1", semantic_seed)?,
            revision_ingredients_hash: comparison_hash(
                "legacy_candidate.revision_ingredients.v1",
                revision_seed,
            )?,
            generation: GenerationComparisonV1 {
                generation_ordinal: u32::try_from(frozen.entity_version.saturating_sub(1))
                    .map_err(|_| conflict(SOURCE_COMPARISON_RECORD_INVALID))?,
                generation_seal_hash: comparison_hash(
                    "legacy_candidate.generation_seal.v1",
                    generation_seed.clone(),
                )?,
                generation_member_set_hash: comparison_hash(
                    "legacy_candidate.generation_members.v1",
                    generation_seed.clone(),
                )?,
                generation_event_set_hash: comparison_hash(
                    "legacy_candidate.generation_events.v1",
                    generation_seed.clone(),
                )?,
                open_obligation_set_hash: comparison_hash(
                    "legacy_candidate.open_obligations.v1",
                    json!({"generation": generation_seed, "disposition": frozen.disposition}),
                )?,
            },
            disposition,
            readiness,
            tool_truth_member_hashes: vec![comparison_hash(
                "legacy_candidate.tool_truth_contract.v1",
                json!(frozen.tool_truth_contract),
            )?],
            candidate_plan_member_hashes: plan_hashes,
            legacy_coverage_member_hashes: vec![coverage_hash],
            finding_lineage_member_hashes: (frozen.disposition == "verified")
                .then_some(result_hash.clone())
                .into_iter()
                .collect(),
            refutation_lineage_member_hashes: (frozen.disposition == "refuted")
                .then_some(result_hash.clone())
                .into_iter()
                .collect(),
            residual_member_hashes: matches!(
                frozen.disposition.as_str(),
                "rejected" | "blocked" | "no_candidate"
            )
            .then_some(result_hash)
            .into_iter()
            .collect(),
        }
        .into_input()
    }

    fn reduce_attempt(base_body: &Value) -> Result<InvestigationComparisonRecordInputV1> {
        let frozen: FrozenLegacyAttemptShadowV1 = serde_json::from_value(base_body.clone())
            .map_err(|_| conflict(SOURCE_COMPARISON_RECORD_INVALID))?;
        let semantic_seed = json!({
            "candidate_plan_hash": frozen.candidate_plan_hash,
            "result_hash": frozen.result_hash,
        });
        let revision_seed = json!({
            "semantic": semantic_seed,
            "entity_version": frozen.entity_version,
            "disposition": frozen.disposition,
        });
        let generation_seed = json!({
            "revision": revision_seed,
            "source_contract": "legacy_attempt_shadow.v1",
        });
        let (disposition, readiness) = candidate_disposition(&frozen.disposition);
        let plan_hash = normalized_hash(
            "legacy_attempt.candidate_plan.v1",
            &frozen.candidate_plan_hash,
        )?;
        let result_hash = normalized_hash("legacy_attempt.result.v1", &frozen.result_hash)?;
        GrandfatheredComparisonParts {
            semantic_key_hash: comparison_hash("legacy_attempt.semantic_key.v1", semantic_seed)?,
            revision_ingredients_hash: comparison_hash(
                "legacy_attempt.revision_ingredients.v1",
                revision_seed,
            )?,
            generation: GenerationComparisonV1 {
                generation_ordinal: u32::try_from(frozen.entity_version.saturating_sub(1))
                    .map_err(|_| conflict(SOURCE_COMPARISON_RECORD_INVALID))?,
                generation_seal_hash: comparison_hash(
                    "legacy_attempt.generation_seal.v1",
                    generation_seed.clone(),
                )?,
                generation_member_set_hash: comparison_hash(
                    "legacy_attempt.generation_members.v1",
                    generation_seed.clone(),
                )?,
                generation_event_set_hash: comparison_hash(
                    "legacy_attempt.generation_events.v1",
                    generation_seed.clone(),
                )?,
                open_obligation_set_hash: comparison_hash(
                    "legacy_attempt.open_obligations.v1",
                    json!({"generation": generation_seed, "disposition": frozen.disposition}),
                )?,
            },
            disposition,
            readiness,
            tool_truth_member_hashes: vec![comparison_hash(
                "legacy_attempt.tool_truth_contract.v1",
                json!(frozen.tool_truth_contract),
            )?],
            candidate_plan_member_hashes: vec![plan_hash],
            legacy_coverage_member_hashes: vec![result_hash.clone()],
            finding_lineage_member_hashes: (frozen.disposition == "verified")
                .then_some(result_hash.clone())
                .into_iter()
                .collect(),
            refutation_lineage_member_hashes: (frozen.disposition == "refuted")
                .then_some(result_hash.clone())
                .into_iter()
                .collect(),
            residual_member_hashes: matches!(
                frozen.disposition.as_str(),
                "failed" | "blocked" | "rejected"
            )
            .then_some(result_hash)
            .into_iter()
            .collect(),
        }
        .into_input()
    }
}

fn legacy_attempt_comparison_input(
    source: &LegacyAttemptShadowSourceV1,
    tool_truth_contract: &str,
) -> Result<InvestigationComparisonRecordInputV1> {
    let semantic_seed = json!({
        "candidate_plan_hash": source.candidate_plan_hash,
        "result_hash": source.result_hash,
    });
    let revision_seed = json!({
        "semantic": semantic_seed,
        "entity_version": source.entity_version,
        "disposition": source.disposition,
    });
    let generation_seed = json!({
        "revision": revision_seed,
        "source_contract": "legacy_attempt_shadow.v1",
    });
    let (disposition, readiness) = candidate_disposition(&source.disposition);
    let plan_hash = normalized_hash(
        "legacy_attempt.candidate_plan.v1",
        &source.candidate_plan_hash,
    )?;
    let result_hash = normalized_hash("legacy_attempt.result.v1", &source.result_hash)?;
    GrandfatheredComparisonParts {
        semantic_key_hash: comparison_hash("legacy_attempt.semantic_key.v1", semantic_seed)?,
        revision_ingredients_hash: comparison_hash(
            "legacy_attempt.revision_ingredients.v1",
            revision_seed,
        )?,
        generation: GenerationComparisonV1 {
            generation_ordinal: u32::try_from(source.entity_version.saturating_sub(1))
                .map_err(|_| conflict(SOURCE_COMPARISON_RECORD_INVALID))?,
            generation_seal_hash: comparison_hash(
                "legacy_attempt.generation_seal.v1",
                generation_seed.clone(),
            )?,
            generation_member_set_hash: comparison_hash(
                "legacy_attempt.generation_members.v1",
                generation_seed.clone(),
            )?,
            generation_event_set_hash: comparison_hash(
                "legacy_attempt.generation_events.v1",
                generation_seed.clone(),
            )?,
            open_obligation_set_hash: comparison_hash(
                "legacy_attempt.open_obligations.v1",
                json!({"generation": generation_seed, "disposition": source.disposition}),
            )?,
        },
        disposition,
        readiness,
        tool_truth_member_hashes: vec![comparison_hash(
            "legacy_attempt.tool_truth_contract.v1",
            json!(tool_truth_contract),
        )?],
        candidate_plan_member_hashes: vec![plan_hash],
        legacy_coverage_member_hashes: vec![result_hash.clone()],
        finding_lineage_member_hashes: (source.disposition == "verified")
            .then_some(result_hash.clone())
            .into_iter()
            .collect(),
        refutation_lineage_member_hashes: (source.disposition == "refuted")
            .then_some(result_hash.clone())
            .into_iter()
            .collect(),
        residual_member_hashes: matches!(
            source.disposition.as_str(),
            "failed" | "blocked" | "rejected"
        )
        .then_some(result_hash)
        .into_iter()
        .collect(),
    }
    .into_input()
}

#[derive(Debug, Deserialize)]
struct FrozenLegacyAttemptShadowV1 {
    entity_version: i64,
    tool_truth_contract: String,
    candidate_plan_hash: String,
    result_hash: String,
    disposition: String,
}

/// Freeze independently complete legacy/registry comparison inputs into a
/// canonical projection-source body. Each present side is compiled before it
/// is serialized, so production writers cannot freeze a partial record.
pub fn freeze_comparison_projection_source_body_v1(
    mut base_body: Value,
    legacy: Option<InvestigationComparisonRecordInputV1>,
    registry: Option<InvestigationComparisonRecordInputV1>,
) -> Result<CanonicalJsonObject> {
    for input in [legacy.as_ref(), registry.as_ref()].into_iter().flatten() {
        InvestigationComparisonRecordV1::compile(input.clone())
            .map_err(|_| conflict(SOURCE_COMPARISON_RECORD_INVALID))?;
    }
    let object = base_body
        .as_object_mut()
        .ok_or_else(|| conflict(SOURCE_COMPARISON_RECORD_INVALID))?;
    if object.contains_key("comparison_record_v1") {
        return Err(conflict(SOURCE_COMPARISON_RECORD_INVALID));
    }
    object.insert(
        "comparison_record_v1".to_owned(),
        json!({"legacy":legacy,"registry":registry}),
    );
    CanonicalJsonObject::try_from_value(base_body)
        .map_err(|_| conflict(SOURCE_COMPARISON_RECORD_INVALID))
}

#[derive(Debug, Clone)]
pub struct ProjectionOutboxSourceRow {
    pub outbox_member_id: Uuid,
    pub change_kind: ProjectionChangeKind,
    pub source: ProjectionSourceSnapshotV1,
    pub source_occurred_at: Option<DateTime<Utc>>,
    pub source_time_status: ProjectionSourceTimeStatusV1,
    pub invalidation_reason: Option<ProjectionInvalidationReason>,
    pub storage: ProjectionSourceStorageV1,
}

/// Storage policy is selected by trusted repository code.  Blob bytes and
/// hashes are always derived from the typed snapshot; callers cannot provide
/// either value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionSourceStorageV1 {
    Inline,
    Blob { redaction_contract_version: String },
}

#[derive(Debug, Clone)]
pub(crate) struct LegacyCandidateShadowSourceV1 {
    pub entity_id: Uuid,
    pub entity_version: i64,
    pub organization_id: Uuid,
    pub source_work_item_id: Uuid,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub hypothesis_hash: Option<String>,
    pub technique: Option<String>,
    pub candidate_plan_hash: Option<String>,
    pub disposition: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LegacyAttemptShadowSourceV1 {
    pub attempt_id: Uuid,
    pub entity_version: i64,
    pub organization_id: Uuid,
    pub candidate_id: Uuid,
    pub candidate_plan_hash: String,
    pub result_hash: String,
    pub disposition: String,
}

pub(crate) async fn append_legacy_candidate_shadow_batch_with_connection(
    connection: &mut sqlx::PgConnection,
    mode: InvestigationRolloutMode,
    operation_id: Uuid,
    stable_source_id: Uuid,
    occurred_at: DateTime<Utc>,
    sources: Vec<LegacyCandidateShadowSourceV1>,
) -> Result<Option<ProjectionSourceBatchView>> {
    if !mode.policy().write_registry_shadow {
        return Ok(None);
    }
    let (project_scope_id, tool_truth_contract): (Option<Uuid>, String) = sqlx::query_as(
        "SELECT project_scope_id,tool_truth_contract FROM operation_state WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_one(&mut *connection)
    .await?;
    let stable_request_id = Uuid::new_v5(&stable_source_id, b"legacy-candidate-shadow-batch.v1");
    let mut members = Vec::with_capacity(sources.len());
    for source in sources {
        let base_body = json!({
            "source_contract": "legacy_candidate_shadow.v1",
            "entity_version": source.entity_version,
            "tool_truth_contract": tool_truth_contract,
            "organization_id": source.organization_id,
            "root_id": source.entity_id,
            "revision_id": source.entity_id,
            "legacy_work_item_id": source.source_work_item_id,
            "target_type_at_time": source.target_type_at_time,
            "target_value_at_time": source.target_value_at_time,
            "target_identity_hash": source.target_identity_hash,
            "hypothesis_hash": source.hypothesis_hash,
            "technique": source.technique,
            "candidate_plan_hash": source.candidate_plan_hash,
            "disposition": source.disposition,
            "comparison_record_availability": "complete_grandfathered_legacy",
            "comparison_authority_basis": "grandfathered_legacy",
        });
        let legacy = legacy_candidate_comparison_input(&source, &tool_truth_contract)?;
        let registry = RegistryShadowAdapterV1::reduce_candidate(&base_body)?;
        let body =
            freeze_comparison_projection_source_body_v1(base_body, Some(legacy), Some(registry))?;
        let record = HypothesisProjectionRecordV1::try_new(
            source.entity_id.to_string(),
            u64::try_from(source.entity_version)
                .map_err(|_| conflict(SOURCE_SNAPSHOT_ID_INVALID))?,
            1,
            body,
        )
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        members.push(ProjectionOutboxSourceRow {
            outbox_member_id: Uuid::new_v5(
                &stable_request_id,
                format!(
                    "legacy-candidate-shadow-member.v1:{}:{}",
                    source.entity_id, source.entity_version
                )
                .as_bytes(),
            ),
            change_kind: ProjectionChangeKind::Insert,
            source: ProjectionSourceSnapshotV1::Hypothesis(record),
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            invalidation_reason: None,
            storage: ProjectionSourceStorageV1::Blob {
                redaction_contract_version: "legacy_candidate_shadow.v1".to_owned(),
            },
        });
    }
    if members.is_empty() {
        return Ok(None);
    }
    append_projection_source_batch_with_connection(
        connection,
        AppendProjectionSourceBatchRow {
            batch_id: Uuid::new_v5(&stable_request_id, b"batch"),
            operation_id,
            project_scope_id,
            stable_request_id,
            source_transaction_id: stable_source_id,
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            members,
        },
    )
    .await
    .map(Some)
}

pub(crate) async fn append_legacy_attempt_shadow_with_connection(
    connection: &mut sqlx::PgConnection,
    mode: InvestigationRolloutMode,
    operation_id: Uuid,
    occurred_at: DateTime<Utc>,
    source: LegacyAttemptShadowSourceV1,
) -> Result<Option<ProjectionSourceBatchView>> {
    if !mode.policy().write_registry_shadow {
        return Ok(None);
    }
    let (project_scope_id, tool_truth_contract): (Option<Uuid>, String) = sqlx::query_as(
        "SELECT project_scope_id,tool_truth_contract FROM operation_state WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_one(&mut *connection)
    .await?;
    let base_body = json!({
        "source_contract": "legacy_attempt_shadow.v1",
        "entity_version": source.entity_version,
        "tool_truth_contract": tool_truth_contract,
        "organization_id": source.organization_id,
        "attempt_id": source.attempt_id,
        "candidate_id": source.candidate_id,
        "candidate_plan_hash": source.candidate_plan_hash,
        "result_hash": source.result_hash,
        "disposition": source.disposition,
        "plan_c_authority": "not_available_plan_c",
        "comparison_record_availability": "complete_grandfathered_legacy",
        "comparison_authority_basis": "grandfathered_legacy",
    });
    let legacy = legacy_attempt_comparison_input(&source, &tool_truth_contract)?;
    let registry = RegistryShadowAdapterV1::reduce_attempt(&base_body)?;
    let body =
        freeze_comparison_projection_source_body_v1(base_body, Some(legacy), Some(registry))?;
    let record = LegacyAttemptProjectionRecordV1::try_new(
        source.attempt_id.to_string(),
        u64::try_from(source.entity_version).map_err(|_| conflict(SOURCE_SNAPSHOT_ID_INVALID))?,
        1,
        body,
    )
    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    let stable_request_id = Uuid::new_v5(
        &source.attempt_id,
        format!("legacy-attempt-shadow.v1:{}", source.entity_version).as_bytes(),
    );
    append_projection_source_batch_with_connection(
        connection,
        AppendProjectionSourceBatchRow {
            batch_id: Uuid::new_v5(&stable_request_id, b"batch"),
            operation_id,
            project_scope_id,
            stable_request_id,
            source_transaction_id: stable_request_id,
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            members: vec![ProjectionOutboxSourceRow {
                outbox_member_id: Uuid::new_v5(&stable_request_id, b"member"),
                change_kind: ProjectionChangeKind::Insert,
                source: ProjectionSourceSnapshotV1::LegacyAttemptProjection(record),
                source_occurred_at: Some(occurred_at),
                source_time_status: ProjectionSourceTimeStatusV1::Known,
                invalidation_reason: None,
                storage: ProjectionSourceStorageV1::Blob {
                    redaction_contract_version: "legacy_attempt_shadow.v1".to_owned(),
                },
            }],
        },
    )
    .await
    .map(Some)
}

#[derive(Debug, Clone)]
pub struct AppendProjectionSourceBatchRow {
    pub batch_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Option<Uuid>,
    pub stable_request_id: Uuid,
    pub source_transaction_id: Uuid,
    pub source_occurred_at: Option<DateTime<Utc>>,
    pub source_time_status: ProjectionSourceTimeStatusV1,
    pub members: Vec<ProjectionOutboxSourceRow>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct ProjectionSourceBatchView {
    pub batch_id: Uuid,
    pub operation_id: Uuid,
    pub source_batch_seq: i64,
    pub predecessor_batch_id: Option<Uuid>,
    pub project_scope_id: Option<Uuid>,
    pub stable_request_id: Uuid,
    pub source_transaction_id: Uuid,
    pub member_count: i64,
    pub member_set_hash: String,
    pub source_occurred_at: Option<DateTime<Utc>>,
    pub source_time_status: String,
    #[sqlx(default)]
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyCompatibilityReadDisposition {
    Ready,
    HistoricalReadOnly,
    Hold,
    Missing,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct LegacyCompatibilityProjectionVersion {
    pub operation_id: Uuid,
    pub entity_id: Uuid,
    pub entity_version: i64,
    pub source_generation_id: Uuid,
    pub source_revision_id: Uuid,
    pub source_contract_hash: String,
    pub projection_status: String,
    pub projection_body: Option<Value>,
    pub projection_hash: String,
    pub batch_id: Uuid,
    pub source_batch_seq: i64,
    pub change_seq: i64,
    pub invalidation_reason: Option<String>,
    pub source_occurred_at: Option<DateTime<Utc>>,
    pub source_time_status: String,
    pub projected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyCompatibilityRead {
    pub disposition: LegacyCompatibilityReadDisposition,
    pub projection: Option<LegacyCompatibilityProjectionVersion>,
}

fn compatibility_read_disposition(
    policy: LegacyProjectionPolicy,
    projection_status: &str,
) -> LegacyCompatibilityReadDisposition {
    if projection_status != "ready" {
        return LegacyCompatibilityReadDisposition::Hold;
    }
    match policy {
        LegacyProjectionPolicy::HistoricalReadOnly => {
            LegacyCompatibilityReadDisposition::HistoricalReadOnly
        }
        LegacyProjectionPolicy::Native | LegacyProjectionPolicy::CanonicalDerivedFailClosed => {
            LegacyCompatibilityReadDisposition::Ready
        }
    }
}

#[derive(Debug)]
struct PreparedSourceMember {
    outbox_member_id: Uuid,
    member_ordinal: i32,
    entity_kind: &'static str,
    change_kind: &'static str,
    source_entity_id: String,
    source_entity_version: i64,
    source_entity_hash: String,
    source_occurred_at: Option<DateTime<Utc>>,
    source_time_status: &'static str,
    source_snapshot_hash: String,
    immutable_source_body: Option<Value>,
    blob_bytes: Option<Vec<u8>>,
    blob_hash: Option<String>,
    blob_redaction_contract_version: Option<String>,
    timeline_event_kind: &'static str,
    invalidation_reason: Option<&'static str>,
    member_hash: String,
}

fn prepare_member(member: &ProjectionOutboxSourceRow) -> Result<PreparedSourceMember> {
    let entity_kind = member.source.entity_kind();
    let timeline_event_kind = projection_timeline_event_kind(entity_kind, member.change_kind)
        .ok_or_else(|| conflict(SOURCE_TIMELINE_ROUTE_INVALID))?;
    if (member.change_kind == ProjectionChangeKind::Invalidate)
        != member.invalidation_reason.is_some()
    {
        return Err(conflict(SOURCE_TIMELINE_ROUTE_INVALID));
    }
    if (member.source_time_status == ProjectionSourceTimeStatusV1::Known)
        != member.source_occurred_at.is_some()
    {
        return Err(conflict(SOURCE_TIMELINE_ROUTE_INVALID));
    }

    let record = member.source.record();
    let source_entity_id = record.entity_id().to_owned();
    let source_entity_version = i64::try_from(record.entity_version())
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| conflict(SOURCE_SNAPSHOT_ID_INVALID))?;
    let source_entity_hash = record.content_sha256().to_owned();
    let body = serde_json::to_value(&member.source)?;
    let source_bytes = serde_json::to_vec(&body)?;
    let source_snapshot_hash = sha256_bytes(&source_bytes);
    let invalidation_reason = member.invalidation_reason.map(|value| value.as_str());
    Ok(PreparedSourceMember {
        outbox_member_id: member.outbox_member_id,
        member_ordinal: -1,
        entity_kind: entity_kind.as_str(),
        change_kind: member.change_kind.as_str(),
        source_entity_id,
        source_entity_version,
        source_entity_hash,
        source_occurred_at: member.source_occurred_at,
        source_time_status: member.source_time_status.as_str(),
        source_snapshot_hash: source_snapshot_hash.clone(),
        immutable_source_body: matches!(member.storage, ProjectionSourceStorageV1::Inline)
            .then_some(body),
        blob_bytes: matches!(member.storage, ProjectionSourceStorageV1::Blob { .. })
            .then_some(source_bytes),
        blob_hash: matches!(member.storage, ProjectionSourceStorageV1::Blob { .. })
            .then_some(source_snapshot_hash.clone()),
        blob_redaction_contract_version: match &member.storage {
            ProjectionSourceStorageV1::Inline => None,
            ProjectionSourceStorageV1::Blob {
                redaction_contract_version,
            } => Some(redaction_contract_version.clone()),
        },
        timeline_event_kind: timeline_event_kind.as_str(),
        invalidation_reason,
        member_hash: String::new(),
    })
}

fn finalize_prepared_member(member: &mut PreparedSourceMember, ordinal: usize) -> Result<()> {
    member.member_ordinal = i32::try_from(ordinal).map_err(|_| conflict(SOURCE_BATCH_EMPTY))?;
    member.member_hash = sha256_bytes(&serde_json::to_vec(&json!({
        "domain": "investigation_projection_outbox_member.v1",
        "ordinal": ordinal,
        "entity_kind": member.entity_kind,
        "change_kind": member.change_kind,
        "source_entity_id": member.source_entity_id,
        "source_entity_version": member.source_entity_version,
        "source_entity_hash": member.source_entity_hash,
        "source_snapshot_hash": member.source_snapshot_hash,
        "source_time_status": member.source_time_status,
        "source_occurred_at": member.source_occurred_at,
        "timeline_event_kind": member.timeline_event_kind,
        "invalidation_reason": member.invalidation_reason,
        "storage": if member.blob_hash.is_some() { "blob" } else { "inline" },
        "source_blob_hash": member.blob_hash,
    }))?);
    Ok(())
}

/// Append a complete source batch inside the caller's canonical transaction.
///
/// This function intentionally has no `PgPool` overload: root/revision,
/// generation, residual and outbox truth must share one commit boundary.
pub(crate) async fn append_projection_source_batch_on(
    tx: &mut Transaction<'_, Postgres>,
    input: AppendProjectionSourceBatchRow,
) -> Result<ProjectionSourceBatchView> {
    append_projection_source_batch_with_connection(tx, input).await
}

pub(crate) async fn append_projection_source_batch_with_connection(
    connection: &mut sqlx::PgConnection,
    input: AppendProjectionSourceBatchRow,
) -> Result<ProjectionSourceBatchView> {
    if input.members.is_empty() {
        return Err(conflict(SOURCE_BATCH_EMPTY));
    }
    if (input.source_time_status == ProjectionSourceTimeStatusV1::Known)
        != input.source_occurred_at.is_some()
    {
        return Err(conflict(SOURCE_TIMELINE_ROUTE_INVALID));
    }

    sqlx::query("SELECT operation_id FROM operation_state WHERE operation_id=$1 FOR UPDATE")
        .bind(input.operation_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or_else(|| DbError::NotFound("operation_state".into()))?;
    let (last_source_batch_seq, predecessor_batch_id): (i64, Option<Uuid>) = sqlx::query_as(
        r#"SELECT last_source_batch_seq,last_source_batch_id
             FROM investigation_projection_source_heads
            WHERE operation_id=$1 FOR UPDATE"#,
    )
    .bind(input.operation_id)
    .fetch_one(&mut *connection)
    .await?;

    let mut prepared = input
        .members
        .iter()
        .map(prepare_member)
        .collect::<Result<Vec<_>>>()?;
    prepared.sort_by(|left, right| {
        (
            left.entity_kind,
            left.source_entity_id.as_str(),
            left.source_entity_version,
            left.change_kind,
        )
            .cmp(&(
                right.entity_kind,
                right.source_entity_id.as_str(),
                right.source_entity_version,
                right.change_kind,
            ))
    });
    for (ordinal, member) in prepared.iter_mut().enumerate() {
        finalize_prepared_member(member, ordinal)?;
    }
    let member_hashes = prepared
        .iter()
        .map(|member| member.member_hash.clone())
        .collect::<Vec<_>>();
    let member_set_hash: String =
        sqlx::query_scalar("SELECT tool_truth_sha256(to_jsonb($1::TEXT[])::TEXT)")
            .bind(&member_hashes)
            .fetch_one(&mut *connection)
            .await?;
    let member_count = i64::try_from(prepared.len()).map_err(|_| conflict(SOURCE_BATCH_EMPTY))?;

    if let Some(existing) = sqlx::query_as::<_, ProjectionSourceBatchView>(
        r#"SELECT batch_id,operation_id,source_batch_seq,predecessor_batch_id,
                  project_scope_id,stable_request_id,source_transaction_id,
                  member_count,member_set_hash,source_occurred_at,source_time_status
             FROM investigation_projection_outbox_batches
            WHERE operation_id=$1 AND stable_request_id=$2
            ORDER BY source_batch_seq LIMIT 1"#,
    )
    .bind(input.operation_id)
    .bind(input.stable_request_id)
    .fetch_optional(&mut *connection)
    .await?
    {
        if existing.batch_id != input.batch_id
            || existing.project_scope_id != input.project_scope_id
            || existing.source_transaction_id != input.source_transaction_id
            || existing.member_count != member_count
            || existing.member_set_hash != member_set_hash
            || existing.source_occurred_at != input.source_occurred_at
            || existing.source_time_status != input.source_time_status.as_str()
        {
            return Err(conflict(SOURCE_BATCH_REPLAY_DRIFT));
        }
        sqlx::query("SELECT pg_notify($1,$2)")
            .bind(crate::repo::investigation_projection::INVESTIGATION_PROJECTION_NOTIFY_CHANNEL)
            .bind(input.operation_id.to_string())
            .execute(&mut *connection)
            .await?;
        return Ok(ProjectionSourceBatchView {
            replayed: true,
            ..existing
        });
    }

    let source_batch_seq = last_source_batch_seq + 1;
    sqlx::query(
        r#"INSERT INTO investigation_projection_outbox_batches(
               batch_id,operation_id,project_scope_id,source_batch_seq,
               predecessor_batch_id,stable_request_id,source_transaction_id,
               member_count,member_set_hash,source_occurred_at,source_time_status
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
    )
    .bind(input.batch_id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(source_batch_seq)
    .bind(predecessor_batch_id)
    .bind(input.stable_request_id)
    .bind(input.source_transaction_id)
    .bind(member_count)
    .bind(&member_set_hash)
    .bind(input.source_occurred_at)
    .bind(input.source_time_status.as_str())
    .execute(&mut *connection)
    .await?;

    for member in prepared {
        let source_blob_id = if let (Some(bytes), Some(content_hash), Some(redaction_version)) = (
            member.blob_bytes.as_ref(),
            member.blob_hash.as_ref(),
            member.blob_redaction_contract_version.as_ref(),
        ) {
            let derived_blob_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, content_hash.as_bytes());
            sqlx::query(
                r#"INSERT INTO investigation_projection_source_blobs(
                       blob_id,content_hash,byte_count,immutable_redacted_bytes,
                       redaction_contract_version,redaction_metadata
                   ) VALUES($1,$2,$3,$4,$5,$6)
                   ON CONFLICT(payload_schema,payload_schema_version,content_hash) DO NOTHING"#,
            )
            .bind(derived_blob_id)
            .bind(content_hash)
            .bind(i64::try_from(bytes.len()).map_err(|_| conflict(SOURCE_BATCH_EMPTY))?)
            .bind(bytes)
            .bind(redaction_version)
            .bind(json!({
                "source": "typed_projection_source_snapshot.v1",
                "redacted": true,
            }))
            .execute(&mut *connection)
            .await?;
            Some(
                sqlx::query_scalar::<_, Uuid>(
                    r#"SELECT blob_id FROM investigation_projection_source_blobs
                        WHERE payload_schema='projection_source_snapshot.v1'
                          AND payload_schema_version=1 AND content_hash=$1"#,
                )
                .bind(content_hash)
                .fetch_one(&mut *connection)
                .await?,
            )
        } else {
            None
        };
        sqlx::query(
            r#"INSERT INTO investigation_projection_outbox(
                   outbox_member_id,batch_id,operation_id,source_batch_seq,member_ordinal,
                   entity_kind,change_kind,source_entity_id,source_entity_version,
                   source_entity_hash,source_occurred_at,source_time_status,
                   source_snapshot_hash,immutable_source_body,source_blob_id,source_blob_hash,
                   timeline_event_kind,invalidation_reason,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)"#,
        )
        .bind(member.outbox_member_id)
        .bind(input.batch_id)
        .bind(input.operation_id)
        .bind(source_batch_seq)
        .bind(member.member_ordinal)
        .bind(member.entity_kind)
        .bind(member.change_kind)
        .bind(&member.source_entity_id)
        .bind(member.source_entity_version)
        .bind(&member.source_entity_hash)
        .bind(member.source_occurred_at)
        .bind(member.source_time_status)
        .bind(&member.source_snapshot_hash)
        .bind(member.immutable_source_body)
        .bind(source_blob_id)
        .bind(member.blob_hash)
        .bind(member.timeline_event_kind)
        .bind(member.invalidation_reason)
        .bind(&member.member_hash)
        .execute(&mut *connection)
        .await?;
    }

    let head_advance = sqlx::query(
        r#"UPDATE investigation_projection_source_heads
              SET last_source_batch_seq=$2,last_source_batch_id=$3
            WHERE operation_id=$1 AND last_source_batch_seq=$4
              AND last_source_batch_id IS NOT DISTINCT FROM $5"#,
    )
    .bind(input.operation_id)
    .bind(source_batch_seq)
    .bind(input.batch_id)
    .bind(last_source_batch_seq)
    .bind(predecessor_batch_id)
    .execute(&mut *connection)
    .await?;
    if head_advance.rows_affected() != 1 {
        return Err(conflict("INVESTIGATION_SOURCE_HEAD_CAS_INVALID"));
    }
    // PostgreSQL delivers NOTIFY only when the surrounding canonical
    // transaction commits. A rollback therefore cannot wake the worker for a
    // batch that never became truth; polling still recovers lost notifications.
    sqlx::query("SELECT pg_notify($1,$2)")
        .bind(crate::repo::investigation_projection::INVESTIGATION_PROJECTION_NOTIFY_CHANNEL)
        .bind(input.operation_id.to_string())
        .execute(&mut *connection)
        .await?;

    Ok(ProjectionSourceBatchView {
        batch_id: input.batch_id,
        operation_id: input.operation_id,
        source_batch_seq,
        predecessor_batch_id,
        project_scope_id: input.project_scope_id,
        stable_request_id: input.stable_request_id,
        source_transaction_id: input.source_transaction_id,
        member_count,
        member_set_hash,
        source_occurred_at: input.source_occurred_at,
        source_time_status: input.source_time_status.as_str().to_owned(),
        replayed: false,
    })
}

async fn read_latest_compatibility_projection(
    pool: &PgPool,
    operation_id: Uuid,
    entity_id: Uuid,
    table: &'static str,
    entity_kind: &'static str,
) -> Result<LegacyCompatibilityRead> {
    let mut tx = pool.begin().await?;
    let contract = crate::repo::operation_rollout::source_pair_for_share(&mut tx, operation_id)
        .await
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    let sql = format!(
        r#"SELECT projection.operation_id,projection.entity_id,projection.entity_version,
                  projection.source_generation_id,projection.source_revision_id,
                  projection.source_contract_hash,projection.projection_status,
                  projection.projection_body,projection.projection_hash,
                  projection.batch_id,batch.source_batch_seq,projection.change_seq,
                  projection.invalidation_reason,entity.source_occurred_at,
                  entity.source_time_status,projection.projected_at
             FROM {table} projection
             JOIN investigation_projection_outbox_batches batch
               ON batch.batch_id=projection.batch_id
             JOIN investigation_projection_entity_versions entity
               ON entity.operation_id=projection.operation_id
              AND entity.entity_kind=$3
              AND entity.entity_id=projection.entity_id::TEXT
              AND entity.entity_version=projection.entity_version
              AND entity.batch_id=projection.batch_id
              AND entity.change_seq=projection.change_seq
            WHERE projection.operation_id=$1 AND projection.entity_id=$2
            ORDER BY projection.entity_version DESC LIMIT 1"#,
    );
    let row = sqlx::query_as::<_, LegacyCompatibilityProjectionVersion>(&sql)
        .bind(operation_id)
        .bind(entity_id)
        .bind(entity_kind)
        .fetch_optional(&mut *tx)
        .await?;
    tx.commit().await?;
    let Some(projection) = row else {
        return Ok(LegacyCompatibilityRead {
            disposition: LegacyCompatibilityReadDisposition::Hold,
            projection: None,
        });
    };
    let disposition = compatibility_read_disposition(
        contract
            .investigation_rollout_mode()
            .policy()
            .legacy_projection,
        &projection.projection_status,
    );
    Ok(LegacyCompatibilityRead {
        disposition,
        projection: Some(projection),
    })
}

/// Read the latest Candidate compatibility version. `new_only` keeps existing
/// history readable but marks it immutable; unsupported derived rows HOLD.
pub async fn read_legacy_candidate_projection(
    pool: &PgPool,
    operation_id: Uuid,
    entity_id: Uuid,
) -> Result<LegacyCompatibilityRead> {
    read_latest_compatibility_projection(
        pool,
        operation_id,
        entity_id,
        "hypothesis_legacy_candidate_projection_versions",
        "legacy_candidate_projection",
    )
    .await
}

/// Read the latest Attempt compatibility version with the same fail-closed
/// policy as Candidate projections.
pub async fn read_legacy_attempt_projection(
    pool: &PgPool,
    operation_id: Uuid,
    entity_id: Uuid,
) -> Result<LegacyCompatibilityRead> {
    read_latest_compatibility_projection(
        pool,
        operation_id,
        entity_id,
        "hypothesis_legacy_attempt_projection_versions",
        "legacy_attempt_projection",
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_core::investigation_comparison::{
        compare_whole_records_v1, WholeRecordComparisonStateV1,
    };
    use serial_test::serial;

    #[derive(sqlx::FromRow)]
    struct RealProducerComparisonSample {
        record_kind: String,
        comparison_state: String,
        legacy_hash: Option<String>,
        registry_hash: Option<String>,
        diff_summary: Value,
    }

    fn digest(nibble: char) -> String {
        format!("sha256:{}", nibble.to_string().repeat(64))
    }

    fn reserve_local_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("reserve local postgres port")
            .local_addr()
            .expect("read local postgres port")
            .port()
    }

    async fn dual_read_fixture(label: &str) -> (crate::GolishDb, tempfile::TempDir, Uuid) {
        let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
        let db = crate::GolishDb::start(crate::DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port(),
            database: format!("legacy_compare_{label}_{}", Uuid::new_v4().simple()),
            ..crate::DbConfig::default()
        })
        .await
        .expect("start isolated migrated postgres");
        let operation_id = Uuid::new_v4();
        let project_scope_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) VALUES($1,$2,$3)",
        )
        .bind(project_scope_id)
        .bind(format!("/tmp/legacy-compare-{project_scope_id}"))
        .bind(digest('1'))
        .execute(db.pool())
        .await
        .expect("insert comparison project scope");
        for statement in [
            "ALTER TABLE tool_truth_rollout DISABLE TRIGGER tool_truth_rollout_direct_mutation_guard",
            "ALTER TABLE investigation_rollout DISABLE TRIGGER investigation_rollout_direct_mutation_guard",
        ] {
            sqlx::query(statement)
                .execute(db.pool())
                .await
                .expect("disable rollout guard in isolated fixture");
        }
        sqlx::query(
            "UPDATE tool_truth_rollout SET new_operation_contract='shadow_v1',row_version=row_version+1 WHERE singleton=TRUE",
        )
        .execute(db.pool())
        .await
        .expect("install shadow Tool Truth fixture");
        sqlx::query(
            r#"UPDATE investigation_rollout
                  SET contract_version='hypothesis_registry_v1',rollout_mode='dual_read_compare',
                      mode_rank=2,row_version=row_version+1 WHERE singleton=TRUE"#,
        )
        .execute(db.pool())
        .await
        .expect("install dual-read fixture");
        sqlx::query(
            r#"INSERT INTO operation_state(
                   operation_id,profile,current_stage,runtime_memory_contract,
                   attack_execution_contract,tool_truth_contract,project_scope_id,
                   investigation_contract_version,investigation_rollout_mode)
               VALUES($1,'assessment','attack_candidate','legacy_v1','legacy','shadow_v1',$2,
                      'hypothesis_registry_v1','dual_read_compare')"#,
        )
        .bind(operation_id)
        .bind(project_scope_id)
        .execute(db.pool())
        .await
        .expect("insert dual-read operation");
        (db, data_dir, operation_id)
    }

    #[test]
    fn legacy_compatibility_read_holds_unsupported_and_preserves_history() {
        assert_eq!(
            compatibility_read_disposition(
                LegacyProjectionPolicy::CanonicalDerivedFailClosed,
                "unsupported",
            ),
            LegacyCompatibilityReadDisposition::Hold
        );
        assert_eq!(
            compatibility_read_disposition(LegacyProjectionPolicy::HistoricalReadOnly, "ready"),
            LegacyCompatibilityReadDisposition::HistoricalReadOnly
        );
        assert_eq!(
            compatibility_read_disposition(
                LegacyProjectionPolicy::HistoricalReadOnly,
                "unsupported",
            ),
            LegacyCompatibilityReadDisposition::Hold
        );
        assert_eq!(
            compatibility_read_disposition(
                LegacyProjectionPolicy::HistoricalReadOnly,
                "invalidated",
            ),
            LegacyCompatibilityReadDisposition::Hold
        );
        assert_eq!(
            compatibility_read_disposition(
                LegacyProjectionPolicy::CanonicalDerivedFailClosed,
                "ready",
            ),
            LegacyCompatibilityReadDisposition::Ready
        );
    }

    #[test]
    fn registry_shadow_adapter_fault_changes_only_the_registry_whole_record() {
        let source = LegacyCandidateShadowSourceV1 {
            entity_id: Uuid::new_v4(),
            entity_version: 1,
            organization_id: Uuid::new_v4(),
            source_work_item_id: Uuid::new_v4(),
            target_type_at_time: "service".to_owned(),
            target_value_at_time: "tcp/443".to_owned(),
            target_identity_hash: digest('2'),
            hypothesis_hash: Some(digest('3')),
            technique: Some("network_service_exposure".to_owned()),
            candidate_plan_hash: Some(digest('4')),
            disposition: "proposed".to_owned(),
        };
        let legacy = InvestigationComparisonRecordV1::compile(
            legacy_candidate_comparison_input(&source, "shadow_v1")
                .expect("serialize complete legacy record"),
        )
        .expect("compile complete legacy record");
        let faulty_registry_source = json!({
            "source_contract":"legacy_candidate_shadow.v1",
            "entity_version":source.entity_version,
            "tool_truth_contract":"shadow_v1",
            "target_type_at_time":source.target_type_at_time,
            "target_identity_hash":source.target_identity_hash,
            "hypothesis_hash":source.hypothesis_hash,
            "technique":source.technique,
            "candidate_plan_hash":source.candidate_plan_hash,
            "disposition":"blocked",
        });
        let registry = InvestigationComparisonRecordV1::compile(
            RegistryShadowAdapterV1::reduce_candidate(&faulty_registry_source)
                .expect("reduce independently faulty Registry shadow"),
        )
        .expect("compile independently faulty Registry shadow");
        let compared = compare_whole_records_v1(Some(&legacy), Some(&registry));
        assert_eq!(compared.state, WholeRecordComparisonStateV1::Mismatch);
        assert_ne!(compared.legacy_hash, compared.registry_hash);
    }

    #[tokio::test]
    #[serial]
    async fn legacy_candidate_and_attempt_producers_emit_complete_dual_records() {
        let (mut db, _data_dir, operation_id) = dual_read_fixture("producer").await;
        let candidate_id = Uuid::new_v4();
        let candidate_source = LegacyCandidateShadowSourceV1 {
            entity_id: candidate_id,
            entity_version: 1,
            organization_id: Uuid::new_v4(),
            source_work_item_id: Uuid::new_v4(),
            target_type_at_time: "service".to_owned(),
            target_value_at_time: "tcp/443".to_owned(),
            target_identity_hash: digest('5'),
            hypothesis_hash: Some(digest('6')),
            technique: Some("network_service_exposure".to_owned()),
            candidate_plan_hash: Some(digest('7')),
            disposition: "proposed".to_owned(),
        };
        let mut connection = db
            .pool()
            .acquire()
            .await
            .expect("acquire producer connection");
        let candidate_batch = append_legacy_candidate_shadow_batch_with_connection(
            &mut connection,
            InvestigationRolloutMode::DualReadCompare,
            operation_id,
            Uuid::new_v4(),
            Utc::now(),
            vec![candidate_source],
        )
        .await
        .expect("append real legacy Candidate producer batch")
        .expect("dual-read emits Candidate shadow batch");
        drop(connection);
        crate::repo::investigation_projection::project_projection_batch(
            db.pool(),
            operation_id,
            candidate_batch.batch_id,
        )
        .await
        .expect("project real legacy Candidate producer batch");

        let attempt_id = Uuid::new_v4();
        let mut connection = db
            .pool()
            .acquire()
            .await
            .expect("acquire attempt producer connection");
        let attempt_batch = append_legacy_attempt_shadow_with_connection(
            &mut connection,
            InvestigationRolloutMode::DualReadCompare,
            operation_id,
            Utc::now(),
            LegacyAttemptShadowSourceV1 {
                attempt_id,
                entity_version: 1,
                organization_id: Uuid::new_v4(),
                candidate_id,
                candidate_plan_hash: digest('7'),
                result_hash: digest('8'),
                disposition: "verified".to_owned(),
            },
        )
        .await
        .expect("append real legacy Attempt producer batch")
        .expect("dual-read emits Attempt shadow batch");
        drop(connection);
        crate::repo::investigation_projection::project_projection_batch(
            db.pool(),
            operation_id,
            attempt_batch.batch_id,
        )
        .await
        .expect("project real legacy Attempt producer batch");

        let samples: Vec<RealProducerComparisonSample> = sqlx::query_as(
            r#"SELECT record_kind,comparison_state,legacy_hash,registry_hash,diff_summary
                 FROM investigation_projection_compare_samples
                WHERE operation_id=$1 ORDER BY as_of_change_seq"#,
        )
        .bind(operation_id)
        .fetch_all(db.pool())
        .await
        .expect("load real-producer comparison samples");
        assert_eq!(samples.len(), 2);
        for sample in &samples {
            assert!(!sample.record_kind.is_empty());
            assert_eq!(sample.comparison_state, "match");
            assert!(sample.legacy_hash.is_some());
            assert_eq!(sample.legacy_hash, sample.registry_hash);
            assert_eq!(sample.diff_summary["legacy_complete"], true);
            assert_eq!(sample.diff_summary["registry_complete"], true);
            assert_eq!(sample.diff_summary["field_fallback"], false);
        }
        let serialized_bases: Vec<Value> = sqlx::query_scalar(
            r#"SELECT projection_body FROM investigation_projection_entity_versions
                WHERE operation_id=$1 ORDER BY change_seq"#,
        )
        .bind(operation_id)
        .fetch_all(db.pool())
        .await
        .expect("load materialized real-producer envelopes");
        let forbidden_plan_b_authority_fields = [
            "checked_authority",
            "knowledge_feed",
            "claim_component_member_hashes",
            "verification_contract_member_hashes",
            "verification_plan_member_hashes",
            "verification_plan_objective_member_hashes",
            "verification_plan_path_member_hashes",
            "coverage_subreview_member_hashes",
            "coverage_synthesis_member_hashes",
            "coverage_final_review_member_hashes",
            "coverage_checklist_member_hashes",
            "sampling_degraded_residual_member_hashes",
        ];
        assert!(serialized_bases.iter().all(|body| {
            let text = body.to_string();
            text.contains("grandfathered_legacy")
                && forbidden_plan_b_authority_fields
                    .iter()
                    .all(|field| !text.contains(&format!("\"{field}\"")))
        }));
        db.stop().await;
    }
}
