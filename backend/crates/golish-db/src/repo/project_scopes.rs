//! Stable workspace/project identity schema contract.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

use super::runtime_memory_tx::{RuntimeMemoryStoreError, RuntimeMemoryStoreResult};

pub const TABLE_NAME: &str = "project_scopes";
pub const PRIMARY_KEY_SQL: &str = "PRIMARY KEY(project_scope_id)";
pub const ONE_ACTIVE_PATH_INDEX_NAME: &str = "project_scopes_one_active_path";
pub const ONE_ACTIVE_PATH_INDEX_SQL: &str = "CREATE UNIQUE INDEX project_scopes_one_active_path \
     ON project_scopes(canonical_project_path) WHERE retired_at IS NULL";

const REGISTER_FIRST_OPEN_SQL: &str = r#"INSERT INTO project_scopes
        (project_scope_id, canonical_project_path, path_sha256)
    VALUES ($1, $2, $3)
    ON CONFLICT (canonical_project_path) WHERE retired_at IS NULL
    DO UPDATE SET canonical_project_path = EXCLUDED.canonical_project_path
    RETURNING project_scope_id, canonical_project_path, path_sha256,
              row_version, created_at, updated_at, retired_at"#;
const RENAME_SQL: &str = r#"UPDATE project_scopes
    SET canonical_project_path = $4,
        path_sha256 = $5,
        row_version = row_version + 1,
        updated_at = NOW()
    WHERE project_scope_id = $1
      AND canonical_project_path = $2
      AND row_version = $3
      AND retired_at IS NULL
    RETURNING project_scope_id, canonical_project_path, path_sha256,
              row_version, created_at, updated_at, retired_at"#;
const GET_BY_ID_FOR_SHARE_SQL: &str = r#"SELECT project_scope_id, canonical_project_path,
        path_sha256, row_version, created_at, updated_at, retired_at
    FROM project_scopes
    WHERE project_scope_id = $1
    FOR SHARE"#;
const GET_ACTIVE_FOR_SHARE_SQL: &str = r#"SELECT project_scope_id, canonical_project_path,
        path_sha256, row_version, created_at, updated_at, retired_at
    FROM project_scopes
    WHERE project_scope_id = $1 AND retired_at IS NULL
    FOR SHARE"#;

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectScopeRow {
    pub project_scope_id: Uuid,
    pub canonical_project_path: String,
    pub path_sha256: String,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub retired_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectScopeStatus {
    Active,
    Retired,
}

impl ProjectScopeRow {
    pub fn status(&self) -> ProjectScopeStatus {
        if self.retired_at.is_some() {
            ProjectScopeStatus::Retired
        } else {
            ProjectScopeStatus::Active
        }
    }
}

/// Register a trusted canonical workspace path on first open. Reopening the
/// same active path returns its stable identity; a different hash for the same
/// path fails closed instead of silently rebinding provenance.
pub async fn register_first_open(
    pool: &PgPool,
    canonical_project_path: &str,
    path_sha256: &str,
) -> RuntimeMemoryStoreResult<ProjectScopeRow> {
    let row = sqlx::query_as::<_, ProjectScopeRow>(REGISTER_FIRST_OPEN_SQL)
        .bind(Uuid::new_v4())
        .bind(canonical_project_path)
        .bind(path_sha256)
        .fetch_one(pool)
        .await?;
    if row.path_sha256 != path_sha256 {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "project_scope_path_hash_mismatch",
        });
    }
    Ok(row)
}

/// Rename an active project identity only when id, old path, and row version
/// all match. The explicit witnesses prevent a new path from being guessed as
/// a rename of some unrelated workspace.
pub async fn rename(
    pool: &PgPool,
    project_scope_id: Uuid,
    expected_old_path: &str,
    expected_row_version: i64,
    new_path: &str,
    new_path_sha256: &str,
) -> RuntimeMemoryStoreResult<ProjectScopeRow> {
    let mut tx = pool.begin().await?;
    let renamed = sqlx::query_as::<_, ProjectScopeRow>(RENAME_SQL)
        .bind(project_scope_id)
        .bind(expected_old_path)
        .bind(expected_row_version)
        .bind(new_path)
        .bind(new_path_sha256)
        .fetch_optional(&mut *tx)
        .await;

    let renamed = match renamed {
        Ok(row) => row,
        Err(error) if is_unique_violation(&error) => {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "project_scope_active_path_conflict",
            });
        }
        Err(error) => return Err(error.into()),
    };
    if let Some(row) = renamed {
        tx.commit().await?;
        return Ok(row);
    }

    let current = get_by_id_for_share(&mut *tx, project_scope_id)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing { entity: TABLE_NAME })?;
    if current.retired_at.is_some() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "project_scope_retired",
        });
    }
    if current.row_version != expected_row_version {
        return Err(RuntimeMemoryStoreError::StaleVersion {
            entity: TABLE_NAME,
            expected: expected_row_version,
            actual: current.row_version,
        });
    }
    if current.canonical_project_path != expected_old_path {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "project_scope_old_path_mismatch",
        });
    }
    Err(RuntimeMemoryStoreError::Conflict {
        code: "project_scope_rename_cas_failed",
    })
}

pub async fn get_active_for_share<'e, E>(
    executor: E,
    project_scope_id: Uuid,
) -> RuntimeMemoryStoreResult<Option<ProjectScopeRow>>
where
    E: Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as::<_, ProjectScopeRow>(GET_ACTIVE_FOR_SHARE_SQL)
        .bind(project_scope_id)
        .fetch_optional(executor)
        .await?;
    Ok(row)
}

/// Enumerate stable project roots for process-level filesystem lifecycle jobs.
/// Retired identities remain included because immutable report revisions can
/// continue referencing blobs under their original project root.
pub async fn list_all(pool: &PgPool) -> RuntimeMemoryStoreResult<Vec<ProjectScopeRow>> {
    Ok(sqlx::query_as::<_, ProjectScopeRow>(
        r#"SELECT project_scope_id, canonical_project_path, path_sha256,
                  row_version, created_at, updated_at, retired_at
             FROM project_scopes
            ORDER BY project_scope_id"#,
    )
    .fetch_all(pool)
    .await?)
}

async fn get_by_id_for_share<'e, E>(
    executor: E,
    project_scope_id: Uuid,
) -> RuntimeMemoryStoreResult<Option<ProjectScopeRow>>
where
    E: Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as::<_, ProjectScopeRow>(GET_BY_ID_FOR_SHARE_SQL)
        .bind(project_scope_id)
        .fetch_optional(executor)
        .await?;
    Ok(row)
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database) if database.code().as_deref() == Some("23505")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_memory_repo_contract_project_scope_has_one_active_path() {
        assert_eq!(TABLE_NAME, "project_scopes");
        assert!(ONE_ACTIVE_PATH_INDEX_SQL.contains("canonical_project_path"));
        assert!(ONE_ACTIVE_PATH_INDEX_SQL.contains("retired_at IS NULL"));
        assert_eq!(ONE_ACTIVE_PATH_INDEX_NAME, "project_scopes_one_active_path");
    }

    #[test]
    fn runtime_memory_store_project_scope_first_open_and_rename_are_cas_guarded() {
        assert!(REGISTER_FIRST_OPEN_SQL.contains("ON CONFLICT (canonical_project_path)"));
        assert!(REGISTER_FIRST_OPEN_SQL.contains("WHERE retired_at IS NULL"));
        assert!(REGISTER_FIRST_OPEN_SQL.contains("RETURNING"));
        assert!(RENAME_SQL.contains("project_scope_id = $1"));
        assert!(RENAME_SQL.contains("canonical_project_path = $2"));
        assert!(RENAME_SQL.contains("row_version = $3"));
        assert!(RENAME_SQL.contains("row_version = row_version + 1"));
    }
}
