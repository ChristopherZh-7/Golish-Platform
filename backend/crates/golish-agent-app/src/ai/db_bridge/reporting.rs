//! Reporting authority adapter seams.
//!
//! This bridge deliberately exposes only canonical DB truth. It has no RAG,
//! wiki, memory-context or Graphiti fallback. In particular, cleanup closeout
//! delegates to the Cleanup-owned deterministic gate query rather than
//! recreating its semantics in Reporting SQL.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Arc;

use golish_agent_kit::harness::handoff_catalog::{
    CanonicalFactKey, CanonicalFactRef, StageHandoffPayload,
};
use golish_core::{InvestigationRolloutMode, StageTopologyContract};
use golish_memory_domain::source_ref::{CanonicalRowId, StoredCanonicalRowId};
use golish_reporting_domain::{
    CitationSourceType, CleanupBlockedDecisionTruth, CleanupCloseoutTruth,
    CoverageSufficiencyProjection, EvidenceAuditTruth, OrganizationReportSection,
    PublicationStatus, ReportAuthorityClass, ReportCitation, ReportClaim, ReportClaimKind,
    ReportClaimValue, ReportFinding, ReportReadModel, ReportResidual, ReportSectionKind,
    ReportSectionModel, ReportSourceKind, ReportSourceVersion, ReportValidationResult,
    ReportValidationTruth, SecurityVerdictAuthority, SecurityVerdictProjection, ValidationStatus,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use golish_reporting_app::{
    redact_report_value, BuiltReportRevision, ContentAddressedArtifact, FinalizePublication,
    ReportPublicationPort, ReportTruthPort, ReportingAppError,
};
use golish_reporting_domain::ReportSourceSnapshot;

/// Exact server-derived project/scope identity witnessed at the Reporting IPC
/// authorization boundary. Critical build persistence and publication paths
/// must compare this witness while holding the authoritative rows in their own
/// transaction; an earlier autocommit authorization read is not sufficient.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportingProjectAuthority {
    project_scope_id: Uuid,
    scope_snapshot_id: Uuid,
    scope_snapshot_hash: String,
    canonical_project_path: String,
    path_sha256: String,
    project_row_version: i64,
}

impl ReportingProjectAuthority {
    pub fn new(
        project_scope_id: Uuid,
        scope_snapshot_id: Uuid,
        scope_snapshot_hash: String,
        canonical_project_path: String,
        path_sha256: String,
        project_row_version: i64,
    ) -> Self {
        Self {
            project_scope_id,
            scope_snapshot_id,
            scope_snapshot_hash,
            canonical_project_path,
            path_sha256,
            project_row_version,
        }
    }

    pub fn canonical_project_path(&self) -> &str {
        &self.canonical_project_path
    }
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct ReportingProjectAuthorityRow {
    project_scope_id: Uuid,
    scope_snapshot_id: Uuid,
    scope_snapshot_hash: String,
    canonical_project_path: String,
    path_sha256: String,
    project_row_version: i64,
}

async fn require_reporting_project_authority_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    expected: &ReportingProjectAuthority,
    lock: bool,
) -> anyhow::Result<()> {
    let query = if lock {
        r#"SELECT project.project_scope_id,snapshot.id AS scope_snapshot_id,
                  snapshot.scope_hash AS scope_snapshot_hash,
                  project.canonical_project_path,project.path_sha256,
                  project.row_version AS project_row_version
             FROM operation_state AS operation
             JOIN project_scopes AS project
               ON project.project_scope_id=operation.project_scope_id
              AND project.retired_at IS NULL
             JOIN operation_org_scope_snapshots AS snapshot
               ON snapshot.operation_id=operation.operation_id
              AND snapshot.project_scope_id=project.project_scope_id
            WHERE operation.operation_id=$1 AND snapshot.id=$2
              AND snapshot.sealed_at IS NOT NULL
              AND snapshot.project_path_at_freeze=project.canonical_project_path
              AND EXISTS(
                  SELECT 1 FROM operation_org_scope_units AS root_unit
                   WHERE root_unit.snapshot_id=snapshot.id
                     AND root_unit.organization_id=snapshot.root_organization_id
                     AND root_unit.role='root'
              )
            FOR SHARE OF operation,project,snapshot"#
    } else {
        r#"SELECT project.project_scope_id,snapshot.id AS scope_snapshot_id,
                  snapshot.scope_hash AS scope_snapshot_hash,
                  project.canonical_project_path,project.path_sha256,
                  project.row_version AS project_row_version
             FROM operation_state AS operation
             JOIN project_scopes AS project
               ON project.project_scope_id=operation.project_scope_id
              AND project.retired_at IS NULL
             JOIN operation_org_scope_snapshots AS snapshot
               ON snapshot.operation_id=operation.operation_id
              AND snapshot.project_scope_id=project.project_scope_id
            WHERE operation.operation_id=$1 AND snapshot.id=$2
              AND snapshot.sealed_at IS NOT NULL
              AND snapshot.project_path_at_freeze=project.canonical_project_path
              AND EXISTS(
                  SELECT 1 FROM operation_org_scope_units AS root_unit
                   WHERE root_unit.snapshot_id=snapshot.id
                     AND root_unit.organization_id=snapshot.root_organization_id
                     AND root_unit.role='root'
              )"#
    };
    let current = sqlx::query_as::<_, ReportingProjectAuthorityRow>(query)
        .bind(operation_id)
        .bind(expected.scope_snapshot_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or_else(|| anyhow::anyhow!("report_project_authority_stale"))?;
    if current.project_scope_id != expected.project_scope_id
        || current.scope_snapshot_id != expected.scope_snapshot_id
        || current.scope_snapshot_hash != expected.scope_snapshot_hash
        || current.canonical_project_path != expected.canonical_project_path
        || current.path_sha256 != expected.path_sha256
        || current.project_row_version != expected.project_row_version
    {
        anyhow::bail!("report_project_authority_stale");
    }
    Ok(())
}

/// Load the active exact project/scope witness for an internal Reporting stage
/// entry. The operation binding, unretired project identity, sealed snapshot,
/// frozen project path and exact root unit are read server-side; callers never
/// get to supply or omit this authority.
pub(super) async fn load_reporting_project_authority(
    pool: &PgPool,
    operation_id: Uuid,
) -> anyhow::Result<ReportingProjectAuthority> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query_as::<_, ReportingProjectAuthorityRow>(
        r#"SELECT project.project_scope_id,snapshot.id AS scope_snapshot_id,
                  snapshot.scope_hash AS scope_snapshot_hash,
                  project.canonical_project_path,project.path_sha256,
                  project.row_version AS project_row_version
             FROM operation_state AS operation
             JOIN project_scopes AS project
               ON project.project_scope_id=operation.project_scope_id
              AND project.retired_at IS NULL
             JOIN operation_org_scope_snapshots AS snapshot
               ON snapshot.operation_id=operation.operation_id
              AND snapshot.project_scope_id=project.project_scope_id
              AND snapshot.sealed_at IS NOT NULL
              AND snapshot.project_path_at_freeze=project.canonical_project_path
            WHERE operation.operation_id=$1
              AND EXISTS(
                  SELECT 1 FROM operation_org_scope_units AS root_unit
                   WHERE root_unit.snapshot_id=snapshot.id
                     AND root_unit.organization_id=snapshot.root_organization_id
                     AND root_unit.role='root'
              )
            FOR SHARE OF operation,project,snapshot"#,
    )
    .bind(operation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("report_project_authority_stale"))?;
    tx.commit().await?;
    Ok(ReportingProjectAuthority::new(
        row.project_scope_id,
        row.scope_snapshot_id,
        row.scope_snapshot_hash,
        row.canonical_project_path,
        row.path_sha256,
        row.project_row_version,
    ))
}

pub(super) async fn lock_reporting_project_authority(
    pool: &PgPool,
    operation_id: Uuid,
    authority: &ReportingProjectAuthority,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;
    require_reporting_project_authority_on(&mut tx, operation_id, authority, true).await?;
    tx.commit().await?;
    Ok(())
}

/// Deterministic two-connection test seam for proving that Reporting Gate
/// cannot combine facts from different PostgreSQL snapshots.
#[doc(hidden)]
pub async fn load_reporting_gate_truth_with_barrier<F, Fut>(
    pool: &Arc<PgPool>,
    operation_id: Uuid,
    after_bundle: F,
) -> anyhow::Result<Option<golish_agent_kit::harness::ReportingGateTruth>>
where
    F: FnOnce() -> Fut + Send,
    Fut: Future<Output = ()> + Send,
{
    super::reporting_gate::load_reporting_gate_truth_with_barrier(pool, operation_id, after_bundle)
        .await
}

pub async fn cleanup_closeout_truth(
    pool: &PgPool,
    operation_id: Uuid,
    organization_ids: &BTreeSet<Uuid>,
) -> anyhow::Result<Vec<CleanupCloseoutTruth>> {
    let mut connection = pool.acquire().await?;
    cleanup_closeout_truth_on(&mut connection, operation_id, organization_ids).await
}

pub(super) async fn cleanup_closeout_truth_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    organization_ids: &BTreeSet<Uuid>,
) -> anyhow::Result<Vec<CleanupCloseoutTruth>> {
    let mut truth = Vec::with_capacity(organization_ids.len());
    for organization_id in organization_ids {
        let gate = golish_db::repo::organization_deletion_jobs::cleanup_closeout_gate_on(
            &mut *connection,
            operation_id,
            *organization_id,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        truth.push(CleanupCloseoutTruth {
            organization_id_at_time: *organization_id,
            missing_obligation_count: gate.missing_obligation_count,
            nonterminal_obligation_count: gate.nonterminal_obligation_count,
            undisclosed_residual_count: gate.undisclosed_residual_count,
            invalid_terminal_truth_count: gate.invalid_terminal_truth_count,
            residual_obligation_ids: gate.residual_obligation_ids,
        });
    }
    Ok(truth)
}

pub async fn frozen_organization_ids(
    pool: &PgPool,
    operation_id: Uuid,
) -> anyhow::Result<BTreeSet<Uuid>> {
    let mut connection = pool.acquire().await?;
    frozen_organization_ids_on(&mut connection, operation_id).await
}

pub(super) async fn frozen_organization_ids_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
) -> anyhow::Result<BTreeSet<Uuid>> {
    let ids = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT unit.organization_id
             FROM operation_org_scope_snapshots AS snapshot
             JOIN operation_org_scope_units AS unit ON unit.snapshot_id=snapshot.id
            WHERE snapshot.operation_id=$1 AND snapshot.sealed_at IS NOT NULL
            ORDER BY unit.ordinal"#,
    )
    .bind(operation_id)
    .fetch_all(&mut *connection)
    .await?;
    Ok(ids.into_iter().collect())
}

/// A missing exact operation binding on a reportable source is a hard block,
/// not permission to infer ownership from a session/run string.
pub async fn technique_outcomes_have_exact_operation_binding(
    pool: &PgPool,
) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM information_schema.columns
                WHERE table_schema='public' AND table_name='technique_outcomes'
                  AND column_name='operation_id'
           )"#,
    )
    .fetch_one(pool)
    .await?)
}

const REPORTING_COMMON_SOURCE_STAGES: &[&str] = &[
    "target_intel",
    "external_attack_surface",
    "enumeration",
    "vuln_triage",
];

const REPORTING_LEGACY_SOURCE_STAGES: &[&str] = &[
    "attack_candidate",
    "verification",
    "access_validation",
    "internal_discovery",
    "objective_pathing",
    "objective_simulation",
];

const REPORTING_UNIFIED_SOURCE_STAGES: &[&str] = &["application_understanding", "investigation"];

async fn frozen_reporting_topology_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
) -> anyhow::Result<StageTopologyContract> {
    let (topology_value, canonical_json, sha256, freeze_source, rollout_mode): (
        String,
        String,
        String,
        String,
        String,
    ) = sqlx::query_as(
        r#"SELECT stage_topology_contract,stage_topology_canonical_json,
                  stage_topology_sha256,stage_topology_freeze_source,
                  investigation_rollout_mode
             FROM operation_state
            WHERE operation_id=$1"#,
    )
    .bind(operation_id)
    .fetch_one(&mut *connection)
    .await?;
    let topology = StageTopologyContract::try_parse(&topology_value)
        .map_err(|_| anyhow::anyhow!("report_stage_topology_unknown"))?;
    let material = topology.freeze_material();
    if canonical_json != material.canonical_json || sha256 != material.sha256 {
        anyhow::bail!("report_stage_topology_material_mismatch");
    }
    let legal_freeze = match freeze_source.as_str() {
        "legacy_backfill_v1" => topology == StageTopologyContract::LegacyCandidateVerificationV1,
        "deployment_pair_v1" => InvestigationRolloutMode::try_from(rollout_mode.as_str())
            .is_ok_and(|mode| topology.allows_investigation_rollout(mode)),
        _ => false,
    };
    if !legal_freeze {
        anyhow::bail!("report_stage_topology_authority_invalid");
    }
    Ok(topology)
}

fn reporting_source_stages(topology: StageTopologyContract) -> Vec<String> {
    REPORTING_COMMON_SOURCE_STAGES
        .iter()
        .chain(match topology {
            StageTopologyContract::LegacyCandidateVerificationV1 => {
                REPORTING_LEGACY_SOURCE_STAGES.iter()
            }
            StageTopologyContract::UnifiedInvestigationV1 => REPORTING_UNIFIED_SOURCE_STAGES.iter(),
        })
        .map(|stage| (*stage).to_string())
        .collect()
}

const fn reporting_terminal_authority_stage(topology: StageTopologyContract) -> &'static str {
    match topology {
        StageTopologyContract::LegacyCandidateVerificationV1 => "verification",
        StageTopologyContract::UnifiedInvestigationV1 => "investigation",
    }
}

const fn reporting_source_kind_is_authoritative(
    topology: StageTopologyContract,
    kind: ReportSourceKind,
) -> bool {
    match kind {
        ReportSourceKind::CandidateAttempt
        | ReportSourceKind::FindingLineage
        | ReportSourceKind::PostExploitAction
        | ReportSourceKind::Foothold
        | ReportSourceKind::InternalAssetObservation
        | ReportSourceKind::AttackPath
        | ReportSourceKind::ObjectiveAttempt
        | ReportSourceKind::LegacyAttemptAuthorityReceipt
        | ReportSourceKind::LegacyReportAuthoritySeal => {
            matches!(
                topology,
                StageTopologyContract::LegacyCandidateVerificationV1
            )
        }
        ReportSourceKind::HypothesisRoot
        | ReportSourceKind::HypothesisRevision
        | ReportSourceKind::HypothesisEvent
        | ReportSourceKind::HypothesisRelation
        | ReportSourceKind::CandidateAnalysisSnapshot
        | ReportSourceKind::InputProcessingDisposition
        | ReportSourceKind::VerificationCampaign
        | ReportSourceKind::VerificationCampaignRound
        | ReportSourceKind::VerificationStrategyDecision
        | ReportSourceKind::PreparedAction
        | ReportSourceKind::PreparedActionAuthorization
        | ReportSourceKind::PreparedActionExecutionReceipt
        | ReportSourceKind::ActionOracleAssessment
        | ReportSourceKind::CampaignAdjudication
        | ReportSourceKind::CampaignTerminalReceipt
        | ReportSourceKind::CampaignObjectiveOutcome
        | ReportSourceKind::HypothesisVerificationPlanSeal
        | ReportSourceKind::HypothesisProofPathSet
        | ReportSourceKind::HypothesisClaimComponentSet
        | ReportSourceKind::HypothesisRevisionAdjudication
        | ReportSourceKind::HypothesisRevisionTerminalDecision
        | ReportSourceKind::FactDeltaConsumption
        | ReportSourceKind::HypothesisGenerationSeal
        | ReportSourceKind::EnrichmentObligation
        | ReportSourceKind::CapabilityAssessment
        | ReportSourceKind::OracleCensusReceipt
        | ReportSourceKind::FinalWaveCoverageReceipt
        | ReportSourceKind::AuthorityQuarantineEvent
        | ReportSourceKind::HypothesisResidual
        | ReportSourceKind::InvestigationClosurePublication
        | ReportSourceKind::InvestigationClosurePublicationMember
        | ReportSourceKind::InvestigationClosureResidual => {
            matches!(topology, StageTopologyContract::UnifiedInvestigationV1)
        }
        ReportSourceKind::StageEpisode
        | ReportSourceKind::StageHandoff
        | ReportSourceKind::Finding
        | ReportSourceKind::TechniqueOutcome
        | ReportSourceKind::CleanupObligation
        | ReportSourceKind::CleanupWaiver
        | ReportSourceKind::CleanupBlockedDecision
        | ReportSourceKind::EvidenceAudit
        | ReportSourceKind::RefutationContract
        | ReportSourceKind::HistoricalArtifactReceipt => true,
    }
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).expect("serialize string"),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("serialize key"),
                        canonical_json(&object[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn sha256(value: &Value) -> String {
    Sha256::digest(canonical_json(value).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_sha256(value: &str) -> anyhow::Result<[u8; 32]> {
    if value.len() != 64 {
        anyhow::bail!("report_source_hash_invalid");
    }
    let mut bytes = [0_u8; 32];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| anyhow::anyhow!("report_source_hash_invalid"))?;
    }
    Ok(bytes)
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TechniqueAuthorityKey {
    organization_id: Uuid,
    run_id: String,
    asset: String,
    technique: String,
}

#[derive(Clone, Debug)]
struct TechniqueAuthority {
    content_sha256: String,
    evidence_ids: Vec<i64>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct TechniqueSourceRow {
    id: i64,
    organization_id: Uuid,
    run_id: String,
    asset: String,
    technique: String,
    outcome: String,
    source: Option<String>,
    collected_at: Option<chrono::DateTime<chrono::Utc>>,
    updated_at: chrono::DateTime<chrono::Utc>,
    row_version: i64,
    content: Value,
    evidence_ids: Vec<i64>,
}

#[derive(Clone, Debug)]
struct TechniqueOutcomeSetAuthority {
    reference: CanonicalFactRef,
    freshness_floor: chrono::DateTime<chrono::Utc>,
    gate_passed_at: chrono::DateTime<chrono::Utc>,
    handoff_evidence: BTreeSet<i64>,
}

#[derive(Clone, Debug)]
struct LegacyEasLivenessAuthority {
    organization_id: Uuid,
    run_id: String,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    freshness_floor: chrono::DateTime<chrono::Utc>,
    gate_passed_at: chrono::DateTime<chrono::Utc>,
    asset_evidence: BTreeMap<String, BTreeSet<i64>>,
}

#[derive(Clone, Debug)]
struct LegacyEasWebFingerprintAuthority {
    organization_id: Uuid,
    run_id: String,
    gate_passed_at: chrono::DateTime<chrono::Utc>,
    claim_evidence_sets: Vec<BTreeSet<i64>>,
    handoff_evidence: BTreeSet<i64>,
}

type LegacyEasAssetEvidence = BTreeMap<String, BTreeSet<i64>>;
type LegacyEasLivenessClaims = Option<(String, LegacyEasAssetEvidence)>;

fn exact_legacy_eas_liveness_claims(
    payload: &StageHandoffPayload,
    organization_id: Uuid,
    authorized_run_ids: &[String],
    handoff_evidence: &BTreeSet<i64>,
) -> anyhow::Result<LegacyEasLivenessClaims> {
    let watermark = &payload.coverage_watermark;
    if watermark.get("kind").and_then(Value::as_str) != Some("information_coverage_v1")
        || watermark.get("stage").and_then(Value::as_str) != Some("external_attack_surface")
    {
        return Ok(None);
    }
    let watermark_org = watermark
        .get("organization_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok());
    let run_id = watermark
        .get("run_id")
        .and_then(Value::as_str)
        .filter(|run_id| authorized_run_ids.iter().any(|allowed| allowed == run_id))
        .ok_or_else(|| anyhow::anyhow!("report_eas_liveness_watermark_run_invalid"))?;
    let techniques = watermark
        .get("techniques")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("report_eas_liveness_watermark_techniques_invalid"))?;
    if !techniques.iter().any(|technique| {
        technique.as_str() == Some(golish_db::repo::coverage_truth::TECH_EAS_LIVENESS)
    }) {
        return Ok(None);
    }
    // Current handoffs freeze EAS liveness as a canonical TechniqueOutcome
    // reference. The typed `discovery` claim below is only a compatibility
    // authority for older handoffs that predate that canonical reference.
    // Returning `None` here does not trust the reference: the normal canonical
    // reference path below still validates its ownership, hash, evidence and
    // exact persisted TechniqueOutcome row.
    if payload.canonical_fact_refs.iter().any(|reference| {
        reference.organization_id == organization_id
            && matches!(
                &reference.key,
                CanonicalFactKey::TechniqueOutcome {
                    organization_id: reference_org,
                    run_id: reference_run,
                    technique,
                    ..
                } if *reference_org == organization_id
                    && reference_run == run_id
                    && technique == golish_db::repo::coverage_truth::TECH_EAS_LIVENESS
            )
    }) {
        return Ok(None);
    }
    let complete_counts = [
        ("canonical_ref_total", payload.canonical_fact_refs.len()),
        ("canonical_ref_included", payload.canonical_fact_refs.len()),
        ("typed_claim_total", payload.typed_claims.len()),
        ("typed_claim_included", payload.typed_claims.len()),
        ("evidence_id_total", payload.evidence_ids.len()),
        ("evidence_id_included", payload.evidence_ids.len()),
    ]
    .into_iter()
    .all(|(field, expected)| {
        watermark.get(field).and_then(Value::as_u64) == u64::try_from(expected).ok()
    });
    let complete_flags = [
        "canonical_ref_truncated",
        "typed_claim_truncated",
        "evidence_id_truncated",
    ]
    .into_iter()
    .all(|field| watermark.get(field).and_then(Value::as_bool) == Some(false));
    let terminal_count = watermark
        .get("terminal_cells")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let disposition_count = ["found", "checked_empty", "blocked", "not_applicable"]
        .into_iter()
        .filter_map(|field| watermark.get(field).and_then(Value::as_u64))
        .sum::<u64>();
    if watermark_org != Some(organization_id)
        || !complete_counts
        || !complete_flags
        || terminal_count == 0
        || terminal_count != disposition_count
        || watermark
            .get("terminal_cell_set_sha256")
            .and_then(Value::as_str)
            .is_none_or(|hash| decode_sha256(hash).is_err())
    {
        anyhow::bail!("report_eas_liveness_watermark_invalid");
    }
    let watermark_assets = watermark
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("report_eas_liveness_watermark_assets_invalid"))?
        .iter()
        .map(|asset| {
            asset
                .as_str()
                .and_then(golish_agent_kit::harness::evidence_facts::eas_liveness_asset_key)
                .ok_or_else(|| anyhow::anyhow!("report_eas_liveness_watermark_asset_invalid"))
        })
        .collect::<anyhow::Result<BTreeSet<_>>>()?;
    let mut asset_evidence = BTreeMap::new();
    for claim in &payload.typed_claims {
        if claim.get("kind").and_then(Value::as_str) != Some("discovery") {
            continue;
        }
        let claim_payload = claim
            .get("payload")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("report_eas_liveness_claim_invalid"))?;
        let Some(asset) = claim_payload
            .get("subject")
            .and_then(Value::as_str)
            .and_then(golish_agent_kit::harness::evidence_facts::eas_liveness_asset_key)
        else {
            continue;
        };
        let evidence = claim_payload
            .get("evidence_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("report_eas_liveness_claim_evidence_invalid"))?
            .iter()
            .map(|value| {
                value
                    .as_i64()
                    .filter(|evidence_id| *evidence_id > 0)
                    .ok_or_else(|| anyhow::anyhow!("report_eas_liveness_claim_evidence_invalid"))
            })
            .collect::<anyhow::Result<BTreeSet<_>>>()?;
        if evidence.is_empty()
            || !evidence.is_subset(handoff_evidence)
            || !watermark_assets.contains(&asset)
            || asset_evidence.insert(asset, evidence).is_some()
        {
            anyhow::bail!("report_eas_liveness_claim_invalid");
        }
    }
    if asset_evidence.is_empty() {
        anyhow::bail!("report_eas_liveness_claim_missing");
    }
    Ok(Some((run_id.to_string(), asset_evidence)))
}

fn exact_legacy_eas_web_fingerprint_claims(
    payload: &StageHandoffPayload,
    organization_id: Uuid,
    authorized_run_ids: &[String],
    handoff_evidence: &BTreeSet<i64>,
) -> anyhow::Result<Option<(String, Vec<BTreeSet<i64>>)>> {
    let watermark = &payload.coverage_watermark;
    if watermark.get("kind").and_then(Value::as_str) != Some("information_coverage_v1")
        || watermark.get("stage").and_then(Value::as_str) != Some("external_attack_surface")
    {
        return Ok(None);
    }
    let run_id = watermark
        .get("run_id")
        .and_then(Value::as_str)
        .filter(|run_id| authorized_run_ids.iter().any(|allowed| allowed == run_id))
        .ok_or_else(|| anyhow::anyhow!("report_eas_web_fingerprint_watermark_run_invalid"))?;
    let techniques = watermark
        .get("techniques")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow::anyhow!("report_eas_web_fingerprint_watermark_techniques_invalid")
        })?;
    if !techniques.iter().any(|technique| {
        technique.as_str() == Some(golish_db::repo::coverage_truth::TECH_EAS_WEB_FP)
    }) {
        return Ok(None);
    }
    if payload.canonical_fact_refs.iter().any(|reference| {
        reference.organization_id == organization_id
            && matches!(
                &reference.key,
                CanonicalFactKey::TechniqueOutcome {
                    organization_id: reference_org,
                    run_id: reference_run,
                    technique,
                    ..
                } if *reference_org == organization_id
                    && reference_run == run_id
                    && technique == golish_db::repo::coverage_truth::TECH_EAS_WEB_FP
            )
    }) {
        return Ok(None);
    }
    let complete_counts = [
        ("canonical_ref_total", payload.canonical_fact_refs.len()),
        ("canonical_ref_included", payload.canonical_fact_refs.len()),
        ("typed_claim_total", payload.typed_claims.len()),
        ("typed_claim_included", payload.typed_claims.len()),
        ("evidence_id_total", payload.evidence_ids.len()),
        ("evidence_id_included", payload.evidence_ids.len()),
    ]
    .into_iter()
    .all(|(field, expected)| {
        watermark.get(field).and_then(Value::as_u64) == u64::try_from(expected).ok()
    });
    let complete_flags = [
        "canonical_ref_truncated",
        "typed_claim_truncated",
        "evidence_id_truncated",
    ]
    .into_iter()
    .all(|field| watermark.get(field).and_then(Value::as_bool) == Some(false));
    let terminal_count = watermark
        .get("terminal_cells")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let disposition_count = ["found", "checked_empty", "blocked", "not_applicable"]
        .into_iter()
        .filter_map(|field| watermark.get(field).and_then(Value::as_u64))
        .sum::<u64>();
    if watermark
        .get("organization_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        != Some(organization_id)
        || !complete_counts
        || !complete_flags
        || terminal_count == 0
        || terminal_count != disposition_count
        || watermark
            .get("terminal_cell_set_sha256")
            .and_then(Value::as_str)
            .is_none_or(|hash| decode_sha256(hash).is_err())
    {
        anyhow::bail!("report_eas_web_fingerprint_watermark_invalid");
    }
    let claim_evidence_sets = payload
        .typed_claims
        .iter()
        .filter(|claim| claim.get("kind").and_then(Value::as_str) == Some("web_fingerprint"))
        .map(|claim| {
            let evidence = claim
                .get("payload")
                .and_then(|payload| payload.get("evidence_ids"))
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    anyhow::anyhow!("report_eas_web_fingerprint_claim_evidence_invalid")
                })?
                .iter()
                .map(|value| {
                    value.as_i64().filter(|id| *id > 0).ok_or_else(|| {
                        anyhow::anyhow!("report_eas_web_fingerprint_claim_evidence_invalid")
                    })
                })
                .collect::<anyhow::Result<BTreeSet<_>>>()?;
            if evidence.is_empty() || !evidence.is_subset(handoff_evidence) {
                anyhow::bail!("report_eas_web_fingerprint_claim_evidence_invalid");
            }
            Ok(evidence)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Some((run_id.to_string(), claim_evidence_sets)))
}

fn exact_legacy_eas_web_fingerprint_evidence_membership(
    claim_evidence_sets: &[BTreeSet<i64>],
    handoff_evidence: &BTreeSet<i64>,
    row_evidence: &BTreeSet<i64>,
) -> bool {
    if row_evidence.is_empty() {
        return false;
    }
    if claim_evidence_sets.is_empty() {
        return row_evidence.is_subset(handoff_evidence);
    }
    claim_evidence_sets
        .iter()
        .any(|evidence| evidence == row_evidence)
}

fn technique_outcome_evidence_shape_is_reportable(outcome: &str, evidence_ids: &[i64]) -> bool {
    !evidence_ids.is_empty() || matches!(outcome, "blocked" | "not_applicable")
}

#[cfg(test)]
fn eas_evidence_producer_status_is_terminal(unit_status: &str, worker_status: &str) -> bool {
    matches!(
        (unit_status, worker_status),
        ("passed", "passed") | ("superseded", "superseded")
    )
}

/// Enumerate every exact operation-bound TechniqueOutcome and require it to be
/// present in a final-sealed, non-invalidated StageHandoff. This preserves the
/// current composite row identity without guessing ownership from arbitrary
/// run strings. A new/unsealed or content-drifted row fails closed.
pub async fn authoritative_technique_outcome_sources(
    pool: &PgPool,
    operation_id: Uuid,
    organization_ids: &BTreeSet<Uuid>,
) -> anyhow::Result<Vec<ReportSourceVersion>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let sources =
        authoritative_technique_outcome_sources_on(&mut tx, operation_id, organization_ids).await?;
    tx.commit().await?;
    Ok(sources)
}

async fn latest_final_sealed_handoffs_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    organization_id: Uuid,
    source_stage_kinds: &[String],
) -> anyhow::Result<Vec<golish_db::repo::stage_handoffs::StageHandoffRow>> {
    if source_stage_kinds.is_empty() {
        return Ok(Vec::new());
    }
    Ok(
        sqlx::query_as::<_, golish_db::repo::stage_handoffs::StageHandoffRow>(
            r#"SELECT DISTINCT ON (handoff.from_stage_kind) handoff.*
             FROM stage_handoffs AS handoff
             JOIN stage_run_units AS unit
               ON unit.id=handoff.source_stage_run_unit_id
              AND unit.operation_id=handoff.operation_id
              AND unit.stage_execution_id=handoff.stage_execution_id
              AND unit.organization_id=handoff.organization_id
              AND unit.status='passed'
             JOIN stage_deliverable_submissions AS submission
               ON submission.id=handoff.deliverable_submission_id
              AND submission.stage_run_unit_id=unit.id
             LEFT JOIN stage_worker_runs AS worker
               ON worker.id=submission.worker_run_id
              AND worker.operation_id=handoff.operation_id
              AND worker.stage_execution_id=handoff.stage_execution_id
              AND worker.stage_run_unit_id=unit.id
              AND worker.organization_id=handoff.organization_id
            WHERE handoff.operation_id=$1 AND handoff.organization_id=$2
              AND handoff.from_stage_kind=ANY($3)
              AND handoff.invalidated_at IS NULL
              AND (
                    (submission.worker_run_id IS NULL AND handoff.from_stage_kind='scoping')
                    OR worker.status='passed'
                  )
            ORDER BY handoff.from_stage_kind, handoff.gate_passed_at DESC, handoff.id DESC"#,
        )
        .bind(operation_id)
        .bind(organization_id)
        .bind(source_stage_kinds)
        .fetch_all(&mut *connection)
        .await?,
    )
}

async fn authoritative_technique_outcome_sources_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    organization_ids: &BTreeSet<Uuid>,
) -> anyhow::Result<Vec<ReportSourceVersion>> {
    let operation_chat_session_key = sqlx::query_scalar::<_, Option<String>>(
        r#"SELECT session.chat_session_key
             FROM tasks AS task
             JOIN sessions AS session ON session.id=task.session_id
            WHERE task.id=$1"#,
    )
    .bind(operation_id)
    .fetch_optional(&mut *connection)
    .await?
    .flatten()
    .filter(|key| !key.trim().is_empty());
    let mut authorized_run_ids = vec![operation_id.to_string()];
    if let Some(chat_session_key) = operation_chat_session_key.as_deref() {
        if !authorized_run_ids
            .iter()
            .any(|run_id| run_id == chat_session_key)
        {
            authorized_run_ids.push(chat_session_key.to_string());
        }
    }
    let topology = frozen_reporting_topology_on(&mut *connection, operation_id).await?;
    let source_stages = reporting_source_stages(topology);
    let terminal_authority_stage = reporting_terminal_authority_stage(topology);
    let mut sealed_refs = BTreeMap::<TechniqueAuthorityKey, TechniqueAuthority>::new();
    let mut sealed_sets = Vec::<TechniqueOutcomeSetAuthority>::new();
    let mut legacy_eas_liveness_authorities = Vec::<LegacyEasLivenessAuthority>::new();
    let mut legacy_eas_web_fingerprint_authorities = Vec::<LegacyEasWebFingerprintAuthority>::new();
    let mut handoff_sources = Vec::new();
    for organization_id in organization_ids {
        let handoffs = latest_final_sealed_handoffs_on(
            &mut *connection,
            operation_id,
            *organization_id,
            &source_stages,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        for handoff in handoffs {
            let is_terminal_authority_handoff = handoff.from_stage_kind == terminal_authority_stage;
            let handoff_content = serde_json::to_value(&handoff)
                .map_err(|_| anyhow::anyhow!("report_handoff_payload_invalid"))?;
            let payload: StageHandoffPayload = serde_json::from_value(handoff.payload.clone())
                .map_err(|_| anyhow::anyhow!("report_handoff_payload_invalid"))?;
            let handoff_evidence = handoff
                .evidence_ids
                .iter()
                .copied()
                .collect::<BTreeSet<_>>();
            if let Some((run_id, asset_evidence)) = exact_legacy_eas_liveness_claims(
                &payload,
                *organization_id,
                &authorized_run_ids,
                &handoff_evidence,
            )? {
                let freshness_floor = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
                    r#"SELECT unit.started_at
                             FROM stage_run_units AS unit
                            WHERE unit.id=$1 AND unit.operation_id=$2
                              AND unit.stage_execution_id=$3
                              AND unit.organization_id=$4
                              AND unit.stage_kind='external_attack_surface'
                              AND unit.status='passed' AND unit.started_at IS NOT NULL"#,
                )
                .bind(handoff.source_stage_run_unit_id)
                .bind(operation_id)
                .bind(handoff.stage_execution_id)
                .bind(organization_id)
                .fetch_optional(&mut *connection)
                .await?
                .ok_or_else(|| anyhow::anyhow!("report_eas_liveness_handoff_lineage_invalid"))?;
                legacy_eas_liveness_authorities.push(LegacyEasLivenessAuthority {
                    organization_id: *organization_id,
                    run_id,
                    stage_execution_id: handoff.stage_execution_id,
                    stage_run_unit_id: handoff.source_stage_run_unit_id,
                    freshness_floor,
                    gate_passed_at: handoff.gate_passed_at,
                    asset_evidence,
                });
            }
            if let Some((run_id, claim_evidence_sets)) = exact_legacy_eas_web_fingerprint_claims(
                &payload,
                *organization_id,
                &authorized_run_ids,
                &handoff_evidence,
            )? {
                legacy_eas_web_fingerprint_authorities.push(LegacyEasWebFingerprintAuthority {
                    organization_id: *organization_id,
                    run_id,
                    gate_passed_at: handoff.gate_passed_at,
                    claim_evidence_sets,
                    handoff_evidence: handoff_evidence.clone(),
                });
            }
            let mut contains_technique_ref = false;
            for reference in payload.canonical_fact_refs {
                match &reference.key {
                    CanonicalFactKey::TechniqueOutcome {
                        organization_id: ref_org,
                        run_id,
                        asset,
                        technique,
                    } => {
                        contains_technique_ref = true;
                        if *ref_org != *organization_id
                            || reference.organization_id != *organization_id
                            || !authorized_run_ids.iter().any(|allowed| allowed == run_id)
                            || reference
                                .evidence_ids
                                .iter()
                                .any(|evidence_id| !handoff_evidence.contains(evidence_id))
                            || decode_sha256(&reference.content_sha256).is_err()
                        {
                            anyhow::bail!("report_technique_handoff_authority_invalid");
                        }
                        let key = TechniqueAuthorityKey {
                            organization_id: *ref_org,
                            run_id: run_id.clone(),
                            asset: asset.clone(),
                            technique: technique.clone(),
                        };
                        if sealed_refs
                            .insert(
                                key,
                                TechniqueAuthority {
                                    content_sha256: reference.content_sha256,
                                    evidence_ids: reference.evidence_ids,
                                },
                            )
                            .is_some()
                        {
                            anyhow::bail!("report_technique_handoff_ref_duplicate");
                        }
                    }
                    CanonicalFactKey::TechniqueOutcomeSet {
                        organization_id: ref_org,
                        run_id,
                        stage,
                        terminal_cell_count,
                        outcome_set_sha256,
                    } => {
                        contains_technique_ref = true;
                        if *ref_org != *organization_id
                            || reference.organization_id != *organization_id
                            || run_id != &operation_id.to_string()
                            || stage != "vuln_triage"
                            || *terminal_cell_count == 0
                            || decode_sha256(outcome_set_sha256).is_err()
                            || decode_sha256(&reference.content_sha256).is_err()
                            || reference.evidence_ids.is_empty()
                            || reference
                                .evidence_ids
                                .iter()
                                .any(|evidence_id| !handoff_evidence.contains(evidence_id))
                        {
                            anyhow::bail!("report_technique_outcome_set_authority_invalid");
                        }
                        let freshness_floor =
                            sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
                                r#"SELECT unit.started_at
                                 FROM stage_run_units AS unit
                                WHERE unit.id=$1 AND unit.operation_id=$2
                                  AND unit.stage_execution_id=$3
                                  AND unit.organization_id=$4
                                  AND unit.stage_kind='vuln_triage'
                                  AND unit.status='passed' AND unit.started_at IS NOT NULL"#,
                            )
                            .bind(handoff.source_stage_run_unit_id)
                            .bind(operation_id)
                            .bind(handoff.stage_execution_id)
                            .bind(organization_id)
                            .fetch_optional(&mut *connection)
                            .await?
                            .ok_or_else(|| {
                                anyhow::anyhow!("report_technique_outcome_set_lineage_invalid")
                            })?;
                        sealed_sets.push(TechniqueOutcomeSetAuthority {
                            reference,
                            freshness_floor,
                            gate_passed_at: handoff.gate_passed_at,
                            handoff_evidence: handoff_evidence.clone(),
                        });
                    }
                    _ => {}
                }
            }
            if contains_technique_ref || is_terminal_authority_handoff {
                handoff_sources.push(ReportSourceVersion {
                    kind: ReportSourceKind::StageHandoff,
                    authority_class: ReportAuthorityClass::MethodAuditOnly,
                    id: CanonicalRowId::Uuid(handoff.id),
                    row_version: 0,
                    content_hash: decode_sha256(&sha256(&handoff_content))?,
                });
            }
        }
    }

    let rows = sqlx::query_as::<_, TechniqueSourceRow>(
        r#"SELECT outcome.id,outcome.organization_id,outcome.run_id,outcome.asset,
                  outcome.technique,outcome.outcome,outcome.source,
                  outcome.collected_at,outcome.updated_at,
                  outcome.row_version,to_jsonb(outcome.*) AS content,outcome.evidence_ids
             FROM technique_outcomes AS outcome
            WHERE outcome.run_id=ANY($1) AND outcome.organization_id=ANY($2)
            ORDER BY outcome.organization_id,outcome.asset,outcome.technique"#,
    )
    .bind(&authorized_run_ids)
    .bind(organization_ids.iter().copied().collect::<Vec<_>>())
    .fetch_all(&mut *connection)
    .await?;

    let mut rows_by_key = BTreeMap::new();
    for row in rows {
        let key = TechniqueAuthorityKey {
            organization_id: row.organization_id,
            run_id: row.run_id.clone(),
            asset: row.asset.clone(),
            technique: row.technique.clone(),
        };
        if rows_by_key.insert(key, row).is_some() {
            anyhow::bail!("report_technique_outcome_duplicate");
        }
    }
    for sealed_set in sealed_sets {
        let CanonicalFactKey::TechniqueOutcomeSet {
            organization_id,
            run_id,
            stage,
            terminal_cell_count,
            outcome_set_sha256,
        } = &sealed_set.reference.key
        else {
            unreachable!("stored authority is an outcome set")
        };
        let members = rows_by_key
            .values()
            .filter_map(|row| {
                let observed_at = row.collected_at?;
                (row.organization_id == *organization_id
                    && row.run_id == *run_id
                    && observed_at >= sealed_set.freshness_floor
                    && observed_at <= sealed_set.gate_passed_at
                    && row.updated_at <= sealed_set.gate_passed_at)
                    .then(
                        || golish_db::repo::canonical_fact_refs::TechniqueOutcomeSetMember {
                            organization_id: row.organization_id,
                            run_id: row.run_id.clone(),
                            asset: row.asset.clone(),
                            technique: row.technique.clone(),
                            outcome: row.outcome.clone(),
                            observed_at,
                            evidence_ids: row.evidence_ids.clone(),
                            content: row.content.clone(),
                        },
                    )
            })
            .collect::<Vec<_>>();
        let attestation =
            golish_db::repo::canonical_fact_refs::technique_outcome_set_attestation_at(
                stage,
                *organization_id,
                run_id,
                &members,
                Some(sealed_set.gate_passed_at),
            )
            .map_err(|_| anyhow::anyhow!("report_technique_outcome_set_changed"))?;
        let mut ref_evidence_ids = sealed_set.reference.evidence_ids.clone();
        ref_evidence_ids.sort_unstable();
        let original_ref_evidence_count = ref_evidence_ids.len();
        ref_evidence_ids.dedup();
        if attestation.terminal_cell_count != *terminal_cell_count
            || attestation.outcome_set_sha256 != *outcome_set_sha256
            || attestation.content_sha256 != sealed_set.reference.content_sha256
            || attestation.observed_at != sealed_set.reference.observed_at
            || attestation.evidence_ids != ref_evidence_ids
            || ref_evidence_ids.len() != original_ref_evidence_count
            || ref_evidence_ids
                .iter()
                .any(|evidence_id| !sealed_set.handoff_evidence.contains(evidence_id))
        {
            anyhow::bail!("report_technique_outcome_set_changed");
        }
        for member in members {
            let key = TechniqueAuthorityKey {
                organization_id: member.organization_id,
                run_id: member.run_id,
                asset: member.asset,
                technique: member.technique,
            };
            if sealed_refs
                .insert(
                    key,
                    TechniqueAuthority {
                        content_sha256: sha256(&member.content),
                        evidence_ids: member.evidence_ids,
                    },
                )
                .is_some()
            {
                anyhow::bail!("report_technique_handoff_ref_duplicate");
            }
        }
    }
    for (key, row) in &rows_by_key {
        if sealed_refs.contains_key(key)
            || key.technique != golish_db::repo::coverage_truth::TECH_EAS_LIVENESS
            || row.source.as_deref() != Some("eas_probe_http_liveness")
            || row.outcome != "found"
        {
            continue;
        }
        let matching = legacy_eas_liveness_authorities
            .iter()
            .filter(|authority| {
                authority.organization_id == key.organization_id
                    && authority.run_id == key.run_id
                    && authority.asset_evidence.contains_key(&key.asset)
            })
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            anyhow::bail!("report_eas_liveness_handoff_authority_duplicate");
        }
        let Some(authority) = matching.first().copied() else {
            continue;
        };
        let claim_evidence = authority
            .asset_evidence
            .get(&key.asset)
            .expect("matching authority has the liveness asset");
        let row_evidence = row.evidence_ids.iter().copied().collect::<BTreeSet<_>>();
        let row_is_frozen_in_handoff = row.collected_at.as_ref().is_some_and(|collected_at| {
            *collected_at >= authority.freshness_floor && *collected_at <= authority.gate_passed_at
        }) && row.updated_at <= authority.gate_passed_at
            && !row_evidence.is_empty()
            && row_evidence.len() == row.evidence_ids.len()
            && &row_evidence == claim_evidence;
        if !row_is_frozen_in_handoff {
            anyhow::bail!("report_eas_liveness_handoff_authority_invalid");
        }
        let evidence_count: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*)
                 FROM audit_log AS evidence
                 JOIN stage_worker_runs AS worker
                   ON worker.id::text=(evidence.detail #>> '{tool_truth_producer,worker_run_id}')
                  AND worker.operation_id=$2
                  AND worker.stage_execution_id=$9
                  AND worker.stage_run_unit_id=$10
                  AND worker.organization_id=$3
                  AND worker.status='passed'
                 JOIN tool_calls AS tool
                   ON tool.id::text=(evidence.detail #>> '{tool_truth_producer,source_tool_call_id}')
                  AND tool.worker_run_id=worker.id
                  AND tool.operation_id=worker.operation_id
                  AND tool.stage_execution_id=worker.stage_execution_id
                  AND tool.stage_run_unit_id=worker.stage_run_unit_id
                  AND tool.organization_id=worker.organization_id
                  AND tool.name='eas_probe_http_liveness'
                  AND tool.status='finished'
                WHERE evidence.id=ANY($1) AND evidence.audit_role='evidence'
                  AND evidence.run_id=$2
                  AND evidence.detail->>'organization_id'=$3::text
                  AND evidence.session_id=$4
                  AND evidence.evidence_asset=$5
                  AND evidence.evidence_technique=$6
                  AND evidence.evidence_outcome=$7
                  AND evidence.created_at BETWEEN $8 AND $11
                  AND evidence.detail #>> '{tool_truth_producer,stage_execution_id}'=$9::text
                  AND evidence.detail #>> '{tool_truth_producer,stage_run_unit_id}'=$10::text
                  AND evidence.detail #>> '{tool_truth_producer,organization_id}'=$3::text
                  AND evidence.detail #>> '{tool_truth_producer,producer_tool_name}'='eas_probe_http_liveness'"#,
        )
        .bind(&row.evidence_ids)
        .bind(operation_id)
        .bind(row.organization_id)
        .bind(&row.run_id)
        .bind(&row.asset)
        .bind(&row.technique)
        .bind(&row.outcome)
        .bind(authority.freshness_floor)
        .bind(authority.stage_execution_id)
        .bind(authority.stage_run_unit_id)
        .bind(authority.gate_passed_at)
        .fetch_one(&mut *connection)
        .await?;
        if usize::try_from(evidence_count).ok() != Some(row.evidence_ids.len()) {
            anyhow::bail!("report_eas_liveness_evidence_unresolvable");
        }
        sealed_refs.insert(
            key.clone(),
            TechniqueAuthority {
                content_sha256: sha256(&row.content),
                evidence_ids: row.evidence_ids.clone(),
            },
        );
    }
    for (key, row) in &rows_by_key {
        if sealed_refs.contains_key(key)
            || key.technique != golish_db::repo::coverage_truth::TECH_EAS_WEB_FP
            || row.source.as_deref() != Some("eas_fingerprint_web_stack")
            || row.outcome != "found"
        {
            continue;
        }
        let row_evidence = row.evidence_ids.iter().copied().collect::<BTreeSet<_>>();
        let matching = legacy_eas_web_fingerprint_authorities
            .iter()
            .filter(|authority| {
                authority.organization_id == key.organization_id
                    && authority.run_id == key.run_id
                    && exact_legacy_eas_web_fingerprint_evidence_membership(
                        &authority.claim_evidence_sets,
                        &authority.handoff_evidence,
                        &row_evidence,
                    )
            })
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            anyhow::bail!("report_eas_web_fingerprint_handoff_authority_duplicate");
        }
        let Some(authority) = matching.first().copied() else {
            continue;
        };
        let row_is_frozen_in_handoff = row
            .collected_at
            .as_ref()
            .is_some_and(|collected_at| *collected_at <= authority.gate_passed_at)
            && row.updated_at <= authority.gate_passed_at
            && !row_evidence.is_empty()
            && row_evidence.len() == row.evidence_ids.len();
        if !row_is_frozen_in_handoff {
            anyhow::bail!("report_eas_web_fingerprint_handoff_authority_invalid");
        }
        let evidence_count: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*)
                 FROM audit_log AS evidence
                 JOIN stage_run_units AS source_unit
                   ON source_unit.id::text=(evidence.detail #>> '{tool_truth_producer,stage_run_unit_id}')
                  AND source_unit.operation_id=$2
                  AND source_unit.stage_execution_id::text=(evidence.detail #>> '{tool_truth_producer,stage_execution_id}')
                  AND source_unit.organization_id=$3
                  AND source_unit.stage_kind='external_attack_surface'
                  AND source_unit.status IN ('passed','superseded')
                  AND source_unit.started_at <= evidence.created_at
                  AND source_unit.terminal_at IS NOT NULL
                  AND source_unit.terminal_at <= $8
                 JOIN stage_worker_runs AS worker
                   ON worker.id::text=(evidence.detail #>> '{tool_truth_producer,worker_run_id}')
                  AND worker.operation_id=$2
                 AND worker.stage_execution_id=source_unit.stage_execution_id
                 AND worker.stage_run_unit_id=source_unit.id
                 AND worker.organization_id=$3
                  AND ((source_unit.status='passed' AND worker.status='passed')
                    OR (source_unit.status='superseded' AND worker.status='superseded'))
                 JOIN tool_calls AS tool
                   ON tool.id::text=(evidence.detail #>> '{tool_truth_producer,source_tool_call_id}')
                  AND tool.worker_run_id=worker.id
                  AND tool.operation_id=worker.operation_id
                  AND tool.stage_execution_id=worker.stage_execution_id
                  AND tool.stage_run_unit_id=worker.stage_run_unit_id
                  AND tool.organization_id=worker.organization_id
                  AND tool.name='eas_fingerprint_web_stack'
                  AND tool.status='finished'
                WHERE evidence.id=ANY($1) AND evidence.audit_role='evidence'
                  AND evidence.run_id=$2
                  AND evidence.detail->>'organization_id'=$3::text
                  AND evidence.session_id=$4
                  AND evidence.evidence_asset=$5
                  AND evidence.evidence_technique=$6
                  AND evidence.evidence_outcome=$7
                  AND evidence.created_at <= $8
                  AND evidence.detail #>> '{tool_truth_producer,organization_id}'=$3::text
                  AND evidence.detail #>> '{tool_truth_producer,producer_tool_name}'='eas_fingerprint_web_stack'"#,
        )
        .bind(&row.evidence_ids)
        .bind(operation_id)
        .bind(row.organization_id)
        .bind(&row.run_id)
        .bind(&row.asset)
        .bind(&row.technique)
        .bind(&row.outcome)
        .bind(authority.gate_passed_at)
        .fetch_one(&mut *connection)
        .await?;
        if usize::try_from(evidence_count).ok() != Some(row.evidence_ids.len()) {
            anyhow::bail!("report_eas_web_fingerprint_evidence_unresolvable");
        }
        sealed_refs.insert(
            key.clone(),
            TechniqueAuthority {
                content_sha256: sha256(&row.content),
                evidence_ids: row.evidence_ids.clone(),
            },
        );
    }
    for key in sealed_refs.keys() {
        if !rows_by_key.contains_key(key) {
            anyhow::bail!("report_technique_handoff_row_missing");
        }
    }
    for key in rows_by_key.keys() {
        if !sealed_refs.contains_key(key) {
            anyhow::bail!("report_technique_outcome_unsealed");
        }
    }

    let mut sources = handoff_sources;
    sources.reserve(rows_by_key.len());
    for (key, row) in rows_by_key {
        let current_hash = sha256(&row.content);
        let authority = sealed_refs
            .get(&key)
            .ok_or_else(|| anyhow::anyhow!("report_technique_outcome_unsealed"))?;
        if authority.content_sha256 != current_hash || authority.evidence_ids != row.evidence_ids {
            anyhow::bail!("report_technique_outcome_source_changed");
        }
        if !technique_outcome_evidence_shape_is_reportable(&row.outcome, &row.evidence_ids) {
            anyhow::bail!("report_technique_evidence_missing");
        }
        let evidence_count: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*) FROM audit_log
                WHERE id=ANY($1) AND audit_role='evidence' AND run_id=$2
                  AND detail->>'organization_id'=$3"#,
        )
        .bind(&row.evidence_ids)
        .bind(operation_id)
        .bind(row.organization_id.to_string())
        .fetch_one(&mut *connection)
        .await?;
        if usize::try_from(evidence_count).ok() != Some(row.evidence_ids.len()) {
            anyhow::bail!("report_technique_evidence_unresolvable");
        }
        sources.push(ReportSourceVersion {
            kind: ReportSourceKind::TechniqueOutcome,
            authority_class: ReportAuthorityClass::ExecutionObservationAudit,
            id: CanonicalRowId::Int64(row.id),
            row_version: row.row_version,
            content_hash: decode_sha256(&current_hash)?,
        });
    }
    Ok(sources)
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct UuidSourceRow {
    id: Uuid,
    row_version: i64,
    content: Value,
}

async fn uuid_sources(
    connection: &mut PgConnection,
    operation_id: Uuid,
    kind: ReportSourceKind,
    sql: &str,
) -> anyhow::Result<Vec<ReportSourceVersion>> {
    uuid_sources_with_authority(
        connection,
        operation_id,
        kind,
        ReportAuthorityClass::MethodAuditOnly,
        sql,
    )
    .await
}

async fn uuid_sources_for_topology(
    connection: &mut PgConnection,
    operation_id: Uuid,
    topology: StageTopologyContract,
    kind: ReportSourceKind,
    sql: &str,
) -> anyhow::Result<Vec<ReportSourceVersion>> {
    let rows = sqlx::query_as::<_, UuidSourceRow>(sql)
        .bind(operation_id)
        .bind(topology.as_str())
        .fetch_all(&mut *connection)
        .await?;
    rows.into_iter()
        .map(|row| {
            Ok(ReportSourceVersion {
                kind,
                authority_class: ReportAuthorityClass::MethodAuditOnly,
                id: CanonicalRowId::Uuid(row.id),
                row_version: row.row_version,
                content_hash: decode_sha256(&sha256(&row.content))?,
            })
        })
        .collect()
}

async fn uuid_sources_with_authority(
    connection: &mut PgConnection,
    operation_id: Uuid,
    kind: ReportSourceKind,
    authority_class: ReportAuthorityClass,
    sql: &str,
) -> anyhow::Result<Vec<ReportSourceVersion>> {
    let rows = sqlx::query_as::<_, UuidSourceRow>(sql)
        .bind(operation_id)
        .fetch_all(&mut *connection)
        .await?;
    rows.into_iter()
        .map(|row| {
            Ok(ReportSourceVersion {
                kind,
                authority_class,
                id: CanonicalRowId::Uuid(row.id),
                row_version: row.row_version,
                content_hash: decode_sha256(&sha256(&row.content))?,
            })
        })
        .collect()
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct CleanupBlockedDecisionSourceRow {
    id: Uuid,
    obligation_id: Uuid,
    organization_id_at_time: Uuid,
    decided_by_principal_id: Uuid,
    reason: String,
    residual_risk: Value,
    content: Value,
    evidence_ids: Vec<i64>,
    decision_evidence_ids: Vec<i64>,
}

async fn report_cleanup_blocked_decision_truth_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
) -> anyhow::Result<(
    Vec<ReportSourceVersion>,
    BTreeMap<Uuid, CleanupBlockedDecisionTruth>,
)> {
    let rows = sqlx::query_as::<_, CleanupBlockedDecisionSourceRow>(
        r#"SELECT decision.id,decision.obligation_id,
                  decision.organization_id_at_time,decision.decided_by_principal_id,
                  decision.reason,decision.residual_risk,
                  jsonb_build_object(
                      'row',to_jsonb(decision),
                      'evidence',COALESCE((
                          SELECT jsonb_agg(
                              jsonb_build_object(
                                  'evidenceId',link.evidence_id,'role',link.role
                              ) ORDER BY link.evidence_id,link.role
                          ) FROM cleanup_blocked_decision_evidence AS link
                           WHERE link.blocked_decision_id=decision.id
                      ),'[]'::jsonb)
                  ) AS content,
                  COALESCE((
                      SELECT array_agg(DISTINCT link.evidence_id ORDER BY link.evidence_id)
                        FROM cleanup_blocked_decision_evidence AS link
                       WHERE link.blocked_decision_id=decision.id
                  ),'{}'::bigint[]) AS evidence_ids,
                  COALESCE((
                      SELECT array_agg(link.evidence_id ORDER BY link.evidence_id)
                        FROM cleanup_blocked_decision_evidence AS link
                       WHERE link.blocked_decision_id=decision.id
                         AND link.role='decision'
                  ),'{}'::bigint[]) AS decision_evidence_ids
             FROM cleanup_blocked_decisions AS decision
            WHERE decision.operation_id=$1
            ORDER BY decision.id"#,
    )
    .bind(operation_id)
    .fetch_all(&mut *connection)
    .await?;
    let mut sources = Vec::with_capacity(rows.len());
    let mut truth = BTreeMap::new();
    for row in rows {
        let source = ReportSourceVersion {
            kind: ReportSourceKind::CleanupBlockedDecision,
            authority_class: ReportAuthorityClass::MethodAuditOnly,
            id: CanonicalRowId::Uuid(row.id),
            row_version: 0,
            content_hash: decode_sha256(&sha256(&row.content))?,
        };
        sources.push(source.clone());
        let decision = CleanupBlockedDecisionTruth {
            decision_id: row.id,
            obligation_id: row.obligation_id,
            organization_id_at_time: row.organization_id_at_time,
            decided_by_principal_id: row.decided_by_principal_id,
            reason: row.reason,
            residual_risk: row.residual_risk,
            evidence_ids: row.evidence_ids.into_iter().collect(),
            decision_evidence_ids: row.decision_evidence_ids.into_iter().collect(),
            source,
        };
        if truth.insert(row.id, decision).is_some() {
            anyhow::bail!("report_cleanup_blocked_decision_duplicate");
        }
    }
    Ok((sources, truth))
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct EvidenceAuditSourceRow {
    evidence_audit_id: i64,
    referenced_organization_ids: Vec<Uuid>,
    run_id: Option<Uuid>,
    audit_role: Option<String>,
    organization_id_value: Option<String>,
    content: Option<Value>,
}

async fn report_evidence_audit_truth_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    topology: StageTopologyContract,
) -> anyhow::Result<(Vec<ReportSourceVersion>, BTreeMap<i64, EvidenceAuditTruth>)> {
    let rows = sqlx::query_as::<_, EvidenceAuditSourceRow>(
        r#"WITH evidence_refs(evidence_id,organization_id) AS (
               SELECT episode_evidence.evidence_id,episode.organization_id_at_time
                 FROM stage_episodes AS episode
                 CROSS JOIN LATERAL unnest(episode.evidence_refs)
                   AS episode_evidence(evidence_id)
                WHERE episode.source_operation_id=$1
                  AND operation_stage_rank_for_topology($2,episode.stage_kind) IS NOT NULL
               UNION ALL
               SELECT link.evidence_id,attempt.organization_id
                 FROM candidate_attempts AS attempt
                 JOIN candidate_attempt_evidence AS link ON link.attempt_id=attempt.id
                WHERE attempt.operation_id=$1
                  AND $2='legacy_candidate_verification_v1'
               UNION ALL
               SELECT outcome_evidence.evidence_id,outcome.organization_id
                 FROM technique_outcomes AS outcome
                 CROSS JOIN LATERAL unnest(outcome.evidence_ids)
                   AS outcome_evidence(evidence_id)
                WHERE outcome.run_id=$1::text
               UNION ALL
               SELECT link.evidence_id,action.organization_id_at_time
                 FROM post_exploit_actions AS action
                 JOIN post_exploit_action_evidence AS link ON link.action_id=action.id
                WHERE action.operation_id=$1
                  AND $2='legacy_candidate_verification_v1'
               UNION ALL
               SELECT link.evidence_id,foothold.organization_id_at_time
                 FROM footholds AS foothold
                 JOIN foothold_evidence AS link ON link.foothold_id=foothold.id
                WHERE foothold.operation_id=$1
                  AND $2='legacy_candidate_verification_v1'
               UNION ALL
               SELECT link.evidence_id,observation.organization_id_at_time
                 FROM internal_asset_observations AS observation
                 JOIN internal_asset_observation_evidence AS link
                   ON link.observation_id=observation.id
                WHERE observation.operation_id=$1
                  AND $2='legacy_candidate_verification_v1'
               UNION ALL
               SELECT link.evidence_id,path.organization_id_at_time
                 FROM attack_paths AS path
                 JOIN attack_path_edges AS edge ON edge.attack_path_id=path.id
                 JOIN attack_path_edge_evidence AS link ON link.attack_path_edge_id=edge.id
                WHERE path.operation_id=$1
                  AND $2='legacy_candidate_verification_v1'
               UNION ALL
               SELECT link.evidence_id,objective.organization_id_at_time
                 FROM objective_attempts AS objective
                 JOIN objective_attempt_evidence AS link ON link.objective_attempt_id=objective.id
                WHERE objective.operation_id=$1
                  AND $2='legacy_candidate_verification_v1'
               UNION ALL
               SELECT link.evidence_id,obligation.organization_id_at_time
                 FROM cleanup_obligations AS obligation
                 JOIN cleanup_obligation_evidence AS link ON link.obligation_id=obligation.id
                WHERE obligation.operation_id=$1
               UNION ALL
               SELECT link.evidence_id,waiver.organization_id_at_time
                 FROM cleanup_waivers AS waiver
                 JOIN cleanup_waiver_evidence AS link ON link.waiver_id=waiver.id
                WHERE waiver.operation_id=$1
               UNION ALL
               SELECT link.evidence_id,decision.organization_id_at_time
                 FROM cleanup_blocked_decisions AS decision
                 JOIN cleanup_blocked_decision_evidence AS link
                   ON link.blocked_decision_id=decision.id
                WHERE decision.operation_id=$1
           ), referenced AS (
               SELECT evidence_id,
                      array_agg(DISTINCT organization_id ORDER BY organization_id)
                          AS referenced_organization_ids
                 FROM evidence_refs
                GROUP BY evidence_id
           )
           SELECT referenced.evidence_id AS evidence_audit_id,
                  referenced.referenced_organization_ids,
                  evidence.run_id,evidence.audit_role,
                  evidence.detail->>'organization_id' AS organization_id_value,
                  CASE WHEN evidence.id IS NULL THEN NULL ELSE to_jsonb(evidence) END AS content
             FROM referenced
             LEFT JOIN audit_log AS evidence ON evidence.id=referenced.evidence_id
            ORDER BY referenced.evidence_id"#,
    )
    .bind(operation_id)
    .bind(topology.as_str())
    .fetch_all(&mut *connection)
    .await?;

    let mut sources = Vec::new();
    let mut truth = BTreeMap::new();
    for row in rows {
        let source = if let Some(content) = row.content.as_ref() {
            Some(ReportSourceVersion {
                kind: ReportSourceKind::EvidenceAudit,
                authority_class: ReportAuthorityClass::MethodAuditOnly,
                id: CanonicalRowId::Int64(row.evidence_audit_id),
                row_version: 0,
                content_hash: decode_sha256(&sha256(content))?,
            })
        } else {
            None
        };
        if let Some(source) = &source {
            sources.push(source.clone());
        }
        let organization_id_at_time = row
            .organization_id_value
            .as_deref()
            .and_then(|value| Uuid::parse_str(value).ok())
            .or(match row.referenced_organization_ids.as_slice() {
                [organization_id] => Some(*organization_id),
                _ => None,
            });
        truth.insert(
            row.evidence_audit_id,
            EvidenceAuditTruth {
                evidence_audit_id: row.evidence_audit_id,
                run_id: row.run_id,
                organization_id_at_time,
                audit_role: row.audit_role,
                referenced_organization_ids: row.referenced_organization_ids.into_iter().collect(),
                source,
            },
        );
    }
    Ok((sources, truth))
}

/// Re-run the complete canonical Reporting source query. This query is shared
/// by build/validate/finalize adapters; it never trusts a prior manifest and
/// therefore detects newly inserted, changed, deleted, invalidated, or
/// previously unsealed sources.
pub async fn current_reportable_source_snapshot(
    pool: &PgPool,
    operation_id: Uuid,
) -> anyhow::Result<ReportSourceSnapshot> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let snapshot = current_reportable_source_snapshot_on(&mut tx, operation_id).await?;
    tx.commit().await?;
    Ok(snapshot)
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct InvestigationClosurePublicationSourceRow {
    id: Uuid,
    row_version: i64,
    content: Value,
    publication_id: Uuid,
    closure_id: Uuid,
    authority_id: Uuid,
    stage_execution_id: Uuid,
    owning_stage_run_request_id: String,
    scope_snapshot_id: Uuid,
    closure_sha256: String,
    disposition: String,
    member_count: i64,
    member_set_sha256: String,
    publication_sha256: String,
    expected_publication_sha256: String,
    residual_member_count: i64,
    residual_member_set_sha256: String,
    contract_version: String,
    fixed_point_receipt_id: Uuid,
    fixed_point_receipt_sha256: String,
    run_state: String,
    admission_open: bool,
    execution_status: String,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct InvestigationClosurePublicationMemberSourceRow {
    id: Uuid,
    row_version: i64,
    content: Value,
    publication_id: Uuid,
    member_ordinal: i32,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    scope_snapshot_id: Uuid,
    stage_run_unit_id: Uuid,
    organization_id: Uuid,
    member_sha256: String,
    expected_member_sha256: String,
    passed_at: chrono::DateTime<chrono::Utc>,
    terminal_at: Option<chrono::DateTime<chrono::Utc>>,
    unit_status: String,
    pass_watermark: Value,
    plan_requests_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    completion_stage_run_id: Option<String>,
    completion_passed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct InvestigationClosureResidualSourceRow {
    id: Uuid,
    row_version: i64,
    content: Value,
    member_key: String,
    member_hash: String,
    organization_id: Uuid,
    reason_code: String,
    affected_input_ids: Vec<String>,
    owner_code: String,
    next_action_code: String,
}

async fn investigation_closure_residual_rows_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    scope_snapshot_id: Uuid,
) -> anyhow::Result<Vec<InvestigationClosureResidualSourceRow>> {
    Ok(sqlx::query_as::<_, InvestigationClosureResidualSourceRow>(
        r#"WITH residual_members AS (
               SELECT residual.residual_id AS id,
                      'risk:' || residual.residual_id::TEXT AS member_key,
                      residual.residual_hash AS member_hash,
                      residual.organization_id,
                      residual.reason_code,
                      COALESCE((SELECT array_agg(value ORDER BY ordinal)
                                  FROM jsonb_array_elements_text(residual.affected_inputs)
                                       WITH ORDINALITY item(value,ordinal)),ARRAY[]::TEXT[])
                          AS affected_input_ids,
                      residual.owner_kind AS owner_code,
                      COALESCE(NULLIF(residual.next_action->>'code',''),
                               NULLIF(residual.next_action->>'kind',''),'follow_up_required')
                          AS next_action_code,
                      to_jsonb(residual) AS source_row
                FROM hypothesis_residual_risks residual
                WHERE residual.operation_id=$1 AND residual.closed_at IS NULL
                  AND unified_investigation_residual_has_stage_authority_v1(
                          residual.residual_id,$1,$2,$3
                      )
               UNION ALL
               SELECT member.admission_member_id,
                      'admission:' || member.admission_member_id::TEXT,
                      member.member_sha256,member.organization_id,member.reason_code,
                      ARRAY[member.hypothesis_revision_id::TEXT],
                      'verification_admission',
                      'resolve_admission_' || member.disposition,
                      to_jsonb(member)
                 FROM verification_admission_members member
                WHERE member.operation_id=$1 AND member.stage_execution_id=$2
                  AND member.scope_snapshot_id=$3
                  AND member.disposition IN('needs_enrichment','deferred','out_of_scope','unsafe')
               UNION ALL
               SELECT member.assignment_member_id,
                      'assignment:' || member.assignment_member_id::TEXT,
                      member.residual_receipt_sha256,task.organization_id,
                      member.residual_reason_code,
                      ARRAY[member.verification_objective_id::TEXT],
                      member.residual_owner,member.residual_next_action,to_jsonb(member)
                 FROM hypothesis_verification_task_assignment_members member
                 JOIN hypothesis_verification_tasks task ON task.task_id=member.task_id
                WHERE task.operation_id=$1 AND task.stage_execution_id=$2
                  AND task.scope_snapshot_id=$3 AND member.assignment_kind='residual'
               UNION ALL
               SELECT cycle.semantic_cycle_receipt_id,
                      'cycle:' || cycle.semantic_cycle_receipt_id::TEXT,
                      cycle.receipt_sha256,cycle.organization_id,
                      COALESCE(cycle.residual_reason_code,'investigation_stopped'),
                      ARRAY[cycle.task_id::TEXT],'investigation_scheduler',
                      CASE WHEN cycle.disposition='stopped' THEN 'resume_after_stop'
                           ELSE 'resolve_semantic_cycle' END,
                      to_jsonb(cycle)
                 FROM investigation_semantic_cycle_receipts cycle
                WHERE cycle.operation_id=$1 AND cycle.stage_execution_id=$2
                  AND cycle.scope_snapshot_id=$3
                  AND cycle.disposition IN('residual','stopped')
           )
           SELECT id,0::BIGINT AS row_version,
                  jsonb_build_object(
                      'contractVersion','investigation-closure-residual.v1',
                      'memberKey',member_key,'memberHash',member_hash,
                      'organizationId',organization_id,'reasonCode',reason_code,
                      'affectedInputIds',affected_input_ids,'ownerCode',owner_code,
                      'nextActionCode',next_action_code,'sourceRow',source_row
                  ) AS content,
                  member_key,member_hash,organization_id,reason_code,
                  affected_input_ids,owner_code,next_action_code
             FROM residual_members ORDER BY member_key"#,
    )
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(scope_snapshot_id)
    .fetch_all(&mut *connection)
    .await?)
}

/// Load and validate the closure authority inside the caller's Reporting
/// REPEATABLE READ snapshot. This intentionally does not use the pool-based
/// runtime repository: publication, members, residuals, and report sources
/// must be one PostgreSQL snapshot or the manifest can mix two realities.
async fn investigation_closure_publication_sources_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    organization_ids: &BTreeSet<Uuid>,
) -> anyhow::Result<Vec<ReportSourceVersion>> {
    let headers = sqlx::query_as::<_, InvestigationClosurePublicationSourceRow>(
        r#"SELECT publication.publication_id AS id,0::BIGINT AS row_version,
                  jsonb_build_object(
                      'contractVersion','investigation-closure-report-authority.v1',
                      'publication',to_jsonb(publication),
                      'closureHeader',to_jsonb(closure_header),
                      'closureAuthority',to_jsonb(closure_authority),
                      'fixedPointReceipt',to_jsonb(fixed_point)
                  ) AS content,
                  publication.publication_id,publication.closure_id,
                  publication.authority_id,publication.stage_execution_id,
                  publication.owning_stage_run_request_id,publication.scope_snapshot_id,
                  publication.closure_sha256,publication.disposition,
                  publication.member_count,publication.member_set_sha256,
                  publication.publication_sha256,
                  tool_truth_sha256(jsonb_build_object(
                      'contract_version','investigation-stage-closure-publication.v1',
                      'publication_id',publication.publication_id,
                      'closure_id',publication.closure_id,
                      'closure_sha256',publication.closure_sha256,
                      'authority_id',publication.authority_id,
                      'operation_id',publication.operation_id,
                      'stage_execution_id',publication.stage_execution_id,
                      'owning_stage_run_request_id',publication.owning_stage_run_request_id,
                      'scope_snapshot_id',publication.scope_snapshot_id,
                      'disposition',publication.disposition,
                      'member_count',publication.member_count,
                      'member_set_sha256',publication.member_set_sha256
                  )::TEXT) AS expected_publication_sha256,
                  closure_authority.residual_member_count,
                  closure_authority.residual_member_set_sha256,
                  closure_authority.contract_version,
                  closure_authority.fixed_point_receipt_id,
                  closure_authority.fixed_point_receipt_sha256,
                  head.run_state,head.admission_open,execution.status AS execution_status
             FROM investigation_stage_closure_publications publication
             JOIN investigation_run_closures closure_header
               ON closure_header.closure_id=publication.closure_id
              AND closure_header.authority_id=publication.authority_id
             JOIN investigation_run_closure_v1_authorities closure_authority
               ON closure_authority.closure_id=publication.closure_id
              AND closure_authority.authority_id=publication.authority_id
             JOIN investigation_stage_fixed_point_receipts fixed_point
               ON fixed_point.fixed_point_receipt_id=closure_authority.fixed_point_receipt_id
              AND fixed_point.authority_id=publication.authority_id
             JOIN investigation_run_heads head
               ON head.authority_id=publication.authority_id
              AND head.operation_id=publication.operation_id
              AND head.stage_execution_id=publication.stage_execution_id
              AND head.owning_stage_run_request_id=publication.owning_stage_run_request_id
              AND head.scope_snapshot_id=publication.scope_snapshot_id
             JOIN stage_runs execution
               ON execution.id=publication.stage_execution_id
              AND execution.operation_id=publication.operation_id
              AND execution.stage_kind='investigation'
            WHERE publication.operation_id=$1
            ORDER BY publication.published_at,publication.publication_id"#,
    )
    .bind(operation_id)
    .fetch_all(&mut *connection)
    .await?;
    let header = match headers.as_slice() {
        [header] => header,
        [] => anyhow::bail!("report_investigation_closure_publication_missing"),
        _ => anyhow::bail!("report_investigation_closure_publication_not_unique"),
    };
    if header.publication_sha256 != header.expected_publication_sha256
        || header.contract_version != "investigation_run_closure.v1"
        || header.closure_sha256
            != header
                .content
                .pointer("/closureAuthority/closure_sha256")
                .or_else(|| header.content.pointer("/closureAuthority/closureSha256"))
                .and_then(Value::as_str)
                .unwrap_or_default()
        || header.closure_sha256
            != header
                .content
                .pointer("/closureHeader/closure_sha256")
                .and_then(Value::as_str)
                .unwrap_or_default()
        || header.fixed_point_receipt_sha256
            != header
                .content
                .pointer("/fixedPointReceipt/receipt_sha256")
                .and_then(Value::as_str)
                .unwrap_or_default()
        || header.authority_id.is_nil()
        || header.closure_id.is_nil()
        || header.owning_stage_run_request_id.trim().is_empty()
        || header.owning_stage_run_request_id
            != header
                .content
                .pointer("/closureAuthority/owning_stage_run_request_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
        || header.fixed_point_receipt_id.is_nil()
        || !header.fixed_point_receipt_sha256.starts_with("sha256:")
        || header.run_state != "closed"
        || header.admission_open
        || header.execution_status != "completed"
        || !matches!(header.disposition.as_str(), "pass" | "pass_with_gaps")
    {
        anyhow::bail!("report_investigation_closure_publication_invalid");
    }
    let members = sqlx::query_as::<_, InvestigationClosurePublicationMemberSourceRow>(
        r#"SELECT member.publication_member_id AS id,0::BIGINT AS row_version,
                  jsonb_build_object(
                      'contractVersion','investigation-stage-closure-member.v1',
                      'member',to_jsonb(member)
                  ) AS content,
                  member.publication_id,member.member_ordinal,member.operation_id,
                  member.stage_execution_id,member.scope_snapshot_id,
                  member.stage_run_unit_id,member.organization_id,member.member_sha256,
                  tool_truth_sha256(jsonb_build_object(
                      'contract_version','investigation-stage-closure-member.v1',
                      'closure_id',publication.closure_id,
                      'closure_sha256',publication.closure_sha256,
                      'stage_run_unit_id',member.stage_run_unit_id,
                      'organization_id',member.organization_id,
                      'stage_team_plan_id',member.stage_team_plan_id
                  )::TEXT) AS expected_member_sha256,
                  member.passed_at,unit.terminal_at,unit.status AS unit_status,
                  unit.pass_watermark,plan.requests_closed_at AS plan_requests_closed_at,
                  completion.stage_run_id AS completion_stage_run_id,
                  completion.passed_at AS completion_passed_at
             FROM investigation_stage_closure_publication_members member
             JOIN investigation_stage_closure_publications publication
               ON publication.publication_id=member.publication_id
             JOIN stage_run_units unit ON unit.id=member.stage_run_unit_id
             JOIN stage_team_plans plan ON plan.id=member.stage_team_plan_id
             LEFT JOIN org_stage_completions completion
               ON completion.organization_id=member.organization_id
              AND completion.stage_kind='investigation'
            WHERE member.publication_id=$1 ORDER BY member.member_ordinal"#,
    )
    .bind(header.publication_id)
    .fetch_all(&mut *connection)
    .await?;
    let member_orgs = members
        .iter()
        .map(|member| member.organization_id)
        .collect::<BTreeSet<_>>();
    let operation_id_string = operation_id.to_string();
    let publication_id_string = header.publication_id.to_string();
    let closure_id_string = header.closure_id.to_string();
    if i64::try_from(members.len()).ok() != Some(header.member_count)
        || &member_orgs != organization_ids
        || members.iter().enumerate().any(|(ordinal, member)| {
            member.publication_id != header.publication_id
                || member.member_ordinal != ordinal as i32
                || member.operation_id != operation_id
                || member.stage_execution_id != header.stage_execution_id
                || member.scope_snapshot_id != header.scope_snapshot_id
                || member.member_sha256 != member.expected_member_sha256
                || member.unit_status != "passed"
                || member.terminal_at != Some(member.passed_at)
                || member.plan_requests_closed_at.is_none()
                || member.completion_stage_run_id.as_deref() != Some(operation_id_string.as_str())
                || member.completion_passed_at != Some(member.passed_at)
                || member.pass_watermark.get("schema").and_then(Value::as_str)
                    != Some("investigation_stage_closure_publication.v1")
                || member
                    .pass_watermark
                    .get("publication_id")
                    .and_then(Value::as_str)
                    != Some(publication_id_string.as_str())
                || member
                    .pass_watermark
                    .get("closure_id")
                    .and_then(Value::as_str)
                    != Some(closure_id_string.as_str())
                || member
                    .pass_watermark
                    .get("closure_sha256")
                    .and_then(Value::as_str)
                    != Some(header.closure_sha256.as_str())
                || member
                    .pass_watermark
                    .get("disposition")
                    .and_then(Value::as_str)
                    != Some(header.disposition.as_str())
                || member
                    .pass_watermark
                    .get("member_sha256")
                    .and_then(Value::as_str)
                    != Some(member.member_sha256.as_str())
                || member.stage_run_unit_id.is_nil()
        })
    {
        anyhow::bail!("report_investigation_closure_member_authority_invalid");
    }
    let expected_member_set_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash(
             'investigation_stage_closure_publication_members.v1',$1::TEXT[])",
    )
    .bind(
        members
            .iter()
            .map(|member| member.expected_member_sha256.clone())
            .collect::<Vec<_>>(),
    )
    .fetch_one(&mut *connection)
    .await?;
    if expected_member_set_sha256 != header.member_set_sha256 {
        anyhow::bail!("report_investigation_closure_member_set_mismatch");
    }
    let residuals = investigation_closure_residual_rows_on(
        &mut *connection,
        operation_id,
        header.stage_execution_id,
        header.scope_snapshot_id,
    )
    .await?;
    let expected_residual_set_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash(
             'investigation_run_residuals.v1',$1::TEXT[])",
    )
    .bind(
        residuals
            .iter()
            .map(|member| format!("{}:{}", member.member_key, member.member_hash))
            .collect::<Vec<_>>(),
    )
    .fetch_one(&mut *connection)
    .await?;
    if i64::try_from(residuals.len()).ok() != Some(header.residual_member_count)
        || expected_residual_set_sha256 != header.residual_member_set_sha256
        || (residuals.is_empty() && header.disposition != "pass")
        || (!residuals.is_empty() && header.disposition != "pass_with_gaps")
        || residuals
            .iter()
            .any(|residual| !organization_ids.contains(&residual.organization_id))
    {
        anyhow::bail!("report_investigation_closure_residual_authority_invalid");
    }
    let mut sources = Vec::with_capacity(1 + members.len() + residuals.len());
    sources.push(ReportSourceVersion {
        kind: ReportSourceKind::InvestigationClosurePublication,
        authority_class: ReportAuthorityClass::MethodAuditOnly,
        id: CanonicalRowId::Uuid(header.id),
        row_version: header.row_version,
        content_hash: decode_sha256(&sha256(&header.content))?,
    });
    sources.extend(members.into_iter().map(|member| ReportSourceVersion {
        kind: ReportSourceKind::InvestigationClosurePublicationMember,
        authority_class: ReportAuthorityClass::MethodAuditOnly,
        id: CanonicalRowId::Uuid(member.id),
        row_version: member.row_version,
        content_hash: decode_sha256(&sha256(&member.content)).expect("JSON SHA-256 is canonical"),
    }));
    sources.extend(residuals.into_iter().map(|residual| ReportSourceVersion {
        kind: ReportSourceKind::InvestigationClosureResidual,
        authority_class: ReportAuthorityClass::MethodAuditOnly,
        id: CanonicalRowId::Uuid(residual.id),
        row_version: residual.row_version,
        content_hash: decode_sha256(&sha256(&residual.content)).expect("JSON SHA-256 is canonical"),
    }));
    Ok(sources)
}

pub(super) async fn current_reportable_source_snapshot_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
) -> anyhow::Result<ReportSourceSnapshot> {
    // This is deliberately the first read in the transaction so the recorded
    // identifier describes the exact PostgreSQL snapshot used below.
    let transaction_snapshot: String = sqlx::query_scalar("SELECT txid_current_snapshot()::text")
        .fetch_one(&mut *connection)
        .await?;
    let topology = frozen_reporting_topology_on(&mut *connection, operation_id).await?;
    let organization_ids = frozen_organization_ids_on(&mut *connection, operation_id).await?;
    if organization_ids.is_empty() {
        anyhow::bail!("report_frozen_scope_missing");
    }
    let mut sources = Vec::new();
    if topology == StageTopologyContract::UnifiedInvestigationV1 {
        sources.extend(
            investigation_closure_publication_sources_on(
                &mut *connection,
                operation_id,
                &organization_ids,
            )
            .await?,
        );
    }
    sources.extend(
        uuid_sources_for_topology(
            &mut *connection,
            operation_id,
            topology,
            ReportSourceKind::StageEpisode,
            r#"SELECT episode_id AS id,0::bigint AS row_version,to_jsonb(episode) AS content
                 FROM stage_episodes AS episode
                WHERE source_operation_id=$1
                  AND operation_stage_rank_for_topology($2,stage_kind) IS NOT NULL
                ORDER BY episode_id"#,
        )
        .await?,
    );
    if topology == StageTopologyContract::LegacyCandidateVerificationV1 {
        sources.extend(
            uuid_sources(
                &mut *connection,
                operation_id,
                ReportSourceKind::Finding,
                r#"SELECT finding.id,finding.row_version,
                      jsonb_build_object(
                          'row',to_jsonb(finding),
                          'lineage',to_jsonb(lineage),
                          'evidence',COALESCE((
                              SELECT jsonb_agg(
                                  jsonb_build_object(
                                      'evidenceId',link.evidence_id,'role',link.role
                                  ) ORDER BY link.evidence_id,link.role
                              )
                                FROM candidate_attempt_evidence AS link
                               WHERE link.attempt_id=lineage.candidate_attempt_id
                          ),'[]'::jsonb)
                      ) AS content
                 FROM finding_lineage AS lineage
                 JOIN findings AS finding ON finding.id=lineage.finding_id
                WHERE lineage.operation_id=$1
                ORDER BY finding.id"#,
            )
            .await?,
        );
    }
    sources.extend(
        authoritative_technique_outcome_sources_on(
            &mut *connection,
            operation_id,
            &organization_ids,
        )
        .await?,
    );
    for (kind, query) in [
        (
            ReportSourceKind::CandidateAttempt,
            r#"SELECT source_row.id,source_row.row_version,
                      jsonb_build_object(
                          'row',to_jsonb(source_row) - 'target_live_id',
                          'evidence',COALESCE((
                              SELECT jsonb_agg(
                                  jsonb_build_object(
                                      'evidenceId',link.evidence_id,'role',link.role
                                  ) ORDER BY link.evidence_id,link.role
                              ) FROM candidate_attempt_evidence AS link
                               WHERE link.attempt_id=source_row.id
                          ),'[]'::jsonb)
                      ) AS content
                 FROM candidate_attempts AS source_row
                WHERE source_row.operation_id=$1 ORDER BY source_row.id"#,
        ),
        (
            ReportSourceKind::FindingLineage,
            r#"SELECT source_row.id,source_row.row_version,
                      jsonb_build_object(
                          'row',to_jsonb(source_row),
                          'evidence',COALESCE((
                              SELECT jsonb_agg(
                                  jsonb_build_object(
                                      'evidenceId',link.evidence_id,'role',link.role
                                  ) ORDER BY link.evidence_id,link.role
                              ) FROM candidate_attempt_evidence AS link
                               WHERE link.attempt_id=source_row.candidate_attempt_id
                          ),'[]'::jsonb)
                      ) AS content
                 FROM finding_lineage AS source_row
                WHERE source_row.operation_id=$1 ORDER BY source_row.id"#,
        ),
        (
            ReportSourceKind::PostExploitAction,
            r#"SELECT source_row.id,source_row.row_version,
                      jsonb_build_object(
                          'row',to_jsonb(source_row),
                          'evidence',COALESCE((
                              SELECT jsonb_agg(
                                  jsonb_build_object(
                                      'evidenceId',link.evidence_id,'role',link.role
                                  ) ORDER BY link.evidence_id,link.role
                              ) FROM post_exploit_action_evidence AS link
                               WHERE link.action_id=source_row.id
                          ),'[]'::jsonb)
                      ) AS content
                 FROM post_exploit_actions AS source_row
                WHERE source_row.operation_id=$1 ORDER BY source_row.id"#,
        ),
        (
            ReportSourceKind::Foothold,
            r#"SELECT source_row.id,source_row.row_version,
                      jsonb_build_object(
                          'row',to_jsonb(source_row),
                          'evidence',COALESCE((
                              SELECT jsonb_agg(
                                  jsonb_build_object(
                                      'evidenceId',link.evidence_id,'role',link.role
                                  ) ORDER BY link.evidence_id,link.role
                              ) FROM foothold_evidence AS link
                               WHERE link.foothold_id=source_row.id
                          ),'[]'::jsonb)
                      ) AS content
                 FROM footholds AS source_row
                WHERE source_row.operation_id=$1 ORDER BY source_row.id"#,
        ),
        (
            ReportSourceKind::InternalAssetObservation,
            r#"SELECT source_row.id,source_row.row_version,
                      jsonb_build_object(
                          'row',to_jsonb(source_row),
                          'evidence',COALESCE((
                              SELECT jsonb_agg(
                                  jsonb_build_object(
                                      'evidenceId',link.evidence_id,'role',link.role
                                  ) ORDER BY link.evidence_id,link.role
                              ) FROM internal_asset_observation_evidence AS link
                               WHERE link.observation_id=source_row.id
                          ),'[]'::jsonb)
                      ) AS content
                 FROM internal_asset_observations AS source_row
                WHERE source_row.operation_id=$1 ORDER BY source_row.id"#,
        ),
        (
            ReportSourceKind::AttackPath,
            r#"SELECT source_row.id,source_row.row_version,
                      jsonb_build_object(
                          'row',to_jsonb(source_row),
                          'edges',COALESCE((
                              SELECT jsonb_agg(
                                  jsonb_build_object(
                                      'row',to_jsonb(edge),
                                      'evidence',COALESCE((
                                          SELECT jsonb_agg(
                                              jsonb_build_object(
                                                  'evidenceId',link.evidence_id,
                                                  'role',link.role
                                              ) ORDER BY link.evidence_id,link.role
                                          ) FROM attack_path_edge_evidence AS link
                                           WHERE link.attack_path_edge_id=edge.id
                                      ),'[]'::jsonb)
                                  ) ORDER BY edge.ordinal,edge.id
                              ) FROM attack_path_edges AS edge
                               WHERE edge.attack_path_id=source_row.id
                          ),'[]'::jsonb)
                      ) AS content
                 FROM attack_paths AS source_row
                WHERE source_row.operation_id=$1 ORDER BY source_row.id"#,
        ),
        (
            ReportSourceKind::ObjectiveAttempt,
            r#"SELECT source_row.id,source_row.row_version,
                      jsonb_build_object(
                          'row',to_jsonb(source_row),
                          'evidence',COALESCE((
                              SELECT jsonb_agg(
                                  jsonb_build_object(
                                      'evidenceId',link.evidence_id,'role',link.role
                                  ) ORDER BY link.evidence_id,link.role
                              ) FROM objective_attempt_evidence AS link
                               WHERE link.objective_attempt_id=source_row.id
                          ),'[]'::jsonb)
                      ) AS content
                 FROM objective_attempts AS source_row
                WHERE source_row.operation_id=$1 ORDER BY source_row.id"#,
        ),
        (
            ReportSourceKind::CleanupObligation,
            r#"SELECT source_row.id,source_row.row_version,
                      jsonb_build_object(
                          'row',to_jsonb(source_row),
                          'evidence',COALESCE((
                              SELECT jsonb_agg(
                                  jsonb_build_object(
                                      'evidenceId',link.evidence_id,'role',link.role
                                  ) ORDER BY link.evidence_id,link.role
                              ) FROM cleanup_obligation_evidence AS link
                               WHERE link.obligation_id=source_row.id
                          ),'[]'::jsonb)
                      ) AS content
                 FROM cleanup_obligations AS source_row
                WHERE source_row.operation_id=$1 ORDER BY source_row.id"#,
        ),
        (
            ReportSourceKind::CleanupWaiver,
            r#"SELECT source_row.id,source_row.row_version,
                      jsonb_build_object(
                          'row',to_jsonb(source_row),
                          'evidence',COALESCE((
                              SELECT jsonb_agg(
                                  jsonb_build_object(
                                      'evidenceId',link.evidence_id,'role',link.role
                                  ) ORDER BY link.evidence_id,link.role
                              ) FROM cleanup_waiver_evidence AS link
                               WHERE link.waiver_id=source_row.id
                          ),'[]'::jsonb)
                      ) AS content
                 FROM cleanup_waivers AS source_row
                WHERE source_row.operation_id=$1 ORDER BY source_row.id"#,
        ),
    ] {
        if !reporting_source_kind_is_authoritative(topology, kind) {
            continue;
        }
        sources.extend(uuid_sources(&mut *connection, operation_id, kind, query).await?);
    }
    for (kind, authority_class, query) in [
        (
            ReportSourceKind::HypothesisRoot,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT root_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM attack_hypotheses source_row
                WHERE operation_id=$1 ORDER BY root_id"#,
        ),
        (
            ReportSourceKind::HypothesisRevision,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT revision_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM attack_hypothesis_revisions source_row
                WHERE operation_id=$1 ORDER BY revision_id"#,
        ),
        (
            ReportSourceKind::HypothesisEvent,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT event_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM attack_hypothesis_state_events source_row
                WHERE operation_id=$1 ORDER BY event_id"#,
        ),
        (
            ReportSourceKind::HypothesisRelation,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT relation_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM attack_hypothesis_relations source_row
                WHERE operation_id=$1 ORDER BY relation_id"#,
        ),
        (
            ReportSourceKind::CandidateAnalysisSnapshot,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT snapshot_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM candidate_analysis_snapshots source_row
                WHERE operation_id=$1 ORDER BY snapshot_id"#,
        ),
        (
            ReportSourceKind::InputProcessingDisposition,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT disposition.input_disposition_id AS id,0::bigint AS row_version,
                      to_jsonb(disposition) AS content
                 FROM input_processing_dispositions disposition
                 JOIN candidate_analysis_attempts attempt
                   ON attempt.analysis_attempt_id=disposition.analysis_attempt_id
                WHERE attempt.operation_id=$1
                ORDER BY disposition.input_disposition_id"#,
        ),
        (
            ReportSourceKind::VerificationCampaign,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT campaign_id AS id,row_version,to_jsonb(source_row) AS content
                 FROM verification_campaigns source_row
                WHERE operation_id=$1 ORDER BY campaign_id"#,
        ),
        (
            ReportSourceKind::VerificationCampaignRound,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT round_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM verification_campaign_rounds source_row
                WHERE operation_id=$1 ORDER BY round_id"#,
        ),
        (
            ReportSourceKind::VerificationStrategyDecision,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT strategy_artifact_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM verification_strategy_artifacts source_row
                WHERE operation_id=$1 ORDER BY strategy_artifact_id"#,
        ),
        (
            ReportSourceKind::PreparedAction,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT prepared_action_id AS id,row_version,to_jsonb(source_row) AS content
                 FROM verification_prepared_actions source_row
                WHERE operation_id=$1 ORDER BY prepared_action_id"#,
        ),
        (
            ReportSourceKind::PreparedActionAuthorization,
            ReportAuthorityClass::AuthorizationAudit,
            r#"SELECT authorization_receipt_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM verification_prepared_action_authorizations source_row
                WHERE operation_id=$1 ORDER BY authorization_receipt_id"#,
        ),
        (
            ReportSourceKind::PreparedActionExecutionReceipt,
            ReportAuthorityClass::ExecutionObservationAudit,
            r#"SELECT action_execution_id AS id,row_version,to_jsonb(source_row) AS content
                 FROM verification_action_executions source_row
                WHERE operation_id=$1 ORDER BY action_execution_id"#,
        ),
        (
            ReportSourceKind::ActionOracleAssessment,
            ReportAuthorityClass::ExecutionObservationAudit,
            r#"SELECT oracle_assessment_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM verification_oracle_assessments source_row
                WHERE operation_id=$1 ORDER BY oracle_assessment_id"#,
        ),
        (
            ReportSourceKind::CampaignAdjudication,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT campaign_adjudication_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM verification_campaign_adjudications source_row
                WHERE operation_id=$1 ORDER BY campaign_adjudication_id"#,
        ),
        (
            ReportSourceKind::CampaignTerminalReceipt,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT campaign_terminal_decision_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM verification_campaign_terminal_decisions source_row
                WHERE operation_id=$1 ORDER BY campaign_terminal_decision_id"#,
        ),
        (
            ReportSourceKind::CampaignObjectiveOutcome,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT objective_outcome_receipt_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM hypothesis_objective_outcome_receipts source_row
                WHERE operation_id=$1 ORDER BY objective_outcome_receipt_id"#,
        ),
        (
            ReportSourceKind::FactDeltaConsumption,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT fact_delta_consumption_id AS id,0::bigint AS row_version,
                      to_jsonb(source_row) AS content
                 FROM fact_delta_consumptions source_row
                WHERE operation_id=$1 ORDER BY fact_delta_consumption_id"#,
        ),
        (
            ReportSourceKind::AuthorityQuarantineEvent,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT quarantine_event_id AS id,0::bigint AS row_version,
                      to_jsonb(source_row) AS content
                 FROM verification_authority_quarantine_events source_row
                WHERE operation_id=$1 ORDER BY quarantine_event_id"#,
        ),
        (
            ReportSourceKind::HypothesisVerificationPlanSeal,
            ReportAuthorityClass::SecurityVerdictAuthority,
            r#"SELECT plan.plan_id AS id,0::bigint AS row_version,to_jsonb(plan) AS content
                 FROM attack_hypothesis_verification_plans plan
                 JOIN attack_hypothesis_revisions revision USING(revision_id)
                WHERE revision.operation_id=$1 ORDER BY plan.plan_id"#,
        ),
        (
            ReportSourceKind::HypothesisProofPathSet,
            ReportAuthorityClass::SecurityVerdictAuthority,
            r#"SELECT plan.plan_id AS id,0::bigint AS row_version,
                      jsonb_build_object('planId',plan.plan_id,'revisionId',plan.revision_id,
                          'memberCount',plan.proof_path_count,
                          'memberSetHash',plan.proof_path_set_hash) AS content
                 FROM attack_hypothesis_verification_plans plan
                 JOIN attack_hypothesis_revisions revision USING(revision_id)
                WHERE revision.operation_id=$1 ORDER BY plan.plan_id"#,
        ),
        (
            ReportSourceKind::HypothesisClaimComponentSet,
            ReportAuthorityClass::SecurityVerdictAuthority,
            r#"SELECT plan.plan_id AS id,0::bigint AS row_version,
                      jsonb_build_object('planId',plan.plan_id,'revisionId',plan.revision_id,
                          'memberCount',plan.required_claim_component_count,
                          'memberSetHash',plan.required_claim_component_set_hash) AS content
                 FROM attack_hypothesis_verification_plans plan
                 JOIN attack_hypothesis_revisions revision USING(revision_id)
                WHERE revision.operation_id=$1 ORDER BY plan.plan_id"#,
        ),
        (
            ReportSourceKind::HypothesisRevisionAdjudication,
            ReportAuthorityClass::SecurityVerdictAuthority,
            r#"SELECT revision_adjudication_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM hypothesis_revision_adjudications source_row
                WHERE operation_id=$1 ORDER BY revision_adjudication_id"#,
        ),
        (
            ReportSourceKind::HypothesisRevisionTerminalDecision,
            ReportAuthorityClass::SecurityVerdictAuthority,
            r#"SELECT revision_terminal_decision_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM hypothesis_revision_terminal_decisions source_row
                WHERE operation_id=$1 ORDER BY revision_terminal_decision_id"#,
        ),
        (
            ReportSourceKind::HypothesisGenerationSeal,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT seal.seal_id AS id,0::bigint AS row_version,to_jsonb(seal) AS content
                 FROM hypothesis_generation_seals seal
                 JOIN hypothesis_generations generation USING(generation_id)
                WHERE generation.operation_id=$1 ORDER BY seal.seal_id"#,
        ),
        (
            ReportSourceKind::EnrichmentObligation,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT enrichment_obligation_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM enrichment_obligations source_row
                WHERE operation_id=$1 ORDER BY enrichment_obligation_id"#,
        ),
        (
            ReportSourceKind::CapabilityAssessment,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT assessment_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM verification_capability_assessments source_row
                WHERE operation_id=$1 ORDER BY assessment_id"#,
        ),
        (
            ReportSourceKind::OracleCensusReceipt,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT oracle_census_seal_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM verification_oracle_census_seals source_row
                WHERE operation_id=$1 ORDER BY oracle_census_seal_id"#,
        ),
        (
            ReportSourceKind::FinalWaveCoverageReceipt,
            ReportAuthorityClass::CoverageAuthority,
            r#"SELECT wave_coverage_receipt_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM verification_wave_coverage_receipts source_row
                WHERE operation_id=$1 ORDER BY wave_coverage_receipt_id"#,
        ),
        (
            ReportSourceKind::LegacyAttemptAuthorityReceipt,
            ReportAuthorityClass::GrandfatheredLegacySecurityVerdict,
            r#"SELECT receipt_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM legacy_attempt_authority_receipts source_row
                WHERE operation_id=$1 ORDER BY receipt_id"#,
        ),
        (
            ReportSourceKind::LegacyReportAuthoritySeal,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT seal_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM legacy_report_authority_seals source_row
                WHERE operation_id=$1 ORDER BY seal_id"#,
        ),
        (
            ReportSourceKind::HypothesisResidual,
            ReportAuthorityClass::MethodAuditOnly,
            r#"SELECT residual_id AS id,0::bigint AS row_version,to_jsonb(source_row) AS content
                 FROM hypothesis_residual_risks source_row
                WHERE operation_id=$1 ORDER BY residual_id"#,
        ),
    ] {
        sources.extend(
            uuid_sources_with_authority(
                &mut *connection,
                operation_id,
                kind,
                authority_class,
                query,
            )
            .await?,
        );
    }
    let (blocked_decision_sources, _) =
        report_cleanup_blocked_decision_truth_on(&mut *connection, operation_id).await?;
    sources.extend(blocked_decision_sources);
    let (evidence_sources, _) =
        report_evidence_audit_truth_on(&mut *connection, operation_id, topology).await?;
    sources.extend(evidence_sources);
    ReportSourceSnapshot::freeze(transaction_snapshot, sources)
        .map_err(|error| anyhow::anyhow!(error))
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct ReportClaimSeedRow {
    source_kind: String,
    source_id_value: String,
    organization_id: Uuid,
    evidence_id: i64,
    claim_kind: String,
    section_kind: String,
    subject_ref: String,
    predicate: String,
    object_value: Value,
    candidate_id: Option<Uuid>,
    lineage_id: Option<Uuid>,
    residual_obligation_id: Option<Uuid>,
    residual_status: Option<String>,
}

#[derive(Clone, Debug)]
struct TypedReportClaimSeed {
    source_kind: String,
    source_id_value: String,
    organization_id: Uuid,
    claim_kind: ReportClaimKind,
    section_kind: String,
    subject_ref: String,
    predicate: String,
    authority_class: ReportAuthorityClass,
    value: ReportClaimValue,
}

#[derive(sqlx::FromRow)]
struct RevisionVerdictSeedRow {
    organization_id: Uuid,
    hypothesis_revision_id: Uuid,
    decision: String,
    verification_plan_seal_id: Uuid,
    verification_plan_seal_hash: String,
    proof_path_set_hash: String,
    claim_component_set_hash: String,
    revision_adjudication_id: Uuid,
    revision_adjudication_hash: String,
    revision_terminal_decision_id: Uuid,
    revision_terminal_decision_hash: String,
    latest_objective_outcome_member_count: i64,
    latest_objective_outcome_set_hash: String,
    finding_id: Option<Uuid>,
    refutation_receipt_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
struct CoverageClaimSeedRow {
    organization_id: Uuid,
    wave_coverage_receipt_id: Uuid,
    wave_coverage_receipt_hash: String,
    denominator_id: Uuid,
    denominator_hash: String,
    planned: i64,
    tested_complete: i64,
    tested_degraded: i64,
    untested: i64,
    blocked: i64,
    residual_ids: Vec<Uuid>,
}

#[derive(sqlx::FromRow)]
struct ResidualClaimSeedRow {
    source_id: Uuid,
    organization_id: Uuid,
    reason_code: String,
    affected_input_ids: Vec<String>,
    owner_code: String,
    next_action_code: String,
}

#[derive(sqlx::FromRow)]
struct InputDispositionLimitationSeedRow {
    input_disposition_id: Uuid,
    organization_id: Uuid,
    snapshot_input_id: Uuid,
    disposition: String,
    reason_code: String,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct LegacyVerdictSeedRow {
    legacy_report_authority_seal_id: Uuid,
    legacy_report_authority_seal_hash: String,
    legacy_attempt_authority_receipt_id: Uuid,
    legacy_attempt_authority_receipt_hash: String,
    organization_id: Uuid,
    candidate_id: Uuid,
    attempt_id: Uuid,
    hypothesis_revision_id: Uuid,
    terminal_status: String,
    source_record_hash: String,
    evidence_membership_hash: String,
    adapter_version: String,
    adapter_digest: String,
    finding_id: Option<Uuid>,
    refutation_receipt_id: Option<Uuid>,
}

fn nonnegative_u64(value: i64) -> anyhow::Result<u64> {
    u64::try_from(value).map_err(|_| anyhow::anyhow!("report_coverage_count_invalid"))
}

async fn typed_report_claim_seeds_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
) -> anyhow::Result<Vec<TypedReportClaimSeed>> {
    let revision_authoritative = frozen_reporting_topology_on(&mut *connection, operation_id)
        .await?
        == StageTopologyContract::UnifiedInvestigationV1;
    let verdict_rows = sqlx::query_as::<_, RevisionVerdictSeedRow>(
        r#"SELECT terminal.organization_id,
                  terminal.hypothesis_revision_id,terminal.decision,
                  plan.plan_id AS verification_plan_seal_id,
                  plan.plan_hash AS verification_plan_seal_hash,
                  plan.proof_path_set_hash,plan.required_claim_component_set_hash
                      AS claim_component_set_hash,
                  adjudication.revision_adjudication_id,
                  adjudication.adjudication_hash AS revision_adjudication_hash,
                  terminal.revision_terminal_decision_id,
                  terminal.decision_hash AS revision_terminal_decision_hash,
                  outcome_set.member_count AS latest_objective_outcome_member_count,
                  outcome_set.member_set_hash AS latest_objective_outcome_set_hash,
                  terminal.finding_id,
                  terminal.refutation_lineage_id AS refutation_receipt_id
             FROM hypothesis_revision_terminal_decisions terminal
             JOIN attack_hypothesis_heads head
               ON head.operation_id=terminal.operation_id
              AND head.organization_id=terminal.organization_id
              AND head.head_revision_id=terminal.terminal_successor_revision_id
              AND head.head_lifecycle_state='closed'
              AND head.head_epistemic_state=terminal.decision
             JOIN hypothesis_revision_adjudications adjudication
               ON adjudication.revision_adjudication_id=terminal.revision_adjudication_id
              AND adjudication.hypothesis_revision_id=terminal.hypothesis_revision_id
              AND adjudication.operation_id=terminal.operation_id
             JOIN attack_hypothesis_verification_plans plan
               ON plan.plan_id=adjudication.verification_plan_id
              AND plan.revision_id=terminal.hypothesis_revision_id
             JOIN hypothesis_objective_outcome_set_seals outcome_set
               ON outcome_set.objective_outcome_set_seal_id=
                  adjudication.objective_outcome_set_seal_id
              AND outcome_set.sealed_at IS NOT NULL
            WHERE terminal.operation_id=$1
              AND terminal.decision=adjudication.outcome
              AND transaction_timestamp()<=adjudication.effective_valid_until
              AND NOT EXISTS (
                  SELECT 1
                    FROM hypothesis_objective_outcome_set_members outcome_member
                    JOIN verification_authority_quarantine_events quarantine
                      ON quarantine.objective_outcome_receipt_id=
                         outcome_member.selected_current_outcome_id
                     AND quarantine.operation_id=terminal.operation_id
                   WHERE outcome_member.objective_outcome_set_seal_id=
                         adjudication.objective_outcome_set_seal_id
              )
            ORDER BY terminal.organization_id,terminal.hypothesis_revision_id"#,
    )
    .bind(operation_id)
    .fetch_all(&mut *connection)
    .await?;

    let coverage_rows = sqlx::query_as::<_, CoverageClaimSeedRow>(
        r#"WITH dispositions AS (
               SELECT wave.wave_denominator_id,campaign_result.coverage_disposition,
                      campaign_result.residual_id
                 FROM verification_campaign_coverage_results campaign_result
                 JOIN verification_campaign_coverage_receipts campaign_receipt
                   ON campaign_receipt.campaign_coverage_receipt_id=
                      campaign_result.campaign_coverage_receipt_id
                 JOIN verification_campaign_coverage_denominators campaign_denominator
                   ON campaign_denominator.campaign_denominator_id=
                      campaign_receipt.campaign_denominator_id
                 JOIN verification_wave_coverage_denominators wave
                   ON wave.wave_denominator_id=campaign_denominator.wave_denominator_id
                WHERE wave.operation_id=$1
               UNION ALL
               SELECT wave_receipt.wave_denominator_id,unassigned.disposition,
                      unassigned.residual_id
                 FROM verification_wave_unassigned_coverage_results unassigned
                 JOIN verification_wave_coverage_receipts wave_receipt
                   ON wave_receipt.wave_coverage_receipt_id=
                      unassigned.wave_coverage_receipt_id
                WHERE wave_receipt.operation_id=$1
           )
           SELECT denominator.organization_id,
                  receipt.wave_coverage_receipt_id,
                  receipt.receipt_hash AS wave_coverage_receipt_hash,
                  denominator.wave_denominator_id AS denominator_id,
                  denominator.member_set_hash AS denominator_hash,
                  denominator.member_count AS planned,
                  COUNT(*) FILTER(
                      WHERE dispositions.coverage_disposition='tested_complete'
                  )::BIGINT
                      AS tested_complete,
                  COUNT(*) FILTER(
                      WHERE dispositions.coverage_disposition='tested_degraded'
                  )::BIGINT
                      AS tested_degraded,
                  COUNT(*) FILTER(
                      WHERE dispositions.coverage_disposition='untested'
                  )::BIGINT AS untested,
                  COUNT(*) FILTER(
                      WHERE dispositions.coverage_disposition='blocked'
                  )::BIGINT AS blocked,
                  COALESCE(array_agg(DISTINCT residual_id ORDER BY residual_id)
                      FILTER(WHERE residual_id IS NOT NULL),'{}'::uuid[]) AS residual_ids
             FROM verification_wave_coverage_receipts receipt
             JOIN verification_wave_coverage_denominators denominator
               ON denominator.wave_denominator_id=receipt.wave_denominator_id
              AND denominator.sealed_at IS NOT NULL
             JOIN hypothesis_generation_seals generation_seal
               ON generation_seal.seal_id=denominator.generation_seal_id
             JOIN hypothesis_generations generation
               ON generation.generation_id=generation_seal.generation_id
              AND generation.operation_id=receipt.operation_id
              AND generation.organization_id=denominator.organization_id
             JOIN hypothesis_consolidation_batches consolidation_batch
               ON consolidation_batch.generation_id=generation.generation_id
              AND consolidation_batch.wave_coverage_receipt_id=
                  receipt.wave_coverage_receipt_id
              AND consolidation_batch.sealed_at IS NOT NULL
             JOIN hypothesis_consolidation_receipts consolidation
               ON consolidation.consolidation_batch_id=
                  consolidation_batch.consolidation_batch_id
             LEFT JOIN hypothesis_fixed_point_receipts fixed_point
               ON fixed_point.consolidation_receipt_id=
                  consolidation.consolidation_receipt_id
              AND fixed_point.generation_id=generation.generation_id
             LEFT JOIN dispositions
               ON dispositions.wave_denominator_id=denominator.wave_denominator_id
            WHERE receipt.operation_id=$1 AND receipt.coverage_status<>'invalid'
              AND ((consolidation.disposition='fixed_point'
                    AND fixed_point.fixed_point_receipt_id IS NOT NULL)
                   OR consolidation.disposition='blocked')
              AND NOT EXISTS (
                  SELECT 1
                    FROM hypothesis_generations newer
                   WHERE newer.operation_id=generation.operation_id
                     AND newer.organization_id=generation.organization_id
                     AND (newer.generation_ordinal>generation.generation_ordinal
                          OR (newer.generation_ordinal=generation.generation_ordinal
                              AND newer.generation_id>generation.generation_id))
              )
              AND NOT EXISTS (
                  SELECT 1
                    FROM verification_authority_quarantine_events quarantine
                    JOIN verification_campaign_coverage_receipts campaign_receipt
                      ON campaign_receipt.campaign_coverage_receipt_id=
                         quarantine.campaign_coverage_receipt_id
                    JOIN verification_campaign_coverage_denominators campaign_denominator
                      ON campaign_denominator.campaign_denominator_id=
                         campaign_receipt.campaign_denominator_id
                   WHERE campaign_denominator.wave_denominator_id=
                         receipt.wave_denominator_id
              )
            GROUP BY denominator.organization_id,receipt.wave_coverage_receipt_id,
                     receipt.receipt_hash,denominator.wave_denominator_id,
                     denominator.member_set_hash,denominator.member_count
           HAVING COUNT(*)=denominator.member_count
            ORDER BY denominator.organization_id,receipt.wave_coverage_receipt_id"#,
    )
    .bind(operation_id)
    .fetch_all(&mut *connection)
    .await?;

    let residual_rows = if revision_authoritative {
        let identities = sqlx::query_as::<_, (Uuid, Uuid)>(
            r#"SELECT stage_execution_id,scope_snapshot_id
                 FROM investigation_stage_closure_publications
                WHERE operation_id=$1 ORDER BY published_at,publication_id"#,
        )
        .bind(operation_id)
        .fetch_all(&mut *connection)
        .await?;
        let (stage_execution_id, scope_snapshot_id) = match identities.as_slice() {
            [identity] => *identity,
            [] => anyhow::bail!("report_investigation_closure_publication_missing"),
            _ => anyhow::bail!("report_investigation_closure_publication_not_unique"),
        };
        investigation_closure_residual_rows_on(
            &mut *connection,
            operation_id,
            stage_execution_id,
            scope_snapshot_id,
        )
        .await?
        .into_iter()
        .map(|row| ResidualClaimSeedRow {
            source_id: row.id,
            organization_id: row.organization_id,
            reason_code: row.reason_code,
            affected_input_ids: row.affected_input_ids,
            owner_code: row.owner_code,
            next_action_code: row.next_action_code,
        })
        .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let input_disposition_rows = sqlx::query_as::<_, InputDispositionLimitationSeedRow>(
        r#"SELECT disposition.input_disposition_id,attempt.organization_id,
                  disposition.snapshot_input_id,disposition.disposition,
                  COALESCE(NULLIF(disposition.reason_code,''),
                           'input_processing_' || disposition.disposition) AS reason_code
             FROM input_processing_dispositions disposition
             JOIN candidate_analysis_attempts attempt
               ON attempt.analysis_attempt_id=disposition.analysis_attempt_id
            WHERE attempt.operation_id=$1
              AND disposition.disposition IN ('gap','blocked')
            ORDER BY attempt.organization_id,disposition.input_disposition_id"#,
    )
    .bind(operation_id)
    .fetch_all(&mut *connection)
    .await?;

    let legacy_rows = sqlx::query_as::<_, LegacyVerdictSeedRow>(
        r#"WITH selected_seal AS (
               SELECT seal_id,seal_hash
                 FROM legacy_report_authority_seals
                WHERE operation_id=$1
                ORDER BY sealed_at DESC,seal_id DESC LIMIT 1
           )
           SELECT seal.seal_id AS legacy_report_authority_seal_id,
                  seal.seal_hash AS legacy_report_authority_seal_hash,
                  receipt.receipt_id AS legacy_attempt_authority_receipt_id,
                  member.receipt_hash AS legacy_attempt_authority_receipt_hash,
                  receipt.organization_id,receipt.candidate_id,receipt.attempt_id,
                  receipt.hypothesis_revision_id,receipt.terminal_status,
                  receipt.source_record_hash,receipt.evidence_membership_hash,
                  receipt.adapter_version,receipt.adapter_digest,receipt.finding_id,
                  receipt.refutation_receipt_id
             FROM selected_seal selected
             JOIN legacy_report_authority_seals seal ON seal.seal_id=selected.seal_id
             JOIN legacy_report_authority_members member ON member.seal_id=seal.seal_id
             JOIN legacy_attempt_authority_receipts receipt
               ON receipt.receipt_id=member.legacy_attempt_authority_receipt_id
              AND receipt.operation_id=seal.operation_id
            ORDER BY member.ordinal"#,
    )
    .bind(operation_id)
    .fetch_all(&mut *connection)
    .await?;

    let mut seeds = Vec::with_capacity(
        verdict_rows.len()
            + coverage_rows.len()
            + residual_rows.len()
            + input_disposition_rows.len()
            + legacy_rows.len(),
    );
    for row in verdict_rows {
        if !revision_authoritative {
            continue;
        }
        let verdict = match row.decision.as_str() {
            "verified" => SecurityVerdictProjection::Verified,
            "refuted" => SecurityVerdictProjection::Refuted,
            _ => anyhow::bail!("report_revision_verdict_invalid"),
        };
        let latest_objective_outcome_member_count =
            nonnegative_u64(row.latest_objective_outcome_member_count)?;
        seeds.push(TypedReportClaimSeed {
            source_kind: ReportSourceKind::HypothesisRevisionTerminalDecision
                .as_str()
                .to_owned(),
            source_id_value: row.revision_terminal_decision_id.to_string(),
            organization_id: row.organization_id,
            claim_kind: ReportClaimKind::Finding,
            section_kind: "findings".to_owned(),
            subject_ref: format!("hypothesis_revision:{}", row.hypothesis_revision_id),
            predicate: "security_verdict".to_owned(),
            authority_class: ReportAuthorityClass::SecurityVerdictAuthority,
            value: ReportClaimValue::SecurityVerdict {
                verdict,
                hypothesis_revision_id: row.hypothesis_revision_id,
                authority: SecurityVerdictAuthority::RevisionAdjudicationV1 {
                    verification_plan_seal_id: row.verification_plan_seal_id,
                    verification_plan_seal_hash: row.verification_plan_seal_hash,
                    proof_path_set_hash: row.proof_path_set_hash,
                    claim_component_set_hash: row.claim_component_set_hash,
                    revision_adjudication_id: row.revision_adjudication_id,
                    revision_adjudication_hash: row.revision_adjudication_hash,
                    revision_terminal_decision_id: row.revision_terminal_decision_id,
                    revision_terminal_decision_hash: row.revision_terminal_decision_hash,
                    latest_objective_outcome_member_count,
                    latest_objective_outcome_set_hash: row.latest_objective_outcome_set_hash,
                    finding_id: row.finding_id,
                    refutation_receipt_id: row.refutation_receipt_id,
                },
            },
        });
    }
    for row in coverage_rows {
        if !revision_authoritative {
            continue;
        }
        let planned = nonnegative_u64(row.planned)?;
        let tested_complete = nonnegative_u64(row.tested_complete)?;
        let tested_degraded = nonnegative_u64(row.tested_degraded)?;
        let untested = nonnegative_u64(row.untested)?;
        let blocked = nonnegative_u64(row.blocked)?;
        if tested_complete
            .checked_add(tested_degraded)
            .and_then(|value| value.checked_add(untested))
            .and_then(|value| value.checked_add(blocked))
            != Some(planned)
        {
            anyhow::bail!("report_coverage_denominator_invalid");
        }
        seeds.push(TypedReportClaimSeed {
            source_kind: ReportSourceKind::FinalWaveCoverageReceipt
                .as_str()
                .to_owned(),
            source_id_value: row.wave_coverage_receipt_id.to_string(),
            organization_id: row.organization_id,
            claim_kind: ReportClaimKind::Scope,
            section_kind: "organization".to_owned(),
            subject_ref: format!("coverage_denominator:{}", row.denominator_id),
            predicate: "declared_coverage".to_owned(),
            authority_class: ReportAuthorityClass::CoverageAuthority,
            value: ReportClaimValue::Coverage {
                final_wave_coverage_receipt_id: row.wave_coverage_receipt_id,
                final_wave_coverage_receipt_hash: row.wave_coverage_receipt_hash,
                denominator_id: row.denominator_id,
                denominator_hash: row.denominator_hash,
                planned,
                tested_complete,
                tested_degraded,
                untested,
                blocked,
                residual_ids: row.residual_ids,
                coverage_sufficiency: CoverageSufficiencyProjection::NotAssessed,
            },
        });
    }
    for row in residual_rows {
        if !revision_authoritative {
            continue;
        }
        seeds.push(TypedReportClaimSeed {
            source_kind: ReportSourceKind::InvestigationClosureResidual
                .as_str()
                .to_owned(),
            source_id_value: row.source_id.to_string(),
            organization_id: row.organization_id,
            claim_kind: ReportClaimKind::Limitation,
            section_kind: "limitations".to_owned(),
            subject_ref: format!("investigation_closure_residual:{}", row.source_id),
            predicate: "residual_risk".to_owned(),
            authority_class: ReportAuthorityClass::MethodAuditOnly,
            value: ReportClaimValue::Limitation {
                reason_code: row.reason_code,
                affected_input_ids: row.affected_input_ids,
                residual_ids: vec![row.source_id],
                owner_code: row.owner_code,
                next_action_code: row.next_action_code,
            },
        });
    }
    for row in input_disposition_rows {
        if !revision_authoritative {
            continue;
        }
        seeds.push(TypedReportClaimSeed {
            source_kind: ReportSourceKind::InputProcessingDisposition
                .as_str()
                .to_owned(),
            source_id_value: row.input_disposition_id.to_string(),
            organization_id: row.organization_id,
            claim_kind: ReportClaimKind::Limitation,
            section_kind: "limitations".to_owned(),
            subject_ref: format!("snapshot_input:{}", row.snapshot_input_id),
            predicate: "input_processing_limitation".to_owned(),
            authority_class: ReportAuthorityClass::MethodAuditOnly,
            value: ReportClaimValue::Limitation {
                reason_code: row.reason_code,
                affected_input_ids: vec![row.snapshot_input_id.to_string()],
                residual_ids: Vec::new(),
                owner_code: "candidate_analysis".to_owned(),
                next_action_code: format!("resolve_input_processing_{}", row.disposition),
            },
        });
    }
    let mut legacy_limitations = BTreeMap::<(Uuid, Uuid, String), Vec<String>>::new();
    for row in legacy_rows {
        if revision_authoritative {
            continue;
        }
        let verdict = match row.terminal_status.as_str() {
            "verified" => SecurityVerdictProjection::Verified,
            "refuted" => SecurityVerdictProjection::Refuted,
            _ => anyhow::bail!("report_legacy_verdict_invalid"),
        };
        let claim_kind = if verdict == SecurityVerdictProjection::Verified {
            ReportClaimKind::Finding
        } else {
            ReportClaimKind::CandidateDisposition
        };
        legacy_limitations
            .entry((
                row.organization_id,
                row.legacy_report_authority_seal_id,
                row.legacy_report_authority_seal_hash.clone(),
            ))
            .or_default()
            .push(row.legacy_attempt_authority_receipt_id.to_string());
        seeds.push(TypedReportClaimSeed {
            source_kind: ReportSourceKind::LegacyAttemptAuthorityReceipt
                .as_str()
                .to_owned(),
            source_id_value: row.legacy_attempt_authority_receipt_id.to_string(),
            organization_id: row.organization_id,
            claim_kind,
            section_kind: "findings".to_owned(),
            subject_ref: format!("hypothesis_revision:{}", row.hypothesis_revision_id),
            predicate: "grandfathered_legacy_security_verdict".to_owned(),
            authority_class: ReportAuthorityClass::GrandfatheredLegacySecurityVerdict,
            value: ReportClaimValue::SecurityVerdict {
                verdict,
                hypothesis_revision_id: row.hypothesis_revision_id,
                authority: SecurityVerdictAuthority::LegacyAttemptV1 {
                    candidate_id: row.candidate_id,
                    attempt_id: row.attempt_id,
                    legacy_attempt_authority_receipt_id: row.legacy_attempt_authority_receipt_id,
                    legacy_attempt_authority_receipt_hash: row
                        .legacy_attempt_authority_receipt_hash,
                    legacy_report_authority_seal_id: row.legacy_report_authority_seal_id,
                    legacy_report_authority_seal_hash: row.legacy_report_authority_seal_hash,
                    legacy_contract_version: "legacy_attempt_report_authority.v1".to_owned(),
                    terminal_status: row.terminal_status,
                    source_record_hash: row.source_record_hash,
                    evidence_membership_hash: row.evidence_membership_hash,
                    adapter_version: row.adapter_version,
                    adapter_digest: row.adapter_digest,
                    finding_id: row.finding_id,
                    refutation_receipt_id: row.refutation_receipt_id,
                    limitation_codes: vec!["legacy_coverage_unavailable".to_owned()],
                },
            },
        });
    }
    for ((organization_id, seal_id, seal_hash), mut affected_input_ids) in legacy_limitations {
        affected_input_ids.sort();
        seeds.push(TypedReportClaimSeed {
            source_kind: ReportSourceKind::LegacyReportAuthoritySeal
                .as_str()
                .to_owned(),
            source_id_value: seal_id.to_string(),
            organization_id,
            claim_kind: ReportClaimKind::Limitation,
            section_kind: "limitations".to_owned(),
            subject_ref: format!("legacy_report_authority:{seal_id}"),
            predicate: "legacy_coverage_unavailable".to_owned(),
            authority_class: ReportAuthorityClass::MethodAuditOnly,
            value: ReportClaimValue::Limitation {
                reason_code: "legacy_coverage_unavailable".to_owned(),
                affected_input_ids,
                residual_ids: Vec::new(),
                owner_code: "legacy_report_adapter".to_owned(),
                next_action_code: format!("migrate_to_revision_adjudication:{seal_hash}"),
            },
        });
    }
    Ok(seeds)
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct FrozenOrganizationRow {
    organization_id: Uuid,
    organization_name: String,
    ordinal: i32,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct ReportOperationContextRow {
    tool_truth_contract: String,
    project_scope_id: Uuid,
    scope_snapshot_id: Uuid,
    scope_snapshot_hash: String,
}

async fn report_operation_context_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
) -> anyhow::Result<(ReportOperationContextRow, Vec<FrozenOrganizationRow>)> {
    let context = sqlx::query_as::<_, ReportOperationContextRow>(
        r#"SELECT operation.tool_truth_contract,
                  snapshot.project_scope_id,
                  snapshot.id AS scope_snapshot_id,
                  snapshot.scope_hash AS scope_snapshot_hash
             FROM operation_org_scope_snapshots AS snapshot
             JOIN operation_state AS operation ON operation.operation_id=snapshot.operation_id
            WHERE snapshot.operation_id=$1 AND snapshot.sealed_at IS NOT NULL
            ORDER BY snapshot.sealed_at DESC,snapshot.id DESC
            LIMIT 1"#,
    )
    .bind(operation_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or_else(|| anyhow::anyhow!("report_frozen_scope_missing"))?;
    let organizations = sqlx::query_as::<_, FrozenOrganizationRow>(
        r#"SELECT unit.organization_id,
                  unit.organization_name_at_freeze AS organization_name,
                  unit.ordinal
             FROM operation_org_scope_units AS unit
            WHERE unit.snapshot_id=$1
            ORDER BY unit.ordinal,unit.organization_id"#,
    )
    .bind(context.scope_snapshot_id)
    .fetch_all(&mut *connection)
    .await?;
    if organizations.is_empty() {
        anyhow::bail!("report_frozen_scope_missing");
    }
    Ok((context, organizations))
}

async fn report_claim_seeds_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    topology: StageTopologyContract,
) -> anyhow::Result<Vec<ReportClaimSeedRow>> {
    Ok(sqlx::query_as::<_, ReportClaimSeedRow>(
        r#"SELECT 'stage_episode'::text AS source_kind,
                  episode.episode_id::text AS source_id_value,
                  episode.organization_id_at_time AS organization_id,
                  evidence.id AS evidence_id,
                  'scope'::text AS claim_kind,
                  'organization'::text AS section_kind,
                  'stage_episode:' || episode.episode_id::text AS subject_ref,
                  'completed_with_verdict'::text AS predicate,
                  jsonb_build_object(
                      'stageKind',episode.stage_kind,
                      'verdict',episode.verdict,
                      'reasonCodes',episode.reason_codes
                  ) AS object_value,
                  NULL::uuid AS finding_id,NULL::uuid AS candidate_id,
                  NULL::uuid AS lineage_id,NULL::uuid AS residual_obligation_id,
                  NULL::text AS residual_status
             FROM stage_episodes AS episode
             CROSS JOIN LATERAL unnest(episode.evidence_refs) AS episode_evidence(evidence_id)
             JOIN audit_log AS evidence
               ON evidence.id=episode_evidence.evidence_id
              AND evidence.audit_role='evidence' AND evidence.run_id=$1
            WHERE episode.source_operation_id=$1
              AND operation_stage_rank_for_topology($2,episode.stage_kind) IS NOT NULL
            UNION ALL
           SELECT 'finding',finding.id::text,lineage.organization_id,
                  evidence.id,'finding','findings',
                  'finding:' || finding.id::text,'verified',
                  jsonb_build_object(
                      'title',finding.title,'severity',finding.sev::text,
                      'status',finding.status::text,'target',finding.target
                  ),
                  finding.id,lineage.candidate_id,lineage.id,NULL::uuid,NULL::text
             FROM finding_lineage AS lineage
             JOIN findings AS finding ON finding.id=lineage.finding_id
             JOIN candidate_attempt_evidence AS link
               ON link.attempt_id=lineage.candidate_attempt_id
             JOIN audit_log AS evidence
               ON evidence.id=link.evidence_id
              AND evidence.audit_role='evidence' AND evidence.run_id=$1
            WHERE lineage.operation_id=$1
              AND $2='legacy_candidate_verification_v1'
            UNION ALL
           SELECT 'candidate_attempt',attempt.id::text,attempt.organization_id,
                  evidence.id,'candidate_disposition','findings',
                  'candidate_attempt:' || attempt.id::text,'disposition',
                  jsonb_build_object(
                      'candidateId',attempt.candidate_id,
                      'status',attempt.status,
                      'targetType',attempt.target_type_at_time,
                      'targetValue',attempt.target_value_at_time,
                      'result',attempt.result_json
                  ),
                  NULL::uuid,attempt.candidate_id,NULL::uuid,NULL::uuid,NULL::text
             FROM candidate_attempts AS attempt
             JOIN candidate_attempt_evidence AS link ON link.attempt_id=attempt.id
             JOIN audit_log AS evidence
               ON evidence.id=link.evidence_id
              AND evidence.audit_role='evidence' AND evidence.run_id=$1
            WHERE attempt.operation_id=$1
              AND $2='legacy_candidate_verification_v1'
              AND attempt.status IN (
                  'verified','refuted','blocked','retryable_failed','abandoned'
              )
            UNION ALL
           SELECT 'technique_outcome',outcome.id::text,outcome.organization_id,
                  evidence.id,'technique_outcome','methodology',
                  'technique_outcome:' || outcome.id::text,'outcome',
                  jsonb_build_object(
                      'asset',outcome.asset,'technique',outcome.technique,
                      'outcome',outcome.outcome,'source',outcome.source,
                      'resultCount',outcome.result_count,
                      'confidence',outcome.confidence
                  ),
                  NULL::uuid,NULL::uuid,NULL::uuid,NULL::uuid,NULL::text
             FROM technique_outcomes AS outcome
             CROSS JOIN LATERAL unnest(outcome.evidence_ids) AS outcome_evidence(evidence_id)
             JOIN audit_log AS evidence
               ON evidence.id=outcome_evidence.evidence_id
              AND evidence.audit_role='evidence' AND evidence.run_id=$1
            WHERE outcome.run_id=$1::text
            UNION ALL
           SELECT 'post_exploit_action',action.id::text,action.organization_id_at_time,
                  evidence.id,'scope','organization',
                  'post_exploit_action:' || action.id::text,'status',
                  jsonb_build_object(
                      'capabilityId',action.capability_id,
                      'sideEffectClass',action.side_effect_class,
                      'status',action.status,'plan',action.plan
                  ),
                  NULL::uuid,NULL::uuid,NULL::uuid,NULL::uuid,NULL::text
             FROM post_exploit_actions AS action
             JOIN post_exploit_action_evidence AS link ON link.action_id=action.id
             JOIN audit_log AS evidence
               ON evidence.id=link.evidence_id
              AND evidence.audit_role='evidence' AND evidence.run_id=$1
            WHERE action.operation_id=$1
            UNION ALL
           SELECT 'foothold',foothold.id::text,foothold.organization_id_at_time,
                  evidence.id,'scope','organization',
                  'foothold:' || foothold.id::text,'status',
                  jsonb_build_object(
                      'status',foothold.status,
                      'targetType',foothold.target_type_at_time,
                      'targetValue',foothold.target_value_at_time
                  ),
                  NULL::uuid,NULL::uuid,NULL::uuid,NULL::uuid,NULL::text
             FROM footholds AS foothold
             JOIN foothold_evidence AS link ON link.foothold_id=foothold.id
             JOIN audit_log AS evidence
               ON evidence.id=link.evidence_id
              AND evidence.audit_role='evidence' AND evidence.run_id=$1
            WHERE foothold.operation_id=$1
            UNION ALL
           SELECT 'internal_asset_observation',observation.id::text,
                  observation.organization_id_at_time,evidence.id,
                  'scope','organization',
                  'internal_asset_observation:' || observation.id::text,
                  observation.observation_kind,
                  jsonb_build_object(
                      'assetType',observation.asset_type,
                      'assetValue',observation.asset_value_at_time,
                      'observation',observation.observation
                  ),
                  NULL::uuid,NULL::uuid,NULL::uuid,NULL::uuid,NULL::text
             FROM internal_asset_observations AS observation
             JOIN internal_asset_observation_evidence AS link
               ON link.observation_id=observation.id
             JOIN audit_log AS evidence
               ON evidence.id=link.evidence_id
              AND evidence.audit_role='evidence' AND evidence.run_id=$1
            WHERE observation.operation_id=$1
            UNION ALL
           SELECT 'attack_path',path.id::text,path.organization_id_at_time,
                  evidence.id,'attack_path','attack_paths',
                  'attack_path:' || path.id::text,'status',
                  jsonb_build_object('status',path.status,'pathHash',path.path_hash),
                  NULL::uuid,NULL::uuid,NULL::uuid,NULL::uuid,NULL::text
             FROM attack_paths AS path
             JOIN attack_path_edges AS edge ON edge.attack_path_id=path.id
             JOIN attack_path_edge_evidence AS link ON link.attack_path_edge_id=edge.id
             JOIN audit_log AS evidence
               ON evidence.id=link.evidence_id
              AND evidence.audit_role='evidence' AND evidence.run_id=$1
            WHERE path.operation_id=$1
            UNION ALL
           SELECT 'objective_attempt',objective.id::text,
                  objective.organization_id_at_time,evidence.id,
                  'objective_outcome','attack_paths',
                  'objective_attempt:' || objective.id::text,'outcome',
                  jsonb_build_object(
                      'objectiveKind',objective.objective_kind,
                      'outcome',objective.outcome,
                      'simulationPlan',objective.simulation_plan
                  ),
                  NULL::uuid,NULL::uuid,NULL::uuid,NULL::uuid,NULL::text
             FROM objective_attempts AS objective
             JOIN objective_attempt_evidence AS link
               ON link.objective_attempt_id=objective.id
             JOIN audit_log AS evidence
               ON evidence.id=link.evidence_id
              AND evidence.audit_role='evidence' AND evidence.run_id=$1
            WHERE objective.operation_id=$1
            UNION ALL
           SELECT 'cleanup_blocked_decision',decision.id::text,
                  decision.organization_id_at_time,evidence.id,
                  'cleanup_residual','cleanup_residuals',
                  'cleanup_obligation:' || decision.obligation_id::text,'residual_risk',
                  jsonb_build_object(
                      'status','blocked',
                      'decidedByPrincipalId',decision.decided_by_principal_id,
                      'reason',decision.reason,
                      'residualRisk',decision.residual_risk
                  ),
                  NULL::uuid,NULL::uuid,NULL::uuid,decision.obligation_id,'blocked'::text
             FROM cleanup_blocked_decisions AS decision
             JOIN cleanup_blocked_decision_evidence AS link
               ON link.blocked_decision_id=decision.id
             JOIN audit_log AS evidence
               ON evidence.id=link.evidence_id
              AND evidence.audit_role='evidence' AND evidence.run_id=$1
            WHERE decision.operation_id=$1
            UNION ALL
           SELECT 'cleanup_waiver',waiver.id::text,waiver.organization_id_at_time,
                  evidence.id,'cleanup_residual','cleanup_residuals',
                  'cleanup_obligation:' || waiver.obligation_id::text,'residual_risk',
                  jsonb_build_object(
                      'status','waived_by_user','reason',waiver.reason,
                      'residualRisk',waiver.residual_risk
                  ),
                  NULL::uuid,NULL::uuid,NULL::uuid,waiver.obligation_id,
                  'waived_by_user'::text
             FROM cleanup_waivers AS waiver
             JOIN cleanup_waiver_evidence AS link ON link.waiver_id=waiver.id
             JOIN audit_log AS evidence
               ON evidence.id=link.evidence_id
              AND evidence.audit_role='evidence' AND evidence.run_id=$1
            WHERE waiver.operation_id=$1
            ORDER BY organization_id,section_kind,source_kind,source_id_value,evidence_id"#,
    )
    .bind(operation_id)
    .bind(topology.as_str())
    .fetch_all(&mut *connection)
    .await?)
}

#[derive(Clone, Debug)]
struct AccumulatedClaimSeed {
    row: ReportClaimSeedRow,
    evidence_ids: BTreeSet<i64>,
    typed_value: Option<ReportClaimValue>,
    authority_class: ReportAuthorityClass,
}

fn claim_kind(value: &str) -> anyhow::Result<ReportClaimKind> {
    match value {
        "scope" => Ok(ReportClaimKind::Scope),
        "finding" => Ok(ReportClaimKind::Finding),
        "candidate_disposition" => Ok(ReportClaimKind::CandidateDisposition),
        "technique_outcome" => Ok(ReportClaimKind::TechniqueOutcome),
        "attack_path" => Ok(ReportClaimKind::AttackPath),
        "objective_outcome" => Ok(ReportClaimKind::ObjectiveOutcome),
        "cleanup_residual" => Ok(ReportClaimKind::CleanupResidual),
        "limitation" => Ok(ReportClaimKind::Limitation),
        _ => anyhow::bail!("report_claim_kind_invalid"),
    }
}

const fn claim_kind_wire(value: ReportClaimKind) -> &'static str {
    match value {
        ReportClaimKind::Scope => "scope",
        ReportClaimKind::Finding => "finding",
        ReportClaimKind::CandidateDisposition => "candidate_disposition",
        ReportClaimKind::TechniqueOutcome => "technique_outcome",
        ReportClaimKind::AttackPath => "attack_path",
        ReportClaimKind::ObjectiveOutcome => "objective_outcome",
        ReportClaimKind::CleanupResidual => "cleanup_residual",
        ReportClaimKind::Limitation => "limitation",
    }
}

fn section_kind(value: &str) -> anyhow::Result<ReportSectionKind> {
    match value {
        "executive_summary" => Ok(ReportSectionKind::ExecutiveSummary),
        "organization" => Ok(ReportSectionKind::Organization),
        "findings" => Ok(ReportSectionKind::Findings),
        "attack_paths" => Ok(ReportSectionKind::AttackPaths),
        "cleanup_residuals" => Ok(ReportSectionKind::CleanupResiduals),
        "methodology" => Ok(ReportSectionKind::Methodology),
        "limitations" => Ok(ReportSectionKind::Limitations),
        _ => anyhow::bail!("report_section_kind_invalid"),
    }
}

fn typed_canonical_seed_value(seed: &ReportClaimSeedRow) -> Option<ReportClaimValue> {
    if seed.claim_kind != "cleanup_residual" {
        return None;
    }
    let reason_code = seed
        .object_value
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("cleanup_residual_risk")
        .to_owned();
    let status = seed
        .object_value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("retained");
    Some(ReportClaimValue::Limitation {
        reason_code,
        affected_input_ids: seed
            .subject_ref
            .strip_prefix("cleanup_obligation:")
            .map(str::to_owned)
            .into_iter()
            .collect(),
        residual_ids: Vec::new(),
        owner_code: "cleanup_authority".to_owned(),
        next_action_code: format!("resolve_cleanup_{status}"),
    })
}

fn validate_typed_report_value(value: ReportClaimValue) -> anyhow::Result<ReportClaimValue> {
    let encoded = serde_json::to_value(&value)?;
    let validated =
        redact_report_value(encoded).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    serde_json::from_value(validated).map_err(Into::into)
}

fn build_report_read_model(
    report_id: Uuid,
    revision_id: Uuid,
    operation_id: Uuid,
    context: &ReportOperationContextRow,
    organizations: &[FrozenOrganizationRow],
    source_snapshot: ReportSourceSnapshot,
    seeds: Vec<ReportClaimSeedRow>,
    typed_seeds: Vec<TypedReportClaimSeed>,
) -> anyhow::Result<ReportReadModel> {
    let source_index = source_snapshot
        .ordered_sources
        .iter()
        .map(|source| {
            let id = StoredCanonicalRowId::from_domain(&source.id)
                .map_err(|error| anyhow::anyhow!(error.code()))?;
            Ok(((source.kind.as_str().to_string(), id.value), source.clone()))
        })
        .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    let organization_names = organizations
        .iter()
        .map(|organization| {
            (
                organization.organization_id,
                organization.organization_name.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut accumulated = BTreeMap::<(Uuid, String, String, String), AccumulatedClaimSeed>::new();
    for seed in seeds {
        if !organization_names.contains_key(&seed.organization_id) {
            anyhow::bail!("report_claim_organization_out_of_scope");
        }
        let key = (
            seed.organization_id,
            seed.section_kind.clone(),
            seed.source_kind.clone(),
            seed.source_id_value.clone(),
        );
        let entry = accumulated
            .entry(key)
            .or_insert_with(|| AccumulatedClaimSeed {
                typed_value: typed_canonical_seed_value(&seed),
                row: seed.clone(),
                evidence_ids: BTreeSet::new(),
                authority_class: ReportAuthorityClass::MethodAuditOnly,
            });
        if entry.row.claim_kind != seed.claim_kind
            || entry.row.subject_ref != seed.subject_ref
            || entry.row.predicate != seed.predicate
            || entry.row.object_value != seed.object_value
        {
            anyhow::bail!("report_claim_projection_conflict");
        }
        entry.evidence_ids.insert(seed.evidence_id);
    }

    let mut by_section = BTreeMap::<(Uuid, String), Vec<AccumulatedClaimSeed>>::new();
    for ((organization_id, section, _, _), seed) in accumulated {
        by_section
            .entry((organization_id, section))
            .or_default()
            .push(seed);
    }
    for seed in typed_seeds {
        if !organization_names.contains_key(&seed.organization_id) {
            anyhow::bail!("report_claim_organization_out_of_scope");
        }
        if seed
            .value
            .required_section_kind()
            .is_some_and(|required| section_kind(&seed.section_kind).ok() != Some(required))
        {
            anyhow::bail!("report_typed_claim_section_invalid");
        }
        by_section
            .entry((seed.organization_id, seed.section_kind.clone()))
            .or_default()
            .push(AccumulatedClaimSeed {
                row: ReportClaimSeedRow {
                    source_kind: seed.source_kind,
                    source_id_value: seed.source_id_value,
                    organization_id: seed.organization_id,
                    evidence_id: 0,
                    claim_kind: claim_kind_wire(seed.claim_kind).to_owned(),
                    section_kind: seed.section_kind,
                    subject_ref: seed.subject_ref,
                    predicate: seed.predicate,
                    object_value: serde_json::Value::Null,
                    candidate_id: None,
                    lineage_id: None,
                    residual_obligation_id: None,
                    residual_status: None,
                },
                evidence_ids: BTreeSet::new(),
                typed_value: Some(seed.value),
                authority_class: seed.authority_class,
            });
    }
    for organization in organizations {
        by_section
            .entry((organization.organization_id, "organization".to_string()))
            .or_default();
    }

    let organization_ordinals = organizations
        .iter()
        .map(|organization| (organization.organization_id, organization.ordinal))
        .collect::<BTreeMap<_, _>>();
    let mut section_entries = by_section.into_iter().collect::<Vec<_>>();
    section_entries.sort_by_key(|((organization_id, kind), _)| {
        (
            organization_ordinals
                .get(organization_id)
                .copied()
                .unwrap_or(i32::MAX),
            match kind.as_str() {
                "organization" => 0,
                "findings" => 1,
                "attack_paths" => 2,
                "cleanup_residuals" => 3,
                "methodology" => 4,
                "limitations" => 5,
                _ => 6,
            },
        )
    });

    let mut organization_sections = Vec::new();
    let mut citations = Vec::new();
    let mut findings = Vec::new();
    let mut cleanup_residuals = Vec::new();
    for (section_ordinal, ((organization_id, section_name), mut claims)) in
        section_entries.into_iter().enumerate()
    {
        let section_id = Uuid::new_v5(
            &revision_id,
            format!("section:{organization_id}:{section_name}").as_bytes(),
        );
        claims.sort_by_key(|seed| {
            (
                seed.row.source_kind.clone(),
                seed.row.source_id_value.clone(),
            )
        });
        let mut report_claims = Vec::with_capacity(claims.len());
        for (claim_ordinal, seed) in claims.into_iter().enumerate() {
            let source = source_index
                .get(&(
                    seed.row.source_kind.clone(),
                    seed.row.source_id_value.clone(),
                ))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("report_claim_source_not_in_snapshot"))?;
            let claim_id = Uuid::new_v5(
                &section_id,
                format!(
                    "claim:{}:{}",
                    seed.row.source_kind, seed.row.source_id_value
                )
                .as_bytes(),
            );
            let mut citation_ids = Vec::with_capacity(seed.evidence_ids.len());
            for (citation_ordinal, evidence_id) in seed.evidence_ids.into_iter().enumerate() {
                let citation_id =
                    Uuid::new_v5(&claim_id, format!("evidence:{evidence_id}").as_bytes());
                citation_ids.push(citation_id);
                citations.push(ReportCitation {
                    citation_id,
                    revision_id,
                    claim_id,
                    source_type: CitationSourceType::CanonicalFact,
                    source: source.clone(),
                    evidence_audit_id: Some(evidence_id),
                    organization_id_at_time: organization_id,
                    display_label: format!(
                        "{} {} evidence {}",
                        seed.row.source_kind, seed.row.predicate, evidence_id
                    ),
                    ordinal: i32::try_from(citation_ordinal)
                        .map_err(|_| anyhow::anyhow!("report_citation_overflow"))?,
                });
            }
            if citation_ids.is_empty() && seed.typed_value.is_some() {
                let citation_id = Uuid::new_v5(&claim_id, b"canonical-authority");
                citation_ids.push(citation_id);
                citations.push(ReportCitation {
                    citation_id,
                    revision_id,
                    claim_id,
                    source_type: CitationSourceType::CanonicalFact,
                    source: source.clone(),
                    evidence_audit_id: None,
                    organization_id_at_time: organization_id,
                    display_label: format!("{} authority", seed.row.source_kind),
                    ordinal: 0,
                });
            }
            let report_claim = ReportClaim {
                claim_id,
                revision_id,
                section_id,
                organization_id_at_time: Some(organization_id),
                claim_kind: claim_kind(&seed.row.claim_kind)?,
                authority_class: seed.authority_class,
                subject_ref: seed.row.subject_ref.clone(),
                predicate: seed.row.predicate.clone(),
                value: if let Some(value) = seed.typed_value {
                    validate_typed_report_value(value)?
                } else {
                    let redacted = redact_report_value(seed.row.object_value.clone())
                        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                    ReportClaimValue::from_legacy_redacted(
                        seed.row.subject_ref.clone(),
                        seed.row.source_kind.clone(),
                        seed.row.predicate.clone(),
                        &redacted,
                    )
                    .map_err(|code| anyhow::anyhow!(code))?
                },
                citation_ids,
                ordinal: i32::try_from(claim_ordinal)
                    .map_err(|_| anyhow::anyhow!("report_claim_overflow"))?,
            };
            let verified_authority_finding = match &report_claim.value {
                ReportClaimValue::SecurityVerdict {
                    verdict: SecurityVerdictProjection::Verified,
                    authority:
                        SecurityVerdictAuthority::RevisionAdjudicationV1 { finding_id, .. }
                        | SecurityVerdictAuthority::LegacyAttemptV1 { finding_id, .. },
                    ..
                } => *finding_id,
                _ => None,
            };
            if let Some(finding_id) = verified_authority_finding {
                findings.push(ReportFinding {
                    finding_id,
                    organization_id_at_time: organization_id,
                    candidate_id: seed.row.candidate_id,
                    verified_lineage_id: seed.row.lineage_id,
                    claim_id,
                });
            }
            if let (Some(obligation_id), Some(status)) = (
                seed.row.residual_obligation_id,
                seed.row.residual_status.clone(),
            ) {
                cleanup_residuals.push(ReportResidual {
                    obligation_id,
                    organization_id_at_time: organization_id,
                    status,
                    claim_id,
                });
            }
            report_claims.push(report_claim);
        }
        let organization_name = organization_names
            .get(&organization_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("report_organization_name_missing"))?;
        organization_sections.push(OrganizationReportSection {
            organization_id_at_time: organization_id,
            organization_name_at_snapshot: organization_name.clone(),
            section: ReportSectionModel {
                section_id,
                revision_id,
                organization_id_at_time: Some(organization_id),
                organization_name_at_snapshot: Some(organization_name),
                kind: section_kind(&section_name)?,
                claims: report_claims,
                rendered_content: None,
                ordinal: i32::try_from(section_ordinal)
                    .map_err(|_| anyhow::anyhow!("report_section_overflow"))?,
            },
        });
    }

    Ok(ReportReadModel {
        report_id,
        revision_id,
        operation_id,
        project_scope_id: context.project_scope_id,
        scope_snapshot_id: context.scope_snapshot_id,
        scope_snapshot_hash: context.scope_snapshot_hash.clone(),
        source_snapshot,
        organization_sections,
        findings,
        cleanup_residuals,
        citations,
    })
}

#[derive(Clone)]
pub struct PgReportTruthPort {
    pool: Arc<PgPool>,
    project_authority: Option<ReportingProjectAuthority>,
    persistence_mode: ReportPersistenceMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReportPersistenceMode {
    Publishable,
    EvidenceSummary,
}

impl PgReportTruthPort {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self {
            pool,
            project_authority: None,
            persistence_mode: ReportPersistenceMode::Publishable,
        }
    }

    pub fn with_project_authority(
        pool: Arc<PgPool>,
        project_authority: ReportingProjectAuthority,
    ) -> Self {
        Self {
            pool,
            project_authority: Some(project_authority),
            persistence_mode: ReportPersistenceMode::Publishable,
        }
    }

    pub(super) fn evidence_summary(
        pool: Arc<PgPool>,
        project_authority: ReportingProjectAuthority,
    ) -> Self {
        Self {
            pool,
            project_authority: Some(project_authority),
            persistence_mode: ReportPersistenceMode::EvidenceSummary,
        }
    }

    fn repository_error(error: impl std::fmt::Display) -> ReportingAppError {
        let message = error.to_string();
        if message.contains("report_project_authority_stale") {
            ReportingAppError::SourceSnapshotStale
        } else {
            ReportingAppError::Repository(message)
        }
    }
}

#[async_trait::async_trait]
impl ReportTruthPort for PgReportTruthPort {
    async fn build_repeatable_read_snapshot(
        &self,
        operation_id: Uuid,
    ) -> Result<BuiltReportRevision, ReportingAppError> {
        let mut tx = self.pool.begin().await.map_err(Self::repository_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(Self::repository_error)?;
        let source_snapshot = current_reportable_source_snapshot_on(&mut tx, operation_id)
            .await
            .map_err(Self::repository_error)?;
        let topology = frozen_reporting_topology_on(&mut tx, operation_id)
            .await
            .map_err(Self::repository_error)?;
        let (_, evidence_audits) = report_evidence_audit_truth_on(&mut tx, operation_id, topology)
            .await
            .map_err(Self::repository_error)?;
        let (_, cleanup_blocked_decisions) =
            report_cleanup_blocked_decision_truth_on(&mut tx, operation_id)
                .await
                .map_err(Self::repository_error)?;
        let (context, organizations) = report_operation_context_on(&mut tx, operation_id)
            .await
            .map_err(Self::repository_error)?;
        if let Some(authority) = &self.project_authority {
            if context.project_scope_id != authority.project_scope_id
                || context.scope_snapshot_id != authority.scope_snapshot_id
                || context.scope_snapshot_hash != authority.scope_snapshot_hash
            {
                return Err(ReportingAppError::SourceSnapshotStale);
            }
            require_reporting_project_authority_on(&mut tx, operation_id, authority, false)
                .await
                .map_err(Self::repository_error)?;
        }
        let existing_report = sqlx::query_as::<_, golish_db::repo::reports::ReportRow>(
            "SELECT * FROM reports WHERE operation_id=$1",
        )
        .bind(operation_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(Self::repository_error)?;
        let (report_id, expected_current_revision_id, revision_number) = if let Some(report) =
            existing_report
        {
            if report.project_scope_id != context.project_scope_id
                || report.scope_snapshot_id != context.scope_snapshot_id
                || report.scope_snapshot_hash != context.scope_snapshot_hash
            {
                return Err(Self::repository_error("report_scope_identity_mismatch"));
            }
            let next_number: i32 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(revision_number),0)+1 FROM report_revisions WHERE report_id=$1",
                )
                .bind(report.report_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(Self::repository_error)?;
            (report.report_id, report.current_revision_id, next_number)
        } else {
            (
                Uuid::new_v5(
                    &Uuid::NAMESPACE_OID,
                    format!("golish-report:{operation_id}").as_bytes(),
                ),
                None,
                1,
            )
        };
        let revision_id = Uuid::new_v5(
            &report_id,
            format!(
                "revision:{revision_number}:{}",
                hex(&source_snapshot.source_set_hash)
            )
            .as_bytes(),
        );
        let seeds = report_claim_seeds_on(&mut tx, operation_id, topology)
            .await
            .map_err(Self::repository_error)?;
        let typed_seeds = typed_report_claim_seeds_on(&mut tx, operation_id)
            .await
            .map_err(Self::repository_error)?;
        let model = build_report_read_model(
            report_id,
            revision_id,
            operation_id,
            &context,
            &organizations,
            source_snapshot,
            seeds,
            typed_seeds,
        )
        .map_err(Self::repository_error)?;
        let allowed_organization_ids = organizations
            .iter()
            .map(|organization| organization.organization_id)
            .collect::<BTreeSet<_>>();
        let cleanup = cleanup_closeout_truth_on(&mut tx, operation_id, &allowed_organization_ids)
            .await
            .map_err(Self::repository_error)?;
        tx.commit().await.map_err(Self::repository_error)?;
        if self.persistence_mode == ReportPersistenceMode::Publishable {
            match context.tool_truth_contract.as_str() {
                "receipt_v1" => {
                    for organization in &organizations {
                        let authority_request = golish_db::repo::capability_execution_receipts::CheckToolTruthAuthorityBundle {
                            stable_consumer_request_id: Uuid::new_v5(
                                &revision_id,
                                format!("report-tool-truth:{}", organization.organization_id)
                                    .as_bytes(),
                            ),
                            operation_id,
                            organization_id: organization.organization_id,
                            consumer_kind: golish_db::repo::capability_execution_receipts::ToolTruthAuthorityBundleConsumerV1::CurrentReport,
                        };
                        golish_db::repo::capability_execution_receipts::with_all_fresh_tool_truth_authority_bundle(
                            &self.pool,
                            &authority_request,
                            |_authority_tx, _authority| {
                                Box::pin(async { Ok::<(), golish_db::DbError>(()) })
                            },
                        )
                        .await
                        .map_err(Self::repository_error)?;
                    }
                }
                "legacy_v1" | "shadow_v1" => {}
                other => {
                    return Err(Self::repository_error(format!(
                        "report_tool_truth_contract_unknown:{other}"
                    )));
                }
            }
        }
        let validation_truth = ReportValidationTruth {
            current_revision_id: revision_id,
            validation_status: ValidationStatus::Validated,
            publication_status: PublicationStatus::Unpublished,
            allowed_organization_ids,
            current_sources: model.source_snapshot.ordered_sources.clone(),
            evidence_audits,
            cleanup_blocked_decisions,
            cleanup,
        };
        Ok(BuiltReportRevision {
            model,
            validation_truth,
            revision_number,
            expected_current_revision_id,
            expected_row_version: 1,
        })
    }

    async fn current_source_snapshot(
        &self,
        operation_id: Uuid,
    ) -> Result<ReportSourceSnapshot, ReportingAppError> {
        let Some(authority) = &self.project_authority else {
            return current_reportable_source_snapshot(&self.pool, operation_id)
                .await
                .map_err(Self::repository_error);
        };
        let mut tx = self.pool.begin().await.map_err(Self::repository_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(Self::repository_error)?;
        let current = current_reportable_source_snapshot_on(&mut tx, operation_id)
            .await
            .map_err(Self::repository_error)?;
        require_reporting_project_authority_on(&mut tx, operation_id, authority, false)
            .await
            .map_err(Self::repository_error)?;
        tx.commit().await.map_err(Self::repository_error)?;
        Ok(current)
    }

    async fn persist_validated_revision(
        &self,
        revision: &BuiltReportRevision,
        validation_result: &ReportValidationResult,
    ) -> Result<i64, ReportingAppError> {
        let mut tx = self.pool.begin().await.map_err(Self::repository_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(Self::repository_error)?;
        let current = current_reportable_source_snapshot_on(&mut tx, revision.model.operation_id)
            .await
            .map_err(Self::repository_error)?;
        if current.ordered_sources != revision.model.source_snapshot.ordered_sources
            || current.source_set_hash != revision.model.source_snapshot.source_set_hash
        {
            return Err(ReportingAppError::SourceSnapshotStale);
        }
        if let Some(authority) = &self.project_authority {
            if revision.model.project_scope_id != authority.project_scope_id
                || revision.model.scope_snapshot_id != authority.scope_snapshot_id
                || revision.model.scope_snapshot_hash != authority.scope_snapshot_hash
            {
                return Err(ReportingAppError::SourceSnapshotStale);
            }
            require_reporting_project_authority_on(
                &mut tx,
                revision.model.operation_id,
                authority,
                true,
            )
            .await
            .map_err(Self::repository_error)?;
        }

        let existing_report = sqlx::query_as::<_, golish_db::repo::reports::ReportRow>(
            "SELECT * FROM reports WHERE operation_id=$1 FOR UPDATE",
        )
        .bind(revision.model.operation_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(Self::repository_error)?;
        match existing_report {
            Some(report) => {
                if report.report_id != revision.model.report_id
                    || report.project_scope_id != revision.model.project_scope_id
                    || report.scope_snapshot_id != revision.model.scope_snapshot_id
                    || report.scope_snapshot_hash != revision.model.scope_snapshot_hash
                    || report.current_revision_id != revision.expected_current_revision_id
                {
                    return Err(ReportingAppError::SourceSnapshotStale);
                }
            }
            None => {
                if revision.expected_current_revision_id.is_some() {
                    return Err(ReportingAppError::SourceSnapshotStale);
                }
                golish_db::repo::reports::create(
                    &mut tx,
                    &golish_db::repo::reports::CreateReport {
                        report_id: revision.model.report_id,
                        operation_id: revision.model.operation_id,
                        project_scope_id: revision.model.project_scope_id,
                        scope_snapshot_id: revision.model.scope_snapshot_id,
                        scope_snapshot_hash: revision.model.scope_snapshot_hash.clone(),
                    },
                )
                .await
                .map_err(Self::repository_error)?;
            }
        }
        golish_db::repo::report_revisions::begin_revision(
            &mut tx,
            &golish_db::repo::report_revisions::BeginReportRevision {
                revision_id: revision.model.revision_id,
                report_id: revision.model.report_id,
                revision_number: revision.revision_number,
                expected_report_current_revision_id: revision.expected_current_revision_id,
                snapshot: revision.model.source_snapshot.clone(),
            },
        )
        .await
        .map_err(Self::repository_error)?;
        golish_db::repo::report_revisions::store_read_model(&mut tx, &revision.model)
            .await
            .map_err(Self::repository_error)?;
        if self.persistence_mode == ReportPersistenceMode::Publishable {
            let report_input_seal =
                golish_db::repo::report_input_authority::seal_current_report_input_authority_on(
                    &mut tx,
                    revision.model.operation_id,
                    revision.model.revision_id,
                    &revision.model.source_snapshot,
                )
                .await
                .map_err(Self::repository_error)?;
            golish_db::repo::report_input_seals::seal_report_input_on(
                &mut tx,
                revision.model.operation_id,
                revision.model.revision_id,
                &revision.model.source_snapshot,
                &report_input_seal,
            )
            .await
            .map_err(Self::repository_error)?;
        }
        let draft_row_version: i64 =
            sqlx::query_scalar("SELECT row_version FROM report_revisions WHERE revision_id=$1")
                .bind(revision.model.revision_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(Self::repository_error)?;
        let validated = golish_db::repo::report_revisions::validate_revision(
            &mut tx,
            &golish_db::repo::report_revisions::ValidateReportRevision {
                report_id: revision.model.report_id,
                revision_id: revision.model.revision_id,
                expected_row_version: draft_row_version,
                expected_source_set_hash: revision.model.source_snapshot.source_set_hash,
                validation_result: validation_result.clone(),
            },
        )
        .await
        .map_err(Self::repository_error)?;
        tx.commit().await.map_err(Self::repository_error)?;
        Ok(validated.row_version)
    }
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct StoredReportArtifactView {
    pub revision_id: Uuid,
    pub artifact_kind: String,
    pub content_key: String,
    pub sha256: String,
    pub byte_len: i64,
    pub redaction_version: i32,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub(super) struct StoredReportGateIntegrity {
    pub claim_count: i64,
    pub citation_count: i64,
    pub source_count: i64,
    pub uncited_claim_count: i64,
    pub invalid_citation_count: i64,
    pub invalid_blocked_residual_count: i64,
    pub invalid_technique_claim_count: i64,
    pub out_of_scope_section_count: i64,
}

pub(super) async fn load_report_gate_integrity_on(
    connection: &mut PgConnection,
    revision_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
) -> anyhow::Result<StoredReportGateIntegrity> {
    Ok(sqlx::query_as::<_, StoredReportGateIntegrity>(
        r#"SELECT
              (SELECT COUNT(*) FROM report_claims WHERE revision_id=$1) AS claim_count,
              (SELECT COUNT(*) FROM report_claim_citations WHERE revision_id=$1)
                  AS citation_count,
              (SELECT COUNT(*) FROM report_source_manifest WHERE revision_id=$1)
                  AS source_count,
              (SELECT COUNT(*)
                 FROM report_claims AS claim
                WHERE claim.revision_id=$1
                  AND NOT EXISTS (
                      SELECT 1 FROM report_claim_citations AS citation
                       WHERE citation.revision_id=claim.revision_id
                         AND citation.claim_id=claim.claim_id
                  )) AS uncited_claim_count,
              (SELECT COUNT(*)
                 FROM report_claim_citations AS citation
                 JOIN report_claims AS claim
                   ON claim.revision_id=citation.revision_id
                  AND claim.claim_id=citation.claim_id
                 JOIN report_sections AS section
                   ON section.revision_id=claim.revision_id
                  AND section.section_id=claim.section_id
                 LEFT JOIN audit_log AS evidence ON evidence.id=citation.evidence_audit_id
                 LEFT JOIN report_source_manifest AS evidence_source
                   ON evidence_source.revision_id=citation.revision_id
                  AND evidence_source.source_kind='evidence_audit'
                  AND evidence_source.source_id_kind='int64'
                  AND evidence_source.source_id_value=citation.evidence_audit_id::text
                  AND evidence_source.source_row_version=0
                WHERE citation.revision_id=$1
                  AND (
                      (citation.evidence_audit_id IS NULL AND NOT (
                          citation.source_type='canonical_fact'
                          AND (
                              (claim.object_value->>'kind'='security_verdict'
                               AND claim.authority_class IN (
                                   'security_verdict_authority',
                                   'grandfathered_legacy_security_verdict'
                               ))
                              OR (claim.object_value->>'kind'='coverage'
                                  AND claim.authority_class='coverage_authority')
                              OR (claim.object_value->>'kind'='limitation'
                                  AND claim.authority_class='method_audit_only'
                                  AND citation.source_kind IN (
                                      'hypothesis_residual','investigation_closure_residual',
                                      'legacy_report_authority_seal',
                                      'input_processing_disposition'
                                  ))
                          )
                      ))
                      OR (citation.evidence_audit_id IS NOT NULL AND (
                          evidence.id IS NULL
                          OR evidence_source.revision_id IS NULL
                          OR evidence.audit_role IS DISTINCT FROM 'evidence'
                          OR evidence.run_id IS DISTINCT FROM $2
                          OR (evidence.detail ? 'organization_id'
                              AND evidence.detail->>'organization_id'
                                  IS DISTINCT FROM citation.organization_id_at_time::text)
                          OR claim.organization_id_at_time
                             IS DISTINCT FROM citation.organization_id_at_time
                          OR section.organization_id_at_time
                             IS DISTINCT FROM claim.organization_id_at_time
                      ))
                  )) AS invalid_citation_count,
              ((SELECT COUNT(*)
                  FROM cleanup_blocked_decisions AS decision
                 WHERE decision.operation_id=$2
                   AND NOT EXISTS (
                       SELECT 1 FROM report_claims AS claim
                        WHERE claim.revision_id=$1
                          AND claim.claim_kind='cleanup_residual'
                          AND claim.subject_ref=
                              'cleanup_obligation:' || decision.obligation_id::text
                          AND claim.predicate='residual_risk'
                          AND claim.organization_id_at_time=
                              decision.organization_id_at_time
                          AND claim.object_value->>'kind'='limitation'
                          AND claim.object_value->>'reasonCode'=decision.reason
                          AND claim.object_value->'affectedInputIds'=
                              jsonb_build_array(decision.obligation_id::text)
                          AND claim.object_value->'residualIds'='[]'::jsonb
                          AND claim.object_value->>'ownerCode'='cleanup_authority'
                          AND claim.object_value->>'nextActionCode'=
                              'resolve_cleanup_blocked'
                          AND EXISTS (
                              SELECT 1 FROM cleanup_blocked_decision_evidence AS link
                               WHERE link.blocked_decision_id=decision.id
                                 AND link.role='decision'
                          )
                          AND NOT EXISTS (
                              SELECT 1 FROM report_claim_citations AS citation
                               WHERE citation.revision_id=claim.revision_id
                                 AND citation.claim_id=claim.claim_id
                                 AND (
                                     citation.source_kind<>'cleanup_blocked_decision'
                                     OR citation.source_id_kind<>'uuid'
                                     OR citation.source_id_value<>decision.id::text
                                     OR citation.source_row_version<>0
                                     OR citation.organization_id_at_time<>
                                        decision.organization_id_at_time
                                 )
                          )
                          AND ARRAY(
                              SELECT citation.evidence_audit_id
                                FROM report_claim_citations AS citation
                               WHERE citation.revision_id=claim.revision_id
                                 AND citation.claim_id=claim.claim_id
                               ORDER BY citation.evidence_audit_id
                          )=ARRAY(
                              SELECT DISTINCT link.evidence_id
                                FROM cleanup_blocked_decision_evidence AS link
                               WHERE link.blocked_decision_id=decision.id
                               ORDER BY link.evidence_id
                          )
                   ))
               +
               (SELECT COUNT(*)
                  FROM report_claims AS claim
                 WHERE claim.revision_id=$1
                   AND claim.claim_kind='cleanup_residual'
                   AND claim.object_value->>'kind'='limitation'
                   AND claim.object_value->>'nextActionCode'='resolve_cleanup_blocked'
                   AND NOT EXISTS (
                       SELECT 1 FROM cleanup_blocked_decisions AS decision
                        WHERE decision.operation_id=$2
                          AND claim.subject_ref=
                              'cleanup_obligation:' || decision.obligation_id::text
                          AND claim.predicate='residual_risk'
                          AND claim.organization_id_at_time=
                              decision.organization_id_at_time
                          AND claim.object_value->>'reasonCode'=decision.reason
                          AND claim.object_value->'affectedInputIds'=
                              jsonb_build_array(decision.obligation_id::text)
                          AND claim.object_value->'residualIds'='[]'::jsonb
                          AND claim.object_value->>'ownerCode'='cleanup_authority'
                          AND NOT EXISTS (
                              SELECT 1 FROM report_claim_citations AS citation
                               WHERE citation.revision_id=claim.revision_id
                                 AND citation.claim_id=claim.claim_id
                                 AND (
                                     citation.source_kind<>'cleanup_blocked_decision'
                                     OR citation.source_id_kind<>'uuid'
                                     OR citation.source_id_value<>decision.id::text
                                     OR citation.source_row_version<>0
                                     OR citation.organization_id_at_time<>
                                        decision.organization_id_at_time
                                 )
                          )
                          AND ARRAY(
                              SELECT citation.evidence_audit_id
                                FROM report_claim_citations AS citation
                               WHERE citation.revision_id=claim.revision_id
                                 AND citation.claim_id=claim.claim_id
                               ORDER BY citation.evidence_audit_id
                          )=ARRAY(
                              SELECT DISTINCT link.evidence_id
                                FROM cleanup_blocked_decision_evidence AS link
                               WHERE link.blocked_decision_id=decision.id
                               ORDER BY link.evidence_id
                          )
                   ))) AS invalid_blocked_residual_count,
              ((SELECT COUNT(*)
                  FROM technique_outcomes AS outcome
                 WHERE outcome.run_id=$2::text
                   AND EXISTS (
                       SELECT 1 FROM operation_org_scope_units AS unit
                        WHERE unit.snapshot_id=$3
                          AND unit.organization_id=outcome.organization_id
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM report_claims AS claim
                        WHERE claim.revision_id=$1
                          AND claim.claim_kind='technique_outcome'
                          AND claim.subject_ref='technique_outcome:' || outcome.id::text
                          AND claim.predicate='outcome'
                          AND claim.organization_id_at_time=outcome.organization_id
                          AND claim.object_value->>'kind'='observation_audit'
                          AND claim.object_value->>'sourceId'=
                              'technique_outcome:' || outcome.id::text
                          AND claim.object_value->>'provenance'='technique_outcome'
                          AND claim.object_value->>'outcomeCode'='outcome'
                          AND claim.object_value->>'sourceHash' ~ '^sha256:[0-9a-f]{64}$'
                          AND NOT EXISTS (
                              SELECT 1 FROM report_claim_citations AS citation
                               WHERE citation.revision_id=claim.revision_id
                                 AND citation.claim_id=claim.claim_id
                                 AND (
                                     citation.source_kind<>'technique_outcome'
                                     OR citation.source_id_kind<>'int64'
                                     OR citation.source_id_value<>outcome.id::text
                                     OR citation.source_row_version<>outcome.row_version
                                     OR citation.organization_id_at_time<>
                                        outcome.organization_id
                                 )
                          )
                          AND ARRAY(
                              SELECT citation.evidence_audit_id
                                FROM report_claim_citations AS citation
                               WHERE citation.revision_id=claim.revision_id
                                 AND citation.claim_id=claim.claim_id
                               ORDER BY citation.evidence_audit_id
                          )=ARRAY(
                              SELECT DISTINCT evidence_id
                                FROM unnest(outcome.evidence_ids) AS evidence(evidence_id)
                               ORDER BY evidence_id
                          )
                   ))
               +
               (SELECT COUNT(*)
                  FROM report_claims AS claim
                 WHERE claim.revision_id=$1
                   AND claim.claim_kind='technique_outcome'
                   AND NOT EXISTS (
                       SELECT 1 FROM technique_outcomes AS outcome
                        WHERE outcome.run_id=$2::text
                          AND EXISTS (
                              SELECT 1 FROM operation_org_scope_units AS unit
                               WHERE unit.snapshot_id=$3
                                 AND unit.organization_id=outcome.organization_id
                          )
                          AND claim.subject_ref='technique_outcome:' || outcome.id::text
                          AND claim.predicate='outcome'
                          AND claim.organization_id_at_time=outcome.organization_id
                          AND claim.object_value->>'kind'='observation_audit'
                          AND claim.object_value->>'sourceId'=
                              'technique_outcome:' || outcome.id::text
                          AND claim.object_value->>'provenance'='technique_outcome'
                          AND claim.object_value->>'outcomeCode'='outcome'
                          AND claim.object_value->>'sourceHash'
                              ~ '^sha256:[0-9a-f]{64}$'
                          AND NOT EXISTS (
                              SELECT 1 FROM report_claim_citations AS citation
                               WHERE citation.revision_id=claim.revision_id
                                 AND citation.claim_id=claim.claim_id
                                 AND (
                                     citation.source_kind<>'technique_outcome'
                                     OR citation.source_id_kind<>'int64'
                                     OR citation.source_id_value<>outcome.id::text
                                     OR citation.source_row_version<>outcome.row_version
                                     OR citation.organization_id_at_time<>
                                        outcome.organization_id
                                 )
                          )
                          AND ARRAY(
                              SELECT citation.evidence_audit_id
                                FROM report_claim_citations AS citation
                               WHERE citation.revision_id=claim.revision_id
                                 AND citation.claim_id=claim.claim_id
                               ORDER BY citation.evidence_audit_id
                          )=ARRAY(
                              SELECT DISTINCT evidence_id
                                FROM unnest(outcome.evidence_ids) AS evidence(evidence_id)
                               ORDER BY evidence_id
                          )
                   ))) AS invalid_technique_claim_count,
              (SELECT COUNT(*)
                 FROM report_sections AS section
                 LEFT JOIN operation_org_scope_units AS unit
                   ON unit.snapshot_id=$3
                  AND unit.organization_id=section.organization_id_at_time
                WHERE section.revision_id=$1
                  AND section.organization_id_at_time IS NOT NULL
                  AND unit.organization_id IS NULL) AS out_of_scope_section_count"#,
    )
    .bind(revision_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .fetch_one(&mut *connection)
    .await?)
}

#[derive(Clone, Debug)]
pub struct StoredReportBundle {
    pub report: golish_db::repo::reports::ReportRow,
    pub revisions: Vec<golish_db::repo::report_revisions::ReportRevisionRow>,
    pub current_revision: Option<golish_db::repo::report_revisions::ReportRevisionRow>,
    pub sections: Vec<golish_db::repo::report_sections::ReportSectionRow>,
    pub claims: Vec<golish_db::repo::report_claims::ReportClaimRow>,
    pub citations: Vec<golish_db::repo::report_claim_citations::ReportClaimCitationRow>,
    pub artifacts: Vec<StoredReportArtifactView>,
    pub source_snapshot: Option<ReportSourceSnapshot>,
}

pub async fn load_report_bundle(
    pool: &PgPool,
    operation_id: Uuid,
) -> anyhow::Result<Option<StoredReportBundle>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let bundle = load_report_bundle_on(&mut tx, operation_id).await?;
    if let Some(revision_id) = bundle
        .as_ref()
        .and_then(|bundle| bundle.current_revision.as_ref())
        .map(|revision| revision.revision_id)
    {
        validate_persisted_report_or_summary_authority_on(&mut tx, operation_id, revision_id)
            .await?;
    }
    tx.commit().await?;
    Ok(bundle)
}

pub(super) async fn validate_persisted_report_or_summary_authority_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    revision_id: Uuid,
) -> anyhow::Result<()> {
    let input_seal_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM report_input_seals WHERE revision_id=$1")
            .bind(revision_id)
            .fetch_one(&mut *connection)
            .await?;
    match input_seal_count {
        1 => {
            golish_db::repo::report_input_authority::validate_persisted_report_input_authority_on(
                connection,
                operation_id,
                revision_id,
            )
            .await?;
            Ok(())
        }
        0 => {
            let summary_valid: bool = sqlx::query_scalar(
                r#"SELECT EXISTS(
                       SELECT 1
                         FROM report_revisions revision
                         JOIN reports report ON report.report_id=revision.report_id
                        WHERE revision.revision_id=$1
                          AND report.operation_id=$2
                          AND report.current_revision_id=revision.revision_id
                          AND revision.validation_status='validated'
                          AND revision.publication_status='unpublished'
                          AND revision.validation_result IS NOT NULL
                          AND revision.finalized_at IS NULL
                          AND EXISTS(
                              SELECT 1 FROM report_source_manifest source
                               WHERE source.revision_id=revision.revision_id
                          )
                          AND NOT EXISTS(
                              SELECT 1 FROM report_revision_artifacts artifact
                               WHERE artifact.revision_id=revision.revision_id
                          )
                          AND NOT EXISTS(
                              SELECT 1 FROM report_authority_invalidation_events invalidation
                               WHERE invalidation.report_revision_id=revision.revision_id
                                 AND invalidation.operation_id=report.operation_id
                          )
                   )"#,
            )
            .bind(revision_id)
            .bind(operation_id)
            .fetch_one(&mut *connection)
            .await?;
            if !summary_valid {
                anyhow::bail!("REPORT_SUMMARY_AUTHORITY_INVALID");
            }
            Ok(())
        }
        _ => anyhow::bail!("REPORT_INPUT_SEAL_NOT_UNIQUE"),
    }
}

pub(super) fn is_report_or_summary_authority_rejection(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("REPORT_INPUT_") || message.contains("REPORT_SUMMARY_")
}

pub(super) async fn load_report_bundle_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
) -> anyhow::Result<Option<StoredReportBundle>> {
    let Some(report) = sqlx::query_as::<_, golish_db::repo::reports::ReportRow>(
        "SELECT * FROM reports WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_optional(&mut *connection)
    .await?
    else {
        return Ok(None);
    };
    let revisions = sqlx::query_as::<_, golish_db::repo::report_revisions::ReportRevisionRow>(
        "SELECT * FROM report_revisions WHERE report_id=$1 ORDER BY revision_number",
    )
    .bind(report.report_id)
    .fetch_all(&mut *connection)
    .await?;
    let current_revision = report.current_revision_id.and_then(|id| {
        revisions
            .iter()
            .find(|revision| revision.revision_id == id)
            .cloned()
    });
    let Some(current_id) = report.current_revision_id else {
        return Ok(Some(StoredReportBundle {
            report,
            revisions,
            current_revision,
            sections: Vec::new(),
            claims: Vec::new(),
            citations: Vec::new(),
            artifacts: Vec::new(),
            source_snapshot: None,
        }));
    };
    let sections = sqlx::query_as::<_, golish_db::repo::report_sections::ReportSectionRow>(
        "SELECT * FROM report_sections WHERE revision_id=$1 ORDER BY ordinal,section_id",
    )
    .bind(current_id)
    .fetch_all(&mut *connection)
    .await?;
    let claims = sqlx::query_as::<_, golish_db::repo::report_claims::ReportClaimRow>(
        "SELECT * FROM report_claims WHERE revision_id=$1 ORDER BY section_id,ordinal",
    )
    .bind(current_id)
    .fetch_all(&mut *connection)
    .await?;
    let citations =
        sqlx::query_as::<_, golish_db::repo::report_claim_citations::ReportClaimCitationRow>(
            r#"SELECT * FROM report_claim_citations
                WHERE revision_id=$1 ORDER BY claim_id,citation_ordinal"#,
        )
        .bind(current_id)
        .fetch_all(&mut *connection)
        .await?;
    let manifest =
        sqlx::query_as::<_, golish_db::repo::report_source_manifest::ReportSourceManifestRow>(
            "SELECT * FROM report_source_manifest WHERE revision_id=$1 ORDER BY ordinal",
        )
        .bind(current_id)
        .fetch_all(&mut *connection)
        .await?;
    let ordered_sources = manifest
        .into_iter()
        .map(golish_db::repo::report_source_manifest::row_to_source)
        .collect::<golish_db::Result<Vec<_>>>()?;
    let source_snapshot = Some(
        ReportSourceSnapshot::freeze("stored", ordered_sources)
            .map_err(|error| anyhow::anyhow!(error))?,
    );
    let artifacts = sqlx::query_as::<_, StoredReportArtifactView>(
        r#"SELECT link.revision_id,link.artifact_kind,link.content_key,
                  blob.sha256,blob.byte_len,link.redaction_version
             FROM report_revision_artifacts AS link
             JOIN report_artifact_blobs AS blob ON blob.content_key=link.content_key
            WHERE link.revision_id=$1
            ORDER BY link.artifact_kind"#,
    )
    .bind(current_id)
    .fetch_all(&mut *connection)
    .await?;
    Ok(Some(StoredReportBundle {
        report,
        revisions,
        current_revision,
        sections,
        claims,
        citations,
        artifacts,
        source_snapshot,
    }))
}

#[derive(Debug)]
struct LockedFinalizationTruth {
    revision: golish_db::repo::report_revisions::ReportRevisionRow,
    claims: Vec<golish_db::repo::report_claims::ReportClaimRow>,
    citations: Vec<golish_db::repo::report_claim_citations::ReportClaimCitationRow>,
}

async fn lock_finalization_truth_on(
    connection: &mut PgConnection,
    report_id: Uuid,
    revision_id: Uuid,
) -> anyhow::Result<Option<LockedFinalizationTruth>> {
    let Some(revision) = sqlx::query_as::<_, golish_db::repo::report_revisions::ReportRevisionRow>(
        "SELECT * FROM report_revisions WHERE report_id=$1 AND revision_id=$2 FOR UPDATE",
    )
    .bind(report_id)
    .bind(revision_id)
    .fetch_optional(&mut *connection)
    .await?
    else {
        return Ok(None);
    };

    // Lock every mutable child row before revalidation. The parent FOR UPDATE
    // lock also conflicts with FK checks for concurrent child INSERTs.
    let _manifest_ordinals = sqlx::query_scalar::<_, i32>(
        "SELECT ordinal FROM report_source_manifest WHERE revision_id=$1 FOR SHARE",
    )
    .bind(revision_id)
    .fetch_all(&mut *connection)
    .await?;
    let _section_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT section_id FROM report_sections WHERE revision_id=$1 FOR SHARE",
    )
    .bind(revision_id)
    .fetch_all(&mut *connection)
    .await?;
    let claims = sqlx::query_as::<_, golish_db::repo::report_claims::ReportClaimRow>(
        r#"SELECT * FROM report_claims
            WHERE revision_id=$1 ORDER BY section_id,ordinal FOR SHARE"#,
    )
    .bind(revision_id)
    .fetch_all(&mut *connection)
    .await?;
    let citations =
        sqlx::query_as::<_, golish_db::repo::report_claim_citations::ReportClaimCitationRow>(
            r#"SELECT * FROM report_claim_citations
            WHERE revision_id=$1 ORDER BY claim_id,citation_ordinal FOR SHARE"#,
        )
        .bind(revision_id)
        .fetch_all(&mut *connection)
        .await?;
    let evidence_ids = citations
        .iter()
        .filter_map(|citation| citation.evidence_audit_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if !evidence_ids.is_empty() {
        let _locked_evidence_ids = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM audit_log WHERE id=ANY($1) ORDER BY id FOR SHARE",
        )
        .bind(&evidence_ids)
        .fetch_all(&mut *connection)
        .await?;
    }

    Ok(Some(LockedFinalizationTruth {
        revision,
        claims,
        citations,
    }))
}

async fn finalization_integrity_is_valid_on(
    connection: &mut PgConnection,
    report: &golish_db::repo::reports::ReportRow,
    command: &FinalizePublication,
    locked: &LockedFinalizationTruth,
) -> anyhow::Result<bool> {
    let revision = &locked.revision;
    if report.current_revision_id != Some(command.revision_id)
        || revision.report_id != report.report_id
        || revision.row_version != command.expected_row_version
        || revision.validation_status != "validated"
        || revision.publication_status != "unpublished"
        || revision.validated_at.is_none()
    {
        return Ok(false);
    }

    let integrity = load_report_gate_integrity_on(
        &mut *connection,
        command.revision_id,
        command.operation_id,
        report.scope_snapshot_id,
    )
    .await?;
    let validation_result = revision
        .validation_result
        .clone()
        .and_then(|value| serde_json::from_value::<ReportValidationResult>(value).ok());
    let attestation_valid = validation_result.as_ref().is_some_and(|result| {
        result.revision_id == command.revision_id
            && i64::try_from(result.claim_count).ok() == Some(integrity.claim_count)
            && i64::try_from(result.citation_count).ok() == Some(integrity.citation_count)
            && i64::try_from(result.source_count).ok() == Some(integrity.source_count)
            && super::reporting_gate::stored_claim_hashes_are_valid(
                &locked.claims,
                &locked.citations,
            )
            .unwrap_or(false)
    });
    if !attestation_valid
        || integrity.uncited_claim_count != 0
        || integrity.invalid_citation_count != 0
        || integrity.invalid_blocked_residual_count != 0
        || integrity.invalid_technique_claim_count != 0
        || integrity.out_of_scope_section_count != 0
    {
        return Ok(false);
    }

    let disclosed_residuals = locked
        .claims
        .iter()
        .filter(|claim| claim.claim_kind == "cleanup_residual")
        .filter_map(|claim| {
            claim
                .subject_ref
                .strip_prefix("cleanup_obligation:")
                .and_then(|value| Uuid::parse_str(value).ok())
        })
        .collect::<BTreeSet<_>>();
    let organization_ids =
        frozen_organization_ids_on(&mut *connection, command.operation_id).await?;
    if organization_ids.is_empty() {
        return Ok(false);
    }
    for organization_id in organization_ids {
        let cleanup = golish_db::repo::organization_deletion_jobs::cleanup_closeout_gate_on(
            &mut *connection,
            command.operation_id,
            organization_id,
        )
        .await?;
        if cleanup.missing_obligation_count != 0
            || cleanup.nonterminal_obligation_count != 0
            || cleanup.undisclosed_residual_count != 0
            || cleanup.invalid_terminal_truth_count != 0
            || !cleanup
                .residual_obligation_ids
                .is_subset(&disclosed_residuals)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Clone)]
pub struct PgReportPublicationPort {
    pool: Arc<PgPool>,
    project_authority: Option<ReportingProjectAuthority>,
}

impl PgReportPublicationPort {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self {
            pool,
            project_authority: None,
        }
    }

    pub fn with_project_authority(
        pool: Arc<PgPool>,
        project_authority: ReportingProjectAuthority,
    ) -> Self {
        Self {
            pool,
            project_authority: Some(project_authority),
        }
    }

    fn repository_error(error: impl std::fmt::Display) -> ReportingAppError {
        let message = error.to_string();
        if message.contains("report_source_snapshot_stale")
            || message.contains("report_project_authority_stale")
        {
            ReportingAppError::SourceSnapshotStale
        } else {
            ReportingAppError::Repository(message)
        }
    }

    fn artifact_ref(
        artifact: ContentAddressedArtifact,
    ) -> anyhow::Result<golish_db::repo::report_revisions::FinalizedArtifactRef> {
        Ok(golish_db::repo::report_revisions::FinalizedArtifactRef {
            artifact_kind: match artifact.format {
                golish_reporting_app::ReportFormat::Markdown => "markdown".to_string(),
                golish_reporting_app::ReportFormat::Json => "json".to_string(),
            },
            storage_path: format!(".golish/reports/blobs/{}", artifact.content_key),
            content_key: artifact.content_key,
            sha256: artifact.sha256,
            byte_len: i64::try_from(artifact.byte_len)
                .map_err(|_| anyhow::anyhow!("report_artifact_too_large"))?,
            redaction_version: 1,
        })
    }
}

#[async_trait::async_trait]
impl ReportPublicationPort for PgReportPublicationPort {
    async fn finalize_publication(
        &self,
        command: FinalizePublication,
    ) -> Result<(), ReportingAppError> {
        let artifacts = command
            .artifacts
            .iter()
            .cloned()
            .map(Self::artifact_ref)
            .collect::<anyhow::Result<Vec<_>>>()
            .map_err(|error| ReportingAppError::Artifact(error.to_string()))?;
        let mut tx = self.pool.begin().await.map_err(Self::repository_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(Self::repository_error)?;
        let report = sqlx::query_as::<_, golish_db::repo::reports::ReportRow>(
            "SELECT * FROM reports WHERE report_id=$1 FOR UPDATE",
        )
        .bind(command.report_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(Self::repository_error)?
        .ok_or_else(|| ReportingAppError::Repository("report_not_found".to_string()))?;
        if report.operation_id != command.operation_id {
            return Err(ReportingAppError::Repository(
                "report_operation_mismatch".to_string(),
            ));
        }
        if let Some(authority) = &self.project_authority {
            if report.project_scope_id != authority.project_scope_id
                || report.scope_snapshot_id != authority.scope_snapshot_id
                || report.scope_snapshot_hash != authority.scope_snapshot_hash
            {
                return Err(ReportingAppError::SourceSnapshotStale);
            }
            require_reporting_project_authority_on(&mut tx, command.operation_id, authority, true)
                .await
                .map_err(Self::repository_error)?;
        }
        let locked = lock_finalization_truth_on(&mut tx, command.report_id, command.revision_id)
            .await
            .map_err(Self::repository_error)?
            .ok_or(ReportingAppError::RevisionNotValidated)?;
        let current = current_reportable_source_snapshot_on(&mut tx, command.operation_id)
            .await
            .map_err(Self::repository_error)?;
        if current.ordered_sources != command.expected_source_snapshot.ordered_sources
            || current.source_set_hash != command.expected_source_snapshot.source_set_hash
        {
            return Err(ReportingAppError::SourceSnapshotStale);
        }
        if !finalization_integrity_is_valid_on(&mut tx, &report, &command, &locked)
            .await
            .map_err(Self::repository_error)?
        {
            return Err(ReportingAppError::RevisionNotValidated);
        }
        golish_db::repo::report_revisions::finalize_revision_with_artifacts_and_outbox(
            &mut tx,
            &golish_db::repo::report_revisions::FinalizeReportRevision {
                report_id: command.report_id,
                revision_id: command.revision_id,
                operation_id: command.operation_id,
                project_scope_id: report.project_scope_id,
                principal_id: command.principal_id,
                expected_row_version: command.expected_row_version,
                expected_source_snapshot: command.expected_source_snapshot,
                current_source_snapshot: current,
                artifacts,
            },
        )
        .await
        .map_err(Self::repository_error)?;
        tx.commit().await.map_err(Self::repository_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        eas_evidence_producer_status_is_terminal, exact_legacy_eas_liveness_claims,
        exact_legacy_eas_web_fingerprint_claims,
        exact_legacy_eas_web_fingerprint_evidence_membership,
        reporting_source_kind_is_authoritative, reporting_source_stages,
        reporting_terminal_authority_stage, technique_outcome_evidence_shape_is_reportable,
        CanonicalFactKey, CanonicalFactRef, ReportSourceKind, StageHandoffPayload,
        StageTopologyContract,
    };

    #[test]
    fn reporting_bridge_has_no_retrieval_or_graph_dependency() {
        let source = include_str!("reporting.rs");
        let graph_import = ["use ", "golish_graphiti"].concat();
        let wiki_call = ["wiki_", "search("].concat();
        assert!(!source.contains(&graph_import));
        assert!(!source.contains(&wiki_call));
    }

    #[test]
    fn reporting_residual_projection_reuses_the_closure_authority_contract() {
        let source = include_str!("reporting.rs");
        assert!(source.contains("unified_investigation_residual_has_stage_authority_v1("));
        assert!(!source.contains(
            "AND (generation_member.revision_id=residual.revision_id\n                               OR generation.candidate_snapshot_id=residual.snapshot_id)"
        ));
    }

    #[test]
    fn reporting_technique_sources_share_the_canonical_operation_run_aliases() {
        let source = include_str!("reporting.rs");
        assert!(source.contains("SELECT session.chat_session_key"));
        assert!(source.contains("WHERE task.id=$1"));
        assert!(source.contains("authorized_run_ids.push(chat_session_key.to_string())"));
        assert!(source.contains("outcome.run_id=ANY($1)"));
        assert!(source.contains("!authorized_run_ids.iter().any(|allowed| allowed == run_id)"));
    }

    #[test]
    fn reporting_coverage_counts_use_the_wave_disposition_not_consolidation_state() {
        let source = include_str!("reporting.rs");
        assert!(source.contains("WHERE dispositions.coverage_disposition='tested_complete'"));
        assert!(source.contains("WHERE dispositions.coverage_disposition='blocked'"));
        let unqualified_filter = ["FILTER(WHERE ", "disposition='tested_complete')"].concat();
        assert!(!source.contains(&unqualified_filter));
    }

    #[test]
    fn legacy_eas_liveness_compatibility_requires_the_complete_sealed_claim() {
        let organization_id = uuid::Uuid::new_v4();
        let run_id = "stage-run-exact".to_string();
        let payload = StageHandoffPayload {
            canonical_fact_refs: Vec::new(),
            typed_claims: vec![serde_json::json!({
                "kind": "discovery",
                "payload": {
                    "subject": "http://127.0.0.1:54247",
                    "evidence_ids": [8]
                }
            })],
            coverage_watermark: serde_json::json!({
                "kind": "information_coverage_v1",
                "stage": "external_attack_surface",
                "organization_id": organization_id,
                "run_id": run_id.clone(),
                "terminal_cells": 1,
                "terminal_cell_set_sha256": "1".repeat(64),
                "found": 1,
                "checked_empty": 0,
                "blocked": 0,
                "not_applicable": 0,
                "canonical_ref_total": 0,
                "canonical_ref_included": 0,
                "canonical_ref_truncated": false,
                "typed_claim_total": 1,
                "typed_claim_included": 1,
                "typed_claim_truncated": false,
                "evidence_id_total": 1,
                "evidence_id_included": 1,
                "evidence_id_truncated": false,
                "assets": ["http://127.0.0.1:54247"],
                "techniques": ["GOLISH-EAS-LIVENESS"]
            }),
            evidence_ids: vec![8],
        };
        let claims = exact_legacy_eas_liveness_claims(
            &payload,
            organization_id,
            std::slice::from_ref(&run_id),
            &std::collections::BTreeSet::from([8]),
        )
        .expect("exact immutable EAS handoff is accepted")
        .expect("liveness authority exists");
        assert_eq!(claims.0, run_id);
        assert_eq!(
            claims.1,
            std::collections::BTreeMap::from([(
                "127.0.0.1:54247".to_string(),
                std::collections::BTreeSet::from([8])
            )])
        );

        let error = exact_legacy_eas_liveness_claims(
            &payload,
            organization_id,
            &["stage-run-foreign".to_string()],
            &std::collections::BTreeSet::from([8]),
        )
        .expect_err("a foreign session alias must remain rejected");
        assert!(error
            .to_string()
            .contains("report_eas_liveness_watermark_run_invalid"));
    }

    #[test]
    fn canonical_eas_liveness_reference_does_not_require_a_legacy_typed_claim() {
        let organization_id = uuid::Uuid::new_v4();
        let run_id = "stage-run-canonical".to_string();
        let payload = StageHandoffPayload {
            canonical_fact_refs: vec![CanonicalFactRef {
                key: CanonicalFactKey::TechniqueOutcome {
                    organization_id,
                    run_id: run_id.clone(),
                    asset: "127.0.0.1:61060".to_string(),
                    technique: golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string(),
                },
                organization_id,
                observed_at: chrono::Utc::now(),
                content_sha256: "1".repeat(64),
                evidence_ids: vec![9],
            }],
            typed_claims: Vec::new(),
            coverage_watermark: serde_json::json!({
                "kind": "information_coverage_v1",
                "stage": "external_attack_surface",
                "organization_id": organization_id,
                "run_id": run_id.clone(),
                "techniques": [golish_db::repo::coverage_truth::TECH_EAS_LIVENESS]
            }),
            evidence_ids: vec![9],
        };

        assert_eq!(
            exact_legacy_eas_liveness_claims(
                &payload,
                organization_id,
                std::slice::from_ref(&run_id),
                &std::collections::BTreeSet::from([9]),
            )
            .expect("canonical authority is handled by the normal exact-reference path"),
            None
        );
    }

    #[test]
    fn legacy_eas_web_fingerprint_compatibility_requires_exact_claim_evidence() {
        let organization_id = uuid::Uuid::new_v4();
        let run_id = "stage-run-web-fingerprint".to_string();
        let payload = StageHandoffPayload {
            canonical_fact_refs: Vec::new(),
            typed_claims: vec![serde_json::json!({
                "kind": "web_fingerprint",
                "payload": {
                    "subject": "moresec.cn",
                    "evidence_ids": [341]
                }
            })],
            coverage_watermark: serde_json::json!({
                "kind": "information_coverage_v1",
                "stage": "external_attack_surface",
                "organization_id": organization_id,
                "run_id": run_id.clone(),
                "terminal_cells": 1,
                "terminal_cell_set_sha256": "1".repeat(64),
                "found": 1,
                "checked_empty": 0,
                "blocked": 0,
                "not_applicable": 0,
                "canonical_ref_total": 0,
                "canonical_ref_included": 0,
                "canonical_ref_truncated": false,
                "typed_claim_total": 1,
                "typed_claim_included": 1,
                "typed_claim_truncated": false,
                "evidence_id_total": 1,
                "evidence_id_included": 1,
                "evidence_id_truncated": false,
                "assets": ["https://moresec.cn"],
                "techniques": [golish_db::repo::coverage_truth::TECH_EAS_WEB_FP]
            }),
            evidence_ids: vec![341],
        };

        let claims = exact_legacy_eas_web_fingerprint_claims(
            &payload,
            organization_id,
            std::slice::from_ref(&run_id),
            &std::collections::BTreeSet::from([341]),
        )
        .expect("exact immutable EAS handoff is accepted")
        .expect("web fingerprint authority exists");
        assert_eq!(claims.0, run_id);
        assert_eq!(claims.1, vec![std::collections::BTreeSet::from([341])]);

        assert!(exact_legacy_eas_web_fingerprint_claims(
            &payload,
            organization_id,
            &["stage-run-foreign".to_string()],
            &std::collections::BTreeSet::from([341]),
        )
        .is_err());
        assert!(exact_legacy_eas_web_fingerprint_claims(
            &payload,
            organization_id,
            std::slice::from_ref(&run_id),
            &std::collections::BTreeSet::from([999]),
        )
        .is_err());
    }

    #[test]
    fn legacy_eas_web_fingerprint_compatibility_uses_sealed_handoff_evidence_without_a_model_claim()
    {
        let organization_id = uuid::Uuid::new_v4();
        let run_id = "stage-run-web-fingerprint".to_string();
        let payload = StageHandoffPayload {
            canonical_fact_refs: Vec::new(),
            typed_claims: vec![serde_json::json!({
                "kind": "port_scan",
                "payload": {
                    "subject": "127.0.0.1",
                    "evidence_ids": [402]
                }
            })],
            coverage_watermark: serde_json::json!({
                "kind": "information_coverage_v1",
                "stage": "external_attack_surface",
                "organization_id": organization_id,
                "run_id": run_id.clone(),
                "terminal_cells": 1,
                "terminal_cell_set_sha256": "1".repeat(64),
                "found": 1,
                "checked_empty": 0,
                "blocked": 0,
                "not_applicable": 0,
                "canonical_ref_total": 0,
                "canonical_ref_included": 0,
                "canonical_ref_truncated": false,
                "typed_claim_total": 1,
                "typed_claim_included": 1,
                "typed_claim_truncated": false,
                "evidence_id_total": 2,
                "evidence_id_included": 2,
                "evidence_id_truncated": false,
                "assets": ["https://moresec.cn:443"],
                "techniques": [golish_db::repo::coverage_truth::TECH_EAS_WEB_FP]
            }),
            evidence_ids: vec![372, 402],
        };
        let handoff_evidence = std::collections::BTreeSet::from([372, 402]);

        let claims = exact_legacy_eas_web_fingerprint_claims(
            &payload,
            organization_id,
            std::slice::from_ref(&run_id),
            &handoff_evidence,
        )
        .expect("the complete final-sealed handoff remains an authority")
        .expect("web fingerprint authority exists");
        assert_eq!(claims.0, run_id);
        assert!(claims.1.is_empty());
        assert!(exact_legacy_eas_web_fingerprint_evidence_membership(
            &claims.1,
            &handoff_evidence,
            &std::collections::BTreeSet::from([372]),
        ));
        assert!(!exact_legacy_eas_web_fingerprint_evidence_membership(
            &claims.1,
            &handoff_evidence,
            &std::collections::BTreeSet::from([999]),
        ));

        let typed_claim_evidence = vec![std::collections::BTreeSet::from([372])];
        assert!(!exact_legacy_eas_web_fingerprint_evidence_membership(
            &typed_claim_evidence,
            &handoff_evidence,
            &std::collections::BTreeSet::from([402]),
        ));
    }

    #[test]
    fn reportable_technique_outcomes_require_evidence_except_for_typed_terminal_gaps() {
        assert!(technique_outcome_evidence_shape_is_reportable(
            "found",
            &[372]
        ));
        assert!(technique_outcome_evidence_shape_is_reportable(
            "checked_empty",
            &[369]
        ));
        assert!(technique_outcome_evidence_shape_is_reportable(
            "blocked",
            &[]
        ));
        assert!(technique_outcome_evidence_shape_is_reportable(
            "not_applicable",
            &[]
        ));
        assert!(!technique_outcome_evidence_shape_is_reportable(
            "found",
            &[]
        ));
        assert!(!technique_outcome_evidence_shape_is_reportable(
            "checked_empty",
            &[]
        ));
    }

    #[test]
    fn recovered_eas_evidence_requires_matching_unit_and_worker_terminal_status() {
        assert!(eas_evidence_producer_status_is_terminal("passed", "passed"));
        assert!(eas_evidence_producer_status_is_terminal(
            "superseded",
            "superseded"
        ));
        assert!(!eas_evidence_producer_status_is_terminal(
            "superseded",
            "passed"
        ));
        assert!(!eas_evidence_producer_status_is_terminal(
            "passed",
            "superseded"
        ));
    }

    #[test]
    fn reporting_source_stages_follow_only_the_operation_frozen_topology() {
        let legacy = reporting_source_stages(StageTopologyContract::LegacyCandidateVerificationV1);
        assert!(legacy.iter().any(|stage| stage == "attack_candidate"));
        assert!(legacy.iter().any(|stage| stage == "verification"));
        assert!(!legacy
            .iter()
            .any(|stage| stage == "application_understanding"));
        assert!(!legacy.iter().any(|stage| stage == "investigation"));

        let unified = reporting_source_stages(StageTopologyContract::UnifiedInvestigationV1);
        assert!(unified
            .iter()
            .any(|stage| stage == "application_understanding"));
        assert!(unified.iter().any(|stage| stage == "investigation"));
        assert!(!unified.iter().any(|stage| stage == "attack_candidate"));
        assert!(!unified.iter().any(|stage| stage == "verification"));
    }

    #[test]
    fn frozen_topology_selects_exactly_one_legacy_or_unified_report_authority() {
        let legacy = StageTopologyContract::LegacyCandidateVerificationV1;
        assert_eq!(reporting_terminal_authority_stage(legacy), "verification");
        assert!(reporting_source_kind_is_authoritative(
            legacy,
            ReportSourceKind::LegacyReportAuthoritySeal
        ));
        assert!(!reporting_source_kind_is_authoritative(
            legacy,
            ReportSourceKind::HypothesisRevisionTerminalDecision
        ));
        assert!(!reporting_source_kind_is_authoritative(
            legacy,
            ReportSourceKind::FinalWaveCoverageReceipt
        ));

        let unified = StageTopologyContract::UnifiedInvestigationV1;
        assert_eq!(reporting_terminal_authority_stage(unified), "investigation");
        assert!(!reporting_source_kind_is_authoritative(
            unified,
            ReportSourceKind::LegacyReportAuthoritySeal
        ));
        assert!(reporting_source_kind_is_authoritative(
            unified,
            ReportSourceKind::HypothesisRevisionTerminalDecision
        ));
        assert!(reporting_source_kind_is_authoritative(
            unified,
            ReportSourceKind::FinalWaveCoverageReceipt
        ));
    }
}
