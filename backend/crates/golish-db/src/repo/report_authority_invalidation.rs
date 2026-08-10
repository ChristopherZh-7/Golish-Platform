//! Atomic invalidation of reports that consumed revoked semantic authority.
//!
//! Plan A and Plan C own their source transitions.  Their canonical writers
//! call this module before commit so the typed reverse index, immutable
//! invalidation events, and the single investigation whole-batch outbox share
//! the same transaction boundary.

use chrono::{DateTime, Utc};
use golish_core::hypothesis_semantic_key::CanonicalJsonObject;
use golish_core::investigation_projection::{
    ProjectionChangeKind, ProjectionInvalidationReason, ProjectionSourceSnapshotV1,
    ProjectionSourceTimeStatusV1, ReportProjectionRecordV1,
};
use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::hypothesis_legacy_projection::{
    append_projection_source_batch_on, AppendProjectionSourceBatchRow, ProjectionOutboxSourceRow,
    ProjectionSourceStorageV1,
};
use crate::{DbError, Result};

const SOURCE_INVALID: &str = "REPORT_AUTHORITY_INVALIDATION_SOURCE_INVALID";
const SOURCE_STALE: &str = "REPORT_AUTHORITY_INVALIDATION_SOURCE_STALE";
const REPLAY_DRIFT: &str = "REPORT_AUTHORITY_INVALIDATION_REPLAY_DRIFT";

fn conflict(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

fn valid_tagged_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReportInvalidationSourceV1 {
    ToolTruthSemanticOrphan {
        receipt_id: Uuid,
        reconciliation_id: Uuid,
        reconciliation_hash: String,
    },
    VerificationAuthorityQuarantine {
        quarantine_event_id: Uuid,
        quarantine_hash: String,
    },
}

impl ReportInvalidationSourceV1 {
    fn origin_kind(&self) -> &'static str {
        match self {
            Self::ToolTruthSemanticOrphan { .. } => "tool_truth_semantic_orphan",
            Self::VerificationAuthorityQuarantine { .. } => "verification_authority_quarantine",
        }
    }

    fn origin_id(&self) -> Uuid {
        match self {
            Self::ToolTruthSemanticOrphan {
                reconciliation_id, ..
            } => *reconciliation_id,
            Self::VerificationAuthorityQuarantine {
                quarantine_event_id,
                ..
            } => *quarantine_event_id,
        }
    }

    fn reason_code(&self) -> &'static str {
        match self {
            Self::ToolTruthSemanticOrphan { .. } => "semantic_authority_orphaned",
            Self::VerificationAuthorityQuarantine { .. } => "verification_authority_quarantined",
        }
    }

    fn authority_hash(&self) -> &str {
        match self {
            Self::ToolTruthSemanticOrphan {
                reconciliation_hash,
                ..
            } => reconciliation_hash,
            Self::VerificationAuthorityQuarantine {
                quarantine_hash, ..
            } => quarantine_hash,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportAuthorityInvalidationReceiptV1 {
    pub source_batch_id: Option<Uuid>,
    pub invalidation_event_ids: Vec<Uuid>,
    pub replayed: bool,
}

#[derive(Debug)]
struct SourceAuthority {
    operation_id: Uuid,
    project_scope_id: Uuid,
    organization_id: Uuid,
    hypothesis_revision_id: Option<Uuid>,
}

#[derive(Debug, sqlx::FromRow)]
struct AffectedReport {
    report_revision_id: Uuid,
    report_id: Uuid,
    revision_number: i32,
    source_set_hash: String,
    validation_status: String,
    publication_status: String,
    supersedes_revision_id: Option<Uuid>,
    report_input_seal_id: Uuid,
    dependency_kind: String,
    tool_truth_authority_set_id: Option<Uuid>,
    tool_truth_authority_member_ordinal: Option<i32>,
    revision_adjudication_authority_set_id: Option<Uuid>,
    revision_adjudication_member_ordinal: Option<i32>,
    previous_projection_count: i64,
}

async fn lock_source_authority_on(
    tx: &mut Transaction<'_, Postgres>,
    source: &ReportInvalidationSourceV1,
) -> Result<SourceAuthority> {
    match source {
        ReportInvalidationSourceV1::ToolTruthSemanticOrphan {
            receipt_id,
            reconciliation_id,
            reconciliation_hash,
        } => {
            let authority = sqlx::query_as::<_, (Uuid, Uuid, Uuid)>(
                r#"SELECT execution.operation_id,execution.project_scope_id,
                          execution.organization_id
                     FROM capability_execution_reconciliations reconciliation
                     JOIN capability_execution_receipts receipt
                       ON receipt.id=reconciliation.receipt_id
                      AND receipt.execution_authority_id=reconciliation.execution_authority_id
                     JOIN tool_truth_execution_authorities execution
                       ON execution.id=receipt.execution_authority_id
                    WHERE reconciliation.id=$1 AND reconciliation.receipt_id=$2
                      AND reconciliation.semantic_reconciliation_hash=$3
                      AND reconciliation.reconciliation_state='orphaned'
                      AND reconciliation.sealed_at IS NOT NULL
                      AND receipt.current_semantic_reconciliation_id=reconciliation.id
                      AND receipt.current_semantic_authority_version=
                          reconciliation.semantic_authority_version
                      AND receipt.current_semantic_reconciliation_hash=
                          reconciliation.semantic_reconciliation_hash
                      AND receipt.reconciliation_state='orphaned'
                    FOR SHARE OF reconciliation,receipt,execution"#,
            )
            .bind(reconciliation_id)
            .bind(receipt_id)
            .bind(reconciliation_hash)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| conflict(SOURCE_STALE))?;
            Ok(SourceAuthority {
                operation_id: authority.0,
                project_scope_id: authority.1,
                organization_id: authority.2,
                hypothesis_revision_id: None,
            })
        }
        ReportInvalidationSourceV1::VerificationAuthorityQuarantine {
            quarantine_event_id,
            quarantine_hash,
        } => {
            let authority = sqlx::query_as::<_, (Uuid, Uuid, Uuid, Uuid)>(
                r#"SELECT quarantine.operation_id,quarantine.project_scope_id,
                          quarantine.organization_id,outcome.hypothesis_revision_id
                     FROM verification_authority_quarantine_events quarantine
                     JOIN hypothesis_objective_outcome_receipts outcome
                       ON outcome.objective_outcome_receipt_id=
                          quarantine.objective_outcome_receipt_id
                      AND outcome.operation_id=quarantine.operation_id
                      AND outcome.project_scope_id=quarantine.project_scope_id
                      AND outcome.organization_id=quarantine.organization_id
                    WHERE quarantine.quarantine_event_id=$1
                      AND quarantine.quarantine_hash=$2
                    FOR SHARE OF quarantine,outcome"#,
            )
            .bind(quarantine_event_id)
            .bind(quarantine_hash)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| conflict(SOURCE_STALE))?;
            Ok(SourceAuthority {
                operation_id: authority.0,
                project_scope_id: authority.1,
                organization_id: authority.2,
                hypothesis_revision_id: Some(authority.3),
            })
        }
    }
}

async fn replay_receipt_on(
    tx: &mut Transaction<'_, Postgres>,
    source: &ReportInvalidationSourceV1,
    stable_request_id: Uuid,
) -> Result<Option<ReportAuthorityInvalidationReceiptV1>> {
    let rows = sqlx::query_as::<_, (Uuid, Uuid, Uuid)>(
        r#"SELECT event_id,source_batch_id,stable_request_id
             FROM report_authority_invalidation_events
            WHERE origin_kind=$1 AND origin_id=$2
            ORDER BY report_revision_id,event_id FOR SHARE"#,
    )
    .bind(source.origin_kind())
    .bind(source.origin_id())
    .fetch_all(&mut **tx)
    .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let source_batch_id = rows[0].1;
    if rows
        .iter()
        .any(|row| row.1 != source_batch_id || row.2 != stable_request_id)
    {
        return Err(conflict(REPLAY_DRIFT));
    }
    Ok(Some(ReportAuthorityInvalidationReceiptV1 {
        source_batch_id: Some(source_batch_id),
        invalidation_event_ids: rows.into_iter().map(|row| row.0).collect(),
        replayed: true,
    }))
}

async fn lock_affected_reports_on(
    tx: &mut Transaction<'_, Postgres>,
    source: &ReportInvalidationSourceV1,
    authority: &SourceAuthority,
) -> Result<Vec<AffectedReport>> {
    match source {
        ReportInvalidationSourceV1::ToolTruthSemanticOrphan { receipt_id, .. } => {
            let rows = sqlx::query_as::<_, AffectedReport>(
                r#"SELECT revision.revision_id AS report_revision_id,revision.report_id,
                          revision.revision_number,
                          revision.source_set_hash,revision.validation_status,
                          revision.publication_status,revision.supersedes_revision_id,
                          seal.seal_id AS report_input_seal_id,
                          'tool_truth_authority_member'::TEXT AS dependency_kind,
                          member.authority_set_id AS tool_truth_authority_set_id,
                          member.ordinal AS tool_truth_authority_member_ordinal,
                          NULL::UUID AS revision_adjudication_authority_set_id,
                          NULL::INTEGER AS revision_adjudication_member_ordinal,
                          (SELECT COUNT(*) FROM investigation_projection_outbox previous
                            WHERE previous.operation_id=report.operation_id
                              AND previous.entity_kind='report'
                              AND previous.source_entity_id=revision.revision_id::TEXT)
                              AS previous_projection_count
                     FROM reports report
                     JOIN report_revisions revision ON revision.report_id=report.report_id
                     JOIN report_input_seals seal ON seal.revision_id=revision.revision_id
                     JOIN report_input_tool_truth_authority_members member
                       ON member.authority_set_id=seal.tool_truth_authority_set_id
                      AND member.organization_id=$3
                    WHERE report.operation_id=$1 AND report.project_scope_id=$2
                      AND EXISTS(
                          SELECT 1
                            FROM tool_truth_authority_bundle_members bundle_member
                            JOIN tool_truth_authority_set_members authority_member
                              ON authority_member.authority_set_id=
                                 bundle_member.authority_set_seal_id
                           WHERE bundle_member.bundle_seal_id=
                                 member.tool_truth_authority_bundle_id
                             AND authority_member.receipt_id=$4
                      )
                      AND NOT EXISTS(
                          SELECT 1 FROM report_authority_invalidation_events existing
                           WHERE existing.report_revision_id=revision.revision_id
                             AND existing.origin_kind=$5 AND existing.origin_id=$6
                      )
                    ORDER BY revision.revision_id
                    FOR UPDATE OF revision"#,
            )
            .bind(authority.operation_id)
            .bind(authority.project_scope_id)
            .bind(authority.organization_id)
            .bind(receipt_id)
            .bind(source.origin_kind())
            .bind(source.origin_id())
            .fetch_all(&mut **tx)
            .await?;
            Ok(rows)
        }
        ReportInvalidationSourceV1::VerificationAuthorityQuarantine { .. } => {
            let hypothesis_revision_id = authority
                .hypothesis_revision_id
                .ok_or_else(|| conflict(SOURCE_STALE))?;
            let rows = sqlx::query_as::<_, AffectedReport>(
                r#"SELECT revision.revision_id AS report_revision_id,revision.report_id,
                          revision.revision_number,
                          revision.source_set_hash,revision.validation_status,
                          revision.publication_status,revision.supersedes_revision_id,
                          seal.seal_id AS report_input_seal_id,
                          'revision_adjudication_member'::TEXT AS dependency_kind,
                          NULL::UUID AS tool_truth_authority_set_id,
                          NULL::INTEGER AS tool_truth_authority_member_ordinal,
                          member.authority_set_id AS revision_adjudication_authority_set_id,
                          member.ordinal AS revision_adjudication_member_ordinal,
                          (SELECT COUNT(*) FROM investigation_projection_outbox previous
                            WHERE previous.operation_id=report.operation_id
                              AND previous.entity_kind='report'
                              AND previous.source_entity_id=revision.revision_id::TEXT)
                              AS previous_projection_count
                     FROM reports report
                     JOIN report_revisions revision ON revision.report_id=report.report_id
                     JOIN report_input_seals seal ON seal.revision_id=revision.revision_id
                     JOIN report_input_revision_adjudication_members member
                       ON member.authority_set_id=
                          seal.revision_adjudication_authority_set_id
                      AND member.organization_id=$3
                      AND member.hypothesis_revision_id=$4
                    WHERE report.operation_id=$1 AND report.project_scope_id=$2
                      AND NOT EXISTS(
                          SELECT 1 FROM report_authority_invalidation_events existing
                           WHERE existing.report_revision_id=revision.revision_id
                             AND existing.origin_kind=$5 AND existing.origin_id=$6
                      )
                    ORDER BY revision.revision_id
                    FOR UPDATE OF revision"#,
            )
            .bind(authority.operation_id)
            .bind(authority.project_scope_id)
            .bind(authority.organization_id)
            .bind(hypothesis_revision_id)
            .bind(source.origin_kind())
            .bind(source.origin_id())
            .fetch_all(&mut **tx)
            .await?;
            Ok(rows)
        }
    }
}

fn event_hash(
    event_id: Uuid,
    report: &AffectedReport,
    source: &ReportInvalidationSourceV1,
    authority: &SourceAuthority,
    source_batch_id: Uuid,
) -> String {
    format!(
        "sha256:{}",
        super::operation_scope_decisions::sha256_json(&json!({
            "schema":"report_authority_invalidation.v1",
            "event_id":event_id,
            "report_revision_id":report.report_revision_id,
            "report_input_seal_id":report.report_input_seal_id,
            "operation_id":authority.operation_id,
            "project_scope_id":authority.project_scope_id,
            "organization_id":authority.organization_id,
            "dependency_kind":report.dependency_kind,
            "tool_truth_authority_set_id":report.tool_truth_authority_set_id,
            "tool_truth_authority_member_ordinal":report.tool_truth_authority_member_ordinal,
            "revision_adjudication_authority_set_id":
                report.revision_adjudication_authority_set_id,
            "revision_adjudication_member_ordinal":
                report.revision_adjudication_member_ordinal,
            "origin_kind":source.origin_kind(),
            "origin_id":source.origin_id(),
            "origin_authority_hash":source.authority_hash(),
            "reason_code":source.reason_code(),
            "source_batch_id":source_batch_id,
        }))
    )
}

/// Invalidate every report revision that consumed the exact source authority.
///
/// There is deliberately no pool-owned overload: the A/C source transition
/// and this complete invalidation batch must commit or roll back together.
pub async fn invalidate_reports_for_source_authority_on(
    tx: &mut Transaction<'_, Postgres>,
    source: ReportInvalidationSourceV1,
    stable_request_id: Uuid,
) -> Result<ReportAuthorityInvalidationReceiptV1> {
    if stable_request_id.is_nil()
        || source.origin_id().is_nil()
        || !valid_tagged_sha256(source.authority_hash())
    {
        return Err(conflict(SOURCE_INVALID));
    }
    let authority = lock_source_authority_on(tx, &source).await?;
    if let Some(replay) = replay_receipt_on(tx, &source, stable_request_id).await? {
        return Ok(replay);
    }
    let affected = lock_affected_reports_on(tx, &source, &authority).await?;
    if affected.is_empty() {
        return Ok(ReportAuthorityInvalidationReceiptV1 {
            source_batch_id: None,
            invalidation_event_ids: Vec::new(),
            replayed: false,
        });
    }

    let occurred_at: DateTime<Utc> = sqlx::query_scalar("SELECT transaction_timestamp()")
        .fetch_one(&mut **tx)
        .await?;
    let source_batch_id = Uuid::new_v5(&stable_request_id, b"report-invalidation-batch.v1");
    let mut event_ids = Vec::with_capacity(affected.len());
    let mut projection_members = Vec::with_capacity(affected.len());
    for report in &affected {
        let event_id = Uuid::new_v5(
            &stable_request_id,
            format!(
                "report-invalidation:{}:{}:{}",
                report.report_revision_id,
                report.dependency_kind,
                source.origin_id()
            )
            .as_bytes(),
        );
        let entity_version = report
            .previous_projection_count
            .checked_add(1)
            .and_then(|version| u64::try_from(version).ok())
            .ok_or_else(|| conflict(SOURCE_STALE))?;
        let body = CanonicalJsonObject::try_from_value(json!({
            "source_contract":"report_authority_invalidation.v1",
            "report_id":report.report_id,
            "revision_id":report.report_revision_id,
            "revision_number":report.revision_number,
            "validation_status":report.validation_status,
            "publication_status":report.publication_status,
            "source_set_hash":format!("sha256:{}",report.source_set_hash),
            "supersedes_revision_id":report.supersedes_revision_id,
            "authority_state":"revoked",
            "invalidation_event_id":event_id,
            "report_input_seal_id":report.report_input_seal_id,
            "origin_kind":source.origin_kind(),
            "origin_id":source.origin_id(),
            "reason_code":source.reason_code(),
        }))
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        projection_members.push(ProjectionOutboxSourceRow {
            outbox_member_id: Uuid::new_v5(&event_id, b"projection-member.v1"),
            change_kind: ProjectionChangeKind::Invalidate,
            source: ProjectionSourceSnapshotV1::Report(
                ReportProjectionRecordV1::try_new(
                    report.report_revision_id.to_string(),
                    entity_version,
                    1,
                    body,
                )
                .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
            ),
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            invalidation_reason: Some(ProjectionInvalidationReason::SourceQuarantined),
            storage: ProjectionSourceStorageV1::Inline,
        });
        event_ids.push(event_id);
    }
    append_projection_source_batch_on(
        tx,
        AppendProjectionSourceBatchRow {
            batch_id: source_batch_id,
            operation_id: authority.operation_id,
            project_scope_id: Some(authority.project_scope_id),
            stable_request_id,
            source_transaction_id: source.origin_id(),
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            members: projection_members,
        },
    )
    .await?;

    for (report, event_id) in affected.iter().zip(event_ids.iter().copied()) {
        let hash = event_hash(event_id, report, &source, &authority, source_batch_id);
        let (tool_truth_orphan_reconciliation_id, verification_quarantine_event_id) = match &source
        {
            ReportInvalidationSourceV1::ToolTruthSemanticOrphan {
                reconciliation_id, ..
            } => (Some(*reconciliation_id), None),
            ReportInvalidationSourceV1::VerificationAuthorityQuarantine {
                quarantine_event_id,
                ..
            } => (None, Some(*quarantine_event_id)),
        };
        sqlx::query(
            r#"INSERT INTO report_authority_invalidation_events(
                   event_id,stable_request_id,report_revision_id,report_input_seal_id,
                   operation_id,project_scope_id,organization_id,dependency_kind,
                   tool_truth_authority_set_id,tool_truth_authority_member_ordinal,
                   revision_adjudication_authority_set_id,
                   revision_adjudication_member_ordinal,origin_kind,origin_id,
                   tool_truth_orphan_reconciliation_id,verification_quarantine_event_id,
                   reason_code,source_batch_id,event_hash,invalidated_at
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)"#,
        )
        .bind(event_id)
        .bind(stable_request_id)
        .bind(report.report_revision_id)
        .bind(report.report_input_seal_id)
        .bind(authority.operation_id)
        .bind(authority.project_scope_id)
        .bind(authority.organization_id)
        .bind(&report.dependency_kind)
        .bind(report.tool_truth_authority_set_id)
        .bind(report.tool_truth_authority_member_ordinal)
        .bind(report.revision_adjudication_authority_set_id)
        .bind(report.revision_adjudication_member_ordinal)
        .bind(source.origin_kind())
        .bind(source.origin_id())
        .bind(tool_truth_orphan_reconciliation_id)
        .bind(verification_quarantine_event_id)
        .bind(source.reason_code())
        .bind(source_batch_id)
        .bind(hash)
        .bind(occurred_at)
        .execute(&mut **tx)
        .await?;
    }
    Ok(ReportAuthorityInvalidationReceiptV1 {
        source_batch_id: Some(source_batch_id),
        invalidation_event_ids: event_ids,
        replayed: false,
    })
}
