//! Immutable operation organization-scope snapshot schema contract.
//!
//! Frozen organization UUIDs intentionally have no live `organizations` FK;
//! names and approval provenance survive later organization lifecycle changes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::repo::operation_scope_decisions::{
    self, ApprovedOrgScopeDecision, ApprovedOrgUnit, ScopeDecisionError,
};

pub const SNAPSHOT_TABLE_NAME: &str = "operation_org_scope_snapshots";
pub const UNIT_TABLE_NAME: &str = "operation_org_scope_units";
pub const SNAPSHOT_OPERATION_UNIQUE_SQL: &str = "UNIQUE(id, operation_id)";
pub const SNAPSHOT_ONE_PER_OPERATION_SQL: &str = "UNIQUE(operation_id)";
pub const UNIT_MEMBERSHIP_PRIMARY_KEY_SQL: &str = "PRIMARY KEY(snapshot_id, organization_id)";
pub const UNIT_ORDINAL_UNIQUE_SQL: &str = "UNIQUE(snapshot_id, ordinal)";
pub const UNIT_ROLE_CHECK_SQL: &str = "CHECK (role IN ('root','subsidiary'))";

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationOrgScopeSnapshotRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_decision_id: Uuid,
    pub project_path_at_freeze: String,
    pub root_organization_id: Uuid,
    pub mode: String,
    pub scope_hash: String,
    pub schema_version: i32,
    pub frozen_at: DateTime<Utc>,
    pub sealed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ScopeFreezeError {
    #[error("runtime scope freeze identity mismatch: {code}")]
    IdentityMismatch { code: &'static str },
    #[error("runtime scope freeze conflict: {code}")]
    Conflict { code: &'static str },
    #[error(transparent)]
    Decision(#[from] ScopeDecisionError),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

impl ScopeFreezeError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::IdentityMismatch { code } | Self::Conflict { code } => code,
            Self::Decision(error) => error.code(),
            Self::Sqlx(_) => "scope_freeze_storage",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewOperationOrgScope {
    pub snapshot_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_decision: ApprovedOrgScopeDecision,
    pub project_path_at_freeze: String,
    pub root_organization_id: Uuid,
    pub mode: String,
    pub units: Vec<ApprovedOrgUnit>,
    pub schema_version: i32,
    pub scope_hash: String,
}

impl NewOperationOrgScope {
    pub fn from_decision(
        snapshot_id: Uuid,
        project_path_at_freeze: String,
        decision: &ApprovedOrgScopeDecision,
    ) -> Result<Self, ScopeFreezeError> {
        if project_path_at_freeze.trim().is_empty() {
            return Err(ScopeFreezeError::IdentityMismatch {
                code: "scope_project_path_missing",
            });
        }
        if decision.units.first().is_none_or(|unit| {
            unit.organization_id != decision.root_organization_id
                || unit.depth != 0
                || unit.parent_organization_id.is_some()
        }) {
            return Err(ScopeFreezeError::IdentityMismatch {
                code: "scope_root_unit_mismatch",
            });
        }
        let mut draft = Self {
            snapshot_id,
            operation_id: decision.operation_id,
            project_scope_id: decision.project_scope_id,
            scope_decision: decision.clone(),
            project_path_at_freeze,
            root_organization_id: decision.root_organization_id,
            mode: decision.mode.as_str().to_string(),
            units: decision.units.clone(),
            schema_version: 1,
            scope_hash: String::new(),
        };
        draft.scope_hash = canonical_scope_hash(&draft)?;
        Ok(draft)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenOperationOrgScope {
    pub snapshot: OperationOrgScopeSnapshotRow,
    pub units: Vec<OperationOrgScopeUnitRow>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationOrgScopeUnitRow {
    pub snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub parent_organization_id: Option<Uuid>,
    pub organization_name_at_freeze: String,
    pub role: String,
    pub depth: i32,
    pub ordinal: i32,
    /// Queries loading this row cast PostgreSQL NUMERIC to text, preserving its
    /// exact producer-time representation without a new decimal dependency.
    pub ownership_percent: Option<String>,
    pub decision_row_id: String,
    pub approval_source: Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationOrgScopeRole {
    Root,
    Subsidiary,
}

impl OperationOrgScopeRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Subsidiary => "subsidiary",
        }
    }
}

#[derive(Serialize)]
struct ScopeHashPayload<'a> {
    schema_version: i32,
    project_scope_id: Uuid,
    project_path_at_freeze: &'a str,
    root_organization_id: Uuid,
    mode: &'a str,
    units: &'a [ApprovedOrgUnit],
}

pub fn canonical_scope_hash(draft: &NewOperationOrgScope) -> Result<String, ScopeFreezeError> {
    if draft.schema_version <= 0 {
        return Err(ScopeFreezeError::IdentityMismatch {
            code: "scope_schema_version_invalid",
        });
    }
    let mut units = draft.units.clone();
    units.sort_by_key(|unit| (unit.depth, unit.ordinal, unit.organization_id));
    let payload = serde_json::to_value(ScopeHashPayload {
        schema_version: draft.schema_version,
        project_scope_id: draft.project_scope_id,
        project_path_at_freeze: &draft.project_path_at_freeze,
        root_organization_id: draft.root_organization_id,
        mode: &draft.mode,
        units: &units,
    })
    .map_err(|_| ScopeFreezeError::Conflict {
        code: "scope_hash_serialization_failed",
    })?;
    Ok(operation_scope_decisions::sha256_json(&payload))
}

/// Insert the exact decision, immutable snapshot header and all organization
/// units, then one-way seal the snapshot. The caller owns the transaction so
/// Scoping can continue with submission binding, unit PASS and stage close
/// before committing anything.
pub async fn freeze_with_connection(
    connection: &mut PgConnection,
    draft: &NewOperationOrgScope,
) -> Result<FrozenOperationOrgScope, ScopeFreezeError> {
    let actual_hash = canonical_scope_hash(draft)?;
    if actual_hash != draft.scope_hash {
        return Err(ScopeFreezeError::IdentityMismatch {
            code: "scope_hash_mismatch",
        });
    }
    if draft.scope_decision.operation_id != draft.operation_id
        || draft.scope_decision.project_scope_id != draft.project_scope_id
        || draft.scope_decision.root_organization_id != draft.root_organization_id
        || draft.scope_decision.mode.as_str() != draft.mode
        || draft.scope_decision.units != draft.units
    {
        return Err(ScopeFreezeError::IdentityMismatch {
            code: "scope_decision_snapshot_mismatch",
        });
    }
    operation_scope_decisions::insert_with_connection(connection, &draft.scope_decision).await?;
    sqlx::query(
        r#"INSERT INTO operation_org_scope_snapshots
           (id, operation_id, project_scope_id, scope_decision_id,
            project_path_at_freeze, root_organization_id, mode, scope_hash,
            schema_version)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(draft.snapshot_id)
    .bind(draft.operation_id)
    .bind(draft.project_scope_id)
    .bind(draft.scope_decision.id)
    .bind(&draft.project_path_at_freeze)
    .bind(draft.root_organization_id)
    .bind(&draft.mode)
    .bind(&draft.scope_hash)
    .bind(draft.schema_version)
    .execute(&mut *connection)
    .await?;

    let mut units = draft.units.clone();
    units.sort_by_key(|unit| (unit.depth, unit.ordinal, unit.organization_id));
    for (ordinal, unit) in units.iter_mut().enumerate() {
        unit.ordinal = ordinal as i32;
        let role = if unit.organization_id == draft.root_organization_id {
            OperationOrgScopeRole::Root
        } else {
            OperationOrgScopeRole::Subsidiary
        };
        sqlx::query(
            r#"INSERT INTO operation_org_scope_units
               (snapshot_id, organization_id, parent_organization_id,
                organization_name_at_freeze, role, depth, ordinal,
                ownership_percent, decision_row_id, approval_source)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8::numeric,$9,$10)"#,
        )
        .bind(draft.snapshot_id)
        .bind(unit.organization_id)
        .bind(unit.parent_organization_id)
        .bind(&unit.organization_name)
        .bind(role.as_str())
        .bind(unit.depth)
        .bind(unit.ordinal)
        .bind(unit.ownership_percent.as_deref())
        .bind(&unit.decision_row_id)
        .bind(&unit.approval_source)
        .execute(&mut *connection)
        .await?;
    }
    sqlx::query(
        "UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1 AND sealed_at IS NULL",
    )
    .bind(draft.snapshot_id)
    .execute(&mut *connection)
    .await?;
    load_for_operation_with_connection(connection, draft.operation_id)
        .await?
        .ok_or(ScopeFreezeError::Conflict {
            code: "sealed_scope_snapshot_missing",
        })
}

pub async fn load_for_operation(
    pool: &PgPool,
    operation_id: Uuid,
) -> Result<Option<FrozenOperationOrgScope>, ScopeFreezeError> {
    let mut connection = pool.acquire().await?;
    load_for_operation_with_connection(&mut connection, operation_id).await
}

pub async fn load_for_operation_with_connection(
    connection: &mut PgConnection,
    operation_id: Uuid,
) -> Result<Option<FrozenOperationOrgScope>, ScopeFreezeError> {
    let Some(snapshot) = sqlx::query_as::<_, OperationOrgScopeSnapshotRow>(
        r#"SELECT id, operation_id, project_scope_id, scope_decision_id,
                  project_path_at_freeze, root_organization_id, mode,
                  scope_hash, schema_version, frozen_at, sealed_at
             FROM operation_org_scope_snapshots
            WHERE operation_id=$1"#,
    )
    .bind(operation_id)
    .fetch_optional(&mut *connection)
    .await?
    else {
        return Ok(None);
    };
    let units = sqlx::query_as::<_, OperationOrgScopeUnitRow>(
        r#"SELECT snapshot_id, organization_id, parent_organization_id,
                  organization_name_at_freeze, role, depth, ordinal,
                  ownership_percent::text AS ownership_percent,
                  decision_row_id, approval_source
             FROM operation_org_scope_units
            WHERE snapshot_id=$1
            ORDER BY depth, ordinal, organization_id"#,
    )
    .bind(snapshot.id)
    .fetch_all(connection)
    .await?;
    Ok(Some(FrozenOperationOrgScope { snapshot, units }))
}

/// Resolve one exact sealed operation/project/organization-at-time binding.
/// Application services use this instead of issuing scope SQL from command or
/// bridge layers.
pub async fn sealed_snapshot_id_for_exact_scope(
    pool: &PgPool,
    operation_id: Uuid,
    project_scope_id: Uuid,
    organization_id_at_time: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT snapshot.id
             FROM operation_org_scope_snapshots AS snapshot
             JOIN operation_org_scope_units AS unit ON unit.snapshot_id=snapshot.id
            WHERE snapshot.operation_id=$1 AND snapshot.project_scope_id=$2
              AND snapshot.sealed_at IS NOT NULL AND unit.organization_id=$3"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id_at_time)
    .fetch_optional(pool)
    .await
}

/// Deleting a live organization cascades its current subtree. P1 refuses the
/// delete when any member is referenced by immutable runtime scope history;
/// later phases may replace this with an explicit invalidation workflow.
pub async fn history_exists_for_org_subtree(
    pool: &PgPool,
    organization_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS (
               WITH RECURSIVE subtree AS (
                   SELECT id FROM organizations WHERE id=$1
                   UNION ALL
                   SELECT child.id
                     FROM organizations AS child
                     JOIN subtree ON child.parent_id=subtree.id
               )
               SELECT 1
                 FROM operation_org_scope_units AS historical
                 JOIN subtree ON subtree.id=historical.organization_id
           )"#,
    )
    .bind(organization_id)
    .fetch_one(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_memory_repo_contract_scope_snapshot_retains_frozen_org_identity() {
        assert_eq!(SNAPSHOT_TABLE_NAME, "operation_org_scope_snapshots");
        assert_eq!(UNIT_TABLE_NAME, "operation_org_scope_units");
        assert!(SNAPSHOT_OPERATION_UNIQUE_SQL.contains("UNIQUE(id, operation_id)"));
        assert!(UNIT_MEMBERSHIP_PRIMARY_KEY_SQL.contains("snapshot_id, organization_id"));
        assert!(!UNIT_MEMBERSHIP_PRIMARY_KEY_SQL.contains("REFERENCES organizations"));
        assert!(UNIT_ROLE_CHECK_SQL.contains(OperationOrgScopeRole::Root.as_str()));
    }
}
