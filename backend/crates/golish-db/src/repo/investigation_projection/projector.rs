use chrono::{DateTime, Utc};
use golish_core::investigation_projection::{
    ProjectionChangeKind, ProjectionEntityKind, ProjectionEntityV1, ProjectionInvalidationReason,
    ProjectionSourceSnapshotV1, ProjectionSourceTimeStatusV1, TimelineEventKind,
};
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::types::{
    sha256_bytes, sha256_json, CapturedProjectionHead, InvestigationProjectionChange,
    InvestigationProjectionError, InvestigationProjectionResult, MaterializedProjectionEntity,
    ProjectionBatchClaim, ProjectionBatchReceipt, ProjectionProjectOutcome, ProjectionReadPage,
};

const BATCH_EXACT_SET_INVALID: &str = "INVESTIGATION_PROJECTION_BATCH_EXACT_SET_INVALID";
const SOURCE_SNAPSHOT_INVALID: &str = "INVESTIGATION_PROJECTION_SOURCE_SNAPSHOT_INVALID";
const ENTITY_PREDECESSOR_INVALID: &str = "INVESTIGATION_PROJECTION_ENTITY_PREDECESSOR_INVALID";
const HEAD_CAS_INVALID: &str = "INVESTIGATION_PROJECTION_HEAD_CAS_INVALID";
const CATALOG_INVALID: &str = "INVESTIGATION_PROJECTION_CATALOG_INVALID";

#[derive(Debug, sqlx::FromRow)]
struct BatchRow {
    batch_id: Uuid,
    operation_id: Uuid,
    source_batch_seq: i64,
    predecessor_batch_id: Option<Uuid>,
    member_count: i64,
    member_set_hash: String,
}

impl From<&BatchRow> for ProjectionBatchClaim {
    fn from(value: &BatchRow) -> Self {
        Self {
            batch_id: value.batch_id,
            operation_id: value.operation_id,
            source_batch_seq: value.source_batch_seq,
            predecessor_batch_id: value.predecessor_batch_id,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct OutboxRow {
    outbox_member_id: Uuid,
    member_ordinal: i32,
    entity_kind: String,
    change_kind: String,
    source_entity_id: Uuid,
    source_entity_version: i64,
    source_entity_hash: String,
    source_occurred_at: Option<DateTime<Utc>>,
    source_time_status: String,
    source_snapshot_hash: String,
    immutable_source_body: Option<Value>,
    source_blob_id: Option<Uuid>,
    source_blob_hash: Option<String>,
    timeline_event_kind: String,
    invalidation_reason: Option<String>,
    member_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
struct EntityReadRow {
    entity_kind: String,
    entity_id: Uuid,
    entity_version: i64,
    projection_hash: String,
    projection_body: Value,
    change_seq: i64,
    invalidation_reason: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct ChangeReadRow {
    change_seq: i64,
    event_id: Uuid,
    batch_id: Uuid,
    source_batch_seq: i64,
    outbox_member_id: Uuid,
    entity_kind: String,
    entity_id: Uuid,
    entity_version: i64,
    change_kind: String,
    timeline_event_kind: String,
    invalidation_reason: Option<String>,
    change_hash: String,
    source_occurred_at: Option<DateTime<Utc>>,
    source_time_status: String,
    projected_at: DateTime<Utc>,
}

fn contract(code: &'static str) -> InvestigationProjectionError {
    InvestigationProjectionError::Contract(code)
}

fn parse_entity_kind(value: &str) -> InvestigationProjectionResult<ProjectionEntityKind> {
    ProjectionEntityKind::try_from(value).map_err(|_| contract(CATALOG_INVALID))
}

fn parse_change_kind(value: &str) -> InvestigationProjectionResult<ProjectionChangeKind> {
    ProjectionChangeKind::try_from(value).map_err(|_| contract(CATALOG_INVALID))
}

fn parse_timeline_kind(value: &str) -> InvestigationProjectionResult<TimelineEventKind> {
    TimelineEventKind::try_from(value).map_err(|_| contract(CATALOG_INVALID))
}

fn parse_source_time_status(
    value: &str,
) -> InvestigationProjectionResult<ProjectionSourceTimeStatusV1> {
    ProjectionSourceTimeStatusV1::try_from(value).map_err(|_| contract(CATALOG_INVALID))
}

fn parse_invalidation_reason(
    value: Option<&str>,
) -> InvestigationProjectionResult<Option<ProjectionInvalidationReason>> {
    value
        .map(|value| {
            ProjectionInvalidationReason::try_from(value).map_err(|_| contract(CATALOG_INVALID))
        })
        .transpose()
}

async fn load_snapshot(
    tx: &mut Transaction<'_, Postgres>,
    row: &OutboxRow,
) -> InvestigationProjectionResult<ProjectionSourceSnapshotV1> {
    let body = match (&row.immutable_source_body, row.source_blob_id) {
        (Some(body), None) if row.source_blob_hash.is_none() => body.clone(),
        (None, Some(blob_id)) => {
            let (content_hash, bytes): (String, Vec<u8>) = sqlx::query_as(
                r#"SELECT content_hash,immutable_redacted_bytes
                     FROM investigation_projection_source_blobs
                    WHERE blob_id=$1 FOR SHARE"#,
            )
            .bind(blob_id)
            .fetch_one(&mut **tx)
            .await?;
            if row.source_blob_hash.as_deref() != Some(content_hash.as_str())
                || sha256_bytes(&bytes) != content_hash
            {
                return Err(contract(SOURCE_SNAPSHOT_INVALID));
            }
            serde_json::from_slice(&bytes)?
        }
        _ => return Err(contract(SOURCE_SNAPSHOT_INVALID)),
    };
    if sha256_json(&body)? != row.source_snapshot_hash {
        return Err(contract(SOURCE_SNAPSHOT_INVALID));
    }
    let snapshot: ProjectionSourceSnapshotV1 = serde_json::from_value(body)?;
    if snapshot.entity_kind().as_str() != row.entity_kind {
        return Err(contract(SOURCE_SNAPSHOT_INVALID));
    }
    let typed = serde_json::to_value(&snapshot)?;
    let record = typed
        .get("record")
        .and_then(Value::as_object)
        .ok_or_else(|| contract(SOURCE_SNAPSHOT_INVALID))?;
    let entity_id = record
        .get("entityId")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| contract(SOURCE_SNAPSHOT_INVALID))?;
    let entity_version = record
        .get("entityVersion")
        .and_then(Value::as_u64)
        .and_then(|value| i64::try_from(value).ok())
        .ok_or_else(|| contract(SOURCE_SNAPSHOT_INVALID))?;
    let content_hash = record
        .get("contentSha256")
        .and_then(Value::as_str)
        .ok_or_else(|| contract(SOURCE_SNAPSHOT_INVALID))?;
    if entity_id != row.source_entity_id
        || entity_version != row.source_entity_version
        || content_hash != row.source_entity_hash
    {
        return Err(contract(SOURCE_SNAPSHOT_INVALID));
    }
    Ok(snapshot)
}

async fn validate_batch_exact_set(
    tx: &mut Transaction<'_, Postgres>,
    batch: &BatchRow,
    members: &[OutboxRow],
) -> InvestigationProjectionResult<()> {
    if i64::try_from(members.len()).ok() != Some(batch.member_count)
        || members
            .iter()
            .enumerate()
            .any(|(ordinal, row)| usize::try_from(row.member_ordinal).ok() != Some(ordinal))
    {
        return Err(contract(BATCH_EXACT_SET_INVALID));
    }
    for (ordinal, row) in members.iter().enumerate() {
        let storage = if row.source_blob_hash.is_some() {
            "blob"
        } else {
            "inline"
        };
        let expected = sha256_json(&json!({
            "domain": "investigation_projection_outbox_member.v1",
            "ordinal": ordinal,
            "entity_kind": row.entity_kind,
            "change_kind": row.change_kind,
            "source_entity_id": row.source_entity_id,
            "source_entity_version": row.source_entity_version,
            "source_entity_hash": row.source_entity_hash,
            "source_snapshot_hash": row.source_snapshot_hash,
            "source_time_status": row.source_time_status,
            "source_occurred_at": row.source_occurred_at,
            "timeline_event_kind": row.timeline_event_kind,
            "invalidation_reason": row.invalidation_reason,
            "storage": storage,
            "source_blob_hash": row.source_blob_hash,
        }))?;
        if expected != row.member_hash {
            return Err(contract(BATCH_EXACT_SET_INVALID));
        }
    }
    let member_hashes = members
        .iter()
        .map(|member| member.member_hash.clone())
        .collect::<Vec<_>>();
    let member_set_hash: String =
        sqlx::query_scalar("SELECT tool_truth_sha256(to_jsonb($1::TEXT[])::TEXT)")
            .bind(member_hashes)
            .fetch_one(&mut **tx)
            .await?;
    if member_set_hash != batch.member_set_hash {
        return Err(contract(BATCH_EXACT_SET_INVALID));
    }
    Ok(())
}

pub async fn claim_next_projection_batch(
    pool: &PgPool,
    operation_id: Uuid,
) -> InvestigationProjectionResult<Option<ProjectionBatchClaim>> {
    let row = sqlx::query_as::<_, ProjectionBatchClaim>(
        r#"SELECT b.batch_id,b.operation_id,b.source_batch_seq,b.predecessor_batch_id
             FROM investigation_projection_outbox_batches b
             LEFT JOIN investigation_projection_batch_receipts r ON r.batch_id=b.batch_id
            WHERE b.operation_id=$1 AND r.batch_id IS NULL
            ORDER BY b.source_batch_seq
            LIMIT 1"#,
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn project_next_projection_batch(
    pool: &PgPool,
    operation_id: Uuid,
) -> InvestigationProjectionResult<Option<ProjectionProjectOutcome>> {
    let Some(claim) = claim_next_projection_batch(pool, operation_id).await? else {
        return Ok(None);
    };
    project_projection_batch(pool, operation_id, claim.batch_id)
        .await
        .map(Some)
}

pub async fn project_projection_batch(
    pool: &PgPool,
    operation_id: Uuid,
    batch_id: Uuid,
) -> InvestigationProjectionResult<ProjectionProjectOutcome> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await?;
    let head = sqlx::query_as::<_, CapturedProjectionHead>(
        r#"SELECT operation_id,projection_schema_version,change_seq,
                  last_projected_batch_id,cursor_salt
             FROM investigation_projection_heads
            WHERE operation_id=$1 FOR UPDATE"#,
    )
    .bind(operation_id)
    .fetch_one(&mut *tx)
    .await?;
    if let Some(receipt) = sqlx::query_as::<_, ProjectionBatchReceipt>(
        r#"SELECT receipt_id,batch_id,operation_id,source_batch_seq,predecessor_batch_id,
                  first_change_seq,last_change_seq,entity_version_manifest_hash,
                  change_manifest_hash,timeline_manifest_hash,projected_at
             FROM investigation_projection_batch_receipts
            WHERE batch_id=$1 AND operation_id=$2"#,
    )
    .bind(batch_id)
    .bind(operation_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(ProjectionProjectOutcome::Replay(receipt));
    }
    let batch = sqlx::query_as::<_, BatchRow>(
        r#"SELECT batch_id,operation_id,source_batch_seq,predecessor_batch_id,
                  member_count,member_set_hash
             FROM investigation_projection_outbox_batches
            WHERE batch_id=$1 AND operation_id=$2 FOR UPDATE"#,
    )
    .bind(batch_id)
    .bind(operation_id)
    .fetch_one(&mut *tx)
    .await?;
    if batch.predecessor_batch_id != head.last_projected_batch_id {
        let claim = ProjectionBatchClaim::from(&batch);
        tx.rollback().await?;
        return Ok(ProjectionProjectOutcome::PredecessorPending(claim));
    }
    let expected_source_seq = if head.last_projected_batch_id.is_some() {
        sqlx::query_scalar::<_, i64>(
            "SELECT source_batch_seq+1 FROM investigation_projection_batch_receipts WHERE batch_id=$1",
        )
        .bind(head.last_projected_batch_id)
        .fetch_one(&mut *tx)
        .await?
    } else {
        1
    };
    if batch.source_batch_seq != expected_source_seq {
        let claim = ProjectionBatchClaim::from(&batch);
        tx.rollback().await?;
        return Ok(ProjectionProjectOutcome::PredecessorPending(claim));
    }
    let members = sqlx::query_as::<_, OutboxRow>(
        r#"SELECT outbox_member_id,member_ordinal,entity_kind,change_kind,
                  source_entity_id,source_entity_version,source_entity_hash,
                  source_occurred_at,source_time_status,source_snapshot_hash,
                  immutable_source_body,source_blob_id,source_blob_hash,
                  timeline_event_kind,invalidation_reason,member_hash
             FROM investigation_projection_outbox
            WHERE batch_id=$1 ORDER BY member_ordinal FOR UPDATE"#,
    )
    .bind(batch_id)
    .fetch_all(&mut *tx)
    .await?;
    validate_batch_exact_set(&mut tx, &batch, &members).await?;

    let projected_at: DateTime<Utc> = sqlx::query_scalar("SELECT transaction_timestamp()")
        .fetch_one(&mut *tx)
        .await?;
    let first_change_seq = head.change_seq + 1;
    let mut entity_manifest = Vec::with_capacity(members.len());
    let mut change_manifest = Vec::with_capacity(members.len());
    let mut timeline_manifest = Vec::with_capacity(members.len());

    for (offset, row) in members.iter().enumerate() {
        let entity_kind = parse_entity_kind(&row.entity_kind)?;
        let change_kind = parse_change_kind(&row.change_kind)?;
        let timeline_event_kind = parse_timeline_kind(&row.timeline_event_kind)?;
        let source_time_status = parse_source_time_status(&row.source_time_status)?;
        let invalidation_reason = parse_invalidation_reason(row.invalidation_reason.as_deref())?;
        if (change_kind == ProjectionChangeKind::Invalidate) != invalidation_reason.is_some()
            || (source_time_status == ProjectionSourceTimeStatusV1::Known)
                != row.source_occurred_at.is_some()
        {
            return Err(contract(CATALOG_INVALID));
        }
        let snapshot = load_snapshot(&mut tx, row).await?;
        let entity = ProjectionEntityV1::from(snapshot);
        let projection_body = serde_json::to_value(&entity)?;
        let projection_hash = sha256_json(&projection_body)?;
        let predecessor = sqlx::query_as::<_, (i64, String)>(
            r#"SELECT entity_version,projection_hash
                 FROM investigation_projection_entity_versions
                WHERE operation_id=$1 AND entity_kind=$2 AND entity_id=$3
                ORDER BY entity_version DESC LIMIT 1 FOR UPDATE"#,
        )
        .bind(operation_id)
        .bind(entity_kind.as_str())
        .bind(row.source_entity_id)
        .fetch_optional(&mut *tx)
        .await?;
        let (predecessor_absent, predecessor_version, predecessor_hash) =
            match (row.source_entity_version, predecessor) {
                (1, None) => (true, None, None),
                (version, Some((previous_version, previous_hash)))
                    if version > 1 && previous_version == version - 1 =>
                {
                    (false, Some(previous_version), Some(previous_hash))
                }
                _ => return Err(contract(ENTITY_PREDECESSOR_INVALID)),
            };
        let change_seq = first_change_seq
            + i64::try_from(offset).map_err(|_| contract(BATCH_EXACT_SET_INVALID))?;
        sqlx::query(
            r#"INSERT INTO investigation_projection_entity_versions(
                   operation_id,entity_kind,entity_id,entity_version,batch_id,source_hash,
                   projection_hash,projection_body,predecessor_absent,
                   predecessor_entity_version,predecessor_projection_hash,change_seq,
                   source_occurred_at,source_time_status,projected_at,invalidation_reason
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)"#,
        )
        .bind(operation_id)
        .bind(entity_kind.as_str())
        .bind(row.source_entity_id)
        .bind(row.source_entity_version)
        .bind(batch_id)
        .bind(&row.source_entity_hash)
        .bind(&projection_hash)
        .bind(&projection_body)
        .bind(predecessor_absent)
        .bind(predecessor_version)
        .bind(predecessor_hash.as_deref())
        .bind(change_seq)
        .bind(row.source_occurred_at)
        .bind(source_time_status.as_str())
        .bind(projected_at)
        .bind(invalidation_reason.map(|value| value.as_str()))
        .execute(&mut *tx)
        .await?;

        let event_identity = format!(
            "investigation-projection-event:v1:{operation_id}:{batch_id}:{}:{change_seq}",
            row.outbox_member_id
        );
        let event_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, event_identity.as_bytes());
        let change_hash = sha256_json(&json!({
            "operation_id": operation_id,
            "change_seq": change_seq,
            "event_id": event_id,
            "batch_id": batch_id,
            "source_batch_seq": batch.source_batch_seq,
            "outbox_member_id": row.outbox_member_id,
            "entity_kind": entity_kind.as_str(),
            "entity_id": row.source_entity_id,
            "entity_version": row.source_entity_version,
            "change_kind": change_kind.as_str(),
            "timeline_event_kind": timeline_event_kind.as_str(),
            "invalidation_reason": invalidation_reason.map(|value| value.as_str()),
            "source_occurred_at": row.source_occurred_at,
            "source_time_status": source_time_status.as_str(),
        }))?;
        sqlx::query(
            r#"INSERT INTO investigation_projection_changes(
                   operation_id,change_seq,event_id,batch_id,source_batch_seq,outbox_member_id,
                   entity_kind,entity_id,entity_version,change_kind,timeline_event_kind,
                   invalidation_reason,change_hash,source_occurred_at,source_time_status,projected_at
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)"#,
        )
        .bind(operation_id)
        .bind(change_seq)
        .bind(event_id)
        .bind(batch_id)
        .bind(batch.source_batch_seq)
        .bind(row.outbox_member_id)
        .bind(entity_kind.as_str())
        .bind(row.source_entity_id)
        .bind(row.source_entity_version)
        .bind(change_kind.as_str())
        .bind(timeline_event_kind.as_str())
        .bind(invalidation_reason.map(|value| value.as_str()))
        .bind(&change_hash)
        .bind(row.source_occurred_at)
        .bind(source_time_status.as_str())
        .bind(projected_at)
        .execute(&mut *tx)
        .await?;
        entity_manifest.push(format!(
            "{}:{}:{}:{}",
            entity_kind.as_str(),
            row.source_entity_id,
            row.source_entity_version,
            projection_hash
        ));
        change_manifest.push(change_hash);
        timeline_manifest.push(format!("{event_id}:{}", timeline_event_kind.as_str()));
    }

    let last_change_seq = first_change_seq
        + i64::try_from(members.len()).map_err(|_| contract(BATCH_EXACT_SET_INVALID))?
        - 1;
    let entity_version_manifest_hash = sha256_json(&entity_manifest)?;
    let change_manifest_hash = sha256_json(&change_manifest)?;
    let timeline_manifest_hash = sha256_json(&timeline_manifest)?;
    let receipt_identity = format!("investigation-projection-receipt:v1:{operation_id}:{batch_id}");
    let receipt_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, receipt_identity.as_bytes());
    sqlx::query(
        r#"INSERT INTO investigation_projection_batch_receipts(
               receipt_id,batch_id,operation_id,source_batch_seq,predecessor_batch_id,
               first_change_seq,last_change_seq,entity_version_manifest_hash,
               change_manifest_hash,timeline_manifest_hash,projected_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
    )
    .bind(receipt_id)
    .bind(batch_id)
    .bind(operation_id)
    .bind(batch.source_batch_seq)
    .bind(batch.predecessor_batch_id)
    .bind(first_change_seq)
    .bind(last_change_seq)
    .bind(&entity_version_manifest_hash)
    .bind(&change_manifest_hash)
    .bind(&timeline_manifest_hash)
    .bind(projected_at)
    .execute(&mut *tx)
    .await?;
    let advance = sqlx::query(
        r#"UPDATE investigation_projection_heads
              SET change_seq=$2,last_projected_batch_id=$3
            WHERE operation_id=$1 AND change_seq=$4
              AND last_projected_batch_id IS NOT DISTINCT FROM $5"#,
    )
    .bind(operation_id)
    .bind(last_change_seq)
    .bind(batch_id)
    .bind(head.change_seq)
    .bind(head.last_projected_batch_id)
    .execute(&mut *tx)
    .await?;
    if advance.rows_affected() != 1 {
        return Err(contract(HEAD_CAS_INVALID));
    }
    let receipt = ProjectionBatchReceipt {
        receipt_id,
        batch_id,
        operation_id,
        source_batch_seq: batch.source_batch_seq,
        predecessor_batch_id: batch.predecessor_batch_id,
        first_change_seq,
        last_change_seq,
        entity_version_manifest_hash,
        change_manifest_hash,
        timeline_manifest_hash,
        projected_at,
    };
    tx.commit().await?;
    Ok(ProjectionProjectOutcome::Applied(receipt))
}

pub async fn capture_projection_head(
    pool: &PgPool,
    operation_id: Uuid,
) -> InvestigationProjectionResult<CapturedProjectionHead> {
    Ok(sqlx::query_as::<_, CapturedProjectionHead>(
        r#"SELECT operation_id,projection_schema_version,change_seq,
                  last_projected_batch_id,cursor_salt
             FROM investigation_projection_heads WHERE operation_id=$1"#,
    )
    .bind(operation_id)
    .fetch_one(pool)
    .await?)
}

pub async fn read_projection_at_head(
    pool: &PgPool,
    head: &CapturedProjectionHead,
) -> InvestigationProjectionResult<ProjectionReadPage> {
    let current = capture_projection_head(pool, head.operation_id).await?;
    if current.projection_schema_version != head.projection_schema_version
        || current.cursor_salt != head.cursor_salt
        || current.change_seq < head.change_seq
    {
        return Err(contract(HEAD_CAS_INVALID));
    }
    let entity_rows = sqlx::query_as::<_, EntityReadRow>(
        r#"SELECT entity_kind,entity_id,entity_version,projection_hash,
                  projection_body,change_seq,invalidation_reason
             FROM investigation_projection_entity_versions
            WHERE operation_id=$1 AND change_seq<=$2
            ORDER BY change_seq"#,
    )
    .bind(head.operation_id)
    .bind(head.change_seq)
    .fetch_all(pool)
    .await?;
    let mut entities = Vec::with_capacity(entity_rows.len());
    for row in entity_rows {
        entities.push(MaterializedProjectionEntity {
            entity_kind: parse_entity_kind(&row.entity_kind)?,
            entity_id: row.entity_id,
            entity_version: row.entity_version,
            projection_hash: row.projection_hash,
            entity: serde_json::from_value(row.projection_body)?,
            change_seq: row.change_seq,
            invalidation_reason: parse_invalidation_reason(row.invalidation_reason.as_deref())?,
        });
    }
    let change_rows = sqlx::query_as::<_, ChangeReadRow>(
        r#"SELECT change_seq,event_id,batch_id,source_batch_seq,outbox_member_id,
                  entity_kind,entity_id,entity_version,change_kind,timeline_event_kind,
                  invalidation_reason,change_hash,source_occurred_at,source_time_status,projected_at
             FROM investigation_projection_changes
            WHERE operation_id=$1 AND change_seq<=$2 ORDER BY change_seq"#,
    )
    .bind(head.operation_id)
    .bind(head.change_seq)
    .fetch_all(pool)
    .await?;
    let mut changes = Vec::with_capacity(change_rows.len());
    for row in change_rows {
        changes.push(InvestigationProjectionChange {
            change_seq: row.change_seq,
            event_id: row.event_id,
            batch_id: row.batch_id,
            source_batch_seq: row.source_batch_seq,
            outbox_member_id: row.outbox_member_id,
            entity_kind: parse_entity_kind(&row.entity_kind)?,
            entity_id: row.entity_id,
            entity_version: row.entity_version,
            change_kind: parse_change_kind(&row.change_kind)?,
            timeline_event_kind: parse_timeline_kind(&row.timeline_event_kind)?,
            invalidation_reason: parse_invalidation_reason(row.invalidation_reason.as_deref())?,
            change_hash: row.change_hash,
            source_occurred_at: row.source_occurred_at,
            source_time_status: parse_source_time_status(&row.source_time_status)?,
            projected_at: row.projected_at,
        });
    }
    Ok(ProjectionReadPage {
        head: head.clone(),
        entities,
        changes,
    })
}
