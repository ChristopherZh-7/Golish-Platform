use chrono::{DateTime, Utc};
use golish_core::investigation_projection::{
    ProjectionChangeKind, ProjectionEntityKind, ProjectionEntityV1, ProjectionInvalidationReason,
    ProjectionSourceTimeStatusV1, TimelineEventKind,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionBatchEnqueueReceipt {
    pub batch_id: Uuid,
    pub operation_id: Uuid,
    pub source_batch_seq: i64,
    pub predecessor_batch_id: Option<Uuid>,
    pub member_count: i64,
    pub member_set_hash: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct ProjectionBatchClaim {
    pub batch_id: Uuid,
    pub operation_id: Uuid,
    pub source_batch_seq: i64,
    pub predecessor_batch_id: Option<Uuid>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct ProjectionBatchReceipt {
    pub receipt_id: Uuid,
    pub batch_id: Uuid,
    pub operation_id: Uuid,
    pub source_batch_seq: i64,
    pub predecessor_batch_id: Option<Uuid>,
    pub first_change_seq: i64,
    pub last_change_seq: i64,
    pub entity_version_manifest_hash: String,
    pub change_manifest_hash: String,
    pub timeline_manifest_hash: String,
    pub projected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionProjectOutcome {
    Applied(ProjectionBatchReceipt),
    Replay(ProjectionBatchReceipt),
    PredecessorPending(ProjectionBatchClaim),
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct CapturedProjectionHead {
    pub operation_id: Uuid,
    pub projection_schema_version: i32,
    pub change_seq: i64,
    pub last_projected_batch_id: Option<Uuid>,
    pub cursor_salt: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedProjectionEntity {
    pub entity_kind: ProjectionEntityKind,
    pub entity_id: Uuid,
    pub entity_version: i64,
    pub projection_hash: String,
    pub entity: ProjectionEntityV1,
    pub change_seq: i64,
    pub invalidation_reason: Option<ProjectionInvalidationReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationProjectionChange {
    pub change_seq: i64,
    pub event_id: Uuid,
    pub batch_id: Uuid,
    pub source_batch_seq: i64,
    pub outbox_member_id: Uuid,
    pub entity_kind: ProjectionEntityKind,
    pub entity_id: Uuid,
    pub entity_version: i64,
    pub change_kind: ProjectionChangeKind,
    pub timeline_event_kind: TimelineEventKind,
    pub invalidation_reason: Option<ProjectionInvalidationReason>,
    pub change_hash: String,
    pub source_occurred_at: Option<DateTime<Utc>>,
    pub source_time_status: ProjectionSourceTimeStatusV1,
    pub projected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionReadPage {
    pub head: CapturedProjectionHead,
    pub entities: Vec<MaterializedProjectionEntity>,
    pub changes: Vec<InvestigationProjectionChange>,
}

#[derive(Debug, thiserror::Error)]
pub enum InvestigationProjectionError {
    #[error("INVESTIGATION_PROJECTION_STORAGE: {0}")]
    Storage(#[from] sqlx::Error),
    #[error("INVESTIGATION_PROJECTION_SERIALIZATION: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("{0}")]
    Contract(&'static str),
}

impl InvestigationProjectionError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Storage(_) => "INVESTIGATION_PROJECTION_STORAGE",
            Self::Serialization(_) => "INVESTIGATION_PROJECTION_SERIALIZATION",
            Self::Contract(code) => code,
        }
    }
}

pub type InvestigationProjectionResult<T> = Result<T, InvestigationProjectionError>;

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!("sha256:{hex}")
}

pub(crate) fn sha256_json<T: Serialize>(value: &T) -> InvestigationProjectionResult<String> {
    Ok(sha256_bytes(&serde_json::to_vec(value)?))
}
