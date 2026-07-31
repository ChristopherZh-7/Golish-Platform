//! Plan B Candidate snapshot and analysis persistence boundary.
//!
//! The public freeze entry accepts only server operation/scope identity and
//! enters Plan A's request-scoped opaque authority callback.  The private
//! writer has no pool overload, so the checked bundle and Candidate snapshot
//! necessarily share one repeatable-read transaction.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use golish_core::hypothesis_semantic_key::{ClaimPolarity, PredicateIdentity};
use golish_core::InvestigationAuthority;
use golish_pentest_domain::tool_truth::{
    EvidenceTemporalValidityPolicyV1, TemporalValidityStatus, ToolTruthRootFamilyV1,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Row, Transaction};
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
const H1_CONTROLLER_FENCE_REQUIRED: &str = "CANDIDATE_H1_CONTROLLER_DISPATCH_FENCE_REQUIRED";
const CONFLICT_DECISION_UNRESOLVED: &str = "CANDIDATE_CONFLICT_DECISION_UNRESOLVED";
pub const REGISTRY_CANONICAL_WRITE_FORBIDDEN: &str =
    "HYPOTHESIS_REGISTRY_CANONICAL_WRITE_FORBIDDEN";

fn conflict(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

/// Lock and decode the immutable operation contract before any Plan B
/// canonical write. Deployment defaults are not an authority after operation
/// creation, and shadow/compare modes may only reach their dedicated shadow
/// writer (never these public canonical repositories).
pub(crate) async fn lock_and_require_registry_canonical_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
) -> Result<()> {
    let frozen: Option<(String, String, String)> = sqlx::query_as(
        r#"SELECT tool_truth_contract,investigation_contract_version,
                  investigation_rollout_mode
             FROM operation_state
            WHERE operation_id=$1
            FOR UPDATE"#,
    )
    .bind(operation_id)
    .fetch_optional(&mut **tx)
    .await?;
    let (tool_truth, investigation_contract, investigation_mode) =
        frozen.ok_or_else(|| DbError::NotFound("operation_state".to_owned()))?;
    let tool_truth =
        golish_pentest_domain::tool_truth::ToolTruthContract::try_from(tool_truth.as_str())
            .map_err(|_| conflict(REGISTRY_CANONICAL_WRITE_FORBIDDEN))?;
    let (investigation_contract, investigation_mode) =
        super::investigation_rollout::parse_frozen_pair(
            &investigation_contract,
            &investigation_mode,
        )
        .map_err(|_| conflict(REGISTRY_CANONICAL_WRITE_FORBIDDEN))?;
    super::operation_rollout::validate_joint_pair(
        tool_truth,
        investigation_contract,
        investigation_mode,
    )
    .map_err(|_| conflict(REGISTRY_CANONICAL_WRITE_FORBIDDEN))?;
    if investigation_mode.policy().canonical_writer != InvestigationAuthority::Registry {
        return Err(conflict(REGISTRY_CANONICAL_WRITE_FORBIDDEN));
    }
    Ok(())
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateChunkPageHashInput {
    pub analysis_attempt_id: Uuid,
    pub snapshot_id: Uuid,
    pub snapshot_input_id: Uuid,
    pub chunk_census_id: Uuid,
    pub chunk_census_hash: String,
    pub consumer_worker_run_id: Uuid,
    pub first_ordinal: Option<i32>,
    pub last_ordinal: Option<i32>,
    pub ordered_chunk_hashes: Vec<String>,
    pub source_size_bytes: i64,
    pub chunking_contract_version: String,
    pub redaction_contract_version: String,
}

pub(crate) async fn candidate_chunk_page_hash_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &CandidateChunkPageHashInput,
) -> Result<String> {
    hash_json_on(
        tx,
        &json!({
            "schema":"candidate_chunk_page_receipt.v1",
            "analysis_attempt_id":input.analysis_attempt_id,
            "snapshot_id":input.snapshot_id,
            "snapshot_input_id":input.snapshot_input_id,
            "chunk_census_id":input.chunk_census_id,
            "chunk_census_hash":input.chunk_census_hash,
            "consumer_worker_run_id":input.consumer_worker_run_id,
            "first_ordinal":input.first_ordinal,
            "last_ordinal":input.last_ordinal,
            "returned_count":input.ordered_chunk_hashes.len(),
            "ordered_chunk_hashes":input.ordered_chunk_hashes,
            "source_size_bytes":input.source_size_bytes,
            "chunking_contract_version":input.chunking_contract_version,
            "redaction_contract_version":input.redaction_contract_version,
        }),
    )
    .await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateAnalysisExactClosureRow {
    pub all_input_count: i64,
    pub complete_input_count: i64,
    pub all_input_set_hash: String,
    pub complete_input_set_hash: String,
    pub proposal_census_hash: String,
    pub h1_disposition_set_hash: String,
    pub coverage_checklist_set_hash: String,
    pub coverage_partition_set_hash: String,
    pub coverage_subreview_census_set_hash: String,
    pub page_receipt_set_hash: String,
    pub critic_census_hash: Option<String>,
    pub gate_eligible: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExactClosureAttemptDbRow {
    snapshot_id: Uuid,
    attack_class_checklist_version: String,
    attack_class_checklist_digest: String,
    trust_boundary_checklist_version: String,
    trust_boundary_checklist_digest: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExactClosureInputDbRow {
    snapshot_input_id: Uuid,
    source_ref: String,
    subject_kind_at_time: String,
    subject_identity_hash: String,
    server_chunking_disposition: String,
    input_hash: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExactClosureChecklistDbRow {
    checklist_member_id: Uuid,
    snapshot_input_id: Uuid,
    ordinal: i32,
    attack_class_contract_version: String,
    attack_class_contract_digest: String,
    trust_boundary_contract_version: String,
    trust_boundary_contract_digest: String,
    attack_class_id: String,
    attack_class_version: i32,
    trust_boundary_identity: String,
    trust_boundary_hash: String,
    applicability_basis: Value,
    feed_match_member_refs: Vec<Uuid>,
    applicability_disposition: String,
    enrichment_obligation_id: Option<Uuid>,
    member_hash: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExactClosurePartitionDbRow {
    chunk_partition_id: Uuid,
    snapshot_input_id: Uuid,
    partition_ordinal: i32,
    first_chunk_ordinal: i32,
    last_chunk_ordinal: i32,
    chunk_count: i64,
    chunk_set_hash: String,
    bounded_context_budget: i64,
    partition_hash: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExactClosureSubreviewHeaderDbRow {
    subreview_census_id: Uuid,
    snapshot_input_id: Uuid,
    checklist_member_count: i64,
    checklist_member_set_hash: String,
    chunk_partition_count: i64,
    chunk_partition_set_hash: String,
    expected_member_count: i64,
    member_set_hash: String,
    census_hash: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExactClosureSubreviewMemberDbRow {
    subreview_census_member_id: Uuid,
    subreview_census_id: Uuid,
    snapshot_input_id: Uuid,
    checklist_member_id: Uuid,
    chunk_partition_id: Uuid,
    checklist_ordinal: i32,
    partition_ordinal: i32,
    designated_stage_work_item_id: Uuid,
    disposition: String,
    member_hash: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExactClosurePageReceiptDbRow {
    page_receipt_id: Uuid,
    stable_request_id: Uuid,
    snapshot_id: Uuid,
    snapshot_input_id: Uuid,
    chunk_census_id: Uuid,
    chunk_census_hash: String,
    source_size_bytes: i64,
    chunking_contract_version: String,
    redaction_contract_version: String,
    consumer_worker_run_id: Uuid,
    server_cursor: String,
    first_key: Option<String>,
    last_key: Option<String>,
    returned_count: i64,
    page_hash: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedFeedBlockReason {
    CatalogUnavailable,
    StoreUnavailable,
    TrustStoreDrift,
    ExactAuthorityInvalid,
    FeedStale,
    SignatureInvalid,
    SchemaVersionUnsupported,
}

impl ManagedFeedBlockReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::CatalogUnavailable => "managed_feed_catalog_unavailable",
            Self::StoreUnavailable => "managed_feed_store_unavailable",
            Self::TrustStoreDrift => "managed_feed_trust_store_drift",
            Self::ExactAuthorityInvalid => "managed_feed_exact_authority_invalid",
            Self::FeedStale => "managed_feed_stale",
            Self::SignatureInvalid => "managed_feed_signature_invalid",
            Self::SchemaVersionUnsupported => "managed_feed_schema_version_unsupported",
        }
    }
}

#[derive(Debug, Clone)]
struct ManagedFeedAuthoritySelection {
    store_catalog_id: Option<Uuid>,
    authority_manifest_hash: String,
    blocked_reason: Option<ManagedFeedBlockReason>,
}

/// Bind the operation once to the server catalog head and select every member
/// from the corresponding local signed-store heads. No request field can name
/// a catalog, feed, signer, timestamp, or subset.
async fn select_managed_feed_authority_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    _organization_id: Uuid,
) -> Result<ManagedFeedAuthoritySelection> {
    let catalog_id: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT catalog_id FROM candidate_operation_managed_feed_contracts
            WHERE operation_id=$1 FOR SHARE"#,
    )
    .bind(operation_id)
    .fetch_optional(&mut **tx)
    .await?;
    let catalog_id = if let Some(catalog_id) = catalog_id {
        catalog_id
    } else {
        let inserted: Option<Uuid> = sqlx::query_scalar(
            r#"INSERT INTO candidate_operation_managed_feed_contracts(
                   operation_id,catalog_id,catalog_version,catalog_hash,trust_policy_id,
                   trust_policy_version,trust_policy_hash,signature_algorithm_allowlist_hash,
                   required_source_count,required_source_set_hash,required_member_count,
                   required_member_set_hash)
               SELECT $1,catalog_id,catalog_version,catalog_hash,trust_policy_id,
                      trust_policy_version,trust_policy_hash,signature_algorithm_allowlist_hash,
                      required_source_count,required_source_set_hash,required_member_count,
                      required_member_set_hash
                 FROM candidate_managed_feed_catalog_head WHERE singleton
               ON CONFLICT(operation_id) DO NOTHING RETURNING catalog_id"#,
        )
        .bind(operation_id)
        .fetch_optional(&mut **tx)
        .await?;
        if let Some(catalog_id) = inserted {
            catalog_id
        } else if let Some(catalog_id) = sqlx::query_scalar(
            "SELECT catalog_id FROM candidate_operation_managed_feed_contracts WHERE operation_id=$1 FOR SHARE",
        )
        .bind(operation_id)
        .fetch_optional(&mut **tx)
        .await?
        {
            catalog_id
        } else {
            let blocked_reason = ManagedFeedBlockReason::CatalogUnavailable;
            return Ok(ManagedFeedAuthoritySelection {
                store_catalog_id: None,
                authority_manifest_hash: hash_json_on(
                    tx,
                    &json!({"status":"blocked","reason":blocked_reason.as_str()}),
                )
                .await?,
                blocked_reason: Some(blocked_reason),
            });
        }
    };

    let issue: Option<String> = sqlx::query_scalar(
        r#"WITH contract AS (
               SELECT contract.*,trust.trust_store_version,trust.trust_store_hash,
                      trust.key_revocation_epoch,trust.key_revocation_epoch_hash
                 FROM candidate_operation_managed_feed_contracts contract
                 LEFT JOIN candidate_managed_feed_trust_store_head trust ON trust.singleton
                WHERE contract.operation_id=$1 AND contract.catalog_id=$2
           )
           SELECT CASE
             WHEN EXISTS(SELECT 1 FROM contract WHERE trust_store_version IS NULL)
               THEN 'trust'
             WHEN EXISTS(
               SELECT 1 FROM contract c
                WHERE c.required_source_count<>5 OR c.required_member_count<5
                   OR c.required_member_count<>(SELECT COUNT(*) FROM candidate_managed_feed_catalog_members m WHERE m.catalog_id=c.catalog_id)
                   OR c.required_source_set_hash<>(SELECT tool_truth_sha256(to_jsonb(array_agg(DISTINCT m.source_kind ORDER BY m.source_kind))::TEXT) FROM candidate_managed_feed_catalog_members m WHERE m.catalog_id=c.catalog_id)
                   OR c.required_member_set_hash<>(SELECT tool_truth_sha256(to_jsonb(array_agg(m.member_hash ORDER BY m.ordinal))::TEXT) FROM candidate_managed_feed_catalog_members m WHERE m.catalog_id=c.catalog_id)
                   OR (SELECT array_agg(DISTINCT m.source_kind ORDER BY m.source_kind) FROM candidate_managed_feed_catalog_members m WHERE m.catalog_id=c.catalog_id)
                      <>ARRAY['cpe','cve','detection_rule','kev','vendor_advisory']::TEXT[])
               THEN 'exact'
             WHEN EXISTS(
               SELECT 1 FROM contract c
                WHERE c.required_member_count<>(
                    SELECT COUNT(*) FROM candidate_managed_feed_catalog_members expected
                    JOIN candidate_managed_feed_store_member_heads head
                      ON head.catalog_member_id=expected.catalog_member_id
                     AND head.catalog_id=expected.catalog_id
                    JOIN candidate_managed_feed_store_members member
                      ON member.store_member_id=head.store_member_id
                     AND member.catalog_member_id=head.catalog_member_id
                     AND member.catalog_id=head.catalog_id
                   WHERE expected.catalog_id=c.catalog_id))
               THEN 'store'
             WHEN EXISTS(
               SELECT 1 FROM contract c
               JOIN candidate_managed_feed_catalog_members expected ON expected.catalog_id=c.catalog_id
               JOIN candidate_managed_feed_store_member_heads head
                 ON head.catalog_member_id=expected.catalog_member_id AND head.catalog_id=expected.catalog_id
               JOIN candidate_managed_feed_store_members member
                 ON member.store_member_id=head.store_member_id AND member.catalog_member_id=head.catalog_member_id AND member.catalog_id=head.catalog_id
              WHERE member.feed_schema<>expected.schema_name
                 OR member.feed_version<>expected.schema_version)
               THEN 'schema'
             WHEN EXISTS(
               SELECT 1 FROM contract c
               JOIN candidate_managed_feed_catalog_members expected ON expected.catalog_id=c.catalog_id
               JOIN candidate_managed_feed_store_member_heads head
                 ON head.catalog_member_id=expected.catalog_member_id AND head.catalog_id=expected.catalog_id
               JOIN candidate_managed_feed_store_members member
                 ON member.store_member_id=head.store_member_id AND member.catalog_member_id=head.catalog_member_id AND member.catalog_id=head.catalog_id
              WHERE member.effective_valid_until<=statement_timestamp()
                 OR member.published_at>statement_timestamp()
                 OR member.host_ingested_at>statement_timestamp())
               THEN 'stale'
             WHEN EXISTS(
               SELECT 1 FROM contract c
               JOIN candidate_managed_feed_catalog_members expected ON expected.catalog_id=c.catalog_id
               JOIN candidate_managed_feed_store_member_heads head
                 ON head.catalog_member_id=expected.catalog_member_id AND head.catalog_id=expected.catalog_id
               JOIN candidate_managed_feed_store_members member
                 ON member.store_member_id=head.store_member_id AND member.catalog_member_id=head.catalog_member_id AND member.catalog_id=head.catalog_id
               LEFT JOIN candidate_managed_feed_signer_keys signer
                 ON signer.trust_store_version=c.trust_store_version
                AND signer.trust_store_hash=c.trust_store_hash
                AND signer.key_revocation_epoch=c.key_revocation_epoch
                AND signer.key_revocation_epoch_hash=c.key_revocation_epoch_hash
                AND signer.signer_id=member.signer_id
                AND signer.signer_key_id=member.signer_key_id
                AND signer.signature_algorithm=member.signature_algorithm
                AND signer.key_member_hash=member.signer_key_member_hash
                AND signer.revoked=FALSE
              WHERE signer.signer_key_member_id IS NULL)
               THEN 'signature'
             ELSE NULL END FROM contract"#,
    )
    .bind(operation_id)
    .bind(catalog_id)
    .fetch_one(&mut **tx)
    .await?;
    if let Some(issue) = issue {
        let blocked_reason = match issue.as_str() {
            "store" => ManagedFeedBlockReason::StoreUnavailable,
            "trust" => ManagedFeedBlockReason::TrustStoreDrift,
            "schema" => ManagedFeedBlockReason::SchemaVersionUnsupported,
            "stale" => ManagedFeedBlockReason::FeedStale,
            "signature" => ManagedFeedBlockReason::SignatureInvalid,
            _ => ManagedFeedBlockReason::ExactAuthorityInvalid,
        };
        return Ok(ManagedFeedAuthoritySelection {
            store_catalog_id: None,
            authority_manifest_hash: hash_json_on(
                tx,
                &json!({"catalog_id":catalog_id,"status":"blocked","reason":blocked_reason.as_str()}),
            )
            .await?,
            blocked_reason: Some(blocked_reason),
        });
    }
    let manifest: Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
                 'operation_id',contract.operation_id,'catalog_id',contract.catalog_id,
                 'catalog_hash',contract.catalog_hash,
                 'trust_policy_hash',contract.trust_policy_hash,
                 'required_member_set_hash',contract.required_member_set_hash,
                 'trust_store_hash',trust.trust_store_hash,
                 'key_revocation_epoch_hash',trust.key_revocation_epoch_hash,
                 'store_member_set_hash',tool_truth_sha256(to_jsonb(array_agg(member.member_hash ORDER BY expected.ordinal))::TEXT))
              FROM candidate_operation_managed_feed_contracts contract
              JOIN candidate_managed_feed_catalog_members expected USING(catalog_id)
              JOIN candidate_managed_feed_store_member_heads head
                ON head.catalog_member_id=expected.catalog_member_id AND head.catalog_id=expected.catalog_id
              JOIN candidate_managed_feed_store_members member
                ON member.store_member_id=head.store_member_id AND member.catalog_member_id=head.catalog_member_id AND member.catalog_id=head.catalog_id
              JOIN candidate_managed_feed_trust_store_head trust ON trust.singleton
             WHERE contract.operation_id=$1 AND contract.catalog_id=$2
             GROUP BY contract.operation_id,contract.catalog_id,contract.catalog_hash,
                      contract.trust_policy_hash,contract.required_member_set_hash,
                      trust.trust_store_hash,trust.key_revocation_epoch_hash"#,
    )
    .bind(operation_id)
    .bind(catalog_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(ManagedFeedAuthoritySelection {
        store_catalog_id: Some(catalog_id),
        authority_manifest_hash: hash_json_on(tx, &manifest).await?,
        blocked_reason: None,
    })
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
    lock_and_require_registry_canonical_on(tx, input.operation_id).await?;
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

    let managed_feed =
        select_managed_feed_authority_on(tx, input.operation_id, input.organization_id).await?;
    let disposition = if checked.is_all_fresh() && managed_feed.store_catalog_id.is_some() {
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
            "feed_authority_manifest_hash":managed_feed.authority_manifest_hash,
            "feed_authority_blocked_reason":managed_feed.blocked_reason.map(ManagedFeedBlockReason::as_str),
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
    if let Some(catalog_id) = managed_feed.store_catalog_id {
        persist_managed_feed_store_authority_on(tx, snapshot_id, input.operation_id, catalog_id)
            .await?;
    } else {
        persist_unavailable_feed_authority_on(
            tx,
            snapshot_id,
            managed_feed
                .blocked_reason
                .unwrap_or(ManagedFeedBlockReason::CatalogUnavailable),
        )
        .await?;
    }
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

#[allow(clippy::too_many_arguments)]
async fn persist_frozen_candidate_input_on(
    tx: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
    stable_input_key: &str,
    source_kind: &str,
    source_ref: &str,
    source_authority_hash: &str,
    subject_kind_at_time: &str,
    subject_identity_hash: &str,
    frozen_body: &Value,
) -> Result<String> {
    const CHUNK_BYTES: usize = 16 * 1024;
    const MAX_CHUNKS: usize = 64;
    const MAX_SOURCE_BYTES: usize = 1024 * 1024;
    const CHUNKING_VERSION: &str = "1";
    const REDACTION_VERSION: &str = "1";

    let source_ref_hash = hash_json_on(
        tx,
        &json!({"source_ref":source_ref,"source_authority_hash":source_authority_hash}),
    )
    .await?;
    let source_bytes = serde_json::to_vec(frozen_body)?;
    let source_byte_count = i64::try_from(source_bytes.len())
        .map_err(|_| conflict("CANDIDATE_SNAPSHOT_SOURCE_OVERSIZE"))?;
    let source_content_hash = hash_json_on(tx, frozen_body).await?;
    let disposition = if source_bytes.len() > MAX_SOURCE_BYTES
        || source_bytes.len().div_ceil(CHUNK_BYTES) > MAX_CHUNKS
    {
        "blocked_oversize"
    } else if source_bytes.is_empty() {
        "source_empty"
    } else {
        "complete"
    };
    let input_hash = hash_json_on(
        tx,
        &json!({
            "stable_input_key":stable_input_key,"source_kind":source_kind,
            "source_ref_hash":source_ref_hash,"source_content_hash":source_content_hash,
            "source_byte_count":source_byte_count,"subject_kind_at_time":subject_kind_at_time,
            "subject_identity_hash":subject_identity_hash,"chunking_disposition":disposition,
        }),
    )
    .await?;
    let snapshot_input_id = Uuid::new_v5(&snapshot_id, stable_input_key.as_bytes());
    sqlx::query(
        r#"INSERT INTO candidate_analysis_snapshot_inputs(
               snapshot_input_id,snapshot_id,stable_input_key,source_kind,source_ref,
               source_ref_hash,source_content_hash,source_byte_count,subject_kind_at_time,
               subject_identity_hash,server_chunking_disposition,input_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(snapshot_input_id)
    .bind(snapshot_id)
    .bind(stable_input_key)
    .bind(source_kind)
    .bind(source_ref)
    .bind(&source_ref_hash)
    .bind(&source_content_hash)
    .bind(source_byte_count)
    .bind(subject_kind_at_time)
    .bind(subject_identity_hash)
    .bind(disposition)
    .bind(&input_hash)
    .execute(&mut **tx)
    .await?;

    let chunk_census_id = Uuid::new_v5(&snapshot_input_id, b"candidate_chunk_census.v1");
    let mut chunk_hashes = Vec::new();
    let mut prepared_chunks = Vec::new();
    if disposition == "complete" {
        for (ordinal, bytes) in source_bytes.chunks(CHUNK_BYTES).enumerate() {
            let start = ordinal * CHUNK_BYTES;
            let end = start + bytes.len();
            let mut encoded = String::with_capacity(bytes.len() * 2);
            for byte in bytes {
                use std::fmt::Write;
                write!(encoded, "{byte:02x}").expect("writing to String cannot fail");
            }
            let immutable_body = json!({
                "schema":"candidate_redacted_chunk.v1","encoding":"hex",
                "canonical_source_fragment":encoded,"instruction_authority":false,
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
    }
    let chunk_member_set_hash = hash_text_array_on(tx, &chunk_hashes).await?;
    let census_hash = hash_json_on(
        tx,
        &json!({
            "snapshot_input_id":snapshot_input_id,"source_content_hash":source_content_hash,
            "source_byte_count":source_byte_count,"disposition":disposition,
            "chunk_member_set_hash":chunk_member_set_hash,
            "chunking":CHUNKING_VERSION,"redaction":REDACTION_VERSION,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_input_chunk_censuses(
               chunk_census_id,snapshot_input_id,snapshot_id,chunking_contract_version,
               redaction_contract_version,source_content_hash,source_byte_count,
               disposition,chunk_count,chunk_member_set_hash,census_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
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
                   redaction_contract_version,chunk_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7,'candidate_redacted_chunk.v1',$8,$9,$10,$11,$12)"#,
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
    Ok(input_hash)
}

async fn freeze_ready_snapshot_inputs_and_attempt_on(
    tx: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
    bundle_members: &[BundleMemberRow],
) -> Result<()> {
    const CHUNK_BYTES: usize = 16 * 1024;
    const MAX_CHUNKS: usize = 64;
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
        let disposition = if source_bytes.len() > MAX_SOURCE_BYTES
            || source_bytes.len().div_ceil(CHUNK_BYTES) > MAX_CHUNKS
        {
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
    let source_set_rows: Vec<(String, String, String, String)> = sqlx::query_as(
        r#"SELECT source_set.source_kind,member.source_identity,
                  member.source_hash,member.member_hash
             FROM candidate_analysis_snapshot_source_sets source_set
             JOIN candidate_analysis_snapshot_source_set_members member USING(source_set_id,snapshot_id)
            WHERE source_set.snapshot_id=$1
              AND source_set.source_kind NOT IN ('tool_truth_bundle','managed_knowledge_feed')
            ORDER BY source_set.source_kind,member.ordinal"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    for (source_kind, source_identity, source_hash, member_hash) in source_set_rows {
        let stable_key = format!("source-set:{source_kind}:{source_identity}");
        let source_ref = format!("candidate_snapshot_source_member:{member_hash}");
        let body = json!({
            "schema":"candidate_snapshot_source_member.v1","instruction_authority":false,
            "source_kind":source_kind,"source_identity":source_identity,
            "source_hash":source_hash,"member_hash":member_hash,
        });
        input_hashes.push(
            persist_frozen_candidate_input_on(
                tx,
                snapshot_id,
                &stable_key,
                &source_kind,
                &source_ref,
                &member_hash,
                &source_kind,
                &source_hash,
                &body,
            )
            .await?,
        );
    }
    let feed_rows: Vec<(Uuid, String, String, String, Value)> = sqlx::query_as(
        r#"SELECT member.feed_snapshot_member_id,expected.source_kind,
                  expected.source_identity,member.member_hash,member.immutable_feed_body
             FROM candidate_analysis_knowledge_feed_snapshot_members member
             JOIN candidate_analysis_knowledge_feed_denominator_members expected
               ON expected.expected_member_id=member.expected_member_id
            WHERE member.snapshot_id=$1 AND member.disposition='current'
            ORDER BY member.ordinal"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    for (feed_member_id, feed_kind, source_identity, member_hash, feed_body) in feed_rows {
        let stable_key = format!("knowledge-feed:{feed_kind}:{source_identity}");
        let source_ref = format!("candidate_feed_snapshot_member:{feed_member_id}");
        let body = json!({
            "schema":"candidate_knowledge_feed_input.v1","instruction_authority":false,
            "feed_snapshot_member_id":feed_member_id,"feed_kind":feed_kind,
            "source_identity":source_identity,"member_hash":member_hash,"body":feed_body,
        });
        input_hashes.push(
            persist_frozen_candidate_input_on(
                tx,
                snapshot_id,
                &stable_key,
                "managed_knowledge_feed",
                &source_ref,
                &member_hash,
                "knowledge_feed",
                &member_hash,
                &body,
            )
            .await?,
        );
    }
    let product_rows: Vec<(Uuid, String, String, String, Value)> = sqlx::query_as(
        r#"SELECT product_member_id,subject_kind,subject_identity_hash,member_hash,
                  jsonb_build_object(
                    'product_identity',product_identity,'cpe_candidates',cpe_candidates,
                    'observed_version',observed_version,'disposition',disposition)
             FROM candidate_analysis_product_version_census_members
            WHERE snapshot_id=$1 ORDER BY ordinal"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    for (product_member_id, subject_kind, subject_hash, member_hash, product_body) in product_rows {
        let stable_key = format!("application-product:{product_member_id}");
        let source_ref = format!("candidate_product_version_member:{product_member_id}");
        let body = json!({
            "schema":"candidate_application_context_input.v1","instruction_authority":false,
            "product_member_id":product_member_id,"member_hash":member_hash,"body":product_body,
        });
        input_hashes.push(
            persist_frozen_candidate_input_on(
                tx,
                snapshot_id,
                &stable_key,
                "application_context",
                &source_ref,
                &member_hash,
                &subject_kind,
                &subject_hash,
                &body,
            )
            .await?,
        );
    }
    let match_rows: Vec<(Uuid, String, String, Value)> = sqlx::query_as(
        r#"SELECT match.match_member_id,product.subject_identity_hash,match.member_hash,
                  jsonb_build_object(
                    'product_member_id',match.product_member_id,
                    'feed_snapshot_member_id',match.feed_snapshot_member_id,
                    'disposition',match.disposition,'matched_entry_kind',match.matched_entry_kind,
                    'matched_entry_id',match.matched_entry_id,
                    'matched_entry_version',match.matched_entry_version,
                    'matched_range',match.matched_range,'matched_entry_hash',match.matched_entry_hash)
             FROM candidate_analysis_feed_match_census_members match
             JOIN candidate_analysis_product_version_census_members product
               ON product.product_member_id=match.product_member_id
            WHERE match.snapshot_id=$1 AND match.disposition='matched'
            ORDER BY match.ordinal"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    for (match_member_id, subject_hash, member_hash, match_body) in match_rows {
        let stable_key = format!("knowledge-match:{match_member_id}");
        let source_ref = format!("candidate_feed_match_member:{match_member_id}");
        let body = json!({
            "schema":"candidate_knowledge_match_input.v1","instruction_authority":false,
            "feed_match_member_id":match_member_id,"member_hash":member_hash,"body":match_body,
        });
        input_hashes.push(
            persist_frozen_candidate_input_on(
                tx,
                snapshot_id,
                &stable_key,
                "knowledge_signal",
                &source_ref,
                &member_hash,
                "application_subject",
                &subject_hash,
                &body,
            )
            .await?,
        );
    }
    let incomplete_disposition: Option<String> = sqlx::query_scalar(
        r#"SELECT server_chunking_disposition
             FROM candidate_analysis_snapshot_inputs
            WHERE snapshot_id=$1 AND server_chunking_disposition<>'complete'
            ORDER BY stable_input_key LIMIT 1"#,
    )
    .bind(snapshot_id)
    .fetch_optional(&mut **tx)
    .await?;
    if incomplete_disposition.as_deref() == Some("blocked_oversize") {
        return Err(conflict("CANDIDATE_SNAPSHOT_SOURCE_OVERSIZE"));
    }
    if incomplete_disposition.is_some() {
        return Err(conflict("CANDIDATE_SNAPSHOT_SOURCE_INCOMPLETE"));
    }
    if input_hashes.len() > 300 {
        return Err(conflict("CANDIDATE_SNAPSHOT_INPUT_CAP_EXCEEDED"));
    }
    let input_set_hash = hash_text_array_on(tx, &input_hashes).await?;
    let (operation_id, organization_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT operation_id,organization_id FROM candidate_analysis_snapshots WHERE snapshot_id=$1",
    )
    .bind(snapshot_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
    let attack_class_digest =
        hash_json_on(tx, &candidate_attack_class_catalog_manifest_v1()).await?;
    let trust_boundaries: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT DISTINCT subject_kind_at_time,subject_identity_hash
             FROM candidate_analysis_snapshot_inputs
            WHERE snapshot_id=$1 ORDER BY subject_kind_at_time,subject_identity_hash"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    let trust_boundary_digest = hash_json_on(
        tx,
        &json!({
            "contract":"trust_boundary.v1","version":1,
            "boundaries":trust_boundaries.iter().map(|row|json!({
                "identity":row.0,"hash":row.1,
            })).collect::<Vec<_>>(),
        }),
    )
    .await?;
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
                    'trust_boundary.v1',$7,'coverage_sampling.v1',$8,1)"#,
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

pub(crate) const CANDIDATE_ATTACK_CLASS_CATALOG_V1: [(&str, i32); 8] = [
    ("authentication", 1),
    ("authorization", 1),
    ("business_logic", 1),
    ("configuration", 1),
    ("data_exposure", 1),
    ("injection", 1),
    ("availability", 1),
    ("supply_chain", 1),
];

pub(crate) fn candidate_attack_class_catalog_manifest_v1() -> Value {
    json!({
        "contract":"attack_class.v1","version":1,
        "members":CANDIDATE_ATTACK_CLASS_CATALOG_V1.iter().map(|(id,version)|json!({
            "attack_class_id":id,"attack_class_version":version,
        })).collect::<Vec<_>>(),
    })
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

#[derive(Debug)]
struct FrozenProductVersionDraft {
    subject_kind: String,
    subject_identity_hash: String,
    product_identity: String,
    cpe_candidates: Vec<String>,
    observed_version: Option<String>,
    disposition: &'static str,
    member_hash: String,
}

#[derive(Debug)]
struct FrozenFeedMatchDraft {
    product_member_id: Uuid,
    feed_snapshot_member_id: Uuid,
    disposition: &'static str,
    matched_entry_kind: Option<String>,
    matched_entry_id: Option<String>,
    matched_entry_version: Option<String>,
    matched_range: Option<String>,
    matched_entry_hash: Option<String>,
    member_hash: String,
}

fn required_product_text<'a>(product: &'a Value, field: &'static str) -> Result<&'a str> {
    product
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| conflict(AUTHORITY_MISMATCH))
}

fn valid_authority_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

async fn persist_managed_feed_store_authority_on(
    tx: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
    operation_id: Uuid,
    catalog_id: Uuid,
) -> Result<()> {
    let authority = sqlx::query(
        r#"SELECT contract.catalog_version,contract.catalog_hash,
                  contract.trust_policy_id,contract.trust_policy_version,
                  contract.trust_policy_hash,contract.signature_algorithm_allowlist_hash,
                  contract.required_source_count,contract.required_source_set_hash,
                  contract.required_member_count,contract.required_member_set_hash,
                  trust.trust_store_version,trust.trust_store_hash,
                  trust.key_revocation_epoch,trust.key_revocation_epoch_hash
             FROM candidate_operation_managed_feed_contracts contract
             JOIN candidate_managed_feed_trust_store_head trust ON trust.singleton
            WHERE contract.operation_id=$1 AND contract.catalog_id=$2
            FOR SHARE OF contract,trust"#,
    )
    .bind(operation_id)
    .bind(catalog_id)
    .fetch_one(&mut **tx)
    .await?;
    let catalog_hash: String = authority.try_get("catalog_hash")?;
    let trust_policy_hash: String = authority.try_get("trust_policy_hash")?;
    let trust_store_hash: String = authority.try_get("trust_store_hash")?;
    let key_revocation_epoch_hash: String = authority.try_get("key_revocation_epoch_hash")?;
    let required_member_set_hash: String = authority.try_get("required_member_set_hash")?;
    let denominator_hash = hash_json_on(
        tx,
        &json!({
            "catalog_hash":catalog_hash,
            "trust_policy_hash":trust_policy_hash,
            "trust_store_hash":trust_store_hash,
            "key_revocation_epoch_hash":key_revocation_epoch_hash,
            "required_member_set_hash":required_member_set_hash,
        }),
    )
    .await?;
    let denominator_id = Uuid::new_v5(&snapshot_id, b"candidate_feed_denominator.v1");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_knowledge_feed_denominators(
               denominator_id,snapshot_id,catalog_id,catalog_version,catalog_hash,
               trust_policy_id,trust_policy_version,trust_policy_hash,
               signature_algorithm_allowlist_hash,trust_store_version,trust_store_hash,
               key_revocation_epoch,key_revocation_epoch_hash,required_source_count,
               required_source_set_hash,required_member_count,required_member_set_hash,
               denominator_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#,
    )
    .bind(denominator_id)
    .bind(snapshot_id)
    .bind(catalog_id)
    .bind(authority.try_get::<i32, _>("catalog_version")?)
    .bind(&catalog_hash)
    .bind(authority.try_get::<Uuid, _>("trust_policy_id")?)
    .bind(authority.try_get::<i32, _>("trust_policy_version")?)
    .bind(&trust_policy_hash)
    .bind(authority.try_get::<String, _>("signature_algorithm_allowlist_hash")?)
    .bind(authority.try_get::<i64, _>("trust_store_version")?)
    .bind(&trust_store_hash)
    .bind(authority.try_get::<i64, _>("key_revocation_epoch")?)
    .bind(&key_revocation_epoch_hash)
    .bind(authority.try_get::<i64, _>("required_source_count")?)
    .bind(authority.try_get::<String, _>("required_source_set_hash")?)
    .bind(authority.try_get::<i64, _>("required_member_count")?)
    .bind(&required_member_set_hash)
    .bind(&denominator_hash)
    .execute(&mut **tx)
    .await?;

    let rows = sqlx::query(
        r#"SELECT expected.catalog_member_id,expected.ordinal,expected.source_kind,
                  expected.source_identity,expected.schema_name,expected.schema_version,
                  expected.member_hash AS expected_member_hash,
                  member.store_member_id,member.feed_id,member.source_id,member.feed_schema,
                  member.feed_version,member.published_at,member.host_ingested_at,
                  member.effective_valid_until,member.content_hash,member.signed_manifest_hash,
                  member.signer_id,member.signer_key_id,member.signature_algorithm,
                  member.signature_verification_receipt_hash,member.signer_key_member_hash,
                  member.provenance,member.age_policy_version,member.age_policy_digest,
                  member.immutable_feed_body,member.member_hash AS feed_member_hash
             FROM candidate_managed_feed_catalog_members expected
             JOIN candidate_managed_feed_store_member_heads head
               ON head.catalog_member_id=expected.catalog_member_id AND head.catalog_id=expected.catalog_id
             JOIN candidate_managed_feed_store_members member
               ON member.store_member_id=head.store_member_id AND member.catalog_member_id=head.catalog_member_id AND member.catalog_id=head.catalog_id
            WHERE expected.catalog_id=$1 ORDER BY expected.ordinal
            FOR SHARE OF expected,member"#,
    )
    .bind(catalog_id)
    .fetch_all(&mut **tx)
    .await?;
    let feed_snapshot_id = Uuid::new_v5(&snapshot_id, b"candidate_feed_snapshot.v1");
    let feed_hashes = rows
        .iter()
        .map(|row| row.try_get::<String, _>("feed_member_hash"))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let feed_member_set_hash = hash_text_array_on(tx, &feed_hashes).await?;
    let feed_snapshot_hash = hash_json_on(
        tx,
        &json!({
            "denominator_hash":denominator_hash,
            "feed_member_set_hash":feed_member_set_hash,
            "trust_store_hash":trust_store_hash,
            "key_revocation_epoch_hash":key_revocation_epoch_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_knowledge_feed_snapshots(
               feed_snapshot_id,snapshot_id,denominator_id,trust_policy_hash,
               trust_store_hash,key_revocation_epoch,member_count,member_set_hash,
               feed_snapshot_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(feed_snapshot_id)
    .bind(snapshot_id)
    .bind(denominator_id)
    .bind(&trust_policy_hash)
    .bind(&trust_store_hash)
    .bind(authority.try_get::<i64, _>("key_revocation_epoch")?)
    .bind(i64::try_from(rows.len()).unwrap_or(i64::MAX))
    .bind(&feed_member_set_hash)
    .bind(&feed_snapshot_hash)
    .execute(&mut **tx)
    .await?;
    for row in rows {
        let source_identity: String = row.try_get("source_identity")?;
        let expected_member_id = Uuid::new_v5(&denominator_id, source_identity.as_bytes());
        sqlx::query(
            r#"INSERT INTO candidate_analysis_knowledge_feed_denominator_members(
                   expected_member_id,denominator_id,snapshot_id,ordinal,source_kind,
                   source_identity,schema_name,minimum_schema_version,member_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
        )
        .bind(expected_member_id)
        .bind(denominator_id)
        .bind(snapshot_id)
        .bind(row.try_get::<i32, _>("ordinal")?)
        .bind(row.try_get::<String, _>("source_kind")?)
        .bind(&source_identity)
        .bind(row.try_get::<String, _>("schema_name")?)
        .bind(row.try_get::<i32, _>("schema_version")?)
        .bind(row.try_get::<String, _>("expected_member_hash")?)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO candidate_analysis_knowledge_feed_snapshot_members(
                   feed_snapshot_member_id,feed_snapshot_id,snapshot_id,denominator_id,
                   expected_member_id,ordinal,feed_id,source_id,feed_schema,feed_version,
                   published_at,host_ingested_at,content_hash,signed_manifest_hash,signer_id,
                   signer_key_id,signature_algorithm,signature_verification_receipt_hash,
                   signer_key_member_hash,provenance,age_policy_version,age_policy_digest,
                   computed_age_seconds,effective_valid_until,disposition,immutable_feed_body,
                   member_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                      $18,$19,$20,$21,$22,GREATEST(0,EXTRACT(EPOCH FROM statement_timestamp()-$11::TIMESTAMPTZ)::BIGINT),
                      $23,'current',$24,$25)"#,
        )
        .bind(Uuid::new_v5(&feed_snapshot_id, source_identity.as_bytes()))
        .bind(feed_snapshot_id)
        .bind(snapshot_id)
        .bind(denominator_id)
        .bind(expected_member_id)
        .bind(row.try_get::<i32, _>("ordinal")?)
        .bind(row.try_get::<String, _>("feed_id")?)
        .bind(row.try_get::<String, _>("source_id")?)
        .bind(row.try_get::<String, _>("feed_schema")?)
        .bind(row.try_get::<i32, _>("feed_version")?.to_string())
        .bind(row.try_get::<DateTime<Utc>, _>("published_at")?)
        .bind(row.try_get::<DateTime<Utc>, _>("host_ingested_at")?)
        .bind(row.try_get::<String, _>("content_hash")?)
        .bind(row.try_get::<String, _>("signed_manifest_hash")?)
        .bind(row.try_get::<String, _>("signer_id")?)
        .bind(row.try_get::<String, _>("signer_key_id")?)
        .bind(row.try_get::<String, _>("signature_algorithm")?)
        .bind(row.try_get::<String, _>("signature_verification_receipt_hash")?)
        .bind(row.try_get::<String, _>("signer_key_member_hash")?)
        .bind(row.try_get::<Value, _>("provenance")?)
        .bind(row.try_get::<String, _>("age_policy_version")?)
        .bind(row.try_get::<String, _>("age_policy_digest")?)
        .bind(row.try_get::<DateTime<Utc>, _>("effective_valid_until")?)
        .bind(row.try_get::<Value, _>("immutable_feed_body")?)
        .bind(row.try_get::<String, _>("feed_member_hash")?)
        .execute(&mut **tx)
        .await?;
    }

    let application_model_hash: String = sqlx::query_scalar(
        "SELECT semantic_authority_bundle_hash FROM candidate_analysis_snapshots WHERE snapshot_id=$1",
    )
    .bind(snapshot_id)
    .fetch_one(&mut **tx)
    .await?;
    let product_census_id = Uuid::new_v5(&snapshot_id, b"candidate_product_census.v1");
    let typed_landings: Vec<Value> = sqlx::query_scalar(
        r#"SELECT receipt.typed_landing
              FROM candidate_analysis_snapshots snapshot
              JOIN tool_truth_authority_bundle_members bundle
                ON bundle.bundle_seal_id=snapshot.tool_truth_authority_bundle_seal_id
              JOIN tool_truth_authority_set_members set_member
                ON set_member.authority_set_id=bundle.authority_set_seal_id
              JOIN capability_execution_receipts receipt ON receipt.id=set_member.receipt_id
             WHERE snapshot.snapshot_id=$1
             ORDER BY bundle.ordinal,set_member.ordinal"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut products =
        BTreeMap::<(String, String, String), (BTreeSet<String>, BTreeSet<String>)>::new();
    for landing in typed_landings {
        let Some(application_products) = landing.get("application_products") else {
            continue;
        };
        let application_products = application_products
            .as_array()
            .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
        for product in application_products {
            let subject_kind = required_product_text(product, "subject_kind")?.to_owned();
            let subject_identity_hash =
                required_product_text(product, "subject_identity_hash")?.to_owned();
            if !valid_authority_hash(&subject_identity_hash) {
                return Err(conflict(AUTHORITY_MISMATCH));
            }
            let product_identity = required_product_text(product, "product_identity")?.to_owned();
            let cpe_candidates = product
                .get("cpe_candidates")
                .and_then(Value::as_array)
                .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
            let entry = products
                .entry((subject_kind, subject_identity_hash, product_identity))
                .or_default();
            for cpe in cpe_candidates {
                let cpe = cpe
                    .as_str()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
                entry.0.insert(cpe.to_owned());
            }
            if let Some(version) = product.get("observed_version") {
                if !version.is_null() {
                    let version = version
                        .as_str()
                        .filter(|value| !value.trim().is_empty())
                        .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
                    entry.1.insert(version.to_owned());
                }
            }
        }
    }
    let mut product_drafts = Vec::with_capacity(products.len());
    for ((subject_kind, subject_identity_hash, product_identity), (cpes, versions)) in products {
        let disposition = match versions.len() {
            0 => "unknown",
            1 => "known",
            _ => "conflicting",
        };
        let observed_version =
            (disposition == "known").then(|| versions.iter().next().expect("one version").clone());
        let cpe_candidates = cpes.into_iter().collect::<Vec<_>>();
        let member_hash = hash_json_on(
            tx,
            &json!({
                "subject_kind":subject_kind,
                "subject_identity_hash":subject_identity_hash,
                "product_identity":product_identity,
                "cpe_candidates":cpe_candidates,
                "observed_version":observed_version,
                "disposition":disposition,
            }),
        )
        .await?;
        product_drafts.push(FrozenProductVersionDraft {
            subject_kind,
            subject_identity_hash,
            product_identity,
            cpe_candidates,
            observed_version,
            disposition,
            member_hash,
        });
    }
    let product_hashes = product_drafts
        .iter()
        .map(|product| product.member_hash.clone())
        .collect::<Vec<_>>();
    let product_set_hash = hash_text_array_on(tx, &product_hashes).await?;
    let product_census_hash = hash_json_on(
        tx,
        &json!({
            "application_model_authority_hash":application_model_hash,
            "product_count":product_drafts.len(),
            "product_set_hash":product_set_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_product_version_censuses(
               product_census_id,snapshot_id,application_model_authority_hash,
               product_count,product_set_hash,census_hash)
           VALUES($1,$2,$3,$4,$5,$6)"#,
    )
    .bind(product_census_id)
    .bind(snapshot_id)
    .bind(application_model_hash)
    .bind(i64::try_from(product_drafts.len()).unwrap_or(i64::MAX))
    .bind(&product_set_hash)
    .bind(&product_census_hash)
    .execute(&mut **tx)
    .await?;
    let mut persisted_products = Vec::with_capacity(product_drafts.len());
    for (ordinal, product) in product_drafts.into_iter().enumerate() {
        let product_member_id = Uuid::new_v5(&product_census_id, product.member_hash.as_bytes());
        sqlx::query(
            r#"INSERT INTO candidate_analysis_product_version_census_members(
                   product_member_id,product_census_id,snapshot_id,ordinal,subject_kind,
                   subject_identity_hash,product_identity,cpe_candidates,observed_version,
                   disposition,member_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
        )
        .bind(product_member_id)
        .bind(product_census_id)
        .bind(snapshot_id)
        .bind(i32::try_from(ordinal).unwrap_or(i32::MAX))
        .bind(&product.subject_kind)
        .bind(&product.subject_identity_hash)
        .bind(&product.product_identity)
        .bind(json!(&product.cpe_candidates))
        .bind(&product.observed_version)
        .bind(product.disposition)
        .bind(&product.member_hash)
        .execute(&mut **tx)
        .await?;
        if product.disposition != "known" {
            let reason = if product.disposition == "conflicting" {
                "product_version_conflicting"
            } else {
                "product_version_unknown"
            };
            let obligation_hash = hash_json_on(
                tx,
                &json!({"product_member_id":product_member_id,"reason_code":reason}),
            )
            .await?;
            sqlx::query(
                r#"INSERT INTO candidate_analysis_enrichment_obligations(
                       obligation_id,snapshot_id,obligation_kind,product_member_id,
                       reason_code,affected_checklist_member_key,obligation_hash)
                   VALUES($1,$2,'product_version_enrichment',$3,$4,$5,$6)"#,
            )
            .bind(Uuid::new_v5(
                &product_member_id,
                b"candidate_product_version_enrichment.v1",
            ))
            .bind(snapshot_id)
            .bind(product_member_id)
            .bind(reason)
            .bind(format!("product:{product_member_id}"))
            .bind(obligation_hash)
            .execute(&mut **tx)
            .await?;
        }
        persisted_products.push((product_member_id, product));
    }

    #[derive(Debug)]
    struct FeedEntry {
        kind: String,
        id: String,
        version: Option<String>,
        cpe: String,
        affected_versions: BTreeSet<String>,
        matched_range: String,
        hash: String,
    }
    #[derive(Debug)]
    struct FeedForMatcher {
        member_id: Uuid,
        entries: Option<Vec<FeedEntry>>,
    }
    let frozen_feeds: Vec<(Uuid, String, Value)> = sqlx::query_as(
        r#"SELECT member.feed_snapshot_member_id,expected.source_kind,
                  member.immutable_feed_body
             FROM candidate_analysis_knowledge_feed_snapshot_members member
             JOIN candidate_analysis_knowledge_feed_denominator_members expected
               ON expected.expected_member_id=member.expected_member_id
            WHERE member.snapshot_id=$1 AND member.disposition='current'
            ORDER BY member.ordinal"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut feeds = Vec::with_capacity(frozen_feeds.len());
    for (member_id, source_kind, body) in frozen_feeds {
        let mut parsed = Vec::new();
        let entries = match body.get("entries") {
            None => Some(Vec::new()),
            Some(value) => value.as_array().map(|values| values.to_vec()),
        };
        let mut valid = entries.is_some();
        if let Some(entries) = entries {
            for entry in entries {
                let Some(object) = entry.as_object() else {
                    valid = false;
                    break;
                };
                let text = |field: &str| {
                    object
                        .get(field)
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty())
                        .map(ToOwned::to_owned)
                };
                let (Some(id), Some(cpe)) = (text("entry_id"), text("cpe")) else {
                    valid = false;
                    break;
                };
                let kind = text("entry_kind").unwrap_or_else(|| source_kind.clone());
                let matched_range = text("matched_range").unwrap_or_else(|| "exact".to_owned());
                let affected_versions = match object.get("affected_versions") {
                    None => BTreeSet::new(),
                    Some(Value::Array(values)) => {
                        let mut exact = BTreeSet::new();
                        for value in values {
                            let Some(value) = value
                                .as_str()
                                .filter(|candidate| !candidate.trim().is_empty())
                            else {
                                valid = false;
                                break;
                            };
                            exact.insert(value.to_owned());
                        }
                        exact
                    }
                    Some(_) => {
                        valid = false;
                        BTreeSet::new()
                    }
                };
                if !valid {
                    break;
                }
                parsed.push(FeedEntry {
                    kind,
                    id,
                    version: text("entry_version"),
                    cpe,
                    affected_versions,
                    matched_range,
                    hash: hash_json_on(tx, &entry).await?,
                });
            }
        }
        parsed.sort_by(|left, right| {
            (&left.kind, &left.id, &left.version, &left.hash).cmp(&(
                &right.kind,
                &right.id,
                &right.version,
                &right.hash,
            ))
        });
        if !valid {
            let obligation_hash = hash_json_on(
                tx,
                &json!({"feed_snapshot_member_id":member_id,"reason_code":"feed_matcher_input_invalid"}),
            )
            .await?;
            sqlx::query(
                r#"INSERT INTO candidate_analysis_enrichment_obligations(
                       obligation_id,snapshot_id,obligation_kind,feed_snapshot_member_id,
                       reason_code,affected_checklist_member_key,obligation_hash)
                   VALUES($1,$2,'feed_matcher_upgrade',$3,'feed_matcher_input_invalid',$4,$5)"#,
            )
            .bind(Uuid::new_v5(
                &member_id,
                b"candidate_feed_matcher_upgrade.v1",
            ))
            .bind(snapshot_id)
            .bind(member_id)
            .bind(format!("feed:{member_id}"))
            .bind(obligation_hash)
            .execute(&mut **tx)
            .await?;
        }
        feeds.push(FeedForMatcher {
            member_id,
            entries: valid.then_some(parsed),
        });
    }
    let matcher_contract_digest = hash_json_on(
        tx,
        &json!({
            "contract":"candidate_feed_matcher.v1",
            "cpe_match":"exact_string",
            "version_match":["affected_versions_exact","matched_range_wildcard"],
            "selection":"entry_kind_entry_id_entry_version_entry_hash_ascending_first",
        }),
    )
    .await?;
    let mut match_drafts = Vec::new();
    for (product_member_id, product) in &persisted_products {
        for feed in &feeds {
            let matched_entry = if product.disposition == "known" {
                let observed = product.observed_version.as_deref().expect("known version");
                feed.entries.as_ref().and_then(|entries| {
                    entries.iter().find(|entry| {
                        product.cpe_candidates.iter().any(|cpe| cpe == &entry.cpe)
                            && (entry.matched_range == "*"
                                || entry.affected_versions.contains(observed))
                    })
                })
            } else {
                None
            };
            let disposition = if feed.entries.is_none() {
                "feed_invalid"
            } else if product.disposition != "known" {
                "unknown_product_version"
            } else if matched_entry.is_some() {
                "matched"
            } else {
                "no_match"
            };
            let member_hash = hash_json_on(
                tx,
                &json!({
                    "product_member_id":product_member_id,
                    "feed_snapshot_member_id":feed.member_id,
                    "disposition":disposition,
                    "matched_entry_kind":matched_entry.map(|entry|&entry.kind),
                    "matched_entry_id":matched_entry.map(|entry|&entry.id),
                    "matched_entry_version":matched_entry.and_then(|entry|entry.version.as_ref()),
                    "matched_range":matched_entry.map(|entry|&entry.matched_range),
                    "matched_entry_hash":matched_entry.map(|entry|&entry.hash),
                    "matcher_contract_digest":matcher_contract_digest,
                }),
            )
            .await?;
            match_drafts.push(FrozenFeedMatchDraft {
                product_member_id: *product_member_id,
                feed_snapshot_member_id: feed.member_id,
                disposition,
                matched_entry_kind: matched_entry.map(|entry| entry.kind.clone()),
                matched_entry_id: matched_entry.map(|entry| entry.id.clone()),
                matched_entry_version: matched_entry.and_then(|entry| entry.version.clone()),
                matched_range: matched_entry.map(|entry| entry.matched_range.clone()),
                matched_entry_hash: matched_entry.map(|entry| entry.hash.clone()),
                member_hash,
            });
        }
    }
    let match_hashes = match_drafts
        .iter()
        .map(|member| member.member_hash.clone())
        .collect::<Vec<_>>();
    let match_member_set_hash = hash_text_array_on(tx, &match_hashes).await?;
    let match_census_hash = hash_json_on(
        tx,
        &json!({
            "matcher_contract_digest":matcher_contract_digest,
            "product_set_hash":product_set_hash,
            "feed_set_hash":feed_member_set_hash,
            "member_count":match_drafts.len(),
            "member_set_hash":match_member_set_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_feed_match_censuses(
               match_census_id,snapshot_id,product_census_id,feed_snapshot_id,
               matcher_contract_version,matcher_contract_digest,input_product_count,
               input_product_set_hash,input_feed_count,input_feed_set_hash,
               member_count,member_set_hash,census_hash)
           VALUES($1,$2,$3,$4,'candidate_feed_matcher.v1',$5,$6,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(Uuid::new_v5(
        &snapshot_id,
        b"candidate_feed_match_census.v1",
    ))
    .bind(snapshot_id)
    .bind(product_census_id)
    .bind(feed_snapshot_id)
    .bind(&matcher_contract_digest)
    .bind(i64::try_from(persisted_products.len()).unwrap_or(i64::MAX))
    .bind(&product_set_hash)
    .bind(i64::try_from(feed_hashes.len()).unwrap_or(i64::MAX))
    .bind(&feed_member_set_hash)
    .bind(i64::try_from(match_drafts.len()).unwrap_or(i64::MAX))
    .bind(&match_member_set_hash)
    .bind(&match_census_hash)
    .execute(&mut **tx)
    .await?;
    let match_census_id = Uuid::new_v5(&snapshot_id, b"candidate_feed_match_census.v1");
    for (ordinal, member) in match_drafts.into_iter().enumerate() {
        let match_member_id = Uuid::new_v5(&match_census_id, member.member_hash.as_bytes());
        sqlx::query(
            r#"INSERT INTO candidate_analysis_feed_match_census_members(
                   match_member_id,match_census_id,snapshot_id,product_member_id,
                   feed_snapshot_member_id,ordinal,disposition,matched_entry_kind,
                   matched_entry_id,matched_entry_version,matched_range,matched_entry_hash,
                   member_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#,
        )
        .bind(match_member_id)
        .bind(match_census_id)
        .bind(snapshot_id)
        .bind(member.product_member_id)
        .bind(member.feed_snapshot_member_id)
        .bind(i32::try_from(ordinal).unwrap_or(i32::MAX))
        .bind(member.disposition)
        .bind(member.matched_entry_kind)
        .bind(member.matched_entry_id)
        .bind(member.matched_entry_version)
        .bind(member.matched_range)
        .bind(member.matched_entry_hash)
        .bind(member.member_hash)
        .execute(&mut **tx)
        .await?;
    }
    persist_source_set_on(
        tx,
        snapshot_id,
        "managed_knowledge_feed",
        vec![(feed_snapshot_id.to_string(), feed_snapshot_hash)],
    )
    .await
}

async fn persist_unavailable_feed_authority_on(
    tx: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
    blocked_reason: ManagedFeedBlockReason,
) -> Result<()> {
    #[derive(Debug)]
    struct ExpectedMember {
        ordinal: i32,
        source_kind: String,
        source_identity: String,
        schema_name: String,
        schema_version: i32,
        member_hash: String,
    }
    const REQUIRED: [(&str, &str); 5] = [
        ("cve", "managed:cve"),
        ("cpe", "managed:cpe"),
        ("kev", "managed:kev"),
        ("vendor_advisory", "managed:vendor-advisory"),
        ("detection_rule", "managed:detection-rule"),
    ];
    let operation_id: Uuid = sqlx::query_scalar(
        "SELECT operation_id FROM candidate_analysis_snapshots WHERE snapshot_id=$1",
    )
    .bind(snapshot_id)
    .fetch_one(&mut **tx)
    .await?;
    let denominator_id = Uuid::new_v5(&snapshot_id, b"candidate_feed_denominator.v1");
    let contract = sqlx::query(
        r#"SELECT contract.catalog_id,contract.catalog_version,contract.catalog_hash,
                  contract.trust_policy_id,contract.trust_policy_version,
                  contract.trust_policy_hash,contract.signature_algorithm_allowlist_hash,
                  contract.required_source_count,contract.required_source_set_hash,
                  contract.required_member_count,contract.required_member_set_hash,
                  trust.trust_store_version,trust.trust_store_hash,
                  trust.key_revocation_epoch,trust.key_revocation_epoch_hash
             FROM candidate_operation_managed_feed_contracts contract
             LEFT JOIN candidate_managed_feed_trust_store_head trust ON trust.singleton
            WHERE contract.operation_id=$1 FOR SHARE OF contract"#,
    )
    .bind(operation_id)
    .fetch_optional(&mut **tx)
    .await?;
    let (
        catalog_id,
        catalog_version,
        catalog_hash,
        trust_policy_id,
        trust_policy_version,
        trust_policy_hash,
        signature_hash,
        trust_store_version,
        trust_store_hash,
        revocation_epoch,
        revocation_hash,
        required_source_count,
        required_source_set_hash,
        required_member_count,
        required_member_set_hash,
        expected,
    ) = if let Some(contract) = contract {
        let catalog_id: Uuid = contract.try_get("catalog_id")?;
        let expected = sqlx::query(
            r#"SELECT ordinal,source_kind,source_identity,schema_name,schema_version,
                      member_hash FROM candidate_managed_feed_catalog_members
                WHERE catalog_id=$1 ORDER BY ordinal FOR SHARE"#,
        )
        .bind(catalog_id)
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .map(|row| {
            Ok(ExpectedMember {
                ordinal: row.try_get("ordinal")?,
                source_kind: row.try_get("source_kind")?,
                source_identity: row.try_get("source_identity")?,
                schema_name: row.try_get("schema_name")?,
                schema_version: row.try_get("schema_version")?,
                member_hash: row.try_get("member_hash")?,
            })
        })
        .collect::<std::result::Result<Vec<_>, sqlx::Error>>()?;
        let fallback_trust_hash = hash_json_on(tx, &json!({"status":"not_installed"})).await?;
        let fallback_revocation_hash =
            hash_json_on(tx, &json!({"epoch":0,"status":"not_installed"})).await?;
        (
            catalog_id,
            contract.try_get("catalog_version")?,
            contract.try_get("catalog_hash")?,
            contract.try_get("trust_policy_id")?,
            contract.try_get("trust_policy_version")?,
            contract.try_get("trust_policy_hash")?,
            contract.try_get("signature_algorithm_allowlist_hash")?,
            contract
                .try_get::<Option<i64>, _>("trust_store_version")?
                .unwrap_or(1),
            contract
                .try_get::<Option<String>, _>("trust_store_hash")?
                .unwrap_or(fallback_trust_hash),
            contract
                .try_get::<Option<i64>, _>("key_revocation_epoch")?
                .unwrap_or(0),
            contract
                .try_get::<Option<String>, _>("key_revocation_epoch_hash")?
                .unwrap_or(fallback_revocation_hash),
            contract.try_get("required_source_count")?,
            contract.try_get("required_source_set_hash")?,
            contract.try_get("required_member_count")?,
            contract.try_get("required_member_set_hash")?,
            expected,
        )
    } else {
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
        let revocation_hash =
            hash_json_on(tx, &json!({"epoch":0,"status":"not_installed"})).await?;
        let mut expected = Vec::new();
        for (ordinal, (source_kind, source_identity)) in REQUIRED.into_iter().enumerate() {
            expected.push(ExpectedMember {
                ordinal: i32::try_from(ordinal).unwrap_or(i32::MAX),
                source_kind: source_kind.to_owned(),
                source_identity: source_identity.to_owned(),
                schema_name: "managed_knowledge_feed.v1".to_owned(),
                schema_version: 1,
                member_hash: hash_json_on(
                    tx,
                    &json!({
                        "source_kind":source_kind,"source_identity":source_identity,
                        "schema":"managed_knowledge_feed.v1","minimum_schema_version":1,
                    }),
                )
                .await?,
            });
        }
        let expected_hashes = expected
            .iter()
            .map(|member| member.member_hash.clone())
            .collect::<Vec<_>>();
        let required_member_set_hash = hash_text_array_on(tx, &expected_hashes).await?;
        let mut source_kinds = REQUIRED
            .iter()
            .map(|(kind, _)| (*kind).to_owned())
            .collect::<Vec<_>>();
        source_kinds.sort();
        let required_source_set_hash = hash_text_array_on(tx, &source_kinds).await?;
        (
            Uuid::new_v5(&snapshot_id, b"candidate_feed_catalog.v1"),
            1,
            catalog_hash,
            Uuid::new_v5(&snapshot_id, b"candidate_feed_trust_policy.v1"),
            1,
            trust_policy_hash,
            signature_hash,
            1,
            trust_store_hash,
            0,
            revocation_hash,
            5,
            required_source_set_hash,
            5,
            required_member_set_hash,
            expected,
        )
    };
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
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#,
    )
    .bind(denominator_id)
    .bind(snapshot_id)
    .bind(catalog_id)
    .bind(catalog_version)
    .bind(&catalog_hash)
    .bind(trust_policy_id)
    .bind(trust_policy_version)
    .bind(&trust_policy_hash)
    .bind(&signature_hash)
    .bind(trust_store_version)
    .bind(&trust_store_hash)
    .bind(revocation_epoch)
    .bind(&revocation_hash)
    .bind(required_source_count)
    .bind(&required_source_set_hash)
    .bind(required_member_count)
    .bind(&required_member_set_hash)
    .bind(&denominator_hash)
    .execute(&mut **tx)
    .await?;
    let feed_snapshot_id = Uuid::new_v5(&snapshot_id, b"candidate_feed_snapshot.v1");
    let expected_hashes = expected
        .iter()
        .map(|member| member.member_hash.clone())
        .collect::<Vec<_>>();
    let feed_snapshot_hash = hash_json_on(tx, &json!({
        "denominator_hash":denominator_hash,"members":expected_hashes,"disposition":"unavailable",
    })).await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_knowledge_feed_snapshots(
               feed_snapshot_id,snapshot_id,denominator_id,trust_policy_hash,
               trust_store_hash,key_revocation_epoch,member_count,member_set_hash,
               feed_snapshot_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(feed_snapshot_id)
    .bind(snapshot_id)
    .bind(denominator_id)
    .bind(&trust_policy_hash)
    .bind(&trust_store_hash)
    .bind(revocation_epoch)
    .bind(required_member_count)
    .bind(&required_member_set_hash)
    .bind(&feed_snapshot_hash)
    .execute(&mut **tx)
    .await?;
    for member in expected {
        let expected_member_id = Uuid::new_v5(&denominator_id, member.source_identity.as_bytes());
        sqlx::query(
            r#"INSERT INTO candidate_analysis_knowledge_feed_denominator_members(
                   expected_member_id,denominator_id,snapshot_id,ordinal,source_kind,
                   source_identity,schema_name,minimum_schema_version,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
        )
        .bind(expected_member_id)
        .bind(denominator_id)
        .bind(snapshot_id)
        .bind(member.ordinal)
        .bind(&member.source_kind)
        .bind(&member.source_identity)
        .bind(&member.schema_name)
        .bind(member.schema_version)
        .bind(&member.member_hash)
        .execute(&mut **tx)
        .await?;
        let feed_member_id = Uuid::new_v5(&feed_snapshot_id, member.source_identity.as_bytes());
        sqlx::query(
            r#"INSERT INTO candidate_analysis_knowledge_feed_snapshot_members(
                   feed_snapshot_member_id,feed_snapshot_id,snapshot_id,denominator_id,
                   expected_member_id,ordinal,feed_schema,age_policy_version,
                   age_policy_digest,disposition,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'unavailable',$10)"#,
        )
        .bind(feed_member_id)
        .bind(feed_snapshot_id)
        .bind(snapshot_id)
        .bind(denominator_id)
        .bind(expected_member_id)
        .bind(member.ordinal)
        .bind(&member.schema_name)
        .bind(member.schema_version.to_string())
        .bind(&trust_policy_hash)
        .bind(&member.member_hash)
        .execute(&mut **tx)
        .await?;
        let obligation_hash = hash_json_on(tx, &json!({
            "source_kind":&member.source_kind,"source_identity":&member.source_identity,"reason":blocked_reason.as_str(),
        })).await?;
        sqlx::query(
            r#"INSERT INTO candidate_analysis_enrichment_obligations(
                   obligation_id,snapshot_id,obligation_kind,feed_snapshot_member_id,
                   reason_code,affected_checklist_member_key,obligation_hash
               ) VALUES($1,$2,'feed_refresh',$3,$4,$5,$6)"#,
        )
        .bind(Uuid::new_v5(&feed_member_id, b"candidate_feed_refresh.v1"))
        .bind(snapshot_id)
        .bind(feed_member_id)
        .bind(blocked_reason.as_str())
        .bind(format!("feed:{}", member.source_kind))
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
           ) VALUES($1,$2,$3,$4,'candidate_feed_matcher.v1',$5,0,$6,$7,$8,0,$6,$9)"#,
    )
    .bind(match_census_id)
    .bind(snapshot_id)
    .bind(product_census_id)
    .bind(feed_snapshot_id)
    .bind(&trust_policy_hash)
    .bind(&empty_set_hash)
    .bind(required_member_count)
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
                  AND denominator.required_source_count=5
                  AND (SELECT array_agg(DISTINCT expected.source_kind ORDER BY expected.source_kind)
                         FROM candidate_analysis_knowledge_feed_denominator_members expected
                        WHERE expected.denominator_id=denominator.denominator_id)
                      =ARRAY['cpe','cve','detection_rule','kev','vendor_advisory']::TEXT[]
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
                            OR member.feed_schema<>expected.schema_name
                            OR member.feed_version IS NULL
                            OR member.feed_version<>expected.minimum_schema_version::TEXT
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
                  AND EXISTS(
                      SELECT 1
                        FROM candidate_analysis_product_version_censuses product
                        JOIN candidate_analysis_feed_match_censuses match USING(snapshot_id)
                       WHERE product.snapshot_id=$1
                         AND product.product_count=(
                             SELECT COUNT(*)
                               FROM candidate_analysis_product_version_census_members member
                              WHERE member.product_census_id=product.product_census_id)
                         AND product.product_set_hash=(
                             SELECT tool_truth_sha256(to_jsonb(COALESCE(
                                        array_agg(member.member_hash ORDER BY member.ordinal),
                                        ARRAY[]::TEXT[]))::TEXT)
                               FROM candidate_analysis_product_version_census_members member
                              WHERE member.product_census_id=product.product_census_id)
                         AND match.product_census_id=product.product_census_id
                         AND match.feed_snapshot_id=feed.feed_snapshot_id
                         AND match.input_product_count=product.product_count
                         AND match.input_product_set_hash=product.product_set_hash
                         AND match.input_feed_count=feed.member_count
                         AND match.input_feed_set_hash=feed.member_set_hash
                         AND match.member_count=product.product_count*feed.member_count
                         AND match.member_count=(
                             SELECT COUNT(*)
                               FROM candidate_analysis_feed_match_census_members member
                              WHERE member.match_census_id=match.match_census_id)
                         AND match.member_set_hash=(
                             SELECT tool_truth_sha256(to_jsonb(COALESCE(
                                        array_agg(member.member_hash ORDER BY member.ordinal),
                                        ARRAY[]::TEXT[]))::TEXT)
                               FROM candidate_analysis_feed_match_census_members member
                              WHERE member.match_census_id=match.match_census_id)
                         AND NOT EXISTS(
                             SELECT 1
                               FROM candidate_analysis_product_version_census_members product_member
                               CROSS JOIN candidate_analysis_knowledge_feed_snapshot_members feed_member
                              WHERE product_member.product_census_id=product.product_census_id
                                AND feed_member.feed_snapshot_id=feed.feed_snapshot_id
                                AND (SELECT COUNT(*)
                                       FROM candidate_analysis_feed_match_census_members match_member
                                      WHERE match_member.match_census_id=match.match_census_id
                                        AND match_member.product_member_id=product_member.product_member_id
                                        AND match_member.feed_snapshot_member_id=feed_member.feed_snapshot_member_id)<>1)
                  )
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
                  AND worker.work_item_id=item.id
                WHERE snapshot.snapshot_id=$4 AND snapshot.operation_id=$1
                  AND snapshot.scope_snapshot_id=$2 AND snapshot.organization_id=$3
                  AND snapshot.snapshot_status='sealed_ready'
                  AND attempt.analysis_attempt_id=$8
                  AND attempt.attempt_ordinal=$9
                  AND plan.row_version=$10 AND item.row_version=$11
                  AND worker.checkpoint_version=$12
                  AND worker.lease_token=$13 AND worker.attempt_epoch=$14
                  AND worker.lease_expires_at>statement_timestamp()
                  AND (
                      (item.status='running' AND worker.status='running')
                      OR (
                          item.kind='candidate_controller_final'
                          AND item.status='completed' AND worker.status='passed'
                          AND EXISTS (
                              SELECT 1 FROM candidate_analysis_provider_attempts receipt
                               WHERE receipt.analysis_attempt_id=attempt.analysis_attempt_id
                                 AND receipt.stage_work_item_id=item.id
                                 AND receipt.worker_run_id=worker.id
                              AND receipt.artifact_kind='controller_decision.v1'
                          )
                      )
                      OR (
                          item.kind='candidate_controller_dispatch'
                          AND item.status='completed' AND worker.status='passed'
                          AND EXISTS (
                              SELECT 1 FROM candidate_analysis_provider_attempts receipt
                               WHERE receipt.analysis_attempt_id=attempt.analysis_attempt_id
                                 AND receipt.stage_work_item_id=item.id
                                 AND receipt.worker_run_id=worker.id
                                 AND receipt.artifact_kind='controller_dispatch.v1'
                          )
                      )
                  )
                  AND NOT EXISTS (
                      SELECT 1 FROM candidate_analysis_attempt_state_events terminal
                       WHERE terminal.analysis_attempt_id=attempt.analysis_attempt_id
                         AND terminal.event_kind IN (
                             'superseded_missed_hypothesis','sealed','blocked'
                         )
                  )
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
    if !(1..=64).contains(&input.max_chunks) {
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
    let page_hash = candidate_chunk_page_hash_on(
        &mut tx,
        &CandidateChunkPageHashInput {
            analysis_attempt_id: input.fence.analysis_attempt_id,
            snapshot_id: input.fence.snapshot_id,
            snapshot_input_id: input.input_id,
            chunk_census_id: input.chunk_census_id,
            chunk_census_hash: input.chunk_census_hash.clone(),
            consumer_worker_run_id: input.fence.worker_run_id,
            first_ordinal: first,
            last_ordinal: last,
            ordered_chunk_hashes: chunks
                .iter()
                .map(|chunk| chunk.chunk_hash.clone())
                .collect(),
            source_size_bytes: input.source_size_bytes,
            chunking_contract_version: input.chunking_contract_version.clone(),
            redaction_contract_version: input.redaction_contract_version.clone(),
        },
    )
    .await?;
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
    if matches!(
        &input.artifact,
        AnalysisArtifactBodyRow::HypothesisProposal { .. }
            | AnalysisArtifactBodyRow::ProposalConflictReview { .. }
    ) {
        return Err(conflict(ARTIFACT_KIND_FORBIDDEN));
    }
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
    compilation_material_hash: String,
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
    let compilation_material_hash: String = sqlx::query_scalar(
        r#"SELECT material_hash
             FROM candidate_analysis_host_compilation_materials
            WHERE stable_compilation_request_id=$1
              AND analysis_attempt_id=$2 AND snapshot_id=$3
              AND operation_id=$4 AND organization_id=$5
              AND final_submitter_worker_run_id=$6
              AND mutation_set_hash=$7 AND claim_component_set_hash=$8
              AND verification_contract_set_hash=$9
              AND verification_plan_set_hash=$10
              AND generation_transition_set_hash=$11
            FOR SHARE"#,
    )
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
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
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
            "compilation_material_hash":compilation_material_hash,
        }),
    )
    .await?;
    let existing: Option<PersistedCandidateCompilationSeal> = sqlx::query_as(
        r#"SELECT compilation_seal_id,stable_compilation_request_id,
                      mutation_set_hash,claim_component_set_hash,
                      verification_contract_set_hash,verification_plan_set_hash,
                      generation_transition_set_hash,compilation_material_hash,
                      compiler_seal_hash
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
                compilation_material_hash: compilation_material_hash.clone(),
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
                   compilation_material_hash,compiler_seal_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
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
        .bind(&compilation_material_hash)
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
            let exact_dispatch_fence: bool = sqlx::query_scalar(
                r#"SELECT EXISTS(
                       SELECT 1
                         FROM candidate_analysis_work_items candidate
                         JOIN stage_work_items item
                           ON item.id=candidate.stage_work_item_id
                         JOIN stage_worker_runs worker
                           ON worker.work_item_id=item.id
                          AND worker.id=$3
                         JOIN candidate_analysis_provider_attempts receipt
                           ON receipt.analysis_attempt_id=candidate.analysis_attempt_id
                          AND receipt.stage_work_item_id=item.id
                          AND receipt.worker_run_id=worker.id
                          AND receipt.artifact_kind='controller_dispatch.v1'
                        WHERE candidate.analysis_attempt_id=$1
                          AND candidate.stage_work_item_id=$2
                          AND candidate.phase='controller'
                          AND candidate.capability='candidate_controller_dispatch'
                          AND item.kind='candidate_controller_dispatch'
                          AND item.role='controller'
                          AND item.status='completed'
                          AND worker.specialist='controller'
                          AND worker.work_item_kind='candidate_controller_dispatch'
                          AND worker.status='passed')"#,
            )
            .bind(input.fence.analysis_attempt_id)
            .bind(input.fence.work_item_id)
            .bind(input.fence.worker_run_id)
            .fetch_one(&mut *tx)
            .await?;
            if !exact_dispatch_fence {
                return Err(conflict(H1_CONTROLLER_FENCE_REQUIRED));
            }
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
            let closure:(i64,i64,i64,i64,i64,i64,i64,i64,i64,i64,i64,i64)=sqlx::query_as(r#"SELECT
                (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_subreview_census_members WHERE analysis_attempt_id=$1 AND disposition='required'),
                (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_subreviews WHERE analysis_attempt_id=$1),
                (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_synthesis_census_members WHERE analysis_attempt_id=$1),
                (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_synthesis_reviews WHERE analysis_attempt_id=$1),
                (SELECT count(*) FROM candidate_analysis_snapshot_inputs source JOIN candidate_analysis_attempts attempt ON attempt.snapshot_id=source.snapshot_id WHERE attempt.analysis_attempt_id=$1),
                (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_reviews WHERE analysis_attempt_id=$1),
                (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_global_reviews WHERE analysis_attempt_id=$1),
                (SELECT count(*) FROM hypothesis_proposals WHERE analysis_attempt_id=$1),
                (SELECT count(*) FROM candidate_analysis_conflict_components WHERE analysis_attempt_id=$1),
                (SELECT count(*) FROM hypothesis_merge_decisions WHERE analysis_attempt_id=$1),
                (SELECT count(*) FROM hypothesis_merge_decisions
                  WHERE analysis_attempt_id=$1 AND decision_kind<>'keep_distinct'),
                (SELECT count(*) FROM (
                    (SELECT synthesis_node_id
                       FROM candidate_analysis_hypothesis_coverage_synthesis_census_members
                      WHERE analysis_attempt_id=$1
                     EXCEPT
                     SELECT synthesis_node_id
                       FROM candidate_analysis_hypothesis_coverage_synthesis_reviews
                      WHERE analysis_attempt_id=$1)
                    UNION ALL
                    (SELECT synthesis_node_id
                       FROM candidate_analysis_hypothesis_coverage_synthesis_reviews
                      WHERE analysis_attempt_id=$1
                     EXCEPT
                     SELECT synthesis_node_id
                       FROM candidate_analysis_hypothesis_coverage_synthesis_census_members
                      WHERE analysis_attempt_id=$1)
                ) synthesis_node_drift)"#)
                .bind(input.fence.analysis_attempt_id).fetch_one(&mut *tx).await?;
            if closure.10 != 0 {
                return Err(conflict(CONFLICT_DECISION_UNRESOLVED));
            }
            if closure.0 == 0
                || closure.0 != closure.1
                || closure.2 == 0
                || closure.2 != closure.3
                || closure.11 != 0
                || closure.4 == 0
                || closure.4 != closure.5
                || closure.6 != 1
                || (closure.7 == 0 && (closure.8 != 0 || closure.9 != 0))
                || (closure.7 > 0 && (closure.8 != 1 || closure.9 != 1))
            {
                return Err(conflict(CENSUS_NOT_CLOSED));
            }
            let sources=sqlx::query_as::<_,CriticCensusSourceDbRow>(r#"SELECT member_kind,source_identity,source_hash FROM(
                SELECT 'proposal_conflict_review'::TEXT AS member_kind,
                       component.conflict_component_id AS source_identity,
                       decision.decision_hash AS source_hash
                  FROM candidate_analysis_conflict_components component
                  JOIN hypothesis_merge_decisions decision USING(conflict_component_id,analysis_attempt_id)
                 WHERE component.analysis_attempt_id=$1
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
    partition_hash: String,
}

#[derive(Debug)]
struct CoverageSubreviewMemberDraft {
    member_id: Uuid,
    checklist_member_id: Uuid,
    partition_id: Uuid,
    checklist_ordinal: i32,
    partition_ordinal: i32,
    designated_work_item_id: Uuid,
    disposition: &'static str,
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

async fn persist_provider_artifact_receipt_on(
    tx: &mut Transaction<'_, Postgres>,
    fence: &CandidateWriteFenceRow,
    provider_attempt_id: Uuid,
    provider_artifact_body: &Value,
    artifact_id: Uuid,
    artifact_kind: &str,
    artifact_hash: &str,
) -> Result<()> {
    let provider_hash = hash_json_on(tx, provider_artifact_body).await?;
    let existing: Option<(Uuid, String, Option<Uuid>)> = sqlx::query_as(
        r#"SELECT provider_attempt_id,artifact_hash,artifact_id
             FROM candidate_analysis_provider_attempts WHERE stage_work_item_id=$1"#,
    )
    .bind(fence.work_item_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some((existing_attempt, existing_hash, existing_artifact)) = existing {
        if existing_attempt != provider_attempt_id
            || existing_hash != provider_hash
            || existing_artifact != Some(artifact_id)
        {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
        let output_id = Uuid::new_v5(&artifact_id, b"candidate_stage_worker_output.v1");
        let linked: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM candidate_analysis_artifacts artifact
                   JOIN stage_worker_outputs output ON output.id=artifact.stage_worker_output_id
                  WHERE artifact.artifact_id=$1 AND artifact.stage_worker_output_id=$2
                    AND output.work_item_id=$3 AND output.worker_run_id=$4)"#,
        )
        .bind(artifact_id)
        .bind(output_id)
        .bind(fence.work_item_id)
        .bind(fence.worker_run_id)
        .fetch_one(&mut **tx)
        .await?;
        if !linked {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
        return Ok(());
    }
    sqlx::query(
        r#"INSERT INTO candidate_analysis_provider_attempts(
               provider_attempt_id,analysis_attempt_id,stage_work_item_id,worker_run_id,
               artifact_kind,artifact_body,artifact_hash,artifact_id)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(provider_attempt_id)
    .bind(fence.analysis_attempt_id)
    .bind(fence.work_item_id)
    .bind(fence.worker_run_id)
    .bind(artifact_kind)
    .bind(provider_artifact_body)
    .bind(provider_hash)
    .bind(artifact_id)
    .execute(&mut **tx)
    .await?;
    let output_id = Uuid::new_v5(&artifact_id, b"candidate_stage_worker_output.v1");
    let canonical_output = json!({
        "schema":"candidate_analysis_artifact_receipt.v1",
        "artifact_id":artifact_id,
        "artifact_hash":artifact_hash,
    });
    let output_hash = hash_json_on(tx, &canonical_output).await?;
    sqlx::query(
        r#"INSERT INTO stage_worker_outputs(
               id,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,output_version,
               business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
               checked_empty_cells,blocker_codes,output_hash)
           SELECT $1,plan.id,item.id,worker.id,plan.operation_id,plan.stage_execution_id,
                  plan.stage_run_unit_id,plan.scope_snapshot_id,plan.organization_id,
                  'candidate_analysis_artifact_receipt.v1',1,'artifact_recorded',$2,
                  '[]',ARRAY[]::BIGINT[],'[]',ARRAY[]::TEXT[],$3
             FROM stage_team_plans plan
             JOIN stage_work_items item ON item.team_plan_id=plan.id
             JOIN stage_worker_runs worker ON worker.work_item_id=item.id
            WHERE plan.id=$4 AND item.id=$5 AND worker.id=$6"#,
    )
    .bind(output_id)
    .bind(canonical_output)
    .bind(output_hash)
    .bind(fence.team_plan_id)
    .bind(fence.work_item_id)
    .bind(fence.worker_run_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE stage_work_items SET status='completed',terminal_at=statement_timestamp(),row_version=row_version+1,updated_at=statement_timestamp() WHERE id=$1 AND status='running'",
    )
    .bind(fence.work_item_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE stage_worker_runs SET status='passed',terminal_at=statement_timestamp(),updated_at=statement_timestamp() WHERE id=$1 AND status='running'",
    )
    .bind(fence.worker_run_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
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
        r#"SELECT chunk_partition_id,partition_ordinal,partition_hash
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
        .map(|row| row.partition_hash.clone())
        .collect::<Vec<_>>();
    let checklist_set_hash = hash_text_array_on(&mut tx, &checklist_hashes).await?;
    let partition_set_hash = hash_text_array_on(&mut tx, &partition_hashes).await?;
    let census_id = Uuid::new_v5(&input.stable_census_request_id, input.input_id.as_bytes());
    let mut members = Vec::with_capacity(checklist.len().saturating_mul(partitions.len()));
    for checklist_member in &checklist {
        for partition in &partitions {
            let designated: Vec<(Uuid, String)> = sqlx::query_as(
                r#"SELECT item.stage_work_item_id,item.capability
                      FROM candidate_analysis_work_items item
                     WHERE item.analysis_attempt_id=$1 AND item.phase='critic'
                       AND item.capability IN (
                           'hypothesis_coverage_subreview',
                           'hypothesis_coverage_sampling_omitted'
                       )
                       AND item.component_id=$2 AND item.microbatch_key=$3
                     ORDER BY item.stage_work_item_id"#,
            )
            .bind(input.fence.analysis_attempt_id)
            .bind(checklist_member.checklist_member_id)
            .bind(partition.chunk_partition_id.to_string())
            .fetch_all(&mut *tx)
            .await?;
            if designated.len() != 1 {
                return Err(conflict(CENSUS_NOT_CLOSED));
            }
            let (designated_work_item_id, capability) = &designated[0];
            let disposition = if capability == "hypothesis_coverage_sampling_omitted" {
                "sampling_omitted"
            } else {
                "required"
            };
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
                    "chunk_partition_hash":partition.partition_hash,
                    "designated_stage_work_item_id":designated_work_item_id,
                    "disposition":disposition,
                }),
            )
            .await?;
            members.push(CoverageSubreviewMemberDraft {
                member_id: Uuid::new_v5(&census_id, member_hash.as_bytes()),
                checklist_member_id: checklist_member.checklist_member_id,
                partition_id: partition.chunk_partition_id,
                checklist_ordinal: checklist_member.ordinal,
                partition_ordinal: partition.partition_ordinal,
                designated_work_item_id: *designated_work_item_id,
                disposition,
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
                VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
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
            .bind(member.disposition)
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
    /// Compatibility field name: values are server checklist-member IDs,
    /// never model-selected proposal IDs.
    pub missed_proposal_ids: Vec<Uuid>,
    pub blocker_codes: Vec<String>,
    pub semantic_summary: Value,
    pub review_notes: String,
    pub provider_attempt_id: Option<Uuid>,
    pub provider_artifact_body: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverageSemanticObservationV1 {
    kind: String,
    subject_kind: String,
    subject_identity_hash: String,
    predicate_schema: String,
    predicate_version: u32,
    polarity: String,
    trust_boundary: String,
    input_ids: Vec<Uuid>,
    checklist_member_ids: Vec<Uuid>,
    proposal_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverageSemanticSummaryV1 {
    covered_input_ids: Vec<Uuid>,
    covered_checklist_member_ids: Vec<Uuid>,
    observed_proposal_ids: Vec<Uuid>,
    missed_checklist_member_ids: Vec<Uuid>,
    blocker_codes: Vec<String>,
    semantic_observations: Vec<CoverageSemanticObservationV1>,
}

fn canonical_uuid_values(values: &[Uuid]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn canonical_text_values(values: &[String]) -> bool {
    values.iter().all(|value| !value.trim().is_empty())
        && values.windows(2).all(|pair| pair[0] < pair[1])
}

#[allow(clippy::too_many_arguments)]
async fn validate_coverage_semantic_summary_on(
    tx: &mut Transaction<'_, Postgres>,
    analysis_attempt_id: Uuid,
    summary_value: &Value,
    expected_input_ids: &BTreeSet<Uuid>,
    expected_checklist_ids: &BTreeSet<Uuid>,
    expected_observed_proposal_ids: &BTreeSet<Uuid>,
    expected_missed_checklist_ids: &BTreeSet<Uuid>,
    expected_blocker_codes: &BTreeSet<String>,
    expected_observations: Option<&BTreeSet<String>>,
) -> Result<(i64, String)> {
    let summary: CoverageSemanticSummaryV1 =
        serde_json::from_value(summary_value.clone()).map_err(|_| conflict(AUTHORITY_MISMATCH))?;
    if summary.semantic_observations.len() > 64
        || !canonical_uuid_values(&summary.covered_input_ids)
        || !canonical_uuid_values(&summary.covered_checklist_member_ids)
        || !canonical_uuid_values(&summary.observed_proposal_ids)
        || !canonical_uuid_values(&summary.missed_checklist_member_ids)
        || !canonical_text_values(&summary.blocker_codes)
        || summary
            .covered_input_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            != *expected_input_ids
        || summary
            .covered_checklist_member_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            != *expected_checklist_ids
        || summary
            .observed_proposal_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            != *expected_observed_proposal_ids
        || summary
            .missed_checklist_member_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            != *expected_missed_checklist_ids
        || summary
            .blocker_codes
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            != *expected_blocker_codes
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }

    let loaded_inputs: Vec<(Uuid, String, String)> = sqlx::query_as(
        r#"SELECT snapshot_input_id,subject_kind_at_time,subject_identity_hash
              FROM candidate_analysis_snapshot_inputs input
              JOIN candidate_analysis_attempts attempt ON attempt.snapshot_id=input.snapshot_id
             WHERE attempt.analysis_attempt_id=$1 AND input.snapshot_input_id=ANY($2)
             ORDER BY input.subject_kind_at_time,input.subject_identity_hash"#,
    )
    .bind(analysis_attempt_id)
    .bind(summary.covered_input_ids.as_slice())
    .fetch_all(&mut **tx)
    .await?;
    let loaded_input_ids = loaded_inputs
        .iter()
        .map(|row| row.0)
        .collect::<BTreeSet<_>>();
    let subjects = loaded_inputs
        .into_iter()
        .map(|(_, kind, identity)| (kind, identity))
        .collect::<BTreeSet<_>>();
    let trust_boundaries: BTreeSet<String> = sqlx::query_scalar(
        r#"SELECT trust_boundary_identity
              FROM candidate_analysis_hypothesis_coverage_checklist_members
             WHERE analysis_attempt_id=$1 AND checklist_member_id=ANY($2)
             ORDER BY trust_boundary_identity"#,
    )
    .bind(analysis_attempt_id)
    .bind(summary.covered_checklist_member_ids.as_slice())
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .collect();
    type ProposalSemanticAuthority = (Uuid, String, String, String, i64, String);
    let loaded_proposal_authorities: Vec<ProposalSemanticAuthority> = sqlx::query_as(
        r#"SELECT DISTINCT proposal.proposal_id,
                  proposal.structured_proposal->>'subject_kind',
                  proposal.structured_proposal->>'subject_identity_hash',
                  proposal.structured_proposal->>'predicate_schema',
                  (proposal.structured_proposal->>'predicate_version')::BIGINT,
                  proposal.structured_proposal->>'polarity'
              FROM hypothesis_proposals proposal
              JOIN candidate_analysis_artifacts artifact
                ON artifact.artifact_id=proposal.artifact_id
               AND artifact.analysis_attempt_id=proposal.analysis_attempt_id
              JOIN candidate_analysis_work_items candidate
                ON candidate.candidate_work_item_id=artifact.candidate_work_item_id
               AND candidate.analysis_attempt_id=proposal.analysis_attempt_id
              LEFT JOIN hypothesis_proposal_refs reference
                ON reference.proposal_id=proposal.proposal_id
               AND reference.analysis_attempt_id=proposal.analysis_attempt_id
             WHERE proposal.analysis_attempt_id=$1
               AND proposal.proposal_id=ANY($2)
               AND (candidate.microbatch_key::UUID=ANY($3)
                    OR reference.snapshot_input_id=ANY($3))
             ORDER BY proposal.proposal_id"#,
    )
    .bind(analysis_attempt_id)
    .bind(
        expected_observed_proposal_ids
            .iter()
            .copied()
            .collect::<Vec<_>>(),
    )
    .bind(summary.covered_input_ids.as_slice())
    .fetch_all(&mut **tx)
    .await?;
    let loaded_proposal_ids = loaded_proposal_authorities
        .iter()
        .map(|row| row.0)
        .collect::<BTreeSet<_>>();
    let proposal_authorities = loaded_proposal_authorities
        .into_iter()
        .map(|row| (row.0, (row.1, row.2, row.3, row.4, row.5)))
        .collect::<BTreeMap<_, _>>();
    if loaded_input_ids != *expected_input_ids
        || trust_boundaries.is_empty()
        || loaded_proposal_ids != *expected_observed_proposal_ids
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }

    let mut observation_set = BTreeSet::new();
    for observation in &summary.semantic_observations {
        let input_subject_authorized = observation.proposal_ids.is_empty()
            && subjects.contains(&(
                observation.subject_kind.clone(),
                observation.subject_identity_hash.clone(),
            ));
        let proposal_semantics_authorized = !observation.proposal_ids.is_empty()
            && observation.proposal_ids.iter().all(|proposal_id| {
                proposal_authorities.get(proposal_id).is_some_and(
                    |(
                        subject_kind,
                        subject_identity_hash,
                        predicate_schema,
                        predicate_version,
                        polarity,
                    )| {
                        subject_kind == &observation.subject_kind
                            && subject_identity_hash == &observation.subject_identity_hash
                            && predicate_schema == &observation.predicate_schema
                            && *predicate_version == i64::from(observation.predicate_version)
                            && polarity == &observation.polarity
                    },
                )
            });
        if !matches!(
            observation.kind.as_str(),
            "potential_hypothesis"
                | "supporting_pattern"
                | "contradicting_pattern"
                | "coverage_gap"
        ) || observation.subject_kind.trim().is_empty()
            || observation.predicate_schema.trim().is_empty()
            || observation.predicate_version == 0
            || !matches!(observation.polarity.as_str(), "positive" | "negative")
            || (!input_subject_authorized && !proposal_semantics_authorized)
            || !trust_boundaries.contains(&observation.trust_boundary)
            || observation.input_ids.is_empty()
            || observation.checklist_member_ids.is_empty()
            || !canonical_uuid_values(&observation.input_ids)
            || !canonical_uuid_values(&observation.checklist_member_ids)
            || !canonical_uuid_values(&observation.proposal_ids)
            || !observation
                .input_ids
                .iter()
                .all(|id| expected_input_ids.contains(id))
            || !observation
                .checklist_member_ids
                .iter()
                .all(|id| expected_checklist_ids.contains(id))
            || !observation
                .proposal_ids
                .iter()
                .all(|id| expected_observed_proposal_ids.contains(id))
        {
            return Err(conflict(AUTHORITY_MISMATCH));
        }
        let encoded =
            serde_json::to_string(observation).map_err(|_| conflict(AUTHORITY_MISMATCH))?;
        if !observation_set.insert(encoded) {
            return Err(conflict(AUTHORITY_MISMATCH));
        }
    }
    if expected_observations.is_some_and(|expected| *expected != observation_set) {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    Ok((
        i64::try_from(summary.semantic_observations.len())
            .map_err(|_| conflict(AUTHORITY_MISMATCH))?,
        hash_json_on(tx, summary_value).await?,
    ))
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
    checklist_member_id: Uuid,
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
    if input.provider_attempt_id.is_some() != input.provider_artifact_body.is_some() {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let mut tx = pool.begin().await?;
    validate_write_fence_on(&mut tx, &input.fence).await?;
    let authority = sqlx::query_as::<_, CoverageSubreviewAuthorityDbRow>(
        r#"SELECT member.snapshot_input_id,member.checklist_member_id,
                  member.designated_stage_work_item_id,
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
    ).bind(input.fence.analysis_attempt_id).bind(authority.snapshot_input_id.to_string())
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
    input.blocker_codes.sort_unstable();
    input.blocker_codes.dedup();
    if input
        .missed_proposal_ids
        .iter()
        .any(|reference| *reference != authority.checklist_member_id)
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let valid_outcome = match input.outcome.as_str() {
        "no_local_miss" => input.missed_proposal_ids.is_empty() && input.blocker_codes.is_empty(),
        "missed_hypothesis" => {
            !input.missed_proposal_ids.is_empty() && input.blocker_codes.is_empty()
        }
        "blocked" => {
            input.missed_proposal_ids.is_empty()
                && !input.blocker_codes.is_empty()
                && input
                    .blocker_codes
                    .iter()
                    .all(|value| !value.trim().is_empty())
        }
        _ => false,
    };
    if !valid_outcome {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let blocker_codes = input.blocker_codes.clone();
    let context_truncated = blocker_codes.iter().any(|code| code == "context_truncated");
    if context_truncated && input.outcome != "blocked" {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let expected_input_ids = BTreeSet::from([authority.snapshot_input_id]);
    let expected_checklist_ids = BTreeSet::from([authority.checklist_member_id]);
    let expected_observed_proposal_ids: BTreeSet<Uuid> = sqlx::query_scalar(
        r#"SELECT DISTINCT proposal.proposal_id
              FROM hypothesis_proposals proposal
              JOIN candidate_analysis_artifacts artifact
                ON artifact.artifact_id=proposal.artifact_id
               AND artifact.analysis_attempt_id=proposal.analysis_attempt_id
              JOIN candidate_analysis_work_items candidate
                ON candidate.candidate_work_item_id=artifact.candidate_work_item_id
               AND candidate.analysis_attempt_id=proposal.analysis_attempt_id
              LEFT JOIN hypothesis_proposal_refs reference
                ON reference.proposal_id=proposal.proposal_id
               AND reference.analysis_attempt_id=proposal.analysis_attempt_id
             WHERE proposal.analysis_attempt_id=$1
               AND (candidate.microbatch_key::UUID=$2
                    OR reference.snapshot_input_id=$2)
             ORDER BY proposal.proposal_id"#,
    )
    .bind(input.fence.analysis_attempt_id)
    .bind(authority.snapshot_input_id)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect();
    let expected_missed_checklist_ids = input
        .missed_proposal_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let expected_blocker_codes = blocker_codes.iter().cloned().collect::<BTreeSet<_>>();
    let (semantic_observation_count, semantic_summary_hash) =
        validate_coverage_semantic_summary_on(
            &mut tx,
            input.fence.analysis_attempt_id,
            &input.semantic_summary,
            &expected_input_ids,
            &expected_checklist_ids,
            &expected_observed_proposal_ids,
            &expected_missed_checklist_ids,
            &expected_blocker_codes,
            None,
        )
        .await?;
    let body = json!({"kind":"hypothesis_coverage_subreview.v1",
        "subreview_census_id":input.subreview_census_id,
        "subreview_census_member_id":input.subreview_census_member_id,
        "outcome":input.outcome,"typed_missed_refs":input.missed_proposal_ids,
        "blocker_codes":&blocker_codes,"semantic_summary":&input.semantic_summary,
        "semantic_summary_hash":&semantic_summary_hash,"review_notes":input.review_notes});
    let subreview_hash = hash_json_on(&mut tx, &json!({"body":body,
        "designated_chunk_set_hash":authority.chunk_set_hash,"read_receipt_set_hash":read_set_hash,
        "h1_proposal_ref_set_hash":proposal_ref_set_hash,"primary_worker":primary_worker,
        "map_critic_worker":input.fence.worker_run_id,"context_budget":authority.bounded_context_budget,
        "context_truncated":context_truncated})).await?;
    let subreview_id = Uuid::new_v5(
        &input.stable_review_request_id,
        input.subreview_census_member_id.as_bytes(),
    );
    let artifact_id = Uuid::new_v5(
        &input.stable_review_request_id,
        b"hypothesis_coverage_subreview.v1",
    );
    let artifact_hash = hash_json_on(&mut tx, &body).await?;
    let output_id = Uuid::new_v5(&artifact_id, b"candidate_stage_worker_output.v1");
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
        sqlx::query(
            r#"INSERT INTO candidate_analysis_artifacts(artifact_id,analysis_attempt_id,
                candidate_work_item_id,worker_run_id,stage_worker_output_id,
                artifact_kind,artifact_body,artifact_hash)
            VALUES($1,$2,$3,$4,$5,'hypothesis_coverage_subreview.v1',$6,$7)"#,
        )
        .bind(artifact_id)
        .bind(input.fence.analysis_attempt_id)
        .bind(candidate_item_id)
        .bind(input.fence.worker_run_id)
        .bind(input.provider_attempt_id.map(|_| output_id))
        .bind(&body)
        .bind(&artifact_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(r#"INSERT INTO candidate_analysis_hypothesis_coverage_subreviews(
                subreview_id,subreview_census_member_id,subreview_census_id,analysis_attempt_id,
                snapshot_input_id,designated_chunk_count,designated_chunk_set_hash,
                read_receipt_count,read_receipt_set_hash,h1_proposal_ref_count,h1_proposal_ref_set_hash,
                primary_analyst_worker_run_id,map_critic_worker_run_id,context_budget,context_truncated,
                outcome,typed_missed_refs,blocker_codes,semantic_summary,
                semantic_observation_count,semantic_summary_hash,subreview_hash)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22)"#)
            .bind(subreview_id).bind(input.subreview_census_member_id).bind(input.subreview_census_id)
            .bind(input.fence.analysis_attempt_id).bind(authority.snapshot_input_id)
            .bind(authority.chunk_count).bind(authority.chunk_set_hash).bind(read_hashes.len() as i64)
            .bind(read_set_hash).bind(proposal_ref_hashes.len() as i64).bind(proposal_ref_set_hash)
            .bind(primary_worker).bind(input.fence.worker_run_id).bind(authority.bounded_context_budget)
            .bind(context_truncated).bind(&input.outcome).bind(json!(input.missed_proposal_ids))
            .bind(blocker_codes).bind(&input.semantic_summary).bind(semantic_observation_count)
            .bind(&semantic_summary_hash).bind(&subreview_hash).execute(&mut *tx).await?;
    }
    if let (Some(provider_attempt_id), Some(provider_artifact_body)) = (
        input.provider_attempt_id,
        input.provider_artifact_body.as_ref(),
    ) {
        persist_provider_artifact_receipt_on(
            &mut tx,
            &input.fence,
            provider_attempt_id,
            provider_artifact_body,
            artifact_id,
            "hypothesis_coverage_subreview.v1",
            &artifact_hash,
        )
        .await?;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecomputedCoverageSynthesisGateNodeRow {
    pub node_hash: String,
    pub node_kind: String,
    pub expected_child_hashes: Vec<String>,
    pub observed_child_hashes: Vec<String>,
    pub synthesis_worker_run_id: Uuid,
    pub primary_analyst_worker_run_ids: Vec<Uuid>,
    pub transitive_descendant_worker_run_ids: Vec<Uuid>,
    pub outcome: String,
    pub context_truncated: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct PersistedSynthesisGateNodeDbRow {
    synthesis_node_id: Uuid,
    node_kind: String,
    level: i32,
    partition_ordinal: i32,
    covered_input_count: i64,
    covered_input_set_hash: String,
    covered_checklist_count: i64,
    covered_checklist_set_hash: String,
    child_receipt_count: i64,
    child_receipt_set_hash: String,
    descendant_worker_count: i64,
    descendant_worker_set_hash: String,
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

#[derive(Debug, sqlx::FromRow)]
struct PersistedSynthesisCensusDbRow {
    synthesis_census_id: Uuid,
    relationship_cross_index_hash: String,
    fan_in_limit: i32,
    node_count: i64,
    node_set_hash: String,
    dimension_root_count: i64,
    dimension_root_set_hash: String,
    global_root_node_id: Uuid,
    census_hash: String,
}

type SynthesisLeafGroupKey = (String, String, Uuid, Uuid);
type SynthesisLeafGroupValue = Vec<(String, Uuid, Uuid)>;
type PersistedSynthesisChildRow = (Uuid, String, Option<Uuid>, Option<Uuid>, String);
type TypedMissedSubreviewRow = (
    Uuid,
    Uuid,
    Uuid,
    String,
    Value,
    Vec<String>,
    Value,
    i64,
    String,
);
type TypedMissedSynthesisRow = (
    Uuid,
    String,
    Value,
    Vec<String>,
    Value,
    i64,
    String,
    i64,
    String,
    i64,
    String,
    i64,
);

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
    const FAN_IN: usize = 32;
    let mut tx = pool.begin().await?;
    validate_write_fence_on(&mut tx, &input.fence).await?;
    let (expected_count,review_count):(i64,i64)=sqlx::query_as(r#"SELECT
        (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_subreview_census_members WHERE analysis_attempt_id=$1 AND disposition='required'),
        (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_subreviews WHERE analysis_attempt_id=$1)"#)
        .bind(input.fence.analysis_attempt_id).fetch_one(&mut *tx).await?;
    if expected_count == 0 || expected_count != review_count {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let relationship_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT member.member_hash
        FROM candidate_analysis_snapshot_source_sets source_set
        JOIN candidate_analysis_snapshot_source_set_members member USING(source_set_id,snapshot_id)
        WHERE source_set.snapshot_id=$1 AND source_set.source_kind='relations'
        ORDER BY member.ordinal"#,
    )
    .bind(input.fence.snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
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
        entry.push((
            leaf.subreview_hash,
            leaf.primary_analyst_worker_run_id,
            leaf.map_critic_worker_run_id,
        ));
    }
    let mut nodes = Vec::new();
    let mut cross_chunk = Vec::new();
    let mut cross_chunk_ordinal = 0_i32;
    for ((attack, boundary, input_id, checklist_id), leaves) in leaf_groups {
        for leaf_chunk in leaves.chunks(FAN_IN) {
            let child_hashes = leaf_chunk
                .iter()
                .map(|leaf| leaf.0.clone())
                .collect::<Vec<_>>();
            let workers = leaf_chunk
                .iter()
                .flat_map(|leaf| [leaf.1, leaf.2])
                .collect::<Vec<_>>();
            let node = build_synthesis_node_on(
                &mut tx,
                census_id,
                "cross_chunk",
                0,
                cross_chunk_ordinal,
                Some(attack.clone()),
                Some(boundary.clone()),
                vec![input_id],
                vec![checklist_id],
                child_hashes,
                workers,
                &relationship_hash,
            )
            .await?;
            nodes.push(node.clone());
            cross_chunk.push(node);
            cross_chunk_ordinal = cross_chunk_ordinal
                .checked_add(1)
                .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        }
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
    let mut dimension_hashes = dimension_roots
        .iter()
        .map(|node| node.node_hash.clone())
        .collect::<Vec<_>>();
    dimension_hashes.sort();
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
    let mut node_hashes = nodes
        .iter()
        .map(|node| node.node_hash.clone())
        .collect::<Vec<_>>();
    node_hashes.sort();
    let node_set_hash = hash_text_array_on(&mut tx, &node_hashes).await?;
    let node_count = nodes.len() as i64;
    let census_hash=hash_json_on(&mut tx,&json!({"domain":"candidate_hypothesis_coverage_synthesis_census.v1",
        "analysis_attempt_id":input.fence.analysis_attempt_id,"relationship_cross_index_hash":relationship_hash,
        "fan_in_limit":FAN_IN,"node_count":node_count,"node_set_hash":node_set_hash,
        "dimension_root_count":dimension_count,"dimension_root_set_hash":dimension_set_hash,
        "global_root_node_id":global_id})).await?;
    let existing=sqlx::query_as::<_,PersistedSynthesisCensusDbRow>(r#"SELECT synthesis_census_id,
        relationship_cross_index_hash,fan_in_limit,node_count,node_set_hash,
        dimension_root_count,dimension_root_set_hash,global_root_node_id,census_hash
        FROM candidate_analysis_hypothesis_coverage_synthesis_censuses WHERE analysis_attempt_id=$1"#)
        .bind(input.fence.analysis_attempt_id).fetch_optional(&mut *tx).await?;
    let replayed = existing.is_some();
    if let Some(existing) = existing {
        if existing.synthesis_census_id != census_id
            || existing.relationship_cross_index_hash != relationship_hash
            || existing.fan_in_limit != FAN_IN as i32
            || existing.node_count != node_count
            || existing.node_set_hash != node_set_hash
            || existing.dimension_root_count != dimension_count
            || existing.dimension_root_set_hash != dimension_set_hash
            || existing.global_root_node_id != global_id
            || existing.census_hash != census_hash
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
        let synthesis_child_ids = nodes
            .iter()
            .map(|node| (node.node_hash.clone(), node.node_id))
            .collect::<BTreeMap<_, _>>();
        let subreview_child_ids: BTreeMap<String, Uuid> = sqlx::query_as::<_, (String, Uuid)>(
            r#"SELECT subreview_hash,subreview_id
                  FROM candidate_analysis_hypothesis_coverage_subreviews
                 WHERE analysis_attempt_id=$1"#,
        )
        .bind(input.fence.analysis_attempt_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
        let primary_workers: BTreeSet<Uuid> = sqlx::query_scalar(
            r#"SELECT DISTINCT primary_analyst_worker_run_id
                  FROM candidate_analysis_hypothesis_coverage_subreviews
                 WHERE analysis_attempt_id=$1"#,
        )
        .bind(input.fence.analysis_attempt_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
        for node in nodes {
            sqlx::query(r#"INSERT INTO candidate_analysis_hypothesis_coverage_synthesis_census_members(
            synthesis_node_id,synthesis_census_id,analysis_attempt_id,node_kind,level,partition_ordinal,
            attack_class_id,trust_boundary_hash,covered_input_count,covered_input_set_hash,
            covered_checklist_count,covered_checklist_set_hash,child_receipt_count,child_receipt_set_hash,
            relationship_cross_index_hash,descendant_worker_count,descendant_worker_set_hash,node_hash)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#)
            .bind(node.node_id).bind(census_id).bind(input.fence.analysis_attempt_id).bind(node.node_kind)
            .bind(node.level).bind(node.partition_ordinal).bind(&node.attack_class_id).bind(&node.trust_boundary_hash)
            .bind(node.covered_input_ids.len() as i64).bind(&node.covered_input_set_hash)
            .bind(node.covered_checklist_ids.len() as i64).bind(&node.covered_checklist_set_hash)
            .bind(node.child_hashes.len() as i64).bind(&node.child_set_hash).bind(&node.relationship_cross_index_hash)
            .bind(node.descendant_workers.len() as i64).bind(&node.descendant_worker_set_hash).bind(&node.node_hash)
            .execute(&mut *tx).await?;
            for (ordinal, child_hash) in node.child_hashes.iter().enumerate() {
                let (child_kind, child_subreview_id, child_synthesis_node_id) =
                    if node.node_kind == "cross_chunk" {
                        (
                            "subreview",
                            Some(
                                *subreview_child_ids
                                    .get(child_hash)
                                    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?,
                            ),
                            None,
                        )
                    } else {
                        (
                            "synthesis_node",
                            None,
                            Some(
                                *synthesis_child_ids
                                    .get(child_hash)
                                    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?,
                            ),
                        )
                    };
                let member_hash = hash_json_on(
                    &mut tx,
                    &json!({
                        "synthesis_node_id":node.node_id,
                        "ordinal":ordinal,
                        "child_kind":child_kind,
                        "child_subreview_id":child_subreview_id,
                        "child_synthesis_node_id":child_synthesis_node_id,
                        "child_receipt_hash":child_hash,
                    }),
                )
                .await?;
                sqlx::query(
                    r#"INSERT INTO candidate_analysis_hypothesis_coverage_synthesis_node_children(
                           child_member_id,synthesis_node_id,synthesis_census_id,
                           analysis_attempt_id,ordinal,child_kind,child_subreview_id,
                           child_synthesis_node_id,child_receipt_hash,member_hash)
                       VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
                )
                .bind(Uuid::new_v5(&node.node_id, member_hash.as_bytes()))
                .bind(node.node_id)
                .bind(census_id)
                .bind(input.fence.analysis_attempt_id)
                .bind(i32::try_from(ordinal).map_err(|_| conflict(CENSUS_NOT_CLOSED))?)
                .bind(child_kind)
                .bind(child_subreview_id)
                .bind(child_synthesis_node_id)
                .bind(child_hash)
                .bind(member_hash)
                .execute(&mut *tx)
                .await?;
            }
            for (ordinal, worker_run_id) in node.descendant_workers.iter().enumerate() {
                let descendant_role = if primary_workers.contains(worker_run_id) {
                    "primary_analyst"
                } else {
                    "map_critic"
                };
                let member_hash = hash_json_on(
                    &mut tx,
                    &json!({
                        "synthesis_node_id":node.node_id,
                        "ordinal":ordinal,
                        "worker_run_id":worker_run_id,
                        "descendant_role":descendant_role,
                    }),
                )
                .await?;
                sqlx::query(
                    r#"INSERT INTO candidate_analysis_hypothesis_coverage_synthesis_node_descendant_workers(
                           descendant_member_id,synthesis_node_id,synthesis_census_id,
                           analysis_attempt_id,ordinal,worker_run_id,descendant_role,member_hash)
                       VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
                )
                .bind(Uuid::new_v5(&node.node_id, member_hash.as_bytes()))
                .bind(node.node_id)
                .bind(census_id)
                .bind(input.fence.analysis_attempt_id)
                .bind(i32::try_from(ordinal).map_err(|_| conflict(CENSUS_NOT_CLOSED))?)
                .bind(worker_run_id)
                .bind(descendant_role)
                .bind(member_hash)
                .execute(&mut *tx)
                .await?;
            }
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

/// Rebuilds the recursive synthesis tree from durable leaf reviews and
/// compares every persisted node seal with the server reducer. Gate callers
/// receive independently derived child closure and lineage, never a pair of
/// fields copied from the same persisted aggregate hash.
pub async fn load_recomputed_coverage_synthesis_gate_nodes(
    pool: &PgPool,
    analysis_attempt_id: Uuid,
) -> Result<Vec<RecomputedCoverageSynthesisGateNodeRow>> {
    let mut tx = pool.begin().await?;
    let rows =
        load_recomputed_coverage_synthesis_gate_nodes_on(&mut tx, analysis_attempt_id).await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn load_recomputed_coverage_synthesis_gate_nodes_on(
    tx: &mut Transaction<'_, Postgres>,
    analysis_attempt_id: Uuid,
) -> Result<Vec<RecomputedCoverageSynthesisGateNodeRow>> {
    const FAN_IN: usize = 32;
    let (census_id, relationship_hash, fan_in): (Uuid, String, i32) = sqlx::query_as(
        r#"SELECT synthesis_census_id,relationship_cross_index_hash,fan_in_limit
              FROM candidate_analysis_hypothesis_coverage_synthesis_censuses
             WHERE analysis_attempt_id=$1 FOR SHARE"#,
    )
    .bind(analysis_attempt_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    if fan_in != FAN_IN as i32 {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let leaves = sqlx::query_as::<_, SynthesisLeafDbRow>(
        r#"SELECT member.snapshot_input_id,
                  member.checklist_member_id,checklist.attack_class_id,
                  checklist.trust_boundary_hash,review.subreview_hash,
                  review.primary_analyst_worker_run_id,review.map_critic_worker_run_id
             FROM candidate_analysis_hypothesis_coverage_subreview_census_members member
             JOIN candidate_analysis_hypothesis_coverage_checklist_members checklist
               ON checklist.checklist_member_id=member.checklist_member_id
             JOIN candidate_analysis_hypothesis_coverage_subreviews review
               ON review.subreview_census_member_id=member.subreview_census_member_id
            WHERE member.analysis_attempt_id=$1 AND member.disposition='required'
            ORDER BY checklist.attack_class_id,checklist.trust_boundary_hash,
                     member.snapshot_input_id,member.checklist_ordinal,member.partition_ordinal"#,
    )
    .bind(analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    let expected_leaf_count: i64 = sqlx::query_scalar(
        r#"SELECT count(*)
              FROM candidate_analysis_hypothesis_coverage_subreview_census_members
             WHERE analysis_attempt_id=$1 AND disposition='required'"#,
    )
    .bind(analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    if leaves.len() as i64 != expected_leaf_count || leaves.is_empty() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let reviewed_subreview_hashes = leaves
        .iter()
        .map(|leaf| leaf.subreview_hash.clone())
        .collect::<BTreeSet<_>>();
    let primary_workers = leaves
        .iter()
        .map(|leaf| leaf.primary_analyst_worker_run_id)
        .collect::<BTreeSet<_>>();
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
        entry.push((
            leaf.subreview_hash,
            leaf.primary_analyst_worker_run_id,
            leaf.map_critic_worker_run_id,
        ));
    }
    let mut nodes = Vec::new();
    let mut cross_chunk = Vec::new();
    let mut cross_chunk_ordinal = 0_i32;
    for ((attack, boundary, input_id, checklist_id), leaves) in leaf_groups {
        for leaf_chunk in leaves.chunks(FAN_IN) {
            let children = leaf_chunk
                .iter()
                .map(|leaf| leaf.0.clone())
                .collect::<Vec<_>>();
            let workers = leaf_chunk
                .iter()
                .flat_map(|leaf| [leaf.1, leaf.2])
                .collect::<Vec<_>>();
            let node = build_synthesis_node_on(
                tx,
                census_id,
                "cross_chunk",
                0,
                cross_chunk_ordinal,
                Some(attack.clone()),
                Some(boundary.clone()),
                vec![input_id],
                vec![checklist_id],
                children,
                workers,
                &relationship_hash,
            )
            .await?;
            nodes.push(node.clone());
            cross_chunk.push(node);
            cross_chunk_ordinal = cross_chunk_ordinal
                .checked_add(1)
                .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        }
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
        for (ordinal, children) in leaves.chunks(FAN_IN).enumerate() {
            let node = combine_synthesis_nodes_on(
                tx,
                census_id,
                "cross_input_partition",
                0,
                i32::try_from(ordinal).map_err(|_| conflict(CENSUS_NOT_CLOSED))?,
                Some(attack.clone()),
                Some(boundary.clone()),
                children,
                &relationship_hash,
            )
            .await?;
            nodes.push(node.clone());
            current.push(node);
        }
        let mut level = 1;
        while current.len() > 1 {
            let mut next = Vec::new();
            for (ordinal, children) in current.chunks(FAN_IN).enumerate() {
                let node = combine_synthesis_nodes_on(
                    tx,
                    census_id,
                    "cross_input_reduce",
                    level,
                    i32::try_from(ordinal).map_err(|_| conflict(CENSUS_NOT_CLOSED))?,
                    Some(attack.clone()),
                    Some(boundary.clone()),
                    children,
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
    let mut current = dimension_roots;
    let mut level = 0;
    while current.len() > 1 {
        let mut next = Vec::new();
        for (ordinal, children) in current.chunks(FAN_IN).enumerate() {
            let node = combine_synthesis_nodes_on(
                tx,
                census_id,
                "cross_dimension_reduce",
                level,
                i32::try_from(ordinal).map_err(|_| conflict(CENSUS_NOT_CLOSED))?,
                None,
                None,
                children,
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
        tx,
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
    nodes.push(global);

    let persisted = sqlx::query_as::<_, PersistedSynthesisGateNodeDbRow>(
        r#"SELECT synthesis_node_id,node_kind,level,partition_ordinal,
                  covered_input_count,covered_input_set_hash,covered_checklist_count,
                  covered_checklist_set_hash,child_receipt_count,child_receipt_set_hash,
                  descendant_worker_count,descendant_worker_set_hash,node_hash
             FROM candidate_analysis_hypothesis_coverage_synthesis_census_members
            WHERE synthesis_census_id=$1"#,
    )
    .bind(census_id)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|row| (row.synthesis_node_id, row))
    .collect::<BTreeMap<_, _>>();
    if persisted.len() != nodes.len() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    for node in &nodes {
        let row = persisted
            .get(&node.node_id)
            .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        if row.node_kind != node.node_kind
            || row.level != node.level
            || row.partition_ordinal != node.partition_ordinal
            || row.covered_input_count != node.covered_input_ids.len() as i64
            || row.covered_input_set_hash != node.covered_input_set_hash
            || row.covered_checklist_count != node.covered_checklist_ids.len() as i64
            || row.covered_checklist_set_hash != node.covered_checklist_set_hash
            || row.child_receipt_count != node.child_hashes.len() as i64
            || row.child_receipt_set_hash != node.child_set_hash
            || row.descendant_worker_count != node.descendant_workers.len() as i64
            || row.descendant_worker_set_hash != node.descendant_worker_set_hash
            || row.node_hash != node.node_hash
        {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
    }
    let child_rows: Vec<PersistedSynthesisChildRow> = sqlx::query_as(
        r#"SELECT synthesis_node_id,child_kind,child_subreview_id,
                  child_synthesis_node_id,child_receipt_hash
             FROM candidate_analysis_hypothesis_coverage_synthesis_node_children
            WHERE synthesis_census_id=$1
            ORDER BY synthesis_node_id,ordinal"#,
    )
    .bind(census_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut children_by_node =
        BTreeMap::<Uuid, Vec<(String, Option<Uuid>, Option<Uuid>, String)>>::new();
    for (node_id, kind, subreview_id, child_node_id, hash) in child_rows {
        children_by_node.entry(node_id).or_default().push((
            kind,
            subreview_id,
            child_node_id,
            hash,
        ));
    }
    let descendant_rows: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        r#"SELECT synthesis_node_id,worker_run_id,descendant_role
             FROM candidate_analysis_hypothesis_coverage_synthesis_node_descendant_workers
            WHERE synthesis_census_id=$1
            ORDER BY synthesis_node_id,ordinal"#,
    )
    .bind(census_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut descendants_by_node = BTreeMap::<Uuid, Vec<(Uuid, String)>>::new();
    for (node_id, worker_id, role) in descendant_rows {
        descendants_by_node
            .entry(node_id)
            .or_default()
            .push((worker_id, role));
    }
    for node in &nodes {
        let children = children_by_node
            .get(&node.node_id)
            .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        let listed_hashes = children
            .iter()
            .map(|child| child.3.clone())
            .collect::<Vec<_>>();
        if listed_hashes != node.child_hashes {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        for (kind, subreview_id, child_node_id, child_hash) in children {
            let actual_hash: Option<String> = match kind.as_str() {
                "subreview" if node.node_kind == "cross_chunk" => {
                    sqlx::query_scalar(
                        r#"SELECT subreview_hash
                          FROM candidate_analysis_hypothesis_coverage_subreviews
                         WHERE subreview_id=$1 AND analysis_attempt_id=$2"#,
                    )
                    .bind(subreview_id)
                    .bind(analysis_attempt_id)
                    .fetch_optional(&mut **tx)
                    .await?
                }
                "synthesis_node" if node.node_kind != "cross_chunk" => {
                    sqlx::query_scalar(
                        r#"SELECT node_hash
                          FROM candidate_analysis_hypothesis_coverage_synthesis_census_members
                         WHERE synthesis_node_id=$1 AND synthesis_census_id=$2"#,
                    )
                    .bind(child_node_id)
                    .bind(census_id)
                    .fetch_optional(&mut **tx)
                    .await?
                }
                _ => return Err(conflict(CENSUS_NOT_CLOSED)),
            };
            if actual_hash.as_deref() != Some(child_hash) {
                return Err(conflict(CENSUS_NOT_CLOSED));
            }
        }
        let descendants = descendants_by_node
            .get(&node.node_id)
            .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        let listed_workers = descendants
            .iter()
            .map(|descendant| descendant.0)
            .collect::<Vec<_>>();
        if listed_workers != node.descendant_workers {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        for (worker_id, role) in descendants {
            let actual_role: Option<&str> = if primary_workers.contains(worker_id) {
                Some("primary_analyst")
            } else if sqlx::query_scalar::<_, bool>(
                r#"SELECT EXISTS(
                       SELECT 1 FROM candidate_analysis_hypothesis_coverage_subreviews
                        WHERE analysis_attempt_id=$1 AND map_critic_worker_run_id=$2)"#,
            )
            .bind(analysis_attempt_id)
            .bind(worker_id)
            .fetch_one(&mut **tx)
            .await?
            {
                Some("map_critic")
            } else {
                None
            };
            if actual_role != Some(role.as_str()) {
                return Err(conflict(CENSUS_NOT_CLOSED));
            }
        }
    }
    let review_rows: Vec<(Uuid, Uuid, String, bool, String, bool)> = sqlx::query_as(
        r#"SELECT synthesis_node_id,synthesis_worker_run_id,outcome,context_truncated,
                  transitive_descendant_worker_set_hash,worker_separation_valid
              FROM candidate_analysis_hypothesis_coverage_synthesis_reviews
             WHERE synthesis_census_id=$1"#,
    )
    .bind(census_id)
    .fetch_all(&mut **tx)
    .await?;
    let review_by_node = review_rows
        .into_iter()
        .map(|row| (row.0, (row.1, row.2, row.3, row.4, row.5)))
        .collect::<BTreeMap<_, _>>();
    if review_by_node.len() != nodes.len() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let reviewed_node_hashes = nodes
        .iter()
        .filter(|node| review_by_node.contains_key(&node.node_id))
        .map(|node| node.node_hash.clone())
        .collect::<BTreeSet<_>>();
    let mut full_descendants = BTreeMap::<String, BTreeSet<Uuid>>::new();
    let mut result = Vec::with_capacity(nodes.len());
    for node in nodes {
        let mut descendants = node
            .descendant_workers
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if node.node_kind != "cross_chunk" {
            for child_hash in &node.child_hashes {
                if let Some(child_descendants) = full_descendants.get(child_hash) {
                    descendants.extend(child_descendants);
                }
                if let Some(child) = persisted.values().find(|row| &row.node_hash == child_hash) {
                    if let Some((worker, _, _, _, _)) = review_by_node.get(&child.synthesis_node_id)
                    {
                        descendants.insert(*worker);
                    }
                }
            }
        }
        let observed_child_hashes = if node.node_kind == "cross_chunk" {
            node.child_hashes
                .iter()
                .filter(|hash| reviewed_subreview_hashes.contains(*hash))
                .cloned()
                .collect()
        } else {
            node.child_hashes
                .iter()
                .filter(|hash| reviewed_node_hashes.contains(*hash))
                .cloned()
                .collect()
        };
        let (worker, outcome, context_truncated, stored_lineage_hash, stored_separation) =
            review_by_node
                .get(&node.node_id)
                .cloned()
                .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        let descendant_workers = descendants.iter().copied().collect::<Vec<_>>();
        let descendant_worker_text = descendant_workers
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>();
        let recomputed_lineage_hash = hash_text_array_on(&mut *tx, &descendant_worker_text).await?;
        let recomputed_separation = !descendants.contains(&worker);
        if stored_lineage_hash != recomputed_lineage_hash
            || stored_separation != recomputed_separation
            || !recomputed_separation
        {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let node_primary_workers = descendants
            .iter()
            .filter(|worker| primary_workers.contains(worker))
            .copied()
            .collect::<Vec<_>>();
        full_descendants.insert(node.node_hash.clone(), descendants);
        result.push(RecomputedCoverageSynthesisGateNodeRow {
            node_hash: node.node_hash,
            node_kind: node.node_kind.to_owned(),
            expected_child_hashes: node.child_hashes,
            observed_child_hashes,
            synthesis_worker_run_id: worker,
            primary_analyst_worker_run_ids: node_primary_workers,
            transitive_descendant_worker_run_ids: descendant_workers,
            outcome,
            context_truncated,
        });
    }
    Ok(result)
}

#[derive(Debug, Clone)]
pub struct RecordCoverageSynthesisReviewInput {
    pub fence: CandidateWriteFenceRow,
    pub stable_review_request_id: Uuid,
    pub synthesis_census_id: Uuid,
    pub synthesis_census_member_id: Uuid,
    pub node_kind: String,
    pub outcome: String,
    /// Compatibility field name: values are server checklist-member IDs
    /// reachable from this synthesis node.
    pub missed_proposal_ids: Vec<Uuid>,
    pub blocker_codes: Vec<String>,
    pub semantic_summary: Value,
    pub review_notes: String,
    pub provider_attempt_id: Option<Uuid>,
    pub provider_artifact_body: Option<Value>,
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
    relationship_cross_index_hash: String,
    node_hash: String,
    covered_input_count: i64,
    covered_input_set_hash: String,
    covered_checklist_count: i64,
    covered_checklist_set_hash: String,
    child_receipt_count: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct SynthesisChildSemanticDbRow {
    semantic_summary: Value,
    semantic_summary_hash: String,
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
    if input.provider_attempt_id.is_some() != input.provider_artifact_body.is_some() {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let mut tx = pool.begin().await?;
    validate_write_fence_on(&mut tx, &input.fence).await?;
    let node = sqlx::query_as::<_, SynthesisReviewNodeDbRow>(
        r#"SELECT node_kind,level,
        relationship_cross_index_hash,node_hash,
        covered_input_count,covered_input_set_hash,
        covered_checklist_count,covered_checklist_set_hash,child_receipt_count
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
          AND capability IN (
              'coverage_cross_chunk_synthesis',
              'coverage_cross_input_partition',
              'coverage_cross_input_reduce',
              'coverage_cross_dimension_reduce',
              'coverage_global_semantic_root'
          )
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
    let transitive_worker_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"WITH RECURSIVE descendant_nodes(synthesis_node_id) AS (
               SELECT child_synthesis_node_id
                 FROM candidate_analysis_hypothesis_coverage_synthesis_node_children
                WHERE synthesis_node_id=$1 AND child_synthesis_node_id IS NOT NULL
               UNION
               SELECT child.child_synthesis_node_id
                 FROM descendant_nodes prior
                 JOIN candidate_analysis_hypothesis_coverage_synthesis_node_children child
                   ON child.synthesis_node_id=prior.synthesis_node_id
                WHERE child.child_synthesis_node_id IS NOT NULL
           ), lineage(worker_run_id) AS (
               SELECT worker_run_id
                 FROM candidate_analysis_hypothesis_coverage_synthesis_node_descendant_workers
                WHERE synthesis_node_id=$1
               UNION
               SELECT review.synthesis_worker_run_id
                 FROM descendant_nodes descendant
                 JOIN candidate_analysis_hypothesis_coverage_synthesis_reviews review
                   ON review.synthesis_node_id=descendant.synthesis_node_id
           )
           SELECT worker_run_id FROM lineage ORDER BY worker_run_id"#,
    )
    .bind(input.synthesis_census_member_id)
    .fetch_all(&mut *tx)
    .await?;
    if transitive_worker_ids.is_empty()
        || transitive_worker_ids.contains(&input.fence.worker_run_id)
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let transitive_worker_text = transitive_worker_ids
        .iter()
        .map(Uuid::to_string)
        .collect::<Vec<_>>();
    let transitive_worker_set_hash = hash_text_array_on(&mut tx, &transitive_worker_text).await?;
    input.missed_proposal_ids.sort_unstable();
    input.missed_proposal_ids.dedup();
    input.blocker_codes.sort_unstable();
    input.blocker_codes.dedup();
    let child_summaries = sqlx::query_as::<_, SynthesisChildSemanticDbRow>(
        r#"SELECT COALESCE(subreview.semantic_summary,synthesis.semantic_summary) AS semantic_summary,
                  COALESCE(subreview.semantic_summary_hash,synthesis.semantic_summary_hash) AS semantic_summary_hash
              FROM candidate_analysis_hypothesis_coverage_synthesis_node_children child
              LEFT JOIN candidate_analysis_hypothesis_coverage_subreviews subreview
                ON subreview.subreview_id=child.child_subreview_id
              LEFT JOIN candidate_analysis_hypothesis_coverage_synthesis_reviews synthesis
                ON synthesis.synthesis_node_id=child.child_synthesis_node_id
             WHERE child.synthesis_node_id=$1
             ORDER BY child.ordinal"#,
    )
    .bind(input.synthesis_census_member_id)
    .fetch_all(&mut *tx)
    .await?;
    if child_summaries.is_empty()
        || child_summaries.len() > 32
        || child_summaries.len() as i64 != node.child_receipt_count
    {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let mut covered_input_ids = BTreeSet::new();
    let mut covered_checklist_ids = BTreeSet::new();
    let mut observed_proposal_ids = BTreeSet::new();
    let mut child_missed_ids = BTreeSet::new();
    let mut child_blocker_codes = BTreeSet::new();
    let mut child_observations = BTreeSet::new();
    for child in child_summaries {
        if hash_json_on(&mut tx, &child.semantic_summary).await? != child.semantic_summary_hash {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let summary: CoverageSemanticSummaryV1 = serde_json::from_value(child.semantic_summary)
            .map_err(|_| conflict(CENSUS_NOT_CLOSED))?;
        covered_input_ids.extend(summary.covered_input_ids);
        covered_checklist_ids.extend(summary.covered_checklist_member_ids);
        observed_proposal_ids.extend(summary.observed_proposal_ids);
        child_missed_ids.extend(summary.missed_checklist_member_ids);
        child_blocker_codes.extend(summary.blocker_codes);
        for observation in summary.semantic_observations {
            child_observations.insert(
                serde_json::to_string(&observation).map_err(|_| conflict(CENSUS_NOT_CLOSED))?,
            );
        }
    }
    let covered_input_text = covered_input_ids
        .iter()
        .map(Uuid::to_string)
        .collect::<Vec<_>>();
    let covered_checklist_text = covered_checklist_ids
        .iter()
        .map(Uuid::to_string)
        .collect::<Vec<_>>();
    if covered_input_ids.len() as i64 != node.covered_input_count
        || hash_text_array_on(&mut tx, &covered_input_text).await? != node.covered_input_set_hash
        || covered_checklist_ids.len() as i64 != node.covered_checklist_count
        || hash_text_array_on(&mut tx, &covered_checklist_text).await?
            != node.covered_checklist_set_hash
        || input
            .missed_proposal_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            != child_missed_ids
        || input.blocker_codes.iter().cloned().collect::<BTreeSet<_>>() != child_blocker_codes
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let valid = match input.outcome.as_str() {
        "no_composite_miss" => {
            input.missed_proposal_ids.is_empty() && input.blocker_codes.is_empty()
        }
        "missed_hypothesis" => {
            !input.missed_proposal_ids.is_empty() && input.blocker_codes.is_empty()
        }
        "blocked" => {
            input.missed_proposal_ids.is_empty()
                && !input.blocker_codes.is_empty()
                && input
                    .blocker_codes
                    .iter()
                    .all(|value| !value.trim().is_empty())
        }
        _ => false,
    };
    if !valid {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let context_truncated = input
        .blocker_codes
        .iter()
        .any(|code| code == "context_truncated");
    if context_truncated && input.outcome != "blocked" {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let (semantic_observation_count, semantic_summary_hash) =
        validate_coverage_semantic_summary_on(
            &mut tx,
            input.fence.analysis_attempt_id,
            &input.semantic_summary,
            &covered_input_ids,
            &covered_checklist_ids,
            &observed_proposal_ids,
            &child_missed_ids,
            &child_blocker_codes,
            Some(&child_observations),
        )
        .await?;
    let body = json!({"kind":"hypothesis_coverage_synthesis.v1","synthesis_census_id":input.synthesis_census_id,
        "synthesis_node_id":input.synthesis_census_member_id,"node_kind":input.node_kind,
        "outcome":input.outcome,"typed_missed_refs":input.missed_proposal_ids,
        "blocker_codes":&input.blocker_codes,"semantic_summary":&input.semantic_summary,
        "semantic_summary_hash":&semantic_summary_hash,"review_notes":input.review_notes});
    let synthesis_hash = hash_json_on(
        &mut tx,
        &json!({"body":body,"node_hash":node.node_hash,
        "relationship_cross_index_hash":node.relationship_cross_index_hash,
        "transitive_descendant_worker_set_hash":transitive_worker_set_hash,
        "synthesis_worker_run_id":input.fence.worker_run_id,"worker_separation_valid":true,
        "context_truncated":context_truncated}),
    )
    .await?;
    let review_id = Uuid::new_v5(
        &input.stable_review_request_id,
        input.synthesis_census_member_id.as_bytes(),
    );
    let artifact_id = Uuid::new_v5(
        &input.stable_review_request_id,
        b"hypothesis_coverage_synthesis.v1",
    );
    let artifact_hash = hash_json_on(&mut tx, &body).await?;
    let output_id = Uuid::new_v5(&artifact_id, b"candidate_stage_worker_output.v1");
    let existing:Option<(Uuid,String)>=sqlx::query_as("SELECT synthesis_review_id,review_hash FROM candidate_analysis_hypothesis_coverage_synthesis_reviews WHERE synthesis_node_id=$1")
        .bind(input.synthesis_census_member_id).fetch_optional(&mut *tx).await?;
    let replayed = existing.is_some();
    if let Some((id, hash)) = existing {
        if id != review_id || hash != synthesis_hash {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
    } else {
        sqlx::query(r#"INSERT INTO candidate_analysis_artifacts(
            artifact_id,analysis_attempt_id,candidate_work_item_id,worker_run_id,stage_worker_output_id,
            artifact_kind,artifact_body,artifact_hash)
            VALUES($1,$2,$3,$4,$5,'hypothesis_coverage_synthesis.v1',$6,$7)"#).bind(artifact_id)
            .bind(input.fence.analysis_attempt_id).bind(candidate_item_id).bind(input.fence.worker_run_id)
            .bind(input.provider_attempt_id.map(|_| output_id))
            .bind(&body).bind(&artifact_hash).execute(&mut *tx).await?;
        sqlx::query(
            r#"INSERT INTO candidate_analysis_hypothesis_coverage_synthesis_reviews(
            synthesis_review_id,synthesis_node_id,synthesis_census_id,analysis_attempt_id,
            synthesis_worker_run_id,transitive_descendant_worker_set_hash,worker_separation_valid,
            context_truncated,outcome,typed_missed_refs,blocker_codes,semantic_summary,
            semantic_observation_count,semantic_summary_hash,review_hash)
            VALUES($1,$2,$3,$4,$5,$6,TRUE,$7,$8,$9,$10,$11,$12,$13,$14)"#,
        )
        .bind(review_id)
        .bind(input.synthesis_census_member_id)
        .bind(input.synthesis_census_id)
        .bind(input.fence.analysis_attempt_id)
        .bind(input.fence.worker_run_id)
        .bind(&transitive_worker_set_hash)
        .bind(context_truncated)
        .bind(&input.outcome)
        .bind(json!(input.missed_proposal_ids))
        .bind(&input.blocker_codes)
        .bind(&input.semantic_summary)
        .bind(semantic_observation_count)
        .bind(&semantic_summary_hash)
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
    if let (Some(provider_attempt_id), Some(provider_artifact_body)) = (
        input.provider_attempt_id,
        input.provider_artifact_body.as_ref(),
    ) {
        persist_provider_artifact_receipt_on(
            &mut tx,
            &input.fence,
            provider_attempt_id,
            provider_artifact_body,
            artifact_id,
            "hypothesis_coverage_synthesis.v1",
            &artifact_hash,
        )
        .await?;
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
    let synthesis:(Uuid,i64,Uuid)=sqlx::query_as(r#"SELECT census.synthesis_census_id,
        (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_synthesis_census_members member
          WHERE member.synthesis_census_id=census.synthesis_census_id),census.global_root_node_id
        FROM candidate_analysis_hypothesis_coverage_synthesis_censuses census WHERE census.analysis_attempt_id=$1"#)
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
    let artifact_id = Uuid::new_v5(
        &input.stable_reduction_request_id,
        b"hypothesis_coverage_review.v1",
    );
    let existing:Option<(Uuid,Uuid,String,String)>=sqlx::query_as(r#"SELECT coverage_review_id,artifact_id,outcome,review_hash
        FROM candidate_analysis_hypothesis_coverage_reviews WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2"#)
        .bind(input.fence.analysis_attempt_id).bind(input.input_id).fetch_optional(&mut *tx).await?;
    let replayed = existing.is_some();
    if let Some((id, existing_artifact_id, existing_outcome, hash)) = existing {
        if id != review_id
            || existing_artifact_id != artifact_id
            || existing_outcome != outcome
            || hash != review_hash
        {
            return Err(conflict(SNAPSHOT_REPLAY_DRIFT));
        }
    } else {
        let candidate_item_id = candidate_item_id_on(
            &mut tx,
            input.fence.analysis_attempt_id,
            input.fence.work_item_id,
        )
        .await?;
        let artifact_hash = hash_json_on(&mut tx, &body).await?;
        sqlx::query(r#"INSERT INTO candidate_analysis_artifacts(
            artifact_id,analysis_attempt_id,candidate_work_item_id,worker_run_id,artifact_kind,artifact_body,artifact_hash)
            VALUES($1,$2,$3,$4,'hypothesis_coverage_review.v1',$5,$6)"#).bind(artifact_id)
            .bind(input.fence.analysis_attempt_id).bind(candidate_item_id).bind(input.fence.worker_run_id)
            .bind(&body).bind(artifact_hash).execute(&mut *tx).await?;
        sqlx::query(r#"INSERT INTO candidate_analysis_hypothesis_coverage_reviews(
            coverage_review_id,artifact_id,analysis_attempt_id,snapshot_input_id,attempt_ordinal,chunk_census_id,
            chunk_partition_count,chunk_partition_set_hash,subreview_census_id,read_receipt_set_hash,
            h1_proposal_ref_count,h1_proposal_ref_set_hash,attack_class_checklist_version,
            attack_class_checklist_digest,trust_boundary_checklist_version,trust_boundary_checklist_digest,
            checklist_member_set_hash,synthesis_census_id,global_review_id,coverage_sampling_contract_version,
            coverage_sampling_contract_digest,worker_separation_set_hash,review_mode,outcome,
            checklist_dispositions,typed_missed_refs,review_hash)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27)"#)
            .bind(review_id).bind(artifact_id).bind(input.fence.analysis_attempt_id).bind(input.input_id).bind(attempt_policy.2)
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

/// Recomputes Candidate analysis closure from raw authority rows.  Callers
/// must invoke this on the same transaction that consumes the returned
/// hashes; copying persisted census headers is deliberately insufficient.
pub async fn validate_candidate_analysis_exact_closure_on(
    tx: &mut Transaction<'_, Postgres>,
    analysis_attempt_id: Uuid,
    snapshot_id: Uuid,
) -> Result<CandidateAnalysisExactClosureRow> {
    let attempt = sqlx::query_as::<_, ExactClosureAttemptDbRow>(
        r#"SELECT snapshot_id,attack_class_checklist_version,
                  attack_class_checklist_digest,trust_boundary_checklist_version,
                  trust_boundary_checklist_digest
             FROM candidate_analysis_attempts
            WHERE analysis_attempt_id=$1
            FOR SHARE"#,
    )
    .bind(analysis_attempt_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    if attempt.snapshot_id != snapshot_id {
        return Err(conflict(AUTHORITY_MISMATCH));
    }

    let inputs = sqlx::query_as::<_, ExactClosureInputDbRow>(
        r#"SELECT snapshot_input_id,source_ref,
                  subject_kind_at_time,subject_identity_hash,
                  server_chunking_disposition,input_hash
             FROM candidate_analysis_snapshot_inputs
            WHERE snapshot_id=$1
            ORDER BY stable_input_key,snapshot_input_id"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    if inputs.is_empty() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let all_input_hashes = inputs
        .iter()
        .map(|input| input.input_hash.clone())
        .collect::<Vec<_>>();
    let complete_inputs = inputs
        .iter()
        .filter(|input| input.server_chunking_disposition == "complete")
        .cloned()
        .collect::<Vec<_>>();
    let complete_input_hashes = complete_inputs
        .iter()
        .map(|input| input.input_hash.clone())
        .collect::<Vec<_>>();
    let all_input_set_hash = hash_text_array_on(tx, &all_input_hashes).await?;
    let complete_input_set_hash = hash_text_array_on(tx, &complete_input_hashes).await?;
    let complete_input_count = i64::try_from(complete_inputs.len()).unwrap_or(i64::MAX);
    let h1_work_authority_closed: bool = sqlx::query_scalar(
        r#"SELECT
            (SELECT COUNT(*)
               FROM candidate_analysis_work_items candidate
              WHERE candidate.analysis_attempt_id=$1
                AND candidate.phase='controller'
                AND candidate.capability='candidate_controller_dispatch')=1
            AND
            (SELECT COUNT(*)
               FROM candidate_analysis_work_items candidate
               JOIN stage_work_items item
                 ON item.id=candidate.stage_work_item_id
                AND item.kind='candidate_controller_dispatch'
                AND item.role='controller'
                AND item.status='completed'
               JOIN stage_worker_runs worker
                 ON worker.work_item_id=item.id
                AND worker.specialist='controller'
                AND worker.work_item_kind='candidate_controller_dispatch'
                AND worker.status='passed'
               JOIN candidate_analysis_provider_attempts receipt
                 ON receipt.analysis_attempt_id=candidate.analysis_attempt_id
                AND receipt.stage_work_item_id=item.id
                AND receipt.worker_run_id=worker.id
                AND receipt.artifact_kind='controller_dispatch.v1'
                AND receipt.artifact_id IS NULL
                AND receipt.artifact_hash=tool_truth_sha256(receipt.artifact_body::TEXT)
              WHERE candidate.analysis_attempt_id=$1
                AND candidate.phase='controller'
                AND candidate.capability='candidate_controller_dispatch'
                AND (SELECT COUNT(*) FROM stage_worker_runs all_worker
                      WHERE all_worker.work_item_id=item.id)=1
                AND (SELECT COUNT(*) FROM candidate_analysis_artifacts artifact
                      WHERE artifact.candidate_work_item_id=candidate.candidate_work_item_id
                        AND artifact.analysis_attempt_id=candidate.analysis_attempt_id
                        AND NOT (
                            artifact.artifact_kind='hypothesis_coverage_review.v1'
                            AND artifact.worker_run_id=worker.id
                            AND artifact.stage_worker_output_id IS NULL
                            AND EXISTS (
                                SELECT 1
                                  FROM candidate_analysis_hypothesis_coverage_reviews review
                                 WHERE review.artifact_id=artifact.artifact_id
                                   AND review.analysis_attempt_id=artifact.analysis_attempt_id
                                   AND artifact.artifact_body=jsonb_build_object(
                                       'kind','hypothesis_coverage_review.v1',
                                       'analysis_attempt_id',review.analysis_attempt_id,
                                       'snapshot_input_id',review.snapshot_input_id,
                                       'outcome',review.outcome,
                                       'review_mode',review.review_mode,
                                       'checklist_dispositions',review.checklist_dispositions,
                                       'typed_missed_refs',review.typed_missed_refs
                                   )
                                   AND artifact.artifact_hash=
                                       tool_truth_sha256(artifact.artifact_body::TEXT)
                            )
                        ))=0
                AND (SELECT COUNT(*) FROM stage_worker_outputs output
                      WHERE output.work_item_id=item.id)=0)=1
            AND
            (SELECT COUNT(*)
               FROM candidate_analysis_work_items candidate
              WHERE candidate.analysis_attempt_id=$1
                AND candidate.phase='proposal'
                AND candidate.capability='hypothesis_proposal')=$3
            AND
            (SELECT COUNT(DISTINCT candidate.microbatch_key)
               FROM candidate_analysis_work_items candidate
               JOIN candidate_analysis_input_chunk_censuses census
                 ON census.snapshot_id=$2
                AND census.snapshot_input_id::TEXT=candidate.microbatch_key
                AND census.disposition='complete'
              WHERE candidate.analysis_attempt_id=$1
                AND candidate.phase='proposal'
                AND candidate.capability='hypothesis_proposal')=$3
            AND
            (SELECT COUNT(*)
               FROM candidate_analysis_work_items candidate
               JOIN candidate_analysis_input_chunk_censuses census
                 ON census.snapshot_id=$2
                AND census.snapshot_input_id::TEXT=candidate.microbatch_key
                AND census.disposition='complete'
               JOIN stage_work_items item
                 ON item.id=candidate.stage_work_item_id
                AND item.kind='hypothesis_proposal'
                AND item.role='analyst'
                AND item.status='completed'
               JOIN stage_worker_runs worker
                 ON worker.work_item_id=item.id
                AND worker.specialist='analyst'
                AND worker.work_item_kind='hypothesis_proposal'
                AND worker.status='passed'
               JOIN candidate_analysis_provider_attempts receipt
                 ON receipt.analysis_attempt_id=candidate.analysis_attempt_id
                AND receipt.stage_work_item_id=item.id
                AND receipt.worker_run_id=worker.id
                AND receipt.artifact_kind='hypothesis_proposal.v1'
               JOIN candidate_analysis_artifacts artifact
                 ON artifact.artifact_id=receipt.artifact_id
                AND artifact.analysis_attempt_id=receipt.analysis_attempt_id
                AND artifact.candidate_work_item_id=candidate.candidate_work_item_id
                AND artifact.worker_run_id=worker.id
                AND artifact.artifact_kind='hypothesis_proposal.v1'
                AND artifact.artifact_id=uuid_generate_v5(
                        receipt.provider_attempt_id,'hypothesis_proposal.v1'
                    )
                AND artifact.artifact_body=receipt.artifact_body
                AND artifact.artifact_hash=receipt.artifact_hash
                AND artifact.artifact_hash=tool_truth_sha256(artifact.artifact_body::TEXT)
                AND receipt.artifact_hash=tool_truth_sha256(receipt.artifact_body::TEXT)
                AND artifact.artifact_body=jsonb_build_object(
                        'proposals',artifact.artifact_body->'proposals'
                    )
                AND jsonb_typeof(artifact.artifact_body->'proposals')='array'
                AND (SELECT COUNT(*) FROM hypothesis_proposals proposal
                      WHERE proposal.artifact_id=artifact.artifact_id
                        AND proposal.analysis_attempt_id=artifact.analysis_attempt_id)
                    =CASE
                        WHEN jsonb_typeof(artifact.artifact_body->'proposals')='array'
                            THEN jsonb_array_length(artifact.artifact_body->'proposals')
                        ELSE -1
                     END
                AND NOT EXISTS (
                    SELECT 1
                      FROM jsonb_array_elements(CASE
                               WHEN jsonb_typeof(artifact.artifact_body->'proposals')='array'
                                   THEN artifact.artifact_body->'proposals'
                               ELSE '[]'::JSONB
                           END) body(proposal_body)
                      LEFT JOIN hypothesis_proposals proposal
                        ON proposal.artifact_id=artifact.artifact_id
                       AND proposal.analysis_attempt_id=artifact.analysis_attempt_id
                       AND proposal.proposal_id::TEXT=body.proposal_body->>'proposal_id'
                       AND proposal.structured_proposal=body.proposal_body
                       AND proposal.proposal_hash=tool_truth_sha256(body.proposal_body::TEXT)
                     WHERE proposal.proposal_id IS NULL
                )
                AND NOT EXISTS (
                    SELECT 1
                      FROM hypothesis_proposals proposal
                     WHERE proposal.artifact_id=artifact.artifact_id
                       AND proposal.analysis_attempt_id=artifact.analysis_attempt_id
                       AND NOT EXISTS (
                           SELECT 1
                             FROM jsonb_array_elements(CASE
                                      WHEN jsonb_typeof(artifact.artifact_body->'proposals')='array'
                                          THEN artifact.artifact_body->'proposals'
                                      ELSE '[]'::JSONB
                                  END) body(proposal_body)
                            WHERE proposal.proposal_id::TEXT=body.proposal_body->>'proposal_id'
                              AND proposal.structured_proposal=body.proposal_body
                              AND proposal.proposal_hash=tool_truth_sha256(body.proposal_body::TEXT)
                       )
                )
               JOIN stage_worker_outputs output
                 ON output.id=artifact.stage_worker_output_id
                AND output.id=uuid_generate_v5(
                        artifact.artifact_id,'candidate_stage_worker_output.v1'
                    )
                AND output.work_item_id=item.id
                AND output.worker_run_id=worker.id
                AND output.output_schema='candidate_analysis_artifact_receipt.v1'
                AND output.output_version=1
                AND output.business_disposition='artifact_recorded'
                AND output.canonical_output=jsonb_build_object(
                        'schema','candidate_analysis_artifact_receipt.v1',
                        'artifact_id',artifact.artifact_id,
                        'artifact_hash',artifact.artifact_hash
                    )
                AND output.canonical_fact_refs='[]'::JSONB
                AND cardinality(output.evidence_ids)=0
                AND output.checked_empty_cells='[]'::JSONB
                AND cardinality(output.blocker_codes)=0
                AND output.output_hash=tool_truth_sha256(output.canonical_output::TEXT)
              WHERE candidate.analysis_attempt_id=$1
                AND candidate.phase='proposal'
                AND candidate.capability='hypothesis_proposal'
                AND (SELECT COUNT(*) FROM stage_worker_runs all_worker
                      WHERE all_worker.work_item_id=item.id)=1
                AND (SELECT COUNT(*) FROM candidate_analysis_artifacts all_artifact
                      WHERE all_artifact.candidate_work_item_id=candidate.candidate_work_item_id
                        AND all_artifact.analysis_attempt_id=candidate.analysis_attempt_id)=1)=$3"#,
    )
    .bind(analysis_attempt_id)
    .bind(snapshot_id)
    .bind(complete_input_count)
    .fetch_one(&mut **tx)
    .await?;
    if !h1_work_authority_closed {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }

    let proposals: Vec<(Uuid, i32, String)> = sqlx::query_as(
        r#"SELECT proposal_id,proposal_ordinal,proposal_hash
             FROM hypothesis_proposals
            WHERE analysis_attempt_id=$1
            ORDER BY proposal_ordinal,proposal_id"#,
    )
    .bind(analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    if proposals
        .iter()
        .enumerate()
        .any(|(ordinal, proposal)| proposal.1 != i32::try_from(ordinal).unwrap_or(i32::MAX))
    {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let proposal_ref_drift: i64 = sqlx::query_scalar(
        r#"SELECT count(*)
              FROM hypothesis_proposal_refs reference
              JOIN hypothesis_proposals proposal ON proposal.proposal_id=reference.proposal_id
              JOIN candidate_analysis_snapshot_inputs input
                ON input.snapshot_input_id=reference.snapshot_input_id
              LEFT JOIN candidate_analysis_input_chunk_census_members chunk
                ON chunk.chunk_id=reference.chunk_id
             WHERE proposal.analysis_attempt_id=$1
               AND (
                   reference.analysis_attempt_id<>$1
                   OR input.snapshot_id<>$2
                   OR reference.source_hash<>input.source_content_hash
                   OR reference.chunk_id IS NULL
                   OR chunk.chunk_id IS NULL
                   OR chunk.snapshot_input_id<>reference.snapshot_input_id
                   OR chunk.snapshot_id<>input.snapshot_id
                   OR reference.ref_hash<>tool_truth_sha256(jsonb_build_object(
                       'proposal_id',reference.proposal_id,
                       'snapshot_input_id',reference.snapshot_input_id,
                       'chunk_id',reference.chunk_id,
                       'source_role',reference.source_role,
                       'source_hash',input.source_content_hash
                   )::TEXT)
               )"#,
    )
    .bind(analysis_attempt_id)
    .bind(snapshot_id)
    .fetch_one(&mut **tx)
    .await?;
    if proposal_ref_drift != 0 {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let proposal_hashes = proposals
        .iter()
        .map(|proposal| proposal.2.clone())
        .collect::<Vec<_>>();
    let proposal_set_hash = hash_text_array_on(tx, &proposal_hashes).await?;
    let proposal_census_hash = hash_json_on(
        tx,
        &json!({
            "kind":"proposal",
            "attempt":analysis_attempt_id,
            "count":proposals.len(),
            "set":proposal_set_hash,
        }),
    )
    .await?;
    let proposal_header: (Uuid, i64, String, String) = sqlx::query_as(
        r#"SELECT proposal_census_id,proposal_count,proposal_set_hash,census_hash
             FROM candidate_analysis_proposal_censuses
            WHERE analysis_attempt_id=$1"#,
    )
    .bind(analysis_attempt_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    if proposal_header.1 != i64::try_from(proposals.len()).unwrap_or(i64::MAX)
        || proposal_header.2 != proposal_set_hash
        || proposal_header.3 != proposal_census_hash
    {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let proposal_members: Vec<(Uuid, Uuid, i32, String, String)> = sqlx::query_as(
        r#"SELECT census_member_id,proposal_id,ordinal,proposal_hash,member_hash
             FROM candidate_analysis_proposal_census_members
            WHERE proposal_census_id=$1 AND analysis_attempt_id=$2
            ORDER BY ordinal,proposal_id"#,
    )
    .bind(proposal_header.0)
    .bind(analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    if proposal_members.len() != proposals.len() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    for (ordinal, (member, proposal)) in proposal_members.iter().zip(proposals.iter()).enumerate() {
        let expected_member_hash = hash_json_on(
            tx,
            &json!({"proposal_id":proposal.0,"proposal_hash":proposal.2}),
        )
        .await?;
        if member.0 != Uuid::new_v5(&proposal_header.0, expected_member_hash.as_bytes())
            || member.1 != proposal.0
            || member.2 != i32::try_from(ordinal).unwrap_or(i32::MAX)
            || member.3 != proposal.2
            || member.4 != expected_member_hash
        {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
    }

    let persisted_dispositions: Vec<(Uuid, i64, String, String, Option<String>, String)> =
        sqlx::query_as(
            r#"SELECT snapshot_input_id,proposal_ref_count,proposal_ref_set_hash,
                      disposition,blocker_code,disposition_hash
                 FROM candidate_analysis_input_proposal_dispositions
                WHERE analysis_attempt_id=$1
                ORDER BY snapshot_input_id"#,
        )
        .bind(analysis_attempt_id)
        .fetch_all(&mut **tx)
        .await?;
    if persisted_dispositions.len() != inputs.len() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let disposition_by_input = persisted_dispositions
        .iter()
        .map(|row| (row.0, row))
        .collect::<BTreeMap<_, _>>();
    let mut blocked_input_ids = Vec::new();
    for input in &inputs {
        let reference_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT ref_hash FROM hypothesis_proposal_refs
                WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2
                ORDER BY ref_hash,proposal_ref_id"#,
        )
        .bind(analysis_attempt_id)
        .bind(input.snapshot_input_id)
        .fetch_all(&mut **tx)
        .await?;
        let reference_set_hash = hash_text_array_on(tx, &reference_hashes).await?;
        let persisted = disposition_by_input
            .get(&input.snapshot_input_id)
            .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        let expected_disposition = if input.server_chunking_disposition != "complete" {
            "blocked"
        } else if reference_hashes.is_empty() {
            "zero_proposal"
        } else {
            "has_proposal"
        };
        let expected_blocker = if input.server_chunking_disposition != "complete" {
            Some(format!(
                "candidate_input_{}",
                input.server_chunking_disposition
            ))
        } else {
            None
        };
        if expected_disposition == "blocked" {
            if !reference_hashes.is_empty()
                || expected_blocker
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
            {
                return Err(conflict(CENSUS_NOT_CLOSED));
            }
            blocked_input_ids.push(input.snapshot_input_id);
        }
        let expected_hash = hash_json_on(
            tx,
            &json!({
                "analysis_attempt_id":analysis_attempt_id,
                "snapshot_input_id":input.snapshot_input_id,
                "proposal_ref_set_hash":reference_set_hash,
                "disposition":expected_disposition,
                "blocker_code":expected_blocker,
            }),
        )
        .await?;
        if persisted.1 != i64::try_from(reference_hashes.len()).unwrap_or(i64::MAX)
            || persisted.2 != reference_set_hash
            || persisted.3 != expected_disposition
            || persisted.4.as_deref() != expected_blocker.as_deref()
            || persisted.5 != expected_hash
        {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
    }
    let disposition_hashes = persisted_dispositions
        .iter()
        .map(|disposition| disposition.5.clone())
        .collect::<Vec<_>>();
    let h1_disposition_set_hash = hash_text_array_on(tx, &disposition_hashes).await?;

    if !blocked_input_ids.is_empty() {
        let expected_reason = "candidate_noncomplete_input_blocked";
        blocked_input_ids.sort_unstable();
        let residual_id = Uuid::new_v5(
            &analysis_attempt_id,
            b"candidate_analysis_blocked_residual.v1",
        );
        let expected_residual_hash = hash_json_on(
            tx,
            &json!({
                "analysis_attempt_id":analysis_attempt_id,
                "snapshot_id":snapshot_id,
                "reason_code":expected_reason,
                "affected_input_ids":blocked_input_ids,
            }),
        )
        .await?;
        let residual: Option<(String, Value, Value, String)> = sqlx::query_as(
            r#"SELECT reason_code,affected_inputs,next_action,residual_hash
                 FROM hypothesis_residual_risks
                WHERE residual_id=$1 AND snapshot_id=$2
                  AND owner_kind='candidate_analysis'"#,
        )
        .bind(residual_id)
        .bind(snapshot_id)
        .fetch_optional(&mut **tx)
        .await?;
        let residual = residual.ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        if residual.0 != expected_reason
            || residual.1 != json!(blocked_input_ids)
            || residual.2 != json!({"route":"candidate_analysis_closeout","retry":false})
            || residual.3 != expected_residual_hash
        {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let opened_event_id: Uuid = sqlx::query_scalar(
            r#"SELECT attempt_event_id FROM candidate_analysis_attempt_state_events
                WHERE analysis_attempt_id=$1 AND event_kind='opened'"#,
        )
        .bind(analysis_attempt_id)
        .fetch_one(&mut **tx)
        .await?;
        let expected_event_hash = hash_json_on(
            tx,
            &json!({
                "attempt":analysis_attempt_id,
                "ordinal":1,
                "event":"blocked",
                "predecessor_event_id":opened_event_id,
                "residual_hash":expected_residual_hash,
            }),
        )
        .await?;
        let blocked_event: Option<(i32, Uuid, String)> = sqlx::query_as(
            r#"SELECT event_ordinal,predecessor_event_id,event_hash
                 FROM candidate_analysis_attempt_state_events
                WHERE analysis_attempt_id=$1 AND event_kind='blocked'"#,
        )
        .bind(analysis_attempt_id)
        .fetch_optional(&mut **tx)
        .await?;
        if blocked_event != Some((1, opened_event_id, expected_event_hash)) {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let forbidden_h2_count: i64 = sqlx::query_scalar(
            r#"SELECT
                (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_checklist_members WHERE analysis_attempt_id=$1)
              + (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_chunk_partitions WHERE analysis_attempt_id=$1)
              + (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_subreview_censuses WHERE analysis_attempt_id=$1)
              + (SELECT count(*) FROM candidate_analysis_critic_censuses WHERE analysis_attempt_id=$1)"#,
        )
        .bind(analysis_attempt_id)
        .fetch_one(&mut **tx)
        .await?;
        if forbidden_h2_count != 0 {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let empty_hash = hash_text_array_on(tx, &[]).await?;
        return Ok(CandidateAnalysisExactClosureRow {
            all_input_count: i64::try_from(inputs.len()).unwrap_or(i64::MAX),
            complete_input_count: i64::try_from(complete_inputs.len()).unwrap_or(i64::MAX),
            all_input_set_hash,
            complete_input_set_hash,
            proposal_census_hash,
            h1_disposition_set_hash,
            coverage_checklist_set_hash: empty_hash.clone(),
            coverage_partition_set_hash: empty_hash.clone(),
            coverage_subreview_census_set_hash: empty_hash,
            page_receipt_set_hash: hash_text_array_on(tx, &[]).await?,
            critic_census_hash: None,
            gate_eligible: false,
        });
    }

    validate_candidate_analysis_complete_closure_on(
        tx,
        analysis_attempt_id,
        snapshot_id,
        &attempt,
        &inputs,
        all_input_set_hash,
        complete_input_set_hash,
        proposal_census_hash,
        h1_disposition_set_hash,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn validate_candidate_analysis_complete_closure_on(
    tx: &mut Transaction<'_, Postgres>,
    analysis_attempt_id: Uuid,
    snapshot_id: Uuid,
    attempt: &ExactClosureAttemptDbRow,
    inputs: &[ExactClosureInputDbRow],
    all_input_set_hash: String,
    complete_input_set_hash: String,
    proposal_census_hash: String,
    h1_disposition_set_hash: String,
) -> Result<CandidateAnalysisExactClosureRow> {
    if inputs
        .iter()
        .any(|input| input.server_chunking_disposition != "complete")
    {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let boundaries: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT DISTINCT subject_kind_at_time,subject_identity_hash
             FROM candidate_analysis_snapshot_inputs
            WHERE snapshot_id=$1
            ORDER BY subject_kind_at_time,subject_identity_hash"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    if boundaries.is_empty() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let attack_digest = hash_json_on(tx, &candidate_attack_class_catalog_manifest_v1()).await?;
    let boundary_digest = hash_json_on(
        tx,
        &json!({
            "contract":"trust_boundary.v1",
            "version":1,
            "boundaries":boundaries.iter().map(|boundary| json!({
                "identity":boundary.0,
                "hash":boundary.1,
            })).collect::<Vec<_>>(),
        }),
    )
    .await?;
    if attempt.attack_class_checklist_version != "attack_class.v1"
        || attempt.attack_class_checklist_digest != attack_digest
        || attempt.trust_boundary_checklist_version != "trust_boundary.v1"
        || attempt.trust_boundary_checklist_digest != boundary_digest
    {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }

    let persisted_checklist = sqlx::query_as::<_, ExactClosureChecklistDbRow>(
        r#"SELECT checklist_member_id,snapshot_input_id,ordinal,
                  attack_class_contract_version,attack_class_contract_digest,
                  trust_boundary_contract_version,trust_boundary_contract_digest,
                  attack_class_id,attack_class_version,trust_boundary_identity,
                  trust_boundary_hash,applicability_basis,feed_match_member_refs,
                  applicability_disposition,enrichment_obligation_id,member_hash
             FROM candidate_analysis_hypothesis_coverage_checklist_members
            WHERE analysis_attempt_id=$1
            ORDER BY snapshot_input_id,ordinal"#,
    )
    .bind(analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    let expected_checklist_count = inputs
        .len()
        .saturating_mul(CANDIDATE_ATTACK_CLASS_CATALOG_V1.len())
        .saturating_mul(boundaries.len());
    if persisted_checklist.len() != expected_checklist_count {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let checklist_by_key = persisted_checklist
        .iter()
        .map(|member| ((member.snapshot_input_id, member.ordinal), member))
        .collect::<BTreeMap<_, _>>();
    let mut checklist_ids_by_input = BTreeMap::<Uuid, Vec<(Uuid, i32, String)>>::new();
    for input in inputs {
        let product_member_id = input
            .source_ref
            .strip_prefix("candidate_product_version_member:")
            .and_then(|value| Uuid::parse_str(value).ok());
        let feed_member_id = input
            .source_ref
            .strip_prefix("candidate_feed_snapshot_member:")
            .and_then(|value| Uuid::parse_str(value).ok());
        let enrichment_obligation_id: Option<Uuid> =
            if let Some(product_member_id) = product_member_id {
                sqlx::query_scalar(
                    r#"SELECT obligation_id FROM candidate_analysis_enrichment_obligations
                    WHERE snapshot_id=$1 AND product_member_id=$2
                      AND obligation_kind='product_version_enrichment'"#,
                )
                .bind(snapshot_id)
                .bind(product_member_id)
                .fetch_optional(&mut **tx)
                .await?
            } else if let Some(feed_member_id) = feed_member_id {
                sqlx::query_scalar(
                    r#"SELECT obligation_id FROM candidate_analysis_enrichment_obligations
                    WHERE snapshot_id=$1 AND feed_snapshot_member_id=$2
                      AND obligation_kind IN ('feed_refresh','feed_matcher_upgrade')
                    ORDER BY obligation_kind LIMIT 1"#,
                )
                .bind(snapshot_id)
                .bind(feed_member_id)
                .fetch_optional(&mut **tx)
                .await?
            } else {
                None
            };
        let feed_match_refs: Vec<Uuid> = if let Some(product_member_id) = product_member_id {
            sqlx::query_scalar(
                r#"SELECT match_member_id FROM candidate_analysis_feed_match_census_members
                    WHERE snapshot_id=$1 AND product_member_id=$2 AND disposition='matched'
                    ORDER BY ordinal"#,
            )
            .bind(snapshot_id)
            .bind(product_member_id)
            .fetch_all(&mut **tx)
            .await?
        } else if let Some(feed_member_id) = feed_member_id {
            sqlx::query_scalar(
                r#"SELECT match_member_id FROM candidate_analysis_feed_match_census_members
                    WHERE snapshot_id=$1 AND feed_snapshot_member_id=$2 AND disposition='matched'
                    ORDER BY ordinal"#,
            )
            .bind(snapshot_id)
            .bind(feed_member_id)
            .fetch_all(&mut **tx)
            .await?
        } else {
            Vec::new()
        };
        let applicability_disposition = if enrichment_obligation_id.is_some() {
            if product_member_id.is_some() {
                "blocked_product_version"
            } else {
                "blocked_feed_authority"
            }
        } else {
            "required"
        };
        let applicability_basis = json!({
            "source":"server_frozen_catalog_x_boundary",
            "input_subject_kind":input.subject_kind_at_time,
            "input_subject_identity_hash":input.subject_identity_hash,
        });
        let mut ordinal = 0_i32;
        for (attack_class_id, attack_class_version) in CANDIDATE_ATTACK_CLASS_CATALOG_V1 {
            for (boundary_identity, boundary_hash) in &boundaries {
                let persisted = checklist_by_key
                    .get(&(input.snapshot_input_id, ordinal))
                    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
                let expected_id = Uuid::new_v5(
                    &analysis_attempt_id,
                    format!(
                        "checklist:{}:{attack_class_id}:{attack_class_version}:{boundary_identity}:{boundary_hash}",
                        input.snapshot_input_id
                    )
                    .as_bytes(),
                );
                let expected_hash = hash_json_on(
                    tx,
                    &json!({
                        "analysis_attempt_id":analysis_attempt_id,
                        "snapshot_input_id":input.snapshot_input_id,
                        "ordinal":ordinal,
                        "attack_class_id":attack_class_id,
                        "attack_class_version":attack_class_version,
                        "trust_boundary_identity":boundary_identity,
                        "trust_boundary_hash":boundary_hash,
                        "attack_class_contract_digest":attack_digest,
                        "trust_boundary_contract_digest":boundary_digest,
                        "feed_match_member_refs":feed_match_refs,
                        "applicability_disposition":applicability_disposition,
                        "enrichment_obligation_id":enrichment_obligation_id,
                    }),
                )
                .await?;
                if persisted.checklist_member_id != expected_id
                    || persisted.attack_class_contract_version != "attack_class.v1"
                    || persisted.attack_class_contract_digest != attack_digest
                    || persisted.trust_boundary_contract_version != "trust_boundary.v1"
                    || persisted.trust_boundary_contract_digest != boundary_digest
                    || persisted.attack_class_id != attack_class_id
                    || persisted.attack_class_version != attack_class_version
                    || persisted.trust_boundary_identity.as_str() != boundary_identity
                    || persisted.trust_boundary_hash.as_str() != boundary_hash
                    || persisted.applicability_basis != applicability_basis
                    || persisted.feed_match_member_refs.as_slice() != feed_match_refs.as_slice()
                    || persisted.applicability_disposition != applicability_disposition
                    || persisted.enrichment_obligation_id != enrichment_obligation_id
                    || persisted.member_hash != expected_hash
                {
                    return Err(conflict(CENSUS_NOT_CLOSED));
                }
                checklist_ids_by_input
                    .entry(input.snapshot_input_id)
                    .or_default()
                    .push((expected_id, ordinal, expected_hash));
                ordinal = ordinal
                    .checked_add(1)
                    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
            }
        }
    }
    let coverage_checklist_set_hash = hash_text_array_on(
        tx,
        &persisted_checklist
            .iter()
            .map(|member| member.member_hash.clone())
            .collect::<Vec<_>>(),
    )
    .await?;

    validate_candidate_analysis_partition_and_subreview_closure_on(
        tx,
        analysis_attempt_id,
        inputs,
        &checklist_ids_by_input,
        CandidateAnalysisExactClosureRow {
            all_input_count: i64::try_from(inputs.len()).unwrap_or(i64::MAX),
            complete_input_count: i64::try_from(inputs.len()).unwrap_or(i64::MAX),
            all_input_set_hash,
            complete_input_set_hash,
            proposal_census_hash,
            h1_disposition_set_hash,
            coverage_checklist_set_hash,
            coverage_partition_set_hash: String::new(),
            coverage_subreview_census_set_hash: String::new(),
            page_receipt_set_hash: String::new(),
            critic_census_hash: None,
            gate_eligible: true,
        },
    )
    .await
}

async fn validate_candidate_analysis_partition_and_subreview_closure_on(
    tx: &mut Transaction<'_, Postgres>,
    analysis_attempt_id: Uuid,
    inputs: &[ExactClosureInputDbRow],
    checklist_by_input: &BTreeMap<Uuid, Vec<(Uuid, i32, String)>>,
    mut closure: CandidateAnalysisExactClosureRow,
) -> Result<CandidateAnalysisExactClosureRow> {
    let persisted_partitions = sqlx::query_as::<_, ExactClosurePartitionDbRow>(
        r#"SELECT chunk_partition_id,snapshot_input_id,partition_ordinal,
                  first_chunk_ordinal,last_chunk_ordinal,chunk_count,chunk_set_hash,
                  bounded_context_budget,partition_hash
             FROM candidate_analysis_hypothesis_coverage_chunk_partitions
            WHERE analysis_attempt_id=$1
            ORDER BY snapshot_input_id,partition_ordinal"#,
    )
    .bind(analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut partitions_by_input = BTreeMap::<Uuid, Vec<&ExactClosurePartitionDbRow>>::new();
    for partition in &persisted_partitions {
        partitions_by_input
            .entry(partition.snapshot_input_id)
            .or_default()
            .push(partition);
    }
    if partitions_by_input.keys().any(|input_id| {
        !inputs
            .iter()
            .any(|input| input.snapshot_input_id == *input_id)
    }) {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    for input in inputs {
        let chunk_census: (Uuid, String, i64) = sqlx::query_as(
            r#"SELECT chunk_census_id,disposition,chunk_count
                 FROM candidate_analysis_input_chunk_censuses
                WHERE snapshot_input_id=$1"#,
        )
        .bind(input.snapshot_input_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        if chunk_census.1 != "complete" || chunk_census.2 <= 0 {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let chunks: Vec<(i32, String)> = sqlx::query_as(
            r#"SELECT ordinal,chunk_hash
                 FROM candidate_analysis_input_chunk_census_members
                WHERE chunk_census_id=$1
                ORDER BY ordinal,chunk_id"#,
        )
        .bind(chunk_census.0)
        .fetch_all(&mut **tx)
        .await?;
        if chunks.len() as i64 != chunk_census.2
            || chunks
                .iter()
                .enumerate()
                .any(|(ordinal, chunk)| chunk.0 != i32::try_from(ordinal).unwrap_or(i32::MAX))
        {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let partitions = partitions_by_input
            .get(&input.snapshot_input_id)
            .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        let mut next_chunk_ordinal = 0_i32;
        for (partition_ordinal, partition) in partitions.iter().enumerate() {
            if partition.partition_ordinal != i32::try_from(partition_ordinal).unwrap_or(i32::MAX)
                || partition.first_chunk_ordinal != next_chunk_ordinal
                || partition.last_chunk_ordinal < partition.first_chunk_ordinal
                || partition.bounded_context_budget != 262_144
            {
                return Err(conflict(CENSUS_NOT_CLOSED));
            }
            let first = usize::try_from(partition.first_chunk_ordinal)
                .map_err(|_| conflict(CENSUS_NOT_CLOSED))?;
            let last = usize::try_from(partition.last_chunk_ordinal)
                .map_err(|_| conflict(CENSUS_NOT_CLOSED))?;
            let designated_chunks = chunks
                .get(first..=last)
                .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
            let designated_hashes = designated_chunks
                .iter()
                .map(|chunk| chunk.1.clone())
                .collect::<Vec<_>>();
            let expected_chunk_set_hash = hash_text_array_on(tx, &designated_hashes).await?;
            let expected_partition_hash = hash_json_on(
                tx,
                &json!({
                    "analysis_attempt_id":analysis_attempt_id,
                    "snapshot_input_id":input.snapshot_input_id,
                    "chunk_set_hash":expected_chunk_set_hash,
                    "first":partition.first_chunk_ordinal,
                    "last":partition.last_chunk_ordinal,
                }),
            )
            .await?;
            if partition.chunk_count != i64::try_from(designated_chunks.len()).unwrap_or(i64::MAX)
                || partition.chunk_set_hash != expected_chunk_set_hash
                || partition.partition_hash != expected_partition_hash
            {
                return Err(conflict(CENSUS_NOT_CLOSED));
            }
            next_chunk_ordinal = partition
                .last_chunk_ordinal
                .checked_add(1)
                .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        }
        if next_chunk_ordinal != i32::try_from(chunks.len()).unwrap_or(i32::MAX) {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
    }
    closure.coverage_partition_set_hash = hash_text_array_on(
        tx,
        &persisted_partitions
            .iter()
            .map(|partition| partition.partition_hash.clone())
            .collect::<Vec<_>>(),
    )
    .await?;

    let headers = sqlx::query_as::<_, ExactClosureSubreviewHeaderDbRow>(
        r#"SELECT subreview_census_id,snapshot_input_id,checklist_member_count,
                  checklist_member_set_hash,chunk_partition_count,
                  chunk_partition_set_hash,expected_member_count,member_set_hash,
                  census_hash
             FROM candidate_analysis_hypothesis_coverage_subreview_censuses
            WHERE analysis_attempt_id=$1
            ORDER BY snapshot_input_id"#,
    )
    .bind(analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    if headers.len() != inputs.len() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let headers_by_input = headers
        .iter()
        .map(|header| (header.snapshot_input_id, header))
        .collect::<BTreeMap<_, _>>();
    let all_members = sqlx::query_as::<_, ExactClosureSubreviewMemberDbRow>(
        r#"SELECT subreview_census_member_id,subreview_census_id,snapshot_input_id,
                  checklist_member_id,chunk_partition_id,checklist_ordinal,
                  partition_ordinal,designated_stage_work_item_id,disposition,member_hash
             FROM candidate_analysis_hypothesis_coverage_subreview_census_members
            WHERE analysis_attempt_id=$1
            ORDER BY snapshot_input_id,checklist_ordinal,partition_ordinal"#,
    )
    .bind(analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut members_by_input = BTreeMap::<Uuid, Vec<&ExactClosureSubreviewMemberDbRow>>::new();
    for member in &all_members {
        members_by_input
            .entry(member.snapshot_input_id)
            .or_default()
            .push(member);
    }
    for input in inputs {
        let header = headers_by_input
            .get(&input.snapshot_input_id)
            .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        let checklist = checklist_by_input
            .get(&input.snapshot_input_id)
            .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        let partitions = partitions_by_input
            .get(&input.snapshot_input_id)
            .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        let checklist_hashes = checklist
            .iter()
            .map(|member| member.2.clone())
            .collect::<Vec<_>>();
        let input_partition_hashes = partitions
            .iter()
            .map(|partition| partition.partition_hash.clone())
            .collect::<Vec<_>>();
        let checklist_set_hash = hash_text_array_on(tx, &checklist_hashes).await?;
        let partition_set_hash = hash_text_array_on(tx, &input_partition_hashes).await?;
        let mut expected_members =
            Vec::with_capacity(checklist.len().saturating_mul(partitions.len()));
        for (checklist_id, checklist_ordinal, checklist_hash) in checklist {
            for partition in partitions {
                let designated: Vec<(Uuid, String)> = sqlx::query_as(
                    r#"SELECT stage_work_item_id,capability
                         FROM candidate_analysis_work_items
                        WHERE analysis_attempt_id=$1 AND phase='critic'
                          AND capability IN (
                              'hypothesis_coverage_subreview',
                              'hypothesis_coverage_sampling_omitted'
                          )
                          AND component_id=$2 AND microbatch_key=$3
                        ORDER BY stage_work_item_id"#,
                )
                .bind(analysis_attempt_id)
                .bind(checklist_id)
                .bind(partition.chunk_partition_id.to_string())
                .fetch_all(&mut **tx)
                .await?;
                if designated.len() != 1 {
                    return Err(conflict(CENSUS_NOT_CLOSED));
                }
                let disposition = if designated[0].1 == "hypothesis_coverage_sampling_omitted" {
                    "sampling_omitted"
                } else {
                    "required"
                };
                let member_hash = hash_json_on(
                    tx,
                    &json!({
                        "domain":"candidate_hypothesis_coverage_subreview_census_member.v1",
                        "analysis_attempt_id":analysis_attempt_id,
                        "snapshot_input_id":input.snapshot_input_id,
                        "checklist_member_id":checklist_id,
                        "checklist_ordinal":checklist_ordinal,
                        "checklist_member_hash":checklist_hash,
                        "chunk_partition_id":partition.chunk_partition_id,
                        "partition_ordinal":partition.partition_ordinal,
                        "chunk_partition_hash":partition.partition_hash,
                        "designated_stage_work_item_id":designated[0].0,
                        "disposition":disposition,
                    }),
                )
                .await?;
                expected_members.push((
                    *checklist_id,
                    partition.chunk_partition_id,
                    *checklist_ordinal,
                    partition.partition_ordinal,
                    designated[0].0,
                    disposition,
                    member_hash,
                ));
            }
        }
        let persisted_members = members_by_input
            .get(&input.snapshot_input_id)
            .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        if persisted_members.len() != expected_members.len() {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        for (persisted, expected) in persisted_members.iter().zip(expected_members.iter()) {
            if persisted.subreview_census_id != header.subreview_census_id
                || persisted.subreview_census_member_id
                    != Uuid::new_v5(&header.subreview_census_id, expected.6.as_bytes())
                || persisted.checklist_member_id != expected.0
                || persisted.chunk_partition_id != expected.1
                || persisted.checklist_ordinal != expected.2
                || persisted.partition_ordinal != expected.3
                || persisted.designated_stage_work_item_id != expected.4
                || persisted.disposition != expected.5
                || persisted.member_hash != expected.6
            {
                return Err(conflict(CENSUS_NOT_CLOSED));
            }
        }
        let member_hashes = expected_members
            .iter()
            .map(|member| member.6.clone())
            .collect::<Vec<_>>();
        let member_set_hash = hash_text_array_on(tx, &member_hashes).await?;
        let expected_member_count = i64::try_from(expected_members.len()).unwrap_or(i64::MAX);
        let census_hash = hash_json_on(
            tx,
            &json!({
                "domain":"candidate_hypothesis_coverage_subreview_census.v1",
                "analysis_attempt_id":analysis_attempt_id,
                "snapshot_input_id":input.snapshot_input_id,
                "checklist_member_count":checklist.len(),
                "checklist_member_set_hash":checklist_set_hash,
                "chunk_partition_count":partitions.len(),
                "chunk_partition_set_hash":partition_set_hash,
                "expected_member_count":expected_member_count,
                "member_set_hash":member_set_hash,
            }),
        )
        .await?;
        if header.checklist_member_count != i64::try_from(checklist.len()).unwrap_or(i64::MAX)
            || header.checklist_member_set_hash != checklist_set_hash
            || header.chunk_partition_count != i64::try_from(partitions.len()).unwrap_or(i64::MAX)
            || header.chunk_partition_set_hash != partition_set_hash
            || header.expected_member_count != expected_member_count
            || header.member_set_hash != member_set_hash
            || header.census_hash != census_hash
        {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
    }
    closure.coverage_subreview_census_set_hash = hash_text_array_on(
        tx,
        &headers
            .iter()
            .map(|header| header.census_hash.clone())
            .collect::<Vec<_>>(),
    )
    .await?;
    closure.page_receipt_set_hash =
        validate_candidate_analysis_page_closure_on(tx, analysis_attempt_id, inputs, &all_members)
            .await?;
    closure.critic_census_hash = Some(
        validate_candidate_analysis_critic_closure_on(tx, analysis_attempt_id, inputs.len())
            .await?,
    );
    Ok(closure)
}

async fn validate_candidate_analysis_page_closure_on(
    tx: &mut Transaction<'_, Postgres>,
    analysis_attempt_id: Uuid,
    inputs: &[ExactClosureInputDbRow],
    subreview_members: &[ExactClosureSubreviewMemberDbRow],
) -> Result<String> {
    let snapshot_id: Uuid = sqlx::query_scalar(
        "SELECT snapshot_id FROM candidate_analysis_attempts WHERE analysis_attempt_id=$1",
    )
    .bind(analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    let receipts = sqlx::query_as::<_, ExactClosurePageReceiptDbRow>(
        r#"SELECT page_receipt_id,stable_request_id,snapshot_id,snapshot_input_id,
                  chunk_census_id,chunk_census_hash,source_size_bytes,
                  chunking_contract_version,redaction_contract_version,
                  consumer_worker_run_id,server_cursor,first_key,last_key,
                  returned_count,page_hash
             FROM candidate_analysis_page_receipts
            WHERE analysis_attempt_id=$1 AND page_kind='chunk_page'
            ORDER BY page_receipt_id"#,
    )
    .bind(analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut consumed_receipts = BTreeSet::new();
    for input in inputs {
        let analyst_workers: Vec<Uuid> = sqlx::query_scalar(
            r#"SELECT worker.id
                 FROM candidate_analysis_work_items candidate_item
                 JOIN stage_worker_runs worker
                   ON worker.work_item_id=candidate_item.stage_work_item_id
                WHERE candidate_item.analysis_attempt_id=$1
                  AND candidate_item.phase='proposal'
                  AND candidate_item.capability='hypothesis_proposal'
                  AND candidate_item.microbatch_key=$2
                  AND worker.status='passed'
                ORDER BY worker.id"#,
        )
        .bind(analysis_attempt_id)
        .bind(input.snapshot_input_id.to_string())
        .fetch_all(&mut **tx)
        .await?;
        if analyst_workers.len() != 1 {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let chunk_count: i64 = sqlx::query_scalar(
            "SELECT chunk_count FROM candidate_analysis_input_chunk_censuses WHERE snapshot_input_id=$1 AND disposition='complete'",
        )
        .bind(input.snapshot_input_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        if chunk_count <= 0 {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        validate_candidate_chunk_page_range_on(
            tx,
            &receipts,
            analysis_attempt_id,
            snapshot_id,
            analyst_workers[0],
            input.snapshot_input_id,
            0,
            i32::try_from(chunk_count - 1).map_err(|_| conflict(CENSUS_NOT_CLOSED))?,
            &mut consumed_receipts,
        )
        .await?;
    }
    for member in subreview_members {
        let worker_ids: Vec<Uuid> = sqlx::query_scalar(
            r#"SELECT worker.id FROM stage_worker_runs worker
                WHERE worker.work_item_id=$1 AND worker.status='passed'
                ORDER BY worker.id"#,
        )
        .bind(member.designated_stage_work_item_id)
        .fetch_all(&mut **tx)
        .await?;
        if worker_ids.len() != 1 {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        if member.disposition == "sampling_omitted" {
            if receipts.iter().any(|receipt| {
                receipt.consumer_worker_run_id == worker_ids[0]
                    && receipt.snapshot_input_id == member.snapshot_input_id
            }) {
                return Err(conflict(CENSUS_NOT_CLOSED));
            }
            continue;
        }
        if member.disposition != "required" {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let partition: (i32, i32) = sqlx::query_as(
            r#"SELECT first_chunk_ordinal,last_chunk_ordinal
                 FROM candidate_analysis_hypothesis_coverage_chunk_partitions
                WHERE chunk_partition_id=$1 AND analysis_attempt_id=$2
                  AND snapshot_input_id=$3"#,
        )
        .bind(member.chunk_partition_id)
        .bind(analysis_attempt_id)
        .bind(member.snapshot_input_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        validate_candidate_chunk_page_range_on(
            tx,
            &receipts,
            analysis_attempt_id,
            snapshot_id,
            worker_ids[0],
            member.snapshot_input_id,
            partition.0,
            partition.1,
            &mut consumed_receipts,
        )
        .await?;
    }
    if consumed_receipts.len() != receipts.len() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    hash_text_array_on(
        tx,
        &receipts
            .iter()
            .map(|receipt| receipt.page_hash.clone())
            .collect::<Vec<_>>(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn validate_candidate_chunk_page_range_on(
    tx: &mut Transaction<'_, Postgres>,
    all_receipts: &[ExactClosurePageReceiptDbRow],
    analysis_attempt_id: Uuid,
    snapshot_id: Uuid,
    worker_run_id: Uuid,
    snapshot_input_id: Uuid,
    expected_first: i32,
    expected_last: i32,
    consumed_receipts: &mut BTreeSet<Uuid>,
) -> Result<()> {
    let census: (Uuid, String, i64, String, String) = sqlx::query_as(
        r#"SELECT chunk_census_id,census_hash,source_byte_count,
                  chunking_contract_version,redaction_contract_version
             FROM candidate_analysis_input_chunk_censuses
            WHERE snapshot_input_id=$1 AND disposition='complete'"#,
    )
    .bind(snapshot_input_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    let mut receipts = all_receipts
        .iter()
        .filter(|receipt| {
            receipt.consumer_worker_run_id == worker_run_id
                && receipt.snapshot_input_id == snapshot_input_id
        })
        .collect::<Vec<_>>();
    receipts.sort_by_key(|receipt| {
        receipt
            .first_key
            .as_deref()
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(i32::MAX)
    });
    if receipts.is_empty() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let mut next = expected_first;
    for receipt in receipts {
        let first = receipt
            .first_key
            .as_deref()
            .and_then(|value| value.parse::<i32>().ok())
            .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        let last = receipt
            .last_key
            .as_deref()
            .and_then(|value| value.parse::<i32>().ok())
            .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
        let mut cursor_parts = receipt.server_cursor.split(':');
        let cursor_valid = cursor_parts.next() == Some("chunk")
            && cursor_parts
                .next()
                .and_then(|value| value.parse::<i32>().ok())
                == Some(first);
        let cursor_limit = cursor_parts
            .next()
            .and_then(|value| value.parse::<i64>().ok());
        if !cursor_valid
            || cursor_parts.next().is_some()
            || cursor_limit
                .is_none_or(|limit| !(1..=64).contains(&limit) || receipt.returned_count > limit)
            || first != next
            || last < first
            || last > expected_last
            || receipt.returned_count != i64::from(last - first + 1)
            || receipt.chunk_census_id != census.0
            || receipt.snapshot_id != snapshot_id
            || receipt.chunk_census_hash != census.1
            || receipt.source_size_bytes != census.2
            || receipt.chunking_contract_version != census.3
            || receipt.redaction_contract_version != census.4
            || receipt.page_receipt_id
                != Uuid::new_v5(&receipt.stable_request_id, b"candidate_page_receipt.v1")
            || !consumed_receipts.insert(receipt.page_receipt_id)
        {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let chunk_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT chunk_hash
                 FROM candidate_analysis_input_chunk_census_members
                WHERE chunk_census_id=$1 AND ordinal BETWEEN $2 AND $3
                ORDER BY ordinal"#,
        )
        .bind(census.0)
        .bind(first)
        .bind(last)
        .fetch_all(&mut **tx)
        .await?;
        if chunk_hashes.len() as i64 != receipt.returned_count {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let expected_page_hash = candidate_chunk_page_hash_on(
            tx,
            &CandidateChunkPageHashInput {
                analysis_attempt_id,
                snapshot_id,
                snapshot_input_id,
                chunk_census_id: census.0,
                chunk_census_hash: census.1.clone(),
                consumer_worker_run_id: worker_run_id,
                first_ordinal: Some(first),
                last_ordinal: Some(last),
                ordered_chunk_hashes: chunk_hashes,
                source_size_bytes: census.2,
                chunking_contract_version: census.3.clone(),
                redaction_contract_version: census.4.clone(),
            },
        )
        .await?;
        if receipt.page_hash != expected_page_hash {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        next = last
            .checked_add(1)
            .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    }
    if next != expected_last.saturating_add(1) {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    Ok(())
}

async fn validate_candidate_analysis_critic_closure_on(
    tx: &mut Transaction<'_, Postgres>,
    analysis_attempt_id: Uuid,
    input_count: usize,
) -> Result<String> {
    validate_candidate_analysis_typed_missed_refs_on(tx, analysis_attempt_id).await?;
    let leaf_drift: i64 = sqlx::query_scalar(
        r#"SELECT
            (SELECT count(*)
               FROM candidate_analysis_hypothesis_coverage_subreview_census_members member
               LEFT JOIN candidate_analysis_hypothesis_coverage_subreviews review
                 ON review.subreview_census_member_id=member.subreview_census_member_id
              WHERE member.analysis_attempt_id=$1
                AND ((member.disposition='required' AND review.subreview_id IS NULL)
                  OR (member.disposition='sampling_omitted' AND review.subreview_id IS NOT NULL)))
          + (SELECT count(*)
               FROM candidate_analysis_hypothesis_coverage_subreviews review
               LEFT JOIN candidate_analysis_hypothesis_coverage_subreview_census_members member
                 ON member.subreview_census_member_id=review.subreview_census_member_id
                AND member.analysis_attempt_id=$1
              WHERE review.analysis_attempt_id=$1
                AND (member.subreview_census_member_id IS NULL OR member.disposition<>'required'))"#,
    )
    .bind(analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    let (synthesis_node_count, synthesis_review_count): (i64, i64) = sqlx::query_as(
        r#"SELECT
            (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_synthesis_census_members WHERE analysis_attempt_id=$1),
            (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_synthesis_reviews WHERE analysis_attempt_id=$1)"#,
    )
    .bind(analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    let synthesis_review_drift: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM (
              (SELECT synthesis_node_id
                 FROM candidate_analysis_hypothesis_coverage_synthesis_census_members
                WHERE analysis_attempt_id=$1
               EXCEPT
               SELECT synthesis_node_id
                 FROM candidate_analysis_hypothesis_coverage_synthesis_reviews
                WHERE analysis_attempt_id=$1)
              UNION ALL
              (SELECT synthesis_node_id
                 FROM candidate_analysis_hypothesis_coverage_synthesis_reviews
                WHERE analysis_attempt_id=$1
               EXCEPT
               SELECT synthesis_node_id
                 FROM candidate_analysis_hypothesis_coverage_synthesis_census_members
                WHERE analysis_attempt_id=$1)
            ) drift"#,
    )
    .bind(analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    let coverage_review_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM candidate_analysis_hypothesis_coverage_reviews WHERE analysis_attempt_id=$1",
    )
    .bind(analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    let global_review_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM candidate_analysis_hypothesis_coverage_global_reviews WHERE analysis_attempt_id=$1",
    )
    .bind(analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    if leaf_drift != 0
        || synthesis_node_count == 0
        || synthesis_node_count != synthesis_review_count
        || synthesis_review_drift != 0
        || coverage_review_count != i64::try_from(input_count).unwrap_or(i64::MAX)
        || global_review_count != 1
    {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }

    let unresolved_conflicts: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM hypothesis_merge_decisions
            WHERE analysis_attempt_id=$1 AND decision_kind<>'keep_distinct'"#,
    )
    .bind(analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    if unresolved_conflicts != 0 {
        return Err(conflict(CONFLICT_DECISION_UNRESOLVED));
    }
    let retry_input_still_exact: bool = sqlx::query_scalar(
        r#"WITH current_attempt AS (
               SELECT predecessor_attempt_id,attempt_input_hash
                 FROM candidate_analysis_attempts WHERE analysis_attempt_id=$1
           ), predecessor AS (
               SELECT attempt.analysis_attempt_id,attempt.attempt_input_hash,
                      terminal.attempt_event_id,terminal.event_hash
                 FROM current_attempt current
                 JOIN candidate_analysis_attempts attempt
                   ON attempt.analysis_attempt_id=current.predecessor_attempt_id
                 JOIN candidate_analysis_attempt_state_events terminal
                   ON terminal.analysis_attempt_id=attempt.analysis_attempt_id
                  AND terminal.event_kind='superseded_missed_hypothesis'
           ), missed(checklist_member_id) AS (
               SELECT jsonb_array_elements_text(subreview.typed_missed_refs)::UUID
                 FROM current_attempt current
                 JOIN candidate_analysis_hypothesis_coverage_subreviews subreview
                   ON subreview.analysis_attempt_id=current.predecessor_attempt_id
                WHERE subreview.outcome='missed_hypothesis'
               UNION
               SELECT jsonb_array_elements_text(review.typed_missed_refs)::UUID
                 FROM current_attempt current
                 JOIN candidate_analysis_hypothesis_coverage_synthesis_reviews review
                   ON review.analysis_attempt_id=current.predecessor_attempt_id
                WHERE review.outcome='missed_hypothesis'
           ), signals AS (
               SELECT checklist.checklist_member_id,
                      tool_truth_sha256(jsonb_build_object(
                          'checklist_member_id',checklist.checklist_member_id,
                          'attack_class_id',checklist.attack_class_id,
                          'attack_class_version',checklist.attack_class_version,
                          'trust_boundary_identity',checklist.trust_boundary_identity,
                          'trust_boundary_hash',checklist.trust_boundary_hash,
                          'covered_input_ids',jsonb_build_array(checklist.snapshot_input_id)
                      )::TEXT) AS signal_hash
                 FROM missed
                 JOIN candidate_analysis_hypothesis_coverage_checklist_members checklist
                   USING(checklist_member_id)
                WHERE checklist.analysis_attempt_id=(
                    SELECT predecessor_attempt_id FROM current_attempt)
           )
           SELECT CASE WHEN current.predecessor_attempt_id IS NULL THEN TRUE
              ELSE current.attempt_input_hash=tool_truth_sha256(jsonb_build_object(
                  'schema','candidate_retry_attempt_input.v1',
                  'predecessor_attempt_id',predecessor.analysis_attempt_id,
                  'predecessor_attempt_input_hash',predecessor.attempt_input_hash,
                  'predecessor_terminal_event_id',predecessor.attempt_event_id,
                  'predecessor_terminal_event_hash',predecessor.event_hash,
                  'missed_hypothesis_signal_count',(SELECT COUNT(*) FROM signals),
                  'missed_hypothesis_signal_set_hash',tool_truth_sha256(to_jsonb(ARRAY(
                      SELECT signal_hash FROM signals ORDER BY checklist_member_id
                  ))::TEXT)
              )::TEXT) END
             FROM current_attempt current LEFT JOIN predecessor ON TRUE"#,
    )
    .bind(analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    if !retry_input_still_exact {
        return Err(conflict("CANDIDATE_RETRY_ATTEMPT_INPUT_HASH_INVALID"));
    }

    let sources = sqlx::query_as::<_, CriticCensusSourceDbRow>(
        r#"SELECT member_kind,source_identity,source_hash FROM(
            SELECT 'proposal_conflict_review'::TEXT AS member_kind,
                   component.conflict_component_id AS source_identity,
                   decision.decision_hash AS source_hash
              FROM candidate_analysis_conflict_components component
              JOIN hypothesis_merge_decisions decision USING(conflict_component_id,analysis_attempt_id)
             WHERE component.analysis_attempt_id=$1
            UNION ALL SELECT 'hypothesis_coverage_subreview',subreview_id,subreview_hash
              FROM candidate_analysis_hypothesis_coverage_subreviews WHERE analysis_attempt_id=$1
            UNION ALL SELECT 'hypothesis_coverage_synthesis',synthesis_review_id,review_hash
              FROM candidate_analysis_hypothesis_coverage_synthesis_reviews WHERE analysis_attempt_id=$1
            UNION ALL SELECT 'hypothesis_coverage_input_review',coverage_review_id,review_hash
              FROM candidate_analysis_hypothesis_coverage_reviews WHERE analysis_attempt_id=$1
            UNION ALL SELECT 'hypothesis_coverage_global_review',global_review_id,review_hash
              FROM candidate_analysis_hypothesis_coverage_global_reviews WHERE analysis_attempt_id=$1
        ) source ORDER BY member_kind,source_identity"#,
    )
    .bind(analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    if sources.is_empty() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let mut expected_member_hashes = Vec::with_capacity(sources.len());
    for source in &sources {
        expected_member_hashes.push(
            hash_json_on(
                tx,
                &json!({
                    "member_kind":source.member_kind,
                    "source_identity":source.source_identity,
                    "source_hash":source.source_hash,
                }),
            )
            .await?,
        );
    }
    let member_set_hash = hash_text_array_on(tx, &expected_member_hashes).await?;
    let census_hash = hash_json_on(
        tx,
        &json!({
            "kind":"critic",
            "attempt":analysis_attempt_id,
            "count":sources.len(),
            "set":member_set_hash,
        }),
    )
    .await?;
    let header: (Uuid, i64, String, String) = sqlx::query_as(
        r#"SELECT critic_census_id,member_count,member_set_hash,census_hash
             FROM candidate_analysis_critic_censuses
            WHERE analysis_attempt_id=$1"#,
    )
    .bind(analysis_attempt_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    if header.1 != i64::try_from(sources.len()).unwrap_or(i64::MAX)
        || header.2 != member_set_hash
        || header.3 != census_hash
    {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let members: Vec<(Uuid, i32, String, Uuid, String, String)> = sqlx::query_as(
        r#"SELECT critic_member_id,ordinal,member_kind,source_identity,source_hash,member_hash
             FROM candidate_analysis_critic_census_members
            WHERE critic_census_id=$1 AND analysis_attempt_id=$2
            ORDER BY ordinal,member_kind,source_identity"#,
    )
    .bind(header.0)
    .bind(analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    if members.len() != sources.len() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    for (ordinal, ((member, source), expected_hash)) in members
        .iter()
        .zip(sources.iter())
        .zip(expected_member_hashes.iter())
        .enumerate()
    {
        if member.0 != Uuid::new_v5(&header.0, expected_hash.as_bytes())
            || member.1 != i32::try_from(ordinal).unwrap_or(i32::MAX)
            || member.2 != source.member_kind
            || member.3 != source.source_identity
            || member.4 != source.source_hash
            || member.5.as_str() != expected_hash
        {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
    }
    Ok(census_hash)
}

fn exact_uuid_set(value: &Value) -> Result<BTreeSet<Uuid>> {
    let values = value
        .as_array()
        .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    let parsed = values
        .iter()
        .map(|value| {
            value
                .as_str()
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if parsed.len() != values.len() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    Ok(parsed)
}

async fn validate_candidate_analysis_typed_missed_refs_on(
    tx: &mut Transaction<'_, Postgres>,
    analysis_attempt_id: Uuid,
) -> Result<()> {
    let subreviews: Vec<TypedMissedSubreviewRow> = sqlx::query_as(
        r#"SELECT review.subreview_id,review.snapshot_input_id,member.checklist_member_id,
                  review.outcome,review.typed_missed_refs,review.blocker_codes,
                  review.semantic_summary,review.semantic_observation_count,
                  review.semantic_summary_hash
             FROM candidate_analysis_hypothesis_coverage_subreviews review
             JOIN candidate_analysis_hypothesis_coverage_subreview_census_members member
               ON member.subreview_census_member_id=review.subreview_census_member_id
            WHERE review.analysis_attempt_id=$1
            ORDER BY review.subreview_id"#,
    )
    .bind(analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    for (
        _,
        snapshot_input_id,
        checklist_member_id,
        outcome,
        typed_refs,
        blocker_codes,
        semantic_summary,
        semantic_observation_count,
        semantic_summary_hash,
    ) in subreviews
    {
        let refs = exact_uuid_set(&typed_refs)?;
        let expected_blockers = blocker_codes.into_iter().collect::<BTreeSet<_>>();
        let valid = match outcome.as_str() {
            "missed_hypothesis" => {
                refs == BTreeSet::from([checklist_member_id]) && expected_blockers.is_empty()
            }
            "no_local_miss" => refs.is_empty() && expected_blockers.is_empty(),
            "blocked" => refs.is_empty() && !expected_blockers.is_empty(),
            _ => false,
        };
        if !valid {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let expected_observed: BTreeSet<Uuid> = sqlx::query_scalar(
            r#"SELECT DISTINCT proposal.proposal_id
                  FROM hypothesis_proposals proposal
                  JOIN candidate_analysis_artifacts artifact
                    ON artifact.artifact_id=proposal.artifact_id
                   AND artifact.analysis_attempt_id=proposal.analysis_attempt_id
                  JOIN candidate_analysis_work_items candidate
                    ON candidate.candidate_work_item_id=artifact.candidate_work_item_id
                   AND candidate.analysis_attempt_id=proposal.analysis_attempt_id
                  LEFT JOIN hypothesis_proposal_refs reference
                    ON reference.proposal_id=proposal.proposal_id
                   AND reference.analysis_attempt_id=proposal.analysis_attempt_id
                 WHERE proposal.analysis_attempt_id=$1
                   AND (candidate.microbatch_key::UUID=$2
                        OR reference.snapshot_input_id=$2)
                 ORDER BY proposal.proposal_id"#,
        )
        .bind(analysis_attempt_id)
        .bind(snapshot_input_id)
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .collect();
        let (count, hash) = validate_coverage_semantic_summary_on(
            tx,
            analysis_attempt_id,
            &semantic_summary,
            &BTreeSet::from([snapshot_input_id]),
            &BTreeSet::from([checklist_member_id]),
            &expected_observed,
            &refs,
            &expected_blockers,
            None,
        )
        .await
        .map_err(|_| conflict(CENSUS_NOT_CLOSED))?;
        if count != semantic_observation_count || hash != semantic_summary_hash {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
    }
    let synthesis_reviews: Vec<TypedMissedSynthesisRow> = sqlx::query_as(
        r#"SELECT review.synthesis_node_id,review.outcome,review.typed_missed_refs,
                  review.blocker_codes,review.semantic_summary,
                  review.semantic_observation_count,review.semantic_summary_hash,
                  node.covered_input_count,node.covered_input_set_hash,
                  node.covered_checklist_count,node.covered_checklist_set_hash,
                  node.child_receipt_count
             FROM candidate_analysis_hypothesis_coverage_synthesis_reviews review
             JOIN candidate_analysis_hypothesis_coverage_synthesis_census_members node
               ON node.synthesis_node_id=review.synthesis_node_id
            WHERE review.analysis_attempt_id=$1
            ORDER BY review.synthesis_node_id"#,
    )
    .bind(analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    for (
        synthesis_node_id,
        outcome,
        typed_refs,
        blocker_codes,
        semantic_summary,
        semantic_observation_count,
        semantic_summary_hash,
        covered_input_count,
        covered_input_set_hash,
        covered_checklist_count,
        covered_checklist_set_hash,
        child_receipt_count,
    ) in synthesis_reviews
    {
        let refs = exact_uuid_set(&typed_refs)?;
        if outcome != "missed_hypothesis" {
            if !matches!(outcome.as_str(), "no_composite_miss" | "blocked") || !refs.is_empty() {
                return Err(conflict(CENSUS_NOT_CLOSED));
            }
            if outcome == "blocked" && blocker_codes.is_empty() {
                return Err(conflict(CENSUS_NOT_CLOSED));
            }
        } else if refs.is_empty() {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let child_summaries = sqlx::query_as::<_, SynthesisChildSemanticDbRow>(
            r#"SELECT COALESCE(subreview.semantic_summary,synthesis.semantic_summary) AS semantic_summary,
                      COALESCE(subreview.semantic_summary_hash,synthesis.semantic_summary_hash) AS semantic_summary_hash
                  FROM candidate_analysis_hypothesis_coverage_synthesis_node_children child
                  LEFT JOIN candidate_analysis_hypothesis_coverage_subreviews subreview
                    ON subreview.subreview_id=child.child_subreview_id
                  LEFT JOIN candidate_analysis_hypothesis_coverage_synthesis_reviews synthesis
                    ON synthesis.synthesis_node_id=child.child_synthesis_node_id
                 WHERE child.synthesis_node_id=$1
                 ORDER BY child.ordinal"#,
        )
        .bind(synthesis_node_id)
        .fetch_all(&mut **tx)
        .await?;
        if child_summaries.is_empty()
            || child_summaries.len() > 32
            || child_summaries.len() as i64 != child_receipt_count
        {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let mut expected_inputs = BTreeSet::new();
        let mut expected_checklists = BTreeSet::new();
        let mut expected_observed = BTreeSet::new();
        let mut expected_missed = BTreeSet::new();
        let mut expected_blockers = BTreeSet::new();
        let mut expected_observations = BTreeSet::new();
        for child in child_summaries {
            if hash_json_on(tx, &child.semantic_summary).await? != child.semantic_summary_hash {
                return Err(conflict(CENSUS_NOT_CLOSED));
            }
            let child: CoverageSemanticSummaryV1 = serde_json::from_value(child.semantic_summary)
                .map_err(|_| conflict(CENSUS_NOT_CLOSED))?;
            expected_inputs.extend(child.covered_input_ids);
            expected_checklists.extend(child.covered_checklist_member_ids);
            expected_observed.extend(child.observed_proposal_ids);
            expected_missed.extend(child.missed_checklist_member_ids);
            expected_blockers.extend(child.blocker_codes);
            for observation in child.semantic_observations {
                expected_observations.insert(
                    serde_json::to_string(&observation).map_err(|_| conflict(CENSUS_NOT_CLOSED))?,
                );
            }
        }
        let input_text = expected_inputs
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>();
        let checklist_text = expected_checklists
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>();
        let stored_blockers = blocker_codes.into_iter().collect::<BTreeSet<_>>();
        let valid_outcome = match outcome.as_str() {
            "no_composite_miss" => expected_missed.is_empty() && expected_blockers.is_empty(),
            "missed_hypothesis" => !expected_missed.is_empty() && expected_blockers.is_empty(),
            "blocked" => expected_missed.is_empty() && !expected_blockers.is_empty(),
            _ => false,
        };
        if !valid_outcome
            || refs != expected_missed
            || stored_blockers != expected_blockers
            || expected_inputs.len() as i64 != covered_input_count
            || hash_text_array_on(tx, &input_text).await? != covered_input_set_hash
            || expected_checklists.len() as i64 != covered_checklist_count
            || hash_text_array_on(tx, &checklist_text).await? != covered_checklist_set_hash
        {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
        let (count, hash) = validate_coverage_semantic_summary_on(
            tx,
            analysis_attempt_id,
            &semantic_summary,
            &expected_inputs,
            &expected_checklists,
            &expected_observed,
            &expected_missed,
            &expected_blockers,
            Some(&expected_observations),
        )
        .await
        .map_err(|_| conflict(CENSUS_NOT_CLOSED))?;
        if count != semantic_observation_count || hash != semantic_summary_hash {
            return Err(conflict(CENSUS_NOT_CLOSED));
        }
    }
    Ok(())
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
    pub controller_dispatch_worker_run_id: Uuid,
    pub snapshot_row_version: i64,
    pub attempt_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidatePreGateMaterialRow {
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
    pub exact_closure: CandidateAnalysisExactClosureRow,
    pub final_submitter_worker_run_id: Uuid,
    pub controller_dispatch_worker_run_id: Uuid,
    pub snapshot_row_version: i64,
    pub attempt_row_version: i64,
}

pub async fn load_candidate_pre_gate_material_on(
    tx: &mut Transaction<'_, Postgres>,
    input: LoadCandidateGateMaterialInput,
) -> Result<CandidatePreGateMaterialRow> {
    if input.expected_snapshot_row_version != 0 || input.expected_attempt_row_version != 0 {
        return Err(conflict(WRITE_FENCE_MISMATCH));
    }
    let snapshot = load_snapshot_on(tx, input.snapshot_id).await?;
    if snapshot.disposition != CandidateSnapshotDispositionRow::SealedReady
        || snapshot.operation_id != input.operation_id
        || snapshot.organization_id != input.organization_id
        || snapshot.scope_snapshot_id != input.scope_snapshot_id
    {
        return Err(conflict(SNAPSHOT_NOT_READY));
    }
    let unresolved_conflicts: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM hypothesis_merge_decisions
            WHERE analysis_attempt_id=$1 AND decision_kind<>'keep_distinct'"#,
    )
    .bind(input.analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    if unresolved_conflicts != 0 {
        return Err(conflict(CONFLICT_DECISION_UNRESOLVED));
    }
    let gate_reevaluation = reevaluate_candidate_gate_authority_on(tx, input.snapshot_id).await?;
    let exact_closure = validate_candidate_analysis_exact_closure_on(
        tx,
        input.analysis_attempt_id,
        input.snapshot_id,
    )
    .await?;
    if !exact_closure.gate_eligible {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let attempt_epoch: i64 = sqlx::query_scalar(r#"SELECT COALESCE(MAX(worker.attempt_epoch),attempt.attempt_ordinal::BIGINT)
        FROM candidate_analysis_attempts attempt
        LEFT JOIN candidate_analysis_work_items candidate_item ON candidate_item.analysis_attempt_id=attempt.analysis_attempt_id
        LEFT JOIN stage_worker_runs worker ON worker.work_item_id=candidate_item.stage_work_item_id
        WHERE attempt.analysis_attempt_id=$1 AND attempt.snapshot_id=$2 AND attempt.attempt_ordinal=$3
        GROUP BY attempt.attempt_ordinal"#).bind(input.analysis_attempt_id).bind(input.snapshot_id)
        .bind(input.analysis_attempt_ordinal).fetch_optional(&mut **tx).await?
        .ok_or_else(||conflict(AUTHORITY_MISMATCH))?;
    let proposal: String = sqlx::query_scalar(
        "SELECT census_hash FROM candidate_analysis_proposal_censuses WHERE analysis_attempt_id=$1",
    )
    .bind(input.analysis_attempt_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    let critic: String = sqlx::query_scalar(
        "SELECT census_hash FROM candidate_analysis_critic_censuses WHERE analysis_attempt_id=$1",
    )
    .bind(input.analysis_attempt_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    let controller_workers: Vec<(Uuid, Uuid)> = sqlx::query_as(
        r#"SELECT final_worker.id,dispatch_worker.id
              FROM candidate_analysis_work_items final_candidate
              JOIN stage_work_items final_item
                ON final_item.id=final_candidate.stage_work_item_id
               AND final_item.kind='candidate_controller_final'
               AND final_item.role='controller'
               AND final_item.status='completed'
              JOIN stage_worker_runs final_worker
                ON final_worker.work_item_id=final_item.id
               AND final_worker.specialist='controller'
               AND final_worker.work_item_kind='candidate_controller_final'
               AND final_worker.status='passed'
              JOIN stage_team_plans plan
                ON plan.id=final_item.team_plan_id
               AND plan.final_submitter_worker_run_id=final_worker.id
              JOIN candidate_analysis_provider_attempts final_receipt
                ON final_receipt.analysis_attempt_id=final_candidate.analysis_attempt_id
               AND final_receipt.stage_work_item_id=final_item.id
               AND final_receipt.worker_run_id=final_worker.id
               AND final_receipt.artifact_kind='controller_decision.v1'
              JOIN candidate_analysis_work_items dispatch_candidate
                ON dispatch_candidate.analysis_attempt_id=final_candidate.analysis_attempt_id
               AND dispatch_candidate.phase='controller'
               AND dispatch_candidate.capability='candidate_controller_dispatch'
              JOIN stage_work_items dispatch_item
                ON dispatch_item.id=dispatch_candidate.stage_work_item_id
               AND dispatch_item.team_plan_id=plan.id
               AND dispatch_item.kind='candidate_controller_dispatch'
               AND dispatch_item.role='controller'
               AND dispatch_item.status='completed'
              JOIN stage_worker_runs dispatch_worker
                ON dispatch_worker.work_item_id=dispatch_item.id
               AND dispatch_worker.specialist='controller'
               AND dispatch_worker.work_item_kind='candidate_controller_dispatch'
               AND dispatch_worker.status='passed'
              JOIN candidate_analysis_provider_attempts dispatch_receipt
                ON dispatch_receipt.analysis_attempt_id=dispatch_candidate.analysis_attempt_id
               AND dispatch_receipt.stage_work_item_id=dispatch_item.id
               AND dispatch_receipt.worker_run_id=dispatch_worker.id
               AND dispatch_receipt.artifact_kind='controller_dispatch.v1'
             WHERE final_candidate.analysis_attempt_id=$1
               AND final_candidate.phase='controller'
               AND final_candidate.capability='candidate_controller_final'
               AND final_item.operation_id=$2
               AND final_item.organization_id=$3
               AND final_item.scope_snapshot_id=$4
               AND final_worker.id<>dispatch_worker.id"#,
    )
    .bind(input.analysis_attempt_id)
    .bind(input.operation_id)
    .bind(input.organization_id)
    .bind(input.scope_snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    let [(worker, controller_dispatch_worker)] = controller_workers.as_slice() else {
        return Err(conflict(CENSUS_NOT_CLOSED));
    };
    let (worker, controller_dispatch_worker) = (*worker, *controller_dispatch_worker);
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
    .fetch_all(&mut **tx)
    .await?;
    let chain = hash_text_array_on(tx, &chain_hashes).await?;
    let chunk_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT census_hash FROM
        candidate_analysis_input_chunk_censuses WHERE snapshot_id=$1 ORDER BY snapshot_input_id"#,
    )
    .bind(input.snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    let input_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1",
    )
    .bind(input.snapshot_id)
    .fetch_one(&mut **tx)
    .await?;
    if input_count == 0 || chunk_hashes.len() as i64 != input_count {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let input_chunk_census_set_hash = hash_text_array_on(tx, &chunk_hashes).await?;
    let subreview_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT census_hash FROM
        candidate_analysis_hypothesis_coverage_subreview_censuses WHERE analysis_attempt_id=$1
        ORDER BY snapshot_input_id"#,
    )
    .bind(input.analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    if subreview_hashes.len() as i64 != input_count {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let coverage_subreview_census_set_hash = hash_text_array_on(tx, &subreview_hashes).await?;
    let (coverage_synthesis_census_set_hash,global_node_id):(String,Uuid)=sqlx::query_as(r#"SELECT
        census_hash,global_root_node_id FROM candidate_analysis_hypothesis_coverage_synthesis_censuses
        WHERE analysis_attempt_id=$1"#).bind(input.analysis_attempt_id).fetch_optional(&mut **tx).await?
        .ok_or_else(||conflict(CENSUS_NOT_CLOSED))?;
    let coverage_global_semantic_root_hash: String = sqlx::query_scalar(
        r#"SELECT node_hash FROM
        candidate_analysis_hypothesis_coverage_synthesis_census_members WHERE synthesis_node_id=$1
        AND analysis_attempt_id=$2 AND node_kind='global_semantic_root'"#,
    )
    .bind(global_node_id)
    .bind(input.analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    let coverage_global_review_hash: String = sqlx::query_scalar(
        r#"SELECT review_hash FROM
        candidate_analysis_hypothesis_coverage_global_reviews WHERE analysis_attempt_id=$1"#,
    )
    .bind(input.analysis_attempt_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    let coverage_hashes:Vec<String>=sqlx::query_scalar(r#"SELECT review_hash FROM
        candidate_analysis_hypothesis_coverage_reviews WHERE analysis_attempt_id=$1 ORDER BY snapshot_input_id"#)
        .bind(input.analysis_attempt_id).fetch_all(&mut **tx).await?;
    if coverage_hashes.len() as i64 != input_count {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let coverage_review_set_hash = hash_text_array_on(tx, &coverage_hashes).await?;
    let checklist_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT member_hash FROM
        candidate_analysis_hypothesis_coverage_checklist_members WHERE analysis_attempt_id=$1
        ORDER BY snapshot_input_id,ordinal"#,
    )
    .bind(input.analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    if checklist_hashes.is_empty() {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let coverage_checklist_set_hash = hash_text_array_on(tx, &checklist_hashes).await?;
    let controller_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT artifact.artifact_hash
              FROM candidate_analysis_artifacts artifact
              JOIN candidate_analysis_work_items candidate_item
                ON candidate_item.candidate_work_item_id=artifact.candidate_work_item_id
               AND candidate_item.analysis_attempt_id=artifact.analysis_attempt_id
               AND candidate_item.phase='controller'
               AND candidate_item.capability='candidate_controller_final'
              JOIN candidate_analysis_provider_attempts provider
                ON provider.artifact_id=artifact.artifact_id
               AND provider.analysis_attempt_id=artifact.analysis_attempt_id
               AND provider.worker_run_id=artifact.worker_run_id
               AND provider.artifact_kind='controller_decision.v1'
               AND provider.artifact_hash=artifact.artifact_hash
             WHERE artifact.analysis_attempt_id=$1
               AND artifact.worker_run_id=$2
               AND artifact.artifact_kind='controller_decision.v1'
             ORDER BY artifact.artifact_hash"#,
    )
    .bind(input.analysis_attempt_id)
    .bind(worker)
    .fetch_all(&mut **tx)
    .await?;
    if controller_hashes.len() != 1 {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    let controller_decision_set_hash = hash_text_array_on(tx, &controller_hashes).await?;
    if proposal != exact_closure.proposal_census_hash
        || exact_closure.critic_census_hash.as_deref() != Some(critic.as_str())
        || coverage_subreview_census_set_hash != exact_closure.coverage_subreview_census_set_hash
        || coverage_checklist_set_hash != exact_closure.coverage_checklist_set_hash
    {
        return Err(conflict(CENSUS_NOT_CLOSED));
    }
    Ok(CandidatePreGateMaterialRow {
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
        exact_closure,
        final_submitter_worker_run_id: worker,
        controller_dispatch_worker_run_id: controller_dispatch_worker,
        snapshot_row_version: 0,
        attempt_row_version: 0,
    })
}

pub async fn load_candidate_gate_material(
    pool: &PgPool,
    input: LoadCandidateGateMaterialInput,
) -> Result<CandidateGateMaterialRow> {
    let mut tx = pool.begin().await?;
    let material = load_candidate_gate_material_on(&mut tx, input).await?;
    tx.commit().await?;
    Ok(material)
}

pub async fn load_candidate_gate_material_on(
    tx: &mut Transaction<'_, Postgres>,
    input: LoadCandidateGateMaterialInput,
) -> Result<CandidateGateMaterialRow> {
    let pre = load_candidate_pre_gate_material_on(tx, input.clone()).await?;
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
    .bind(pre.final_submitter_worker_run_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(CENSUS_NOT_CLOSED))?;
    Ok(CandidateGateMaterialRow {
        snapshot: pre.snapshot,
        active_analysis_attempt_id: pre.active_analysis_attempt_id,
        active_analysis_attempt_ordinal: pre.active_analysis_attempt_ordinal,
        attempt_epoch: pre.attempt_epoch,
        prior_terminal_attempt_chain_hash: pre.prior_terminal_attempt_chain_hash,
        gate_temporal_reevaluation_hash: pre.gate_temporal_reevaluation_hash,
        gate_knowledge_feed_reevaluation_hash: pre.gate_knowledge_feed_reevaluation_hash,
        input_chunk_census_set_hash: pre.input_chunk_census_set_hash,
        proposal_census_hash: pre.proposal_census_hash,
        critic_census_hash: pre.critic_census_hash,
        coverage_subreview_census_set_hash: pre.coverage_subreview_census_set_hash,
        coverage_synthesis_census_set_hash: pre.coverage_synthesis_census_set_hash,
        coverage_global_semantic_root_hash: pre.coverage_global_semantic_root_hash,
        coverage_global_review_hash: pre.coverage_global_review_hash,
        coverage_review_set_hash: pre.coverage_review_set_hash,
        coverage_checklist_set_hash: pre.coverage_checklist_set_hash,
        controller_decision_set_hash: pre.controller_decision_set_hash,
        mutation_set_hash,
        claim_component_set_hash,
        verification_contract_set_hash,
        verification_plan_set_hash,
        generation_transition_set_hash,
        compiler_seal_hash,
        final_submitter_worker_run_id: pre.final_submitter_worker_run_id,
        controller_dispatch_worker_run_id: pre.controller_dispatch_worker_run_id,
        snapshot_row_version: pre.snapshot_row_version,
        attempt_row_version: pre.attempt_row_version,
    })
}

#[cfg(test)]
mod managed_feed_authority_tests {
    use serial_test::serial;
    use sqlx::PgPool;

    use super::*;
    use crate::{DbConfig, GolishDb};

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

    async fn exact_hash(pool: &PgPool, values: &[String]) -> String {
        sqlx::query_scalar("SELECT tool_truth_sha256(to_jsonb($1::TEXT[])::TEXT)")
            .bind(values)
            .fetch_one(pool)
            .await
            .expect("hash exact set")
    }

    #[tokio::test]
    #[serial]
    async fn managed_feed_store_selection_is_positive_and_schema_drift_blocks() {
        let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
        let db = GolishDb::start(DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port(),
            database: format!("managed_feed_{}", Uuid::new_v4().simple()),
            ..DbConfig::default()
        })
        .await
        .expect("start isolated migrated postgres");
        let pool = db.pool();
        let operation_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO operation_state(
                   operation_id,profile,current_stage,runtime_memory_contract,
                   attack_execution_contract,tool_truth_contract)
               VALUES($1,'assessment','target_intel','legacy_v1','legacy','legacy_v1')"#,
        )
        .bind(operation_id)
        .execute(pool)
        .await
        .expect("insert Registry operation");

        let catalog_id = Uuid::new_v4();
        let trust_policy_id = Uuid::new_v4();
        let sources = [
            ("cve", "managed:cve"),
            ("cpe", "managed:cpe"),
            ("kev", "managed:kev"),
            ("vendor_advisory", "managed:vendor-advisory"),
            ("detection_rule", "managed:detection-rule"),
        ];
        let mut source_kinds = sources
            .iter()
            .map(|(kind, _)| (*kind).to_owned())
            .collect::<Vec<_>>();
        source_kinds.sort();
        let source_hash = exact_hash(pool, &source_kinds).await;
        let member_hashes = sources
            .iter()
            .enumerate()
            .map(|(ordinal, _)| digest(char::from_digit((ordinal + 1) as u32, 16).unwrap()))
            .collect::<Vec<_>>();
        let member_set_hash = exact_hash(pool, &member_hashes).await;
        sqlx::query(
            r#"INSERT INTO candidate_managed_feed_catalogs(
                   catalog_id,catalog_version,catalog_hash,trust_policy_id,
                   trust_policy_version,trust_policy_hash,signature_algorithm_allowlist_hash,
                   required_source_count,required_source_set_hash,required_member_count,
                   required_member_set_hash)
               VALUES($1,1,$2,$3,1,$4,$5,5,$6,5,$7)"#,
        )
        .bind(catalog_id)
        .bind(digest('a'))
        .bind(trust_policy_id)
        .bind(digest('b'))
        .bind(digest('c'))
        .bind(&source_hash)
        .bind(&member_set_hash)
        .execute(pool)
        .await
        .expect("insert managed catalog");
        sqlx::query(
            r#"INSERT INTO candidate_managed_feed_catalog_head(
                   singleton,catalog_id,catalog_version,catalog_hash,trust_policy_id,
                   trust_policy_version,trust_policy_hash,signature_algorithm_allowlist_hash,
                   required_source_count,required_source_set_hash,required_member_count,
                   required_member_set_hash)
               SELECT TRUE,catalog_id,catalog_version,catalog_hash,trust_policy_id,
                      trust_policy_version,trust_policy_hash,signature_algorithm_allowlist_hash,
                      required_source_count,required_source_set_hash,required_member_count,
                      required_member_set_hash
                 FROM candidate_managed_feed_catalogs WHERE catalog_id=$1"#,
        )
        .bind(catalog_id)
        .execute(pool)
        .await
        .expect("install managed catalog head");
        sqlx::query(
            r#"INSERT INTO candidate_managed_feed_trust_stores(
                   trust_store_version,trust_store_hash,key_revocation_epoch,
                   key_revocation_epoch_hash) VALUES(1,$1,0,$2)"#,
        )
        .bind(digest('d'))
        .bind(digest('e'))
        .execute(pool)
        .await
        .expect("insert trust store");
        sqlx::query(
            r#"INSERT INTO candidate_managed_feed_trust_store_head(
                   singleton,trust_store_version,trust_store_hash,key_revocation_epoch,
                   key_revocation_epoch_hash) VALUES(TRUE,1,$1,0,$2)"#,
        )
        .bind(digest('d'))
        .bind(digest('e'))
        .execute(pool)
        .await
        .expect("install trust head");
        sqlx::query(
            r#"INSERT INTO candidate_managed_feed_signer_keys(
                   signer_key_member_id,trust_store_version,trust_store_hash,
                   key_revocation_epoch,key_revocation_epoch_hash,signer_id,signer_key_id,
                   signature_algorithm,revoked,key_member_hash)
               VALUES($1,1,$2,0,$3,'managed-signer','managed-key','ed25519',FALSE,$4)"#,
        )
        .bind(Uuid::new_v4())
        .bind(digest('d'))
        .bind(digest('e'))
        .bind(digest('f'))
        .execute(pool)
        .await
        .expect("install signer key");
        let mut first_catalog_member = None;
        for (ordinal, ((source_kind, source_identity), member_hash)) in
            sources.into_iter().zip(member_hashes).enumerate()
        {
            let catalog_member_id = Uuid::new_v4();
            first_catalog_member.get_or_insert(catalog_member_id);
            sqlx::query(
                r#"INSERT INTO candidate_managed_feed_catalog_members(
                       catalog_member_id,catalog_id,ordinal,source_kind,source_identity,
                       schema_name,schema_version,member_hash)
                   VALUES($1,$2,$3,$4,$5,'managed_knowledge_feed.v1',1,$6)"#,
            )
            .bind(catalog_member_id)
            .bind(catalog_id)
            .bind(i32::try_from(ordinal).unwrap())
            .bind(source_kind)
            .bind(source_identity)
            .bind(member_hash)
            .execute(pool)
            .await
            .expect("insert catalog member");
            let store_member_id = Uuid::new_v4();
            sqlx::query(
                r#"INSERT INTO candidate_managed_feed_store_members(
                       store_member_id,catalog_member_id,catalog_id,feed_id,source_id,
                       feed_schema,feed_version,published_at,host_ingested_at,
                       effective_valid_until,content_hash,signed_manifest_hash,signer_id,
                       signer_key_id,signature_algorithm,signature_verification_receipt_hash,
                       signer_key_member_hash,provenance,age_policy_version,age_policy_digest,
                       immutable_feed_body,member_hash)
                   VALUES($1,$2,$3,$4,$5,'managed_knowledge_feed.v1',1,
                          statement_timestamp()-INTERVAL '1 minute',statement_timestamp(),
                          statement_timestamp()+INTERVAL '1 hour',$6,$7,'managed-signer',
                          'managed-key','ed25519',$8,$9,'{}','v1',$10,$11,$12)"#,
            )
            .bind(store_member_id)
            .bind(catalog_member_id)
            .bind(catalog_id)
            .bind(format!("feed:{source_kind}"))
            .bind(source_identity)
            .bind(digest('1'))
            .bind(digest('2'))
            .bind(digest('3'))
            .bind(digest('f'))
            .bind(digest('4'))
            .bind(json!({
                "entries":[{
                    "entry_kind":source_kind,
                    "entry_id":format!("{source_kind}:nginx:1.2.3"),
                    "entry_version":"1",
                    "cpe":"cpe:2.3:a:nginx:nginx:*:*:*:*:*:*:*:*",
                    "affected_versions":["1.2.3"],
                    "matched_range":"exact",
                }]
            }))
            .bind(digest('5'))
            .execute(pool)
            .await
            .expect("insert signed store member");
            sqlx::query(
                r#"INSERT INTO candidate_managed_feed_store_member_heads(
                       catalog_member_id,catalog_id,store_member_id)
                   VALUES($1,$2,$3)"#,
            )
            .bind(catalog_member_id)
            .bind(catalog_id)
            .bind(store_member_id)
            .execute(pool)
            .await
            .expect("install store member head");
        }

        let mut tx = pool.begin().await.expect("begin positive selection");
        let selected = select_managed_feed_authority_on(&mut tx, operation_id, Uuid::new_v4())
            .await
            .expect("select complete signed managed feed");
        assert_eq!(selected.store_catalog_id, Some(catalog_id));
        assert_eq!(selected.blocked_reason, None);
        tx.commit().await.expect("freeze operation feed contract");

        let snapshot_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let bundle_seal_id = Uuid::new_v4();
        let mut seed_tx = pool.begin().await.expect("begin snapshot seed");
        sqlx::query("SET LOCAL session_replication_role='replica'")
            .execute(&mut *seed_tx)
            .await
            .expect("disable fixture referential triggers");
        sqlx::query(
            r#"INSERT INTO candidate_analysis_snapshots(
                   snapshot_id,operation_id,organization_id,wave_ordinal,genesis,
                   source_set_hash,capability_revision_hash,policy_revision_hash,
                   credential_revision_hash,snapshot_status,tool_truth_authority_bundle_seal_id,
                   stable_consumer_request_id,relevant_root_count,relevant_root_set_hash,
                   bundle_member_count,bundle_member_set_hash,semantic_authority_bundle_hash,
                   freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
                   temporal_validity_policy_set_hash,target_state_epoch_set_hash,
                   observation_window_hash,bundle_sealed_at,candidate_snapshot_authority_hash)
               VALUES($1,$2,$3,0,TRUE,$4,$4,$4,$4,'blocked_authority_bundle',$5,$6,
                      4,$4,4,$4,$4,$4,$4,$4,$4,$4,statement_timestamp(),$4)"#,
        )
        .bind(snapshot_id)
        .bind(operation_id)
        .bind(organization_id)
        .bind(digest('9'))
        .bind(bundle_seal_id)
        .bind(Uuid::new_v4())
        .execute(&mut *seed_tx)
        .await
        .expect("seed isolated Candidate snapshot owner row");
        let receipt_id = Uuid::new_v4();
        let execution_authority_id = Uuid::new_v4();
        let denominator_id = Uuid::new_v4();
        let authority_set_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO capability_execution_receipts(
                   id,denominator_id,execution_authority_id,capability,attempt_ordinal,
                   receipt_authority_hash,input_manifest_hash,destination_policy_id,
                   destination_policy_hash,temporal_validity_policy_id,
                   temporal_validity_policy_hash,attempt_state,landing_state,
                   observation_state,coverage_extent,coverage_gap_reason,
                   reconciliation_state,security_interpretation,typed_landing)
               VALUES($1,$2,$3,'application_model_snapshot',1,$4,$4,$5,$4,$6,$4,
                      'running','not_attempted','indeterminate','none','none','pending',
                      'not_assessed',$7)"#,
        )
        .bind(receipt_id)
        .bind(denominator_id)
        .bind(execution_authority_id)
        .bind(digest('8'))
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(json!({
            "application_products":[
                {
                    "subject_kind":"service",
                    "subject_identity_hash":digest('6'),
                    "product_identity":"nginx",
                    "cpe_candidates":["cpe:2.3:a:nginx:nginx:*:*:*:*:*:*:*:*"],
                    "observed_version":"1.2.3"
                },
                {
                    "subject_kind":"service",
                    "subject_identity_hash":digest('7'),
                    "product_identity":"openssl",
                    "cpe_candidates":["cpe:2.3:a:openssl:openssl:*:*:*:*:*:*:*:*"],
                    "observed_version":null
                }
            ]
        }))
        .execute(&mut *seed_tx)
        .await
        .expect("seed frozen application product receipt");
        sqlx::query(
            r#"INSERT INTO tool_truth_authority_set_members(
                   id,authority_set_id,execution_authority_id,denominator_id,receipt_id,
                   reconciliation_id,semantic_authority_version,semantic_hash,ordinal,member_hash)
               VALUES($1,$2,$3,$4,$5,$6,1,$7,0,$7)"#,
        )
        .bind(Uuid::new_v4())
        .bind(authority_set_id)
        .bind(execution_authority_id)
        .bind(denominator_id)
        .bind(receipt_id)
        .bind(Uuid::new_v4())
        .bind(digest('8'))
        .execute(&mut *seed_tx)
        .await
        .expect("seed frozen application authority-set member");
        sqlx::query(
            r#"INSERT INTO tool_truth_authority_bundle_members(
                   id,bundle_seal_id,operation_id,organization_id,ordinal,root_family,
                   root_execution_authority_id,root_denominator_id,root_denominator_hash,
                   authority_set_seal_id,authority_set_semantic_hash,
                   authority_set_graph_hash,authority_set_freshness_hash,
                   temporal_validity_policy_set_hash,target_state_epoch_set_hash,
                   observation_window_started_at,observation_window_completed_at,
                   effective_valid_until,
                   semantic_status,temporal_validity_status,member_status,member_hash)
               VALUES($1,$2,$3,$4,0,'ti',$5,$6,$7,$8,$7,$7,$7,$7,$7,
                      statement_timestamp()-INTERVAL '2 minutes',
                      statement_timestamp()-INTERVAL '1 minute',
                      statement_timestamp()+INTERVAL '1 hour',
                      'consistent','fresh','consistent_fresh',$7)"#,
        )
        .bind(Uuid::new_v4())
        .bind(bundle_seal_id)
        .bind(operation_id)
        .bind(organization_id)
        .bind(execution_authority_id)
        .bind(denominator_id)
        .bind(digest('8'))
        .bind(authority_set_id)
        .execute(&mut *seed_tx)
        .await
        .expect("seed frozen application authority-bundle member");
        seed_tx.commit().await.expect("commit snapshot seed");
        let mut persist_tx = pool.begin().await.expect("begin feed freeze");
        persist_managed_feed_store_authority_on(
            &mut persist_tx,
            snapshot_id,
            operation_id,
            catalog_id,
        )
        .await
        .expect("freeze positive local managed-feed authority");
        persist_tx.commit().await.expect("commit feed authority");
        let closure: (i64, i64, i64, i64) = sqlx::query_as(
            r#"SELECT
                 (SELECT COUNT(*) FROM candidate_analysis_knowledge_feed_denominator_members WHERE snapshot_id=$1),
                 (SELECT COUNT(*) FROM candidate_analysis_knowledge_feed_snapshot_members WHERE snapshot_id=$1),
                 (SELECT COUNT(*) FROM candidate_analysis_product_version_censuses WHERE snapshot_id=$1),
                 (SELECT COUNT(*) FROM candidate_analysis_feed_match_censuses WHERE snapshot_id=$1)"#,
        )
        .bind(snapshot_id)
        .fetch_one(pool)
        .await
        .expect("inspect frozen managed-feed closure");
        assert_eq!(closure, (5, 5, 1, 1));
        let matcher_closure: (i64, i64, i64, i64, i64) = sqlx::query_as(
            r#"SELECT
                 (SELECT COUNT(*) FROM candidate_analysis_product_version_census_members
                    WHERE snapshot_id=$1),
                 (SELECT COUNT(*) FROM candidate_analysis_feed_match_census_members
                    WHERE snapshot_id=$1),
                 (SELECT COUNT(*) FROM candidate_analysis_feed_match_census_members
                    WHERE snapshot_id=$1 AND disposition='matched'),
                 (SELECT COUNT(*) FROM candidate_analysis_feed_match_census_members
                    WHERE snapshot_id=$1 AND disposition='unknown_product_version'),
                 (SELECT COUNT(*) FROM candidate_analysis_enrichment_obligations
                    WHERE snapshot_id=$1 AND obligation_kind='product_version_enrichment')"#,
        )
        .bind(snapshot_id)
        .fetch_one(pool)
        .await
        .expect("inspect deterministic product/feed matcher closure");
        assert_eq!(matcher_closure, (2, 10, 5, 5, 1));

        let catalog_member_id = first_catalog_member.expect("first catalog member");
        let bad_store_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO candidate_managed_feed_store_members(
                   store_member_id,catalog_member_id,catalog_id,feed_id,source_id,
                   feed_schema,feed_version,published_at,host_ingested_at,effective_valid_until,
                   content_hash,signed_manifest_hash,signer_id,signer_key_id,
                   signature_algorithm,signature_verification_receipt_hash,
                   signer_key_member_hash,provenance,age_policy_version,age_policy_digest,
                   immutable_feed_body,member_hash)
               VALUES($1,$2,$3,'bad-version','managed:cve','managed_knowledge_feed.v1',2,
                      statement_timestamp()-INTERVAL '1 minute',statement_timestamp(),
                      statement_timestamp()+INTERVAL '1 hour',$4,$5,'managed-signer',
                      'managed-key','ed25519',$6,$7,'{}','v1',$8,'{}',$9)"#,
        )
        .bind(bad_store_id)
        .bind(catalog_member_id)
        .bind(catalog_id)
        .bind(digest('1'))
        .bind(digest('2'))
        .bind(digest('3'))
        .bind(digest('f'))
        .bind(digest('4'))
        .bind(digest('5'))
        .execute(pool)
        .await
        .expect("insert unknown-version store member");
        sqlx::query(
            r#"UPDATE candidate_managed_feed_store_member_heads
                  SET store_member_id=$1,head_version=head_version+1
                WHERE catalog_member_id=$2"#,
        )
        .bind(bad_store_id)
        .bind(catalog_member_id)
        .execute(pool)
        .await
        .expect("advance server-owned store head");
        let mut tx = pool.begin().await.expect("begin negative selection");
        let blocked = select_managed_feed_authority_on(&mut tx, operation_id, Uuid::new_v4())
            .await
            .expect("classify invalid managed feed");
        assert_eq!(blocked.store_catalog_id, None);
        assert_eq!(
            blocked.blocked_reason,
            Some(ManagedFeedBlockReason::SchemaVersionUnsupported)
        );
        tx.rollback().await.expect("rollback negative selection");
    }
}
