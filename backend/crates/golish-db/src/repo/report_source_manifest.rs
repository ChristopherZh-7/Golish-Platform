use golish_memory_domain::source_ref::StoredCanonicalRowId;
use golish_reporting_domain::{
    ReportAuthorityClass, ReportSourceKind, ReportSourceSnapshot, ReportSourceVersion,
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::Result;

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
pub struct ReportSourceManifestRow {
    pub revision_id: Uuid,
    pub ordinal: i32,
    pub source_kind: String,
    pub authority_class: String,
    pub source_id_kind: String,
    pub source_id_value: String,
    pub source_row_version: i64,
    pub content_hash: Vec<u8>,
}

pub async fn insert_snapshot(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: Uuid,
    snapshot: &ReportSourceSnapshot,
) -> Result<()> {
    for (ordinal, source) in snapshot.ordered_sources.iter().enumerate() {
        let id = StoredCanonicalRowId::from_domain(&source.id)
            .map_err(|error| anyhow::anyhow!(error.code()))?;
        sqlx::query(
            r#"INSERT INTO report_source_manifest(
                   revision_id,ordinal,source_kind,authority_class,source_id_kind,source_id_value,
                   source_row_version,content_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(revision_id)
        .bind(i32::try_from(ordinal).map_err(|_| anyhow::anyhow!("report_source_overflow"))?)
        .bind(source.kind.as_str())
        .bind(source.authority_class.as_str())
        .bind(id.kind)
        .bind(id.value)
        .bind(source.row_version)
        .bind(source.content_hash.as_slice())
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub fn parse_source_kind(value: &str) -> Result<ReportSourceKind> {
    let kind = match value {
        "stage_episode" => ReportSourceKind::StageEpisode,
        "stage_handoff" => ReportSourceKind::StageHandoff,
        "investigation_closure_publication" => ReportSourceKind::InvestigationClosurePublication,
        "investigation_closure_publication_member" => {
            ReportSourceKind::InvestigationClosurePublicationMember
        }
        "investigation_closure_residual" => ReportSourceKind::InvestigationClosureResidual,
        "finding" => ReportSourceKind::Finding,
        "technique_outcome" => ReportSourceKind::TechniqueOutcome,
        "candidate_attempt" => ReportSourceKind::CandidateAttempt,
        "finding_lineage" => ReportSourceKind::FindingLineage,
        "post_exploit_action" => ReportSourceKind::PostExploitAction,
        "foothold" => ReportSourceKind::Foothold,
        "internal_asset_observation" => ReportSourceKind::InternalAssetObservation,
        "attack_path" => ReportSourceKind::AttackPath,
        "objective_attempt" => ReportSourceKind::ObjectiveAttempt,
        "cleanup_obligation" => ReportSourceKind::CleanupObligation,
        "cleanup_waiver" => ReportSourceKind::CleanupWaiver,
        "cleanup_blocked_decision" => ReportSourceKind::CleanupBlockedDecision,
        "evidence_audit" => ReportSourceKind::EvidenceAudit,
        "hypothesis_root" => ReportSourceKind::HypothesisRoot,
        "hypothesis_revision" => ReportSourceKind::HypothesisRevision,
        "hypothesis_event" => ReportSourceKind::HypothesisEvent,
        "hypothesis_relation" => ReportSourceKind::HypothesisRelation,
        "candidate_analysis_snapshot" => ReportSourceKind::CandidateAnalysisSnapshot,
        "input_processing_disposition" => ReportSourceKind::InputProcessingDisposition,
        "verification_campaign" => ReportSourceKind::VerificationCampaign,
        "verification_campaign_round" => ReportSourceKind::VerificationCampaignRound,
        "verification_strategy_decision" => ReportSourceKind::VerificationStrategyDecision,
        "prepared_action" => ReportSourceKind::PreparedAction,
        "prepared_action_authorization" => ReportSourceKind::PreparedActionAuthorization,
        "prepared_action_execution_receipt" => ReportSourceKind::PreparedActionExecutionReceipt,
        "action_oracle_assessment" => ReportSourceKind::ActionOracleAssessment,
        "campaign_adjudication" => ReportSourceKind::CampaignAdjudication,
        "campaign_terminal_receipt" => ReportSourceKind::CampaignTerminalReceipt,
        "campaign_objective_outcome" => ReportSourceKind::CampaignObjectiveOutcome,
        "hypothesis_verification_plan_seal" => ReportSourceKind::HypothesisVerificationPlanSeal,
        "hypothesis_proof_path_set" => ReportSourceKind::HypothesisProofPathSet,
        "hypothesis_claim_component_set" => ReportSourceKind::HypothesisClaimComponentSet,
        "hypothesis_revision_adjudication" => ReportSourceKind::HypothesisRevisionAdjudication,
        "hypothesis_revision_terminal_decision" => {
            ReportSourceKind::HypothesisRevisionTerminalDecision
        }
        "refutation_contract" => ReportSourceKind::RefutationContract,
        "fact_delta_consumption" => ReportSourceKind::FactDeltaConsumption,
        "hypothesis_generation_seal" => ReportSourceKind::HypothesisGenerationSeal,
        "enrichment_obligation" => ReportSourceKind::EnrichmentObligation,
        "capability_assessment" => ReportSourceKind::CapabilityAssessment,
        "oracle_census_receipt" => ReportSourceKind::OracleCensusReceipt,
        "final_wave_coverage_receipt" => ReportSourceKind::FinalWaveCoverageReceipt,
        "legacy_attempt_authority_receipt" => ReportSourceKind::LegacyAttemptAuthorityReceipt,
        "legacy_report_authority_seal" => ReportSourceKind::LegacyReportAuthoritySeal,
        "historical_artifact_receipt" => ReportSourceKind::HistoricalArtifactReceipt,
        "authority_quarantine_event" => ReportSourceKind::AuthorityQuarantineEvent,
        "hypothesis_residual" => ReportSourceKind::HypothesisResidual,
        _ => return Err(anyhow::anyhow!("report_source_kind_corrupt").into()),
    };
    Ok(kind)
}

pub fn row_to_source(row: ReportSourceManifestRow) -> Result<ReportSourceVersion> {
    let content_hash: [u8; 32] = row
        .content_hash
        .try_into()
        .map_err(|_| anyhow::anyhow!("report_source_hash_corrupt"))?;
    Ok(ReportSourceVersion {
        kind: parse_source_kind(&row.source_kind)?,
        authority_class: ReportAuthorityClass::try_from(row.authority_class.as_str())
            .map_err(|code| anyhow::anyhow!(code))?,
        id: StoredCanonicalRowId {
            kind: row.source_id_kind,
            value: row.source_id_value,
        }
        .into_domain()
        .map_err(|error| anyhow::anyhow!(error.code()))?,
        row_version: row.source_row_version,
        content_hash,
    })
}

pub async fn list(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: Uuid,
) -> Result<Vec<ReportSourceManifestRow>> {
    Ok(sqlx::query_as::<_, ReportSourceManifestRow>(
        "SELECT * FROM report_source_manifest WHERE revision_id=$1 ORDER BY ordinal",
    )
    .bind(revision_id)
    .fetch_all(&mut **tx)
    .await?)
}
