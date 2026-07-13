use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use golish_memory_domain::event_catalog::{
    KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, KnowledgeEventPayloadV1,
};
use golish_memory_domain::scope::ProjectScopeId;
use golish_memory_domain::source_ref::{CanonicalRowId, CanonicalSourceKind, SourceRef};
use golish_reporting_domain::{
    ReportClaimKind, ReportReadModel, ReportSectionKind, ReportSourceSnapshot,
    ReportValidationResult,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::Result;

#[derive(Clone, Debug, sqlx::FromRow, PartialEq)]
pub struct ReportRevisionRow {
    pub revision_id: Uuid,
    pub report_id: Uuid,
    pub revision_number: i32,
    pub row_version: i64,
    pub transaction_snapshot: String,
    pub source_set_hash: String,
    pub validation_status: String,
    pub publication_status: String,
    pub supersedes_revision_id: Option<Uuid>,
    pub validation_result: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub validated_at: Option<DateTime<Utc>>,
    pub finalized_at: Option<DateTime<Utc>>,
    pub finalized_by_principal_id: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginReportRevision {
    pub revision_id: Uuid,
    pub report_id: Uuid,
    pub revision_number: i32,
    pub expected_report_current_revision_id: Option<Uuid>,
    pub snapshot: ReportSourceSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidateReportRevision {
    pub report_id: Uuid,
    pub revision_id: Uuid,
    pub expected_row_version: i64,
    pub expected_source_set_hash: [u8; 32],
    pub validation_result: ReportValidationResult,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalizedArtifactRef {
    pub artifact_kind: String,
    pub content_key: String,
    pub sha256: String,
    pub storage_path: String,
    pub byte_len: i64,
    pub redaction_version: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalizeReportRevision {
    pub report_id: Uuid,
    pub revision_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub principal_id: Uuid,
    pub expected_row_version: i64,
    pub expected_source_snapshot: ReportSourceSnapshot,
    /// Must come from the server-owned ReportTruthPort re-running the complete
    /// canonical source query immediately before this transaction.
    pub current_source_snapshot: ReportSourceSnapshot,
    pub artifacts: Vec<FinalizedArtifactRef>,
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn content_hash<T: serde::Serialize>(value: &T) -> Result<String> {
    let encoded = serde_json::to_vec(value)?;
    Ok(hex(&Sha256::digest(encoded)))
}

fn section_kind(kind: ReportSectionKind) -> &'static str {
    match kind {
        ReportSectionKind::ExecutiveSummary => "executive_summary",
        ReportSectionKind::Organization => "organization",
        ReportSectionKind::Findings => "findings",
        ReportSectionKind::AttackPaths => "attack_paths",
        ReportSectionKind::CleanupResiduals => "cleanup_residuals",
        ReportSectionKind::Methodology => "methodology",
        ReportSectionKind::Limitations => "limitations",
    }
}

fn claim_kind(kind: ReportClaimKind) -> &'static str {
    match kind {
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

pub async fn begin_revision(
    tx: &mut Transaction<'_, Postgres>,
    input: &BeginReportRevision,
) -> Result<ReportRevisionRow> {
    let report_current = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT current_revision_id FROM reports WHERE report_id=$1 FOR UPDATE",
    )
    .bind(input.report_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("report_not_found"))?;
    if report_current != input.expected_report_current_revision_id {
        return Err(anyhow::anyhow!("report_current_revision_conflict").into());
    }
    let row = sqlx::query_as::<_, ReportRevisionRow>(
        r#"INSERT INTO report_revisions(
               revision_id,report_id,revision_number,transaction_snapshot,
               source_set_hash,validation_status,publication_status,
               supersedes_revision_id
           ) VALUES($1,$2,$3,$4,$5,'building','unpublished',$6)
           RETURNING *"#,
    )
    .bind(input.revision_id)
    .bind(input.report_id)
    .bind(input.revision_number)
    .bind(&input.snapshot.transaction_snapshot)
    .bind(hex(&input.snapshot.source_set_hash))
    .bind(input.expected_report_current_revision_id)
    .fetch_one(&mut **tx)
    .await?;
    super::report_source_manifest::insert_snapshot(tx, input.revision_id, &input.snapshot).await?;
    Ok(row)
}

pub async fn store_read_model(
    tx: &mut Transaction<'_, Postgres>,
    model: &ReportReadModel,
) -> Result<()> {
    let stored_source_hash: String = sqlx::query_scalar(
        "SELECT source_set_hash FROM report_revisions WHERE revision_id=$1 AND report_id=$2 FOR UPDATE",
    )
    .bind(model.revision_id)
    .bind(model.report_id)
    .fetch_one(&mut **tx)
    .await?;
    if stored_source_hash != hex(&model.source_snapshot.source_set_hash) {
        return Err(anyhow::anyhow!("report_source_snapshot_mismatch").into());
    }

    for organization in &model.organization_sections {
        let section = &organization.section;
        if section.revision_id != model.revision_id
            || section.organization_id_at_time != Some(organization.organization_id_at_time)
        {
            return Err(anyhow::anyhow!("report_section_scope_mismatch").into());
        }
        super::report_sections::insert(
            tx,
            &super::report_sections::ReportSectionRow {
                section_id: section.section_id,
                revision_id: section.revision_id,
                organization_id_at_time: section.organization_id_at_time,
                organization_name_at_snapshot: section.organization_name_at_snapshot.clone(),
                section_kind: section_kind(section.kind).to_string(),
                ordinal: section.ordinal,
                rendered_content: section.rendered_content.clone(),
                content_hash: content_hash(section)?,
            },
        )
        .await?;
        for claim in &section.claims {
            if claim.revision_id != model.revision_id
                || claim.section_id != section.section_id
                || claim.citation_ids.is_empty()
            {
                return Err(anyhow::anyhow!("report_claim_citation_required").into());
            }
            super::report_claims::insert(
                tx,
                &super::report_claims::ReportClaimRow {
                    claim_id: claim.claim_id,
                    revision_id: claim.revision_id,
                    section_id: claim.section_id,
                    organization_id_at_time: claim.organization_id_at_time,
                    claim_kind: claim_kind(claim.claim_kind).to_string(),
                    subject_ref: claim.subject_ref.clone(),
                    predicate: claim.predicate.clone(),
                    object_value: claim.value.clone(),
                    claim_hash: content_hash(claim)?,
                    ordinal: claim.ordinal,
                },
            )
            .await?;
            for citation_id in &claim.citation_ids {
                let citation = model
                    .citations
                    .iter()
                    .find(|citation| {
                        citation.citation_id == *citation_id
                            && citation.claim_id == claim.claim_id
                            && citation.revision_id == model.revision_id
                    })
                    .ok_or_else(|| anyhow::anyhow!("report_citation_unresolved"))?;
                super::report_claim_citations::insert(tx, citation).await?;
            }
        }
    }
    let written_citations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM report_claim_citations WHERE revision_id=$1")
            .bind(model.revision_id)
            .fetch_one(&mut **tx)
            .await?;
    if usize::try_from(written_citations).ok() != Some(model.citations.len()) {
        return Err(anyhow::anyhow!("report_orphan_citation_forbidden").into());
    }
    let updated = sqlx::query(
        r#"UPDATE report_revisions
              SET validation_status='draft'
            WHERE revision_id=$1 AND validation_status='building'"#,
    )
    .bind(model.revision_id)
    .execute(&mut **tx)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(anyhow::anyhow!("report_revision_store_conflict").into());
    }
    Ok(())
}

pub async fn validate_revision(
    tx: &mut Transaction<'_, Postgres>,
    input: &ValidateReportRevision,
) -> Result<ReportRevisionRow> {
    let validation_result = serde_json::to_value(&input.validation_result)?;
    let row = sqlx::query_as::<_, ReportRevisionRow>(
        r#"UPDATE report_revisions
              SET validation_status='validated',validation_result=$5,validated_at=NOW()
            WHERE revision_id=$1 AND report_id=$2 AND row_version=$3
              AND source_set_hash=$4 AND validation_status='draft'
              AND publication_status='unpublished'
            RETURNING *"#,
    )
    .bind(input.revision_id)
    .bind(input.report_id)
    .bind(input.expected_row_version)
    .bind(hex(&input.expected_source_set_hash))
    .bind(validation_result)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("report_revision_validation_conflict"))?;
    sqlx::query("UPDATE reports SET current_revision_id=$2,updated_at=NOW() WHERE report_id=$1")
        .bind(input.report_id)
        .bind(input.revision_id)
        .execute(&mut **tx)
        .await?;
    Ok(row)
}

fn exact_snapshot(expected: &ReportSourceSnapshot, actual: &ReportSourceSnapshot) -> bool {
    expected.ordered_sources == actual.ordered_sources
        && expected.source_set_hash == actual.source_set_hash
}

pub async fn finalize_revision_with_artifacts_and_outbox(
    tx: &mut Transaction<'_, Postgres>,
    input: &FinalizeReportRevision,
) -> Result<ReportRevisionRow> {
    if !exact_snapshot(
        &input.expected_source_snapshot,
        &input.current_source_snapshot,
    ) {
        return Err(anyhow::anyhow!("report_source_snapshot_stale").into());
    }
    let active_principal: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM operator_principals WHERE id=$1 AND active AND principal_kind='local_operator')",
    )
    .bind(input.principal_id)
    .fetch_one(&mut **tx)
    .await?;
    if !active_principal {
        return Err(anyhow::anyhow!("report_finalize_actor_untrusted").into());
    }
    let is_current: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM reports
                WHERE report_id=$1 AND operation_id=$2 AND project_scope_id=$3
                  AND current_revision_id=$4
           )"#,
    )
    .bind(input.report_id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(input.revision_id)
    .fetch_one(&mut **tx)
    .await?;
    if !is_current {
        return Err(anyhow::anyhow!("report_revision_not_current").into());
    }

    let manifest = super::report_source_manifest::list(tx, input.revision_id).await?;
    let manifest_sources = manifest
        .into_iter()
        .map(super::report_source_manifest::row_to_source)
        .collect::<Result<Vec<_>>>()?;
    if manifest_sources != input.current_source_snapshot.ordered_sources {
        return Err(anyhow::anyhow!("report_source_snapshot_stale").into());
    }
    if input.artifacts.is_empty() {
        return Err(anyhow::anyhow!("report_finalize_artifact_required").into());
    }

    sqlx::query(
        r#"UPDATE report_revisions
              SET publication_status='superseded'
            WHERE report_id=$1 AND publication_status='final' AND revision_id<>$2"#,
    )
    .bind(input.report_id)
    .bind(input.revision_id)
    .execute(&mut **tx)
    .await?;

    for artifact in &input.artifacts {
        super::report_artifact_blobs::put(
            tx,
            &super::report_artifact_blobs::PutReportArtifactBlob {
                content_key: artifact.content_key.clone(),
                sha256: artifact.sha256.clone(),
                storage_path: artifact.storage_path.clone(),
                byte_len: artifact.byte_len,
            },
        )
        .await?;
        super::report_revision_artifacts::attach(
            tx,
            &super::report_revision_artifacts::ReportRevisionArtifactRow {
                revision_id: input.revision_id,
                artifact_kind: artifact.artifact_kind.clone(),
                content_key: artifact.content_key.clone(),
                redaction_version: artifact.redaction_version,
                created_at: Utc::now(),
            },
        )
        .await?;
    }

    let finalized = sqlx::query_as::<_, ReportRevisionRow>(
        r#"UPDATE report_revisions
              SET publication_status='final',finalized_at=NOW(),
                  finalized_by_principal_id=$5
            WHERE revision_id=$1 AND report_id=$2 AND row_version=$3
              AND source_set_hash=$4 AND validation_status='validated'
              AND publication_status='unpublished'
            RETURNING *"#,
    )
    .bind(input.revision_id)
    .bind(input.report_id)
    .bind(input.expected_row_version)
    .bind(hex(&input.expected_source_snapshot.source_set_hash))
    .bind(input.principal_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("report_finalize_cas_conflict"))?;

    let occurred_at = finalized
        .finalized_at
        .ok_or_else(|| anyhow::anyhow!("report_finalize_timestamp_missing"))?;
    let event = KnowledgeEventEnvelopeV1 {
        event_id: Uuid::new_v5(
            &input.revision_id,
            format!("report-finalized:{}", finalized.row_version).as_bytes(),
        ),
        project_scope_id: Some(ProjectScopeId(input.project_scope_id)),
        organization_id_at_time: None,
        source_operation_id: input.operation_id,
        event_name: KnowledgeEventNameV1::ReportRevisionFinalized,
        schema_version: 1,
        payload: KnowledgeEventPayloadV1 {
            source: SourceRef {
                source_kind: CanonicalSourceKind::ReportRevision,
                row_id: CanonicalRowId::Uuid(input.revision_id),
                source_stream_key: format!("report:{}", input.report_id),
                version: finalized.row_version,
            },
            source_stream_key: format!("report:{}", input.report_id),
            source_version: finalized.row_version,
            structured_payload: json!({
                "reportId": input.report_id,
                "revisionId": input.revision_id,
                "artifacts": input.artifacts.iter().map(|artifact| json!({
                    "kind": artifact.artifact_kind,
                    "contentKey": artifact.content_key,
                    "sha256": artifact.sha256,
                    "byteLen": artifact.byte_len,
                })).collect::<Vec<_>>(),
            }),
        },
        occurred_at,
    };
    super::knowledge_outbox::append_event_with_catalog_deliveries(tx, &event)
        .await
        .map_err(|error| crate::DbError::Other(anyhow::anyhow!(error)))?;
    Ok(finalized)
}

pub async fn purge_unvalidated_draft(
    tx: &mut Transaction<'_, Postgres>,
    report_id: Uuid,
    revision_id: Uuid,
) -> Result<()> {
    let purgeable: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM report_revisions
                WHERE report_id=$1 AND revision_id=$2
                  AND validation_status IN ('building','draft')
                  AND publication_status='unpublished'
           )"#,
    )
    .bind(report_id)
    .bind(revision_id)
    .fetch_one(&mut **tx)
    .await?;
    if !purgeable {
        return Err(anyhow::anyhow!("report_revision_history_retained").into());
    }
    sqlx::query("DELETE FROM report_claim_citations WHERE revision_id=$1")
        .bind(revision_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM report_claims WHERE revision_id=$1")
        .bind(revision_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM report_sections WHERE revision_id=$1")
        .bind(revision_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM report_source_manifest WHERE revision_id=$1")
        .bind(revision_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM report_revisions WHERE revision_id=$1 AND report_id=$2")
        .bind(revision_id)
        .bind(report_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn get(pool: &PgPool, revision_id: Uuid) -> Result<Option<ReportRevisionRow>> {
    Ok(sqlx::query_as::<_, ReportRevisionRow>(
        "SELECT * FROM report_revisions WHERE revision_id=$1",
    )
    .bind(revision_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn list_for_report(pool: &PgPool, report_id: Uuid) -> Result<Vec<ReportRevisionRow>> {
    Ok(sqlx::query_as::<_, ReportRevisionRow>(
        "SELECT * FROM report_revisions WHERE report_id=$1 ORDER BY revision_number",
    )
    .bind(report_id)
    .fetch_all(pool)
    .await?)
}
