//! Immutable projection-source boundary for Hypothesis Registry writes.
//!
//! Canonical writers may append one complete, typed source batch and advance
//! only the source head.  Materialized entity/legacy rows and the projection
//! head remain exclusively owned by the whole-batch projector.

use chrono::{DateTime, Utc};
use golish_core::investigation_projection::{
    projection_timeline_event_kind, ProjectionChangeKind, ProjectionInvalidationReason,
    ProjectionSourceSnapshotV1, ProjectionSourceTimeStatusV1,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{DbError, Result};

const SOURCE_BATCH_REPLAY_DRIFT: &str = "INVESTIGATION_SOURCE_BATCH_REPLAY_DRIFT";
const SOURCE_BATCH_EMPTY: &str = "INVESTIGATION_SOURCE_BATCH_EMPTY";
const SOURCE_SNAPSHOT_ID_INVALID: &str = "INVESTIGATION_SOURCE_SNAPSHOT_ID_INVALID";
const SOURCE_TIMELINE_ROUTE_INVALID: &str = "INVESTIGATION_SOURCE_TIMELINE_ROUTE_INVALID";

fn conflict(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!("sha256:{hex}")
}

#[derive(Debug, Clone)]
pub struct ProjectionOutboxSourceRow {
    pub outbox_member_id: Uuid,
    pub change_kind: ProjectionChangeKind,
    pub source: ProjectionSourceSnapshotV1,
    pub source_occurred_at: Option<DateTime<Utc>>,
    pub source_time_status: ProjectionSourceTimeStatusV1,
    pub invalidation_reason: Option<ProjectionInvalidationReason>,
    pub storage: ProjectionSourceStorageV1,
}

/// Storage policy is selected by trusted repository code.  Blob bytes and
/// hashes are always derived from the typed snapshot; callers cannot provide
/// either value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionSourceStorageV1 {
    Inline,
    Blob { redaction_contract_version: String },
}

#[derive(Debug, Clone)]
pub struct AppendProjectionSourceBatchRow {
    pub batch_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Option<Uuid>,
    pub stable_request_id: Uuid,
    pub source_transaction_id: Uuid,
    pub source_occurred_at: Option<DateTime<Utc>>,
    pub source_time_status: ProjectionSourceTimeStatusV1,
    pub members: Vec<ProjectionOutboxSourceRow>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct ProjectionSourceBatchView {
    pub batch_id: Uuid,
    pub operation_id: Uuid,
    pub source_batch_seq: i64,
    pub predecessor_batch_id: Option<Uuid>,
    pub stable_request_id: Uuid,
    pub source_transaction_id: Uuid,
    pub member_count: i64,
    pub member_set_hash: String,
}

#[derive(Debug)]
struct PreparedSourceMember {
    outbox_member_id: Uuid,
    member_ordinal: i32,
    entity_kind: &'static str,
    change_kind: &'static str,
    source_entity_id: Uuid,
    source_entity_version: i64,
    source_entity_hash: String,
    source_occurred_at: Option<DateTime<Utc>>,
    source_time_status: &'static str,
    source_snapshot_hash: String,
    immutable_source_body: Option<Value>,
    blob_bytes: Option<Vec<u8>>,
    blob_hash: Option<String>,
    blob_redaction_contract_version: Option<String>,
    timeline_event_kind: &'static str,
    invalidation_reason: Option<&'static str>,
    member_hash: String,
}

fn prepare_member(member: &ProjectionOutboxSourceRow) -> Result<PreparedSourceMember> {
    let entity_kind = member.source.entity_kind();
    let timeline_event_kind = projection_timeline_event_kind(entity_kind, member.change_kind)
        .ok_or_else(|| conflict(SOURCE_TIMELINE_ROUTE_INVALID))?;
    if (member.change_kind == ProjectionChangeKind::Invalidate)
        != member.invalidation_reason.is_some()
    {
        return Err(conflict(SOURCE_TIMELINE_ROUTE_INVALID));
    }
    if (member.source_time_status == ProjectionSourceTimeStatusV1::Known)
        != member.source_occurred_at.is_some()
    {
        return Err(conflict(SOURCE_TIMELINE_ROUTE_INVALID));
    }

    let body = serde_json::to_value(&member.source)?;
    let record = body
        .get("record")
        .and_then(Value::as_object)
        .ok_or_else(|| conflict(SOURCE_SNAPSHOT_ID_INVALID))?;
    let source_entity_id = record
        .get("entityId")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| conflict(SOURCE_SNAPSHOT_ID_INVALID))?;
    let source_entity_version = record
        .get("entityVersion")
        .and_then(Value::as_u64)
        .and_then(|value| i64::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| conflict(SOURCE_SNAPSHOT_ID_INVALID))?;
    let source_entity_hash = record
        .get("contentSha256")
        .and_then(Value::as_str)
        .ok_or_else(|| conflict(SOURCE_SNAPSHOT_ID_INVALID))?
        .to_owned();
    let source_bytes = serde_json::to_vec(&body)?;
    let source_snapshot_hash = sha256_bytes(&source_bytes);
    let invalidation_reason = member.invalidation_reason.map(|value| value.as_str());
    Ok(PreparedSourceMember {
        outbox_member_id: member.outbox_member_id,
        member_ordinal: -1,
        entity_kind: entity_kind.as_str(),
        change_kind: member.change_kind.as_str(),
        source_entity_id,
        source_entity_version,
        source_entity_hash,
        source_occurred_at: member.source_occurred_at,
        source_time_status: member.source_time_status.as_str(),
        source_snapshot_hash: source_snapshot_hash.clone(),
        immutable_source_body: matches!(member.storage, ProjectionSourceStorageV1::Inline)
            .then_some(body),
        blob_bytes: matches!(member.storage, ProjectionSourceStorageV1::Blob { .. })
            .then_some(source_bytes),
        blob_hash: matches!(member.storage, ProjectionSourceStorageV1::Blob { .. })
            .then_some(source_snapshot_hash.clone()),
        blob_redaction_contract_version: match &member.storage {
            ProjectionSourceStorageV1::Inline => None,
            ProjectionSourceStorageV1::Blob {
                redaction_contract_version,
            } => Some(redaction_contract_version.clone()),
        },
        timeline_event_kind: timeline_event_kind.as_str(),
        invalidation_reason,
        member_hash: String::new(),
    })
}

fn finalize_prepared_member(member: &mut PreparedSourceMember, ordinal: usize) -> Result<()> {
    member.member_ordinal = i32::try_from(ordinal).map_err(|_| conflict(SOURCE_BATCH_EMPTY))?;
    member.member_hash = sha256_bytes(&serde_json::to_vec(&json!({
        "domain": "investigation_projection_outbox_member.v1",
        "ordinal": ordinal,
        "entity_kind": member.entity_kind,
        "change_kind": member.change_kind,
        "source_entity_id": member.source_entity_id,
        "source_entity_version": member.source_entity_version,
        "source_entity_hash": member.source_entity_hash,
        "source_snapshot_hash": member.source_snapshot_hash,
        "source_time_status": member.source_time_status,
        "source_occurred_at": member.source_occurred_at,
        "timeline_event_kind": member.timeline_event_kind,
        "invalidation_reason": member.invalidation_reason,
        "storage": if member.blob_hash.is_some() { "blob" } else { "inline" },
        "source_blob_hash": member.blob_hash,
    }))?);
    Ok(())
}

/// Append a complete source batch inside the caller's canonical transaction.
///
/// This function intentionally has no `PgPool` overload: root/revision,
/// generation, residual and outbox truth must share one commit boundary.
pub(crate) async fn append_projection_source_batch_on(
    tx: &mut Transaction<'_, Postgres>,
    input: AppendProjectionSourceBatchRow,
) -> Result<ProjectionSourceBatchView> {
    if input.members.is_empty() {
        return Err(conflict(SOURCE_BATCH_EMPTY));
    }
    if (input.source_time_status == ProjectionSourceTimeStatusV1::Known)
        != input.source_occurred_at.is_some()
    {
        return Err(conflict(SOURCE_TIMELINE_ROUTE_INVALID));
    }

    sqlx::query("SELECT operation_id FROM operation_state WHERE operation_id=$1 FOR UPDATE")
        .bind(input.operation_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| DbError::NotFound("operation_state".into()))?;
    let (last_source_batch_seq, predecessor_batch_id): (i64, Option<Uuid>) = sqlx::query_as(
        r#"SELECT last_source_batch_seq,last_source_batch_id
             FROM investigation_projection_source_heads
            WHERE operation_id=$1 FOR UPDATE"#,
    )
    .bind(input.operation_id)
    .fetch_one(&mut **tx)
    .await?;

    let mut prepared = input
        .members
        .iter()
        .map(prepare_member)
        .collect::<Result<Vec<_>>>()?;
    prepared.sort_by(|left, right| {
        (
            left.entity_kind,
            left.source_entity_id,
            left.source_entity_version,
            left.change_kind,
        )
            .cmp(&(
                right.entity_kind,
                right.source_entity_id,
                right.source_entity_version,
                right.change_kind,
            ))
    });
    for (ordinal, member) in prepared.iter_mut().enumerate() {
        finalize_prepared_member(member, ordinal)?;
    }
    let member_hashes = prepared
        .iter()
        .map(|member| member.member_hash.clone())
        .collect::<Vec<_>>();
    let member_set_hash: String =
        sqlx::query_scalar("SELECT tool_truth_sha256(to_jsonb($1::TEXT[])::TEXT)")
            .bind(&member_hashes)
            .fetch_one(&mut **tx)
            .await?;
    let member_count = i64::try_from(prepared.len()).map_err(|_| conflict(SOURCE_BATCH_EMPTY))?;

    if let Some(existing) = sqlx::query_as::<_, ProjectionSourceBatchView>(
        r#"SELECT batch_id,operation_id,source_batch_seq,predecessor_batch_id,
                  stable_request_id,source_transaction_id,member_count,member_set_hash
             FROM investigation_projection_outbox_batches
            WHERE operation_id=$1 AND stable_request_id=$2
            ORDER BY source_batch_seq LIMIT 1"#,
    )
    .bind(input.operation_id)
    .bind(input.stable_request_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        if existing.batch_id != input.batch_id
            || existing.source_transaction_id != input.source_transaction_id
            || existing.member_count != member_count
            || existing.member_set_hash != member_set_hash
        {
            return Err(conflict(SOURCE_BATCH_REPLAY_DRIFT));
        }
        return Ok(existing);
    }

    let source_batch_seq = last_source_batch_seq + 1;
    sqlx::query(
        r#"INSERT INTO investigation_projection_outbox_batches(
               batch_id,operation_id,project_scope_id,source_batch_seq,
               predecessor_batch_id,stable_request_id,source_transaction_id,
               member_count,member_set_hash,source_occurred_at,source_time_status
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
    )
    .bind(input.batch_id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(source_batch_seq)
    .bind(predecessor_batch_id)
    .bind(input.stable_request_id)
    .bind(input.source_transaction_id)
    .bind(member_count)
    .bind(&member_set_hash)
    .bind(input.source_occurred_at)
    .bind(input.source_time_status.as_str())
    .execute(&mut **tx)
    .await?;

    for member in prepared {
        let source_blob_id = if let (Some(bytes), Some(content_hash), Some(redaction_version)) = (
            member.blob_bytes.as_ref(),
            member.blob_hash.as_ref(),
            member.blob_redaction_contract_version.as_ref(),
        ) {
            let derived_blob_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, content_hash.as_bytes());
            sqlx::query(
                r#"INSERT INTO investigation_projection_source_blobs(
                       blob_id,content_hash,byte_count,immutable_redacted_bytes,
                       redaction_contract_version,redaction_metadata
                   ) VALUES($1,$2,$3,$4,$5,$6)
                   ON CONFLICT(payload_schema,payload_schema_version,content_hash) DO NOTHING"#,
            )
            .bind(derived_blob_id)
            .bind(content_hash)
            .bind(i64::try_from(bytes.len()).map_err(|_| conflict(SOURCE_BATCH_EMPTY))?)
            .bind(bytes)
            .bind(redaction_version)
            .bind(json!({
                "source": "typed_projection_source_snapshot.v1",
                "redacted": true,
            }))
            .execute(&mut **tx)
            .await?;
            Some(
                sqlx::query_scalar::<_, Uuid>(
                    r#"SELECT blob_id FROM investigation_projection_source_blobs
                        WHERE payload_schema='projection_source_snapshot.v1'
                          AND payload_schema_version=1 AND content_hash=$1"#,
                )
                .bind(content_hash)
                .fetch_one(&mut **tx)
                .await?,
            )
        } else {
            None
        };
        sqlx::query(
            r#"INSERT INTO investigation_projection_outbox(
                   outbox_member_id,batch_id,operation_id,source_batch_seq,member_ordinal,
                   entity_kind,change_kind,source_entity_id,source_entity_version,
                   source_entity_hash,source_occurred_at,source_time_status,
                   source_snapshot_hash,immutable_source_body,source_blob_id,source_blob_hash,
                   timeline_event_kind,invalidation_reason,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)"#,
        )
        .bind(member.outbox_member_id)
        .bind(input.batch_id)
        .bind(input.operation_id)
        .bind(source_batch_seq)
        .bind(member.member_ordinal)
        .bind(member.entity_kind)
        .bind(member.change_kind)
        .bind(member.source_entity_id)
        .bind(member.source_entity_version)
        .bind(&member.source_entity_hash)
        .bind(member.source_occurred_at)
        .bind(member.source_time_status)
        .bind(&member.source_snapshot_hash)
        .bind(member.immutable_source_body)
        .bind(source_blob_id)
        .bind(member.blob_hash)
        .bind(member.timeline_event_kind)
        .bind(member.invalidation_reason)
        .bind(&member.member_hash)
        .execute(&mut **tx)
        .await?;
    }

    let head_advance = sqlx::query(
        r#"UPDATE investigation_projection_source_heads
              SET last_source_batch_seq=$2,last_source_batch_id=$3
            WHERE operation_id=$1 AND last_source_batch_seq=$4
              AND last_source_batch_id IS NOT DISTINCT FROM $5"#,
    )
    .bind(input.operation_id)
    .bind(source_batch_seq)
    .bind(input.batch_id)
    .bind(last_source_batch_seq)
    .bind(predecessor_batch_id)
    .execute(&mut **tx)
    .await?;
    if head_advance.rows_affected() != 1 {
        return Err(conflict("INVESTIGATION_SOURCE_HEAD_CAS_INVALID"));
    }

    Ok(ProjectionSourceBatchView {
        batch_id: input.batch_id,
        operation_id: input.operation_id,
        source_batch_seq,
        predecessor_batch_id,
        stable_request_id: input.stable_request_id,
        source_transaction_id: input.source_transaction_id,
        member_count,
        member_set_hash,
    })
}
