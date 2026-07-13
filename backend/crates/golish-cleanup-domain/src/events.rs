use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{CleanupObligationId, CleanupObligationStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupEventKind {
    ObligationOpened,
    AttemptTerminal,
    ObligationTerminal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CleanupEvent {
    pub event_id: Uuid,
    pub kind: CleanupEventKind,
    pub obligation_id: CleanupObligationId,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub status: CleanupObligationStatus,
    pub evidence_ids: Vec<i64>,
    pub occurred_at: DateTime<Utc>,
}
