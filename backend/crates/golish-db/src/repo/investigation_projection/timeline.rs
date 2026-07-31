//! Typed Timeline semantic mapping over materialized projection changes.

use golish_core::investigation_projection::{
    InvestigationTimelineEventV1, PersistedProjectionChangeV1, PersistedProjectionEntityVersionV1,
    ProjectionChangeKind, ProjectionEntityKind, ProjectionEntityV1, ProjectionInvalidationReason,
    ProjectionSourceTimeStatusV1, TimelineEventKind,
};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::types::{invalid_payload, InvestigationProjectionResult, InvestigationReadAuthority};
use super::InvestigationProjectionReadSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationTimelineQuery {
    pub after: Option<(i64, Uuid)>,
    pub page_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationTimelinePage {
    pub authority: InvestigationReadAuthority,
    pub events: Vec<InvestigationTimelineEventV1>,
    pub next_key: Option<(i64, Uuid)>,
}

#[derive(Debug, sqlx::FromRow)]
struct TimelineRow {
    operation_id: Uuid,
    event_id: Uuid,
    change_seq: i64,
    batch_id: Uuid,
    source_batch_seq: i64,
    outbox_member_id: Uuid,
    entity_kind: String,
    entity_id: String,
    entity_version: i64,
    change_kind: String,
    timeline_event_kind: String,
    invalidation_reason: Option<String>,
    change_source_hash: String,
    change_projection_hash: String,
    change_hash: String,
    source_occurred_at: Option<chrono::DateTime<chrono::Utc>>,
    source_time_status: String,
    projected_at: chrono::DateTime<chrono::Utc>,
    entity_batch_id: Uuid,
    entity_change_seq: i64,
    entity_source_hash: String,
    entity_projection_hash: String,
    projection_body: Value,
}

fn parse_row(row: TimelineRow) -> InvestigationProjectionResult<InvestigationTimelineEventV1> {
    let entity_kind = ProjectionEntityKind::try_from(row.entity_kind.as_str())
        .map_err(|error| invalid_payload(error.to_string()))?;
    let change_kind = ProjectionChangeKind::try_from(row.change_kind.as_str())
        .map_err(|error| invalid_payload(error.to_string()))?;
    let event_kind = TimelineEventKind::try_from(row.timeline_event_kind.as_str())
        .map_err(|error| invalid_payload(error.to_string()))?;
    let source_time_status =
        ProjectionSourceTimeStatusV1::try_from(row.source_time_status.as_str())
            .map_err(|error| invalid_payload(error.to_string()))?;
    let invalidation_reason = row
        .invalidation_reason
        .as_deref()
        .map(ProjectionInvalidationReason::try_from)
        .transpose()
        .map_err(|error| invalid_payload(error.to_string()))?;
    let entity: ProjectionEntityV1 = serde_json::from_value(row.projection_body)
        .map_err(|error| invalid_payload(format!("typed Timeline entity: {error}")))?;
    if entity.entity_kind() != entity_kind {
        return Err(invalid_payload("Timeline entity kind mismatch"));
    }
    let organization_id = if let ProjectionEntityV1::Hypothesis(record) = &entity {
        record
            .record()
            .canonical_redacted_body()
            .as_value()
            .get("semantic_key")
            .and_then(|value| value.get("organization_id"))
            .and_then(Value::as_str)
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|_| invalid_payload("Timeline Hypothesis organization id invalid"))?
    } else {
        None
    };
    let entity_version = u64::try_from(row.entity_version)
        .map_err(|_| invalid_payload("Timeline entity version invalid"))?;
    InvestigationTimelineEventV1::try_from_persisted(
        PersistedProjectionChangeV1 {
            operation_id: row.operation_id,
            event_id: row.event_id,
            change_seq: row.change_seq,
            batch_id: row.batch_id,
            source_batch_seq: row.source_batch_seq,
            outbox_member_id: row.outbox_member_id,
            entity_kind,
            entity_id: row.entity_id.clone(),
            entity_version,
            change_kind,
            event_kind,
            organization_id,
            source_occurred_at: row.source_occurred_at,
            source_time_status,
            projected_at: row.projected_at,
            invalidation_reason,
            source_hash: row.change_source_hash,
            projection_hash: row.change_projection_hash,
            change_hash: row.change_hash.clone(),
        },
        PersistedProjectionEntityVersionV1 {
            operation_id: row.operation_id,
            batch_id: row.entity_batch_id,
            change_seq: row.entity_change_seq,
            entity_kind,
            entity_id: row.entity_id,
            entity_version,
            source_hash: row.entity_source_hash,
            projection_hash: row.entity_projection_hash,
            entity,
            change_hash: row.change_hash,
        },
    )
    .map_err(|error| invalid_payload(error.to_string()))
}

pub async fn read_investigation_timeline(
    pool: &PgPool,
    operation_id: Uuid,
    query: InvestigationTimelineQuery,
) -> InvestigationProjectionResult<InvestigationTimelinePage> {
    let mut snapshot = InvestigationProjectionReadSnapshot::begin(pool, operation_id).await?;
    let head = snapshot.authority.temporal.as_of_change_seq;
    let (after_change_seq, after_event_id) = query.after.unwrap_or((0, Uuid::nil()));
    let page_size = query.page_size.clamp(1, 100) as i64;
    let rows = sqlx::query_as::<_, TimelineRow>(
        r#"SELECT change.operation_id,change.event_id,change.change_seq,change.batch_id,
                  change.source_batch_seq,change.outbox_member_id,change.entity_kind,
                  change.entity_id,change.entity_version,change.change_kind,
                  change.timeline_event_kind,change.invalidation_reason,
                  change.source_hash AS change_source_hash,
                  change.projection_hash AS change_projection_hash,change.change_hash,
                  change.source_occurred_at,change.source_time_status,change.projected_at,
                  entity.batch_id AS entity_batch_id,entity.change_seq AS entity_change_seq,
                  entity.source_hash AS entity_source_hash,
                  entity.projection_hash AS entity_projection_hash,entity.projection_body
             FROM investigation_projection_changes change
             JOIN investigation_projection_entity_versions entity
               ON entity.operation_id=change.operation_id
              AND entity.entity_kind=change.entity_kind
              AND entity.entity_id=change.entity_id
              AND entity.entity_version=change.entity_version
            WHERE change.operation_id=$1 AND change.change_seq<=$2
              AND (change.change_seq,change.event_id)>($3,$4)
            ORDER BY change.change_seq,change.event_id LIMIT $5"#,
    )
    .bind(operation_id)
    .bind(head)
    .bind(after_change_seq)
    .bind(after_event_id)
    .bind(page_size + 1)
    .fetch_all(&mut *snapshot.tx)
    .await?;
    let has_more = i64::try_from(rows.len()).unwrap_or(i64::MAX) > page_size;
    let mut events = rows
        .into_iter()
        .take(page_size as usize)
        .map(parse_row)
        .collect::<InvestigationProjectionResult<Vec<_>>>()?;
    events.sort_by_key(|event| (event.change_seq, event.event_id));
    let next_key = has_more
        .then(|| {
            events
                .last()
                .map(|event| (event.change_seq, event.event_id))
        })
        .flatten();
    let authority = snapshot.authority.clone();
    snapshot.finish().await?;
    Ok(InvestigationTimelinePage {
        authority,
        events,
        next_key,
    })
}
