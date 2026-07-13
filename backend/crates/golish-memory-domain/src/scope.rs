use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable identifier from the P1 `project_scopes` registry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectScopeId(pub Uuid);

/// Exact scope of one operation. This is intentionally separate from the
/// cross-operation visibility of a long-term assertion.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OperationScope {
    pub project_scope_id: ProjectScopeId,
    pub source_operation_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub scope_snapshot_hash: String,
}

impl OperationScope {
    pub fn validate(&self) -> Result<(), ScopeValidationError> {
        if self.scope_snapshot_hash.trim().is_empty() {
            return Err(ScopeValidationError::EmptySnapshotHash);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ScopeValidationError {
    #[error("scope snapshot hash cannot be empty")]
    EmptySnapshotHash,
}

impl ScopeValidationError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::EmptySnapshotHash => "memory_scope_snapshot_hash_empty",
        }
    }
}
