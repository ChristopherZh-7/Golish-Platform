use chrono::{DateTime, Utc};
use golish_core::hypothesis_semantic_key::CanonicalJsonObject;
use golish_core::investigation_projection::{
    projection_change_hash_v1, projection_entity_hash_v1, projection_event_id_v1,
    LegacyAttemptProjectionRecordV1, LegacyCandidateProjectionRecordV1,
    PersistedProjectionChangeV1, ProjectionChangeKind, ProjectionEntityKind, ProjectionEntityV1,
    ProjectionInvalidationReason, ProjectionSourceSnapshotV1, ProjectionSourceTimeStatusV1,
    TimelineEventKind,
};
use golish_core::{ComparePolicy, InvestigationRolloutMode, LegacyProjectionPolicy};
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::types::{
    sha256_bytes, sha256_json, CapturedProjectionHead, InvestigationProjectionChange,
    InvestigationProjectionError, InvestigationProjectionResult, MaterializedProjectionEntity,
    ProjectionBatchClaim, ProjectionBatchReceipt, ProjectionProjectOutcome, ProjectionReadPage,
};

const BATCH_EXACT_SET_INVALID: &str = "INVESTIGATION_PROJECTION_BATCH_EXACT_SET_INVALID";
const SOURCE_SNAPSHOT_INVALID: &str = "INVESTIGATION_PROJECTION_SOURCE_SNAPSHOT_INVALID";
const ENTITY_PREDECESSOR_INVALID: &str = "INVESTIGATION_PROJECTION_ENTITY_PREDECESSOR_INVALID";
const HEAD_CAS_INVALID: &str = "INVESTIGATION_PROJECTION_HEAD_CAS_INVALID";
const CATALOG_INVALID: &str = "INVESTIGATION_PROJECTION_CATALOG_INVALID";

#[derive(Debug, sqlx::FromRow)]
struct BatchRow {
    batch_id: Uuid,
    operation_id: Uuid,
    source_batch_seq: i64,
    predecessor_batch_id: Option<Uuid>,
    source_transaction_id: Uuid,
    member_count: i64,
    member_set_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
struct CompatibilityAuthorityRow {
    generation_id: Uuid,
    revision_id: Uuid,
    revision_hash: String,
    revision_ingredients_hash: String,
    subject_kind: String,
    predicate_schema: String,
    plan_id: Option<Uuid>,
    plan_hash: Option<String>,
    required_claim_component_count: Option<i64>,
    actual_claim_component_count: i64,
    objective_count: Option<i64>,
    actual_objective_count: i64,
    actual_contract_count: i64,
    actual_plan_objective_count: i64,
    proof_path_count: Option<i64>,
    actual_proof_path_count: i64,
    legacy_work_item_source_count: i64,
    forbidden_source_count: i64,
}

#[derive(Debug, Clone, Copy)]
enum CompatibilityKind {
    Candidate,
    Attempt,
}

impl CompatibilityKind {
    const fn table(self) -> &'static str {
        match self {
            Self::Candidate => "hypothesis_legacy_candidate_projection_versions",
            Self::Attempt => "hypothesis_legacy_attempt_projection_versions",
        }
    }

    const fn id_column(self) -> &'static str {
        match self {
            Self::Candidate => "legacy_candidate_projection_id",
            Self::Attempt => "legacy_attempt_projection_id",
        }
    }

    const fn id_domain(self) -> &'static [u8] {
        match self {
            Self::Candidate => b"legacy_candidate_projection.v1",
            Self::Attempt => b"legacy_attempt_projection.v1",
        }
    }
}

#[derive(Debug)]
struct CompatibilityWrite {
    kind: CompatibilityKind,
    projection_id: Uuid,
    source_generation_id: Uuid,
    source_revision_id: Uuid,
    source_contract_hash: String,
    projection_status: &'static str,
    projection_body: Option<Value>,
}

#[derive(Debug)]
struct PreparedProjection<'a> {
    source: &'a OutboxRow,
    entity_kind: ProjectionEntityKind,
    entity_id: String,
    entity_version: i64,
    source_hash: String,
    entity: ProjectionEntityV1,
    change_kind: ProjectionChangeKind,
    timeline_event_kind: TimelineEventKind,
    invalidation_reason: Option<ProjectionInvalidationReason>,
    compatibility: Option<CompatibilityWrite>,
}

impl From<&BatchRow> for ProjectionBatchClaim {
    fn from(value: &BatchRow) -> Self {
        Self {
            batch_id: value.batch_id,
            operation_id: value.operation_id,
            source_batch_seq: value.source_batch_seq,
            predecessor_batch_id: value.predecessor_batch_id,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct OutboxRow {
    outbox_member_id: Uuid,
    member_ordinal: i32,
    entity_kind: String,
    change_kind: String,
    source_entity_id: String,
    source_entity_version: i64,
    source_entity_hash: String,
    source_occurred_at: Option<DateTime<Utc>>,
    source_time_status: String,
    source_snapshot_hash: String,
    immutable_source_body: Option<Value>,
    source_blob_id: Option<Uuid>,
    source_blob_hash: Option<String>,
    timeline_event_kind: String,
    invalidation_reason: Option<String>,
    member_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
struct EntityReadRow {
    entity_kind: String,
    entity_id: String,
    entity_version: i64,
    projection_hash: String,
    projection_body: Value,
    change_seq: i64,
    invalidation_reason: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct ChangeReadRow {
    change_seq: i64,
    event_id: Uuid,
    batch_id: Uuid,
    source_batch_seq: i64,
    outbox_member_id: Uuid,
    entity_kind: String,
    entity_id: String,
    entity_version: i64,
    change_kind: String,
    timeline_event_kind: String,
    invalidation_reason: Option<String>,
    change_hash: String,
    source_occurred_at: Option<DateTime<Utc>>,
    source_time_status: String,
    projected_at: DateTime<Utc>,
}

fn contract(code: &'static str) -> InvestigationProjectionError {
    InvestigationProjectionError::Contract(code)
}

fn retryable_projection_contention(error: &InvestigationProjectionError) -> bool {
    matches!(
        error,
        InvestigationProjectionError::Storage(sqlx::Error::Database(database))
            if matches!(database.code().as_deref(), Some("40001" | "40P01"))
    )
}

fn parse_entity_kind(value: &str) -> InvestigationProjectionResult<ProjectionEntityKind> {
    ProjectionEntityKind::try_from(value).map_err(|_| contract(CATALOG_INVALID))
}

fn parse_change_kind(value: &str) -> InvestigationProjectionResult<ProjectionChangeKind> {
    ProjectionChangeKind::try_from(value).map_err(|_| contract(CATALOG_INVALID))
}

fn parse_timeline_kind(value: &str) -> InvestigationProjectionResult<TimelineEventKind> {
    TimelineEventKind::try_from(value).map_err(|_| contract(CATALOG_INVALID))
}

fn parse_source_time_status(
    value: &str,
) -> InvestigationProjectionResult<ProjectionSourceTimeStatusV1> {
    ProjectionSourceTimeStatusV1::try_from(value).map_err(|_| contract(CATALOG_INVALID))
}

fn parse_invalidation_reason(
    value: Option<&str>,
) -> InvestigationProjectionResult<Option<ProjectionInvalidationReason>> {
    value
        .map(|value| {
            ProjectionInvalidationReason::try_from(value).map_err(|_| contract(CATALOG_INVALID))
        })
        .transpose()
}

async fn load_snapshot(
    tx: &mut Transaction<'_, Postgres>,
    row: &OutboxRow,
) -> InvestigationProjectionResult<ProjectionSourceSnapshotV1> {
    let body = match (&row.immutable_source_body, row.source_blob_id) {
        (Some(body), None) if row.source_blob_hash.is_none() => body.clone(),
        (None, Some(blob_id)) => {
            let (content_hash, bytes): (String, Vec<u8>) = sqlx::query_as(
                r#"SELECT content_hash,immutable_redacted_bytes
                     FROM investigation_projection_source_blobs
                    WHERE blob_id=$1 FOR SHARE"#,
            )
            .bind(blob_id)
            .fetch_one(&mut **tx)
            .await?;
            if row.source_blob_hash.as_deref() != Some(content_hash.as_str())
                || sha256_bytes(&bytes) != content_hash
            {
                return Err(contract(SOURCE_SNAPSHOT_INVALID));
            }
            serde_json::from_slice(&bytes)?
        }
        _ => return Err(contract(SOURCE_SNAPSHOT_INVALID)),
    };
    if sha256_json(&body)? != row.source_snapshot_hash {
        return Err(contract(SOURCE_SNAPSHOT_INVALID));
    }
    let snapshot: ProjectionSourceSnapshotV1 = serde_json::from_value(body)?;
    if snapshot.entity_kind().as_str() != row.entity_kind {
        return Err(contract(SOURCE_SNAPSHOT_INVALID));
    }
    let record = snapshot.record();
    if record.entity_id() != row.source_entity_id
        || i64::try_from(record.entity_version()).ok() != Some(row.source_entity_version)
        || record.content_sha256() != row.source_entity_hash
    {
        return Err(contract(SOURCE_SNAPSHOT_INVALID));
    }
    Ok(snapshot)
}

fn uuid_field(body: &Value, field: &'static str) -> InvestigationProjectionResult<Uuid> {
    body.get(field)
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| contract(SOURCE_SNAPSHOT_INVALID))
}

async fn load_compatibility_authority(
    tx: &mut Transaction<'_, Postgres>,
    batch: &BatchRow,
    revision_id: Uuid,
) -> InvestigationProjectionResult<Option<CompatibilityAuthorityRow>> {
    Ok(sqlx::query_as::<_, CompatibilityAuthorityRow>(
        r#"SELECT member.generation_id,revision.revision_id,revision.revision_hash,
                  revision.revision_ingredients_hash,revision.subject_kind,
                  revision.predicate_schema,plan.plan_id,plan.plan_hash,
                  plan.required_claim_component_count,
                  (SELECT COUNT(*) FROM attack_hypothesis_claim_components component
                    WHERE component.revision_id=revision.revision_id) AS actual_claim_component_count,
                  plan.objective_count,
                  (SELECT COUNT(*) FROM attack_hypothesis_verification_objectives objective
                    WHERE objective.revision_id=revision.revision_id) AS actual_objective_count,
                  (SELECT COUNT(*) FROM attack_hypothesis_verification_contracts contract
                    WHERE contract.revision_id=revision.revision_id) AS actual_contract_count,
                  (SELECT COUNT(*) FROM attack_hypothesis_verification_plan_objectives objective
                    WHERE objective.revision_id=revision.revision_id) AS actual_plan_objective_count,
                  plan.proof_path_count,
                  (SELECT COUNT(*) FROM attack_hypothesis_verification_plan_paths path
                    WHERE path.plan_id=plan.plan_id) AS actual_proof_path_count,
                  (SELECT COUNT(*)
                     FROM input_hypothesis_relations relation
                     JOIN candidate_analysis_snapshot_inputs input
                       ON input.snapshot_input_id=relation.snapshot_input_id
                     JOIN attack_candidate_work_items work_item
                       ON work_item.id::TEXT=input.source_ref
                      AND work_item.operation_id=revision.operation_id
                      AND work_item.organization_id=revision.organization_id
                    WHERE relation.revision_id=revision.revision_id
                      AND input.snapshot_id=generation.candidate_snapshot_id
                      AND input.source_kind='legacy_attack_candidate_work_item.v1')
                    AS legacy_work_item_source_count,
                  (SELECT COUNT(*) FROM attack_hypothesis_revision_sources source
                    WHERE source.revision_id=revision.revision_id
                      AND source.source_role IN ('application_context','knowledge_signal','gap'))
                    AS forbidden_source_count
             FROM hypothesis_generations generation
             JOIN hypothesis_generation_members member
               ON member.generation_id=generation.generation_id
             JOIN attack_hypothesis_revisions revision
               ON revision.revision_id=member.revision_id
             LEFT JOIN attack_hypothesis_verification_plans plan
               ON plan.revision_id=revision.revision_id
            WHERE generation.candidate_gate_decision_id=$1
              AND member.revision_id=$2
            FOR SHARE OF generation,member,revision"#,
    )
    .bind(batch.source_transaction_id)
    .bind(revision_id)
    .fetch_optional(&mut **tx)
    .await?)
}

fn compatibility_source_contract_hash(
    authority: Option<&CompatibilityAuthorityRow>,
    source_hash: &str,
) -> InvestigationProjectionResult<String> {
    sha256_json(&json!({
        "schema": "derived_legacy_compatibility_authority.v1",
        "source_hash": source_hash,
        "generation_id": authority.map(|value| value.generation_id),
        "revision_id": authority.map(|value| value.revision_id),
        "revision_hash": authority.map(|value| value.revision_hash.as_str()),
        "revision_ingredients_hash": authority
            .map(|value| value.revision_ingredients_hash.as_str()),
        "subject_kind": authority.map(|value| value.subject_kind.as_str()),
        "predicate_schema": authority.map(|value| value.predicate_schema.as_str()),
        "plan_id": authority.and_then(|value| value.plan_id),
        "plan_hash": authority.and_then(|value| value.plan_hash.as_deref()),
        "required_claim_component_count": authority
            .and_then(|value| value.required_claim_component_count),
        "actual_claim_component_count": authority
            .map(|value| value.actual_claim_component_count),
        "objective_count": authority.and_then(|value| value.objective_count),
        "actual_objective_count": authority.map(|value| value.actual_objective_count),
        "actual_contract_count": authority.map(|value| value.actual_contract_count),
        "actual_plan_objective_count": authority.map(|value| value.actual_plan_objective_count),
        "proof_path_count": authority.and_then(|value| value.proof_path_count),
        "actual_proof_path_count": authority.map(|value| value.actual_proof_path_count),
        "legacy_work_item_source_count": authority
            .map(|value| value.legacy_work_item_source_count),
        "forbidden_source_count": authority.map(|value| value.forbidden_source_count),
    }))
}

fn candidate_is_legacy_ready(authority: &CompatibilityAuthorityRow) -> bool {
    authority.plan_id.is_some()
        && authority.plan_hash.is_some()
        && authority.required_claim_component_count == Some(authority.actual_claim_component_count)
        && authority.actual_claim_component_count > 0
        && authority.objective_count == Some(authority.actual_objective_count)
        && authority.actual_objective_count > 0
        && authority.actual_contract_count == authority.actual_objective_count
        && authority.actual_plan_objective_count == authority.actual_objective_count
        && authority.proof_path_count == Some(authority.actual_proof_path_count)
        && authority.actual_proof_path_count > 0
        && authority.subject_kind == "attack_candidate"
        && authority.predicate_schema == "legacy_attack_candidate.v1"
        && authority.legacy_work_item_source_count == 1
        && authority.forbidden_source_count == 0
}

fn build_compatibility_projection<'a>(
    source: &'a OutboxRow,
    kind: CompatibilityKind,
    root_id: Uuid,
    revision_id: Uuid,
    authority: Option<&CompatibilityAuthorityRow>,
) -> InvestigationProjectionResult<PreparedProjection<'a>> {
    let authority_complete = authority.is_some_and(|value| {
        value.plan_id.is_some()
            && value.plan_hash.is_some()
            && value.required_claim_component_count == Some(value.actual_claim_component_count)
            && value.actual_claim_component_count > 0
            && value.objective_count == Some(value.actual_objective_count)
            && value.actual_objective_count > 0
            && value.actual_contract_count == value.actual_objective_count
            && value.actual_plan_objective_count == value.actual_objective_count
            && value.proof_path_count == Some(value.actual_proof_path_count)
            && value.actual_proof_path_count > 0
    });
    let candidate_ready = matches!(kind, CompatibilityKind::Candidate)
        && authority.is_some_and(candidate_is_legacy_ready);
    let invalidation = if candidate_ready {
        None
    } else if !authority_complete {
        Some((
            ProjectionInvalidationReason::LegacyProjectionDerivationFailed,
            "plan_b_authority_incomplete",
        ))
    } else {
        match kind {
            CompatibilityKind::Candidate => Some((
                ProjectionInvalidationReason::LegacyProjectionUnsupported,
                if authority.is_some_and(|value| value.forbidden_source_count > 0) {
                    "non_legacy_source_authority"
                } else if authority.is_some_and(|value| {
                    value.subject_kind != "attack_candidate"
                        || value.predicate_schema != "legacy_attack_candidate.v1"
                }) {
                    "old_classifier_incompatible"
                } else {
                    "legacy_work_item_authority_missing"
                },
            )),
            CompatibilityKind::Attempt => Some((
                ProjectionInvalidationReason::LegacyProjectionUnsupported,
                "not_available_plan_c",
            )),
        }
    };
    let source_contract_hash =
        compatibility_source_contract_hash(authority, &source.source_entity_hash)?;
    let compatibility_kind = match kind {
        CompatibilityKind::Candidate => "candidate",
        CompatibilityKind::Attempt => "attempt",
    };
    let projection_status = if candidate_ready {
        "ready"
    } else {
        "unsupported"
    };
    let body = CanonicalJsonObject::try_from_value(json!({
        "projection_schema_version": 1,
        "projection_authority": "derived_compatibility",
        "read_only": true,
        "compatibility_kind": compatibility_kind,
        "projection_status": projection_status,
        "invalidation_reason": invalidation.map(|value| value.0.as_str()),
        "reason_code": invalidation.map(|value| value.1),
        "source_generation_id": authority.map(|value| value.generation_id),
        "source_revision_id": revision_id,
        "source_revision_hash": authority.map(|value| value.revision_hash.as_str()),
        "source_revision_ingredients_hash": authority
            .map(|value| value.revision_ingredients_hash.as_str()),
        "source_contract_hash": source_contract_hash.clone(),
        "claim_component_count": authority.map(|value| value.actual_claim_component_count),
        "verification_objective_count": authority.map(|value| value.actual_objective_count),
        "verification_contract_count": authority.map(|value| value.actual_contract_count),
        "verification_proof_path_count": authority.map(|value| value.actual_proof_path_count),
        "legacy_work_item_source_count": authority.map(|value| value.legacy_work_item_source_count),
        "verification_plan_id": authority.and_then(|value| value.plan_id),
        "verification_plan_hash": authority.and_then(|value| value.plan_hash.as_deref()),
        "plan_c_authority": "not_available_plan_c",
    }))
    .map_err(|_| contract(CATALOG_INVALID))?;
    let compatibility_projection_body = candidate_ready.then(|| body.as_value().clone());
    let (entity_kind, entity) = match kind {
        CompatibilityKind::Candidate => {
            let record = LegacyCandidateProjectionRecordV1::try_new(
                root_id.to_string(),
                u64::try_from(source.source_entity_version)
                    .map_err(|_| contract(CATALOG_INVALID))?,
                1,
                body,
            )
            .map_err(|_| contract(CATALOG_INVALID))?;
            (
                ProjectionEntityKind::LegacyCandidateProjection,
                ProjectionEntityV1::LegacyCandidateProjection(record),
            )
        }
        CompatibilityKind::Attempt => {
            let record = LegacyAttemptProjectionRecordV1::try_new(
                root_id.to_string(),
                u64::try_from(source.source_entity_version)
                    .map_err(|_| contract(CATALOG_INVALID))?,
                1,
                body,
            )
            .map_err(|_| contract(CATALOG_INVALID))?;
            (
                ProjectionEntityKind::LegacyAttemptProjection,
                ProjectionEntityV1::LegacyAttemptProjection(record),
            )
        }
    };
    let change_kind = if candidate_ready {
        ProjectionChangeKind::Insert
    } else {
        ProjectionChangeKind::Invalidate
    };
    let timeline_event_kind =
        golish_core::investigation_projection::projection_timeline_event_kind(
            entity_kind,
            change_kind,
        )
        .ok_or_else(|| contract(CATALOG_INVALID))?;
    let derived_source_hash =
        projection_entity_hash_v1(&entity).map_err(|_| contract(CATALOG_INVALID))?;
    Ok(PreparedProjection {
        source,
        entity_kind,
        entity_id: root_id.to_string(),
        entity_version: source.source_entity_version,
        source_hash: derived_source_hash,
        entity,
        change_kind,
        timeline_event_kind,
        invalidation_reason: invalidation.map(|value| value.0),
        compatibility: authority.map(|value| CompatibilityWrite {
            kind,
            projection_id: Uuid::new_v5(&revision_id, kind.id_domain()),
            source_generation_id: value.generation_id,
            source_revision_id: value.revision_id,
            source_contract_hash,
            projection_status,
            projection_body: compatibility_projection_body,
        }),
    })
}

async fn prepare_projection_outputs<'a>(
    tx: &mut Transaction<'_, Postgres>,
    mode: InvestigationRolloutMode,
    batch: &BatchRow,
    source: &'a OutboxRow,
) -> InvestigationProjectionResult<Vec<PreparedProjection<'a>>> {
    let snapshot = load_snapshot(tx, source).await?;
    let entity_kind = parse_entity_kind(&source.entity_kind)?;
    let change_kind = parse_change_kind(&source.change_kind)?;
    let timeline_event_kind = parse_timeline_kind(&source.timeline_event_kind)?;
    let source_time_status = parse_source_time_status(&source.source_time_status)?;
    let invalidation_reason = parse_invalidation_reason(source.invalidation_reason.as_deref())?;
    if (change_kind == ProjectionChangeKind::Invalidate) != invalidation_reason.is_some()
        || (source_time_status == ProjectionSourceTimeStatusV1::Known)
            != source.source_occurred_at.is_some()
    {
        return Err(contract(CATALOG_INVALID));
    }
    let mut outputs = vec![PreparedProjection {
        source,
        entity_kind,
        entity_id: source.source_entity_id.clone(),
        entity_version: source.source_entity_version,
        source_hash: source.source_entity_hash.clone(),
        entity: ProjectionEntityV1::from(snapshot.clone()),
        change_kind,
        timeline_event_kind,
        invalidation_reason,
        compatibility: None,
    }];

    if mode.policy().legacy_projection != LegacyProjectionPolicy::CanonicalDerivedFailClosed {
        return Ok(outputs);
    }
    let ProjectionSourceSnapshotV1::Hypothesis(hypothesis) = snapshot else {
        return Ok(outputs);
    };
    let body = hypothesis.record().canonical_redacted_body().as_value();
    let root_id = uuid_field(body, "root_id")?;
    let revision_id = uuid_field(body, "revision_id")?;
    if root_id.to_string() != source.source_entity_id {
        return Err(contract(SOURCE_SNAPSHOT_INVALID));
    }
    let authority = load_compatibility_authority(tx, batch, revision_id).await?;
    outputs.push(build_compatibility_projection(
        source,
        CompatibilityKind::Candidate,
        root_id,
        revision_id,
        authority.as_ref(),
    )?);
    outputs.push(build_compatibility_projection(
        source,
        CompatibilityKind::Attempt,
        root_id,
        revision_id,
        authority.as_ref(),
    )?);
    Ok(outputs)
}

async fn validate_batch_exact_set(
    tx: &mut Transaction<'_, Postgres>,
    batch: &BatchRow,
    members: &[OutboxRow],
) -> InvestigationProjectionResult<()> {
    if i64::try_from(members.len()).ok() != Some(batch.member_count)
        || members
            .iter()
            .enumerate()
            .any(|(ordinal, row)| usize::try_from(row.member_ordinal).ok() != Some(ordinal))
    {
        return Err(contract(BATCH_EXACT_SET_INVALID));
    }
    for (ordinal, row) in members.iter().enumerate() {
        let storage = if row.source_blob_hash.is_some() {
            "blob"
        } else {
            "inline"
        };
        let expected = sha256_json(&json!({
            "domain": "investigation_projection_outbox_member.v1",
            "ordinal": ordinal,
            "entity_kind": row.entity_kind,
            "change_kind": row.change_kind,
            "source_entity_id": row.source_entity_id,
            "source_entity_version": row.source_entity_version,
            "source_entity_hash": row.source_entity_hash,
            "source_snapshot_hash": row.source_snapshot_hash,
            "source_time_status": row.source_time_status,
            "source_occurred_at": row.source_occurred_at,
            "timeline_event_kind": row.timeline_event_kind,
            "invalidation_reason": row.invalidation_reason,
            "storage": storage,
            "source_blob_hash": row.source_blob_hash,
        }))?;
        if expected != row.member_hash {
            return Err(contract(BATCH_EXACT_SET_INVALID));
        }
    }
    let member_hashes = members
        .iter()
        .map(|member| member.member_hash.clone())
        .collect::<Vec<_>>();
    let member_set_hash: String =
        sqlx::query_scalar("SELECT tool_truth_sha256(to_jsonb($1::TEXT[])::TEXT)")
            .bind(member_hashes)
            .fetch_one(&mut **tx)
            .await?;
    if member_set_hash != batch.member_set_hash {
        return Err(contract(BATCH_EXACT_SET_INVALID));
    }
    Ok(())
}

pub async fn claim_next_projection_batch(
    pool: &PgPool,
    operation_id: Uuid,
) -> InvestigationProjectionResult<Option<ProjectionBatchClaim>> {
    let row = sqlx::query_as::<_, ProjectionBatchClaim>(
        r#"SELECT b.batch_id,b.operation_id,b.source_batch_seq,b.predecessor_batch_id
             FROM investigation_projection_outbox_batches b
             LEFT JOIN investigation_projection_batch_receipts r ON r.batch_id=b.batch_id
             JOIN operation_state state ON state.operation_id=b.operation_id
            WHERE b.operation_id=$1 AND (
                  r.batch_id IS NULL OR (
                    state.investigation_rollout_mode IN (
                      'shadow_registry','dual_read_compare',
                      'registry_authoritative_legacy_projection'
                    ) AND EXISTS(
                      SELECT 1 FROM investigation_projection_outbox member
                       WHERE member.batch_id=b.batch_id
                         AND NOT EXISTS(
                           SELECT 1 FROM investigation_projection_compare_samples sample
                            WHERE sample.operation_id=b.operation_id
                              AND sample.as_of_change_seq=r.last_change_seq
                              AND sample.record_kind=member.entity_kind
                              AND sample.record_key=(member.source_entity_id || ':v' ||
                                                     member.source_entity_version::TEXT)
                         )
                    )
                  ))
            ORDER BY b.source_batch_seq
            LIMIT 1"#,
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn project_next_projection_batch(
    pool: &PgPool,
    operation_id: Uuid,
) -> InvestigationProjectionResult<Option<ProjectionProjectOutcome>> {
    let Some(claim) = claim_next_projection_batch(pool, operation_id).await? else {
        return Ok(None);
    };
    project_projection_batch(pool, operation_id, claim.batch_id)
        .await
        .map(Some)
}

pub async fn project_projection_batch(
    pool: &PgPool,
    operation_id: Uuid,
    batch_id: Uuid,
) -> InvestigationProjectionResult<ProjectionProjectOutcome> {
    const MAX_PREDECESSOR_CHAIN: usize = 128;

    // A later worker never waits while holding the projection head. Each
    // one-shot attempt commits or rolls back before we yield and follow the
    // immutable predecessor link. This also lets a worker that arrived early
    // finish its requested batch without relying on a second scheduler tick.
    let requested_batch_id = batch_id;
    let mut current_batch_id = batch_id;
    let mut successors = Vec::new();
    for _ in 0..MAX_PREDECESSOR_CHAIN {
        let outcome =
            match project_projection_batch_once(pool, operation_id, current_batch_id).await {
                Ok(outcome) => outcome,
                Err(error) if retryable_projection_contention(&error) => {
                    tokio::task::yield_now().await;
                    continue;
                }
                Err(error) => return Err(error),
            };
        match outcome {
            outcome @ (ProjectionProjectOutcome::Applied(_)
            | ProjectionProjectOutcome::Replay(_)) => {
                if current_batch_id == requested_batch_id && successors.is_empty() {
                    let as_of_change_seq = match &outcome {
                        ProjectionProjectOutcome::Applied(receipt)
                        | ProjectionProjectOutcome::Replay(receipt) => receipt.last_change_seq,
                        ProjectionProjectOutcome::PredecessorPending(_) => unreachable!(),
                    };
                    record_batch_comparisons(
                        pool,
                        operation_id,
                        requested_batch_id,
                        as_of_change_seq,
                    )
                    .await?;
                    return Ok(outcome);
                }
                current_batch_id = successors
                    .pop()
                    .ok_or_else(|| contract(BATCH_EXACT_SET_INVALID))?;
            }
            ProjectionProjectOutcome::PredecessorPending(claim) => {
                let predecessor_batch_id = claim
                    .predecessor_batch_id
                    .ok_or_else(|| contract(BATCH_EXACT_SET_INVALID))?;
                if predecessor_batch_id == current_batch_id
                    || successors.contains(&predecessor_batch_id)
                {
                    return Err(contract(BATCH_EXACT_SET_INVALID));
                }
                successors.push(current_batch_id);
                current_batch_id = predecessor_batch_id;
            }
        }
        tokio::task::yield_now().await;
    }
    Err(contract(BATCH_EXACT_SET_INVALID))
}

async fn record_batch_comparisons(
    pool: &PgPool,
    operation_id: Uuid,
    batch_id: Uuid,
    as_of_change_seq: i64,
) -> InvestigationProjectionResult<()> {
    let frozen: (String, String) = sqlx::query_as(
        r#"SELECT investigation_contract_version,investigation_rollout_mode
             FROM operation_state WHERE operation_id=$1"#,
    )
    .bind(operation_id)
    .fetch_one(pool)
    .await?;
    let (_, mode) = crate::repo::investigation_rollout::parse_frozen_pair(&frozen.0, &frozen.1)
        .map_err(|_| contract(CATALOG_INVALID))?;
    if mode.policy().compare_policy == ComparePolicy::Off {
        return Ok(());
    }
    let source_members: Vec<(String, String, i64)> = sqlx::query_as(
        r#"SELECT entity_kind,source_entity_id,source_entity_version
             FROM investigation_projection_outbox
            WHERE batch_id=$1 AND operation_id=$2
            ORDER BY member_ordinal"#,
    )
    .bind(batch_id)
    .bind(operation_id)
    .fetch_all(pool)
    .await?;
    for (entity_kind, entity_id, entity_version) in source_members {
        let (legacy, registry) = super::comparison::assemble_frozen_comparison_records_v1(
            pool,
            operation_id,
            batch_id,
            &entity_kind,
            &entity_id,
            entity_version,
        )
        .await?;
        super::comparison::compare_and_record_v1(
            pool,
            super::comparison::CompareAndRecordV1Input {
                operation_id,
                organization_id: None,
                as_of_change_seq,
                record_kind: entity_kind,
                record_key: format!("{entity_id}:v{entity_version}"),
                legacy,
                registry,
            },
        )
        .await?;
    }
    Ok(())
}

async fn project_projection_batch_once(
    pool: &PgPool,
    operation_id: Uuid,
    batch_id: Uuid,
) -> InvestigationProjectionResult<ProjectionProjectOutcome> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await?;
    let head = sqlx::query_as::<_, CapturedProjectionHead>(
        r#"SELECT operation_id,projection_schema_version,change_seq,
                  last_projected_batch_id,cursor_salt
             FROM investigation_projection_heads
            WHERE operation_id=$1 FOR UPDATE"#,
    )
    .bind(operation_id)
    .fetch_one(&mut *tx)
    .await?;
    if let Some(receipt) = sqlx::query_as::<_, ProjectionBatchReceipt>(
        r#"SELECT receipt_id,batch_id,operation_id,source_batch_seq,predecessor_batch_id,
                  first_change_seq,last_change_seq,entity_version_manifest_hash,
                  change_manifest_hash,timeline_manifest_hash,projected_at
             FROM investigation_projection_batch_receipts
            WHERE batch_id=$1 AND operation_id=$2"#,
    )
    .bind(batch_id)
    .bind(operation_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(ProjectionProjectOutcome::Replay(receipt));
    }
    let batch = sqlx::query_as::<_, BatchRow>(
        r#"SELECT batch_id,operation_id,source_batch_seq,predecessor_batch_id,
                  source_transaction_id,member_count,member_set_hash
             FROM investigation_projection_outbox_batches
            WHERE batch_id=$1 AND operation_id=$2 FOR UPDATE"#,
    )
    .bind(batch_id)
    .bind(operation_id)
    .fetch_one(&mut *tx)
    .await?;
    if batch.predecessor_batch_id != head.last_projected_batch_id {
        let claim = ProjectionBatchClaim::from(&batch);
        tx.rollback().await?;
        return Ok(ProjectionProjectOutcome::PredecessorPending(claim));
    }
    let expected_source_seq = if head.last_projected_batch_id.is_some() {
        sqlx::query_scalar::<_, i64>(
            "SELECT source_batch_seq+1 FROM investigation_projection_batch_receipts WHERE batch_id=$1",
        )
        .bind(head.last_projected_batch_id)
        .fetch_one(&mut *tx)
        .await?
    } else {
        1
    };
    if batch.source_batch_seq != expected_source_seq {
        let claim = ProjectionBatchClaim::from(&batch);
        tx.rollback().await?;
        return Ok(ProjectionProjectOutcome::PredecessorPending(claim));
    }
    let members = sqlx::query_as::<_, OutboxRow>(
        r#"SELECT outbox_member_id,member_ordinal,entity_kind,change_kind,
                  source_entity_id,source_entity_version,source_entity_hash,
                  source_occurred_at,source_time_status,source_snapshot_hash,
                  immutable_source_body,source_blob_id,source_blob_hash,
                  timeline_event_kind,invalidation_reason,member_hash
             FROM investigation_projection_outbox
            WHERE batch_id=$1 ORDER BY member_ordinal FOR UPDATE"#,
    )
    .bind(batch_id)
    .fetch_all(&mut *tx)
    .await?;
    validate_batch_exact_set(&mut tx, &batch, &members).await?;

    let (investigation_contract, investigation_mode): (String, String) = sqlx::query_as(
        r#"SELECT investigation_contract_version,investigation_rollout_mode
             FROM operation_state WHERE operation_id=$1 FOR SHARE"#,
    )
    .bind(operation_id)
    .fetch_one(&mut *tx)
    .await?;
    let (_, investigation_mode) = crate::repo::investigation_rollout::parse_frozen_pair(
        &investigation_contract,
        &investigation_mode,
    )
    .map_err(|_| contract(CATALOG_INVALID))?;

    let mut projections = Vec::with_capacity(members.len());
    for member in &members {
        projections
            .extend(prepare_projection_outputs(&mut tx, investigation_mode, &batch, member).await?);
    }

    let projected_at: DateTime<Utc> = sqlx::query_scalar("SELECT transaction_timestamp()")
        .fetch_one(&mut *tx)
        .await?;
    let first_change_seq = head.change_seq + 1;
    let mut entity_manifest = Vec::with_capacity(projections.len());
    let mut change_manifest = Vec::with_capacity(projections.len());
    let mut timeline_manifest = Vec::with_capacity(projections.len());

    for (offset, projection) in projections.iter().enumerate() {
        let projection_body = serde_json::to_value(&projection.entity)?;
        let projection_hash =
            projection_entity_hash_v1(&projection.entity).map_err(|_| contract(CATALOG_INVALID))?;
        let predecessor = sqlx::query_as::<_, (i64, String)>(
            r#"SELECT entity_version,projection_hash
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind=$2 AND entity_id=$3
                ORDER BY entity_version DESC LIMIT 1 FOR UPDATE"#,
        )
        .bind(operation_id)
        .bind(projection.entity_kind.as_str())
        .bind(&projection.entity_id)
        .fetch_optional(&mut *tx)
        .await?;
        let (predecessor_absent, predecessor_version, predecessor_hash) =
            match (projection.entity_version, predecessor) {
                (1, None) => (true, None, None),
                (version, Some((previous_version, previous_hash)))
                    if version > 1 && previous_version == version - 1 =>
                {
                    (false, Some(previous_version), Some(previous_hash))
                }
                _ => return Err(contract(ENTITY_PREDECESSOR_INVALID)),
            };
        let change_seq = first_change_seq
            + i64::try_from(offset).map_err(|_| contract(BATCH_EXACT_SET_INVALID))?;
        sqlx::query(
            r#"INSERT INTO investigation_projection_entity_versions(
                   operation_id,entity_kind,entity_id,entity_version,batch_id,source_hash,
                   projection_hash,projection_body,predecessor_absent,
                   predecessor_entity_version,predecessor_projection_hash,change_seq,
                   source_occurred_at,source_time_status,projected_at,invalidation_reason
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)"#,
        )
        .bind(operation_id)
        .bind(projection.entity_kind.as_str())
        .bind(&projection.entity_id)
        .bind(projection.entity_version)
        .bind(batch_id)
        .bind(&projection.source_hash)
        .bind(&projection_hash)
        .bind(&projection_body)
        .bind(predecessor_absent)
        .bind(predecessor_version)
        .bind(predecessor_hash.as_deref())
        .bind(change_seq)
        .bind(projection.source.source_occurred_at)
        .bind(&projection.source.source_time_status)
        .bind(projected_at)
        .bind(projection.invalidation_reason.map(|value| value.as_str()))
        .execute(&mut *tx)
        .await?;

        if let Some(compatibility) = &projection.compatibility {
            let sql = format!(
                r#"INSERT INTO {}(
                       {},operation_id,entity_id,entity_version,source_generation_id,
                       source_revision_id,source_contract_hash,projection_status,
                       projection_body,projection_hash,batch_id,change_seq,
                       invalidation_reason,projected_at
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
                compatibility.kind.table(),
                compatibility.kind.id_column(),
            );
            sqlx::query(&sql)
                .bind(compatibility.projection_id)
                .bind(operation_id)
                .bind(
                    Uuid::parse_str(&projection.entity_id)
                        .map_err(|_| contract(SOURCE_SNAPSHOT_INVALID))?,
                )
                .bind(projection.entity_version)
                .bind(compatibility.source_generation_id)
                .bind(compatibility.source_revision_id)
                .bind(&compatibility.source_contract_hash)
                .bind(compatibility.projection_status)
                .bind(&compatibility.projection_body)
                .bind(&projection_hash)
                .bind(batch_id)
                .bind(change_seq)
                .bind(projection.invalidation_reason.map(|value| value.as_str()))
                .bind(projected_at)
                .execute(&mut *tx)
                .await?;
        }

        let mut persisted_change = PersistedProjectionChangeV1 {
            operation_id,
            event_id: Uuid::nil(),
            change_seq,
            batch_id,
            source_batch_seq: batch.source_batch_seq,
            outbox_member_id: projection.source.outbox_member_id,
            entity_kind: projection.entity_kind,
            entity_id: projection.entity_id.clone(),
            entity_version: u64::try_from(projection.entity_version)
                .map_err(|_| contract(CATALOG_INVALID))?,
            change_kind: projection.change_kind,
            event_kind: projection.timeline_event_kind,
            organization_id: None,
            source_occurred_at: projection.source.source_occurred_at,
            source_time_status: parse_source_time_status(&projection.source.source_time_status)?,
            projected_at,
            invalidation_reason: projection.invalidation_reason,
            source_hash: projection.source_hash.clone(),
            projection_hash: projection_hash.clone(),
            change_hash: String::new(),
        };
        persisted_change.change_hash =
            projection_change_hash_v1(&persisted_change).map_err(|_| contract(CATALOG_INVALID))?;
        persisted_change.event_id =
            projection_event_id_v1(&persisted_change).map_err(|_| contract(CATALOG_INVALID))?;
        let event_id = persisted_change.event_id;
        let change_hash = persisted_change.change_hash;
        sqlx::query(
            r#"INSERT INTO investigation_projection_changes(
                   operation_id,change_seq,event_id,batch_id,source_batch_seq,outbox_member_id,
                   entity_kind,entity_id,entity_version,change_kind,timeline_event_kind,
                   invalidation_reason,source_hash,projection_hash,change_hash,
                   source_occurred_at,source_time_status,projected_at
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#,
        )
        .bind(operation_id)
        .bind(change_seq)
        .bind(event_id)
        .bind(batch_id)
        .bind(batch.source_batch_seq)
        .bind(projection.source.outbox_member_id)
        .bind(projection.entity_kind.as_str())
        .bind(&projection.entity_id)
        .bind(projection.entity_version)
        .bind(projection.change_kind.as_str())
        .bind(projection.timeline_event_kind.as_str())
        .bind(projection.invalidation_reason.map(|value| value.as_str()))
        .bind(&projection.source_hash)
        .bind(&projection_hash)
        .bind(&change_hash)
        .bind(projection.source.source_occurred_at)
        .bind(&projection.source.source_time_status)
        .bind(projected_at)
        .execute(&mut *tx)
        .await?;
        entity_manifest.push(format!(
            "{}:{}:{}:{}",
            projection.entity_kind.as_str(),
            projection.entity_id,
            projection.entity_version,
            projection_hash
        ));
        change_manifest.push(change_hash);
        timeline_manifest.push(format!(
            "{event_id}:{}",
            projection.timeline_event_kind.as_str()
        ));
    }

    let last_change_seq = first_change_seq
        + i64::try_from(projections.len()).map_err(|_| contract(BATCH_EXACT_SET_INVALID))?
        - 1;
    let entity_version_manifest_hash = sha256_json(&entity_manifest)?;
    let change_manifest_hash = sha256_json(&change_manifest)?;
    let timeline_manifest_hash = sha256_json(&timeline_manifest)?;
    let receipt_identity = format!("investigation-projection-receipt:v1:{operation_id}:{batch_id}");
    let receipt_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, receipt_identity.as_bytes());
    sqlx::query(
        r#"INSERT INTO investigation_projection_batch_receipts(
               receipt_id,batch_id,operation_id,source_batch_seq,predecessor_batch_id,
               first_change_seq,last_change_seq,entity_version_manifest_hash,
               change_manifest_hash,timeline_manifest_hash,projected_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
    )
    .bind(receipt_id)
    .bind(batch_id)
    .bind(operation_id)
    .bind(batch.source_batch_seq)
    .bind(batch.predecessor_batch_id)
    .bind(first_change_seq)
    .bind(last_change_seq)
    .bind(&entity_version_manifest_hash)
    .bind(&change_manifest_hash)
    .bind(&timeline_manifest_hash)
    .bind(projected_at)
    .execute(&mut *tx)
    .await?;
    let advance = sqlx::query(
        r#"UPDATE investigation_projection_heads
              SET change_seq=$2,last_projected_batch_id=$3
            WHERE operation_id=$1 AND change_seq=$4
              AND last_projected_batch_id IS NOT DISTINCT FROM $5"#,
    )
    .bind(operation_id)
    .bind(last_change_seq)
    .bind(batch_id)
    .bind(head.change_seq)
    .bind(head.last_projected_batch_id)
    .execute(&mut *tx)
    .await?;
    if advance.rows_affected() != 1 {
        return Err(contract(HEAD_CAS_INVALID));
    }
    let receipt = ProjectionBatchReceipt {
        receipt_id,
        batch_id,
        operation_id,
        source_batch_seq: batch.source_batch_seq,
        predecessor_batch_id: batch.predecessor_batch_id,
        first_change_seq,
        last_change_seq,
        entity_version_manifest_hash,
        change_manifest_hash,
        timeline_manifest_hash,
        projected_at,
    };
    tx.commit().await?;
    Ok(ProjectionProjectOutcome::Applied(receipt))
}

pub async fn capture_projection_head(
    pool: &PgPool,
    operation_id: Uuid,
) -> InvestigationProjectionResult<CapturedProjectionHead> {
    Ok(sqlx::query_as::<_, CapturedProjectionHead>(
        r#"SELECT operation_id,projection_schema_version,change_seq,
                  last_projected_batch_id,cursor_salt
             FROM investigation_projection_heads WHERE operation_id=$1"#,
    )
    .bind(operation_id)
    .fetch_one(pool)
    .await?)
}

pub async fn read_projection_at_head(
    pool: &PgPool,
    head: &CapturedProjectionHead,
) -> InvestigationProjectionResult<ProjectionReadPage> {
    let current = capture_projection_head(pool, head.operation_id).await?;
    if current.projection_schema_version != head.projection_schema_version
        || current.cursor_salt != head.cursor_salt
        || current.change_seq < head.change_seq
    {
        return Err(contract(HEAD_CAS_INVALID));
    }
    let entity_rows = sqlx::query_as::<_, EntityReadRow>(
        r#"SELECT entity_kind,entity_id,entity_version,projection_hash,
                  projection_body,change_seq,invalidation_reason
             FROM investigation_projection_entity_versions
            WHERE operation_id=$1 AND change_seq<=$2
            ORDER BY change_seq"#,
    )
    .bind(head.operation_id)
    .bind(head.change_seq)
    .fetch_all(pool)
    .await?;
    let mut entities = Vec::with_capacity(entity_rows.len());
    for row in entity_rows {
        entities.push(MaterializedProjectionEntity {
            entity_kind: parse_entity_kind(&row.entity_kind)?,
            entity_id: row.entity_id,
            entity_version: row.entity_version,
            projection_hash: row.projection_hash,
            entity: serde_json::from_value(row.projection_body)?,
            change_seq: row.change_seq,
            invalidation_reason: parse_invalidation_reason(row.invalidation_reason.as_deref())?,
        });
    }
    let change_rows = sqlx::query_as::<_, ChangeReadRow>(
        r#"SELECT change_seq,event_id,batch_id,source_batch_seq,outbox_member_id,
                  entity_kind,entity_id,entity_version,change_kind,timeline_event_kind,
                  invalidation_reason,change_hash,source_occurred_at,source_time_status,projected_at
             FROM investigation_projection_changes
            WHERE operation_id=$1 AND change_seq<=$2 ORDER BY change_seq"#,
    )
    .bind(head.operation_id)
    .bind(head.change_seq)
    .fetch_all(pool)
    .await?;
    let mut changes = Vec::with_capacity(change_rows.len());
    for row in change_rows {
        changes.push(InvestigationProjectionChange {
            change_seq: row.change_seq,
            event_id: row.event_id,
            batch_id: row.batch_id,
            source_batch_seq: row.source_batch_seq,
            outbox_member_id: row.outbox_member_id,
            entity_kind: parse_entity_kind(&row.entity_kind)?,
            entity_id: row.entity_id,
            entity_version: row.entity_version,
            change_kind: parse_change_kind(&row.change_kind)?,
            timeline_event_kind: parse_timeline_kind(&row.timeline_event_kind)?,
            invalidation_reason: parse_invalidation_reason(row.invalidation_reason.as_deref())?,
            change_hash: row.change_hash,
            source_occurred_at: row.source_occurred_at,
            source_time_status: parse_source_time_status(&row.source_time_status)?,
            projected_at: row.projected_at,
        });
    }
    Ok(ProjectionReadPage {
        head: head.clone(),
        entities,
        changes,
    })
}

#[cfg(test)]
mod compatibility_tests {
    use super::{candidate_is_legacy_ready, CompatibilityAuthorityRow};
    use uuid::Uuid;

    fn complete_authority() -> CompatibilityAuthorityRow {
        CompatibilityAuthorityRow {
            generation_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
            revision_hash: format!("sha256:{}", "a".repeat(64)),
            revision_ingredients_hash: format!("sha256:{}", "b".repeat(64)),
            subject_kind: "attack_candidate".to_owned(),
            predicate_schema: "legacy_attack_candidate.v1".to_owned(),
            plan_id: Some(Uuid::new_v4()),
            plan_hash: Some(format!("sha256:{}", "c".repeat(64))),
            required_claim_component_count: Some(2),
            actual_claim_component_count: 2,
            objective_count: Some(1),
            actual_objective_count: 1,
            actual_contract_count: 1,
            actual_plan_objective_count: 1,
            proof_path_count: Some(1),
            actual_proof_path_count: 1,
            legacy_work_item_source_count: 1,
            forbidden_source_count: 0,
        }
    }

    #[test]
    fn legacy_candidate_ready_requires_classifier_work_item_and_full_b_authority() {
        let authority = complete_authority();
        assert!(candidate_is_legacy_ready(&authority));

        let mut missing_work_item = complete_authority();
        missing_work_item.legacy_work_item_source_count = 0;
        assert!(!candidate_is_legacy_ready(&missing_work_item));

        let mut incomplete_plan = complete_authority();
        incomplete_plan.actual_contract_count = 0;
        assert!(!candidate_is_legacy_ready(&incomplete_plan));

        let mut incompatible = complete_authority();
        incompatible.predicate_schema = "dns_observation.v1".to_owned();
        assert!(!candidate_is_legacy_ready(&incompatible));

        let mut non_legacy_source = complete_authority();
        non_legacy_source.forbidden_source_count = 1;
        assert!(!candidate_is_legacy_ready(&non_legacy_source));
    }
}
