use golish_memory_domain::source_ref::{CanonicalRowId, StoredCanonicalRowId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AllFreshToolTruthAuthorityBundleRefV1 {
    pub bundle_id: Uuid,
    pub bundle_hash: [u8; 32],
    pub relevant_root_count: u64,
    pub relevant_root_set_hash: [u8; 32],
    pub relevant_member_count: u64,
    pub relevant_member_set_hash: [u8; 32],
    pub semantic_authority_hash: [u8; 32],
    pub freshness_authority_hash: [u8; 32],
    pub temporal_validity_hash: [u8; 32],
    pub epoch_hash: [u8; 32],
    pub observation_window_hash: [u8; 32],
    pub effective_validity_hash: [u8; 32],
    pub effective_valid_until: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "terminal_kind", rename_all = "snake_case")]
pub enum WaveTerminalReceiptRefV1 {
    Consolidation {
        receipt_id: Uuid,
        receipt_hash: [u8; 32],
    },
    FixedPoint {
        receipt_id: Uuid,
        receipt_hash: [u8; 32],
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportToolTruthAuthoritySetRefV1 {
    pub authority_set_id: Uuid,
    pub authority_member_count: u64,
    pub authority_set_hash: [u8; 32],
    pub earliest_effective_valid_until: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionAdjudicationOutcomeV1 {
    Nonterminal,
    Verified,
    Refuted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RevisionAdjudicationAuthorityMemberV1 {
    pub organization_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub adjudication_tool_truth_authority: AllFreshToolTruthAuthorityBundleRefV1,
    pub generation_seal_id: Uuid,
    pub generation_seal_hash: [u8; 32],
    pub verification_plan_seal_id: Uuid,
    pub verification_plan_seal_hash: [u8; 32],
    pub proof_path_set_hash: [u8; 32],
    pub claim_component_set_hash: [u8; 32],
    pub revision_adjudication_id: Uuid,
    pub revision_adjudication_hash: [u8; 32],
    pub adjudication_outcome: RevisionAdjudicationOutcomeV1,
    pub revision_terminal_decision_id: Option<Uuid>,
    pub revision_terminal_decision_hash: Option<[u8; 32]>,
    pub latest_objective_outcome_member_count: u64,
    pub latest_objective_outcome_set_hash: [u8; 32],
    pub wave_terminal: WaveTerminalReceiptRefV1,
    pub final_wave_coverage_receipt_id: Uuid,
    pub final_wave_coverage_receipt_hash: [u8; 32],
    pub coverage_membership_hash: [u8; 32],
    pub residual_membership_hash: [u8; 32],
    pub effective_valid_until: chrono::DateTime<chrono::Utc>,
    pub member_hash: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RevisionAdjudicationAuthoritySetRefV1 {
    pub authority_set_id: Uuid,
    pub authority_member_count: u64,
    pub authority_set_hash: [u8; 32],
    pub coverage_membership_hash: [u8; 32],
    pub residual_membership_hash: [u8; 32],
    pub earliest_effective_valid_until: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RevisionAdjudicationReportInputSealV1 {
    pub report_tool_truth_authority_set: ReportToolTruthAuthoritySetRefV1,
    pub revision_adjudication_authority_set: RevisionAdjudicationAuthoritySetRefV1,
    pub source_member_count: u64,
    pub source_set_hash: [u8; 32],
    pub report_input_hash: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyCoverageLimitationCode {
    LegacyCoverageUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LegacyReportInputSealV1 {
    pub report_tool_truth_authority_set: ReportToolTruthAuthoritySetRefV1,
    pub legacy_report_authority_seal_id: Uuid,
    pub legacy_report_authority_seal_hash: [u8; 32],
    pub final_scope_source_set_hash: [u8; 32],
    pub source_member_count: u64,
    pub source_set_hash: [u8; 32],
    pub limitation_membership_hash: [u8; 32],
    pub mandatory_limitation_code: LegacyCoverageLimitationCode,
    pub report_input_hash: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "authority_contract", rename_all = "snake_case")]
pub enum ReportInputSealV1 {
    RevisionAdjudication(RevisionAdjudicationReportInputSealV1),
    Legacy(LegacyReportInputSealV1),
}

impl ReportInputSealV1 {
    pub fn source_member_count(&self) -> u64 {
        match self {
            Self::RevisionAdjudication(seal) => seal.source_member_count,
            Self::Legacy(seal) => seal.source_member_count,
        }
    }

    pub fn source_set_hash(&self) -> [u8; 32] {
        match self {
            Self::RevisionAdjudication(seal) => seal.source_set_hash,
            Self::Legacy(seal) => seal.source_set_hash,
        }
    }

    pub fn report_input_hash(&self) -> [u8; 32] {
        match self {
            Self::RevisionAdjudication(seal) => seal.report_input_hash,
            Self::Legacy(seal) => seal.report_input_hash,
        }
    }

    pub fn compute_report_input_hash(&self) -> Result<[u8; 32], serde_json::Error> {
        let mut material = self.clone();
        match &mut material {
            Self::RevisionAdjudication(seal) => seal.report_input_hash = [0; 32],
            Self::Legacy(seal) => seal.report_input_hash = [0; 32],
        }
        Ok(Sha256::digest(serde_json::to_vec(&material)?).into())
    }

    pub fn validate(
        &self,
        source_count: usize,
        source_set_hash: [u8; 32],
        observed_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), String> {
        let (tool_truth_set, authority_effective_valid_until) = match self {
            Self::RevisionAdjudication(seal) => {
                let authority = &seal.revision_adjudication_authority_set;
                if authority.authority_set_id.is_nil()
                    || authority.authority_member_count == 0
                    || !hash_present(authority.authority_set_hash)
                    || !hash_present(authority.coverage_membership_hash)
                    || !hash_present(authority.residual_membership_hash)
                    || authority.earliest_effective_valid_until <= observed_at
                {
                    return Err("report_revision_input_authority_invalid".to_owned());
                }
                (
                    &seal.report_tool_truth_authority_set,
                    authority.earliest_effective_valid_until,
                )
            }
            Self::Legacy(seal) => {
                if seal.legacy_report_authority_seal_id.is_nil()
                    || !hash_present(seal.legacy_report_authority_seal_hash)
                    || !hash_present(seal.final_scope_source_set_hash)
                    || seal.final_scope_source_set_hash != source_set_hash
                    || !hash_present(seal.limitation_membership_hash)
                    || seal.mandatory_limitation_code
                        != LegacyCoverageLimitationCode::LegacyCoverageUnavailable
                {
                    return Err("report_legacy_input_authority_invalid".to_owned());
                }
                (
                    &seal.report_tool_truth_authority_set,
                    seal.report_tool_truth_authority_set
                        .earliest_effective_valid_until,
                )
            }
        };
        let count_matches = usize::try_from(self.source_member_count()).ok() == Some(source_count);
        if tool_truth_set.authority_set_id.is_nil()
            || tool_truth_set.authority_member_count == 0
            || !hash_present(tool_truth_set.authority_set_hash)
            || tool_truth_set.earliest_effective_valid_until <= observed_at
            || authority_effective_valid_until <= observed_at
            || !count_matches
            || self.source_set_hash() != source_set_hash
            || self.compute_report_input_hash().ok() != Some(self.report_input_hash())
        {
            return Err("report_input_seal_invalid".to_owned());
        }
        Ok(())
    }
}

fn hash_present(value: [u8; 32]) -> bool {
    value != [0; 32]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoricalAuthorityTimeStatusV0 {
    AsOfFresh,
    TemporallyStale,
    RevokedHistory,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HistoricalArtifactReadAuthorityV0 {
    pub historical_artifact_receipt_id: Uuid,
    pub metadata_manifest_hash: [u8; 32],
    pub current_read_attestation_id: Uuid,
    pub current_read_attestation_hash: [u8; 32],
    pub request_private_snapshot_hash: [u8; 32],
    pub authority_time_status: HistoricalAuthorityTimeStatusV0,
}

use crate::{OrganizationReportSection, ReportCitation, ReportFinding, ReportResidual};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportSourceKind {
    StageEpisode,
    StageHandoff,
    InvestigationClosurePublication,
    InvestigationClosurePublicationMember,
    InvestigationClosureResidual,
    Finding,
    TechniqueOutcome,
    CandidateAttempt,
    FindingLineage,
    PostExploitAction,
    Foothold,
    InternalAssetObservation,
    AttackPath,
    ObjectiveAttempt,
    CleanupObligation,
    CleanupWaiver,
    CleanupBlockedDecision,
    EvidenceAudit,
    HypothesisRoot,
    HypothesisRevision,
    HypothesisEvent,
    HypothesisRelation,
    CandidateAnalysisSnapshot,
    InputProcessingDisposition,
    VerificationCampaign,
    VerificationCampaignRound,
    VerificationStrategyDecision,
    PreparedAction,
    PreparedActionAuthorization,
    PreparedActionExecutionReceipt,
    ActionOracleAssessment,
    CampaignAdjudication,
    CampaignTerminalReceipt,
    CampaignObjectiveOutcome,
    HypothesisVerificationPlanSeal,
    HypothesisProofPathSet,
    HypothesisClaimComponentSet,
    HypothesisRevisionAdjudication,
    HypothesisRevisionTerminalDecision,
    RefutationContract,
    FactDeltaConsumption,
    HypothesisGenerationSeal,
    EnrichmentObligation,
    CapabilityAssessment,
    OracleCensusReceipt,
    FinalWaveCoverageReceipt,
    LegacyAttemptAuthorityReceipt,
    LegacyReportAuthoritySeal,
    HistoricalArtifactReceipt,
    AuthorityQuarantineEvent,
    HypothesisResidual,
}

impl ReportSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StageEpisode => "stage_episode",
            Self::StageHandoff => "stage_handoff",
            Self::InvestigationClosurePublication => "investigation_closure_publication",
            Self::InvestigationClosurePublicationMember => {
                "investigation_closure_publication_member"
            }
            Self::InvestigationClosureResidual => "investigation_closure_residual",
            Self::Finding => "finding",
            Self::TechniqueOutcome => "technique_outcome",
            Self::CandidateAttempt => "candidate_attempt",
            Self::FindingLineage => "finding_lineage",
            Self::PostExploitAction => "post_exploit_action",
            Self::Foothold => "foothold",
            Self::InternalAssetObservation => "internal_asset_observation",
            Self::AttackPath => "attack_path",
            Self::ObjectiveAttempt => "objective_attempt",
            Self::CleanupObligation => "cleanup_obligation",
            Self::CleanupWaiver => "cleanup_waiver",
            Self::CleanupBlockedDecision => "cleanup_blocked_decision",
            Self::EvidenceAudit => "evidence_audit",
            Self::HypothesisRoot => "hypothesis_root",
            Self::HypothesisRevision => "hypothesis_revision",
            Self::HypothesisEvent => "hypothesis_event",
            Self::HypothesisRelation => "hypothesis_relation",
            Self::CandidateAnalysisSnapshot => "candidate_analysis_snapshot",
            Self::InputProcessingDisposition => "input_processing_disposition",
            Self::VerificationCampaign => "verification_campaign",
            Self::VerificationCampaignRound => "verification_campaign_round",
            Self::VerificationStrategyDecision => "verification_strategy_decision",
            Self::PreparedAction => "prepared_action",
            Self::PreparedActionAuthorization => "prepared_action_authorization",
            Self::PreparedActionExecutionReceipt => "prepared_action_execution_receipt",
            Self::ActionOracleAssessment => "action_oracle_assessment",
            Self::CampaignAdjudication => "campaign_adjudication",
            Self::CampaignTerminalReceipt => "campaign_terminal_receipt",
            Self::CampaignObjectiveOutcome => "campaign_objective_outcome",
            Self::HypothesisVerificationPlanSeal => "hypothesis_verification_plan_seal",
            Self::HypothesisProofPathSet => "hypothesis_proof_path_set",
            Self::HypothesisClaimComponentSet => "hypothesis_claim_component_set",
            Self::HypothesisRevisionAdjudication => "hypothesis_revision_adjudication",
            Self::HypothesisRevisionTerminalDecision => "hypothesis_revision_terminal_decision",
            Self::RefutationContract => "refutation_contract",
            Self::FactDeltaConsumption => "fact_delta_consumption",
            Self::HypothesisGenerationSeal => "hypothesis_generation_seal",
            Self::EnrichmentObligation => "enrichment_obligation",
            Self::CapabilityAssessment => "capability_assessment",
            Self::OracleCensusReceipt => "oracle_census_receipt",
            Self::FinalWaveCoverageReceipt => "final_wave_coverage_receipt",
            Self::LegacyAttemptAuthorityReceipt => "legacy_attempt_authority_receipt",
            Self::LegacyReportAuthoritySeal => "legacy_report_authority_seal",
            Self::HistoricalArtifactReceipt => "historical_artifact_receipt",
            Self::AuthorityQuarantineEvent => "authority_quarantine_event",
            Self::HypothesisResidual => "hypothesis_residual",
        }
    }
}

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ReportAuthorityClass {
    SecurityVerdictAuthority,
    GrandfatheredLegacySecurityVerdict,
    CoverageAuthority,
    ExecutionObservationAudit,
    #[default]
    MethodAuditOnly,
    AuthorizationAudit,
    HistoricalArtifactReadOnly,
}

impl ReportAuthorityClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SecurityVerdictAuthority => "security_verdict_authority",
            Self::GrandfatheredLegacySecurityVerdict => "grandfathered_legacy_security_verdict",
            Self::CoverageAuthority => "coverage_authority",
            Self::ExecutionObservationAudit => "execution_observation_audit",
            Self::MethodAuditOnly => "method_audit_only",
            Self::AuthorizationAudit => "authorization_audit",
            Self::HistoricalArtifactReadOnly => "historical_artifact_read_only",
        }
    }
}

impl TryFrom<&str> for ReportAuthorityClass {
    type Error = &'static str;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "security_verdict_authority" => Ok(Self::SecurityVerdictAuthority),
            "grandfathered_legacy_security_verdict" => Ok(Self::GrandfatheredLegacySecurityVerdict),
            "coverage_authority" => Ok(Self::CoverageAuthority),
            "execution_observation_audit" => Ok(Self::ExecutionObservationAudit),
            "method_audit_only" => Ok(Self::MethodAuditOnly),
            "authorization_audit" => Ok(Self::AuthorizationAudit),
            "historical_artifact_read_only" => Ok(Self::HistoricalArtifactReadOnly),
            _ => Err("report_authority_class_unknown"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportSourceVersion {
    pub kind: ReportSourceKind,
    #[serde(default)]
    pub authority_class: ReportAuthorityClass,
    pub id: CanonicalRowId,
    pub row_version: i64,
    pub content_hash: [u8; 32],
}

impl ReportSourceVersion {
    fn canonical_key(&self) -> Result<(String, String, String), String> {
        let stored = StoredCanonicalRowId::from_domain(&self.id)
            .map_err(|error| error.code().to_string())?;
        Ok((
            format!("{}:{}", self.kind.as_str(), self.authority_class.as_str()),
            stored.kind,
            stored.value,
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportSourceSnapshot {
    pub transaction_snapshot: String,
    pub ordered_sources: Vec<ReportSourceVersion>,
    pub source_set_hash: [u8; 32],
}

impl ReportSourceSnapshot {
    pub fn freeze(
        transaction_snapshot: impl Into<String>,
        mut sources: Vec<ReportSourceVersion>,
    ) -> Result<Self, String> {
        if sources.iter().any(|source| source.row_version < 0) {
            return Err("report_source_version_invalid".to_string());
        }
        sources.sort_by_cached_key(|source| source.canonical_key());
        let has_duplicate = sources
            .windows(2)
            .any(|pair| pair[0].kind == pair[1].kind && pair[0].id == pair[1].id);
        if has_duplicate {
            return Err("report_source_duplicate".to_string());
        }
        let source_set_hash = canonical_source_set_hash(&sources)?;
        Ok(Self {
            transaction_snapshot: transaction_snapshot.into(),
            ordered_sources: sources,
            source_set_hash,
        })
    }
}

pub fn canonical_source_set_hash(sources: &[ReportSourceVersion]) -> Result<[u8; 32], String> {
    let mut hasher = Sha256::new();
    for source in sources {
        let (kind, id_kind, id_value) = source.canonical_key()?;
        for field in [kind.as_bytes(), id_kind.as_bytes(), id_value.as_bytes()] {
            hasher.update((field.len() as u64).to_be_bytes());
            hasher.update(field);
        }
        hasher.update(source.row_version.to_be_bytes());
        hasher.update(source.content_hash);
    }
    Ok(hasher.finalize().into())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportReadModel {
    pub report_id: Uuid,
    pub revision_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub scope_snapshot_hash: String,
    pub source_snapshot: ReportSourceSnapshot,
    pub organization_sections: Vec<OrganizationReportSection>,
    pub findings: Vec<ReportFinding>,
    pub cleanup_residuals: Vec<ReportResidual>,
    pub citations: Vec<ReportCitation>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(kind: ReportSourceKind, id: &str, version: i64, byte: u8) -> ReportSourceVersion {
        ReportSourceVersion {
            kind,
            authority_class: ReportAuthorityClass::MethodAuditOnly,
            id: CanonicalRowId::Text(id.to_string()),
            row_version: version,
            content_hash: [byte; 32],
        }
    }

    #[test]
    fn source_set_hash_is_order_independent_but_rejects_new_source() {
        let first = source(ReportSourceKind::Finding, "finding-1", 4, 7);
        let second = source(ReportSourceKind::CleanupObligation, "cleanup-1", 2, 9);
        let a = ReportSourceSnapshot::freeze("tx-1", vec![first.clone(), second.clone()])
            .expect("snapshot");
        let b = ReportSourceSnapshot::freeze("tx-2", vec![second.clone(), first.clone()])
            .expect("snapshot");
        assert_eq!(a.source_set_hash, b.source_set_hash);

        let changed = ReportSourceSnapshot::freeze(
            "tx-3",
            vec![
                first,
                second,
                source(ReportSourceKind::TechniqueOutcome, "outcome-1", 1, 3),
            ],
        )
        .expect("changed snapshot");
        assert_ne!(a.source_set_hash, changed.source_set_hash);
    }

    fn sealed_revision_input(observed_at: chrono::DateTime<chrono::Utc>) -> ReportInputSealV1 {
        let mut seal =
            ReportInputSealV1::RevisionAdjudication(RevisionAdjudicationReportInputSealV1 {
                report_tool_truth_authority_set: ReportToolTruthAuthoritySetRefV1 {
                    authority_set_id: Uuid::from_u128(1),
                    authority_member_count: 2,
                    authority_set_hash: [1; 32],
                    earliest_effective_valid_until: observed_at + chrono::Duration::minutes(5),
                },
                revision_adjudication_authority_set: RevisionAdjudicationAuthoritySetRefV1 {
                    authority_set_id: Uuid::from_u128(2),
                    authority_member_count: 3,
                    authority_set_hash: [2; 32],
                    coverage_membership_hash: [3; 32],
                    residual_membership_hash: [4; 32],
                    earliest_effective_valid_until: observed_at + chrono::Duration::minutes(4),
                },
                source_member_count: 4,
                source_set_hash: [5; 32],
                report_input_hash: [0; 32],
            });
        let hash = seal.compute_report_input_hash().expect("input hash");
        let ReportInputSealV1::RevisionAdjudication(value) = &mut seal else {
            unreachable!()
        };
        value.report_input_hash = hash;
        seal
    }

    #[test]
    fn operation_level_report_input_seal_rejects_missing_or_tampered_exact_sets() {
        let observed_at = chrono::Utc::now();
        let seal = sealed_revision_input(observed_at);
        assert_eq!(seal.validate(4, [5; 32], observed_at), Ok(()));

        let mut missing_member = seal.clone();
        let ReportInputSealV1::RevisionAdjudication(value) = &mut missing_member else {
            unreachable!()
        };
        value
            .revision_adjudication_authority_set
            .authority_member_count = 0;
        assert_eq!(
            missing_member.validate(4, [5; 32], observed_at),
            Err("report_revision_input_authority_invalid".to_owned())
        );

        let mut tampered_coverage = seal;
        let ReportInputSealV1::RevisionAdjudication(value) = &mut tampered_coverage else {
            unreachable!()
        };
        value
            .revision_adjudication_authority_set
            .coverage_membership_hash = [9; 32];
        assert_eq!(
            tampered_coverage.validate(4, [5; 32], observed_at),
            Err("report_input_seal_invalid".to_owned())
        );
    }
}
