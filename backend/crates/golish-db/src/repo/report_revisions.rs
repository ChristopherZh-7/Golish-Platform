use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use golish_core::hypothesis_semantic_key::CanonicalJsonObject;
use golish_core::investigation_projection::{
    ProjectionChangeKind, ProjectionSourceSnapshotV1, ProjectionSourceTimeStatusV1,
    ReportProjectionRecordV1,
};
use golish_memory_domain::event_catalog::{
    KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, KnowledgeEventPayloadV1,
};
use golish_memory_domain::scope::ProjectScopeId;
use golish_memory_domain::source_ref::{CanonicalRowId, CanonicalSourceKind, SourceRef};
use golish_reporting_domain::{
    ReportClaimKind, ReportInputSealV1, ReportReadModel, ReportSectionKind, ReportSourceSnapshot,
    ReportValidationResult,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::Result;

use super::hypothesis_legacy_projection::{
    append_projection_source_batch_on, AppendProjectionSourceBatchRow, ProjectionOutboxSourceRow,
    ProjectionSourceStorageV1,
};

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

#[derive(sqlx::FromRow)]
struct SealedReportInputRow {
    typed_seal: Value,
    authority_contract: String,
    tool_truth_authority_set_id: Uuid,
    revision_adjudication_authority_set_id: Option<Uuid>,
    legacy_report_authority_seal_id: Option<Uuid>,
    source_member_count: i64,
    source_set_hash: Vec<u8>,
    report_input_hash: Vec<u8>,
    effective_valid_until: DateTime<Utc>,
    observed_at: DateTime<Utc>,
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

async fn append_report_projection_batch_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    project_scope_id: Uuid,
    stable_request_id: Uuid,
    source_transaction_id: Uuid,
    occurred_at: DateTime<Utc>,
    mutations: Vec<(ReportRevisionRow, ProjectionChangeKind)>,
) -> Result<()> {
    let mut members = Vec::with_capacity(mutations.len());
    for (row, change_kind) in mutations {
        let previous_projection_count: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*)
                 FROM investigation_projection_outbox member
                 JOIN investigation_projection_outbox_batches batch
                   ON batch.batch_id=member.batch_id
                WHERE member.operation_id=$1 AND member.entity_kind='report'
                  AND member.source_entity_id=$2
                  AND batch.stable_request_id<>$3"#,
        )
        .bind(operation_id)
        .bind(row.revision_id.to_string())
        .bind(stable_request_id)
        .fetch_one(&mut **tx)
        .await?;
        let entity_version = previous_projection_count
            .checked_add(1)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| anyhow::anyhow!("report_projection_version_invalid"))?;
        let body = CanonicalJsonObject::try_from_value(json!({
            "source_contract":"report_lifecycle_projection.v1",
            "report_id":row.report_id,
            "revision_id":row.revision_id,
            "revision_number":row.revision_number,
            "validation_status":row.validation_status,
            "publication_status":row.publication_status,
            "source_set_hash":format!("sha256:{}",row.source_set_hash),
            "supersedes_revision_id":row.supersedes_revision_id,
        }))
        .map_err(|error| crate::DbError::Other(anyhow::Error::new(error)))?;
        members.push(ProjectionOutboxSourceRow {
            outbox_member_id: Uuid::new_v5(
                &stable_request_id,
                format!(
                    "report-projection:{}:{}:{}",
                    row.revision_id,
                    row.row_version,
                    change_kind.as_str()
                )
                .as_bytes(),
            ),
            change_kind,
            source: ProjectionSourceSnapshotV1::Report(
                ReportProjectionRecordV1::try_new(
                    row.revision_id.to_string(),
                    entity_version,
                    1,
                    body,
                )
                .map_err(|error| crate::DbError::Other(anyhow::Error::new(error)))?,
            ),
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            invalidation_reason: None,
            storage: ProjectionSourceStorageV1::Inline,
        });
    }
    append_projection_source_batch_on(
        tx,
        AppendProjectionSourceBatchRow {
            batch_id: Uuid::new_v5(&stable_request_id, b"report-projection-batch.v1"),
            operation_id,
            project_scope_id: Some(project_scope_id),
            stable_request_id,
            source_transaction_id,
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            members,
        },
    )
    .await?;
    Ok(())
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
    let (report_current, operation_id, project_scope_id): (Option<Uuid>, Uuid, Uuid) =
        sqlx::query_as(
        "SELECT current_revision_id,operation_id,project_scope_id FROM reports WHERE report_id=$1 FOR UPDATE",
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
    append_report_projection_batch_on(
        tx,
        operation_id,
        project_scope_id,
        input.revision_id,
        input.revision_id,
        row.created_at,
        vec![(row.clone(), ProjectionChangeKind::Insert)],
    )
    .await?;
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
            claim
                .value
                .validate_authority(claim.authority_class)
                .map_err(|code| anyhow::anyhow!(code))?;
            super::report_claims::insert(
                tx,
                &super::report_claims::ReportClaimRow {
                    claim_id: claim.claim_id,
                    revision_id: claim.revision_id,
                    section_id: claim.section_id,
                    organization_id_at_time: claim.organization_id_at_time,
                    claim_kind: claim_kind(claim.claim_kind).to_string(),
                    authority_class: claim.authority_class.as_str().to_owned(),
                    subject_ref: claim.subject_ref.clone(),
                    predicate: claim.predicate.clone(),
                    object_value: serde_json::to_value(&claim.value)?,
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
    let invalidated_source_exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM report_authority_invalidation_events invalidation
                WHERE invalidation.report_revision_id=$1
                  AND invalidation.operation_id=$2
           )"#,
    )
    .bind(input.revision_id)
    .bind(input.operation_id)
    .fetch_one(&mut **tx)
    .await?;
    if invalidated_source_exists {
        return Err(anyhow::anyhow!("report_source_authority_invalidated").into());
    }
    let sealed_input = sqlx::query_as::<_, SealedReportInputRow>(
        r#"SELECT seal.typed_seal,open.authority_contract,
                  seal.tool_truth_authority_set_id,
                  seal.revision_adjudication_authority_set_id,
                  seal.legacy_report_authority_seal_id,
                  seal.source_member_count,seal.source_set_hash,seal.report_input_hash,
                  seal.effective_valid_until,transaction_timestamp() AS observed_at
             FROM report_input_seals seal
             JOIN report_input_open_headers open ON open.open_id=seal.open_id
            WHERE seal.revision_id=$1 FOR SHARE OF seal,open"#,
    )
    .bind(input.revision_id)
    .fetch_optional(&mut **tx)
    .await?;
    let SealedReportInputRow {
        typed_seal,
        authority_contract,
        tool_truth_authority_set_id,
        revision_adjudication_authority_set_id,
        legacy_report_authority_seal_id,
        source_member_count: sealed_member_count,
        source_set_hash: sealed_source_set_hash,
        report_input_hash: sealed_report_input_hash,
        effective_valid_until,
        observed_at,
    } = sealed_input.ok_or_else(|| anyhow::anyhow!("report_input_seal_required"))?;
    let typed_seal: ReportInputSealV1 = serde_json::from_value(typed_seal)
        .map_err(|_| anyhow::anyhow!("report_input_seal_corrupt"))?;
    let authority_identity_matches = match &typed_seal {
        ReportInputSealV1::RevisionAdjudication(value) => {
            authority_contract == "revision_adjudication"
                && tool_truth_authority_set_id
                    == value.report_tool_truth_authority_set.authority_set_id
                && revision_adjudication_authority_set_id
                    == Some(value.revision_adjudication_authority_set.authority_set_id)
                && legacy_report_authority_seal_id.is_none()
        }
        ReportInputSealV1::Legacy(value) => {
            authority_contract == "legacy"
                && tool_truth_authority_set_id
                    == value.report_tool_truth_authority_set.authority_set_id
                && revision_adjudication_authority_set_id.is_none()
                && legacy_report_authority_seal_id == Some(value.legacy_report_authority_seal_id)
        }
    };
    let typed_effective_valid_until = match &typed_seal {
        ReportInputSealV1::RevisionAdjudication(value) => value
            .report_tool_truth_authority_set
            .earliest_effective_valid_until
            .min(
                value
                    .revision_adjudication_authority_set
                    .earliest_effective_valid_until,
            ),
        ReportInputSealV1::Legacy(value) => {
            value
                .report_tool_truth_authority_set
                .earliest_effective_valid_until
        }
    };
    let current_source_count = input.current_source_snapshot.ordered_sources.len();
    if !authority_identity_matches
        || i64::try_from(current_source_count).ok() != Some(sealed_member_count)
        || sealed_source_set_hash.as_slice() != input.current_source_snapshot.source_set_hash
        || sealed_report_input_hash.as_slice() != typed_seal.report_input_hash()
        || effective_valid_until != typed_effective_valid_until
        || effective_valid_until <= observed_at
    {
        return Err(anyhow::anyhow!("report_input_seal_stale").into());
    }
    typed_seal
        .validate(
            current_source_count,
            input.current_source_snapshot.source_set_hash,
            observed_at,
        )
        .map_err(|code| anyhow::anyhow!(code))?;
    super::report_input_authority::validate_current_report_input_authority_on(
        tx,
        input.operation_id,
        input.revision_id,
        &typed_seal,
        observed_at,
    )
    .await?;

    let open_work_exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM verification_campaigns
                WHERE operation_id=$1 AND state NOT IN ('terminal','superseded')
               UNION ALL
               SELECT 1 FROM verification_prepared_actions
                WHERE operation_id=$1
                  AND state IN ('pending_authorization','authorized','started','outcome_unknown')
               UNION ALL
               SELECT 1 FROM verification_cleanup_obligations obligation
                 JOIN verification_action_executions execution
                   ON execution.action_execution_id=obligation.action_execution_id
                WHERE execution.operation_id=$1 AND obligation.status IN ('pending','outcome_unknown')
               UNION ALL
               SELECT 1 FROM verification_callback_obligations obligation
                 JOIN verification_action_executions execution
                   ON execution.action_execution_id=obligation.action_execution_id
                WHERE execution.operation_id=$1 AND obligation.status='pending'
               UNION ALL
               SELECT 1 FROM hypothesis_consolidation_batches batch
                WHERE batch.operation_id=$1 AND NOT EXISTS(
                    SELECT 1 FROM hypothesis_consolidation_receipts receipt
                     WHERE receipt.consolidation_batch_id=batch.consolidation_batch_id
                )
               UNION ALL
               SELECT 1 FROM enrichment_obligations
                WHERE operation_id=$1 AND status='open'
               UNION ALL
               SELECT 1 FROM application_fact_refinement_obligations
                WHERE operation_id=$1 AND status='open'
               UNION ALL
               SELECT 1 FROM hypothesis_re_adjudication_obligations
                WHERE operation_id=$1 AND status='open'
           )"#,
    )
    .bind(input.operation_id)
    .fetch_one(&mut **tx)
    .await?;
    if open_work_exists {
        return Err(anyhow::anyhow!("report_finalization_open_work").into());
    }
    if input.artifacts.is_empty() {
        return Err(anyhow::anyhow!("report_finalize_artifact_required").into());
    }

    let superseded = sqlx::query_as::<_, ReportRevisionRow>(
        r#"UPDATE report_revisions
              SET publication_status='superseded'
            WHERE report_id=$1 AND publication_status='final' AND revision_id<>$2
            RETURNING *"#,
    )
    .bind(input.report_id)
    .bind(input.revision_id)
    .fetch_all(&mut **tx)
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
        super::historical_report_artifacts::create_historical_artifact_receipt_on(
            tx,
            &super::historical_report_artifacts::CreateHistoricalArtifactReceiptV0 {
                report_id: input.report_id,
                revision_id: input.revision_id,
                operation_id: input.operation_id,
                project_scope_id: input.project_scope_id,
                artifact_kind: artifact.artifact_kind.clone(),
                content_key: artifact.content_key.clone(),
                sha256: artifact.sha256.clone(),
                storage_path: artifact.storage_path.clone(),
                byte_len: artifact.byte_len,
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
    let projection_request_id = Uuid::new_v5(
        &input.revision_id,
        format!("report-finalization-projection:{}", finalized.row_version).as_bytes(),
    );
    let mut projection_mutations = superseded
        .into_iter()
        .map(|revision| (revision, ProjectionChangeKind::Supersede))
        .collect::<Vec<_>>();
    projection_mutations.push((finalized.clone(), ProjectionChangeKind::Close));
    append_report_projection_batch_on(
        tx,
        input.operation_id,
        input.project_scope_id,
        projection_request_id,
        input.revision_id,
        occurred_at,
        projection_mutations,
    )
    .await?;
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
