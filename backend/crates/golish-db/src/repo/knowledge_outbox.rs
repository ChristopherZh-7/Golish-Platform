use chrono::{DateTime, Utc};
use golish_memory_domain::event_catalog::{
    routes_for, EventCatalogError, KnowledgeEventEnvelopeV1, KnowledgeEventNameV1,
    KnowledgeEventPayloadV1, ProjectorId,
};
use golish_memory_domain::scope::ProjectScopeId;
use golish_memory_domain::source_ref::StoredCanonicalRowId;
use serde_json::Value;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectorLifecycle {
    Enabled,
    Paused,
    Disabled,
}

impl ProjectorLifecycle {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Paused => "paused",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryStatus {
    BlockedDependency,
    Pending,
    Leased,
    Succeeded,
    SucceededSuppressed,
    RetryableFailed,
    Stale,
    DeadLetter,
}

pub async fn get_delivery_status(
    pool: &PgPool,
    event_id: Uuid,
    projector: ProjectorId,
) -> Result<Option<DeliveryStatus>, KnowledgeOutboxError> {
    sqlx::query_scalar::<_, String>(
        r#"SELECT status
             FROM knowledge_projection_deliveries
            WHERE event_id=$1 AND projector_name=$2 AND projector_schema_version=$3"#,
    )
    .bind(event_id)
    .bind(projector.name())
    .bind(projector.schema_version())
    .fetch_optional(pool)
    .await?
    .map(|status| DeliveryStatus::parse(&status))
    .transpose()
}

impl DeliveryStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BlockedDependency => "blocked_dependency",
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Succeeded => "succeeded",
            Self::SucceededSuppressed => "succeeded_suppressed",
            Self::RetryableFailed => "retryable_failed",
            Self::Stale => "stale",
            Self::DeadLetter => "dead_letter",
        }
    }

    fn parse(value: &str) -> Result<Self, KnowledgeOutboxError> {
        match value {
            "blocked_dependency" => Ok(Self::BlockedDependency),
            "pending" => Ok(Self::Pending),
            "leased" => Ok(Self::Leased),
            "succeeded" => Ok(Self::Succeeded),
            "succeeded_suppressed" => Ok(Self::SucceededSuppressed),
            "retryable_failed" => Ok(Self::RetryableFailed),
            "stale" => Ok(Self::Stale),
            "dead_letter" => Ok(Self::DeadLetter),
            other => Err(KnowledgeOutboxError::CorruptStatus(other.to_string())),
        }
    }
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct RawDeliveryRow {
    event_id: Uuid,
    projector_name: String,
    projector_schema_version: i32,
    status: String,
    attempt_count: i32,
    lease_owner: Option<String>,
    lease_expires_at: Option<DateTime<Utc>>,
    depends_on_projector: Option<String>,
    depends_on_schema_version: Option<i32>,
    terminal_reason: Option<String>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct ExistingEventIdentity {
    event_id: Uuid,
    event_name: String,
    schema_version: i32,
    project_scope_id: Option<Uuid>,
    organization_id_at_time: Option<Uuid>,
    source_operation_id: Uuid,
    source_kind: String,
    source_id_kind: String,
    source_id_value: String,
    source_stream_key: String,
    source_version: i64,
    payload: Value,
    occurred_at: DateTime<Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct StoredKnowledgeEventRow {
    event_id: Uuid,
    event_name: String,
    schema_version: i32,
    project_scope_id: Option<Uuid>,
    organization_id_at_time: Option<Uuid>,
    source_operation_id: Uuid,
    source_kind: String,
    source_id_kind: String,
    source_id_value: String,
    source_stream_key: String,
    source_version: i64,
    payload: Value,
    occurred_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryRow {
    pub event_id: Uuid,
    pub projector_name: String,
    pub projector_schema_version: i32,
    pub status: DeliveryStatus,
    pub attempt_count: i32,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub depends_on_projector: Option<String>,
    pub depends_on_schema_version: Option<i32>,
    pub terminal_reason: Option<String>,
}

impl TryFrom<RawDeliveryRow> for DeliveryRow {
    type Error = KnowledgeOutboxError;

    fn try_from(value: RawDeliveryRow) -> Result<Self, Self::Error> {
        Ok(Self {
            event_id: value.event_id,
            projector_name: value.projector_name,
            projector_schema_version: value.projector_schema_version,
            status: DeliveryStatus::parse(&value.status)?,
            attempt_count: value.attempt_count,
            lease_owner: value.lease_owner,
            lease_expires_at: value.lease_expires_at,
            depends_on_projector: value.depends_on_projector,
            depends_on_schema_version: value.depends_on_schema_version,
            terminal_reason: value.terminal_reason,
        })
    }
}

pub async fn set_projector_lifecycle(
    pool: &PgPool,
    projector: ProjectorId,
    lifecycle: ProjectorLifecycle,
    disabled_reason: Option<&str>,
) -> Result<(), KnowledgeOutboxError> {
    if lifecycle == ProjectorLifecycle::Disabled
        && disabled_reason.is_none_or(|reason| reason.trim().is_empty())
    {
        return Err(KnowledgeOutboxError::DisabledReasonRequired);
    }
    sqlx::query(
        r#"UPDATE knowledge_projector_registry
           SET lifecycle = $3,
               disabled_reason = $4,
               updated_at = NOW()
           WHERE projector_name = $1 AND projector_schema_version = $2"#,
    )
    .bind(projector.name())
    .bind(projector.schema_version())
    .bind(lifecycle.as_str())
    .bind(disabled_reason)
    .execute(pool)
    .await?;
    Ok(())
}

/// Activates a projector implementation that has reached the composition
/// root. A persisted administrative `disabled` decision is never overridden;
/// this only advances the migration-time `paused` placeholder to `enabled`.
pub async fn activate_paused_projector(
    pool: &PgPool,
    projector: ProjectorId,
) -> Result<bool, KnowledgeOutboxError> {
    let activated = sqlx::query_scalar::<_, String>(
        r#"UPDATE knowledge_projector_registry
           SET lifecycle = 'enabled', disabled_reason = NULL, updated_at = NOW()
           WHERE projector_name = $1
             AND projector_schema_version = $2
             AND lifecycle = 'paused'
           RETURNING lifecycle"#,
    )
    .bind(projector.name())
    .bind(projector.schema_version())
    .fetch_optional(pool)
    .await?;
    if activated.is_some() {
        return Ok(true);
    }
    let exists = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
               SELECT 1 FROM knowledge_projector_registry
               WHERE projector_name = $1 AND projector_schema_version = $2
           )"#,
    )
    .bind(projector.name())
    .bind(projector.schema_version())
    .fetch_one(pool)
    .await?;
    if !exists {
        return Err(KnowledgeOutboxError::ProjectorMissing(projector.key()));
    }
    Ok(false)
}

pub async fn append_event_with_catalog_deliveries(
    tx: &mut Transaction<'_, Postgres>,
    event: &KnowledgeEventEnvelopeV1,
) -> Result<Uuid, KnowledgeOutboxError> {
    append_event_with_catalog_deliveries_with_connection(tx, event).await
}

pub async fn append_event_with_catalog_deliveries_with_connection(
    connection: &mut PgConnection,
    event: &KnowledgeEventEnvelopeV1,
) -> Result<Uuid, KnowledgeOutboxError> {
    event.validate()?;
    let stored = StoredCanonicalRowId::from_domain(&event.payload.source.row_id)
        .map_err(|error| KnowledgeOutboxError::InvalidSource(error.code()))?;
    let payload = serde_json::to_value(&event.payload)?;
    let dedupe_key = event.dedupe_key()?;
    let inserted = sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO knowledge_outbox_events (
               event_id, event_name, schema_version, project_scope_id,
               organization_id_at_time, source_operation_id, source_kind,
               source_id_kind, source_id_value, source_stream_key, source_version,
               payload, occurred_at, dedupe_key
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
           ON CONFLICT (dedupe_key) DO NOTHING
           RETURNING event_id"#,
    )
    .bind(event.event_id)
    .bind(event.event_name.as_str())
    .bind(event.schema_version)
    .bind(event.project_scope_id.map(|id| id.0))
    .bind(event.organization_id_at_time)
    .bind(event.source_operation_id)
    .bind(event.payload.source.source_kind.as_str())
    .bind(&stored.kind)
    .bind(&stored.value)
    .bind(&event.payload.source_stream_key)
    .bind(event.payload.source_version)
    .bind(&payload)
    .bind(event.occurred_at)
    .bind(&dedupe_key)
    .fetch_optional(&mut *connection)
    .await?;
    let actual_event_id = match inserted {
        Some(event_id) => event_id,
        None => {
            let existing = sqlx::query_as::<_, ExistingEventIdentity>(
                r#"SELECT event_id, event_name, schema_version, project_scope_id,
                          organization_id_at_time, source_operation_id, source_kind,
                          source_id_kind, source_id_value, source_stream_key,
                          source_version, payload, occurred_at
                   FROM knowledge_outbox_events WHERE dedupe_key = $1"#,
            )
            .bind(&dedupe_key)
            .fetch_one(&mut *connection)
            .await?;
            if existing.event_id != event.event_id
                || existing.event_name != event.event_name.as_str()
                || existing.schema_version != event.schema_version
                || existing.project_scope_id != event.project_scope_id.map(|id| id.0)
                || existing.organization_id_at_time != event.organization_id_at_time
                || existing.source_operation_id != event.source_operation_id
                || existing.source_kind != event.payload.source.source_kind.as_str()
                || existing.source_id_kind != stored.kind
                || existing.source_id_value != stored.value
                || existing.source_stream_key != event.payload.source_stream_key
                || existing.source_version != event.payload.source_version
                || existing.payload != payload
                || existing.occurred_at.timestamp_micros() != event.occurred_at.timestamp_micros()
            {
                return Err(KnowledgeOutboxError::DedupeConflict);
            }
            existing.event_id
        }
    };

    let is_stale = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
               SELECT 1 FROM knowledge_outbox_events
               WHERE project_scope_id IS NOT DISTINCT FROM $1
                 AND source_stream_key = $2
                 AND source_version > $3
           )"#,
    )
    .bind(event.project_scope_id.map(|id| id.0))
    .bind(&event.payload.source_stream_key)
    .bind(event.payload.source_version)
    .fetch_one(&mut *connection)
    .await?;

    for route in routes_for(event.event_name) {
        let (lifecycle, disabled_reason) = sqlx::query_as::<_, (String, Option<String>)>(
            r#"SELECT lifecycle, disabled_reason
               FROM knowledge_projector_registry
               WHERE projector_name = $1 AND projector_schema_version = $2"#,
        )
        .bind(route.projector.name())
        .bind(route.projector.schema_version())
        .fetch_one(&mut *connection)
        .await?;
        let (status, terminal_reason) = if is_stale {
            (DeliveryStatus::Stale, Some("newer_source_version_exists"))
        } else if lifecycle == "disabled" {
            (
                DeliveryStatus::SucceededSuppressed,
                Some(disabled_reason.as_deref().unwrap_or("projector_disabled")),
            )
        } else if let Some(dependency) = route.depends_on {
            let dependency_terminal = sqlx::query_scalar::<_, bool>(
                r#"SELECT status IN ('succeeded','succeeded_suppressed')
                   FROM knowledge_projection_deliveries
                   WHERE event_id = $1
                     AND projector_name = $2
                     AND projector_schema_version = $3"#,
            )
            .bind(actual_event_id)
            .bind(dependency.name())
            .bind(dependency.schema_version())
            .fetch_one(&mut *connection)
            .await?;
            if dependency_terminal {
                (DeliveryStatus::Pending, None)
            } else {
                (DeliveryStatus::BlockedDependency, None)
            }
        } else {
            (DeliveryStatus::Pending, None)
        };

        sqlx::query(
            r#"INSERT INTO knowledge_projection_deliveries (
                   event_id, projector_name, projector_schema_version, status,
                   depends_on_projector, depends_on_schema_version,
                   terminal_reason, completed_at
               ) VALUES ($1,$2,$3,$4,$5,$6,$7,
                   CASE WHEN $4 IN ('succeeded_suppressed','stale') THEN NOW() ELSE NULL END)
               ON CONFLICT (event_id, projector_name, projector_schema_version) DO NOTHING"#,
        )
        .bind(actual_event_id)
        .bind(route.projector.name())
        .bind(route.projector.schema_version())
        .bind(status.as_str())
        .bind(route.depends_on.map(ProjectorId::name))
        .bind(route.depends_on.map(ProjectorId::schema_version))
        .bind(terminal_reason)
        .execute(&mut *connection)
        .await?;
    }

    Ok(actual_event_id)
}

pub async fn list_deliveries(
    pool: &PgPool,
    event_id: Uuid,
) -> Result<Vec<DeliveryRow>, KnowledgeOutboxError> {
    sqlx::query_as::<_, RawDeliveryRow>(
        r#"SELECT event_id, projector_name, projector_schema_version, status,
                  attempt_count, lease_owner, lease_expires_at,
                  depends_on_projector, depends_on_schema_version, terminal_reason
           FROM knowledge_projection_deliveries
           WHERE event_id = $1
           ORDER BY projector_name"#,
    )
    .bind(event_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(TryInto::try_into)
    .collect()
}

pub async fn claim_delivery_batch(
    pool: &PgPool,
    projector: ProjectorId,
    worker: &str,
    limit: i64,
) -> Result<Vec<DeliveryRow>, KnowledgeOutboxError> {
    if worker.trim().is_empty() || limit <= 0 {
        return Err(KnowledgeOutboxError::InvalidClaim);
    }
    let mut tx = pool.begin().await?;
    let rows = sqlx::query_as::<_, RawDeliveryRow>(
        r#"WITH claimable AS (
               SELECT delivery.event_id
               FROM knowledge_projection_deliveries delivery
               JOIN knowledge_projector_registry registry
                 ON registry.projector_name = delivery.projector_name
                AND registry.projector_schema_version = delivery.projector_schema_version
               WHERE delivery.projector_name = $1
                 AND delivery.projector_schema_version = $2
                 AND registry.lifecycle = 'enabled'
                 AND delivery.available_at <= NOW()
                 AND (
                     delivery.status IN ('pending','retryable_failed')
                     OR (delivery.status = 'leased' AND delivery.lease_expires_at <= NOW())
                 )
                 AND (
                     delivery.depends_on_projector IS NULL
                     OR EXISTS (
                         SELECT 1 FROM knowledge_projection_deliveries dependency
                         WHERE dependency.event_id = delivery.event_id
                           AND dependency.projector_name = delivery.depends_on_projector
                           AND dependency.projector_schema_version = delivery.depends_on_schema_version
                           AND dependency.status IN ('succeeded','succeeded_suppressed')
                     )
                 )
               ORDER BY delivery.available_at, delivery.event_id
               FOR UPDATE OF delivery SKIP LOCKED
               LIMIT $4
           )
           UPDATE knowledge_projection_deliveries delivery
           SET status = 'leased',
               lease_owner = $3,
               lease_expires_at = NOW() + INTERVAL '60 seconds',
               attempt_count = delivery.attempt_count + 1,
               updated_at = NOW()
           FROM claimable
           WHERE delivery.event_id = claimable.event_id
             AND delivery.projector_name = $1
             AND delivery.projector_schema_version = $2
           RETURNING delivery.event_id, delivery.projector_name,
                     delivery.projector_schema_version, delivery.status,
                     delivery.attempt_count, delivery.lease_owner,
                     delivery.lease_expires_at, delivery.depends_on_projector,
                     delivery.depends_on_schema_version, delivery.terminal_reason"#,
    )
    .bind(projector.name())
    .bind(projector.schema_version())
    .bind(worker)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    rows.into_iter().map(TryInto::try_into).collect()
}

pub async fn complete_delivery(
    pool: &PgPool,
    event_id: Uuid,
    projector: ProjectorId,
    worker: &str,
    outcome: DeliveryStatus,
    terminal_reason: Option<&str>,
) -> Result<(), KnowledgeOutboxError> {
    if !matches!(
        outcome,
        DeliveryStatus::Succeeded | DeliveryStatus::SucceededSuppressed
    ) {
        return Err(KnowledgeOutboxError::InvalidTerminalOutcome);
    }
    if outcome == DeliveryStatus::SucceededSuppressed
        && terminal_reason.is_none_or(|reason| reason.trim().is_empty())
    {
        return Err(KnowledgeOutboxError::SuppressionReasonRequired);
    }
    let mut tx = pool.begin().await?;
    let updated = sqlx::query(
        r#"UPDATE knowledge_projection_deliveries
           SET status = $5,
               lease_owner = NULL,
               lease_expires_at = NULL,
               terminal_reason = $6,
               completed_at = NOW(),
               updated_at = NOW()
           WHERE event_id = $1
             AND projector_name = $2
             AND projector_schema_version = $3
             AND status = 'leased'
             AND lease_owner = $4
             AND lease_expires_at > NOW()"#,
    )
    .bind(event_id)
    .bind(projector.name())
    .bind(projector.schema_version())
    .bind(worker)
    .bind(outcome.as_str())
    .bind(terminal_reason)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(KnowledgeOutboxError::LeaseFenceLost);
    }
    sqlx::query(
        r#"UPDATE knowledge_projection_deliveries dependent
           SET status = 'pending', updated_at = NOW()
           WHERE dependent.event_id = $1
             AND dependent.status = 'blocked_dependency'
             AND dependent.depends_on_projector = $2
             AND dependent.depends_on_schema_version = $3
             AND EXISTS (
                 SELECT 1 FROM knowledge_projection_deliveries predecessor
                 WHERE predecessor.event_id = dependent.event_id
                   AND predecessor.projector_name = dependent.depends_on_projector
                   AND predecessor.projector_schema_version = dependent.depends_on_schema_version
                   AND predecessor.status IN ('succeeded','succeeded_suppressed')
             )"#,
    )
    .bind(event_id)
    .bind(projector.name())
    .bind(projector.schema_version())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn fail_delivery(
    pool: &PgPool,
    event_id: Uuid,
    projector: ProjectorId,
    worker: &str,
    error_code: &str,
    max_attempts: i32,
) -> Result<DeliveryStatus, KnowledgeOutboxError> {
    if worker.trim().is_empty() || error_code.trim().is_empty() || max_attempts <= 0 {
        return Err(KnowledgeOutboxError::InvalidFailure);
    }
    let raw_status = sqlx::query_scalar::<_, String>(
        r#"UPDATE knowledge_projection_deliveries
           SET status = CASE
                   WHEN attempt_count >= $6 THEN 'dead_letter'
                   ELSE 'retryable_failed'
               END,
               lease_owner = NULL,
               lease_expires_at = NULL,
               last_error = $5,
               terminal_reason = CASE
                   WHEN attempt_count >= $6 THEN $5
                   ELSE NULL
               END,
               available_at = CASE
                   WHEN attempt_count >= $6 THEN available_at
                   ELSE NOW() + INTERVAL '1 second'
                        * LEAST(300, GREATEST(1, attempt_count * attempt_count))
               END,
               completed_at = CASE
                   WHEN attempt_count >= $6 THEN NOW()
                   ELSE NULL
               END,
               updated_at = NOW()
           WHERE event_id = $1
             AND projector_name = $2
             AND projector_schema_version = $3
             AND status = 'leased'
             AND lease_owner = $4
             AND lease_expires_at > NOW()
           RETURNING status"#,
    )
    .bind(event_id)
    .bind(projector.name())
    .bind(projector.schema_version())
    .bind(worker)
    .bind(error_code.trim())
    .bind(max_attempts)
    .fetch_optional(pool)
    .await?;
    let status = raw_status.ok_or(KnowledgeOutboxError::LeaseFenceLost)?;
    DeliveryStatus::parse(&status)
}

pub async fn event_payload(pool: &PgPool, event_id: Uuid) -> Result<Value, KnowledgeOutboxError> {
    Ok(
        sqlx::query_scalar("SELECT payload FROM knowledge_outbox_events WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(pool)
            .await?,
    )
}

/// Loads the immutable typed event envelope consumed by projector adapters.
/// Redundant source columns are checked against the serialized payload so a
/// corrupt row cannot silently redirect a delivery to another canonical row.
pub async fn get_event(
    pool: &PgPool,
    event_id: Uuid,
) -> Result<KnowledgeEventEnvelopeV1, KnowledgeOutboxError> {
    let stored = sqlx::query_as::<_, StoredKnowledgeEventRow>(
        r#"SELECT event_id, event_name, schema_version, project_scope_id,
                  organization_id_at_time, source_operation_id, source_kind,
                  source_id_kind, source_id_value, source_stream_key,
                  source_version, payload, occurred_at
           FROM knowledge_outbox_events WHERE event_id = $1"#,
    )
    .bind(event_id)
    .fetch_one(pool)
    .await?;
    let payload: KnowledgeEventPayloadV1 = serde_json::from_value(stored.payload)?;
    let payload_source_id = StoredCanonicalRowId::from_domain(&payload.source.row_id)
        .map_err(|error| KnowledgeOutboxError::InvalidSource(error.code()))?;
    if payload.source.source_kind.as_str() != stored.source_kind
        || payload_source_id.kind != stored.source_id_kind
        || payload_source_id.value != stored.source_id_value
        || payload.source_stream_key != stored.source_stream_key
        || payload.source_version != stored.source_version
    {
        return Err(KnowledgeOutboxError::StoredEventCorrupt);
    }
    let event_name = match stored.event_name.as_str() {
        "StageEpisodeClosed.v1" => KnowledgeEventNameV1::StageEpisodeClosed,
        "CandidateAttemptTerminal.v1" => KnowledgeEventNameV1::CandidateAttemptTerminal,
        "FactDeltaAccepted.v1" => KnowledgeEventNameV1::FactDeltaAccepted,
        "PostExploitActionPrepared.v1" => KnowledgeEventNameV1::PostExploitActionPrepared,
        "PostExploitFactTerminal.v1" => KnowledgeEventNameV1::PostExploitFactTerminal,
        "CleanupObligationTerminal.v1" => KnowledgeEventNameV1::CleanupObligationTerminal,
        "SourceScopeInvalidated.v1" => KnowledgeEventNameV1::SourceScopeInvalidated,
        "ReportRevisionFinalized.v1" => KnowledgeEventNameV1::ReportRevisionFinalized,
        _ => return Err(KnowledgeOutboxError::StoredEventCorrupt),
    };
    let event = KnowledgeEventEnvelopeV1 {
        event_id: stored.event_id,
        project_scope_id: stored.project_scope_id.map(ProjectScopeId),
        organization_id_at_time: stored.organization_id_at_time,
        source_operation_id: stored.source_operation_id,
        event_name,
        schema_version: stored.schema_version,
        payload,
        occurred_at: stored.occurred_at,
    };
    event.validate().map_err(KnowledgeOutboxError::Catalog)?;
    Ok(event)
}

#[derive(Debug, thiserror::Error)]
pub enum KnowledgeOutboxError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Catalog(#[from] EventCatalogError),
    #[error("invalid canonical source: {0}")]
    InvalidSource(&'static str),
    #[error("corrupt delivery status: {0}")]
    CorruptStatus(String),
    #[error("disabled projector requires a reason")]
    DisabledReasonRequired,
    #[error("invalid delivery claim")]
    InvalidClaim,
    #[error("invalid delivery terminal outcome")]
    InvalidTerminalOutcome,
    #[error("suppressed delivery requires a reason")]
    SuppressionReasonRequired,
    #[error("delivery lease fence was lost")]
    LeaseFenceLost,
    #[error("invalid delivery failure record")]
    InvalidFailure,
    #[error("outbox dedupe identity conflicts with the stored immutable event")]
    DedupeConflict,
    #[error("projector registry row is missing for {0}")]
    ProjectorMissing(&'static str),
    #[error("stored knowledge event columns disagree with its typed payload")]
    StoredEventCorrupt,
}

impl KnowledgeOutboxError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Sqlx(_) => "memory_outbox_database_error",
            Self::Json(_) => "memory_outbox_payload_invalid",
            Self::Catalog(error) => error.code(),
            Self::InvalidSource(code) => code,
            Self::CorruptStatus(_) => "memory_delivery_status_corrupt",
            Self::DisabledReasonRequired => "memory_projector_disabled_reason_required",
            Self::InvalidClaim => "memory_delivery_claim_invalid",
            Self::InvalidTerminalOutcome => "memory_delivery_terminal_outcome_invalid",
            Self::SuppressionReasonRequired => "memory_delivery_suppression_reason_required",
            Self::LeaseFenceLost => "memory_delivery_lease_fence_lost",
            Self::InvalidFailure => "memory_delivery_failure_invalid",
            Self::DedupeConflict => "memory_outbox_dedupe_conflict",
            Self::ProjectorMissing(_) => "memory_projector_registry_missing",
            Self::StoredEventCorrupt => "memory_outbox_event_corrupt",
        }
    }
}
