use golish_core::hypothesis_semantic_key::CandidateMutationEpistemicState;
use golish_db::repo::capability_execution_receipts::{
    CheckedToolTruthAuthorityBundle, ToolTruthAuthorityBundleMemberStatusV1,
};
use golish_pentest_domain::tool_truth::{
    EvidenceTemporalValidityPolicyV1, TemporalValidityStatus, ToolTruthRootFamilyV1,
};
use uuid::Uuid;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum CandidateMutationError {
    #[error("HYPOTHESIS_CANDIDATE_TERMINAL_STATE_FORBIDDEN")]
    TerminalStateForbidden,
    #[error("HYPOTHESIS_INVALID_STATE_SERVER_ONLY")]
    InvalidStateServerOnly,
    #[error("HYPOTHESIS_CANDIDATE_STATE_UNKNOWN: {0}")]
    Unknown(String),
}

pub fn candidate_mutation_state(
    value: &str,
) -> Result<CandidateMutationEpistemicState, CandidateMutationError> {
    match value {
        "proposed" => Ok(CandidateMutationEpistemicState::Proposed),
        "supported" => Ok(CandidateMutationEpistemicState::Supported),
        "contested" => Ok(CandidateMutationEpistemicState::Contested),
        "inconclusive" => Ok(CandidateMutationEpistemicState::Inconclusive),
        "verified" | "refuted" => Err(CandidateMutationError::TerminalStateForbidden),
        "invalid" => Err(CandidateMutationError::InvalidStateServerOnly),
        other => Err(CandidateMutationError::Unknown(other.into())),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateAuthoritySnapshotDispositionV1 {
    SealedReady,
    BlockedSemanticInvalid,
    BlockedExpired,
    BlockedMixedEpoch,
    BlockedSkewExceeded,
    BlockedRootCensusIncomplete,
}

#[derive(Debug, Clone)]
pub struct CandidateAuthorityRootSnapshotV1 {
    root_family: ToolTruthRootFamilyV1,
    root_denominator_id: Uuid,
    root_denominator_hash: String,
    authority_set_seal_id: Uuid,
    authority_set_graph_hash: String,
    authority_set_semantic_hash: String,
    authority_set_freshness_hash: String,
    temporal_validity_policy_set_hash: String,
    target_state_epoch_set_hash: String,
    semantic_status: String,
    temporal_validity_status: TemporalValidityStatus,
    member_status: ToolTruthAuthorityBundleMemberStatusV1,
    temporal_policies: Vec<EvidenceTemporalValidityPolicyV1>,
    observation_window_started_at: Option<chrono::DateTime<chrono::Utc>>,
    observation_window_completed_at: Option<chrono::DateTime<chrono::Utc>>,
    effective_valid_until: Option<chrono::DateTime<chrono::Utc>>,
    revalidation_obligation_ids: Vec<Uuid>,
}

#[derive(Debug, Clone)]
pub struct CandidateAuthorityBundleSnapshotV1 {
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
    observation_window_started_at: Option<chrono::DateTime<chrono::Utc>>,
    observation_window_completed_at: Option<chrono::DateTime<chrono::Utc>>,
    effective_valid_until: Option<chrono::DateTime<chrono::Utc>>,
    roots: Vec<CandidateAuthorityRootSnapshotV1>,
    disposition: CandidateAuthoritySnapshotDispositionV1,
}

impl CandidateAuthorityBundleSnapshotV1 {
    pub const fn bundle_seal_id(&self) -> Uuid {
        self.bundle_seal_id
    }
    pub fn roots(&self) -> &[CandidateAuthorityRootSnapshotV1] {
        &self.roots
    }
    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }
    pub const fn organization_id(&self) -> Uuid {
        self.organization_id
    }
    pub fn relevant_root_set_hash(&self) -> &str {
        &self.relevant_root_set_hash
    }
    pub fn member_set_hash(&self) -> &str {
        &self.member_set_hash
    }
    pub fn semantic_authority_bundle_hash(&self) -> &str {
        &self.semantic_authority_bundle_hash
    }
    pub fn freshness_attestation_bundle_hash(&self) -> &str {
        &self.freshness_attestation_bundle_hash
    }
    pub fn temporal_validity_bundle_hash(&self) -> &str {
        &self.temporal_validity_bundle_hash
    }
    pub fn temporal_validity_policy_set_hash(&self) -> &str {
        &self.temporal_validity_policy_set_hash
    }
    pub fn target_state_epoch_set_hash(&self) -> &str {
        &self.target_state_epoch_set_hash
    }
    pub const fn observation_window(
        &self,
    ) -> (
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) {
        (
            self.observation_window_started_at,
            self.observation_window_completed_at,
        )
    }
    pub const fn effective_valid_until(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.effective_valid_until
    }
    pub const fn disposition(&self) -> CandidateAuthoritySnapshotDispositionV1 {
        self.disposition
    }
}

impl CandidateAuthorityRootSnapshotV1 {
    pub const fn root_family(&self) -> ToolTruthRootFamilyV1 {
        self.root_family
    }
    pub const fn member_status(&self) -> ToolTruthAuthorityBundleMemberStatusV1 {
        self.member_status
    }
    pub const fn root_denominator_id(&self) -> Uuid {
        self.root_denominator_id
    }
    pub fn root_denominator_hash(&self) -> &str {
        &self.root_denominator_hash
    }
    pub const fn authority_set_seal_id(&self) -> Uuid {
        self.authority_set_seal_id
    }
    pub fn authority_set_graph_hash(&self) -> &str {
        &self.authority_set_graph_hash
    }
    pub fn authority_set_semantic_hash(&self) -> &str {
        &self.authority_set_semantic_hash
    }
    pub fn authority_set_freshness_hash(&self) -> &str {
        &self.authority_set_freshness_hash
    }
    pub fn temporal_validity_policy_set_hash(&self) -> &str {
        &self.temporal_validity_policy_set_hash
    }
    pub fn target_state_epoch_set_hash(&self) -> &str {
        &self.target_state_epoch_set_hash
    }
    pub fn semantic_status(&self) -> &str {
        &self.semantic_status
    }
    pub const fn temporal_validity_status(&self) -> TemporalValidityStatus {
        self.temporal_validity_status
    }
    pub fn temporal_policies(&self) -> &[EvidenceTemporalValidityPolicyV1] {
        &self.temporal_policies
    }
    pub const fn observation_window(
        &self,
    ) -> (
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) {
        (
            self.observation_window_started_at,
            self.observation_window_completed_at,
        )
    }
    pub const fn effective_valid_until(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.effective_valid_until
    }
    pub fn revalidation_obligation_ids(&self) -> &[Uuid] {
        &self.revalidation_obligation_ids
    }
}

/// Exact-copy adapter.  Its only authority-bearing input is Plan A's opaque,
/// callback-scoped checked bundle; there is no constructor accepting caller
/// time, epoch, status, roots, policy, or hashes.
pub fn freeze_candidate_authority_bundle(
    bundle: &CheckedToolTruthAuthorityBundle<'_>,
) -> CandidateAuthorityBundleSnapshotV1 {
    let (observation_window_started_at, observation_window_completed_at) =
        bundle.observation_window();
    CandidateAuthorityBundleSnapshotV1 {
        bundle_seal_id: bundle.bundle_seal_id(),
        operation_id: bundle.operation_id(),
        organization_id: bundle.organization_id(),
        relevant_root_set_hash: bundle.relevant_root_set_hash().to_owned(),
        member_set_hash: bundle.member_set_hash().to_owned(),
        semantic_authority_bundle_hash: bundle.semantic_authority_bundle_hash().to_owned(),
        freshness_attestation_bundle_hash: bundle.freshness_attestation_bundle_hash().to_owned(),
        temporal_validity_bundle_hash: bundle.temporal_validity_bundle_hash().to_owned(),
        temporal_validity_policy_set_hash: bundle.temporal_validity_policy_set_hash().to_owned(),
        target_state_epoch_set_hash: bundle.target_state_epoch_set_hash().to_owned(),
        roots: bundle
            .roots()
            .iter()
            .map(|root| CandidateAuthorityRootSnapshotV1 {
                root_family: root.root_family,
                root_denominator_id: root.root_denominator_id,
                root_denominator_hash: root.root_denominator_hash.clone(),
                authority_set_seal_id: root.authority_set_seal_id,
                authority_set_graph_hash: root.authority_set_graph_hash.clone(),
                authority_set_semantic_hash: root.authority_set_semantic_hash.clone(),
                authority_set_freshness_hash: root.authority_set_freshness_hash.clone(),
                temporal_validity_policy_set_hash: root.temporal_validity_policy_set_hash.clone(),
                target_state_epoch_set_hash: root.target_state_epoch_set_hash.clone(),
                semantic_status: root.semantic_status.clone(),
                temporal_validity_status: root.temporal_validity_status,
                member_status: root.member_status,
                temporal_policies: root.temporal_policies.clone(),
                observation_window_started_at: root.observation_window_started_at,
                observation_window_completed_at: root.observation_window_completed_at,
                effective_valid_until: root.effective_valid_until,
                revalidation_obligation_ids: root.revalidation_obligation_ids.clone(),
            })
            .collect(),
        observation_window_started_at,
        observation_window_completed_at,
        effective_valid_until: bundle.effective_valid_until(),
        disposition: candidate_authority_disposition(bundle.roots()),
    }
}

fn candidate_authority_disposition(
    roots: &[golish_db::repo::capability_execution_receipts::CheckedToolTruthAuthorityRoot],
) -> CandidateAuthoritySnapshotDispositionV1 {
    let root_families = roots
        .iter()
        .map(|root| root.root_family)
        .collect::<std::collections::BTreeSet<_>>();
    if roots.len() != ToolTruthRootFamilyV1::ALL.len()
        || root_families.len() != ToolTruthRootFamilyV1::ALL.len()
        || !ToolTruthRootFamilyV1::ALL
            .iter()
            .all(|family| root_families.contains(family))
    {
        return CandidateAuthoritySnapshotDispositionV1::BlockedRootCensusIncomplete;
    }
    let statuses = roots
        .iter()
        .map(|root| root.member_status)
        .collect::<Vec<_>>();
    if statuses.contains(&ToolTruthAuthorityBundleMemberStatusV1::SemanticInvalid) {
        return CandidateAuthoritySnapshotDispositionV1::BlockedSemanticInvalid;
    }
    if statuses.contains(&ToolTruthAuthorityBundleMemberStatusV1::MixedEpoch) {
        return CandidateAuthoritySnapshotDispositionV1::BlockedMixedEpoch;
    }
    if statuses.contains(&ToolTruthAuthorityBundleMemberStatusV1::SkewExceeded) {
        return CandidateAuthoritySnapshotDispositionV1::BlockedSkewExceeded;
    }
    if statuses.contains(&ToolTruthAuthorityBundleMemberStatusV1::Expired) {
        return CandidateAuthoritySnapshotDispositionV1::BlockedExpired;
    }
    CandidateAuthoritySnapshotDispositionV1::SealedReady
}
