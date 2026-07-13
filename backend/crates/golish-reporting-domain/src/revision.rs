use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Building,
    Draft,
    Validated,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationStatus {
    Unpublished,
    Final,
    Superseded,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportRevision {
    pub revision_id: Uuid,
    pub report_id: Uuid,
    pub revision_number: i32,
    pub row_version: i64,
    pub source_set_hash: [u8; 32],
    pub validation_status: ValidationStatus,
    pub publication_status: PublicationStatus,
    pub supersedes_revision_id: Option<Uuid>,
    pub validated_at: Option<DateTime<Utc>>,
    pub finalized_at: Option<DateTime<Utc>>,
}
