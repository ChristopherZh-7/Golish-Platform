//! Opaque, request-scoped multi-root Tool Truth authority host.
//!
//! The caller supplies only stable consumer and operation identity. Root,
//! receipt, semantic, freshness, temporal-policy and target-epoch censuses are
//! derived while locked in the same repeatable-read transaction that invokes
//! the consumer callback. Guard constructors stay private to this module.

use std::{collections::BTreeMap, future::Future, marker::PhantomData, pin::Pin};

use chrono::{DateTime, Utc};
use golish_pentest_domain::tool_truth::{
    EvidenceTemporalValidityPolicyMemberV1, EvidenceTemporalValidityPolicyV1,
    TemporalValidityStatus, ToolTruthRootFamilyV1,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{fail, sha256_json, AUTHORITY_STALE, CONTRACT_INVALID, MANIFEST_DRIFT};
use crate::{repo::tool_truth_revalidation, Result};

const BUNDLE_ROOT_CENSUS_INCOMPLETE: &str = "TOOL_TRUTH_AUTHORITY_BUNDLE_ROOT_CENSUS_INCOMPLETE";
const BUNDLE_DRIFT: &str = "TOOL_TRUTH_AUTHORITY_BUNDLE_DRIFT";
const BUNDLE_NOT_ALL_FRESH: &str = "TOOL_TRUTH_AUTHORITY_BUNDLE_NOT_ALL_FRESH";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTruthAuthorityBundleConsumerV1 {
    CandidateAnalysis,
    VerificationCampaign,
    CurrentReport,
    ReportDownload,
    Ui,
}

impl ToolTruthAuthorityBundleConsumerV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CandidateAnalysis => "candidate_analysis",
            Self::VerificationCampaign => "verification_campaign",
            Self::CurrentReport => "current_report",
            Self::ReportDownload => "report_download",
            Self::Ui => "ui",
        }
    }

    const fn obligation_consumer_kind(self) -> &'static str {
        match self {
            Self::CandidateAnalysis => "candidate",
            Self::VerificationCampaign => "campaign",
            Self::CurrentReport => "reporting",
            Self::ReportDownload => "report_download",
            Self::Ui => "ui",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckToolTruthAuthorityBundle {
    pub stable_consumer_request_id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub consumer_kind: ToolTruthAuthorityBundleConsumerV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTruthAuthorityBundleMemberStatusV1 {
    ConsistentFresh,
    SemanticInvalid,
    Expired,
    MixedEpoch,
    SkewExceeded,
}

impl ToolTruthAuthorityBundleMemberStatusV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConsistentFresh => "consistent_fresh",
            Self::SemanticInvalid => "semantic_invalid",
            Self::Expired => "expired",
            Self::MixedEpoch => "mixed_epoch",
            Self::SkewExceeded => "skew_exceeded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedToolTruthAuthorityRoot {
    pub root_family: ToolTruthRootFamilyV1,
    pub root_denominator_id: Uuid,
    pub root_denominator_hash: String,
    pub authority_set_seal_id: Uuid,
    pub authority_set_graph_hash: String,
    pub authority_set_semantic_hash: String,
    pub authority_set_freshness_hash: String,
    pub temporal_validity_policy_set_hash: String,
    pub target_state_epoch_set_hash: String,
    pub observation_window_started_at: Option<DateTime<Utc>>,
    pub observation_window_completed_at: Option<DateTime<Utc>>,
    pub effective_valid_until: Option<DateTime<Utc>>,
    pub semantic_status: String,
    pub temporal_validity_status: TemporalValidityStatus,
    pub member_status: ToolTruthAuthorityBundleMemberStatusV1,
    pub temporal_policies: Vec<EvidenceTemporalValidityPolicyV1>,
    pub revalidation_obligation_ids: Vec<Uuid>,
}

struct CheckedBundleState {
    bundle_seal_id: Uuid,
    operation_id: Uuid,
    organization_id: Uuid,
    relevant_root_set_hash: String,
    member_set_hash: String,
    semantic_authority_bundle_hash: String,
    freshness_attestation_bundle_hash: String,
    temporal_validity_bundle_hash: String,
    temporal_validity_policy_set_hash: String,
    target_state_epoch_set_hash: String,
    observation_window_started_at: Option<DateTime<Utc>>,
    observation_window_completed_at: Option<DateTime<Utc>>,
    effective_valid_until: Option<DateTime<Utc>>,
    roots: Vec<CheckedToolTruthAuthorityRoot>,
}

/// Non-cloneable and non-serializable. Its invariant lifetime is created only
/// while the host retains the locked transaction and complete root census.
pub struct CheckedToolTruthAuthorityBundle<'guard> {
    state: CheckedBundleState,
    _invariant: PhantomData<&'guard mut &'guard ()>,
}

impl<'guard> CheckedToolTruthAuthorityBundle<'guard> {
    pub fn bundle_seal_id(&self) -> Uuid {
        self.state.bundle_seal_id
    }

    pub fn operation_id(&self) -> Uuid {
        self.state.operation_id
    }

    pub fn organization_id(&self) -> Uuid {
        self.state.organization_id
    }

    pub fn roots(&self) -> &[CheckedToolTruthAuthorityRoot] {
        &self.state.roots
    }

    pub fn relevant_root_set_hash(&self) -> &str {
        &self.state.relevant_root_set_hash
    }

    pub fn member_set_hash(&self) -> &str {
        &self.state.member_set_hash
    }

    pub fn semantic_authority_bundle_hash(&self) -> &str {
        &self.state.semantic_authority_bundle_hash
    }

    pub fn freshness_attestation_bundle_hash(&self) -> &str {
        &self.state.freshness_attestation_bundle_hash
    }

    pub fn temporal_validity_bundle_hash(&self) -> &str {
        &self.state.temporal_validity_bundle_hash
    }

    pub fn temporal_validity_policy_set_hash(&self) -> &str {
        &self.state.temporal_validity_policy_set_hash
    }

    pub fn target_state_epoch_set_hash(&self) -> &str {
        &self.state.target_state_epoch_set_hash
    }

    pub fn observation_window(&self) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
        (
            self.state.observation_window_started_at,
            self.state.observation_window_completed_at,
        )
    }

    pub fn effective_valid_until(&self) -> Option<DateTime<Utc>> {
        self.state.effective_valid_until
    }

    pub fn is_all_fresh(&self) -> bool {
        self.state.roots.len() == ToolTruthRootFamilyV1::EXECUTION_RECEIPT_ROOTS.len()
            && self.state.roots.iter().all(|root| {
                root.member_status == ToolTruthAuthorityBundleMemberStatusV1::ConsistentFresh
            })
    }

    fn as_all_fresh(&'guard self) -> Result<AllFreshToolTruthAuthorityBundle<'guard>> {
        if !self.is_all_fresh() {
            return Err(fail(BUNDLE_NOT_ALL_FRESH));
        }
        Ok(AllFreshToolTruthAuthorityBundle {
            checked: self,
            _invariant: PhantomData,
        })
    }
}

/// Stronger guard whose only constructor is the checked bundle's private,
/// exact all-fresh conversion.
pub struct AllFreshToolTruthAuthorityBundle<'guard> {
    checked: &'guard CheckedToolTruthAuthorityBundle<'guard>,
    _invariant: PhantomData<&'guard mut &'guard ()>,
}

impl<'guard> AllFreshToolTruthAuthorityBundle<'guard> {
    pub fn checked(&self) -> &'guard CheckedToolTruthAuthorityBundle<'guard> {
        self.checked
    }

    pub fn bundle_seal_id(&self) -> Uuid {
        self.checked.bundle_seal_id()
    }
}

pub type ToolTruthAuthorityBundleFuture<'guard, T> =
    Pin<Box<dyn Future<Output = Result<T>> + Send + 'guard>>;

#[derive(Debug, sqlx::FromRow)]
struct RootRow {
    operation_id: Uuid,
    denominator_id: Uuid,
    execution_authority_id: Uuid,
    denominator_hash: String,
    stage_kind: String,
    project_scope_id: Uuid,
    project_path_at_freeze: String,
    scope_snapshot_id: Uuid,
}

fn root_matches_bundle_scope(
    root: &RootRow,
    request_operation_id: Uuid,
    bundle_scope_snapshot_id: Uuid,
    bundle_project_scope_id: Uuid,
    bundle_project_path: &str,
    is_stage_fork: bool,
) -> bool {
    root.project_scope_id == bundle_project_scope_id
        && root.project_path_at_freeze == bundle_project_path
        && if is_stage_fork {
            root.operation_id != request_operation_id
                || root.scope_snapshot_id == bundle_scope_snapshot_id
        } else {
            root.scope_snapshot_id == bundle_scope_snapshot_id
        }
}

#[derive(Debug, sqlx::FromRow)]
struct ReceiptRow {
    id: Uuid,
    denominator_id: Uuid,
    execution_authority_id: Uuid,
    capability: String,
    reconciliation_state: String,
    current_semantic_authority_version: i64,
    current_semantic_reconciliation_id: Option<Uuid>,
    current_semantic_reconciliation_hash: Option<String>,
    temporal_validity_policy_id: Uuid,
    temporal_validity_policy_hash: String,
    temporal_census_id: Option<Uuid>,
    finalized_at: Option<DateTime<Utc>>,
    snapshot_authority_id: Option<Uuid>,
    vault_object_ref_token_hash: Option<String>,
    snapshot_sha256: Option<String>,
    snapshot_byte_count: Option<i64>,
}

#[derive(Debug, sqlx::FromRow)]
struct TemporalCensusRow {
    id: Uuid,
    observation_window_started_at: DateTime<Utc>,
    observation_window_completed_at: DateTime<Utc>,
    effective_valid_until: DateTime<Utc>,
    target_state_epoch_set_hash: String,
    max_cross_observation_skew_ms: i64,
}

#[derive(Debug)]
struct ReceiptMember {
    receipt_id: Uuid,
    denominator_id: Uuid,
    execution_authority_id: Uuid,
    reconciliation_id: Uuid,
    semantic_authority_version: i64,
    semantic_hash: String,
    freshness_attestation_id: Uuid,
    freshness_attestation_hash: String,
    member_hash: String,
}

struct RootState {
    root: RootRow,
    graph_hash: String,
    semantic_hash: String,
    freshness_hash: String,
    authority_set_id: Uuid,
    semantic_status: String,
    temporal_status: TemporalValidityStatus,
    member_status: ToolTruthAuthorityBundleMemberStatusV1,
    temporal_policy_set_hash: String,
    target_state_epoch_set_hash: String,
    observation_window_started_at: Option<DateTime<Utc>>,
    observation_window_completed_at: Option<DateTime<Utc>>,
    effective_valid_until: Option<DateTime<Utc>>,
    temporal_policies: Vec<EvidenceTemporalValidityPolicyV1>,
    receipt_ids: Vec<Uuid>,
    revalidation_obligation_ids: Vec<Uuid>,
}

#[derive(Debug, sqlx::FromRow)]
struct BundleHeaderRow {
    id: Uuid,
    relevant_root_set_hash: String,
    member_set_hash: String,
    semantic_authority_bundle_hash: String,
    freshness_attestation_bundle_hash: String,
    temporal_validity_bundle_hash: String,
    temporal_validity_policy_set_hash: String,
    target_state_epoch_set_hash: String,
    observation_window_started_at: Option<DateTime<Utc>>,
    observation_window_completed_at: Option<DateTime<Utc>>,
    effective_valid_until: Option<DateTime<Utc>>,
}

fn temporal_status_wire(status: TemporalValidityStatus) -> &'static str {
    match status {
        TemporalValidityStatus::Fresh => "fresh",
        TemporalValidityStatus::Expired => "expired",
        TemporalValidityStatus::MixedEpoch => "mixed_epoch",
        TemporalValidityStatus::SkewExceeded => "skew_exceeded",
    }
}

fn temporal_member_status(
    semantic_status: &str,
    temporal_status: TemporalValidityStatus,
) -> ToolTruthAuthorityBundleMemberStatusV1 {
    if semantic_status != "consistent" {
        return ToolTruthAuthorityBundleMemberStatusV1::SemanticInvalid;
    }
    match temporal_status {
        TemporalValidityStatus::Fresh => ToolTruthAuthorityBundleMemberStatusV1::ConsistentFresh,
        TemporalValidityStatus::Expired => ToolTruthAuthorityBundleMemberStatusV1::Expired,
        TemporalValidityStatus::MixedEpoch => ToolTruthAuthorityBundleMemberStatusV1::MixedEpoch,
        TemporalValidityStatus::SkewExceeded => {
            ToolTruthAuthorityBundleMemberStatusV1::SkewExceeded
        }
    }
}

async fn load_roots(
    tx: &mut Transaction<'static, Postgres>,
    request: &CheckToolTruthAuthorityBundle,
) -> Result<Vec<RootRow>> {
    let stage_kinds = ToolTruthRootFamilyV1::EXECUTION_RECEIPT_ROOTS
        .map(|family| family.stage_kind().to_string())
        .to_vec();
    let candidates = sqlx::query_as::<_, RootRow>(
        r#"SELECT d.operation_id,d.id AS denominator_id,d.execution_authority_id,d.denominator_hash,
                  d.stage_kind,d.project_scope_id,d.project_path_at_freeze,
                  d.scope_snapshot_id
             FROM coverage_denominators d
             JOIN tool_truth_execution_authorities authority
               ON authority.id=d.execution_authority_id
            WHERE d.organization_id=$2
              AND d.denominator_kind='root' AND d.sealed_at IS NOT NULL
              AND authority.execution_owner_kind='host_stage'
              AND d.stage_kind=ANY($3)
              AND (
                   d.operation_id=$1
                   OR EXISTS (
                       SELECT 1
                         FROM operation_stage_forks fork
                         JOIN operation_stage_fork_inputs input
                           ON input.operation_id=fork.operation_id
                          AND input.source_operation_id=fork.source_operation_id
                          AND input.source_scope_snapshot_id=fork.source_scope_snapshot_id
                          AND input.organization_id=$2
                          AND input.source_stage_kind=d.stage_kind
                         JOIN operation_state source_operation
                           ON source_operation.operation_id=input.source_operation_id
                          AND source_operation.superseded_by IS NULL
                         JOIN stage_handoffs source_handoff
                           ON source_handoff.id=input.source_handoff_id
                          AND source_handoff.operation_id=input.source_operation_id
                          AND source_handoff.organization_id=input.organization_id
                          AND source_handoff.from_stage_kind=input.source_stage_kind
                          AND source_handoff.invalidated_at IS NULL
                        WHERE fork.operation_id=$1
                          AND input.source_operation_id=d.operation_id
                          AND input.source_scope_snapshot_id=d.scope_snapshot_id
                   )
              )
            ORDER BY d.stage_kind,(d.operation_id=$1) DESC,d.created_at DESC,d.id DESC
            FOR SHARE OF d,authority"#,
    )
    .bind(request.operation_id)
    .bind(request.organization_id)
    .bind(stage_kinds)
    .fetch_all(&mut **tx)
    .await?;
    let mut by_family = BTreeMap::new();
    for candidate in candidates {
        let family = ToolTruthRootFamilyV1::from_stage_kind(&candidate.stage_kind)
            .map_err(|_| fail(CONTRACT_INVALID))?;
        by_family.entry(family).or_insert(candidate);
    }
    if by_family.len() != ToolTruthRootFamilyV1::EXECUTION_RECEIPT_ROOTS.len()
        || ToolTruthRootFamilyV1::EXECUTION_RECEIPT_ROOTS
            .iter()
            .any(|family| !by_family.contains_key(family))
    {
        return Err(fail(BUNDLE_ROOT_CENSUS_INCOMPLETE));
    }
    Ok(ToolTruthRootFamilyV1::EXECUTION_RECEIPT_ROOTS
        .into_iter()
        .map(|family| {
            by_family
                .remove(&family)
                .expect("checked exact root family")
        })
        .collect())
}

async fn load_policy(
    tx: &mut Transaction<'static, Postgres>,
    policy_id: Uuid,
) -> Result<EvidenceTemporalValidityPolicyV1> {
    let header = sqlx::query_as::<_, (Uuid, Uuid, String, i64, String, String)>(
        r#"SELECT id,execution_authority_id,policy_contract_version,
                  max_cross_observation_skew_ms,policy_hash,member_set_hash
             FROM evidence_temporal_validity_policies
            WHERE id=$1 AND sealed_at IS NOT NULL FOR SHARE"#,
    )
    .bind(policy_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| fail(AUTHORITY_STALE))?;
    let members = sqlx::query_as::<_, (i32, String, i64, i64, i64, bool, String, String)>(
        r#"SELECT ordinal,fact_class,positive_ttl_ms,negative_ttl_ms,refutation_ttl_ms,
                  require_same_target_state_epoch,required_recheck_source,member_hash
             FROM evidence_temporal_validity_policy_members
            WHERE policy_id=$1 ORDER BY ordinal FOR SHARE"#,
    )
    .bind(policy_id)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|member| {
        Ok(EvidenceTemporalValidityPolicyMemberV1 {
            ordinal: u32::try_from(member.0).map_err(|_| fail(AUTHORITY_STALE))?,
            fact_class: member.1,
            positive_ttl_ms: u64::try_from(member.2).map_err(|_| fail(AUTHORITY_STALE))?,
            negative_ttl_ms: u64::try_from(member.3).map_err(|_| fail(AUTHORITY_STALE))?,
            refutation_ttl_ms: u64::try_from(member.4).map_err(|_| fail(AUTHORITY_STALE))?,
            require_same_target_state_epoch: member.5,
            required_recheck_source: member.6,
            member_hash: member.7,
        })
    })
    .collect::<Result<Vec<_>>>()?;
    let policy = EvidenceTemporalValidityPolicyV1 {
        id: header.0,
        execution_authority_id: header.1,
        policy_contract_version: header.2,
        max_cross_observation_skew_ms: u64::try_from(header.3)
            .map_err(|_| fail(AUTHORITY_STALE))?,
        policy_hash: header.4,
        member_set_hash: header.5,
        members,
    };
    policy.validate().map_err(|_| fail(AUTHORITY_STALE))?;
    Ok(policy)
}

async fn attest_receipt(
    tx: &mut Transaction<'static, Postgres>,
    stable_request_id: Uuid,
    consumer_kind: &str,
    receipt: &ReceiptRow,
) -> Result<Option<ReceiptMember>> {
    let (Some(reconciliation_id), Some(semantic_hash), Some(snapshot_authority_id)) = (
        receipt.current_semantic_reconciliation_id,
        receipt.current_semantic_reconciliation_hash.as_ref(),
        receipt.snapshot_authority_id,
    ) else {
        return Ok(None);
    };
    let (Some(object_identity_hash), Some(snapshot_sha256), Some(snapshot_byte_count)) = (
        receipt.vault_object_ref_token_hash.as_ref(),
        receipt.snapshot_sha256.as_ref(),
        receipt.snapshot_byte_count,
    ) else {
        return Ok(None);
    };
    sqlx::query("SELECT id FROM capability_execution_receipts WHERE id=$1 FOR UPDATE")
        .bind(receipt.id)
        .execute(&mut **tx)
        .await?;
    let status = if receipt.reconciliation_state == "consistent" {
        "consistent"
    } else {
        "orphaned"
    };
    let existing = sqlx::query_as::<_, (Uuid, String, String)>(
        r#"SELECT id,attestation_hash,freshness_status
             FROM capability_execution_freshness_attestations
            WHERE stable_consumer_request_id=$1 AND receipt_id=$2 FOR SHARE"#,
    )
    .bind(stable_request_id)
    .bind(receipt.id)
    .fetch_optional(&mut **tx)
    .await?;
    let (attestation_id, attestation_hash) = if let Some(existing) = existing {
        if existing.2 != status {
            return Err(fail(BUNDLE_DRIFT));
        }
        (existing.0, existing.1)
    } else {
        let predecessor = sqlx::query_as::<_, (Uuid, i64)>(
            r#"SELECT id,event_ordinal FROM capability_execution_freshness_attestations
                WHERE receipt_id=$1 ORDER BY event_ordinal DESC LIMIT 1 FOR SHARE"#,
        )
        .bind(receipt.id)
        .fetch_optional(&mut **tx)
        .await?;
        let event_ordinal = predecessor.map_or(0, |value| value.1 + 1);
        let predecessor_id = predecessor.map(|value| value.0);
        let attestation_hash = sha256_json(&serde_json::json!({
            "receipt_id": receipt.id,
            "reconciliation_id": reconciliation_id,
            "semantic_authority_version": receipt.current_semantic_authority_version,
            "semantic_hash": semantic_hash,
            "stable_consumer_request_id": stable_request_id,
            "artifact_id": snapshot_authority_id,
            "artifact_object_identity_hash": object_identity_hash,
            "snapshot_sha256": snapshot_sha256,
            "snapshot_byte_count": snapshot_byte_count,
            "freshness_status": status,
        }))?;
        let attestation_id = Uuid::new_v5(
            &stable_request_id,
            format!("receipt:{}:{attestation_hash}", receipt.id).as_bytes(),
        );
        sqlx::query(
            r#"INSERT INTO capability_execution_freshness_attestations(
                   id,receipt_id,reconciliation_id,semantic_authority_version,
                   semantic_hash,execution_authority_id,predecessor_attestation_id,
                   event_ordinal,consumer_kind,stable_consumer_request_id,
                   artifact_object_identity_hash,snapshot_sha256,snapshot_byte_count,
                   freshness_status,attestation_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)"#,
        )
        .bind(attestation_id)
        .bind(receipt.id)
        .bind(reconciliation_id)
        .bind(receipt.current_semantic_authority_version)
        .bind(semantic_hash)
        .bind(receipt.execution_authority_id)
        .bind(predecessor_id)
        .bind(event_ordinal)
        .bind(consumer_kind)
        .bind(stable_request_id)
        .bind(object_identity_hash)
        .bind(snapshot_sha256)
        .bind(snapshot_byte_count)
        .bind(status)
        .bind(&attestation_hash)
        .execute(&mut **tx)
        .await?;
        (attestation_id, attestation_hash)
    };
    let member_hash = sha256_json(&serde_json::json!({
        "receipt_id": receipt.id,
        "denominator_id": receipt.denominator_id,
        "reconciliation_id": reconciliation_id,
        "semantic_authority_version": receipt.current_semantic_authority_version,
        "semantic_hash": semantic_hash,
        "freshness_attestation_id": attestation_id,
        "freshness_attestation_hash": attestation_hash,
    }))?;
    Ok(Some(ReceiptMember {
        receipt_id: receipt.id,
        denominator_id: receipt.denominator_id,
        execution_authority_id: receipt.execution_authority_id,
        reconciliation_id,
        semantic_authority_version: receipt.current_semantic_authority_version,
        semantic_hash: semantic_hash.clone(),
        freshness_attestation_id: attestation_id,
        freshness_attestation_hash: attestation_hash,
        member_hash,
    }))
}

async fn seal_authority_set(
    tx: &mut Transaction<'static, Postgres>,
    request: &CheckToolTruthAuthorityBundle,
    family: ToolTruthRootFamilyV1,
    root: &RootRow,
    graph_hash: &str,
    semantic_hash: &str,
    freshness_hash: &str,
    members: &[ReceiptMember],
) -> Result<Uuid> {
    let stable_set_request_id = Uuid::new_v5(
        &request.stable_consumer_request_id,
        format!("authority-set:{}", family.as_str()).as_bytes(),
    );
    if let Some(existing) = sqlx::query_as::<_, (Uuid, Uuid, String, String, String, bool)>(
        r#"SELECT id,denominator_id,graph_hash,semantic_hash,freshness_hash,
                  sealed_at IS NOT NULL
             FROM tool_truth_authority_set_seals
            WHERE execution_authority_id=$1 AND stable_consumer_request_id=$2 FOR SHARE"#,
    )
    .bind(root.execution_authority_id)
    .bind(stable_set_request_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        if existing.1 != root.denominator_id
            || existing.2 != graph_hash
            || existing.3 != semantic_hash
            || existing.4 != freshness_hash
            || !existing.5
        {
            return Err(fail(BUNDLE_DRIFT));
        }
        let existing_members = sqlx::query_scalar::<_, String>(
            r#"SELECT member_hash FROM tool_truth_authority_set_members
                WHERE authority_set_id=$1 ORDER BY ordinal"#,
        )
        .bind(existing.0)
        .fetch_all(&mut **tx)
        .await?;
        if existing_members
            != members
                .iter()
                .map(|member| member.member_hash.clone())
                .collect::<Vec<_>>()
        {
            return Err(fail(BUNDLE_DRIFT));
        }
        return Ok(existing.0);
    }

    let set_id = Uuid::new_v5(
        &stable_set_request_id,
        format!(
            "{}:{graph_hash}:{semantic_hash}:{freshness_hash}",
            root.denominator_id
        )
        .as_bytes(),
    );
    sqlx::query(
        r#"INSERT INTO tool_truth_authority_set_seals(
               id,stable_consumer_request_id,execution_authority_id,denominator_id,
               denominator_hash,consumer_kind,graph_hash,semantic_hash,freshness_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(set_id)
    .bind(stable_set_request_id)
    .bind(root.execution_authority_id)
    .bind(root.denominator_id)
    .bind(&root.denominator_hash)
    .bind(request.consumer_kind.as_str())
    .bind(graph_hash)
    .bind(semantic_hash)
    .bind(freshness_hash)
    .execute(&mut **tx)
    .await?;
    for (ordinal, member) in members.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO tool_truth_authority_set_members(
                   id,authority_set_id,execution_authority_id,denominator_id,
                   receipt_id,reconciliation_id,semantic_authority_version,
                   semantic_hash,freshness_attestation_id,ordinal,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
        )
        .bind(Uuid::new_v5(&set_id, member.member_hash.as_bytes()))
        .bind(set_id)
        .bind(member.execution_authority_id)
        .bind(member.denominator_id)
        .bind(member.receipt_id)
        .bind(member.reconciliation_id)
        .bind(member.semantic_authority_version)
        .bind(&member.semantic_hash)
        .bind(member.freshness_attestation_id)
        .bind(i32::try_from(ordinal).map_err(|_| fail(CONTRACT_INVALID))?)
        .bind(&member.member_hash)
        .execute(&mut **tx)
        .await?;
    }
    sqlx::query(
        "UPDATE tool_truth_authority_set_seals SET sealed_at=statement_timestamp() WHERE id=$1",
    )
    .bind(set_id)
    .execute(&mut **tx)
    .await?;
    Ok(set_id)
}

async fn derive_root_state(
    tx: &mut Transaction<'static, Postgres>,
    request: &CheckToolTruthAuthorityBundle,
    family: ToolTruthRootFamilyV1,
    root: RootRow,
    transaction_now: DateTime<Utc>,
) -> Result<RootState> {
    let graph = sqlx::query_as::<_, (Uuid, String)>(
        r#"WITH RECURSIVE graph(id,denominator_hash) AS (
               SELECT id,denominator_hash FROM coverage_denominators WHERE id=$1
               UNION ALL
               SELECT child.id,child.denominator_hash FROM coverage_denominators child
               JOIN graph parent ON child.parent_denominator_id=parent.id
               WHERE child.execution_authority_id=$2 AND child.sealed_at IS NOT NULL
           ) SELECT id,denominator_hash FROM graph ORDER BY id FOR SHARE"#,
    )
    .bind(root.denominator_id)
    .bind(root.execution_authority_id)
    .fetch_all(&mut **tx)
    .await?;
    let graph_ids = graph.iter().map(|value| value.0).collect::<Vec<_>>();
    let graph_hash = sha256_json(&serde_json::json!(graph))?;
    let receipts = sqlx::query_as::<_, ReceiptRow>(
        r#"WITH latest AS (
               SELECT DISTINCT ON (r.denominator_id,r.capability)
                      r.id,r.denominator_id,r.execution_authority_id,r.capability,
                      r.reconciliation_state,r.current_semantic_authority_version,
                      r.current_semantic_reconciliation_id,r.current_semantic_reconciliation_hash,
                      r.temporal_validity_policy_id,r.temporal_validity_policy_hash,
                      r.temporal_census_id,r.finalized_at,r.raw_witness_artifact_id
                 FROM capability_execution_receipts r
                WHERE r.denominator_id=ANY($1)
                ORDER BY r.denominator_id,r.capability,r.attempt_ordinal DESC,r.id DESC
           )
           SELECT latest.id,latest.denominator_id,latest.execution_authority_id,
                  latest.capability,latest.reconciliation_state,
                  latest.current_semantic_authority_version,
                  latest.current_semantic_reconciliation_id,
                  latest.current_semantic_reconciliation_hash,
                  latest.temporal_validity_policy_id,
                  latest.temporal_validity_policy_hash,
                  latest.temporal_census_id,latest.finalized_at,
                  COALESCE(a.id,fingerprint.finalization_id) AS snapshot_authority_id,
                  COALESCE(a.vault_object_ref_token_hash,fingerprint.finalization_hash)
                      AS vault_object_ref_token_hash,
                  COALESCE(a.sha256,fingerprint.observation_hash) AS snapshot_sha256,
                  COALESCE(a.stored_byte_count,reconciliation.observed_artifact_byte_count)
                      AS snapshot_byte_count
             FROM latest
             LEFT JOIN capability_raw_witness_artifacts a
               ON a.id=latest.raw_witness_artifact_id AND a.receipt_id=latest.id
             LEFT JOIN verification_action_capability_receipt_finalizations fingerprint
               ON fingerprint.capability_execution_receipt_id=latest.id
              AND fingerprint.witness_completeness='complete_fingerprint_v1'
             LEFT JOIN capability_execution_reconciliations reconciliation
               ON reconciliation.id=latest.current_semantic_reconciliation_id
              AND reconciliation.receipt_id=latest.id
              AND reconciliation.reconciliation_state='consistent'
              AND reconciliation.sealed_at IS NOT NULL
            ORDER BY latest.denominator_id,latest.capability,latest.id"#,
    )
    .bind(&graph_ids)
    .fetch_all(&mut **tx)
    .await?;
    let receipt_ids = receipts
        .iter()
        .map(|receipt| receipt.id)
        .collect::<Vec<_>>();
    let expected_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM coverage_denominator_items WHERE denominator_id=ANY($1)",
    )
    .bind(&graph_ids)
    .fetch_one(&mut **tx)
    .await?;
    let covered_count: i64 = if receipt_ids.is_empty() {
        0
    } else {
        sqlx::query_scalar(
            r#"SELECT count(*)::bigint FROM coverage_denominator_items item
                WHERE item.denominator_id=ANY($1) AND 1=(
                    SELECT count(*) FROM capability_execution_receipt_inputs input
                     WHERE input.denominator_item_id=item.id
                       AND input.receipt_id=ANY($2) AND input.sealed_at IS NOT NULL
                       AND input.coverage_extent='complete' AND input.landing_state='committed'
                )"#,
        )
        .bind(&graph_ids)
        .bind(&receipt_ids)
        .fetch_one(&mut **tx)
        .await?
    };
    let semantic_status = if expected_count == covered_count
        && (expected_count == 0 || !receipts.is_empty())
        && receipts.iter().all(|receipt| {
            receipt.finalized_at.is_some()
                && receipt.reconciliation_state == "consistent"
                && receipt.current_semantic_reconciliation_id.is_some()
                && receipt.current_semantic_reconciliation_hash.is_some()
        }) {
        "consistent".to_string()
    } else if receipts
        .iter()
        .any(|receipt| receipt.reconciliation_state == "orphaned")
    {
        "orphaned".to_string()
    } else if receipts
        .iter()
        .any(|receipt| receipt.reconciliation_state == "superseded")
    {
        "superseded".to_string()
    } else {
        "pending".to_string()
    };
    let semantic_hash = sha256_json(&serde_json::json!({
        "graph_hash": graph_hash,
        "expected_count": expected_count,
        "covered_count": covered_count,
        "receipts": receipts.iter().map(|receipt| serde_json::json!({
            "receipt_id": receipt.id,
            "capability": receipt.capability,
            "reconciliation_state": receipt.reconciliation_state,
            "semantic_authority_version": receipt.current_semantic_authority_version,
            "semantic_hash": receipt.current_semantic_reconciliation_hash,
        })).collect::<Vec<_>>(),
    }))?;
    let stable_set_request_id = Uuid::new_v5(
        &request.stable_consumer_request_id,
        format!("authority-set:{}", family.as_str()).as_bytes(),
    );
    let mut set_members = Vec::new();
    for receipt in &receipts {
        if let Some(member) = attest_receipt(
            tx,
            stable_set_request_id,
            request.consumer_kind.as_str(),
            receipt,
        )
        .await?
        {
            set_members.push(member);
        }
    }
    set_members.sort_by(|left, right| {
        (left.denominator_id, left.receipt_id).cmp(&(right.denominator_id, right.receipt_id))
    });
    let freshness_hash = sha256_json(&serde_json::json!(set_members
        .iter()
        .map(|member| (&member.receipt_id, &member.freshness_attestation_hash))
        .collect::<Vec<_>>()))?;
    let authority_set_id = seal_authority_set(
        tx,
        request,
        family,
        &root,
        &graph_hash,
        &semantic_hash,
        &freshness_hash,
        &set_members,
    )
    .await?;

    let mut policies = BTreeMap::new();
    let mut censuses = Vec::new();
    let mut epoch_mismatch = false;
    for receipt in &receipts {
        policies
            .entry(receipt.temporal_validity_policy_id)
            .or_insert(load_policy(tx, receipt.temporal_validity_policy_id).await?);
        let Some(census_id) = receipt.temporal_census_id else {
            continue;
        };
        let census = sqlx::query_as::<_, TemporalCensusRow>(
            r#"SELECT c.id,c.observation_window_started_at,c.observation_window_completed_at,
                      c.effective_valid_until,c.target_state_epoch_set_hash,
                      p.max_cross_observation_skew_ms
                 FROM capability_execution_temporal_censuses c
                 JOIN evidence_temporal_validity_policies p
                   ON p.id=c.temporal_validity_policy_id
                WHERE c.id=$1 AND c.sealed_at IS NOT NULL FOR SHARE"#,
        )
        .bind(census_id)
        .fetch_optional(&mut **tx)
        .await?;
        if let Some(census) = census {
            let mismatch: bool = sqlx::query_scalar(
                r#"SELECT EXISTS(
                      SELECT 1 FROM capability_execution_temporal_census_members member
                      LEFT JOIN tool_truth_target_state_epoch_heads head
                        ON head.operation_id=member.target_state_operation_id
                       AND head.organization_id=member.target_state_organization_id
                       AND head.target_scope_identity_hash=member.target_scope_identity_hash
                     WHERE member.census_id=$1 AND (
                         head.current_event_id IS NULL
                         OR head.current_event_id<>member.target_state_epoch_event_id
                         OR head.current_epoch<>member.target_state_epoch
                     )
                   )"#,
            )
            .bind(census.id)
            .fetch_one(&mut **tx)
            .await?;
            epoch_mismatch |= mismatch;
            censuses.push(census);
        }
    }
    let temporal_policies = policies.into_values().collect::<Vec<_>>();
    let temporal_policy_set_hash = sha256_json(&serde_json::json!({
        "policies": temporal_policies
            .iter()
            .map(|policy| (&policy.id, &policy.policy_hash, &policy.member_set_hash))
            .collect::<Vec<_>>(),
        "receipt_bindings": receipts
            .iter()
            .map(|receipt| (&receipt.id, &receipt.temporal_validity_policy_hash))
            .collect::<Vec<_>>()
    }))?;
    let target_state_epoch_set_hash = sha256_json(&serde_json::json!(censuses
        .iter()
        .map(|census| (&census.id, &census.target_state_epoch_set_hash))
        .collect::<Vec<_>>()))?;
    let window_started = censuses
        .iter()
        .map(|census| census.observation_window_started_at)
        .min();
    let window_completed = censuses
        .iter()
        .map(|census| census.observation_window_completed_at)
        .max();
    let effective_valid_until = censuses
        .iter()
        .map(|census| census.effective_valid_until)
        .min();
    let max_skew_ms = censuses
        .iter()
        .map(|census| census.max_cross_observation_skew_ms)
        .min();
    let temporal_status = if epoch_mismatch {
        TemporalValidityStatus::MixedEpoch
    } else if window_started
        .zip(window_completed)
        .zip(max_skew_ms)
        .is_some_and(|((start, end), max_skew)| (end - start).num_milliseconds() > max_skew)
    {
        TemporalValidityStatus::SkewExceeded
    } else if effective_valid_until.is_none()
        || effective_valid_until.is_some_and(|valid_until| valid_until <= transaction_now)
    {
        TemporalValidityStatus::Expired
    } else {
        TemporalValidityStatus::Fresh
    };
    let member_status = temporal_member_status(&semantic_status, temporal_status);
    Ok(RootState {
        root,
        graph_hash,
        semantic_hash,
        freshness_hash,
        authority_set_id,
        semantic_status,
        temporal_status,
        member_status,
        temporal_policy_set_hash,
        target_state_epoch_set_hash,
        observation_window_started_at: window_started,
        observation_window_completed_at: window_completed,
        effective_valid_until,
        temporal_policies,
        receipt_ids,
        revalidation_obligation_ids: Vec::new(),
    })
}

async fn record_root_obligations(
    tx: &mut Transaction<'static, Postgres>,
    request: &CheckToolTruthAuthorityBundle,
    root: &mut RootState,
) -> Result<()> {
    // A stage fork may consume an immutable predecessor root through its exact
    // operation_stage_fork_inputs manifest.  The target analysis records that
    // root as a typed residual, but must not append revalidation obligations
    // into the adopted source operation.
    if root.root.operation_id != request.operation_id {
        return Ok(());
    }
    if root.member_status == ToolTruthAuthorityBundleMemberStatusV1::ConsistentFresh {
        return Ok(());
    }
    let reason_code = match root.member_status {
        ToolTruthAuthorityBundleMemberStatusV1::ConsistentFresh => return Ok(()),
        ToolTruthAuthorityBundleMemberStatusV1::SemanticInvalid => "semantic_invalid",
        ToolTruthAuthorityBundleMemberStatusV1::Expired => "temporal_expired",
        ToolTruthAuthorityBundleMemberStatusV1::MixedEpoch => "target_state_epoch_mismatch",
        ToolTruthAuthorityBundleMemberStatusV1::SkewExceeded => "observation_skew_exceeded",
    };
    if root.receipt_ids.is_empty() {
        return Ok(());
    }
    let inputs = sqlx::query_as::<_, (Uuid, Uuid, String, String, Uuid)>(
        r#"SELECT input.receipt_id,input.id,input.input_key,item.technique,
                  receipt.temporal_validity_policy_id
             FROM capability_execution_receipt_inputs input
             JOIN capability_execution_receipts receipt ON receipt.id=input.receipt_id
             JOIN coverage_denominator_items item ON item.id=input.denominator_item_id
            WHERE input.receipt_id=ANY($1) AND input.sealed_at IS NOT NULL
              AND receipt.finalized_at IS NOT NULL
            ORDER BY input.input_key,input.receipt_id"#,
    )
    .bind(&root.receipt_ids)
    .fetch_all(&mut **tx)
    .await?;
    for (receipt_id, input_id, input_key, fact_class, policy_id) in inputs {
        let obligation = tool_truth_revalidation::record_obligation_on(
            tx,
            &tool_truth_revalidation::RecordRevalidationObligation {
                operation_id: request.operation_id,
                organization_id: request.organization_id,
                source_receipt_id: receipt_id,
                source_receipt_input_id: input_id,
                source_input_key: input_key,
                fact_class,
                temporal_policy_id: policy_id,
                reason_code: reason_code.to_string(),
                risk_tier: "T1".to_string(),
                mandatory_axis: true,
                consumer_kind: request.consumer_kind.obligation_consumer_kind().to_string(),
                consumer_key: request.stable_consumer_request_id.to_string(),
            },
        )
        .await?;
        root.revalidation_obligation_ids.push(obligation.id);
    }
    root.revalidation_obligation_ids.sort_unstable();
    root.revalidation_obligation_ids.dedup();
    Ok(())
}

async fn seal_bundle(
    tx: &mut Transaction<'static, Postgres>,
    request: &CheckToolTruthAuthorityBundle,
    roots: &[RootState],
) -> Result<BundleHeaderRow> {
    let scope = roots
        .first()
        .ok_or_else(|| fail(BUNDLE_ROOT_CENSUS_INCOMPLETE))?;
    let fork_scope = sqlx::query_as::<_, (Uuid, Uuid, String)>(
        r#"SELECT scope.id,scope.project_scope_id,scope.project_path_at_freeze
             FROM operation_stage_forks fork
             JOIN operation_org_scope_snapshots scope
               ON scope.id=fork.target_scope_snapshot_id
              AND scope.operation_id=fork.operation_id
             JOIN operation_org_scope_units unit
               ON unit.snapshot_id=scope.id AND unit.organization_id=$2
            WHERE fork.operation_id=$1 AND scope.sealed_at IS NOT NULL
            FOR SHARE OF fork,scope,unit"#,
    )
    .bind(request.operation_id)
    .bind(request.organization_id)
    .fetch_optional(&mut **tx)
    .await?;
    let is_stage_fork = fork_scope.is_some();
    let (bundle_scope_snapshot_id, bundle_project_scope_id, bundle_project_path) = fork_scope
        .unwrap_or_else(|| {
            (
                scope.root.scope_snapshot_id,
                scope.root.project_scope_id,
                scope.root.project_path_at_freeze.clone(),
            )
        });
    // `load_roots` already proves every cross-operation root against its exact
    // immutable fork input and source handoff. A formal fork therefore
    // legitimately combines those source snapshots with roots produced under
    // the target snapshot. Only target-operation roots must carry the target
    // snapshot itself.
    if roots.iter().any(|root| {
        !root_matches_bundle_scope(
            &root.root,
            request.operation_id,
            bundle_scope_snapshot_id,
            bundle_project_scope_id,
            &bundle_project_path,
            is_stage_fork,
        )
    }) {
        return Err(fail(AUTHORITY_STALE));
    }
    let member_hashes = roots
        .iter()
        .map(|root| {
            sha256_json(&serde_json::json!({
                "root_family": ToolTruthRootFamilyV1::from_stage_kind(&root.root.stage_kind)
                    .map_err(|_| fail(CONTRACT_INVALID))?.as_str(),
                "root_denominator_id": root.root.denominator_id,
                "root_denominator_hash": root.root.denominator_hash,
                "authority_set_seal_id": root.authority_set_id,
                "authority_set_semantic_hash": root.semantic_hash,
                "authority_set_graph_hash": root.graph_hash,
                "authority_set_freshness_hash": root.freshness_hash,
                "temporal_validity_policy_set_hash": root.temporal_policy_set_hash,
                "target_state_epoch_set_hash": root.target_state_epoch_set_hash,
                "observation_window_started_at": root.observation_window_started_at,
                "observation_window_completed_at": root.observation_window_completed_at,
                "effective_valid_until": root.effective_valid_until,
                "semantic_status": root.semantic_status,
                "temporal_validity_status": temporal_status_wire(root.temporal_status),
                "member_status": root.member_status.as_str(),
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    if let Some(existing_id) = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM tool_truth_authority_bundle_seals
            WHERE operation_id=$1 AND organization_id=$2 AND consumer_kind=$3
              AND stable_consumer_request_id=$4 FOR SHARE"#,
    )
    .bind(request.operation_id)
    .bind(request.organization_id)
    .bind(request.consumer_kind.as_str())
    .bind(request.stable_consumer_request_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        let stored_hashes = sqlx::query_scalar::<_, String>(
            "SELECT member_hash FROM tool_truth_authority_bundle_members WHERE bundle_seal_id=$1 ORDER BY ordinal",
        )
        .bind(existing_id)
        .fetch_all(&mut **tx)
        .await?;
        if stored_hashes != member_hashes {
            return Err(fail(BUNDLE_DRIFT));
        }
        return sqlx::query_as::<_, BundleHeaderRow>(
            r#"SELECT id,relevant_root_set_hash,member_set_hash,
                      semantic_authority_bundle_hash,freshness_attestation_bundle_hash,
                      temporal_validity_bundle_hash,temporal_validity_policy_set_hash,
                      target_state_epoch_set_hash,observation_window_started_at,
                      observation_window_completed_at,effective_valid_until
                 FROM tool_truth_authority_bundle_seals
                WHERE id=$1 AND sealed_at IS NOT NULL"#,
        )
        .bind(existing_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| fail(AUTHORITY_STALE));
    }

    let bundle_id = Uuid::new_v5(
        &request.stable_consumer_request_id,
        format!(
            "authority-bundle:{}:{}:{}",
            request.operation_id,
            request.organization_id,
            request.consumer_kind.as_str()
        )
        .as_bytes(),
    );
    sqlx::query(
        r#"INSERT INTO tool_truth_authority_bundle_seals(
               id,operation_id,project_scope_id,project_path_at_freeze,scope_snapshot_id,
               organization_id,consumer_kind,stable_consumer_request_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(bundle_id)
    .bind(request.operation_id)
    .bind(bundle_project_scope_id)
    .bind(&bundle_project_path)
    .bind(bundle_scope_snapshot_id)
    .bind(request.organization_id)
    .bind(request.consumer_kind.as_str())
    .bind(request.stable_consumer_request_id)
    .execute(&mut **tx)
    .await?;
    for (ordinal, (root, member_hash)) in roots.iter().zip(member_hashes.iter()).enumerate() {
        let family = ToolTruthRootFamilyV1::from_stage_kind(&root.root.stage_kind)
            .map_err(|_| fail(CONTRACT_INVALID))?;
        sqlx::query(
            r#"INSERT INTO tool_truth_authority_bundle_members(
                   id,bundle_seal_id,operation_id,organization_id,ordinal,root_family,
                   root_execution_authority_id,root_denominator_id,root_denominator_hash,
                   authority_set_seal_id,authority_set_semantic_hash,
                   authority_set_graph_hash,authority_set_freshness_hash,
                   temporal_validity_policy_set_hash,target_state_epoch_set_hash,
                   observation_window_started_at,observation_window_completed_at,
                   effective_valid_until,semantic_status,temporal_validity_status,
                   member_status,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,
                        $16,$17,$18,$19,$20,$21,$22)"#,
        )
        .bind(Uuid::new_v5(&bundle_id, member_hash.as_bytes()))
        .bind(bundle_id)
        .bind(request.operation_id)
        .bind(request.organization_id)
        .bind(i32::try_from(ordinal).map_err(|_| fail(CONTRACT_INVALID))?)
        .bind(family.as_str())
        .bind(root.root.execution_authority_id)
        .bind(root.root.denominator_id)
        .bind(&root.root.denominator_hash)
        .bind(root.authority_set_id)
        .bind(&root.semantic_hash)
        .bind(&root.graph_hash)
        .bind(&root.freshness_hash)
        .bind(&root.temporal_policy_set_hash)
        .bind(&root.target_state_epoch_set_hash)
        .bind(root.observation_window_started_at)
        .bind(root.observation_window_completed_at)
        .bind(root.effective_valid_until)
        .bind(&root.semantic_status)
        .bind(temporal_status_wire(root.temporal_status))
        .bind(root.member_status.as_str())
        .bind(member_hash)
        .execute(&mut **tx)
        .await?;
    }
    sqlx::query(
        "UPDATE tool_truth_authority_bundle_seals SET sealed_at=statement_timestamp() WHERE id=$1",
    )
    .bind(bundle_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query_as::<_, BundleHeaderRow>(
        r#"SELECT id,relevant_root_set_hash,member_set_hash,
                  semantic_authority_bundle_hash,freshness_attestation_bundle_hash,
                  temporal_validity_bundle_hash,temporal_validity_policy_set_hash,
                  target_state_epoch_set_hash,observation_window_started_at,
                  observation_window_completed_at,effective_valid_until
             FROM tool_truth_authority_bundle_seals WHERE id=$1 AND sealed_at IS NOT NULL"#,
    )
    .bind(bundle_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| fail(MANIFEST_DRIFT))
}

async fn derive_and_seal_bundle(
    tx: &mut Transaction<'static, Postgres>,
    request: &CheckToolTruthAuthorityBundle,
) -> Result<CheckedBundleState> {
    if request.stable_consumer_request_id.is_nil()
        || request.operation_id.is_nil()
        || request.organization_id.is_nil()
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let transaction_now: DateTime<Utc> = sqlx::query_scalar("SELECT statement_timestamp()")
        .fetch_one(&mut **tx)
        .await?;
    let roots = load_roots(tx, request).await?;
    let mut states = Vec::with_capacity(roots.len());
    for (family, root) in ToolTruthRootFamilyV1::EXECUTION_RECEIPT_ROOTS
        .into_iter()
        .zip(roots)
    {
        states.push(derive_root_state(tx, request, family, root, transaction_now).await?);
    }
    // Each temporal policy belongs to one execution authority/root. Its
    // cross-observation skew bounds the receipts inside that root and is
    // already enforced by `derive_root_state`. EAS, Enumeration and Vuln are
    // sequential stages, so comparing their aggregate wall-clock window to a
    // single root's skew budget would deterministically make every normal
    // multi-stage bundle stale. The aggregate window remains sealed below as
    // descriptive authority; freshness is the exact conjunction of the
    // independently validated root states and their target-state epochs.
    for state in &mut states {
        record_root_obligations(tx, request, state).await?;
    }
    let header = seal_bundle(tx, request, &states).await?;
    let roots = states
        .into_iter()
        .map(|state| CheckedToolTruthAuthorityRoot {
            root_family: ToolTruthRootFamilyV1::from_stage_kind(&state.root.stage_kind)
                .expect("server selected a closed root family"),
            root_denominator_id: state.root.denominator_id,
            root_denominator_hash: state.root.denominator_hash,
            authority_set_seal_id: state.authority_set_id,
            authority_set_graph_hash: state.graph_hash,
            authority_set_semantic_hash: state.semantic_hash,
            authority_set_freshness_hash: state.freshness_hash,
            temporal_validity_policy_set_hash: state.temporal_policy_set_hash,
            target_state_epoch_set_hash: state.target_state_epoch_set_hash,
            observation_window_started_at: state.observation_window_started_at,
            observation_window_completed_at: state.observation_window_completed_at,
            effective_valid_until: state.effective_valid_until,
            semantic_status: state.semantic_status,
            temporal_validity_status: state.temporal_status,
            member_status: state.member_status,
            temporal_policies: state.temporal_policies,
            revalidation_obligation_ids: state.revalidation_obligation_ids,
        })
        .collect();
    Ok(CheckedBundleState {
        bundle_seal_id: header.id,
        operation_id: request.operation_id,
        organization_id: request.organization_id,
        relevant_root_set_hash: header.relevant_root_set_hash,
        member_set_hash: header.member_set_hash,
        semantic_authority_bundle_hash: header.semantic_authority_bundle_hash,
        freshness_attestation_bundle_hash: header.freshness_attestation_bundle_hash,
        temporal_validity_bundle_hash: header.temporal_validity_bundle_hash,
        temporal_validity_policy_set_hash: header.temporal_validity_policy_set_hash,
        target_state_epoch_set_hash: header.target_state_epoch_set_hash,
        observation_window_started_at: header.observation_window_started_at,
        observation_window_completed_at: header.observation_window_completed_at,
        effective_valid_until: header.effective_valid_until,
        roots,
    })
}

pub async fn with_checked_tool_truth_authority_bundle<T, F>(
    pool: &PgPool,
    request: &CheckToolTruthAuthorityBundle,
    callback: F,
) -> Result<T>
where
    T: Send + 'static,
    F: for<'guard> FnOnce(
            &'guard mut Transaction<'static, Postgres>,
            &'guard CheckedToolTruthAuthorityBundle<'guard>,
        ) -> ToolTruthAuthorityBundleFuture<'guard, T>
        + Send,
{
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;
    let state = derive_and_seal_bundle(&mut tx, request).await?;
    let checked = CheckedToolTruthAuthorityBundle {
        state,
        _invariant: PhantomData,
    };
    let output = callback(&mut tx, &checked).await?;
    tx.commit().await?;
    Ok(output)
}

pub async fn with_all_fresh_tool_truth_authority_bundle<T, F>(
    pool: &PgPool,
    request: &CheckToolTruthAuthorityBundle,
    callback: F,
) -> Result<T>
where
    T: Send + 'static,
    F: for<'guard> FnOnce(
            &'guard mut Transaction<'static, Postgres>,
            &'guard AllFreshToolTruthAuthorityBundle<'guard>,
        ) -> ToolTruthAuthorityBundleFuture<'guard, T>
        + Send,
{
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;
    let state = derive_and_seal_bundle(&mut tx, request).await?;
    let checked = CheckedToolTruthAuthorityBundle {
        state,
        _invariant: PhantomData,
    };
    let all_fresh = checked.as_all_fresh()?;
    let output = callback(&mut tx, &all_fresh).await?;
    tx.commit().await?;
    Ok(output)
}

#[cfg(test)]
mod compile_tests {
    use super::*;

    fn guards_have_no_owned_escape_or_public_constructor(
        checked: &CheckedToolTruthAuthorityBundle<'_>,
    ) -> (Uuid, usize) {
        (checked.bundle_seal_id(), checked.roots().len())
    }

    #[test]
    fn checked_bundle_exposes_only_borrowed_census_views() {
        let _ = guards_have_no_owned_escape_or_public_constructor;
        assert_eq!(ToolTruthRootFamilyV1::EXECUTION_RECEIPT_ROOTS.len(), 3);
    }

    #[test]
    fn formal_fork_accepts_adopted_source_snapshot_but_not_target_drift() {
        let target_operation_id = Uuid::new_v4();
        let target_scope_snapshot_id = Uuid::new_v4();
        let project_scope_id = Uuid::new_v4();
        let mut root = RootRow {
            operation_id: Uuid::new_v4(),
            denominator_id: Uuid::new_v4(),
            execution_authority_id: Uuid::new_v4(),
            denominator_hash: format!("sha256:{}", "1".repeat(64)),
            stage_kind: "enumeration".to_string(),
            project_scope_id,
            project_path_at_freeze: "/tmp/project".to_string(),
            scope_snapshot_id: Uuid::new_v4(),
        };
        assert!(root_matches_bundle_scope(
            &root,
            target_operation_id,
            target_scope_snapshot_id,
            project_scope_id,
            "/tmp/project",
            true,
        ));

        root.operation_id = target_operation_id;
        assert!(!root_matches_bundle_scope(
            &root,
            target_operation_id,
            target_scope_snapshot_id,
            project_scope_id,
            "/tmp/project",
            true,
        ));
        root.scope_snapshot_id = target_scope_snapshot_id;
        assert!(root_matches_bundle_scope(
            &root,
            target_operation_id,
            target_scope_snapshot_id,
            project_scope_id,
            "/tmp/project",
            true,
        ));
    }
}
