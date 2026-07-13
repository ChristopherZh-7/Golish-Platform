use chrono::{DateTime, Utc};
use golish_memory_domain::assertion::{
    AssertionIdentity, AssertionKind, AssertionObject, AssertionStatus, KnowledgeAssertion,
    KnowledgeAssertionDraft, VaultRef,
};
use golish_memory_domain::classification::{AssertionVisibility, KnowledgeClassification};
use golish_memory_domain::event_catalog::{KnowledgeEventEnvelopeV1, KnowledgeEventNameV1};
use golish_memory_domain::scope::ProjectScopeId;
use golish_memory_domain::source_ref::{
    CanonicalSourceKind, SourceRef, SourceRefError, StoredCanonicalRowId,
};
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::{knowledge_documents, knowledge_outbox};

#[derive(Clone, Debug, sqlx::FromRow)]
struct StoredAssertionRow {
    assertion_id: Uuid,
    visibility: String,
    project_scope_id: Option<Uuid>,
    organization_id_at_time: Option<Uuid>,
    source_operation_id: Uuid,
    source_scope_snapshot_hash: String,
    source_kind: String,
    source_id_kind: String,
    source_id_value: String,
    source_stream_key: String,
    source_version: i64,
    subject_key: String,
    predicate: String,
    object_hash: String,
    assertion_identity_hash: String,
    object_kind: String,
    object_value: Option<Value>,
    vault_ref: Option<Uuid>,
    assertion_kind: String,
    status: String,
    evidence_refs: Vec<i64>,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    fresh_until: Option<DateTime<Utc>>,
    classification: String,
    content_hash: String,
}

impl StoredAssertionRow {
    fn into_domain(self) -> Result<KnowledgeAssertion, AssertionRepoError> {
        let visibility = match self.visibility.as_str() {
            "organization_long_term" => AssertionVisibility::OrganizationLongTerm {
                project_scope_id: ProjectScopeId(
                    self.project_scope_id
                        .ok_or(AssertionRepoError::Corrupt("missing_project_scope"))?,
                ),
                organization_id_at_time: self
                    .organization_id_at_time
                    .ok_or(AssertionRepoError::Corrupt("missing_organization"))?,
            },
            "global_sanitized" => {
                if self.project_scope_id.is_some() || self.organization_id_at_time.is_some() {
                    return Err(AssertionRepoError::Corrupt("global_scope_present"));
                }
                AssertionVisibility::GlobalSanitized
            }
            _ => return Err(AssertionRepoError::Corrupt("visibility")),
        };
        let source = SourceRef {
            source_kind: parse_source_kind(&self.source_kind)?,
            row_id: StoredCanonicalRowId {
                kind: self.source_id_kind,
                value: self.source_id_value,
            }
            .into_domain()?,
            source_stream_key: self.source_stream_key,
            version: self.source_version,
        };
        let object = match self.object_kind.as_str() {
            "json" => AssertionObject::Json(
                self.object_value
                    .ok_or(AssertionRepoError::Corrupt("json_object_missing"))?,
            ),
            "vault_ref" => AssertionObject::VaultRef(VaultRef(
                self.vault_ref
                    .ok_or(AssertionRepoError::Corrupt("vault_ref_missing"))?,
            )),
            _ => return Err(AssertionRepoError::Corrupt("object_kind")),
        };
        let identity = AssertionIdentity {
            subject_key: self.subject_key,
            predicate: self.predicate,
            object_hash: self.object_hash,
            identity_hash: self.assertion_identity_hash,
        };
        let draft = KnowledgeAssertionDraft {
            assertion_id: self.assertion_id,
            visibility,
            source_operation_id: self.source_operation_id,
            source_scope_snapshot_hash: self.source_scope_snapshot_hash,
            source,
            identity,
            kind: parse_assertion_kind(&self.assertion_kind)?,
            status: parse_assertion_status(&self.status)?,
            object,
            classification: parse_classification(&self.classification)?,
            evidence_ids: self.evidence_refs,
            valid_from: self.valid_from,
            valid_to: self.valid_to,
            fresh_until: self.fresh_until,
        };
        let assertion = draft
            .validate()
            .map_err(|_| AssertionRepoError::Corrupt("domain_validation"))?;
        if assertion.content_hash != self.content_hash {
            return Err(AssertionRepoError::Corrupt("content_hash"));
        }
        Ok(assertion)
    }
}

pub async fn insert_with_connection(
    connection: &mut PgConnection,
    assertion: &KnowledgeAssertion,
) -> Result<KnowledgeAssertion, AssertionRepoError> {
    let validated = KnowledgeAssertionDraft {
        assertion_id: assertion.assertion_id,
        visibility: assertion.visibility.clone(),
        source_operation_id: assertion.source_operation_id,
        source_scope_snapshot_hash: assertion.source_scope_snapshot_hash.clone(),
        source: assertion.source.clone(),
        identity: assertion.identity.clone(),
        kind: assertion.kind,
        status: assertion.status,
        object: assertion.object.clone(),
        classification: assertion.classification,
        evidence_ids: assertion.evidence_ids.clone(),
        valid_from: assertion.valid_from,
        valid_to: assertion.valid_to,
        fresh_until: assertion.fresh_until,
    }
    .validate()
    .map_err(|error| AssertionRepoError::Invalid(error.code()))?;
    if validated.content_hash != assertion.content_hash {
        return Err(AssertionRepoError::Invalid(
            "memory_assertion_content_hash_mismatch",
        ));
    }
    let source_id = StoredCanonicalRowId::from_domain(&assertion.source.row_id)?;
    let (object_kind, object_value, vault_ref) = match &assertion.object {
        AssertionObject::Json(value) => ("json", Some(value.clone()), None),
        AssertionObject::VaultRef(value) => ("vault_ref", None, Some(value.0)),
    };
    let inserted = sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO knowledge_assertions (
               assertion_id, visibility, project_scope_id, organization_id_at_time,
               source_operation_id, source_scope_snapshot_hash, source_kind,
               source_id_kind, source_id_value, source_stream_key, source_version,
               subject_key, predicate, object_hash, assertion_identity_hash,
               object_kind, object_value, vault_ref, assertion_kind, status,
               evidence_refs, valid_from, valid_to, fresh_until, classification,
               content_hash
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,
               $19,$20,$21,$22,$23,$24,$25,$26
           )
           ON CONFLICT DO NOTHING
           RETURNING assertion_id"#,
    )
    .bind(assertion.assertion_id)
    .bind(assertion.visibility.as_str())
    .bind(assertion.visibility.project_scope_id().map(|id| id.0))
    .bind(assertion.visibility.organization_id_at_time())
    .bind(assertion.source_operation_id)
    .bind(&assertion.source_scope_snapshot_hash)
    .bind(assertion.source.source_kind.as_str())
    .bind(&source_id.kind)
    .bind(&source_id.value)
    .bind(&assertion.source.source_stream_key)
    .bind(assertion.source.version)
    .bind(&assertion.identity.subject_key)
    .bind(&assertion.identity.predicate)
    .bind(&assertion.identity.object_hash)
    .bind(&assertion.identity.identity_hash)
    .bind(object_kind)
    .bind(object_value)
    .bind(vault_ref)
    .bind(assertion.kind.as_str())
    .bind(assertion.status.as_str())
    .bind(&assertion.evidence_ids)
    .bind(assertion.valid_from)
    .bind(assertion.valid_to)
    .bind(assertion.fresh_until)
    .bind(assertion.classification.as_str())
    .bind(&assertion.content_hash)
    .fetch_optional(&mut *connection)
    .await?;

    let assertion_id = match inserted {
        Some(assertion_id) => assertion_id,
        None => {
            let existing_id = sqlx::query_scalar::<_, Uuid>(
                r#"SELECT assertion_id FROM knowledge_assertions
               WHERE project_scope_id IS NOT DISTINCT FROM $1
                 AND source_stream_key = $2
                 AND source_version = $3
                 AND subject_key = $4
                 AND predicate = $5
                 AND object_hash = $6"#,
            )
            .bind(assertion.visibility.project_scope_id().map(|id| id.0))
            .bind(&assertion.source.source_stream_key)
            .bind(assertion.source.version)
            .bind(&assertion.identity.subject_key)
            .bind(&assertion.identity.predicate)
            .bind(&assertion.identity.object_hash)
            .fetch_one(&mut *connection)
            .await?;
            let existing = get_with_connection(connection, existing_id).await?;
            if !exact_storage_replay(&existing, assertion) {
                return Err(AssertionRepoError::ReplayConflict);
            }
            existing_id
        }
    };
    get_with_connection(connection, assertion_id).await
}

/// Caller-owned connection seam for a future canonical terminal compound
/// transaction. The immutable Assertion and its catalog deliveries either
/// both commit with the caller or both roll back.
pub async fn promote_assertion_with_event_with_connection(
    connection: &mut PgConnection,
    assertion: &KnowledgeAssertion,
    event: &KnowledgeEventEnvelopeV1,
) -> Result<Uuid, AssertionRepoError> {
    validate_promotion_event(assertion, event)?;
    insert_with_connection(connection, assertion).await?;
    knowledge_outbox::append_event_with_catalog_deliveries_with_connection(connection, event)
        .await
        .map_err(AssertionRepoError::Outbox)
}

/// Caller-owned connection seam for scope/source invalidation. Assertion,
/// Document and Embedding validity close before the immutable invalidation
/// event is appended, all under the same transaction owned by the caller.
pub async fn invalidate_projection_chain_with_event_with_connection(
    connection: &mut PgConnection,
    source: &SourceRef,
    invalidated_at: DateTime<Utc>,
    event: &KnowledgeEventEnvelopeV1,
) -> Result<Uuid, AssertionRepoError> {
    event
        .validate()
        .map_err(|_| AssertionRepoError::EventMismatch)?;
    if event.event_name != KnowledgeEventNameV1::SourceScopeInvalidated
        || event.payload.source != *source
    {
        return Err(AssertionRepoError::EventMismatch);
    }
    let stored_source_id = StoredCanonicalRowId::from_domain(&source.row_id)?;
    let updated = sqlx::query(
        r#"UPDATE knowledge_assertions
           SET status = CASE WHEN status = 'active' THEN 'expired' ELSE status END,
               valid_to = LEAST(COALESCE(valid_to, $9), $9),
               updated_at = NOW()
           WHERE project_scope_id IS NOT DISTINCT FROM $1
             AND organization_id_at_time IS NOT DISTINCT FROM $2
             AND source_operation_id = $3
             AND source_kind = $4
             AND source_id_kind = $5
             AND source_id_value = $6
             AND source_stream_key = $7
             AND source_version = $8"#,
    )
    .bind(event.project_scope_id.map(|id| id.0))
    .bind(event.organization_id_at_time)
    .bind(event.source_operation_id)
    .bind(source.source_kind.as_str())
    .bind(&stored_source_id.kind)
    .bind(&stored_source_id.value)
    .bind(&source.source_stream_key)
    .bind(source.version)
    .bind(invalidated_at)
    .execute(&mut *connection)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(AssertionRepoError::InvalidationSourceMissing);
    }
    knowledge_documents::invalidate_source_with_connection(
        connection,
        event.project_scope_id.map(|id| id.0),
        &source.source_stream_key,
        source.version,
        invalidated_at,
    )
    .await?;
    knowledge_outbox::append_event_with_catalog_deliveries_with_connection(connection, event)
        .await
        .map_err(AssertionRepoError::Outbox)
}

fn validate_promotion_event(
    assertion: &KnowledgeAssertion,
    event: &KnowledgeEventEnvelopeV1,
) -> Result<(), AssertionRepoError> {
    event
        .validate()
        .map_err(|_| AssertionRepoError::EventMismatch)?;
    let kind_matches = matches!(
        (event.event_name, assertion.source.source_kind),
        (
            KnowledgeEventNameV1::CandidateAttemptTerminal,
            CanonicalSourceKind::CandidateAttempt
        ) | (
            KnowledgeEventNameV1::FactDeltaAccepted,
            CanonicalSourceKind::FactDelta
        ) | (
            KnowledgeEventNameV1::PostExploitActionPrepared,
            CanonicalSourceKind::PostExploitAction
        ) | (
            KnowledgeEventNameV1::PostExploitFactTerminal,
            CanonicalSourceKind::Foothold | CanonicalSourceKind::ObjectiveOutcome
        ) | (
            KnowledgeEventNameV1::CleanupObligationTerminal,
            CanonicalSourceKind::CleanupObligation | CanonicalSourceKind::ResidualRisk
        )
    );
    let scope_matches = match &assertion.visibility {
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id,
            organization_id_at_time,
        } => {
            event.project_scope_id == Some(*project_scope_id)
                && event.organization_id_at_time == Some(*organization_id_at_time)
        }
        AssertionVisibility::GlobalSanitized => {
            event.project_scope_id.is_none() && event.organization_id_at_time.is_none()
        }
    };
    if !kind_matches
        || !scope_matches
        || assertion.status != AssertionStatus::Active
        || assertion.source_operation_id != event.source_operation_id
        || assertion.source != event.payload.source
    {
        return Err(AssertionRepoError::EventMismatch);
    }
    Ok(())
}

fn exact_storage_replay(left: &KnowledgeAssertion, right: &KnowledgeAssertion) -> bool {
    left.assertion_id == right.assertion_id
        && left.visibility == right.visibility
        && left.source_operation_id == right.source_operation_id
        && left.source_scope_snapshot_hash == right.source_scope_snapshot_hash
        && left.source == right.source
        && left.identity == right.identity
        && left.kind == right.kind
        && left.status == right.status
        && left.object == right.object
        && left.classification == right.classification
        && left.evidence_ids == right.evidence_ids
        && left.valid_from.timestamp_micros() == right.valid_from.timestamp_micros()
        && left.valid_to.map(|value| value.timestamp_micros())
            == right.valid_to.map(|value| value.timestamp_micros())
        && left.fresh_until.map(|value| value.timestamp_micros())
            == right.fresh_until.map(|value| value.timestamp_micros())
        && left.content_hash == right.content_hash
}

pub async fn get(
    pool: &PgPool,
    assertion_id: Uuid,
) -> Result<KnowledgeAssertion, AssertionRepoError> {
    let row = fetch_row(pool, assertion_id).await?;
    row.into_domain()
}

pub async fn list_active_for_visibility(
    pool: &PgPool,
    visibility: &AssertionVisibility,
) -> Result<Vec<KnowledgeAssertion>, AssertionRepoError> {
    let select = r#"SELECT assertion_id, visibility, project_scope_id,
              organization_id_at_time, source_operation_id,
              source_scope_snapshot_hash, source_kind, source_id_kind,
              source_id_value, source_stream_key, source_version, subject_key,
              predicate, object_hash, assertion_identity_hash, object_kind,
              object_value, vault_ref, assertion_kind, status, evidence_refs,
              valid_from, valid_to, fresh_until, classification, content_hash
       FROM knowledge_assertions"#;
    let rows = match visibility {
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id,
            organization_id_at_time,
        } => {
            sqlx::query_as::<_, StoredAssertionRow>(&format!(
                r#"{select}
                   WHERE visibility = 'organization_long_term'
                     AND project_scope_id = $1
                     AND organization_id_at_time = $2
                     AND status = 'active'
                   ORDER BY source_stream_key, source_version DESC,
                            assertion_identity_hash, assertion_id"#
            ))
            .bind(project_scope_id.0)
            .bind(organization_id_at_time)
            .fetch_all(pool)
            .await?
        }
        AssertionVisibility::GlobalSanitized => {
            sqlx::query_as::<_, StoredAssertionRow>(&format!(
                r#"{select}
                   WHERE visibility = 'global_sanitized'
                     AND project_scope_id IS NULL
                     AND organization_id_at_time IS NULL
                     AND status = 'active'
                   ORDER BY source_stream_key, source_version DESC,
                            assertion_identity_hash, assertion_id"#
            ))
            .fetch_all(pool)
            .await?
        }
    };
    rows.into_iter()
        .map(StoredAssertionRow::into_domain)
        .collect()
}

pub async fn list_for_event_source(
    pool: &PgPool,
    event: &KnowledgeEventEnvelopeV1,
) -> Result<Vec<KnowledgeAssertion>, AssertionRepoError> {
    event
        .validate()
        .map_err(|_| AssertionRepoError::EventMismatch)?;
    let stored_source_id = StoredCanonicalRowId::from_domain(&event.payload.source.row_id)?;
    let rows = sqlx::query_as::<_, StoredAssertionRow>(&format!(
        r#"{}
           WHERE project_scope_id IS NOT DISTINCT FROM $1
             AND organization_id_at_time IS NOT DISTINCT FROM $2
             AND source_operation_id = $3
             AND source_kind = $4
             AND source_id_kind = $5
             AND source_id_value = $6
             AND source_stream_key = $7
             AND source_version = $8
           ORDER BY assertion_identity_hash, assertion_id"#,
        assertion_select_sql()
    ))
    .bind(event.project_scope_id.map(|id| id.0))
    .bind(event.organization_id_at_time)
    .bind(event.source_operation_id)
    .bind(event.payload.source.source_kind.as_str())
    .bind(&stored_source_id.kind)
    .bind(&stored_source_id.value)
    .bind(&event.payload.source_stream_key)
    .bind(event.payload.source_version)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(StoredAssertionRow::into_domain)
        .collect()
}

async fn get_with_connection(
    connection: &mut PgConnection,
    assertion_id: Uuid,
) -> Result<KnowledgeAssertion, AssertionRepoError> {
    let row = sqlx::query_as::<_, StoredAssertionRow>(&select_by_id_sql())
        .bind(assertion_id)
        .fetch_one(&mut *connection)
        .await?;
    row.into_domain()
}

async fn fetch_row(pool: &PgPool, assertion_id: Uuid) -> Result<StoredAssertionRow, sqlx::Error> {
    sqlx::query_as::<_, StoredAssertionRow>(&select_by_id_sql())
        .bind(assertion_id)
        .fetch_one(pool)
        .await
}

fn select_by_id_sql() -> String {
    format!("{} WHERE assertion_id = $1", assertion_select_sql())
}

fn assertion_select_sql() -> &'static str {
    r#"SELECT assertion_id, visibility, project_scope_id,
              organization_id_at_time, source_operation_id,
              source_scope_snapshot_hash, source_kind, source_id_kind,
              source_id_value, source_stream_key, source_version, subject_key,
              predicate, object_hash, assertion_identity_hash, object_kind,
              object_value, vault_ref, assertion_kind, status, evidence_refs,
              valid_from, valid_to, fresh_until, classification, content_hash
       FROM knowledge_assertions"#
}

fn parse_source_kind(value: &str) -> Result<CanonicalSourceKind, AssertionRepoError> {
    match value {
        "stage_episode" => Ok(CanonicalSourceKind::StageEpisode),
        "finding" => Ok(CanonicalSourceKind::Finding),
        "candidate_attempt" => Ok(CanonicalSourceKind::CandidateAttempt),
        "technique_outcome" => Ok(CanonicalSourceKind::TechniqueOutcome),
        "fact_delta" => Ok(CanonicalSourceKind::FactDelta),
        "post_exploit_action" => Ok(CanonicalSourceKind::PostExploitAction),
        "foothold" => Ok(CanonicalSourceKind::Foothold),
        "objective_outcome" => Ok(CanonicalSourceKind::ObjectiveOutcome),
        "cleanup_obligation" => Ok(CanonicalSourceKind::CleanupObligation),
        "residual_risk" => Ok(CanonicalSourceKind::ResidualRisk),
        "report_revision" => Ok(CanonicalSourceKind::ReportRevision),
        _ => Err(AssertionRepoError::Corrupt("source_kind")),
    }
}

fn parse_assertion_kind(value: &str) -> Result<AssertionKind, AssertionRepoError> {
    match value {
        "observation" => Ok(AssertionKind::Observation),
        "checked_empty" => Ok(AssertionKind::CheckedEmpty),
        "verified_outcome" => Ok(AssertionKind::VerifiedOutcome),
        "refuted_outcome" => Ok(AssertionKind::RefutedOutcome),
        "technique_experience" => Ok(AssertionKind::TechniqueExperience),
        "cleanup_attestation" => Ok(AssertionKind::CleanupAttestation),
        "residual_risk" => Ok(AssertionKind::ResidualRisk),
        _ => Err(AssertionRepoError::Corrupt("assertion_kind")),
    }
}

fn parse_assertion_status(value: &str) -> Result<AssertionStatus, AssertionRepoError> {
    match value {
        "active" => Ok(AssertionStatus::Active),
        "superseded" => Ok(AssertionStatus::Superseded),
        "refuted" => Ok(AssertionStatus::Refuted),
        "expired" => Ok(AssertionStatus::Expired),
        _ => Err(AssertionRepoError::Corrupt("assertion_status")),
    }
}

fn parse_classification(value: &str) -> Result<KnowledgeClassification, AssertionRepoError> {
    match value {
        "public" => Ok(KnowledgeClassification::Public),
        "internal" => Ok(KnowledgeClassification::Internal),
        "customer_confidential" => Ok(KnowledgeClassification::CustomerConfidential),
        "restricted" => Ok(KnowledgeClassification::Restricted),
        _ => Err(AssertionRepoError::Corrupt("classification")),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AssertionRepoError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Source(#[from] SourceRefError),
    #[error("invalid assertion: {0}")]
    Invalid(&'static str),
    #[error("corrupt assertion row: {0}")]
    Corrupt(&'static str),
    #[error("assertion identity replay conflicts with the immutable stored row")]
    ReplayConflict,
    #[error(transparent)]
    Outbox(knowledge_outbox::KnowledgeOutboxError),
    #[error("assertion and typed event identity disagree")]
    EventMismatch,
    #[error("projection invalidation source was not found")]
    InvalidationSourceMissing,
}

impl AssertionRepoError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Sqlx(_) => "memory_assertion_database_error",
            Self::Source(error) => error.code(),
            Self::Invalid(code) => code,
            Self::Corrupt(_) => "memory_assertion_row_corrupt",
            Self::ReplayConflict => "memory_assertion_replay_conflict",
            Self::Outbox(error) => error.code(),
            Self::EventMismatch => "memory_assertion_event_source_mismatch",
            Self::InvalidationSourceMissing => "memory_invalidation_source_missing",
        }
    }
}
