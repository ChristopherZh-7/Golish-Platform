//! Plan B Candidate snapshot and analysis persistence boundary.
//!
//! The public freeze entry accepts only server operation/scope identity and
//! enters Plan A's request-scoped opaque authority callback.  The private
//! writer has no pool overload, so the checked bundle and Candidate snapshot
//! necessarily share one repeatable-read transaction.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use golish_core::hypothesis_semantic_key::{ClaimPolarity, PredicateIdentity};
use golish_pentest_domain::tool_truth::{
    EvidenceTemporalValidityPolicyV1, TemporalValidityStatus, ToolTruthRootFamilyV1,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::capability_execution_receipts::{
    with_checked_tool_truth_authority_bundle, CheckToolTruthAuthorityBundle,
    CheckedToolTruthAuthorityBundle, ToolTruthAuthorityBundleConsumerV1,
};
use crate::{DbError, Result};

const AUTHORITY_MISMATCH: &str = "HYPOTHESIS_REGISTRY_AUTHORITY_MISMATCH";
const SNAPSHOT_REPLAY_DRIFT: &str = "CANDIDATE_SNAPSHOT_REPLAY_DRIFT";
const SNAPSHOT_NOT_READY: &str = "CANDIDATE_ANALYSIS_SNAPSHOT_NOT_READY";
const WRITE_FENCE_MISMATCH: &str = "CANDIDATE_REPOSITORY_WRITE_FENCE_MISMATCH";
const ARTIFACT_KIND_FORBIDDEN: &str = "HYPOTHESIS_REGISTRY_ARTIFACT_KIND_FORBIDDEN";
const PAGE_SIZE_INVALID: &str = "CANDIDATE_ANALYSIS_PAGE_SIZE_INVALID";
const CENSUS_NOT_CLOSED: &str = "CANDIDATE_ANALYSIS_CENSUS_NOT_CLOSED";

fn conflict(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

const fn temporal_status_text(status: TemporalValidityStatus) -> &'static str {
    match status {
        TemporalValidityStatus::Fresh => "fresh",
        TemporalValidityStatus::Expired => "expired",
        TemporalValidityStatus::MixedEpoch => "mixed_epoch",
        TemporalValidityStatus::SkewExceeded => "skew_exceeded",
    }
}

fn parse_temporal_status(value: &str) -> Result<TemporalValidityStatus> {
    match value {
        "fresh" => Ok(TemporalValidityStatus::Fresh),
        "expired" => Ok(TemporalValidityStatus::Expired),
        "mixed_epoch" => Ok(TemporalValidityStatus::MixedEpoch),
        "skew_exceeded" => Ok(TemporalValidityStatus::SkewExceeded),
        _ => Err(conflict(AUTHORITY_MISMATCH)),
    }
}

async fn hash_json_on(tx: &mut Transaction<'_, Postgres>, value: &Value) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
            .bind(value)
            .fetch_one(&mut **tx)
            .await?,
    )
}

pub(crate) async fn hash_text_array_on(
    tx: &mut Transaction<'_, Postgres>,
    values: &[String],
) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT tool_truth_sha256(to_jsonb($1::TEXT[])::TEXT)")
            .bind(values)
            .fetch_one(&mut **tx)
            .await?,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreezeCandidateSnapshotInput {
    pub stable_consumer_request_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateWriteFenceRow {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub team_plan_id: Uuid,
    pub work_item_id: Uuid,
    pub worker_run_id: Uuid,
    pub lease_token: Uuid,
    pub lease_epoch: i64,
    pub analysis_attempt_id: Uuid,
    pub analysis_attempt_ordinal: i32,
    pub attempt_epoch: i64,
    pub expected_snapshot_row_version: i64,
    pub expected_team_plan_row_version: i64,
    pub expected_work_item_row_version: i64,
    pub expected_worker_row_version: i64,
    pub expected_attempt_row_version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateSnapshotDispositionRow {
    SealedReady,
    BlockedAuthorityBundle,
}

impl CandidateSnapshotDispositionRow {
    const fn as_str(self) -> &'static str {
        match self {
            Self::SealedReady => "sealed_ready",
            Self::BlockedAuthorityBundle => "blocked_authority_bundle",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "sealed_ready" => Ok(Self::SealedReady),
            "blocked_authority_bundle" => Ok(Self::BlockedAuthorityBundle),
            _ => Err(conflict(AUTHORITY_MISMATCH)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateAuthorityRootRowView {
    pub ordinal: i32,
    pub root_family: ToolTruthRootFamilyV1,
    pub root_denominator_id: Uuid,
    pub root_denominator_hash: String,
    pub authority_set_seal_id: Uuid,
    pub authority_set_graph_hash: String,
    pub authority_set_semantic_hash: String,
    pub authority_set_freshness_hash: String,
    pub temporal_validity_policy_set_hash: String,
    pub temporal_validity_decision_set_hash: String,
    pub target_state_epoch_set_hash: String,
    pub receipt_count: i64,
    pub receipt_set_hash: String,
    pub semantic_status: String,
    pub temporal_status: TemporalValidityStatus,
    pub temporal_policies: Vec<EvidenceTemporalValidityPolicyV1>,
    pub member_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateSnapshotRowView {
    pub snapshot_id: Uuid,
    pub stable_consumer_request_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub disposition: CandidateSnapshotDispositionRow,
    pub snapshot_hash: String,
    pub candidate_snapshot_authority_hash: String,
    pub tool_truth_authority_bundle_seal_id: Uuid,
    pub tool_truth_authority_root_count: i64,
    pub tool_truth_authority_root_set_hash: String,
    pub tool_truth_authority_bundle_member_count: i64,
    pub tool_truth_authority_bundle_member_set_hash: String,
    pub tool_truth_authority_receipt_count: i64,
    pub tool_truth_authority_receipt_set_hash: String,
    pub denominator_graph_bundle_hash: String,
    pub semantic_authority_bundle_hash: String,
    pub freshness_attestation_bundle_hash: String,
    pub temporal_validity_bundle_hash: String,
    pub temporal_validity_policy_set_hash: String,
    pub temporal_validity_decision_set_hash: String,
    pub observation_window_hash: String,
    pub target_state_epoch_set_hash: String,
    pub authority_roots: Vec<CandidateAuthorityRootRowView>,
    pub knowledge_feed_catalog_policy_seal_hash: String,
    pub knowledge_feed_required_member_set_hash: String,
    pub knowledge_feed_signature_algorithm_set_hash: String,
    pub knowledge_feed_trust_store_hash: String,
    pub knowledge_feed_key_revocation_epoch_hash: String,
    pub knowledge_feed_snapshot_set_hash: String,
    pub product_version_census_hash: String,
    pub knowledge_feed_match_census_hash: String,
    pub stale_revalidation_obligation_set_hash: String,
    pub knowledge_feed_obligation_set_hash: String,
    pub row_version: i64,
    pub sealed_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct BundleHeaderRow {
    id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    stable_consumer_request_id: Uuid,
    relevant_root_count: i64,
    relevant_root_set_hash: String,
    member_count: i64,
    member_set_hash: String,
    semantic_authority_bundle_hash: String,
    freshness_attestation_bundle_hash: String,
    temporal_validity_bundle_hash: String,
    temporal_validity_policy_set_hash: String,
    target_state_epoch_set_hash: String,
    observation_window_started_at: Option<DateTime<Utc>>,
    observation_window_completed_at: Option<DateTime<Utc>>,
    sealed_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct BundleMemberRow {
    id: Uuid,
    bundle_seal_id: Uuid,
    operation_id: Uuid,
    organization_id: Uuid,
    ordinal: i32,
    root_family: String,
    root_execution_authority_id: Uuid,
    root_denominator_id: Uuid,
    root_denominator_hash: String,
    authority_set_seal_id: Uuid,
    authority_set_semantic_hash: String,
    authority_set_graph_hash: String,
    authority_set_freshness_hash: String,
    temporal_validity_policy_set_hash: String,
    target_state_epoch_set_hash: String,
    semantic_status: String,
    temporal_validity_status: String,
    member_status: String,
    member_hash: String,
}

async fn load_bundle_rows_on(
    tx: &mut Transaction<'_, Postgres>,
    checked: &CheckedToolTruthAuthorityBundle<'_>,
) -> Result<(BundleHeaderRow, Vec<BundleMemberRow>)> {
    let header = sqlx::query_as::<_, BundleHeaderRow>(
        r#"SELECT id,operation_id,scope_snapshot_id,organization_id,
                  stable_consumer_request_id,relevant_root_count,relevant_root_set_hash,
                  member_count,member_set_hash,semantic_authority_bundle_hash,
                  freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
                  temporal_validity_policy_set_hash,target_state_epoch_set_hash,
                  observation_window_started_at,observation_window_completed_at,sealed_at
             FROM tool_truth_authority_bundle_seals WHERE id=$1 FOR SHARE"#,
    )
    .bind(checked.bundle_seal_id())
    .fetch_one(&mut **tx)
    .await?;
    let members = sqlx::query_as::<_, BundleMemberRow>(
        r#"SELECT id,bundle_seal_id,operation_id,organization_id,ordinal,root_family,
                  root_execution_authority_id,root_denominator_id,root_denominator_hash,
                  authority_set_seal_id,authority_set_semantic_hash,
                  authority_set_graph_hash,authority_set_freshness_hash,
                  temporal_validity_policy_set_hash,target_state_epoch_set_hash,
                  semantic_status,temporal_validity_status,member_status,member_hash
             FROM tool_truth_authority_bundle_members
            WHERE bundle_seal_id=$1 ORDER BY ordinal FOR SHARE"#,
    )
    .bind(checked.bundle_seal_id())
    .fetch_all(&mut **tx)
    .await?;
    if header.operation_id != checked.operation_id()
        || header.organization_id != checked.organization_id()
        || header.relevant_root_count != 4
        || header.member_count != 4
        || members.len() != 4
        || header.relevant_root_set_hash != checked.relevant_root_set_hash()
        || header.member_set_hash != checked.member_set_hash()
        || header.semantic_authority_bundle_hash != checked.semantic_authority_bundle_hash()
        || header.freshness_attestation_bundle_hash != checked.freshness_attestation_bundle_hash()
        || header.temporal_validity_bundle_hash != checked.temporal_validity_bundle_hash()
        || header.temporal_validity_policy_set_hash != checked.temporal_validity_policy_set_hash()
        || header.target_state_epoch_set_hash != checked.target_state_epoch_set_hash()
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    for (ordinal, (persisted, borrowed)) in members.iter().zip(checked.roots()).enumerate() {
        let root_family = ToolTruthRootFamilyV1::try_from(persisted.root_family.as_str())
            .map_err(|_| conflict(AUTHORITY_MISMATCH))?;
        if persisted.ordinal != ordinal as i32
            || root_family != borrowed.root_family
            || persisted.root_denominator_id != borrowed.root_denominator_id
            || persisted.root_denominator_hash != borrowed.root_denominator_hash
            || persisted.authority_set_seal_id != borrowed.authority_set_seal_id
            || persisted.authority_set_graph_hash != borrowed.authority_set_graph_hash
            || persisted.authority_set_semantic_hash != borrowed.authority_set_semantic_hash
            || persisted.authority_set_freshness_hash != borrowed.authority_set_freshness_hash
            || persisted.temporal_validity_policy_set_hash
                != borrowed.temporal_validity_policy_set_hash
            || persisted.target_state_epoch_set_hash != borrowed.target_state_epoch_set_hash
            || persisted.semantic_status != borrowed.semantic_status
            || persisted.temporal_validity_status
                != temporal_status_text(borrowed.temporal_validity_status)
            || persisted.member_status != borrowed.member_status.as_str()
        {
            return Err(conflict(AUTHORITY_MISMATCH));
        }
    }
    Ok((header, members))
}

async fn load_snapshot_source_members_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    organization_id: Uuid,
    scope_snapshot_id: Uuid,
    previous_generation_seal_id: Option<Uuid>,
) -> Result<BTreeMap<&'static str, Vec<(String, String)>>> {
    let mut sources = BTreeMap::new();
    let previous_generation_id: Option<Uuid> = if let Some(seal_id) = previous_generation_seal_id {
        Some(
            sqlx::query_scalar(
                r#"SELECT generation.generation_id
                     FROM hypothesis_generation_seals seal
                     JOIN hypothesis_generations generation USING(generation_id)
                    WHERE seal.seal_id=$1 AND generation.operation_id=$2
                      AND generation.organization_id=$3 FOR SHARE"#,
            )
            .bind(seal_id)
            .bind(operation_id)
            .bind(organization_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?,
        )
    } else {
        None
    };
    let state_events: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT event.event_id::TEXT,event.event_hash
             FROM hypothesis_generation_members member
             JOIN attack_hypothesis_state_events event
               ON event.successor_revision_id=member.revision_id
            WHERE member.generation_id=$1 ORDER BY event.event_id"#,
    )
    .bind(previous_generation_id)
    .fetch_all(&mut **tx)
    .await?;
    sources.insert("state_events", state_events);
    let relations: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT relation.relation_id::TEXT,relation.relation_hash
             FROM attack_hypothesis_relations relation
            WHERE relation.operation_id=$1 AND relation.organization_id=$2
              AND EXISTS(
                  SELECT 1 FROM hypothesis_generation_members member
                   WHERE member.generation_id=$3
                     AND member.revision_id IN (
                         relation.source_revision_id,relation.target_revision_id))
            ORDER BY relation.relation_id"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(previous_generation_id)
    .fetch_all(&mut **tx)
    .await?;
    sources.insert("relations", relations);
    let obligations: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT residual_id::TEXT,residual_hash
             FROM hypothesis_residual_risks
            WHERE operation_id=$1 AND organization_id=$2 AND closed_at IS NULL
            ORDER BY residual_id"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .fetch_all(&mut **tx)
    .await?;
    sources.insert("open_obligations", obligations);
    for (source_kind, predicate) in [
        ("expected_fact_deltas", "TRUE"),
        (
            "unconsumed_fact_deltas",
            "status IN ('proposed','accepted')",
        ),
        ("consumed_fact_deltas", "status='consumed'"),
    ] {
        let query = format!(
            "SELECT id::TEXT,dedupe_hash FROM attack_fact_deltas \
             WHERE operation_id=$1 AND organization_id=$2 AND scope_snapshot_id=$3 \
               AND {predicate} ORDER BY id"
        );
        let members = sqlx::query_as::<_, (String, String)>(&query)
            .bind(operation_id)
            .bind(organization_id)
            .bind(scope_snapshot_id)
            .fetch_all(&mut **tx)
            .await?;
        sources.insert(source_kind, members);
    }
    Ok(sources)
}

pub async fn freeze_candidate_snapshot(
    pool: &PgPool,
    input: FreezeCandidateSnapshotInput,
) -> Result<CandidateSnapshotRowView> {
    let authority_request = CheckToolTruthAuthorityBundle {
        stable_consumer_request_id: input.stable_consumer_request_id,
        operation_id: input.operation_id,
        organization_id: input.organization_id,
        consumer_kind: ToolTruthAuthorityBundleConsumerV1::CandidateAnalysis,
    };
    with_checked_tool_truth_authority_bundle(pool, &authority_request, move |tx, checked| {
        Box::pin(async move { freeze_snapshot_on(tx, checked, input).await })
    })
    .await
}

pub(crate) async fn freeze_snapshot_on(
    tx: &mut Transaction<'_, Postgres>,
    checked: &CheckedToolTruthAuthorityBundle<'_>,
    input: FreezeCandidateSnapshotInput,
) -> Result<CandidateSnapshotRowView> {
    if checked.operation_id() != input.operation_id
        || checked.organization_id() != input.organization_id
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let (header, members) = load_bundle_rows_on(tx, checked).await?;
    if header.scope_snapshot_id != input.scope_snapshot_id
        || header.stable_consumer_request_id != input.stable_consumer_request_id
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    sqlx::query(
        r#"SELECT operation.operation_id
             FROM operation_state operation
             JOIN operation_org_scope_snapshots scope
               ON scope.operation_id=operation.operation_id
              AND scope.project_scope_id=operation.project_scope_id
             JOIN operation_org_scope_units unit
               ON unit.snapshot_id=scope.id
            WHERE operation.operation_id=$1 AND scope.id=$2
              AND unit.organization_id=$3 AND scope.sealed_at IS NOT NULL
            FOR SHARE OF operation,scope,unit"#,
    )
    .bind(input.operation_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;

    let snapshot_id = Uuid::new_v5(
        &input.stable_consumer_request_id,
        b"candidate_analysis_snapshot.v1",
    );
    if sqlx::query_scalar::<_, Uuid>(
        "SELECT snapshot_id FROM candidate_analysis_snapshots WHERE snapshot_id=$1",
    )
    .bind(snapshot_id)
    .fetch_optional(&mut **tx)
    .await?
    .is_some()
    {
        let replay = load_snapshot_on(tx, snapshot_id).await?;
        if replay.operation_id != input.operation_id
            || replay.organization_id != input.organization_id
            || replay.scope_snapshot_id != input.scope_snapshot_id
            || replay.tool_truth_authority_bundle_seal_id != checked.bundle_seal_id()
        {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
        return Ok(replay);
    }

    let wave_ordinal: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(wave_ordinal)+1,0) FROM candidate_analysis_snapshots WHERE operation_id=$1 AND organization_id=$2",
    )
    .bind(input.operation_id)
    .bind(input.organization_id)
    .fetch_one(&mut **tx)
    .await?;
    let previous_generation_seal_id: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT seal.seal_id
             FROM hypothesis_generation_seals seal
             JOIN hypothesis_generations generation ON generation.generation_id=seal.generation_id
            WHERE generation.operation_id=$1 AND generation.organization_id=$2
            ORDER BY generation.generation_ordinal DESC LIMIT 1 FOR SHARE"#,
    )
    .bind(input.operation_id)
    .bind(input.organization_id)
    .fetch_optional(&mut **tx)
    .await?;
    let genesis = previous_generation_seal_id.is_none();
    let frozen_source_members = load_snapshot_source_members_on(
        tx,
        input.operation_id,
        input.organization_id,
        input.scope_snapshot_id,
        previous_generation_seal_id,
    )
    .await?;
    let previous_generation_members = if let Some(seal_id) = previous_generation_seal_id {
        let generation_hash: String = sqlx::query_scalar(
            "SELECT generation_hash FROM hypothesis_generation_seals WHERE seal_id=$1 FOR SHARE",
        )
        .bind(seal_id)
        .fetch_one(&mut **tx)
        .await?;
        vec![(seal_id.to_string(), generation_hash)]
    } else {
        vec![(
            "previous_generation_absent".to_owned(),
            hash_json_on(
                tx,
                &json!({
                    "domain":"candidate_previous_generation_absent.v1",
                    "operation_id":input.operation_id,
                    "organization_id":input.organization_id,
                }),
            )
            .await?,
        )]
    };

    let operation_authority: Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
                 'operation_id',operation_id,'project_scope_id',project_scope_id,
                 'tool_truth_contract',tool_truth_contract,
                 'investigation_contract_version',investigation_contract_version,
                 'investigation_rollout_mode',investigation_rollout_mode,
                 'current_stage',current_stage)
             FROM operation_state WHERE operation_id=$1"#,
    )
    .bind(input.operation_id)
    .fetch_one(&mut **tx)
    .await?;
    let capability_revision_hash = hash_json_on(
        tx,
        &json!({"domain":"candidate_capability_revision.v1","authority":operation_authority}),
    )
    .await?;
    let policy_revision_hash = hash_json_on(
        tx,
        &json!({"domain":"candidate_policy_revision.v1","authority":operation_authority}),
    )
    .await?;
    let credential_revision_hash = hash_json_on(
        tx,
        &json!({"domain":"candidate_credential_revision.v1","authority":operation_authority}),
    )
    .await?;
    let source_set_hash = hash_json_on(
        tx,
        &json!({
            "domain":"candidate_snapshot_source_set.v1",
            "tool_truth_bundle":checked.bundle_seal_id(),
            "previous_generation_seal":previous_generation_seal_id,
            "genesis":genesis,
            "previous_generation_members":previous_generation_members,
            "canonical_source_members":frozen_source_members,
        }),
    )
    .await?;
    let observation_window_hash = hash_json_on(
        tx,
        &json!({
            "domain":"candidate_observation_window.v1",
            "started_at":header.observation_window_started_at,
            "completed_at":header.observation_window_completed_at,
        }),
    )
    .await?;

    // 00006 contains the frozen feed authority tables but no live managed-feed
    // catalog/store.  Missing required feed members therefore fail closed and
    // are represented as unavailable, never silently inferred from extant rows.
    let feed_available = false;
    let disposition = if checked.is_all_fresh() && feed_available {
        CandidateSnapshotDispositionRow::SealedReady
    } else {
        CandidateSnapshotDispositionRow::BlockedAuthorityBundle
    };
    let candidate_snapshot_authority_hash = hash_json_on(
        tx,
        &json!({
            "domain":"candidate_snapshot_authority.v1",
            "snapshot_id":snapshot_id,
            "bundle_seal_id":checked.bundle_seal_id(),
            "root_set_hash":checked.relevant_root_set_hash(),
            "member_set_hash":checked.member_set_hash(),
            "semantic_bundle_hash":checked.semantic_authority_bundle_hash(),
            "freshness_bundle_hash":checked.freshness_attestation_bundle_hash(),
            "temporal_bundle_hash":checked.temporal_validity_bundle_hash(),
            "temporal_policy_set_hash":checked.temporal_validity_policy_set_hash(),
            "target_state_epoch_set_hash":checked.target_state_epoch_set_hash(),
            "observation_window_hash":observation_window_hash,
            "source_set_hash":source_set_hash,
            "feed_authority":"required_unavailable",
        }),
    )
    .await?;

    sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshots(
               snapshot_id,operation_id,organization_id,wave_ordinal,scope_snapshot_id,
               genesis,previous_generation_seal_id,source_set_hash,
               capability_revision_hash,policy_revision_hash,credential_revision_hash,
               snapshot_status,tool_truth_authority_bundle_seal_id,
               stable_consumer_request_id,relevant_root_count,relevant_root_set_hash,
               bundle_member_count,bundle_member_set_hash,semantic_authority_bundle_hash,
               freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
               temporal_validity_policy_set_hash,target_state_epoch_set_hash,
               observation_window_hash,bundle_sealed_at,candidate_snapshot_authority_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,
               $19,$20,$21,$22,$23,$24,$25,$26)"#,
    )
    .bind(snapshot_id)
    .bind(input.operation_id)
    .bind(input.organization_id)
    .bind(wave_ordinal)
    .bind(input.scope_snapshot_id)
    .bind(genesis)
    .bind(previous_generation_seal_id)
    .bind(&source_set_hash)
    .bind(&capability_revision_hash)
    .bind(&policy_revision_hash)
    .bind(&credential_revision_hash)
    .bind(disposition.as_str())
    .bind(header.id)
    .bind(header.stable_consumer_request_id)
    .bind(header.relevant_root_count)
    .bind(&header.relevant_root_set_hash)
    .bind(header.member_count)
    .bind(&header.member_set_hash)
    .bind(&header.semantic_authority_bundle_hash)
    .bind(&header.freshness_attestation_bundle_hash)
    .bind(&header.temporal_validity_bundle_hash)
    .bind(&header.temporal_validity_policy_set_hash)
    .bind(&header.target_state_epoch_set_hash)
    .bind(&observation_window_hash)
    .bind(header.sealed_at)
    .bind(&candidate_snapshot_authority_hash)
    .execute(&mut **tx)
    .await?;

    for member in &members {
        sqlx::query(
            r#"INSERT INTO candidate_analysis_snapshot_authority_bundle_members(
                   snapshot_member_id,snapshot_id,operation_id,organization_id,bundle_seal_id,
                   tool_truth_authority_bundle_member_id,ordinal,root_family,
                   root_execution_authority_id,root_denominator_id,root_denominator_hash,
                   authority_set_seal_id,authority_set_semantic_hash,authority_set_graph_hash,
                   authority_set_freshness_hash,temporal_validity_policy_set_hash,
                   target_state_epoch_set_hash,semantic_status,temporal_validity_status,
                   member_status,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)"#,
        )
        .bind(Uuid::new_v5(&snapshot_id, member.member_hash.as_bytes()))
        .bind(snapshot_id)
        .bind(member.operation_id)
        .bind(member.organization_id)
        .bind(member.bundle_seal_id)
        .bind(member.id)
        .bind(member.ordinal)
        .bind(&member.root_family)
        .bind(member.root_execution_authority_id)
        .bind(member.root_denominator_id)
        .bind(&member.root_denominator_hash)
        .bind(member.authority_set_seal_id)
        .bind(&member.authority_set_semantic_hash)
        .bind(&member.authority_set_graph_hash)
        .bind(&member.authority_set_freshness_hash)
        .bind(&member.temporal_validity_policy_set_hash)
        .bind(&member.target_state_epoch_set_hash)
        .bind(&member.semantic_status)
        .bind(&member.temporal_validity_status)
        .bind(&member.member_status)
        .bind(&member.member_hash)
        .execute(&mut **tx)
        .await?;
    }

    persist_temporal_census_on(tx, snapshot_id, header.id, &members, checked).await?;
    persist_source_set_on(
        tx,
        snapshot_id,
        "tool_truth_bundle",
        vec![(header.id.to_string(), header.member_set_hash.clone())],
    )
    .await?;
    persist_source_set_on(
        tx,
        snapshot_id,
        "previous_generation",
        previous_generation_members,
    )
    .await?;
    for (source_kind, members) in frozen_source_members {
        persist_source_set_on(tx, snapshot_id, source_kind, members).await?;
    }
    persist_unavailable_feed_authority_on(tx, snapshot_id).await?;
    if disposition == CandidateSnapshotDispositionRow::SealedReady {
        freeze_ready_snapshot_inputs_and_attempt_on(tx, snapshot_id, &members).await?;
    }
    load_snapshot_on(tx, snapshot_id).await
}

#[derive(Debug, sqlx::FromRow)]
struct FrozenReceiptSourceRow {
    root_family: String,
    receipt_id: Uuid,
    capability: String,
    receipt_authority_hash: String,
    semantic_hash: String,
    authority_member_hash: String,
    attempt_state: String,
    landing_state: String,
    observation_state: String,
    coverage_extent: String,
    coverage_gap_reason: String,
    security_interpretation: String,
    typed_landing_contract_version: String,
    typed_landing: Value,
    residual: Option<Value>,
}

async fn freeze_ready_snapshot_inputs_and_attempt_on(
    tx: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
    bundle_members: &[BundleMemberRow],
) -> Result<()> {
    const CHUNK_BYTES: usize = 8 * 1024;
    const MAX_SOURCE_BYTES: usize = 1024 * 1024;
    const CHUNKING_VERSION: &str = "1";
    const REDACTION_VERSION: &str = "1";

    let sources = sqlx::query_as::<_, FrozenReceiptSourceRow>(
        r#"SELECT bundle.root_family,set_member.receipt_id,receipt.capability,
                  receipt.receipt_authority_hash,set_member.semantic_hash,
                  set_member.member_hash AS authority_member_hash,
                  receipt.attempt_state,receipt.landing_state,receipt.observation_state,
                  receipt.coverage_extent,receipt.coverage_gap_reason,
                  receipt.security_interpretation,receipt.typed_landing_contract_version,
                  receipt.typed_landing,receipt.residual
             FROM tool_truth_authority_bundle_members bundle
             JOIN tool_truth_authority_set_members set_member
               ON set_member.authority_set_id=bundle.authority_set_seal_id
             JOIN capability_execution_receipts receipt ON receipt.id=set_member.receipt_id
            WHERE bundle.bundle_seal_id=(
                  SELECT tool_truth_authority_bundle_seal_id
                    FROM candidate_analysis_snapshots WHERE snapshot_id=$1)
            ORDER BY bundle.ordinal,set_member.ordinal"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    if sources.is_empty() || bundle_members.len() != ToolTruthRootFamilyV1::ALL.len() {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let subject_identity_hash = hash_json_on(
        tx,
        &json!({
            "organization_id": bundle_members[0].organization_id,
            "identity_contract": "candidate_subject_at_time.v1",
        }),
    )
    .await?;
    let mut input_hashes = Vec::with_capacity(sources.len());
    for source in sources {
        let stable_input_key = format!("{}:receipt:{}", source.root_family, source.receipt_id);
        let source_ref = format!("capability_execution_receipt:{}", source.receipt_id);
        let source_ref_hash = hash_json_on(
            tx,
            &json!({"source_ref":source_ref,"receipt_authority_hash":source.receipt_authority_hash}),
        )
        .await?;
        let frozen_body = json!({
            "schema": "candidate_tool_truth_receipt.v1",
            "instruction_authority": false,
            "root_family": source.root_family,
            "receipt_id": source.receipt_id,
            "capability": source.capability,
            "receipt_authority_hash": source.receipt_authority_hash,
            "semantic_hash": source.semantic_hash,
            "authority_member_hash": source.authority_member_hash,
            "attempt_state": source.attempt_state,
            "landing_state": source.landing_state,
            "observation_state": source.observation_state,
            "coverage_extent": source.coverage_extent,
            "coverage_gap_reason": source.coverage_gap_reason,
            "security_interpretation": source.security_interpretation,
            "typed_landing_contract_version": source.typed_landing_contract_version,
            "typed_landing": source.typed_landing,
            "residual": source.residual,
        });
        let source_bytes = serde_json::to_vec(&frozen_body)?;
        let source_byte_count = i64::try_from(source_bytes.len())
            .map_err(|_| conflict("CANDIDATE_SNAPSHOT_SOURCE_OVERSIZE"))?;
        let source_content_hash = hash_json_on(tx, &frozen_body).await?;
        let disposition = if source_bytes.len() > MAX_SOURCE_BYTES {
            "blocked_oversize"
        } else if source_bytes.is_empty() {
            "source_empty"
        } else {
            "complete"
        };
        let input_hash = hash_json_on(
            tx,
            &json!({
                "stable_input_key":stable_input_key,
                "source_ref_hash":source_ref_hash,
                "source_content_hash":source_content_hash,
                "source_byte_count":source_byte_count,
                "subject_identity_hash":subject_identity_hash,
                "chunking_disposition":disposition,
            }),
        )
        .await?;
        let snapshot_input_id = Uuid::new_v5(&snapshot_id, stable_input_key.as_bytes());
        sqlx::query(
            r#"INSERT INTO candidate_analysis_snapshot_inputs(
                   snapshot_input_id,snapshot_id,stable_input_key,source_kind,source_ref,
                   source_ref_hash,source_content_hash,source_byte_count,subject_kind_at_time,
                   subject_identity_hash,server_chunking_disposition,input_hash
               ) VALUES($1,$2,$3,'tool_truth_bundle',$4,$5,$6,$7,
                        'organization',$8,$9,$10)"#,
        )
        .bind(snapshot_input_id)
        .bind(snapshot_id)
        .bind(&stable_input_key)
        .bind(&source_ref)
        .bind(&source_ref_hash)
        .bind(&source_content_hash)
        .bind(source_byte_count)
        .bind(&subject_identity_hash)
        .bind(disposition)
        .bind(&input_hash)
        .execute(&mut **tx)
        .await?;

        let chunk_census_id = Uuid::new_v5(&snapshot_input_id, b"candidate_chunk_census.v1");
        let chunks = if disposition == "complete" {
            source_bytes
                .chunks(CHUNK_BYTES)
                .enumerate()
                .map(|(ordinal, bytes)| {
                    let start = ordinal * CHUNK_BYTES;
                    (ordinal, start, start + bytes.len(), bytes)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut chunk_hashes = Vec::with_capacity(chunks.len());
        let mut prepared_chunks = Vec::with_capacity(chunks.len());
        for (ordinal, start, end, bytes) in chunks {
            let mut encoded = String::with_capacity(bytes.len() * 2);
            for byte in bytes {
                use std::fmt::Write;
                write!(encoded, "{byte:02x}").expect("writing to String cannot fail");
            }
            let immutable_body = json!({
                "schema":"candidate_redacted_chunk.v1",
                "encoding":"hex",
                "canonical_source_fragment":encoded,
                "instruction_authority":false,
            });
            let body_hash = hash_json_on(tx, &immutable_body).await?;
            let chunk_hash = hash_json_on(
                tx,
                &json!({
                    "ordinal":ordinal,"range_start":start,"range_end":end,
                    "body_hash":body_hash,"chunking":CHUNKING_VERSION,
                    "redaction":REDACTION_VERSION,
                }),
            )
            .await?;
            chunk_hashes.push(chunk_hash.clone());
            prepared_chunks.push((ordinal, start, end, immutable_body, body_hash, chunk_hash));
        }
        let chunk_member_set_hash = hash_text_array_on(tx, &chunk_hashes).await?;
        let census_hash = hash_json_on(
            tx,
            &json!({
                "snapshot_input_id":snapshot_input_id,
                "source_content_hash":source_content_hash,
                "source_byte_count":source_byte_count,
                "disposition":disposition,
                "chunk_member_set_hash":chunk_member_set_hash,
                "chunking":CHUNKING_VERSION,"redaction":REDACTION_VERSION,
            }),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO candidate_analysis_input_chunk_censuses(
                   chunk_census_id,snapshot_input_id,snapshot_id,chunking_contract_version,
                   redaction_contract_version,source_content_hash,source_byte_count,
                   disposition,chunk_count,chunk_member_set_hash,census_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
        )
        .bind(chunk_census_id)
        .bind(snapshot_input_id)
        .bind(snapshot_id)
        .bind(CHUNKING_VERSION)
        .bind(REDACTION_VERSION)
        .bind(&source_content_hash)
        .bind(source_byte_count)
        .bind(disposition)
        .bind(i64::try_from(prepared_chunks.len()).unwrap_or(i64::MAX))
        .bind(&chunk_member_set_hash)
        .bind(&census_hash)
        .execute(&mut **tx)
        .await?;
        for (ordinal, start, end, body, body_hash, chunk_hash) in prepared_chunks {
            let chunk_id = Uuid::new_v5(&chunk_census_id, chunk_hash.as_bytes());
            sqlx::query(
                r#"INSERT INTO candidate_analysis_input_chunk_census_members(
                       chunk_id,chunk_census_id,snapshot_input_id,snapshot_id,ordinal,
                       source_range_start,source_range_end,envelope_schema,
                       immutable_redacted_body,body_or_blob_hash,chunking_contract_version,
                       redaction_contract_version,chunk_hash
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,'candidate_redacted_chunk.v1',
                            $8,$9,$10,$11,$12)"#,
            )
            .bind(chunk_id)
            .bind(chunk_census_id)
            .bind(snapshot_input_id)
            .bind(snapshot_id)
            .bind(i32::try_from(ordinal).unwrap_or(i32::MAX))
            .bind(i64::try_from(start).unwrap_or(i64::MAX))
            .bind(i64::try_from(end).unwrap_or(i64::MAX))
            .bind(body)
            .bind(body_hash)
            .bind(CHUNKING_VERSION)
            .bind(REDACTION_VERSION)
            .bind(chunk_hash)
            .execute(&mut **tx)
            .await?;
        }
        input_hashes.push(input_hash);
    }
    let input_set_hash = hash_text_array_on(tx, &input_hashes).await?;
    let (operation_id, organization_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT operation_id,organization_id FROM candidate_analysis_snapshots WHERE snapshot_id=$1",
    )
    .bind(snapshot_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
    let attack_class_digest = hash_json_on(tx, &json!({"contract":"attack_class.v1"})).await?;
    let trust_boundary_digest = hash_json_on(tx, &json!({"contract":"trust_boundary.v1"})).await?;
    let sampling_digest = hash_json_on(tx, &json!({"contract":"coverage_sampling.v1"})).await?;
    let attempt_input_hash = hash_json_on(
        tx,
        &json!({
            "snapshot_id":snapshot_id,"input_set_hash":input_set_hash,
            "attack_class_digest":attack_class_digest,
            "trust_boundary_digest":trust_boundary_digest,
            "sampling_digest":sampling_digest,
        }),
    )
    .await?;
    let analysis_attempt_id = Uuid::new_v5(&snapshot_id, b"candidate_analysis_attempt:0");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempts(
               analysis_attempt_id,snapshot_id,operation_id,organization_id,attempt_ordinal,
               attempt_input_hash,attack_class_checklist_version,attack_class_checklist_digest,
               trust_boundary_checklist_version,trust_boundary_checklist_digest,
               coverage_sampling_contract_version,coverage_sampling_contract_digest,retry_limit
           ) VALUES($1,$2,$3,$4,0,$5,'attack_class.v1',$6,
                    'trust_boundary.v1',$7,'coverage_sampling.v1',$8,2)"#,
    )
    .bind(analysis_attempt_id)
    .bind(snapshot_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(&attempt_input_hash)
    .bind(&attack_class_digest)
    .bind(&trust_boundary_digest)
    .bind(&sampling_digest)
    .execute(&mut **tx)
    .await?;
    let event_hash = hash_json_on(
        tx,
        &json!({"attempt":analysis_attempt_id,"ordinal":0,"event":"opened"}),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempt_state_events(
               attempt_event_id,analysis_attempt_id,event_ordinal,event_kind,event_hash
           ) VALUES($1,$2,0,'opened',$3)"#,
    )
    .bind(Uuid::new_v5(
        &analysis_attempt_id,
        b"candidate_attempt_opened.v1",
    ))
    .bind(analysis_attempt_id)
    .bind(event_hash)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn persist_source_set_on(
    tx: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
    source_kind: &str,
    members: Vec<(String, String)>,
) -> Result<()> {
    let source_set_id = Uuid::new_v5(&snapshot_id, source_kind.as_bytes());
    let mut member_hashes = Vec::with_capacity(members.len());
    for (identity, source_hash) in &members {
        member_hashes.push(
            hash_json_on(
                tx,
                &json!({"source_identity":identity,"source_hash":source_hash}),
            )
            .await?,
        );
    }
    let member_set_hash = hash_text_array_on(tx, &member_hashes).await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshot_source_sets(
               source_set_id,snapshot_id,source_kind,member_count,member_set_hash,sealed_empty
           ) VALUES($1,$2,$3,$4,$5,$6)"#,
    )
    .bind(source_set_id)
    .bind(snapshot_id)
    .bind(source_kind)
    .bind(i64::try_from(members.len()).unwrap_or(i64::MAX))
    .bind(&member_set_hash)
    .bind(members.is_empty())
    .execute(&mut **tx)
    .await?;
    for (ordinal, ((identity, source_hash), member_hash)) in
        members.into_iter().zip(member_hashes).enumerate()
    {
        sqlx::query(
            r#"INSERT INTO candidate_analysis_snapshot_source_set_members(
                   source_member_id,source_set_id,snapshot_id,ordinal,
                   source_identity,source_hash,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7)"#,
        )
        .bind(Uuid::new_v5(&source_set_id, member_hash.as_bytes()))
        .bind(source_set_id)
        .bind(snapshot_id)
        .bind(i32::try_from(ordinal).unwrap_or(i32::MAX))
        .bind(identity)
        .bind(source_hash)
        .bind(member_hash)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn persist_temporal_census_on(
    tx: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
    bundle_id: Uuid,
    members: &[BundleMemberRow],
    checked: &CheckedToolTruthAuthorityBundle<'_>,
) -> Result<()> {
    #[derive(Debug, sqlx::FromRow)]
    struct Decision {
        bundle_member_id: Uuid,
        root_family: String,
        receipt_id: Uuid,
        temporal_census_id: Uuid,
        temporal_policy_id: Uuid,
        temporal_policy_hash: String,
        policy_member_id: Uuid,
        evidence_class: String,
        observed_at: DateTime<Utc>,
        valid_until: DateTime<Utc>,
        source_epoch: i64,
        current_epoch: i64,
        window_started_at: DateTime<Utc>,
        window_completed_at: DateTime<Utc>,
        max_skew_ms: i64,
        temporal_status: String,
        semantic_status: String,
        decision_hash: String,
    }
    let decisions = sqlx::query_as::<_, Decision>(
        r#"SELECT bundle.id AS bundle_member_id,bundle.root_family,set_member.receipt_id,
                  temporal.id AS temporal_census_id,
                  temporal.temporal_validity_policy_id AS temporal_policy_id,
                  temporal.temporal_validity_policy_hash AS temporal_policy_hash,
                  decision.policy_member_id,decision.temporal_fact_class AS evidence_class,
                  decision.observed_at,decision.effective_valid_until AS valid_until,
                  decision.target_state_epoch AS source_epoch,head.current_epoch,
                  temporal.observation_window_started_at AS window_started_at,
                  temporal.observation_window_completed_at AS window_completed_at,
                  policy.max_cross_observation_skew_ms AS max_skew_ms,
                  temporal.temporal_validity_status AS temporal_status,
                  bundle.semantic_status,decision.member_hash AS decision_hash
             FROM tool_truth_authority_bundle_members bundle
             JOIN tool_truth_authority_set_members set_member
               ON set_member.authority_set_id=bundle.authority_set_seal_id
             JOIN capability_execution_temporal_censuses temporal
               ON temporal.receipt_id=set_member.receipt_id AND temporal.sealed_at IS NOT NULL
             JOIN capability_execution_temporal_census_members decision
               ON decision.census_id=temporal.id
             JOIN evidence_temporal_validity_policies policy
               ON policy.id=temporal.temporal_validity_policy_id
             JOIN tool_truth_target_state_epoch_heads head
               ON head.operation_id=bundle.operation_id
              AND head.organization_id=bundle.organization_id
              AND head.target_scope_identity_hash=decision.target_scope_identity_hash
            WHERE bundle.bundle_seal_id=$1
            ORDER BY bundle.ordinal,set_member.ordinal,decision.ordinal"#,
    )
    .bind(bundle_id)
    .fetch_all(&mut **tx)
    .await?;
    let decision_hashes = decisions
        .iter()
        .map(|decision| decision.decision_hash.clone())
        .collect::<Vec<_>>();
    let decision_set_hash = hash_text_array_on(tx, &decision_hashes).await?;
    let census_id = Uuid::new_v5(&snapshot_id, b"candidate_temporal_census.v1");
    let census_hash = hash_json_on(
        tx,
        &json!({
            "bundle_id":bundle_id,
            "policy_set_hash":checked.temporal_validity_policy_set_hash(),
            "epoch_set_hash":checked.target_state_epoch_set_hash(),
            "decision_set_hash":decision_set_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_temporal_validity_censuses(
               census_id,snapshot_id,tool_truth_authority_bundle_seal_id,
               temporal_validity_policy_set_hash,target_state_epoch_set_hash,
               decision_count,decision_set_hash,census_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(census_id)
    .bind(snapshot_id)
    .bind(bundle_id)
    .bind(checked.temporal_validity_policy_set_hash())
    .bind(checked.target_state_epoch_set_hash())
    .bind(i64::try_from(decisions.len()).unwrap_or(i64::MAX))
    .bind(&decision_set_hash)
    .bind(&census_hash)
    .execute(&mut **tx)
    .await?;
    let member_by_id = members
        .iter()
        .map(|member| (member.id, member))
        .collect::<BTreeMap<_, _>>();
    for (ordinal, decision) in decisions.iter().enumerate() {
        let census_member_id = Uuid::new_v5(&census_id, decision.decision_hash.as_bytes());
        sqlx::query(
            r#"INSERT INTO candidate_analysis_temporal_validity_census_members(
                   census_member_id,census_id,snapshot_id,ordinal,root_family,
                   bundle_member_id,receipt_id,temporal_census_id,temporal_policy_id,
                   temporal_policy_hash,policy_member_id,evidence_class,
                   receipt_observed_at,receipt_valid_until,source_target_state_epoch,
                   current_target_state_epoch,observation_window_started_at,
                   observation_window_completed_at,max_cross_observation_skew_ms,
                   temporal_status,semantic_status,decision_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22)"#,
        )
        .bind(census_member_id)
        .bind(census_id)
        .bind(snapshot_id)
        .bind(i32::try_from(ordinal).unwrap_or(i32::MAX))
        .bind(&decision.root_family)
        .bind(decision.bundle_member_id)
        .bind(decision.receipt_id)
        .bind(decision.temporal_census_id)
        .bind(decision.temporal_policy_id)
        .bind(&decision.temporal_policy_hash)
        .bind(decision.policy_member_id)
        .bind(&decision.evidence_class)
        .bind(decision.observed_at)
        .bind(decision.valid_until)
        .bind(decision.source_epoch)
        .bind(decision.current_epoch)
        .bind(decision.window_started_at)
        .bind(decision.window_completed_at)
        .bind(decision.max_skew_ms)
        .bind(&decision.temporal_status)
        .bind(&decision.semantic_status)
        .bind(&decision.decision_hash)
        .execute(&mut **tx)
        .await?;

        let member = member_by_id
            .get(&decision.bundle_member_id)
            .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
        if member.member_status != "consistent_fresh" {
            let reason = match member.member_status.as_str() {
                "semantic_invalid" => "authority_semantic_invalid",
                "expired" => "authority_expired",
                "mixed_epoch" => "authority_mixed_epoch",
                "skew_exceeded" => "authority_skew_exceeded",
                _ => return Err(conflict(AUTHORITY_MISMATCH)),
            };
            let residual_id = Uuid::new_v5(&census_member_id, b"candidate_stale_residual.v1");
            let root = checked
                .roots()
                .iter()
                .find(|root| root.root_family.as_str() == member.root_family)
                .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
            let residual_hash = hash_json_on(
                tx,
                &json!({"decision_hash":decision.decision_hash,"reason":reason}),
            )
            .await?;
            sqlx::query(
                r#"INSERT INTO candidate_analysis_stale_evidence_residuals(
                       residual_id,snapshot_id,temporal_census_member_id,bundle_member_id,
                       reason_code,target_state_epoch_identity_hash,required_capability,residual_hash
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
            )
            .bind(residual_id)
            .bind(snapshot_id)
            .bind(census_member_id)
            .bind(member.id)
            .bind(reason)
            .bind(&member.target_state_epoch_set_hash)
            .bind(root.root_family.as_str())
            .bind(&residual_hash)
            .execute(&mut **tx)
            .await?;
            let obligation_hash =
                hash_json_on(tx, &json!({"residual_hash":residual_hash,"reason":reason})).await?;
            sqlx::query(
                r#"INSERT INTO candidate_analysis_revalidation_obligations(
                       obligation_id,snapshot_id,stale_residual_id,
                       tool_truth_revalidation_obligation_id,root_family,evidence_identity_hash,
                       target_state_epoch_identity_hash,required_capability,reason_code,obligation_hash
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
            )
            .bind(Uuid::new_v5(&residual_id, b"candidate_revalidation.v1"))
            .bind(snapshot_id)
            .bind(residual_id)
            .bind(root.revalidation_obligation_ids.first().copied())
            .bind(&member.root_family)
            .bind(&decision.decision_hash)
            .bind(&member.target_state_epoch_set_hash)
            .bind(root.root_family.as_str())
            .bind(reason)
            .bind(obligation_hash)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

async fn persist_unavailable_feed_authority_on(
    tx: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
) -> Result<()> {
    const REQUIRED: [(&str, &str); 5] = [
        ("cve", "managed:cve"),
        ("cpe", "managed:cpe"),
        ("kev", "managed:kev"),
        ("vendor_advisory", "managed:vendor-advisory"),
        ("detection_rule", "managed:detection-rule"),
    ];
    let denominator_id = Uuid::new_v5(&snapshot_id, b"candidate_feed_denominator.v1");
    let catalog_hash = hash_json_on(
        tx,
        &json!({"catalog":"plan_b_builtin_required_five","version":1}),
    )
    .await?;
    let trust_policy_hash = hash_json_on(
        tx,
        &json!({"trust_policy":"managed_feed_required","version":1}),
    )
    .await?;
    let signature_hash = hash_json_on(tx, &json!(["ed25519", "ecdsa_p256_sha256"])).await?;
    let trust_store_hash = hash_json_on(tx, &json!({"status":"not_installed"})).await?;
    let revocation_hash = hash_json_on(tx, &json!({"epoch":0,"status":"not_installed"})).await?;
    let mut expected_hashes = Vec::new();
    for (source_kind, source_identity) in REQUIRED {
        expected_hashes.push(
            hash_json_on(
                tx,
                &json!({
                    "source_kind":source_kind,"source_identity":source_identity,
                    "schema":"managed_knowledge_feed.v1","minimum_schema_version":1,
                }),
            )
            .await?,
        );
    }
    let required_member_set_hash = hash_text_array_on(tx, &expected_hashes).await?;
    let required_source_set_hash = hash_text_array_on(
        tx,
        &REQUIRED
            .iter()
            .map(|(kind, _)| (*kind).to_owned())
            .collect::<Vec<_>>(),
    )
    .await?;
    let denominator_hash = hash_json_on(
        tx,
        &json!({
            "catalog_hash":catalog_hash,"trust_policy_hash":trust_policy_hash,
            "required_member_set_hash":required_member_set_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_knowledge_feed_denominators(
               denominator_id,snapshot_id,catalog_id,catalog_version,catalog_hash,
               trust_policy_id,trust_policy_version,trust_policy_hash,
               signature_algorithm_allowlist_hash,trust_store_version,trust_store_hash,
               key_revocation_epoch,key_revocation_epoch_hash,required_source_count,
               required_source_set_hash,required_member_count,required_member_set_hash,
               denominator_hash
           ) VALUES($1,$2,$3,1,$4,$5,1,$6,$7,1,$8,0,$9,5,$10,5,$11,$12)"#,
    )
    .bind(denominator_id)
    .bind(snapshot_id)
    .bind(Uuid::new_v5(&snapshot_id, b"candidate_feed_catalog.v1"))
    .bind(&catalog_hash)
    .bind(Uuid::new_v5(
        &snapshot_id,
        b"candidate_feed_trust_policy.v1",
    ))
    .bind(&trust_policy_hash)
    .bind(&signature_hash)
    .bind(&trust_store_hash)
    .bind(&revocation_hash)
    .bind(&required_source_set_hash)
    .bind(&required_member_set_hash)
    .bind(&denominator_hash)
    .execute(&mut **tx)
    .await?;
    let feed_snapshot_id = Uuid::new_v5(&snapshot_id, b"candidate_feed_snapshot.v1");
    let feed_snapshot_hash = hash_json_on(tx, &json!({
        "denominator_hash":denominator_hash,"members":expected_hashes,"disposition":"unavailable",
    })).await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_knowledge_feed_snapshots(
               feed_snapshot_id,snapshot_id,denominator_id,trust_policy_hash,
               trust_store_hash,key_revocation_epoch,member_count,member_set_hash,
               feed_snapshot_hash
           ) VALUES($1,$2,$3,$4,$5,0,5,$6,$7)"#,
    )
    .bind(feed_snapshot_id)
    .bind(snapshot_id)
    .bind(denominator_id)
    .bind(&trust_policy_hash)
    .bind(&trust_store_hash)
    .bind(&required_member_set_hash)
    .bind(&feed_snapshot_hash)
    .execute(&mut **tx)
    .await?;
    for (ordinal, ((source_kind, source_identity), member_hash)) in
        REQUIRED.into_iter().zip(expected_hashes).enumerate()
    {
        let expected_member_id = Uuid::new_v5(&denominator_id, source_identity.as_bytes());
        sqlx::query(
            r#"INSERT INTO candidate_analysis_knowledge_feed_denominator_members(
                   expected_member_id,denominator_id,snapshot_id,ordinal,source_kind,
                   source_identity,schema_name,minimum_schema_version,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,'managed_knowledge_feed.v1',1,$7)"#,
        )
        .bind(expected_member_id)
        .bind(denominator_id)
        .bind(snapshot_id)
        .bind(i32::try_from(ordinal).unwrap_or(i32::MAX))
        .bind(source_kind)
        .bind(source_identity)
        .bind(&member_hash)
        .execute(&mut **tx)
        .await?;
        let feed_member_id = Uuid::new_v5(&feed_snapshot_id, source_identity.as_bytes());
        sqlx::query(
            r#"INSERT INTO candidate_analysis_knowledge_feed_snapshot_members(
                   feed_snapshot_member_id,feed_snapshot_id,snapshot_id,denominator_id,
                   expected_member_id,ordinal,feed_schema,age_policy_version,
                   age_policy_digest,disposition,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,'managed_knowledge_feed.v1','1',$7,'unavailable',$8)"#,
        )
        .bind(feed_member_id)
        .bind(feed_snapshot_id)
        .bind(snapshot_id)
        .bind(denominator_id)
        .bind(expected_member_id)
        .bind(i32::try_from(ordinal).unwrap_or(i32::MAX))
        .bind(&trust_policy_hash)
        .bind(&member_hash)
        .execute(&mut **tx)
        .await?;
        let obligation_hash = hash_json_on(tx, &json!({
            "source_kind":source_kind,"source_identity":source_identity,"reason":"feed_unavailable",
        })).await?;
        sqlx::query(
            r#"INSERT INTO candidate_analysis_enrichment_obligations(
                   obligation_id,snapshot_id,obligation_kind,feed_snapshot_member_id,
                   reason_code,affected_checklist_member_key,obligation_hash
               ) VALUES($1,$2,'feed_refresh',$3,'feed_unavailable',$4,$5)"#,
        )
        .bind(Uuid::new_v5(&feed_member_id, b"candidate_feed_refresh.v1"))
        .bind(snapshot_id)
        .bind(feed_member_id)
        .bind(format!("feed:{source_kind}"))
        .bind(obligation_hash)
        .execute(&mut **tx)
        .await?;
    }
    let application_model_hash = hash_json_on(tx, &json!({"status":"not_installed"})).await?;
    let empty_set_hash = hash_text_array_on(tx, &[]).await?;
    let product_census_id = Uuid::new_v5(&snapshot_id, b"candidate_product_census.v1");
    let product_census_hash = hash_json_on(tx, &json!({"products":empty_set_hash})).await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_product_version_censuses(
               product_census_id,snapshot_id,application_model_authority_hash,
               product_count,product_set_hash,census_hash
           ) VALUES($1,$2,$3,0,$4,$5)"#,
    )
    .bind(product_census_id)
    .bind(snapshot_id)
    .bind(application_model_hash)
    .bind(&empty_set_hash)
    .bind(&product_census_hash)
    .execute(&mut **tx)
    .await?;
    let match_census_id = Uuid::new_v5(&snapshot_id, b"candidate_feed_match_census.v1");
    let match_census_hash = hash_json_on(tx, &json!({
        "product_set_hash":empty_set_hash,"feed_set_hash":required_member_set_hash,"matches":empty_set_hash,
    })).await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_feed_match_censuses(
               match_census_id,snapshot_id,product_census_id,feed_snapshot_id,
               matcher_contract_version,matcher_contract_digest,input_product_count,
               input_product_set_hash,input_feed_count,input_feed_set_hash,
               member_count,member_set_hash,census_hash
           ) VALUES($1,$2,$3,$4,'candidate_feed_matcher.v1',$5,0,$6,5,$7,0,$6,$8)"#,
    )
    .bind(match_census_id)
    .bind(snapshot_id)
    .bind(product_census_id)
    .bind(feed_snapshot_id)
    .bind(&trust_policy_hash)
    .bind(&empty_set_hash)
    .bind(&required_member_set_hash)
    .bind(&match_census_hash)
    .execute(&mut **tx)
    .await?;
    persist_source_set_on(
        tx,
        snapshot_id,
        "managed_knowledge_feed",
        vec![(feed_snapshot_id.to_string(), feed_snapshot_hash)],
    )
    .await?;
    Ok(())
}

pub(crate) async fn load_snapshot_on(
    tx: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
) -> Result<CandidateSnapshotRowView> {
    #[derive(sqlx::FromRow)]
    struct Snapshot {
        operation_id: Uuid,
        organization_id: Uuid,
        scope_snapshot_id: Option<Uuid>,
        snapshot_status: String,
        tool_truth_authority_bundle_seal_id: Uuid,
        stable_consumer_request_id: Uuid,
        relevant_root_count: i64,
        relevant_root_set_hash: String,
        bundle_member_count: i64,
        bundle_member_set_hash: String,
        semantic_authority_bundle_hash: String,
        freshness_attestation_bundle_hash: String,
        temporal_validity_bundle_hash: String,
        temporal_validity_policy_set_hash: String,
        target_state_epoch_set_hash: String,
        observation_window_hash: String,
        candidate_snapshot_authority_hash: String,
        bundle_sealed_at: DateTime<Utc>,
    }
    let snapshot = sqlx::query_as::<_, Snapshot>(
        r#"SELECT operation_id,organization_id,scope_snapshot_id,snapshot_status,
                  tool_truth_authority_bundle_seal_id,stable_consumer_request_id,
                  relevant_root_count,relevant_root_set_hash,bundle_member_count,
                  bundle_member_set_hash,semantic_authority_bundle_hash,
                  freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
                  temporal_validity_policy_set_hash,target_state_epoch_set_hash,
                  observation_window_hash,candidate_snapshot_authority_hash,bundle_sealed_at
             FROM candidate_analysis_snapshots WHERE snapshot_id=$1"#,
    )
    .bind(snapshot_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| DbError::NotFound("candidate_analysis_snapshot".into()))?;
    let member_rows = sqlx::query_as::<_, BundleMemberRow>(
        r#"SELECT tool_truth_authority_bundle_member_id AS id,bundle_seal_id,
                  operation_id,organization_id,ordinal,root_family,
                  root_execution_authority_id,root_denominator_id,root_denominator_hash,
                  authority_set_seal_id,authority_set_semantic_hash,
                  authority_set_graph_hash,authority_set_freshness_hash,
                  temporal_validity_policy_set_hash,target_state_epoch_set_hash,
                  semantic_status,temporal_validity_status,member_status,member_hash
             FROM candidate_analysis_snapshot_authority_bundle_members
            WHERE snapshot_id=$1 ORDER BY ordinal"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut roots = Vec::with_capacity(member_rows.len());
    let mut graph_hashes = Vec::new();
    let mut receipt_hashes = Vec::new();
    let mut receipt_count = 0i64;
    for member in member_rows {
        let (set_count, set_hash): (Option<i64>, Option<String>) = sqlx::query_as(
            "SELECT member_count,member_set_hash FROM tool_truth_authority_set_seals WHERE id=$1",
        )
        .bind(member.authority_set_seal_id)
        .fetch_one(&mut **tx)
        .await?;
        let temporal_decision_hash: String = sqlx::query_scalar(
            r#"SELECT COALESCE(tool_truth_sha256(to_jsonb(array_agg(decision_hash ORDER BY ordinal))::TEXT),
                               tool_truth_sha256('[]'))
                 FROM candidate_analysis_temporal_validity_census_members
                WHERE snapshot_id=$1 AND root_family=$2"#,
        )
        .bind(snapshot_id)
        .bind(&member.root_family)
        .fetch_one(&mut **tx)
        .await?;
        let policies = sqlx::query_as::<_, (Uuid, Uuid, String, i64, String, String)>(
            r#"SELECT policy.id,policy.execution_authority_id,policy.policy_contract_version,
                      policy.max_cross_observation_skew_ms,policy.policy_hash,policy.member_set_hash
                 FROM evidence_temporal_validity_policies policy
                WHERE policy.execution_authority_id=$1 AND policy.sealed_at IS NOT NULL
                ORDER BY policy.id"#,
        )
        .bind(member.root_execution_authority_id)
        .fetch_all(&mut **tx)
        .await?;
        let mut typed_policies = Vec::with_capacity(policies.len());
        for (id, execution_authority_id, version, max_skew, policy_hash, member_set_hash) in
            policies
        {
            let raw_members = sqlx::query_as::<_, (i32, String, i64, i64, i64, bool, String, String)>(
                r#"SELECT ordinal,fact_class,positive_ttl_ms,negative_ttl_ms,refutation_ttl_ms,
                          require_same_target_state_epoch,required_recheck_source,member_hash
                     FROM evidence_temporal_validity_policy_members WHERE policy_id=$1 ORDER BY ordinal"#,
            )
            .bind(id)
            .fetch_all(&mut **tx)
            .await?;
            typed_policies.push(EvidenceTemporalValidityPolicyV1 {
                id,
                execution_authority_id,
                policy_contract_version: version,
                max_cross_observation_skew_ms: u64::try_from(max_skew)
                    .map_err(|_| conflict(AUTHORITY_MISMATCH))?,
                policy_hash,
                member_set_hash,
                members: raw_members
                    .into_iter()
                    .map(|row| {
                        golish_pentest_domain::tool_truth::EvidenceTemporalValidityPolicyMemberV1 {
                            ordinal: u32::try_from(row.0).unwrap_or(u32::MAX),
                            fact_class: row.1,
                            positive_ttl_ms: u64::try_from(row.2).unwrap_or(0),
                            negative_ttl_ms: u64::try_from(row.3).unwrap_or(0),
                            refutation_ttl_ms: u64::try_from(row.4).unwrap_or(0),
                            require_same_target_state_epoch: row.5,
                            required_recheck_source: row.6,
                            member_hash: row.7,
                        }
                    })
                    .collect(),
            });
        }
        let set_count = set_count.unwrap_or(0);
        let set_hash = set_hash.unwrap_or_else(|| {
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into()
        });
        receipt_count += set_count;
        receipt_hashes.push(set_hash.clone());
        graph_hashes.push(member.authority_set_graph_hash.clone());
        roots.push(CandidateAuthorityRootRowView {
            ordinal: member.ordinal,
            root_family: ToolTruthRootFamilyV1::try_from(member.root_family.as_str())
                .map_err(|_| conflict(AUTHORITY_MISMATCH))?,
            root_denominator_id: member.root_denominator_id,
            root_denominator_hash: member.root_denominator_hash,
            authority_set_seal_id: member.authority_set_seal_id,
            authority_set_graph_hash: member.authority_set_graph_hash,
            authority_set_semantic_hash: member.authority_set_semantic_hash,
            authority_set_freshness_hash: member.authority_set_freshness_hash,
            temporal_validity_policy_set_hash: member.temporal_validity_policy_set_hash,
            temporal_validity_decision_set_hash: temporal_decision_hash,
            target_state_epoch_set_hash: member.target_state_epoch_set_hash,
            receipt_count: set_count,
            receipt_set_hash: set_hash,
            semantic_status: member.semantic_status,
            temporal_status: parse_temporal_status(&member.temporal_validity_status)?,
            temporal_policies: typed_policies,
            member_hash: member.member_hash,
        });
    }
    let denominator_graph_bundle_hash = hash_text_array_on(tx, &graph_hashes).await?;
    let receipt_set_hash = hash_text_array_on(tx, &receipt_hashes).await?;
    let (temporal_decision_set_hash, stale_set_hash): (String, String) = sqlx::query_as(
        r#"SELECT
             COALESCE((SELECT decision_set_hash FROM candidate_analysis_temporal_validity_censuses WHERE snapshot_id=$1),tool_truth_sha256('[]')),
             COALESCE((SELECT tool_truth_sha256(to_jsonb(array_agg(obligation_hash ORDER BY obligation_hash))::TEXT)
                         FROM candidate_analysis_revalidation_obligations WHERE snapshot_id=$1),tool_truth_sha256('[]'))"#,
    )
    .bind(snapshot_id)
    .fetch_one(&mut **tx)
    .await?;
    let feed: (String, String, String, String, String, String, String, String, String) = sqlx::query_as(
        r#"SELECT denominator.catalog_hash,denominator.required_member_set_hash,
                  denominator.signature_algorithm_allowlist_hash,denominator.trust_store_hash,
                  denominator.key_revocation_epoch_hash,feed.feed_snapshot_hash,
                  product.census_hash,match.census_hash,
                  COALESCE((SELECT tool_truth_sha256(to_jsonb(array_agg(obligation_hash ORDER BY obligation_hash))::TEXT)
                              FROM candidate_analysis_enrichment_obligations obligation
                             WHERE obligation.snapshot_id=$1),tool_truth_sha256('[]'))
             FROM candidate_analysis_knowledge_feed_denominators denominator
             JOIN candidate_analysis_knowledge_feed_snapshots feed USING(snapshot_id)
             JOIN candidate_analysis_product_version_censuses product USING(snapshot_id)
             JOIN candidate_analysis_feed_match_censuses match USING(snapshot_id)
            WHERE denominator.snapshot_id=$1"#,
    )
    .bind(snapshot_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
    Ok(CandidateSnapshotRowView {
        snapshot_id,
        stable_consumer_request_id: snapshot.stable_consumer_request_id,
        operation_id: snapshot.operation_id,
        scope_snapshot_id: snapshot
            .scope_snapshot_id
            .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?,
        organization_id: snapshot.organization_id,
        disposition: CandidateSnapshotDispositionRow::parse(&snapshot.snapshot_status)?,
        snapshot_hash: snapshot.candidate_snapshot_authority_hash.clone(),
        candidate_snapshot_authority_hash: snapshot.candidate_snapshot_authority_hash,
        tool_truth_authority_bundle_seal_id: snapshot.tool_truth_authority_bundle_seal_id,
        tool_truth_authority_root_count: snapshot.relevant_root_count,
        tool_truth_authority_root_set_hash: snapshot.relevant_root_set_hash,
        tool_truth_authority_bundle_member_count: snapshot.bundle_member_count,
        tool_truth_authority_bundle_member_set_hash: snapshot.bundle_member_set_hash,
        tool_truth_authority_receipt_count: receipt_count,
        tool_truth_authority_receipt_set_hash: receipt_set_hash,
        denominator_graph_bundle_hash,
        semantic_authority_bundle_hash: snapshot.semantic_authority_bundle_hash,
        freshness_attestation_bundle_hash: snapshot.freshness_attestation_bundle_hash,
        temporal_validity_bundle_hash: snapshot.temporal_validity_bundle_hash,
        temporal_validity_policy_set_hash: snapshot.temporal_validity_policy_set_hash,
        temporal_validity_decision_set_hash: temporal_decision_set_hash,
        observation_window_hash: snapshot.observation_window_hash,
        target_state_epoch_set_hash: snapshot.target_state_epoch_set_hash,
        authority_roots: roots,
        knowledge_feed_catalog_policy_seal_hash: feed.0,
        knowledge_feed_required_member_set_hash: feed.1,
        knowledge_feed_signature_algorithm_set_hash: feed.2,
        knowledge_feed_trust_store_hash: feed.3,
        knowledge_feed_key_revocation_epoch_hash: feed.4,
        knowledge_feed_snapshot_set_hash: feed.5,
        product_version_census_hash: feed.6,
        knowledge_feed_match_census_hash: feed.7,
        stale_revalidation_obligation_set_hash: stale_set_hash,
        knowledge_feed_obligation_set_hash: feed.8,
        row_version: 0,
        sealed_at: snapshot.bundle_sealed_at,
    })
}

/// DB-clock reevaluation performed immediately before Gate material is
/// returned and again by the canonical apply transaction.  The hashes are
/// derived only from locked/frozen authority rows plus current epoch heads;
/// callers cannot supply a clock, epoch list, signer status, or feed member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateGateReevaluationRow {
    pub temporal_hash: String,
    pub knowledge_feed_hash: String,
}

pub(crate) async fn reevaluate_candidate_gate_authority_on(
    tx: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
) -> Result<CandidateGateReevaluationRow> {
    // Lock mutable epoch/trust heads so a successful re-evaluation remains
    // true through the surrounding canonical apply transaction.
    sqlx::query(
        r#"SELECT head.current_event_id
             FROM tool_truth_target_state_epoch_heads head
             JOIN capability_execution_temporal_census_members temporal
               ON temporal.target_state_operation_id=head.operation_id
              AND temporal.target_state_organization_id=head.organization_id
              AND temporal.target_scope_identity_hash=head.target_scope_identity_hash
             JOIN tool_truth_authority_set_members authority
               ON authority.receipt_id=temporal.receipt_id
             JOIN candidate_analysis_snapshot_authority_bundle_members snapshot_member
               ON snapshot_member.authority_set_seal_id=authority.authority_set_id
            WHERE snapshot_member.snapshot_id=$1
            FOR SHARE OF head"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    let temporal_valid: bool = sqlx::query_scalar(
        r#"SELECT
            NOT EXISTS(
                SELECT 1
                  FROM candidate_analysis_snapshot_authority_bundle_members snapshot_member
                  JOIN tool_truth_authority_bundle_members bundle_member
                    ON bundle_member.id=snapshot_member.tool_truth_authority_bundle_member_id
                 WHERE snapshot_member.snapshot_id=$1
                   AND (snapshot_member.member_status<>'consistent_fresh'
                        OR snapshot_member.semantic_status<>'consistent'
                        OR snapshot_member.temporal_validity_status<>'fresh'
                        OR bundle_member.effective_valid_until IS NULL
                        OR bundle_member.effective_valid_until<statement_timestamp())
            )
            AND NOT EXISTS(
                SELECT 1
                  FROM candidate_analysis_snapshot_authority_bundle_members snapshot_member
                  JOIN tool_truth_authority_set_members authority_member
                    ON authority_member.authority_set_id=snapshot_member.authority_set_seal_id
                  JOIN capability_execution_temporal_census_members temporal_member
                    ON temporal_member.receipt_id=authority_member.receipt_id
                  LEFT JOIN tool_truth_target_state_epoch_heads current_head
                    ON current_head.operation_id=temporal_member.target_state_operation_id
                   AND current_head.organization_id=temporal_member.target_state_organization_id
                   AND current_head.target_scope_identity_hash=
                       temporal_member.target_scope_identity_hash
                 WHERE snapshot_member.snapshot_id=$1
                   AND (temporal_member.effective_valid_until<statement_timestamp()
                        OR current_head.current_event_id IS DISTINCT FROM
                           temporal_member.target_state_epoch_event_id
                        OR current_head.current_epoch IS DISTINCT FROM
                           temporal_member.target_state_epoch)
            )"#,
    )
    .bind(snapshot_id)
    .fetch_one(&mut **tx)
    .await?;
    if !temporal_valid {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let temporal_manifest: Value = sqlx::query_scalar(
        r#"SELECT COALESCE(jsonb_agg(jsonb_build_object(
                    'root_family',snapshot_member.root_family,
                    'bundle_member_hash',snapshot_member.member_hash,
                    'receipt_id',authority_member.receipt_id,
                    'target_scope_identity_hash',temporal_member.target_scope_identity_hash,
                    'target_state_epoch_event_id',temporal_member.target_state_epoch_event_id,
                    'target_state_epoch',temporal_member.target_state_epoch,
                    'current_event_id',current_head.current_event_id,
                    'current_epoch',current_head.current_epoch,
                    'effective_valid_until',temporal_member.effective_valid_until
                ) ORDER BY snapshot_member.ordinal,authority_member.ordinal,
                           temporal_member.ordinal),'[]'::JSONB)
              FROM candidate_analysis_snapshot_authority_bundle_members snapshot_member
              LEFT JOIN tool_truth_authority_set_members authority_member
                ON authority_member.authority_set_id=snapshot_member.authority_set_seal_id
              LEFT JOIN capability_execution_temporal_census_members temporal_member
                ON temporal_member.receipt_id=authority_member.receipt_id
              LEFT JOIN tool_truth_target_state_epoch_heads current_head
                ON current_head.operation_id=temporal_member.target_state_operation_id
               AND current_head.organization_id=temporal_member.target_state_organization_id
               AND current_head.target_scope_identity_hash=temporal_member.target_scope_identity_hash
             WHERE snapshot_member.snapshot_id=$1"#,
    )
    .bind(snapshot_id)
    .fetch_one(&mut **tx)
    .await?;
    let temporal_hash = hash_json_on(
        tx,
        &json!({
            "domain":"candidate_gate_temporal_reevaluation.v1",
            "snapshot_id":snapshot_id,
            "authority_manifest":temporal_manifest,
        }),
    )
    .await?;

    let trust_head: Option<(i64, String, i64, String)> = sqlx::query_as(
        r#"SELECT trust_store_version,trust_store_hash,key_revocation_epoch,
                  key_revocation_epoch_hash
             FROM candidate_managed_feed_trust_store_head WHERE singleton FOR SHARE"#,
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some((trust_store_version, trust_store_hash, revocation_epoch, revocation_hash)) =
        trust_head
    else {
        return Err(conflict(AUTHORITY_MISMATCH));
    };
    let feed_valid: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM candidate_analysis_knowledge_feed_denominators denominator
                 JOIN candidate_analysis_knowledge_feed_snapshots feed USING(snapshot_id)
                WHERE denominator.snapshot_id=$1
                  AND denominator.trust_store_version=$2
                  AND denominator.trust_store_hash=$3
                  AND denominator.key_revocation_epoch=$4
                  AND denominator.key_revocation_epoch_hash=$5
                  AND denominator.trust_policy_hash=feed.trust_policy_hash
                  AND denominator.trust_store_hash=feed.trust_store_hash
                  AND denominator.key_revocation_epoch=feed.key_revocation_epoch
                  AND denominator.required_member_count=feed.member_count
                  AND denominator.required_member_set_hash=(
                      SELECT tool_truth_sha256(to_jsonb(array_agg(member.member_hash
                                 ORDER BY member.ordinal))::TEXT)
                        FROM candidate_analysis_knowledge_feed_denominator_members member
                       WHERE member.denominator_id=denominator.denominator_id)
                  AND feed.member_set_hash=(
                      SELECT tool_truth_sha256(to_jsonb(array_agg(member.member_hash
                                 ORDER BY member.ordinal))::TEXT)
                        FROM candidate_analysis_knowledge_feed_snapshot_members member
                       WHERE member.feed_snapshot_id=feed.feed_snapshot_id)
                  AND NOT EXISTS(
                      SELECT 1 FROM candidate_analysis_knowledge_feed_denominator_members expected
                      LEFT JOIN candidate_analysis_knowledge_feed_snapshot_members member
                        ON member.expected_member_id=expected.expected_member_id
                       AND member.feed_snapshot_id=feed.feed_snapshot_id
                      LEFT JOIN candidate_managed_feed_signer_keys signer
                        ON signer.trust_store_version=denominator.trust_store_version
                       AND signer.signer_id=member.signer_id
                       AND signer.signer_key_id=member.signer_key_id
                       AND signer.signature_algorithm=member.signature_algorithm
                       AND signer.key_member_hash=member.signer_key_member_hash
                       AND signer.revoked=FALSE
                     WHERE expected.denominator_id=denominator.denominator_id
                       AND (member.feed_snapshot_member_id IS NULL
                            OR member.disposition<>'current'
                            OR member.effective_valid_until<=statement_timestamp()
                            OR member.published_at IS NULL
                            OR member.published_at>statement_timestamp()
                            OR member.host_ingested_at IS NULL
                            OR member.host_ingested_at>statement_timestamp()
                            OR member.signature_algorithm NOT IN (
                                'ed25519','ecdsa_p256_sha256')
                            OR signer.signer_key_member_id IS NULL)
                  )
                  AND (SELECT COUNT(*)
                         FROM candidate_analysis_knowledge_feed_snapshot_members member
                        WHERE member.snapshot_id=$1)=denominator.required_member_count
           )"#,
    )
    .bind(snapshot_id)
    .bind(trust_store_version)
    .bind(&trust_store_hash)
    .bind(revocation_epoch)
    .bind(&revocation_hash)
    .fetch_one(&mut **tx)
    .await?;
    if !feed_valid {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let feed_manifest: Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
                 'catalog_hash',denominator.catalog_hash,
                 'trust_policy_hash',denominator.trust_policy_hash,
                 'signature_algorithm_allowlist_hash',denominator.signature_algorithm_allowlist_hash,
                 'trust_store_hash',denominator.trust_store_hash,
                 'key_revocation_epoch_hash',denominator.key_revocation_epoch_hash,
                 'required_member_set_hash',denominator.required_member_set_hash,
                 'feed_snapshot_hash',feed.feed_snapshot_hash,
                 'feed_members',COALESCE((
                     SELECT jsonb_agg(jsonb_build_object(
                         'member_hash',member.member_hash,
                         'signer_key_member_hash',member.signer_key_member_hash,
                         'signature_algorithm',member.signature_algorithm,
                         'effective_valid_until',member.effective_valid_until,
                         'published_at',member.published_at,
                         'host_ingested_at',member.host_ingested_at
                     ) ORDER BY member.ordinal)
                       FROM candidate_analysis_knowledge_feed_snapshot_members member
                      WHERE member.snapshot_id=$1
                 ),'[]'::JSONB),
                 'product_census_hash',product.census_hash,
                 'match_census_hash',match.census_hash)
              FROM candidate_analysis_knowledge_feed_denominators denominator
              JOIN candidate_analysis_knowledge_feed_snapshots feed USING(snapshot_id)
              JOIN candidate_analysis_product_version_censuses product USING(snapshot_id)
              JOIN candidate_analysis_feed_match_censuses match USING(snapshot_id)
             WHERE denominator.snapshot_id=$1"#,
    )
    .bind(snapshot_id)
    .fetch_one(&mut **tx)
    .await?;
    let knowledge_feed_hash = hash_json_on(
        tx,
        &json!({
            "domain":"candidate_gate_knowledge_feed_reevaluation.v1",
            "snapshot_id":snapshot_id,
            "authority_manifest":feed_manifest,
        }),
    )
    .await?;
    Ok(CandidateGateReevaluationRow {
        temporal_hash,
        knowledge_feed_hash,
    })
}

pub(crate) async fn validate_write_fence_on(
    tx: &mut Transaction<'_, Postgres>,
    fence: &CandidateWriteFenceRow,
) -> Result<()> {
    if fence.expected_snapshot_row_version != 0 || fence.expected_attempt_row_version != 0 {
        return Err(conflict(WRITE_FENCE_MISMATCH));
    }
    let valid: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM candidate_analysis_snapshots snapshot
                 JOIN candidate_analysis_attempts attempt
                   ON attempt.snapshot_id=snapshot.snapshot_id
                  AND attempt.operation_id=snapshot.operation_id
                  AND attempt.organization_id=snapshot.organization_id
                 JOIN stage_team_plans plan
                   ON plan.id=$5 AND plan.operation_id=snapshot.operation_id
                  AND plan.scope_snapshot_id=snapshot.scope_snapshot_id
                  AND plan.organization_id=snapshot.organization_id
                 JOIN stage_work_items item
                   ON item.id=$6 AND item.team_plan_id=plan.id
                  AND item.operation_id=plan.operation_id
                  AND item.scope_snapshot_id=plan.scope_snapshot_id
                  AND item.organization_id=plan.organization_id
                 JOIN stage_worker_runs worker
                   ON worker.id=$7 AND worker.operation_id=item.operation_id
                  AND worker.stage_execution_id=item.stage_execution_id
                  AND worker.stage_run_unit_id=item.stage_run_unit_id
                  AND worker.organization_id=item.organization_id
                WHERE snapshot.snapshot_id=$4 AND snapshot.operation_id=$1
                  AND snapshot.scope_snapshot_id=$2 AND snapshot.organization_id=$3
                  AND snapshot.snapshot_status='sealed_ready'
                  AND attempt.analysis_attempt_id=$8
                  AND attempt.attempt_ordinal=$9
                  AND plan.row_version=$10 AND item.row_version=$11
                  AND worker.checkpoint_version=$12
                  AND worker.lease_token=$13 AND worker.attempt_epoch=$14
                  AND worker.lease_expires_at>statement_timestamp()
           )"#,
    )
    .bind(fence.operation_id)
    .bind(fence.scope_snapshot_id)
    .bind(fence.organization_id)
    .bind(fence.snapshot_id)
    .bind(fence.team_plan_id)
    .bind(fence.work_item_id)
    .bind(fence.worker_run_id)
    .bind(fence.analysis_attempt_id)
    .bind(fence.analysis_attempt_ordinal)
    .bind(fence.expected_team_plan_row_version)
    .bind(fence.expected_work_item_row_version)
    .bind(fence.expected_worker_row_version)
    .bind(fence.lease_token)
    .bind(fence.attempt_epoch)
    .fetch_one(&mut **tx)
    .await?;
    if !valid || fence.lease_epoch != fence.attempt_epoch {
        return Err(conflict(WRITE_FENCE_MISMATCH));
    }
    Ok(())
}

pub(crate) async fn validate_final_submitter_fence_on(
    tx: &mut Transaction<'_, Postgres>,
    fence: &CandidateWriteFenceRow,
) -> Result<()> {
    let valid: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM stage_team_plans plan
                WHERE plan.id=$1
                  AND plan.operation_id=$2
                  AND plan.scope_snapshot_id=$3
                  AND plan.organization_id=$4
                  AND plan.final_submitter_worker_run_id=$5
           )"#,
    )
    .bind(fence.team_plan_id)
    .bind(fence.operation_id)
    .bind(fence.scope_snapshot_id)
    .bind(fence.organization_id)
    .bind(fence.worker_run_id)
    .fetch_one(&mut **tx)
    .await?;
    if !valid {
        return Err(conflict(WRITE_FENCE_MISMATCH));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadSnapshotPageInput {
    pub fence: CandidateWriteFenceRow,
    pub stable_page_request_id: Uuid,
    pub after_input_ordinal: Option<i32>,
    pub page_size: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotPageItemRow {
    pub input_id: Uuid,
    pub ordinal: i32,
    pub input_kind: String,
    pub stable_key: String,
    pub source_hash: String,
    pub source_size_bytes: i64,
    pub body: Value,
    pub body_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotPageRowView {
    pub snapshot_id: Uuid,
    pub page_receipt_id: Uuid,
    pub first_input_ordinal: Option<i32>,
    pub last_input_ordinal: Option<i32>,
    pub returned_count: i64,
    pub page_hash: String,
    pub items: Vec<SnapshotPageItemRow>,
    pub next_input_ordinal: Option<i32>,
    pub replayed: bool,
}

#[derive(sqlx::FromRow)]
struct SnapshotPageInputDbRow {
    snapshot_input_id: Uuid,
    stable_input_key: String,
    source_kind: String,
    source_content_hash: String,
    source_byte_count: i64,
    descriptor_body: Value,
    descriptor_hash: String,
    ordinal: i64,
}

pub async fn load_snapshot_page(
    pool: &PgPool,
    input: LoadSnapshotPageInput,
) -> Result<SnapshotPageRowView> {
    if !(1..=256).contains(&input.page_size) {
        return Err(conflict(PAGE_SIZE_INVALID));
    }
    let mut tx = pool.begin().await?;
    validate_write_fence_on(&mut tx, &input.fence).await?;
    let after = input.after_input_ordinal.unwrap_or(-1);
    let rows = sqlx::query_as::<_, SnapshotPageInputDbRow>(
        r#"SELECT source.snapshot_input_id,source.stable_input_key,source.source_kind,
                  source.source_content_hash,source.source_byte_count,
                  jsonb_build_object(
                      'schema','candidate_snapshot_input_descriptor.v1',
                      'stable_input_key',source.stable_input_key,
                      'source_kind',source.source_kind,
                      'source_content_hash',source.source_content_hash,
                      'source_byte_count',source.source_byte_count,
                      'server_chunking_disposition',source.server_chunking_disposition,
                      'chunk_census_id',census.chunk_census_id,
                      'chunk_census_hash',census.census_hash,
                      'chunk_count',census.chunk_count,
                      'chunking_contract_version',census.chunking_contract_version,
                      'redaction_contract_version',census.redaction_contract_version
                  ) AS descriptor_body,
                  tool_truth_sha256((jsonb_build_object(
                      'schema','candidate_snapshot_input_descriptor.v1',
                      'stable_input_key',source.stable_input_key,
                      'source_kind',source.source_kind,
                      'source_content_hash',source.source_content_hash,
                      'source_byte_count',source.source_byte_count,
                      'server_chunking_disposition',source.server_chunking_disposition,
                      'chunk_census_id',census.chunk_census_id,
                      'chunk_census_hash',census.census_hash,
                      'chunk_count',census.chunk_count,
                      'chunking_contract_version',census.chunking_contract_version,
                      'redaction_contract_version',census.redaction_contract_version
                  ))::TEXT) AS descriptor_hash,
                  ROW_NUMBER() OVER(ORDER BY source.stable_input_key)-1 AS ordinal
             FROM candidate_analysis_snapshot_inputs source
             JOIN candidate_analysis_input_chunk_censuses census
               ON census.snapshot_input_id=source.snapshot_input_id
            WHERE source.snapshot_id=$1
            ORDER BY source.stable_input_key OFFSET $2 LIMIT $3"#,
    )
    .bind(input.fence.snapshot_id)
    .bind(i64::from(after + 1))
    .bind(input.page_size as i64)
    .fetch_all(&mut *tx)
    .await?;
    let items = rows
        .into_iter()
        .map(|row| SnapshotPageItemRow {
            input_id: row.snapshot_input_id,
            stable_key: row.stable_input_key,
            input_kind: row.source_kind,
            source_hash: row.source_content_hash,
            source_size_bytes: row.source_byte_count,
            body: row.descriptor_body,
            body_hash: row.descriptor_hash,
            ordinal: i32::try_from(row.ordinal).unwrap_or(i32::MAX),
        })
        .collect::<Vec<_>>();
    let first = items.first().map(|item| item.ordinal);
    let last = items.last().map(|item| item.ordinal);
    let page_hash = hash_json_on(&mut tx, &json!({
        "snapshot_id":input.fence.snapshot_id,"first":first,"last":last,
        "members":items.iter().map(|item| (&item.stable_key,&item.source_hash)).collect::<Vec<_>>(),
    })).await?;
    let receipt_id = Uuid::new_v5(&input.stable_page_request_id, b"candidate_page_receipt.v1");
    let cursor = format!("input:{}:{}", after, input.page_size);
    let existing = sqlx::query_as::<_, (String, String)>(
        "SELECT page_hash,server_cursor FROM candidate_analysis_page_receipts WHERE page_receipt_id=$1",
    )
    .bind(receipt_id)
    .fetch_optional(&mut *tx)
    .await?;
    let replayed = existing.is_some();
    if let Some((existing, existing_cursor)) = existing {
        if existing != page_hash || existing_cursor != cursor {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
    } else {
        sqlx::query(
            r#"INSERT INTO candidate_analysis_page_receipts(
            page_receipt_id,analysis_attempt_id,snapshot_id,page_kind,stable_request_id,
            consumer_worker_run_id,
            server_cursor,first_key,last_key,returned_count,page_hash)
            VALUES($1,$2,$3,'input_page',$4,$5,$6,$7,$8,$9,$10)"#,
        )
        .bind(receipt_id)
        .bind(input.fence.analysis_attempt_id)
        .bind(input.fence.snapshot_id)
        .bind(input.stable_page_request_id)
        .bind(input.fence.worker_run_id)
        .bind(cursor)
        .bind(first.map(|v| v.to_string()))
        .bind(last.map(|v| v.to_string()))
        .bind(i64::try_from(items.len()).unwrap_or(i64::MAX))
        .bind(&page_hash)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(SnapshotPageRowView {
        snapshot_id: input.fence.snapshot_id,
        page_receipt_id: receipt_id,
        first_input_ordinal: first,
        last_input_ordinal: last,
        returned_count: items.len() as i64,
        page_hash,
        next_input_ordinal: last.map(|v| v + 1),
        items,
        replayed,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadSnapshotChunkPageInput {
    pub fence: CandidateWriteFenceRow,
    pub stable_page_request_id: Uuid,
    pub input_id: Uuid,
    pub chunk_census_id: Uuid,
    pub chunk_census_hash: String,
    pub source_size_bytes: i64,
    pub chunking_contract_version: String,
    pub redaction_contract_version: String,
    pub first_chunk_ordinal: i32,
    pub max_chunks: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotChunkRowView {
    pub chunk_id: Uuid,
    pub chunk_ordinal: i32,
    pub source_range_start: i64,
    pub source_range_end: i64,
    pub chunk_hash: String,
    pub body_hash: String,
    pub body: Value,
}
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotChunkPageRowView {
    pub snapshot_id: Uuid,
    pub input_id: Uuid,
    pub chunk_census_id: Uuid,
    pub chunk_census_hash: String,
    pub source_size_bytes: i64,
    pub chunking_contract_version: String,
    pub redaction_contract_version: String,
    pub page_receipt_id: Uuid,
    pub first_chunk_ordinal: Option<i32>,
    pub last_chunk_ordinal: Option<i32>,
    pub returned_count: i64,
    pub page_hash: String,
    pub chunks: Vec<SnapshotChunkRowView>,
    pub next_chunk_ordinal: Option<i32>,
    pub replayed: bool,
}

pub async fn load_snapshot_chunk_page(
    pool: &PgPool,
    input: LoadSnapshotChunkPageInput,
) -> Result<SnapshotChunkPageRowView> {
    if !(1..=256).contains(&input.max_chunks) {
        return Err(conflict(PAGE_SIZE_INVALID));
    }
    let mut tx = pool.begin().await?;
    validate_write_fence_on(&mut tx, &input.fence).await?;
    let census_ok:bool=sqlx::query_scalar(r#"SELECT EXISTS(SELECT 1 FROM candidate_analysis_input_chunk_censuses
        WHERE chunk_census_id=$1 AND snapshot_input_id=$2 AND snapshot_id=$3 AND census_hash=$4
          AND source_byte_count=$5 AND chunking_contract_version=$6 AND redaction_contract_version=$7)"#)
        .bind(input.chunk_census_id).bind(input.input_id).bind(input.fence.snapshot_id)
        .bind(&input.chunk_census_hash).bind(input.source_size_bytes).bind(&input.chunking_contract_version)
        .bind(&input.redaction_contract_version).fetch_one(&mut *tx).await?;
    if !census_ok {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let chunks = sqlx::query_as::<_, (Uuid, i32, i64, i64, String, String, Option<Value>)>(
        r#"SELECT chunk_id,ordinal,
        source_range_start,source_range_end,chunk_hash,body_or_blob_hash,immutable_redacted_body
        FROM candidate_analysis_input_chunk_census_members WHERE chunk_census_id=$1 AND ordinal>=$2
        ORDER BY ordinal LIMIT $3"#,
    )
    .bind(input.chunk_census_id)
    .bind(input.first_chunk_ordinal)
    .bind(input.max_chunks as i64)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|row| SnapshotChunkRowView {
        chunk_id: row.0,
        chunk_ordinal: row.1,
        source_range_start: row.2,
        source_range_end: row.3,
        chunk_hash: row.4,
        body_hash: row.5,
        body: row.6.unwrap_or_else(|| json!({"blob_only":true})),
    })
    .collect::<Vec<_>>();
    let first = chunks.first().map(|v| v.chunk_ordinal);
    let last = chunks.last().map(|v| v.chunk_ordinal);
    let page_hash=hash_json_on(&mut tx,&json!({"census":input.chunk_census_hash,
        "members":chunks.iter().map(|v|(&v.chunk_hash,v.source_range_start,v.source_range_end)).collect::<Vec<_>>() })).await?;
    let receipt_id = Uuid::new_v5(&input.stable_page_request_id, b"candidate_page_receipt.v1");
    let cursor = format!("chunk:{}:{}", input.first_chunk_ordinal, input.max_chunks);
    let existing = sqlx::query_as::<_, (String, String)>(
        "SELECT page_hash,server_cursor FROM candidate_analysis_page_receipts WHERE page_receipt_id=$1",
    )
    .bind(receipt_id)
    .fetch_optional(&mut *tx)
    .await?;
    let replayed = existing.is_some();
    if let Some((existing_hash, existing_cursor)) = existing {
        if existing_hash != page_hash || existing_cursor != cursor {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
    } else {
        sqlx::query(
            r#"INSERT INTO candidate_analysis_page_receipts(page_receipt_id,
        analysis_attempt_id,snapshot_id,page_kind,stable_request_id,snapshot_input_id,
        chunk_census_id,chunk_census_hash,source_size_bytes,chunking_contract_version,
        redaction_contract_version,consumer_worker_run_id,server_cursor,first_key,last_key,
        returned_count,page_hash)VALUES($1,$2,$3,'chunk_page',$4,$5,$6,$7,$8,$9,$10,
        $11,$12,$13,$14,$15,$16)"#,
        )
        .bind(receipt_id)
        .bind(input.fence.analysis_attempt_id)
        .bind(input.fence.snapshot_id)
        .bind(input.stable_page_request_id)
        .bind(input.input_id)
        .bind(input.chunk_census_id)
        .bind(&input.chunk_census_hash)
        .bind(input.source_size_bytes)
        .bind(&input.chunking_contract_version)
        .bind(&input.redaction_contract_version)
        .bind(input.fence.worker_run_id)
        .bind(cursor)
        .bind(first.map(|v| v.to_string()))
        .bind(last.map(|v| v.to_string()))
        .bind(chunks.len() as i64)
        .bind(&page_hash)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(SnapshotChunkPageRowView {
        snapshot_id: input.fence.snapshot_id,
        input_id: input.input_id,
        chunk_census_id: input.chunk_census_id,
        chunk_census_hash: input.chunk_census_hash,
        source_size_bytes: input.source_size_bytes,
        chunking_contract_version: input.chunking_contract_version,
        redaction_contract_version: input.redaction_contract_version,
        page_receipt_id: receipt_id,
        first_chunk_ordinal: first,
        last_chunk_ordinal: last,
        returned_count: chunks.len() as i64,
        page_hash,
        chunks,
        next_chunk_ordinal: last.map(|v| v + 1),
        replayed,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnalysisArtifactBodyRow {
    HypothesisProposal {
        proposal_id: Uuid,
        subject_kind: String,
        subject_identity_hash: String,
        predicate: PredicateIdentity,
        trust_boundary: String,
        polarity: ClaimPolarity,
        prose: String,
        confidence: i32,
        priority: i32,
        tags: Vec<String>,
        evidence_refs: Vec<String>,
    },
    ProposalConflictReview {
        conflict_component_id: Uuid,
        proposal_ids: Vec<Uuid>,
        outcome: String,
        rationale: String,
    },
    ControllerDecision {
        decision_id: Uuid,
        proposal_id: Uuid,
        decision: String,
        related_proposal_ids: Vec<Uuid>,
        rationale: String,
    },
}
impl AnalysisArtifactBodyRow {
    fn kind(&self) -> &'static str {
        match self {
            Self::HypothesisProposal { .. } => "hypothesis_proposal.v1",
            Self::ProposalConflictReview { .. } => "proposal_conflict_review.v1",
            Self::ControllerDecision { .. } => "controller_decision.v1",
        }
    }
}
#[derive(Debug, Clone)]
pub struct RecordAnalysisArtifactInput {
    pub fence: CandidateWriteFenceRow,
    pub stable_artifact_request_id: Uuid,
    pub artifact: AnalysisArtifactBodyRow,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisArtifactReceiptRow {
    pub artifact_id: Uuid,
    pub artifact_kind: String,
    pub artifact_hash: String,
    pub artifact_row_version: i64,
    pub replayed: bool,
}

pub async fn record_analysis_artifact(
    pool: &PgPool,
    input: RecordAnalysisArtifactInput,
) -> Result<AnalysisArtifactReceiptRow> {
    let mut tx = pool.begin().await?;
    validate_write_fence_on(&mut tx, &input.fence).await?;
    let kind = input.artifact.kind();
    if matches!(
        kind,
        "hypothesis_coverage_subreview.v1"
            | "hypothesis_coverage_synthesis.v1"
            | "hypothesis_coverage_review.v1"
    ) {
        return Err(conflict(ARTIFACT_KIND_FORBIDDEN));
    }
    let proposal_id = match &input.artifact {
        AnalysisArtifactBodyRow::HypothesisProposal { proposal_id, .. } => Some(*proposal_id),
        _ => None,
    };
    let body = serde_json::to_value(&input.artifact)?;
    let hash = hash_json_on(&mut tx, &body).await?;
    let id = Uuid::new_v5(&input.stable_artifact_request_id, kind.as_bytes());
    let existing = sqlx::query_as::<_, (String, String)>(
        "SELECT artifact_kind,artifact_hash FROM candidate_analysis_artifacts WHERE artifact_id=$1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let replayed = existing.is_some();
    if let Some((existing_kind, existing_hash)) = existing {
        if existing_kind != kind || existing_hash != hash {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
    } else {
        let candidate_item_id = candidate_item_id_on(
            &mut tx,
            input.fence.analysis_attempt_id,
            input.fence.work_item_id,
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO candidate_analysis_artifacts(artifact_id,analysis_attempt_id,
        candidate_work_item_id,worker_run_id,artifact_kind,artifact_body,artifact_hash)
        VALUES($1,$2,$3,$4,$5,$6,$7)"#,
        )
        .bind(id)
        .bind(input.fence.analysis_attempt_id)
        .bind(candidate_item_id)
        .bind(input.fence.worker_run_id)
        .bind(kind)
        .bind(&body)
        .bind(&hash)
        .execute(&mut *tx)
        .await?;
        if let Some(proposal_id) = proposal_id {
            let proposal_ordinal: i32 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(proposal_ordinal)+1,0)::INTEGER FROM hypothesis_proposals WHERE analysis_attempt_id=$1",
            ).bind(input.fence.analysis_attempt_id).fetch_one(&mut *tx).await?;
            sqlx::query(
                r#"INSERT INTO hypothesis_proposals(
                    proposal_id,analysis_attempt_id,artifact_id,proposal_ordinal,
                    structured_proposal,proposal_hash)
                VALUES($1,$2,$3,$4,$5,$6)"#,
            )
            .bind(proposal_id)
            .bind(input.fence.analysis_attempt_id)
            .bind(id)
            .bind(proposal_ordinal)
            .bind(&body)
            .bind(&hash)
            .execute(&mut *tx)
            .await?;
        }
    }
    if let Some(proposal_id) = proposal_id {
        let persisted: Option<(Uuid, String)> = sqlx::query_as(
            "SELECT artifact_id,proposal_hash FROM hypothesis_proposals WHERE proposal_id=$1 AND analysis_attempt_id=$2",
        ).bind(proposal_id).bind(input.fence.analysis_attempt_id).fetch_optional(&mut *tx).await?;
        if persisted != Some((id, hash.clone())) {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
    }
    tx.commit().await?;
    Ok(AnalysisArtifactReceiptRow {
        artifact_id: id,
        artifact_kind: kind.into(),
        artifact_hash: hash,
        artifact_row_version: 0,
        replayed,
    })
}

#[derive(Debug, Clone)]
pub struct SealCandidateCompilationInput {
    pub fence: CandidateWriteFenceRow,
    pub stable_compilation_request_id: Uuid,
    pub mutation_set_hash: String,
    pub claim_component_set_hash: String,
    pub verification_contract_set_hash: String,
    pub verification_plan_set_hash: String,
    pub generation_transition_set_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateCompilationSealRowView {
    pub compilation_seal_id: Uuid,
    pub mutation_set_hash: String,
    pub claim_component_set_hash: String,
    pub verification_contract_set_hash: String,
    pub verification_plan_set_hash: String,
    pub generation_transition_set_hash: String,
    pub compiler_seal_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct PersistedCandidateCompilationSeal {
    compilation_seal_id: Uuid,
    stable_compilation_request_id: Uuid,
    mutation_set_hash: String,
    claim_component_set_hash: String,
    verification_contract_set_hash: String,
    verification_plan_set_hash: String,
    generation_transition_set_hash: String,
    compiler_seal_hash: String,
}

pub async fn seal_candidate_compilation(
    pool: &PgPool,
    input: SealCandidateCompilationInput,
) -> Result<CandidateCompilationSealRowView> {
    let mut tx = pool.begin().await?;
    validate_write_fence_on(&mut tx, &input.fence).await?;
    validate_final_submitter_fence_on(&mut tx, &input.fence).await?;
    let compilation_seal_id = Uuid::new_v5(
        &input.fence.analysis_attempt_id,
        b"candidate_host_compilation_seal.v1",
    );
    let compiler_seal_hash = hash_json_on(
        &mut tx,
        &json!({
            "domain":"candidate_host_compilation_seal.v1",
            "analysis_attempt_id":input.fence.analysis_attempt_id,
            "snapshot_id":input.fence.snapshot_id,
            "operation_id":input.fence.operation_id,
            "organization_id":input.fence.organization_id,
            "final_submitter_worker_run_id":input.fence.worker_run_id,
            "mutation_set_hash":input.mutation_set_hash,
            "claim_component_set_hash":input.claim_component_set_hash,
            "verification_contract_set_hash":input.verification_contract_set_hash,
            "verification_plan_set_hash":input.verification_plan_set_hash,
            "generation_transition_set_hash":input.generation_transition_set_hash,
        }),
    )
    .await?;
    let existing: Option<PersistedCandidateCompilationSeal> = sqlx::query_as(
        r#"SELECT compilation_seal_id,stable_compilation_request_id,
                      mutation_set_hash,claim_component_set_hash,
                      verification_contract_set_hash,verification_plan_set_hash,
                      generation_transition_set_hash,compiler_seal_hash
                 FROM candidate_analysis_host_compilation_seals
                WHERE analysis_attempt_id=$1"#,
    )
    .bind(input.fence.analysis_attempt_id)
    .fetch_optional(&mut *tx)
    .await?;
    let replayed = existing.is_some();
    if let Some(persisted) = existing {
        if persisted
            != (PersistedCandidateCompilationSeal {
                compilation_seal_id,
                stable_compilation_request_id: input.stable_compilation_request_id,
                mutation_set_hash: input.mutation_set_hash.clone(),
                claim_component_set_hash: input.claim_component_set_hash.clone(),
                verification_contract_set_hash: input.verification_contract_set_hash.clone(),
                verification_plan_set_hash: input.verification_plan_set_hash.clone(),
                generation_transition_set_hash: input.generation_transition_set_hash.clone(),
                compiler_seal_hash: compiler_seal_hash.clone(),
            })
        {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
    } else {
        sqlx::query(
            r#"INSERT INTO candidate_analysis_host_compilation_seals(
                   compilation_seal_id,stable_compilation_request_id,
                   analysis_attempt_id,snapshot_id,operation_id,organization_id,
                   final_submitter_worker_run_id,mutation_set_hash,
                   claim_component_set_hash,verification_contract_set_hash,
                   verification_plan_set_hash,generation_transition_set_hash,
                   compiler_seal_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#,
        )
        .bind(compilation_seal_id)
        .bind(input.stable_compilation_request_id)
        .bind(input.fence.analysis_attempt_id)
        .bind(input.fence.snapshot_id)
        .bind(input.fence.operation_id)
        .bind(input.fence.organization_id)
        .bind(input.fence.worker_run_id)
        .bind(&input.mutation_set_hash)
        .bind(&input.claim_component_set_hash)
        .bind(&input.verification_contract_set_hash)
        .bind(&input.verification_plan_set_hash)
        .bind(&input.generation_transition_set_hash)
        .bind(&compiler_seal_hash)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(CandidateCompilationSealRowView {
        compilation_seal_id,
        mutation_set_hash: input.mutation_set_hash,
        claim_component_set_hash: input.claim_component_set_hash,
        verification_contract_set_hash: input.verification_contract_set_hash,
        verification_plan_set_hash: input.verification_plan_set_hash,
        generation_transition_set_hash: input.generation_transition_set_hash,
        compiler_seal_hash,
        row_version: 0,
        replayed,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisCensusKindRow {
    Proposal,
    Critic,
}
#[derive(Debug, Clone)]
pub struct SealAnalysisCensusInput {
    pub fence: CandidateWriteFenceRow,
    pub stable_census_request_id: Uuid,
    pub census_kind: AnalysisCensusKindRow,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisCensusRowView {
    pub census_id: Uuid,
    pub analysis_attempt_id: Uuid,
    pub census_kind: AnalysisCensusKindRow,
    pub member_count: i64,
    pub member_set_hash: String,
    pub census_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct CriticCensusSourceDbRow {
    member_kind: String,
    source_identity: Uuid,
    source_hash: String,
}

pub async fn seal_analysis_census(
    pool: &PgPool,
    input: SealAnalysisCensusInput,
) -> Result<AnalysisCensusRowView> {
    let mut tx = pool.begin().await?;
    validate_write_fence_on(&mut tx, &input.fence).await?;
    let member_kind = match input.census_kind {
        AnalysisCensusKindRow::Proposal => "proposal",
        AnalysisCensusKindRow::Critic => "critic",
    };
    let id = Uuid::new_v5(&input.stable_census_request_id, member_kind.as_bytes());
    let (member_count, set_hash, census_hash, replayed) = match input.census_kind {
        AnalysisCensusKindRow::Proposal => {
            let rows: Vec<(Uuid, String)> = sqlx::query_as(
                r#"SELECT proposal_id,proposal_hash
                FROM hypothesis_proposals WHERE analysis_attempt_id=$1 ORDER BY proposal_ordinal"#,
            )
            .bind(input.fence.analysis_attempt_id)
            .fetch_all(&mut *tx)
            .await?;
            let hashes = rows.iter().map(|row| row.1.clone()).collect::<Vec<_>>();
            let set_hash = hash_text_array_on(&mut tx, &hashes).await?;
            let census_hash = hash_json_on(
                &mut tx,
                &json!({"kind":"proposal",
                "attempt":input.fence.analysis_attempt_id,"count":rows.len(),"set":set_hash}),
            )
            .await?;
            let existing:Option<(Uuid,i64,String,String)>=sqlx::query_as(r#"SELECT proposal_census_id,
                proposal_count,proposal_set_hash,census_hash FROM candidate_analysis_proposal_censuses
                WHERE analysis_attempt_id=$1"#).bind(input.fence.analysis_attempt_id)
                .fetch_optional(&mut *tx).await?;
            let replayed = existing.is_some();
            if let Some((existing_id, count, existing_set, existing_hash)) = existing {
                if existing_id != id
                    || count != rows.len() as i64
                    || existing_set != set_hash
                    || existing_hash != census_hash
                {
                    return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
                }
            } else {
                sqlx::query(
                    r#"INSERT INTO candidate_analysis_proposal_censuses(
                proposal_census_id,analysis_attempt_id,proposal_count,proposal_set_hash,census_hash)
                VALUES($1,$2,$3,$4,$5)"#,
                )
                .bind(id)
                .bind(input.fence.analysis_attempt_id)
                .bind(rows.len() as i64)
                .bind(&set_hash)
                .bind(&census_hash)
                .execute(&mut *tx)
                .await?;
                for (ordinal, (source_id, source_hash)) in rows.iter().enumerate() {
                    let member_hash = hash_json_on(
                        &mut tx,
                        &json!({"proposal_id":source_id,"proposal_hash":source_hash}),
                    )
                    .await?;
                    sqlx::query(
                        r#"INSERT INTO candidate_analysis_proposal_census_members(
                        census_member_id,proposal_census_id,analysis_attempt_id,proposal_id,ordinal,
                        proposal_hash,member_hash)VALUES($1,$2,$3,$4,$5,$6,$7)"#,
                    )
                    .bind(Uuid::new_v5(&id, member_hash.as_bytes()))
                    .bind(id)
                    .bind(input.fence.analysis_attempt_id)
                    .bind(source_id)
                    .bind(ordinal as i32)
                    .bind(source_hash)
                    .bind(member_hash)
                    .execute(&mut *tx)
                    .await?;
                }
            }
            (rows.len() as i64, set_hash, census_hash, replayed)
        }
        AnalysisCensusKindRow::Critic => {
            let closure:(i64,i64,i64,i64,i64,i64,i64)=sqlx::query_as(r#"SELECT
                (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_subreview_census_members WHERE analysis_attempt_id=$1 AND disposition='required'),
                (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_subreviews WHERE analysis_attempt_id=$1),
                (SELECT COALESCE(sum(node_count),0) FROM candidate_analysis_hypothesis_coverage_synthesis_censuses WHERE analysis_attempt_id=$1),
                (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_synthesis_reviews WHERE analysis_attempt_id=$1),
                (SELECT count(*) FROM candidate_analysis_snapshot_inputs source JOIN candidate_analysis_attempts attempt ON attempt.snapshot_id=source.snapshot_id WHERE attempt.analysis_attempt_id=$1),
                (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_reviews WHERE analysis_attempt_id=$1),
                (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_global_reviews WHERE analysis_attempt_id=$1)"#)
                .bind(input.fence.analysis_attempt_id).fetch_one(&mut *tx).await?;
            if closure.0 == 0
                || closure.0 != closure.1
                || closure.2 == 0
                || closure.2 != closure.3
                || closure.4 == 0
                || closure.4 != closure.5
                || closure.6 != 1
            {
                return Err(conflict(CENSUS_NOT_CLOSED));
            }
            let sources=sqlx::query_as::<_,CriticCensusSourceDbRow>(r#"SELECT member_kind,source_identity,source_hash FROM(
                SELECT 'proposal_conflict_component'::TEXT AS member_kind,conflict_component_id AS source_identity,component_hash AS source_hash
                  FROM candidate_analysis_conflict_components WHERE analysis_attempt_id=$1
                UNION ALL SELECT 'hypothesis_coverage_subreview',subreview_id,subreview_hash
                  FROM candidate_analysis_hypothesis_coverage_subreviews WHERE analysis_attempt_id=$1
                UNION ALL SELECT 'hypothesis_coverage_synthesis',synthesis_review_id,review_hash
                  FROM candidate_analysis_hypothesis_coverage_synthesis_reviews WHERE analysis_attempt_id=$1
                UNION ALL SELECT 'hypothesis_coverage_input_review',coverage_review_id,review_hash
                  FROM candidate_analysis_hypothesis_coverage_reviews WHERE analysis_attempt_id=$1
                UNION ALL SELECT 'hypothesis_coverage_global_review',global_review_id,review_hash
                  FROM candidate_analysis_hypothesis_coverage_global_reviews WHERE analysis_attempt_id=$1
            ) source ORDER BY member_kind,source_identity"#).bind(input.fence.analysis_attempt_id)
                .fetch_all(&mut *tx).await?;
            let mut member_hashes = Vec::with_capacity(sources.len());
            for source in &sources {
                member_hashes.push(
                    hash_json_on(
                        &mut tx,
                        &json!({"member_kind":source.member_kind,
                "source_identity":source.source_identity,"source_hash":source.source_hash}),
                    )
                    .await?,
                );
            }
            let set_hash = hash_text_array_on(&mut tx, &member_hashes).await?;
            let census_hash = hash_json_on(
                &mut tx,
                &json!({"kind":"critic","attempt":input.fence.analysis_attempt_id,
                "count":sources.len(),"set":set_hash}),
            )
            .await?;
            let existing: Option<(Uuid, i64, String, String)> = sqlx::query_as(
                r#"SELECT critic_census_id,
                member_count,member_set_hash,census_hash FROM candidate_analysis_critic_censuses
                WHERE analysis_attempt_id=$1"#,
            )
            .bind(input.fence.analysis_attempt_id)
            .fetch_optional(&mut *tx)
            .await?;
            let replayed = existing.is_some();
            if let Some((existing_id, count, existing_set, existing_hash)) = existing {
                if existing_id != id
                    || count != sources.len() as i64
                    || existing_set != set_hash
                    || existing_hash != census_hash
                {
                    return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
                }
            } else {
                sqlx::query(
                    r#"INSERT INTO candidate_analysis_critic_censuses(
                critic_census_id,analysis_attempt_id,member_count,member_set_hash,census_hash)
                VALUES($1,$2,$3,$4,$5)"#,
                )
                .bind(id)
                .bind(input.fence.analysis_attempt_id)
                .bind(sources.len() as i64)
                .bind(&set_hash)
                .bind(&census_hash)
                .execute(&mut *tx)
                .await?;
                for (ordinal, (source, member_hash)) in
                    sources.iter().zip(member_hashes.iter()).enumerate()
                {
                    sqlx::query(
                        r#"INSERT INTO candidate_analysis_critic_census_members(
                        critic_member_id,critic_census_id,analysis_attempt_id,ordinal,member_kind,
                        source_identity,source_hash,member_hash)VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
                    )
                    .bind(Uuid::new_v5(&id, member_hash.as_bytes()))
                    .bind(id)
                    .bind(input.fence.analysis_attempt_id)
                    .bind(ordinal as i32)
                    .bind(&source.member_kind)
                    .bind(source.source_identity)
                    .bind(&source.source_hash)
                    .bind(member_hash)
                    .execute(&mut *tx)
                    .await?;
                }
            }
            (sources.len() as i64, set_hash, census_hash, replayed)
        }
    };
    tx.commit().await?;
    Ok(AnalysisCensusRowView {
        census_id: id,
        analysis_attempt_id: input.fence.analysis_attempt_id,
        census_kind: input.census_kind,
        member_count,
        member_set_hash: set_hash,
        census_hash,
        row_version: 0,
        replayed,
    })
}

#[derive(Debug, Clone)]
pub struct SealCoverageSubreviewCensusInput {
    pub fence: CandidateWriteFenceRow,
    pub stable_census_request_id: Uuid,
    pub input_id: Uuid,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageSubreviewCensusRowView {
    pub census_id: Uuid,
    pub analysis_attempt_id: Uuid,
    pub input_id: Uuid,
    pub member_count: i64,
    pub member_set_hash: String,
    pub census_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct CoverageChecklistDbRow {
    checklist_member_id: Uuid,
    ordinal: i32,
    member_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
struct CoveragePartitionDbRow {
    chunk_partition_id: Uuid,
    partition_ordinal: i32,
    chunk_set_hash: String,
}

#[derive(Debug)]
struct CoverageSubreviewMemberDraft {
    member_id: Uuid,
    checklist_member_id: Uuid,
    partition_id: Uuid,
    checklist_ordinal: i32,
    partition_ordinal: i32,
    designated_work_item_id: Uuid,
    member_hash: String,
}

async fn candidate_item_id_on(
    tx: &mut Transaction<'_, Postgres>,
    attempt_id: Uuid,
    stage_work_item_id: Uuid,
) -> Result<Uuid> {
    sqlx::query_scalar(
        r#"SELECT candidate_work_item_id FROM candidate_analysis_work_items
            WHERE analysis_attempt_id=$1 AND stage_work_item_id=$2"#,
    )
    .bind(attempt_id)
    .bind(stage_work_item_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(WRITE_FENCE_MISMATCH))
}

pub async fn seal_hypothesis_coverage_subreview_census(
    pool: &PgPool,
    input: SealCoverageSubreviewCensusInput,
) -> Result<CoverageSubreviewCensusRowView> {
    let mut tx = pool.begin().await?;
    validate_write_fence_on(&mut tx, &input.fence).await?;
    let input_owned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM candidate_analysis_snapshot_inputs WHERE snapshot_input_id=$1 AND snapshot_id=$2)",
    )
    .bind(input.input_id)
    .bind(input.fence.snapshot_id)
    .fetch_one(&mut *tx)
    .await?;
    if !input_owned {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let checklist = sqlx::query_as::<_, CoverageChecklistDbRow>(
        r#"SELECT checklist_member_id,ordinal,member_hash
              FROM candidate_analysis_hypothesis_coverage_checklist_members
             WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2
             ORDER BY ordinal,checklist_member_id"#,
    )
    .bind(input.fence.analysis_attempt_id)
    .bind(input.input_id)
    .fetch_all(&mut *tx)
    .await?;
    let partitions = sqlx::query_as::<_, CoveragePartitionDbRow>(
        r#"SELECT chunk_partition_id,partition_ordinal,chunk_set_hash
              FROM candidate_analysis_hypothesis_coverage_chunk_partitions
             WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2
             ORDER BY partition_ordinal,chunk_partition_id"#,
    )
    .bind(input.fence.analysis_attempt_id)
    .bind(input.input_id)
    .fetch_all(&mut *tx)
    .await?;
    if checklist.is_empty() || partitions.is_empty() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let checklist_hashes = checklist
        .iter()
        .map(|row| row.member_hash.clone())
        .collect::<Vec<_>>();
    let partition_hashes = partitions
        .iter()
        .map(|row| row.chunk_set_hash.clone())
        .collect::<Vec<_>>();
    let checklist_set_hash = hash_text_array_on(&mut tx, &checklist_hashes).await?;
    let partition_set_hash = hash_text_array_on(&mut tx, &partition_hashes).await?;
    let census_id = Uuid::new_v5(&input.stable_census_request_id, input.input_id.as_bytes());
    let mut members = Vec::with_capacity(checklist.len().saturating_mul(partitions.len()));
    for checklist_member in &checklist {
        for partition in &partitions {
            let designated_work_item_id: Uuid = sqlx::query_scalar(
                r#"SELECT item.stage_work_item_id
                      FROM candidate_analysis_work_items item
                     WHERE item.analysis_attempt_id=$1 AND item.phase='critic'
                       AND item.capability IN ('hypothesis_coverage_subreview','coverage_subreview')
                       AND item.component_id=$2 AND item.microbatch_key=$3
                     ORDER BY item.stage_work_item_id LIMIT 1"#,
            )
            .bind(input.fence.analysis_attempt_id)
            .bind(checklist_member.checklist_member_id)
            .bind(partition.chunk_partition_id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
            let member_hash = hash_json_on(
                &mut tx,
                &json!({
                    "domain":"candidate_hypothesis_coverage_subreview_census_member.v1",
                    "analysis_attempt_id":input.fence.analysis_attempt_id,
                    "snapshot_input_id":input.input_id,
                    "checklist_member_id":checklist_member.checklist_member_id,
                    "checklist_ordinal":checklist_member.ordinal,
                    "checklist_member_hash":checklist_member.member_hash,
                    "chunk_partition_id":partition.chunk_partition_id,
                    "partition_ordinal":partition.partition_ordinal,
                    "chunk_partition_hash":partition.chunk_set_hash,
                    "designated_stage_work_item_id":designated_work_item_id,
                    "disposition":"required",
                }),
            )
            .await?;
            members.push(CoverageSubreviewMemberDraft {
                member_id: Uuid::new_v5(&census_id, member_hash.as_bytes()),
                checklist_member_id: checklist_member.checklist_member_id,
                partition_id: partition.chunk_partition_id,
                checklist_ordinal: checklist_member.ordinal,
                partition_ordinal: partition.partition_ordinal,
                designated_work_item_id,
                member_hash,
            });
        }
    }
    let member_hashes = members
        .iter()
        .map(|row| row.member_hash.clone())
        .collect::<Vec<_>>();
    let member_set_hash = hash_text_array_on(&mut tx, &member_hashes).await?;
    let member_count = i64::try_from(members.len()).map_err(|_| conflict(CENSUS_NOT_CLOSED))?;
    let census_hash = hash_json_on(
        &mut tx,
        &json!({
            "domain":"candidate_hypothesis_coverage_subreview_census.v1",
            "analysis_attempt_id":input.fence.analysis_attempt_id,
            "snapshot_input_id":input.input_id,
            "checklist_member_count":checklist.len(),
            "checklist_member_set_hash":checklist_set_hash,
            "chunk_partition_count":partitions.len(),
            "chunk_partition_set_hash":partition_set_hash,
            "expected_member_count":member_count,
            "member_set_hash":member_set_hash,
        }),
    )
    .await?;
    let existing: Option<(Uuid, i64, String, String)> = sqlx::query_as(
        r#"SELECT subreview_census_id,expected_member_count,member_set_hash,census_hash
              FROM candidate_analysis_hypothesis_coverage_subreview_censuses
             WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2"#,
    )
    .bind(input.fence.analysis_attempt_id)
    .bind(input.input_id)
    .fetch_optional(&mut *tx)
    .await?;
    let replayed = existing.is_some();
    if let Some((existing_id, existing_count, existing_set_hash, existing_hash)) = existing {
        if existing_id != census_id
            || existing_count != member_count
            || existing_set_hash != member_set_hash
            || existing_hash != census_hash
        {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
    } else {
        sqlx::query(
            r#"INSERT INTO candidate_analysis_hypothesis_coverage_subreview_censuses(
                subreview_census_id,analysis_attempt_id,snapshot_input_id,
                checklist_member_count,checklist_member_set_hash,chunk_partition_count,
                chunk_partition_set_hash,expected_member_count,member_set_hash,census_hash)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
        )
        .bind(census_id)
        .bind(input.fence.analysis_attempt_id)
        .bind(input.input_id)
        .bind(checklist.len() as i64)
        .bind(&checklist_set_hash)
        .bind(partitions.len() as i64)
        .bind(&partition_set_hash)
        .bind(member_count)
        .bind(&member_set_hash)
        .bind(&census_hash)
        .execute(&mut *tx)
        .await?;
        for member in members {
            sqlx::query(
                r#"INSERT INTO candidate_analysis_hypothesis_coverage_subreview_census_members(
                    subreview_census_member_id,subreview_census_id,analysis_attempt_id,
                    snapshot_input_id,checklist_member_id,chunk_partition_id,
                    checklist_ordinal,partition_ordinal,designated_stage_work_item_id,
                    disposition,member_hash)
                VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'required',$10)"#,
            )
            .bind(member.member_id)
            .bind(census_id)
            .bind(input.fence.analysis_attempt_id)
            .bind(input.input_id)
            .bind(member.checklist_member_id)
            .bind(member.partition_id)
            .bind(member.checklist_ordinal)
            .bind(member.partition_ordinal)
            .bind(member.designated_work_item_id)
            .bind(member.member_hash)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(CoverageSubreviewCensusRowView {
        census_id,
        analysis_attempt_id: input.fence.analysis_attempt_id,
        input_id: input.input_id,
        member_count,
        member_set_hash,
        census_hash,
        row_version: 0,
        replayed,
    })
}
#[derive(Debug, Clone)]
pub struct RecordCoverageSubreviewInput {
    pub fence: CandidateWriteFenceRow,
    pub stable_review_request_id: Uuid,
    pub subreview_census_id: Uuid,
    pub subreview_census_member_id: Uuid,
    pub outcome: String,
    pub missed_proposal_ids: Vec<Uuid>,
    pub blocker_reason: Option<String>,
    pub review_notes: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageSubreviewReceiptRow {
    pub subreview_id: Uuid,
    pub subreview_census_id: Uuid,
    pub subreview_census_member_id: Uuid,
    pub subreview_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct CoverageSubreviewAuthorityDbRow {
    snapshot_input_id: Uuid,
    designated_stage_work_item_id: Uuid,
    disposition: String,
    chunk_partition_id: Uuid,
    chunk_count: i64,
    chunk_set_hash: String,
    first_chunk_ordinal: i32,
    last_chunk_ordinal: i32,
    bounded_context_budget: i64,
}

pub async fn record_hypothesis_coverage_subreview(
    pool: &PgPool,
    mut input: RecordCoverageSubreviewInput,
) -> Result<CoverageSubreviewReceiptRow> {
    let mut tx = pool.begin().await?;
    validate_write_fence_on(&mut tx, &input.fence).await?;
    let authority = sqlx::query_as::<_, CoverageSubreviewAuthorityDbRow>(
        r#"SELECT member.snapshot_input_id,member.designated_stage_work_item_id,
                  member.disposition,member.chunk_partition_id,partition.chunk_count,
                  partition.chunk_set_hash,partition.first_chunk_ordinal,
                  partition.last_chunk_ordinal,partition.bounded_context_budget
              FROM candidate_analysis_hypothesis_coverage_subreview_census_members member
              JOIN candidate_analysis_hypothesis_coverage_chunk_partitions partition
                ON partition.chunk_partition_id=member.chunk_partition_id
             WHERE member.subreview_census_id=$1 AND member.subreview_census_member_id=$2
               AND member.analysis_attempt_id=$3"#,
    )
    .bind(input.subreview_census_id)
    .bind(input.subreview_census_member_id)
    .bind(input.fence.analysis_attempt_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
    if authority.designated_stage_work_item_id != input.fence.work_item_id
        || authority.disposition != "required"
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let chunk_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT member.chunk_hash
              FROM candidate_analysis_input_chunk_census_members member
              JOIN candidate_analysis_hypothesis_coverage_chunk_partitions partition
                ON partition.snapshot_input_id=member.snapshot_input_id
               AND member.ordinal BETWEEN partition.first_chunk_ordinal AND partition.last_chunk_ordinal
             WHERE partition.chunk_partition_id=$1 ORDER BY member.ordinal"#,
    ).bind(authority.chunk_partition_id).fetch_all(&mut *tx).await?;
    if chunk_hashes.len() as i64 != authority.chunk_count
        || hash_text_array_on(&mut tx, &chunk_hashes).await? != authority.chunk_set_hash
    {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let read_receipts: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        r#"SELECT page_hash,first_key,last_key FROM candidate_analysis_page_receipts
            WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2 AND consumer_worker_run_id=$3
              AND page_kind='chunk_page'
              AND chunk_census_id=(
                  SELECT chunk_census_id FROM candidate_analysis_input_chunk_censuses
                   WHERE snapshot_input_id=$2)
            ORDER BY page_hash"#,
    )
    .bind(input.fence.analysis_attempt_id)
    .bind(authority.snapshot_input_id)
    .bind(input.fence.worker_run_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut covered_ordinals = std::collections::BTreeSet::new();
    for (_, first, last) in &read_receipts {
        let (Some(first), Some(last)) = (first, last) else {
            continue;
        };
        let first = first
            .parse::<i32>()
            .map_err(|_| conflict(CENSUS_NOT_CLOSED))?;
        let last = last
            .parse::<i32>()
            .map_err(|_| conflict(CENSUS_NOT_CLOSED))?;
        if first > last {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        covered_ordinals.extend(first..=last);
    }
    if (authority.first_chunk_ordinal..=authority.last_chunk_ordinal)
        .any(|ordinal| !covered_ordinals.contains(&ordinal))
    {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let read_hashes = read_receipts
        .into_iter()
        .map(|(hash, _, _)| hash)
        .collect::<Vec<_>>();
    let read_set_hash = hash_text_array_on(&mut tx, &read_hashes).await?;
    let proposal_ref_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT reference.ref_hash FROM hypothesis_proposal_refs reference
              JOIN hypothesis_proposals proposal ON proposal.proposal_id=reference.proposal_id
             WHERE proposal.analysis_attempt_id=$1 AND reference.snapshot_input_id=$2
             ORDER BY reference.ref_hash"#,
    )
    .bind(input.fence.analysis_attempt_id)
    .bind(authority.snapshot_input_id)
    .fetch_all(&mut *tx)
    .await?;
    let proposal_ref_set_hash = hash_text_array_on(&mut tx, &proposal_ref_hashes).await?;
    let primary_worker: Uuid = sqlx::query_scalar(
        r#"SELECT output.worker_run_id
              FROM candidate_analysis_work_items candidate_item
              JOIN stage_worker_outputs output ON output.work_item_id=candidate_item.stage_work_item_id
             WHERE candidate_item.analysis_attempt_id=$1 AND candidate_item.phase='proposal'
               AND candidate_item.microbatch_key=$2
             ORDER BY output.created_at,output.worker_run_id LIMIT 1"#,
    ).bind(input.fence.analysis_attempt_id).bind(authority.snapshot_input_id)
      .fetch_optional(&mut *tx).await?
      .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    if primary_worker == input.fence.worker_run_id {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let mismatched_proposal_workers: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM (
              SELECT DISTINCT artifact.worker_run_id
                FROM candidate_analysis_artifacts artifact
                JOIN hypothesis_proposals proposal ON proposal.artifact_id=artifact.artifact_id
                JOIN hypothesis_proposal_refs reference ON reference.proposal_id=proposal.proposal_id
               WHERE proposal.analysis_attempt_id=$1 AND reference.snapshot_input_id=$2
                 AND artifact.worker_run_id<>$3
            ) mismatched"#,
    ).bind(input.fence.analysis_attempt_id).bind(authority.snapshot_input_id).bind(primary_worker)
      .fetch_one(&mut *tx).await?;
    if mismatched_proposal_workers != 0 {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    input.missed_proposal_ids.sort_unstable();
    input.missed_proposal_ids.dedup();
    let valid_outcome = match input.outcome.as_str() {
        "no_local_miss" => input.missed_proposal_ids.is_empty() && input.blocker_reason.is_none(),
        "missed_hypothesis" => {
            !input.missed_proposal_ids.is_empty() && input.blocker_reason.is_none()
        }
        "blocked" => input
            .blocker_reason
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty()),
        _ => false,
    };
    if !valid_outcome {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let blocker_codes = input.blocker_reason.iter().cloned().collect::<Vec<_>>();
    let context_truncated = blocker_codes.iter().any(|code| code == "context_truncated");
    let body = json!({"kind":"hypothesis_coverage_subreview.v1",
        "subreview_census_id":input.subreview_census_id,
        "subreview_census_member_id":input.subreview_census_member_id,
        "outcome":input.outcome,"typed_missed_refs":input.missed_proposal_ids,
        "blocker_codes":blocker_codes,"review_notes":input.review_notes});
    let subreview_hash = hash_json_on(&mut tx, &json!({"body":body,
        "designated_chunk_set_hash":authority.chunk_set_hash,"read_receipt_set_hash":read_set_hash,
        "h1_proposal_ref_set_hash":proposal_ref_set_hash,"primary_worker":primary_worker,
        "map_critic_worker":input.fence.worker_run_id,"context_budget":authority.bounded_context_budget,
        "context_truncated":context_truncated})).await?;
    let subreview_id = Uuid::new_v5(
        &input.stable_review_request_id,
        input.subreview_census_member_id.as_bytes(),
    );
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT subreview_hash FROM candidate_analysis_hypothesis_coverage_subreviews WHERE subreview_census_member_id=$1",
    ).bind(input.subreview_census_member_id).fetch_optional(&mut *tx).await?;
    let replayed = existing.is_some();
    if let Some(existing_hash) = existing {
        if existing_hash != subreview_hash {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
    } else {
        let candidate_item_id = candidate_item_id_on(
            &mut tx,
            input.fence.analysis_attempt_id,
            input.fence.work_item_id,
        )
        .await?;
        let artifact_id = Uuid::new_v5(
            &input.stable_review_request_id,
            b"hypothesis_coverage_subreview.v1",
        );
        let artifact_hash = hash_json_on(&mut tx, &body).await?;
        sqlx::query(
            r#"INSERT INTO candidate_analysis_artifacts(artifact_id,analysis_attempt_id,
                candidate_work_item_id,worker_run_id,artifact_kind,artifact_body,artifact_hash)
            VALUES($1,$2,$3,$4,'hypothesis_coverage_subreview.v1',$5,$6)"#,
        )
        .bind(artifact_id)
        .bind(input.fence.analysis_attempt_id)
        .bind(candidate_item_id)
        .bind(input.fence.worker_run_id)
        .bind(&body)
        .bind(artifact_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(r#"INSERT INTO candidate_analysis_hypothesis_coverage_subreviews(
                subreview_id,subreview_census_member_id,subreview_census_id,analysis_attempt_id,
                snapshot_input_id,designated_chunk_count,designated_chunk_set_hash,
                read_receipt_count,read_receipt_set_hash,h1_proposal_ref_count,h1_proposal_ref_set_hash,
                primary_analyst_worker_run_id,map_critic_worker_run_id,context_budget,context_truncated,
                outcome,typed_missed_refs,blocker_codes,subreview_hash)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)"#)
            .bind(subreview_id).bind(input.subreview_census_member_id).bind(input.subreview_census_id)
            .bind(input.fence.analysis_attempt_id).bind(authority.snapshot_input_id)
            .bind(authority.chunk_count).bind(authority.chunk_set_hash).bind(read_hashes.len() as i64)
            .bind(read_set_hash).bind(proposal_ref_hashes.len() as i64).bind(proposal_ref_set_hash)
            .bind(primary_worker).bind(input.fence.worker_run_id).bind(authority.bounded_context_budget)
            .bind(context_truncated).bind(&input.outcome).bind(json!(input.missed_proposal_ids))
            .bind(blocker_codes).bind(&subreview_hash).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(CoverageSubreviewReceiptRow {
        subreview_id,
        subreview_census_id: input.subreview_census_id,
        subreview_census_member_id: input.subreview_census_member_id,
        subreview_hash,
        row_version: 0,
        replayed,
    })
}
#[derive(Debug, Clone)]
pub struct SealCoverageSynthesisCensusInput {
    pub fence: CandidateWriteFenceRow,
    pub stable_census_request_id: Uuid,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageSynthesisCensusRowView {
    pub census_id: Uuid,
    pub analysis_attempt_id: Uuid,
    pub member_count: i64,
    pub member_set_hash: String,
    pub census_hash: String,
    pub global_semantic_root_member_id: Uuid,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
struct SynthesisNodeDraft {
    node_id: Uuid,
    node_kind: &'static str,
    level: i32,
    partition_ordinal: i32,
    attack_class_id: Option<String>,
    trust_boundary_hash: Option<String>,
    covered_input_ids: Vec<Uuid>,
    covered_input_set_hash: String,
    covered_checklist_ids: Vec<Uuid>,
    covered_checklist_set_hash: String,
    child_hashes: Vec<String>,
    child_set_hash: String,
    descendant_workers: Vec<Uuid>,
    descendant_worker_set_hash: String,
    relationship_cross_index_hash: String,
    node_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
struct SynthesisLeafDbRow {
    snapshot_input_id: Uuid,
    checklist_member_id: Uuid,
    attack_class_id: String,
    trust_boundary_hash: String,
    subreview_hash: String,
    primary_analyst_worker_run_id: Uuid,
    map_critic_worker_run_id: Uuid,
}

type SynthesisLeafGroupKey = (String, String, Uuid, Uuid);
type SynthesisLeafGroupValue = (Vec<String>, Vec<Uuid>);

fn sort_dedup_uuids(values: &mut Vec<Uuid>) {
    values.sort_unstable();
    values.dedup();
}

async fn build_synthesis_node_on(
    tx: &mut Transaction<'_, Postgres>,
    census_id: Uuid,
    node_kind: &'static str,
    level: i32,
    partition_ordinal: i32,
    attack_class_id: Option<String>,
    trust_boundary_hash: Option<String>,
    mut covered_input_ids: Vec<Uuid>,
    mut covered_checklist_ids: Vec<Uuid>,
    mut child_hashes: Vec<String>,
    mut descendant_workers: Vec<Uuid>,
    relationship_cross_index_hash: &str,
) -> Result<SynthesisNodeDraft> {
    sort_dedup_uuids(&mut covered_input_ids);
    sort_dedup_uuids(&mut covered_checklist_ids);
    sort_dedup_uuids(&mut descendant_workers);
    child_hashes.sort();
    child_hashes.dedup();
    if covered_input_ids.is_empty()
        || covered_checklist_ids.is_empty()
        || child_hashes.is_empty()
        || descendant_workers.is_empty()
    {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let input_text = covered_input_ids
        .iter()
        .map(Uuid::to_string)
        .collect::<Vec<_>>();
    let checklist_text = covered_checklist_ids
        .iter()
        .map(Uuid::to_string)
        .collect::<Vec<_>>();
    let worker_text = descendant_workers
        .iter()
        .map(Uuid::to_string)
        .collect::<Vec<_>>();
    let covered_input_set_hash = hash_text_array_on(tx, &input_text).await?;
    let covered_checklist_set_hash = hash_text_array_on(tx, &checklist_text).await?;
    let child_set_hash = hash_text_array_on(tx, &child_hashes).await?;
    let descendant_worker_set_hash = hash_text_array_on(tx, &worker_text).await?;
    let node_hash = hash_json_on(tx,&json!({
        "domain":"candidate_hypothesis_coverage_synthesis_node.v1","node_kind":node_kind,
        "level":level,"partition_ordinal":partition_ordinal,"attack_class_id":attack_class_id,
        "trust_boundary_hash":trust_boundary_hash,"covered_input_set_hash":covered_input_set_hash,
        "covered_checklist_set_hash":covered_checklist_set_hash,"child_receipt_set_hash":child_set_hash,
        "relationship_cross_index_hash":relationship_cross_index_hash,
        "descendant_worker_set_hash":descendant_worker_set_hash,
    })).await?;
    Ok(SynthesisNodeDraft {
        node_id: Uuid::new_v5(&census_id, node_hash.as_bytes()),
        node_kind,
        level,
        partition_ordinal,
        attack_class_id,
        trust_boundary_hash,
        covered_input_ids,
        covered_input_set_hash,
        covered_checklist_ids,
        covered_checklist_set_hash,
        child_hashes,
        child_set_hash,
        descendant_workers,
        descendant_worker_set_hash,
        relationship_cross_index_hash: relationship_cross_index_hash.to_owned(),
        node_hash,
    })
}

async fn combine_synthesis_nodes_on(
    tx: &mut Transaction<'_, Postgres>,
    census_id: Uuid,
    node_kind: &'static str,
    level: i32,
    partition_ordinal: i32,
    attack_class_id: Option<String>,
    trust_boundary_hash: Option<String>,
    children: &[SynthesisNodeDraft],
    relationship_hash: &str,
) -> Result<SynthesisNodeDraft> {
    let mut inputs = Vec::new();
    let mut checklist = Vec::new();
    let mut child_hashes = Vec::new();
    let mut workers = Vec::new();
    for child in children {
        inputs.extend_from_slice(&child.covered_input_ids);
        checklist.extend_from_slice(&child.covered_checklist_ids);
        child_hashes.push(child.node_hash.clone());
        workers.extend_from_slice(&child.descendant_workers);
    }
    build_synthesis_node_on(
        tx,
        census_id,
        node_kind,
        level,
        partition_ordinal,
        attack_class_id,
        trust_boundary_hash,
        inputs,
        checklist,
        child_hashes,
        workers,
        relationship_hash,
    )
    .await
}

pub async fn seal_hypothesis_coverage_synthesis_census(
    pool: &PgPool,
    input: SealCoverageSynthesisCensusInput,
) -> Result<CoverageSynthesisCensusRowView> {
    const FAN_IN: usize = 8;
    let mut tx = pool.begin().await?;
    validate_write_fence_on(&mut tx, &input.fence).await?;
    let (expected_count,review_count):(i64,i64)=sqlx::query_as(r#"SELECT
        (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_subreview_census_members WHERE analysis_attempt_id=$1 AND disposition='required'),
        (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_subreviews WHERE analysis_attempt_id=$1)"#)
        .bind(input.fence.analysis_attempt_id).fetch_one(&mut *tx).await?;
    if expected_count == 0 || expected_count != review_count {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let relationship_hashes:Vec<String>=sqlx::query_scalar(
        "SELECT relation_hash FROM hypothesis_proposal_relations WHERE analysis_attempt_id=$1 ORDER BY relation_hash")
        .bind(input.fence.analysis_attempt_id).fetch_all(&mut *tx).await?;
    let relationship_hash = hash_text_array_on(&mut tx, &relationship_hashes).await?;
    let census_id = Uuid::new_v5(
        &input.stable_census_request_id,
        input.fence.analysis_attempt_id.as_bytes(),
    );
    let leaves = sqlx::query_as::<_, SynthesisLeafDbRow>(
        r#"SELECT member.snapshot_input_id,
        member.checklist_member_id,checklist.attack_class_id,checklist.trust_boundary_hash,
        review.subreview_hash,review.primary_analyst_worker_run_id,review.map_critic_worker_run_id
        FROM candidate_analysis_hypothesis_coverage_subreview_census_members member
        JOIN candidate_analysis_hypothesis_coverage_checklist_members checklist
          ON checklist.checklist_member_id=member.checklist_member_id
        JOIN candidate_analysis_hypothesis_coverage_subreviews review
          ON review.subreview_census_member_id=member.subreview_census_member_id
        WHERE member.analysis_attempt_id=$1 AND member.disposition='required'
        ORDER BY checklist.attack_class_id,checklist.trust_boundary_hash,
                 member.snapshot_input_id,member.checklist_ordinal,member.partition_ordinal"#,
    )
    .bind(input.fence.analysis_attempt_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut leaf_groups: BTreeMap<SynthesisLeafGroupKey, SynthesisLeafGroupValue> = BTreeMap::new();
    for leaf in leaves {
        let entry = leaf_groups
            .entry((
                leaf.attack_class_id,
                leaf.trust_boundary_hash,
                leaf.snapshot_input_id,
                leaf.checklist_member_id,
            ))
            .or_default();
        entry.0.push(leaf.subreview_hash);
        entry.1.extend([
            leaf.primary_analyst_worker_run_id,
            leaf.map_critic_worker_run_id,
        ]);
    }
    let mut nodes = Vec::new();
    let mut cross_chunk = Vec::new();
    for (ordinal, ((attack, boundary, input_id, checklist_id), (child_hashes, workers))) in
        leaf_groups.into_iter().enumerate()
    {
        let node = build_synthesis_node_on(
            &mut tx,
            census_id,
            "cross_chunk",
            0,
            ordinal as i32,
            Some(attack),
            Some(boundary),
            vec![input_id],
            vec![checklist_id],
            child_hashes,
            workers,
            &relationship_hash,
        )
        .await?;
        nodes.push(node.clone());
        cross_chunk.push(node);
    }
    if cross_chunk.is_empty() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let mut dimensions: BTreeMap<(String, String), Vec<SynthesisNodeDraft>> = BTreeMap::new();
    for node in cross_chunk {
        dimensions
            .entry((
                node.attack_class_id.clone().unwrap_or_default(),
                node.trust_boundary_hash.clone().unwrap_or_default(),
            ))
            .or_default()
            .push(node);
    }
    let mut dimension_roots = Vec::new();
    for ((attack, boundary), leaves) in dimensions {
        let mut current = Vec::new();
        for (ordinal, chunk) in leaves.chunks(FAN_IN).enumerate() {
            let node = combine_synthesis_nodes_on(
                &mut tx,
                census_id,
                "cross_input_partition",
                0,
                ordinal as i32,
                Some(attack.clone()),
                Some(boundary.clone()),
                chunk,
                &relationship_hash,
            )
            .await?;
            nodes.push(node.clone());
            current.push(node);
        }
        let mut level = 1;
        while current.len() > 1 {
            let mut next = Vec::new();
            for (ordinal, chunk) in current.chunks(FAN_IN).enumerate() {
                let node = combine_synthesis_nodes_on(
                    &mut tx,
                    census_id,
                    "cross_input_reduce",
                    level,
                    ordinal as i32,
                    Some(attack.clone()),
                    Some(boundary.clone()),
                    chunk,
                    &relationship_hash,
                )
                .await?;
                nodes.push(node.clone());
                next.push(node);
            }
            current = next;
            level += 1;
        }
        dimension_roots.push(current.remove(0));
    }
    let dimension_hashes = dimension_roots
        .iter()
        .map(|node| node.node_hash.clone())
        .collect::<Vec<_>>();
    let dimension_set_hash = hash_text_array_on(&mut tx, &dimension_hashes).await?;
    let dimension_count = dimension_roots.len() as i64;
    let mut current = dimension_roots;
    let mut level = 0;
    while current.len() > 1 {
        let mut next = Vec::new();
        for (ordinal, chunk) in current.chunks(FAN_IN).enumerate() {
            let node = combine_synthesis_nodes_on(
                &mut tx,
                census_id,
                "cross_dimension_reduce",
                level,
                ordinal as i32,
                None,
                None,
                chunk,
                &relationship_hash,
            )
            .await?;
            nodes.push(node.clone());
            next.push(node);
        }
        current = next;
        level += 1;
    }
    let global = combine_synthesis_nodes_on(
        &mut tx,
        census_id,
        "global_semantic_root",
        level,
        0,
        None,
        None,
        &current,
        &relationship_hash,
    )
    .await?;
    let global_id = global.node_id;
    nodes.push(global);
    let node_hashes = nodes
        .iter()
        .map(|node| node.node_hash.clone())
        .collect::<Vec<_>>();
    let node_set_hash = hash_text_array_on(&mut tx, &node_hashes).await?;
    let node_count = nodes.len() as i64;
    let census_hash=hash_json_on(&mut tx,&json!({"domain":"candidate_hypothesis_coverage_synthesis_census.v1",
        "analysis_attempt_id":input.fence.analysis_attempt_id,"relationship_cross_index_hash":relationship_hash,
        "fan_in_limit":FAN_IN,"node_count":node_count,"node_set_hash":node_set_hash,
        "dimension_root_count":dimension_count,"dimension_root_set_hash":dimension_set_hash,
        "global_root_node_id":global_id})).await?;
    let existing:Option<(Uuid,i64,String,Uuid,String)>=sqlx::query_as(r#"SELECT synthesis_census_id,
        node_count,node_set_hash,global_root_node_id,census_hash
        FROM candidate_analysis_hypothesis_coverage_synthesis_censuses WHERE analysis_attempt_id=$1"#)
        .bind(input.fence.analysis_attempt_id).fetch_optional(&mut *tx).await?;
    let replayed = existing.is_some();
    if let Some((id, count, set_hash, root_id, hash)) = existing {
        if id != census_id
            || count != node_count
            || set_hash != node_set_hash
            || root_id != global_id
            || hash != census_hash
        {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
    } else {
        sqlx::query(r#"INSERT INTO candidate_analysis_hypothesis_coverage_synthesis_censuses(
        synthesis_census_id,analysis_attempt_id,relationship_cross_index_hash,fan_in_limit,node_count,
        node_set_hash,dimension_root_count,dimension_root_set_hash,global_root_node_id,census_hash)
        VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#).bind(census_id).bind(input.fence.analysis_attempt_id)
        .bind(&relationship_hash).bind(FAN_IN as i32).bind(node_count).bind(&node_set_hash)
        .bind(dimension_count).bind(&dimension_set_hash).bind(global_id).bind(&census_hash)
        .execute(&mut *tx).await?;
        for node in nodes {
            sqlx::query(r#"INSERT INTO candidate_analysis_hypothesis_coverage_synthesis_census_members(
            synthesis_node_id,synthesis_census_id,analysis_attempt_id,node_kind,level,partition_ordinal,
            attack_class_id,trust_boundary_hash,covered_input_count,covered_input_set_hash,
            covered_checklist_count,covered_checklist_set_hash,child_receipt_count,child_receipt_set_hash,
            relationship_cross_index_hash,descendant_worker_count,descendant_worker_set_hash,node_hash)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#)
            .bind(node.node_id).bind(census_id).bind(input.fence.analysis_attempt_id).bind(node.node_kind)
            .bind(node.level).bind(node.partition_ordinal).bind(node.attack_class_id).bind(node.trust_boundary_hash)
            .bind(node.covered_input_ids.len() as i64).bind(node.covered_input_set_hash)
            .bind(node.covered_checklist_ids.len() as i64).bind(node.covered_checklist_set_hash)
            .bind(node.child_hashes.len() as i64).bind(node.child_set_hash).bind(node.relationship_cross_index_hash)
            .bind(node.descendant_workers.len() as i64).bind(node.descendant_worker_set_hash).bind(node.node_hash)
            .execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    Ok(CoverageSynthesisCensusRowView {
        census_id,
        analysis_attempt_id: input.fence.analysis_attempt_id,
        member_count: node_count,
        member_set_hash: node_set_hash,
        census_hash,
        global_semantic_root_member_id: global_id,
        row_version: 0,
        replayed,
    })
}
#[derive(Debug, Clone)]
pub struct RecordCoverageSynthesisReviewInput {
    pub fence: CandidateWriteFenceRow,
    pub stable_review_request_id: Uuid,
    pub synthesis_census_id: Uuid,
    pub synthesis_census_member_id: Uuid,
    pub node_kind: String,
    pub outcome: String,
    pub missed_proposal_ids: Vec<Uuid>,
    pub blocker_reason: Option<String>,
    pub review_notes: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageSynthesisReceiptRow {
    pub synthesis_review_id: Uuid,
    pub synthesis_census_id: Uuid,
    pub synthesis_census_member_id: Uuid,
    pub synthesis_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}
#[derive(Debug, sqlx::FromRow)]
struct SynthesisReviewNodeDbRow {
    node_kind: String,
    level: i32,
    descendant_worker_set_hash: String,
    relationship_cross_index_hash: String,
    node_hash: String,
}

fn synthesis_rank(kind: &str, level: i32) -> i32 {
    match kind {
        "cross_chunk" => 0,
        "cross_input_partition" => 100,
        "cross_input_reduce" => 200 + level,
        "cross_dimension_reduce" => 400 + level,
        "global_semantic_root" => 1000,
        _ => -1,
    }
}

pub async fn record_hypothesis_coverage_synthesis_review(
    pool: &PgPool,
    mut input: RecordCoverageSynthesisReviewInput,
) -> Result<CoverageSynthesisReceiptRow> {
    let mut tx = pool.begin().await?;
    validate_write_fence_on(&mut tx, &input.fence).await?;
    let node = sqlx::query_as::<_, SynthesisReviewNodeDbRow>(
        r#"SELECT node_kind,level,
        descendant_worker_set_hash,relationship_cross_index_hash,node_hash
        FROM candidate_analysis_hypothesis_coverage_synthesis_census_members
        WHERE synthesis_census_id=$1 AND synthesis_node_id=$2 AND analysis_attempt_id=$3"#,
    )
    .bind(input.synthesis_census_id)
    .bind(input.synthesis_census_member_id)
    .bind(input.fence.analysis_attempt_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
    if node.node_kind != input.node_kind || synthesis_rank(&node.node_kind, node.level) < 0 {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let candidate_item_id = candidate_item_id_on(
        &mut tx,
        input.fence.analysis_attempt_id,
        input.fence.work_item_id,
    )
    .await?;
    let work_matches: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(SELECT 1 FROM candidate_analysis_work_items
        WHERE candidate_work_item_id=$1 AND phase='critic'
          AND capability IN ('hypothesis_coverage_synthesis','coverage_synthesis')
          AND component_id=$2)"#,
    )
    .bind(candidate_item_id)
    .bind(input.synthesis_census_member_id)
    .fetch_one(&mut *tx)
    .await?;
    if !work_matches {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let current_rank = synthesis_rank(&node.node_kind, node.level);
    let missing_prior:i64=sqlx::query_scalar(r#"SELECT count(*) FROM
        candidate_analysis_hypothesis_coverage_synthesis_census_members candidate
        LEFT JOIN candidate_analysis_hypothesis_coverage_synthesis_reviews review
          ON review.synthesis_node_id=candidate.synthesis_node_id
        WHERE candidate.synthesis_census_id=$1 AND review.synthesis_review_id IS NULL
          AND (CASE candidate.node_kind WHEN 'cross_chunk' THEN 0 WHEN 'cross_input_partition' THEN 100
             WHEN 'cross_input_reduce' THEN 200+candidate.level
             WHEN 'cross_dimension_reduce' THEN 400+candidate.level ELSE 1000 END)<$2"#)
        .bind(input.synthesis_census_id).bind(current_rank).fetch_one(&mut *tx).await?;
    if missing_prior != 0 {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let worker_reused:bool=sqlx::query_scalar(r#"SELECT EXISTS(
        SELECT 1 FROM candidate_analysis_hypothesis_coverage_subreviews
         WHERE analysis_attempt_id=$1 AND ($2=primary_analyst_worker_run_id OR $2=map_critic_worker_run_id)
        UNION ALL SELECT 1 FROM candidate_analysis_hypothesis_coverage_synthesis_reviews
         WHERE analysis_attempt_id=$1 AND synthesis_worker_run_id=$2)"#)
        .bind(input.fence.analysis_attempt_id).bind(input.fence.worker_run_id).fetch_one(&mut *tx).await?;
    if worker_reused {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    input.missed_proposal_ids.sort_unstable();
    input.missed_proposal_ids.dedup();
    let valid = match input.outcome.as_str() {
        "no_composite_miss" => {
            input.missed_proposal_ids.is_empty() && input.blocker_reason.is_none()
        }
        "missed_hypothesis" => {
            !input.missed_proposal_ids.is_empty() && input.blocker_reason.is_none()
        }
        "blocked" => input
            .blocker_reason
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty()),
        _ => false,
    };
    if !valid {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let context_truncated = input.blocker_reason.as_deref() == Some("context_truncated");
    let body = json!({"kind":"hypothesis_coverage_synthesis.v1","synthesis_census_id":input.synthesis_census_id,
        "synthesis_node_id":input.synthesis_census_member_id,"node_kind":input.node_kind,
        "outcome":input.outcome,"typed_missed_refs":input.missed_proposal_ids,
        "blocker_reason":input.blocker_reason,"review_notes":input.review_notes});
    let synthesis_hash = hash_json_on(
        &mut tx,
        &json!({"body":body,"node_hash":node.node_hash,
        "relationship_cross_index_hash":node.relationship_cross_index_hash,
        "transitive_descendant_worker_set_hash":node.descendant_worker_set_hash,
        "synthesis_worker_run_id":input.fence.worker_run_id,"worker_separation_valid":true,
        "context_truncated":context_truncated}),
    )
    .await?;
    let review_id = Uuid::new_v5(
        &input.stable_review_request_id,
        input.synthesis_census_member_id.as_bytes(),
    );
    let existing:Option<(Uuid,String)>=sqlx::query_as("SELECT synthesis_review_id,review_hash FROM candidate_analysis_hypothesis_coverage_synthesis_reviews WHERE synthesis_node_id=$1")
        .bind(input.synthesis_census_member_id).fetch_optional(&mut *tx).await?;
    let replayed = existing.is_some();
    if let Some((id, hash)) = existing {
        if id != review_id || hash != synthesis_hash {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
    } else {
        let artifact_id = Uuid::new_v5(
            &input.stable_review_request_id,
            b"hypothesis_coverage_synthesis.v1",
        );
        let artifact_hash = hash_json_on(&mut tx, &body).await?;
        sqlx::query(r#"INSERT INTO candidate_analysis_artifacts(
            artifact_id,analysis_attempt_id,candidate_work_item_id,worker_run_id,artifact_kind,artifact_body,artifact_hash)
            VALUES($1,$2,$3,$4,'hypothesis_coverage_synthesis.v1',$5,$6)"#).bind(artifact_id)
            .bind(input.fence.analysis_attempt_id).bind(candidate_item_id).bind(input.fence.worker_run_id)
            .bind(&body).bind(artifact_hash).execute(&mut *tx).await?;
        sqlx::query(
            r#"INSERT INTO candidate_analysis_hypothesis_coverage_synthesis_reviews(
            synthesis_review_id,synthesis_node_id,synthesis_census_id,analysis_attempt_id,
            synthesis_worker_run_id,transitive_descendant_worker_set_hash,worker_separation_valid,
            context_truncated,outcome,typed_missed_refs,review_hash)
            VALUES($1,$2,$3,$4,$5,$6,TRUE,$7,$8,$9,$10)"#,
        )
        .bind(review_id)
        .bind(input.synthesis_census_member_id)
        .bind(input.synthesis_census_id)
        .bind(input.fence.analysis_attempt_id)
        .bind(input.fence.worker_run_id)
        .bind(&node.descendant_worker_set_hash)
        .bind(context_truncated)
        .bind(&input.outcome)
        .bind(json!(input.missed_proposal_ids))
        .bind(&synthesis_hash)
        .execute(&mut *tx)
        .await?;
        if node.node_kind == "global_semantic_root" {
            let census:(i64,String,String)=sqlx::query_as(r#"SELECT
            dimension_root_count,dimension_root_set_hash,relationship_cross_index_hash
            FROM candidate_analysis_hypothesis_coverage_synthesis_censuses WHERE synthesis_census_id=$1"#)
            .bind(input.synthesis_census_id).fetch_one(&mut *tx).await?;
            let worker_hashes:Vec<String>=sqlx::query_scalar(r#"SELECT worker_id::TEXT FROM(
                SELECT primary_analyst_worker_run_id AS worker_id FROM candidate_analysis_hypothesis_coverage_subreviews WHERE analysis_attempt_id=$1
                UNION SELECT map_critic_worker_run_id FROM candidate_analysis_hypothesis_coverage_subreviews WHERE analysis_attempt_id=$1
                UNION SELECT synthesis_worker_run_id FROM candidate_analysis_hypothesis_coverage_synthesis_reviews WHERE analysis_attempt_id=$1) workers ORDER BY worker_id"#)
                .bind(input.fence.analysis_attempt_id).fetch_all(&mut *tx).await?;
            let worker_set_hash = hash_text_array_on(&mut tx, &worker_hashes).await?;
            let global_outcome = match input.outcome.as_str() {
                "no_composite_miss" => "adequate",
                value => value,
            };
            let global_hash = hash_json_on(
                &mut tx,
                &json!({"global_root_review_id":review_id,
                "dimension_root_set_hash":census.1,"relationship_cross_index_hash":census.2,
                "worker_separation_set_hash":worker_set_hash,"outcome":global_outcome}),
            )
            .await?;
            sqlx::query(r#"INSERT INTO candidate_analysis_hypothesis_coverage_global_reviews(
                global_review_id,analysis_attempt_id,synthesis_census_id,global_root_review_id,
                dimension_root_count,dimension_root_set_hash,relationship_cross_index_hash,
                worker_separation_set_hash,outcome,review_hash)VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#)
                .bind(Uuid::new_v5(&review_id,b"global_review")).bind(input.fence.analysis_attempt_id)
                .bind(input.synthesis_census_id).bind(review_id).bind(census.0).bind(census.1)
                .bind(census.2).bind(worker_set_hash).bind(global_outcome).bind(global_hash)
                .execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    Ok(CoverageSynthesisReceiptRow {
        synthesis_review_id: review_id,
        synthesis_census_id: input.synthesis_census_id,
        synthesis_census_member_id: input.synthesis_census_member_id,
        synthesis_hash,
        row_version: 0,
        replayed,
    })
}
#[derive(Debug, Clone)]
pub struct ReduceCoverageReviewInput {
    pub fence: CandidateWriteFenceRow,
    pub stable_reduction_request_id: Uuid,
    pub input_id: Uuid,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageReviewReceiptRow {
    pub coverage_review_id: Uuid,
    pub input_id: Uuid,
    pub outcome: String,
    pub coverage_review_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct CoverageChecklistPolicyDbRow {
    checklist_member_id: Uuid,
    attack_class_contract_version: String,
    attack_class_contract_digest: String,
    trust_boundary_contract_version: String,
    trust_boundary_contract_digest: String,
    member_hash: String,
}

pub async fn reduce_hypothesis_coverage_review(
    pool: &PgPool,
    input: ReduceCoverageReviewInput,
) -> Result<CoverageReviewReceiptRow> {
    let mut tx = pool.begin().await?;
    validate_write_fence_on(&mut tx, &input.fence).await?;
    let chunk_census_id:Uuid=sqlx::query_scalar(r#"SELECT census.chunk_census_id
        FROM candidate_analysis_input_chunk_censuses census
        JOIN candidate_analysis_snapshot_inputs source ON source.snapshot_input_id=census.snapshot_input_id
        WHERE census.snapshot_input_id=$1 AND source.snapshot_id=$2 AND census.disposition IN ('complete','source_empty')"#)
        .bind(input.input_id).bind(input.fence.snapshot_id).fetch_optional(&mut *tx).await?
        .ok_or_else(||conflict(CENSUS_NOT_CLOSED))?;
    let partition_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT partition_hash FROM
        candidate_analysis_hypothesis_coverage_chunk_partitions WHERE analysis_attempt_id=$1
        AND snapshot_input_id=$2 ORDER BY partition_ordinal"#,
    )
    .bind(input.fence.analysis_attempt_id)
    .bind(input.input_id)
    .fetch_all(&mut *tx)
    .await?;
    if partition_hashes.is_empty() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let partition_set_hash = hash_text_array_on(&mut tx, &partition_hashes).await?;
    let sub_census: (Uuid, i64, String) = sqlx::query_as(
        r#"SELECT subreview_census_id,expected_member_count,
        member_set_hash FROM candidate_analysis_hypothesis_coverage_subreview_censuses
        WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2"#,
    )
    .bind(input.fence.analysis_attempt_id)
    .bind(input.input_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    let subreviews:Vec<(String,String,String)>=sqlx::query_as(r#"SELECT review.subreview_hash,review.outcome,
        review.read_receipt_set_hash FROM candidate_analysis_hypothesis_coverage_subreview_census_members member
        JOIN candidate_analysis_hypothesis_coverage_subreviews review USING(subreview_census_member_id)
        WHERE member.subreview_census_id=$1 AND member.disposition='required' ORDER BY member.checklist_ordinal,member.partition_ordinal"#)
        .bind(sub_census.0).fetch_all(&mut *tx).await?;
    let sampling_omitted:i64=sqlx::query_scalar("SELECT count(*) FROM candidate_analysis_hypothesis_coverage_subreview_census_members WHERE subreview_census_id=$1 AND disposition='sampling_omitted'")
        .bind(sub_census.0).fetch_one(&mut *tx).await?;
    let required_count:i64=sqlx::query_scalar("SELECT count(*) FROM candidate_analysis_hypothesis_coverage_subreview_census_members WHERE subreview_census_id=$1 AND disposition='required'")
        .bind(sub_census.0).fetch_one(&mut *tx).await?;
    let subreviews_closed = subreviews.len() as i64 == required_count
        && sub_census.1 == required_count + sampling_omitted;
    let read_hashes = subreviews
        .iter()
        .map(|row| row.2.clone())
        .collect::<Vec<_>>();
    let read_set_hash = hash_text_array_on(&mut tx, &read_hashes).await?;
    let proposal_disposition: (i64, String, String) = sqlx::query_as(
        r#"SELECT proposal_ref_count,
        proposal_ref_set_hash,disposition FROM candidate_analysis_input_proposal_dispositions
        WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2"#,
    )
    .bind(input.fence.analysis_attempt_id)
    .bind(input.input_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    if proposal_disposition.2 == "blocked" {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let checklist=sqlx::query_as::<_,CoverageChecklistPolicyDbRow>(r#"SELECT checklist_member_id,
        attack_class_contract_version,attack_class_contract_digest,trust_boundary_contract_version,
        trust_boundary_contract_digest,member_hash FROM candidate_analysis_hypothesis_coverage_checklist_members
        WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2 ORDER BY ordinal"#)
        .bind(input.fence.analysis_attempt_id).bind(input.input_id).fetch_all(&mut *tx).await?;
    let first_checklist = checklist
        .first()
        .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    if checklist.iter().any(|row| {
        row.attack_class_contract_version != first_checklist.attack_class_contract_version
            || row.attack_class_contract_digest != first_checklist.attack_class_contract_digest
            || row.trust_boundary_contract_version
                != first_checklist.trust_boundary_contract_version
            || row.trust_boundary_contract_digest != first_checklist.trust_boundary_contract_digest
    }) {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let checklist_hashes = checklist
        .iter()
        .map(|row| row.member_hash.clone())
        .collect::<Vec<_>>();
    let checklist_set_hash = hash_text_array_on(&mut tx, &checklist_hashes).await?;
    let attempt_policy: (String, String, i32) = sqlx::query_as(
        r#"SELECT coverage_sampling_contract_version,
        coverage_sampling_contract_digest,attempt_ordinal FROM candidate_analysis_attempts
        WHERE analysis_attempt_id=$1 AND snapshot_id=$2"#,
    )
    .bind(input.fence.analysis_attempt_id)
    .bind(input.fence.snapshot_id)
    .fetch_one(&mut *tx)
    .await?;
    let synthesis:(Uuid,i64,Uuid)=sqlx::query_as(r#"SELECT synthesis_census_id,node_count,global_root_node_id
        FROM candidate_analysis_hypothesis_coverage_synthesis_censuses WHERE analysis_attempt_id=$1"#)
        .bind(input.fence.analysis_attempt_id).fetch_optional(&mut *tx).await?.ok_or_else(||conflict(CENSUS_NOT_CLOSED))?;
    let synthesis_reviews:Vec<(String,String,Value)>=sqlx::query_as(r#"SELECT review.review_hash,review.outcome,
        review.typed_missed_refs FROM candidate_analysis_hypothesis_coverage_synthesis_reviews review
        WHERE review.synthesis_census_id=$1 ORDER BY review.synthesis_node_id"#).bind(synthesis.0)
        .fetch_all(&mut *tx).await?;
    let synthesis_closed = synthesis_reviews.len() as i64 == synthesis.1;
    let global: (Uuid, String, String, String) = sqlx::query_as(
        r#"SELECT global_review_id,outcome,review_hash,
        worker_separation_set_hash FROM candidate_analysis_hypothesis_coverage_global_reviews
        WHERE analysis_attempt_id=$1 AND synthesis_census_id=$2"#,
    )
    .bind(input.fence.analysis_attempt_id)
    .bind(synthesis.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    let mut missed_refs = Vec::new();
    let sub_missed:Vec<Value>=sqlx::query_scalar(r#"SELECT typed_missed_refs FROM
        candidate_analysis_hypothesis_coverage_subreviews WHERE subreview_census_id=$1 ORDER BY subreview_id"#)
        .bind(sub_census.0).fetch_all(&mut *tx).await?;
    for value in sub_missed
        .into_iter()
        .chain(synthesis_reviews.iter().map(|row| row.2.clone()))
    {
        if let Some(values) = value.as_array() {
            missed_refs.extend(values.iter().cloned());
        }
    }
    missed_refs.sort_by_key(Value::to_string);
    missed_refs.dedup();
    let any_missed = subreviews.iter().any(|row| row.1 == "missed_hypothesis")
        || synthesis_reviews
            .iter()
            .any(|row| row.1 == "missed_hypothesis")
        || global.1 == "missed_hypothesis";
    let any_blocked = !subreviews_closed
        || !synthesis_closed
        || subreviews.iter().any(|row| row.1 == "blocked")
        || synthesis_reviews.iter().any(|row| row.1 == "blocked")
        || global.1 == "blocked"
        || sampling_omitted > 0;
    let outcome = if any_blocked {
        "blocked"
    } else if any_missed {
        "missed_hypothesis"
    } else {
        "adequate"
    };
    let review_mode = if sampling_omitted > 0 {
        "deterministic_sample"
    } else {
        "full"
    };
    let checklist_dispositions=Value::Array(checklist.iter().map(|row|json!({"checklist_member_id":row.checklist_member_id,
        "disposition":if any_blocked{"blocked"}else if any_missed{"missed_hypothesis"}else{"adequate"}})).collect());
    let body = json!({"kind":"hypothesis_coverage_review.v1","analysis_attempt_id":input.fence.analysis_attempt_id,
        "snapshot_input_id":input.input_id,"outcome":outcome,"review_mode":review_mode,
        "checklist_dispositions":checklist_dispositions,"typed_missed_refs":missed_refs});
    let review_hash=hash_json_on(&mut tx,&json!({"body":body,"chunk_census_id":chunk_census_id,
        "chunk_partition_set_hash":partition_set_hash,"subreview_census_id":sub_census.0,
        "subreview_member_set_hash":sub_census.2,"read_receipt_set_hash":read_set_hash,
        "h1_proposal_ref_set_hash":proposal_disposition.1,"checklist_member_set_hash":checklist_set_hash,
        "synthesis_census_id":synthesis.0,"global_root_node_id":synthesis.2,
        "global_review_hash":global.2,"worker_separation_set_hash":global.3,
        "coverage_sampling_contract_digest":attempt_policy.1})).await?;
    let review_id = Uuid::new_v5(
        &input.stable_reduction_request_id,
        input.input_id.as_bytes(),
    );
    let existing:Option<(Uuid,String,String)>=sqlx::query_as(r#"SELECT coverage_review_id,outcome,review_hash
        FROM candidate_analysis_hypothesis_coverage_reviews WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2"#)
        .bind(input.fence.analysis_attempt_id).bind(input.input_id).fetch_optional(&mut *tx).await?;
    let replayed = existing.is_some();
    if let Some((id, existing_outcome, hash)) = existing {
        if id != review_id || existing_outcome != outcome || hash != review_hash {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
    } else {
        let candidate_item_id = candidate_item_id_on(
            &mut tx,
            input.fence.analysis_attempt_id,
            input.fence.work_item_id,
        )
        .await?;
        let artifact_id = Uuid::new_v5(
            &input.stable_reduction_request_id,
            b"hypothesis_coverage_review.v1",
        );
        let artifact_hash = hash_json_on(&mut tx, &body).await?;
        sqlx::query(r#"INSERT INTO candidate_analysis_artifacts(
            artifact_id,analysis_attempt_id,candidate_work_item_id,worker_run_id,artifact_kind,artifact_body,artifact_hash)
            VALUES($1,$2,$3,$4,'hypothesis_coverage_review.v1',$5,$6)"#).bind(artifact_id)
            .bind(input.fence.analysis_attempt_id).bind(candidate_item_id).bind(input.fence.worker_run_id)
            .bind(&body).bind(artifact_hash).execute(&mut *tx).await?;
        sqlx::query(r#"INSERT INTO candidate_analysis_hypothesis_coverage_reviews(
            coverage_review_id,analysis_attempt_id,snapshot_input_id,attempt_ordinal,chunk_census_id,
            chunk_partition_count,chunk_partition_set_hash,subreview_census_id,read_receipt_set_hash,
            h1_proposal_ref_count,h1_proposal_ref_set_hash,attack_class_checklist_version,
            attack_class_checklist_digest,trust_boundary_checklist_version,trust_boundary_checklist_digest,
            checklist_member_set_hash,synthesis_census_id,global_review_id,coverage_sampling_contract_version,
            coverage_sampling_contract_digest,worker_separation_set_hash,review_mode,outcome,
            checklist_dispositions,typed_missed_refs,review_hash)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26)"#)
            .bind(review_id).bind(input.fence.analysis_attempt_id).bind(input.input_id).bind(attempt_policy.2)
            .bind(chunk_census_id).bind(partition_hashes.len() as i64).bind(partition_set_hash)
            .bind(sub_census.0).bind(read_set_hash).bind(proposal_disposition.0).bind(proposal_disposition.1)
            .bind(&first_checklist.attack_class_contract_version).bind(&first_checklist.attack_class_contract_digest)
            .bind(&first_checklist.trust_boundary_contract_version).bind(&first_checklist.trust_boundary_contract_digest)
            .bind(checklist_set_hash).bind(synthesis.0).bind(global.0).bind(attempt_policy.0).bind(attempt_policy.1)
            .bind(global.3).bind(review_mode).bind(outcome).bind(checklist_dispositions).bind(Value::Array(missed_refs))
            .bind(&review_hash).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(CoverageReviewReceiptRow {
        coverage_review_id: review_id,
        input_id: input.input_id,
        outcome: outcome.to_owned(),
        coverage_review_hash: review_hash,
        row_version: 0,
        replayed,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadCandidateGateMaterialInput {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub analysis_attempt_id: Uuid,
    pub analysis_attempt_ordinal: i32,
    pub expected_snapshot_row_version: i64,
    pub expected_attempt_row_version: i64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateGateMaterialRow {
    pub snapshot: CandidateSnapshotRowView,
    pub active_analysis_attempt_id: Uuid,
    pub active_analysis_attempt_ordinal: i32,
    pub attempt_epoch: i64,
    pub prior_terminal_attempt_chain_hash: String,
    pub gate_temporal_reevaluation_hash: String,
    pub gate_knowledge_feed_reevaluation_hash: String,
    pub input_chunk_census_set_hash: String,
    pub proposal_census_hash: String,
    pub critic_census_hash: String,
    pub coverage_subreview_census_set_hash: String,
    pub coverage_synthesis_census_set_hash: String,
    pub coverage_global_semantic_root_hash: String,
    pub coverage_global_review_hash: String,
    pub coverage_review_set_hash: String,
    pub coverage_checklist_set_hash: String,
    pub controller_decision_set_hash: String,
    pub mutation_set_hash: String,
    pub claim_component_set_hash: String,
    pub verification_contract_set_hash: String,
    pub verification_plan_set_hash: String,
    pub generation_transition_set_hash: String,
    pub compiler_seal_hash: String,
    pub final_submitter_worker_run_id: Uuid,
    pub snapshot_row_version: i64,
    pub attempt_row_version: i64,
}
pub async fn load_candidate_gate_material(
    pool: &PgPool,
    input: LoadCandidateGateMaterialInput,
) -> Result<CandidateGateMaterialRow> {
    if input.expected_snapshot_row_version != 0 || input.expected_attempt_row_version != 0 {
        return Err(conflict(WRITE_FENCE_MISMATCH));
    }
    let mut tx = pool.begin().await?;
    let snapshot = load_snapshot_on(&mut tx, input.snapshot_id).await?;
    if snapshot.disposition != CandidateSnapshotDispositionRow::SealedReady
        || snapshot.operation_id != input.operation_id
        || snapshot.organization_id != input.organization_id
        || snapshot.scope_snapshot_id != input.scope_snapshot_id
    {
        return Err(conflict(SNAPSHOT_NOT_READY));
    }
    let gate_reevaluation =
        reevaluate_candidate_gate_authority_on(&mut tx, input.snapshot_id).await?;
    let attempt_epoch: i64 = sqlx::query_scalar(r#"SELECT COALESCE(MAX(worker.attempt_epoch),attempt.attempt_ordinal::BIGINT)
        FROM candidate_analysis_attempts attempt
        LEFT JOIN candidate_analysis_work_items candidate_item ON candidate_item.analysis_attempt_id=attempt.analysis_attempt_id
        LEFT JOIN stage_worker_runs worker ON worker.work_item_id=candidate_item.stage_work_item_id
        WHERE attempt.analysis_attempt_id=$1 AND attempt.snapshot_id=$2 AND attempt.attempt_ordinal=$3
        GROUP BY attempt.attempt_ordinal"#).bind(input.analysis_attempt_id).bind(input.snapshot_id)
        .bind(input.analysis_attempt_ordinal).fetch_optional(&mut *tx).await?
        .ok_or_else(||conflict(AUTHORITY_MISMATCH))?;
    let proposal = sqlx::query_scalar(
        "SELECT census_hash FROM candidate_analysis_proposal_censuses WHERE analysis_attempt_id=$1",
    )
    .bind(input.analysis_attempt_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    let critic = sqlx::query_scalar(
        "SELECT census_hash FROM candidate_analysis_critic_censuses WHERE analysis_attempt_id=$1",
    )
    .bind(input.analysis_attempt_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    let worker=sqlx::query_scalar("SELECT final_submitter_worker_run_id FROM stage_team_plans WHERE operation_id=$1 AND organization_id=$2 AND scope_snapshot_id=$3 AND final_submitter_worker_run_id IS NOT NULL ORDER BY updated_at DESC LIMIT 1").bind(input.operation_id).bind(input.organization_id).bind(input.scope_snapshot_id).fetch_optional(&mut *tx).await?.ok_or_else(||conflict(CENSUS_NOT_CLOSED))?;
    let chain_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT event.event_hash
        FROM candidate_analysis_attempts attempt JOIN candidate_analysis_attempt_state_events event
          ON event.analysis_attempt_id=attempt.analysis_attempt_id
        WHERE attempt.snapshot_id=$1 AND attempt.attempt_ordinal<$2
          AND event.event_kind IN ('superseded_missed_hypothesis','sealed','blocked')
        ORDER BY attempt.attempt_ordinal,event.event_ordinal"#,
    )
    .bind(input.snapshot_id)
    .bind(input.analysis_attempt_ordinal)
    .fetch_all(&mut *tx)
    .await?;
    let chain = hash_text_array_on(&mut tx, &chain_hashes).await?;
    let chunk_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT census_hash FROM
        candidate_analysis_input_chunk_censuses WHERE snapshot_id=$1 ORDER BY snapshot_input_id"#,
    )
    .bind(input.snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let input_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1",
    )
    .bind(input.snapshot_id)
    .fetch_one(&mut *tx)
    .await?;
    if input_count == 0 || chunk_hashes.len() as i64 != input_count {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let input_chunk_census_set_hash = hash_text_array_on(&mut tx, &chunk_hashes).await?;
    let subreview_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT census_hash FROM
        candidate_analysis_hypothesis_coverage_subreview_censuses WHERE analysis_attempt_id=$1
        ORDER BY snapshot_input_id"#,
    )
    .bind(input.analysis_attempt_id)
    .fetch_all(&mut *tx)
    .await?;
    if subreview_hashes.len() as i64 != input_count {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let coverage_subreview_census_set_hash = hash_text_array_on(&mut tx, &subreview_hashes).await?;
    let (coverage_synthesis_census_set_hash,global_node_id):(String,Uuid)=sqlx::query_as(r#"SELECT
        census_hash,global_root_node_id FROM candidate_analysis_hypothesis_coverage_synthesis_censuses
        WHERE analysis_attempt_id=$1"#).bind(input.analysis_attempt_id).fetch_optional(&mut *tx).await?
        .ok_or_else(||conflict(CENSUS_NOT_CLOSED))?;
    let coverage_global_semantic_root_hash: String = sqlx::query_scalar(
        r#"SELECT node_hash FROM
        candidate_analysis_hypothesis_coverage_synthesis_census_members WHERE synthesis_node_id=$1
        AND analysis_attempt_id=$2 AND node_kind='global_semantic_root'"#,
    )
    .bind(global_node_id)
    .bind(input.analysis_attempt_id)
    .fetch_one(&mut *tx)
    .await?;
    let coverage_global_review_hash: String = sqlx::query_scalar(
        r#"SELECT review_hash FROM
        candidate_analysis_hypothesis_coverage_global_reviews WHERE analysis_attempt_id=$1"#,
    )
    .bind(input.analysis_attempt_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    let coverage_hashes:Vec<String>=sqlx::query_scalar(r#"SELECT review_hash FROM
        candidate_analysis_hypothesis_coverage_reviews WHERE analysis_attempt_id=$1 ORDER BY snapshot_input_id"#)
        .bind(input.analysis_attempt_id).fetch_all(&mut *tx).await?;
    if coverage_hashes.len() as i64 != input_count {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let coverage_review_set_hash = hash_text_array_on(&mut tx, &coverage_hashes).await?;
    let checklist_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT member_hash FROM
        candidate_analysis_hypothesis_coverage_checklist_members WHERE analysis_attempt_id=$1
        ORDER BY snapshot_input_id,ordinal"#,
    )
    .bind(input.analysis_attempt_id)
    .fetch_all(&mut *tx)
    .await?;
    if checklist_hashes.is_empty() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let coverage_checklist_set_hash = hash_text_array_on(&mut tx, &checklist_hashes).await?;
    let controller_hashes:Vec<String>=sqlx::query_scalar(r#"SELECT artifact_hash FROM candidate_analysis_artifacts
        WHERE analysis_attempt_id=$1 AND artifact_kind='controller_decision.v1' ORDER BY artifact_hash"#)
        .bind(input.analysis_attempt_id).fetch_all(&mut *tx).await?;
    let controller_decision_set_hash = hash_text_array_on(&mut tx, &controller_hashes).await?;
    let (
        mutation_set_hash,
        claim_component_set_hash,
        verification_contract_set_hash,
        verification_plan_set_hash,
        generation_transition_set_hash,
        compiler_seal_hash,
    ): (String, String, String, String, String, String) = sqlx::query_as(
        r#"SELECT mutation_set_hash,claim_component_set_hash,
                  verification_contract_set_hash,verification_plan_set_hash,
                  generation_transition_set_hash,compiler_seal_hash
             FROM candidate_analysis_host_compilation_seals
            WHERE analysis_attempt_id=$1
              AND snapshot_id=$2
              AND operation_id=$3
              AND organization_id=$4
              AND final_submitter_worker_run_id=$5"#,
    )
    .bind(input.analysis_attempt_id)
    .bind(input.snapshot_id)
    .bind(input.operation_id)
    .bind(input.organization_id)
    .bind(worker)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    tx.commit().await?;
    Ok(CandidateGateMaterialRow {
        snapshot,
        active_analysis_attempt_id: input.analysis_attempt_id,
        active_analysis_attempt_ordinal: input.analysis_attempt_ordinal,
        attempt_epoch,
        prior_terminal_attempt_chain_hash: chain,
        gate_temporal_reevaluation_hash: gate_reevaluation.temporal_hash,
        gate_knowledge_feed_reevaluation_hash: gate_reevaluation.knowledge_feed_hash,
        input_chunk_census_set_hash,
        proposal_census_hash: proposal,
        critic_census_hash: critic,
        coverage_subreview_census_set_hash,
        coverage_synthesis_census_set_hash,
        coverage_global_semantic_root_hash,
        coverage_global_review_hash,
        coverage_review_set_hash,
        coverage_checklist_set_hash,
        controller_decision_set_hash,
        mutation_set_hash,
        claim_component_set_hash,
        verification_contract_set_hash,
        verification_plan_set_hash,
        generation_transition_set_hash,
        compiler_seal_hash,
        final_submitter_worker_run_id: worker,
        snapshot_row_version: 0,
        attempt_row_version: 0,
    })
}
