//! Reporting authority adapter seams.
//!
//! This bridge deliberately exposes only canonical DB truth. It has no RAG,
//! wiki, memory-context or Graphiti fallback. In particular, cleanup closeout
//! delegates to the Cleanup-owned deterministic gate query rather than
//! recreating its semantics in Reporting SQL.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Arc;

use golish_agent_kit::harness::handoff_catalog::{CanonicalFactKey, StageHandoffPayload};
use golish_memory_domain::source_ref::{CanonicalRowId, StoredCanonicalRowId};
use golish_reporting_domain::{
    CitationSourceType, CleanupBlockedDecisionTruth, CleanupCloseoutTruth, EvidenceAuditTruth,
    OrganizationReportSection, PublicationStatus, ReportCitation, ReportClaim, ReportClaimKind,
    ReportFinding, ReportReadModel, ReportResidual, ReportSectionKind, ReportSectionModel,
    ReportSourceKind, ReportSourceVersion, ReportValidationResult, ReportValidationTruth,
    ValidationStatus,
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

const REPORTING_SOURCE_STAGES: &[&str] = &[
    "target_intel",
    "external_attack_surface",
    "enumeration",
    "vuln_triage",
    "attack_candidate",
    "verification",
    "access_validation",
    "internal_discovery",
    "objective_pathing",
    "objective_simulation",
];

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
    row_version: i64,
    content: Value,
    evidence_ids: Vec<i64>,
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
    let source_stages = REPORTING_SOURCE_STAGES
        .iter()
        .map(|stage| (*stage).to_string())
        .collect::<Vec<_>>();
    let mut sealed_refs = BTreeMap::<TechniqueAuthorityKey, TechniqueAuthority>::new();
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
            let handoff_content = serde_json::to_value(&handoff)
                .map_err(|_| anyhow::anyhow!("report_handoff_payload_invalid"))?;
            let payload: StageHandoffPayload = serde_json::from_value(handoff.payload.clone())
                .map_err(|_| anyhow::anyhow!("report_handoff_payload_invalid"))?;
            let handoff_evidence = handoff
                .evidence_ids
                .iter()
                .copied()
                .collect::<BTreeSet<_>>();
            let mut contains_technique_ref = false;
            for reference in payload.canonical_fact_refs {
                if let CanonicalFactKey::TechniqueOutcome {
                    organization_id: ref_org,
                    run_id,
                    asset,
                    technique,
                } = reference.key
                {
                    contains_technique_ref = true;
                    if ref_org != *organization_id
                        || reference.organization_id != *organization_id
                        || run_id != operation_id.to_string()
                        || reference.evidence_ids.is_empty()
                        || reference
                            .evidence_ids
                            .iter()
                            .any(|evidence_id| !handoff_evidence.contains(evidence_id))
                        || decode_sha256(&reference.content_sha256).is_err()
                    {
                        anyhow::bail!("report_technique_handoff_authority_invalid");
                    }
                    let key = TechniqueAuthorityKey {
                        organization_id: ref_org,
                        run_id,
                        asset,
                        technique,
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
            }
            if contains_technique_ref {
                handoff_sources.push(ReportSourceVersion {
                    kind: ReportSourceKind::StageHandoff,
                    id: CanonicalRowId::Uuid(handoff.id),
                    row_version: 0,
                    content_hash: decode_sha256(&sha256(&handoff_content))?,
                });
            }
        }
    }

    let rows = sqlx::query_as::<_, TechniqueSourceRow>(
        r#"SELECT outcome.id,outcome.organization_id,outcome.run_id,outcome.asset,
                  outcome.technique,outcome.row_version,to_jsonb(outcome) AS content,
                  outcome.evidence_ids
             FROM technique_outcomes AS outcome
            WHERE outcome.run_id=$1 AND outcome.organization_id=ANY($2)
            ORDER BY outcome.organization_id,outcome.asset,outcome.technique"#,
    )
    .bind(operation_id.to_string())
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
    let rows = sqlx::query_as::<_, UuidSourceRow>(sql)
        .bind(operation_id)
        .fetch_all(&mut *connection)
        .await?;
    rows.into_iter()
        .map(|row| {
            Ok(ReportSourceVersion {
                kind,
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
) -> anyhow::Result<(Vec<ReportSourceVersion>, BTreeMap<i64, EvidenceAuditTruth>)> {
    let rows = sqlx::query_as::<_, EvidenceAuditSourceRow>(
        r#"WITH evidence_refs(evidence_id,organization_id) AS (
               SELECT episode_evidence.evidence_id,episode.organization_id_at_time
                 FROM stage_episodes AS episode
                 CROSS JOIN LATERAL unnest(episode.evidence_refs)
                   AS episode_evidence(evidence_id)
                WHERE episode.source_operation_id=$1
               UNION ALL
               SELECT link.evidence_id,attempt.organization_id
                 FROM candidate_attempts AS attempt
                 JOIN candidate_attempt_evidence AS link ON link.attempt_id=attempt.id
                WHERE attempt.operation_id=$1
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
               UNION ALL
               SELECT link.evidence_id,foothold.organization_id_at_time
                 FROM footholds AS foothold
                 JOIN foothold_evidence AS link ON link.foothold_id=foothold.id
                WHERE foothold.operation_id=$1
               UNION ALL
               SELECT link.evidence_id,observation.organization_id_at_time
                 FROM internal_asset_observations AS observation
                 JOIN internal_asset_observation_evidence AS link
                   ON link.observation_id=observation.id
                WHERE observation.operation_id=$1
               UNION ALL
               SELECT link.evidence_id,path.organization_id_at_time
                 FROM attack_paths AS path
                 JOIN attack_path_edges AS edge ON edge.attack_path_id=path.id
                 JOIN attack_path_edge_evidence AS link ON link.attack_path_edge_id=edge.id
                WHERE path.operation_id=$1
               UNION ALL
               SELECT link.evidence_id,objective.organization_id_at_time
                 FROM objective_attempts AS objective
                 JOIN objective_attempt_evidence AS link ON link.objective_attempt_id=objective.id
                WHERE objective.operation_id=$1
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
    .fetch_all(&mut *connection)
    .await?;

    let mut sources = Vec::new();
    let mut truth = BTreeMap::new();
    for row in rows {
        let source = if let Some(content) = row.content.as_ref() {
            Some(ReportSourceVersion {
                kind: ReportSourceKind::EvidenceAudit,
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
            .and_then(|value| Uuid::parse_str(value).ok());
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

pub(super) async fn current_reportable_source_snapshot_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
) -> anyhow::Result<ReportSourceSnapshot> {
    // This is deliberately the first read in the transaction so the recorded
    // identifier describes the exact PostgreSQL snapshot used below.
    let transaction_snapshot: String = sqlx::query_scalar("SELECT txid_current_snapshot()::text")
        .fetch_one(&mut *connection)
        .await?;
    let organization_ids = frozen_organization_ids_on(&mut *connection, operation_id).await?;
    if organization_ids.is_empty() {
        anyhow::bail!("report_frozen_scope_missing");
    }
    let mut sources = Vec::new();
    sources.extend(
        uuid_sources(
            &mut *connection,
            operation_id,
            ReportSourceKind::StageEpisode,
            r#"SELECT episode_id AS id,0::bigint AS row_version,to_jsonb(episode) AS content
                 FROM stage_episodes AS episode
                WHERE source_operation_id=$1
                ORDER BY episode_id"#,
        )
        .await?,
    );
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
        sources.extend(uuid_sources(&mut *connection, operation_id, kind, query).await?);
    }
    let (blocked_decision_sources, _) =
        report_cleanup_blocked_decision_truth_on(&mut *connection, operation_id).await?;
    sources.extend(blocked_decision_sources);
    let (evidence_sources, _) =
        report_evidence_audit_truth_on(&mut *connection, operation_id).await?;
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
    finding_id: Option<Uuid>,
    candidate_id: Option<Uuid>,
    lineage_id: Option<Uuid>,
    residual_obligation_id: Option<Uuid>,
    residual_status: Option<String>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct FrozenOrganizationRow {
    organization_id: Uuid,
    organization_name: String,
    ordinal: i32,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct ReportOperationContextRow {
    project_scope_id: Uuid,
    scope_snapshot_id: Uuid,
    scope_snapshot_hash: String,
}

async fn report_operation_context_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
) -> anyhow::Result<(ReportOperationContextRow, Vec<FrozenOrganizationRow>)> {
    let context = sqlx::query_as::<_, ReportOperationContextRow>(
        r#"SELECT snapshot.project_scope_id,
                  snapshot.id AS scope_snapshot_id,
                  snapshot.scope_hash AS scope_snapshot_hash
             FROM operation_org_scope_snapshots AS snapshot
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
    .fetch_all(&mut *connection)
    .await?)
}

#[derive(Clone, Debug)]
struct AccumulatedClaimSeed {
    row: ReportClaimSeedRow,
    evidence_ids: BTreeSet<i64>,
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

fn build_report_read_model(
    report_id: Uuid,
    revision_id: Uuid,
    operation_id: Uuid,
    context: &ReportOperationContextRow,
    organizations: &[FrozenOrganizationRow],
    source_snapshot: ReportSourceSnapshot,
    seeds: Vec<ReportClaimSeedRow>,
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
                row: seed.clone(),
                evidence_ids: BTreeSet::new(),
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
            let report_claim = ReportClaim {
                claim_id,
                revision_id,
                section_id,
                organization_id_at_time: Some(organization_id),
                claim_kind: claim_kind(&seed.row.claim_kind)?,
                subject_ref: seed.row.subject_ref.clone(),
                predicate: seed.row.predicate.clone(),
                value: redact_report_value(seed.row.object_value.clone())
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?,
                citation_ids,
                ordinal: i32::try_from(claim_ordinal)
                    .map_err(|_| anyhow::anyhow!("report_claim_overflow"))?,
            };
            if let Some(finding_id) = seed.row.finding_id {
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
}

impl PgReportTruthPort {
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
        let (_, evidence_audits) = report_evidence_audit_truth_on(&mut tx, operation_id)
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
        let seeds = report_claim_seeds_on(&mut tx, operation_id)
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
                      evidence.id IS NULL
                      OR evidence_source.revision_id IS NULL
                      OR evidence.audit_role IS DISTINCT FROM 'evidence'
                      OR evidence.run_id IS DISTINCT FROM $2
                      OR evidence.detail->>'organization_id'
                         IS DISTINCT FROM citation.organization_id_at_time::text
                      OR claim.organization_id_at_time
                         IS DISTINCT FROM citation.organization_id_at_time
                      OR section.organization_id_at_time
                         IS DISTINCT FROM claim.organization_id_at_time
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
                          AND claim.object_value=jsonb_build_object(
                              'status','blocked',
                              'decidedByPrincipalId',decision.decided_by_principal_id,
                              'reason',decision.reason,
                              'residualRisk',decision.residual_risk
                          )
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
                   AND claim.object_value->>'status'='blocked'
                   AND NOT EXISTS (
                       SELECT 1 FROM cleanup_blocked_decisions AS decision
                        WHERE decision.operation_id=$2
                          AND claim.subject_ref=
                              'cleanup_obligation:' || decision.obligation_id::text
                          AND claim.predicate='residual_risk'
                          AND claim.organization_id_at_time=
                              decision.organization_id_at_time
                          AND claim.object_value=jsonb_build_object(
                              'status','blocked',
                              'decidedByPrincipalId',decision.decided_by_principal_id,
                              'reason',decision.reason,
                              'residualRisk',decision.residual_risk
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
                          AND claim.object_value=jsonb_build_object(
                              'asset',outcome.asset,'technique',outcome.technique,
                              'outcome',outcome.outcome,'source',outcome.source,
                              'resultCount',outcome.result_count,
                              'confidence',outcome.confidence
                          )
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
                          AND claim.object_value=jsonb_build_object(
                              'asset',outcome.asset,'technique',outcome.technique,
                              'outcome',outcome.outcome,'source',outcome.source,
                              'resultCount',outcome.result_count,
                              'confidence',outcome.confidence
                          )
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
    tx.commit().await?;
    Ok(bundle)
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
        .map(|citation| citation.evidence_audit_id)
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
    #[test]
    fn reporting_bridge_has_no_retrieval_or_graph_dependency() {
        let source = include_str!("reporting.rs");
        let graph_import = ["use ", "golish_graphiti"].concat();
        let wiki_call = ["wiki_", "search("].concat();
        assert!(!source.contains(&graph_import));
        assert!(!source.contains(&wiki_call));
    }
}
