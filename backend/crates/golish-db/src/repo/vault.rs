use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::models::{VaultEntry, VaultEntryType};

/// Frontend-safe projection of a vault entry (secret `value` omitted). Carries
/// `entry_type` as text plus the `status`/`source_url`/`last_validated_at`
/// columns that the richer `VaultEntry` model does not expose.
#[derive(Debug, Clone, FromRow)]
pub struct VaultSafeRow {
    pub id: Uuid,
    pub name: String,
    pub entry_type: String,
    pub username: String,
    pub notes: String,
    pub project: String,
    pub tags: serde_json::Value,
    pub status: String,
    pub source_url: String,
    pub last_validated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const SAFE_COLS: &str = "id, name, entry_type::TEXT, username, notes, project, tags, status, source_url, last_validated_at, created_at, updated_at";

pub async fn create(
    pool: &PgPool,
    name: &str,
    entry_type: VaultEntryType,
    value: &str,
    username: &str,
    notes: &str,
    project: &str,
    tags: &serde_json::Value,
    project_path: Option<&str>,
) -> Result<VaultEntry> {
    let row = sqlx::query_as::<_, VaultEntry>(
        r#"INSERT INTO vault_entries (name, entry_type, value, username, notes, project, tags, project_path)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING *"#,
    )
    .bind(name)
    .bind(entry_type)
    .bind(value)
    .bind(username)
    .bind(notes)
    .bind(project)
    .bind(tags)
    .bind(project_path)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<VaultEntry>> {
    super::scoped::list_by_project(pool, "vault_entries", "created_at DESC", project_path).await
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<VaultEntry>> {
    super::scoped::get_by_id(pool, "vault_entries", id).await
}

pub async fn update(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    value: &str,
    username: &str,
    notes: &str,
    tags: &serde_json::Value,
) -> Result<()> {
    sqlx::query(
        "UPDATE vault_entries SET name=$1, value=$2, username=$3, notes=$4, tags=$5, updated_at=NOW() WHERE id=$6",
    )
    .bind(name).bind(value).bind(username).bind(notes).bind(tags).bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    super::scoped::delete_by_id(pool, "vault_entries", id).await?;
    Ok(())
}

// ── Project-scoped helpers (AGENTS.md I2). Command layer must route through
// these instead of writing scoped SQL inline. ─────────────────────────────

/// List frontend-safe vault rows for a project (exact `project_path = $1`).
pub async fn list_safe_by_project(
    pool: &PgPool,
    project_path: Option<&str>,
) -> Result<Vec<VaultSafeRow>> {
    let sql = format!(
        "SELECT {SAFE_COLS} FROM vault_entries WHERE project_path = $1 ORDER BY created_at DESC"
    );
    let rows = sqlx::query_as::<_, VaultSafeRow>(&sql)
        .bind(project_path)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Fetch a single frontend-safe vault row by id (no project scope; caller
/// scope-guards via `exists_scoped` before mutating).
pub async fn get_safe(pool: &PgPool, id: Uuid) -> Result<Option<VaultSafeRow>> {
    let sql = format!("SELECT {SAFE_COLS} FROM vault_entries WHERE id=$1");
    let row = sqlx::query_as::<_, VaultSafeRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// Insert a vault entry with the encrypted value + source_url (entry_type as text).
#[allow(clippy::too_many_arguments)]
pub async fn insert_full(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    entry_type: &str,
    enc_value: &str,
    username: &str,
    notes: &str,
    project: &str,
    tags: &serde_json::Value,
    source_url: &str,
    project_path: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO vault_entries (id, name, entry_type, value, username, notes, project, tags, source_url, project_path)
           VALUES ($1, $2, $3::vault_entry_type, $4, $5, $6, $7, $8, $9, $10)"#,
    )
    .bind(id)
    .bind(name)
    .bind(entry_type)
    .bind(enc_value)
    .bind(username)
    .bind(notes)
    .bind(project)
    .bind(tags)
    .bind(source_url)
    .bind(project_path)
    .execute(pool)
    .await?;
    Ok(())
}

/// Read the encrypted value scoped to project_path (IDOR). `None` == missing/other project.
pub async fn get_value_scoped(
    pool: &PgPool,
    id: Uuid,
    project_path: Option<&str>,
) -> Result<Option<String>> {
    let v = sqlx::query_scalar::<_, String>(
        "SELECT value FROM vault_entries WHERE id = $1 AND project_path IS NOT DISTINCT FROM $2",
    )
    .bind(id)
    .bind(project_path)
    .fetch_optional(pool)
    .await?;
    Ok(v)
}

/// Existence + ownership check scoped to project_path (IDOR). Returns the id when owned.
pub async fn exists_scoped(
    pool: &PgPool,
    id: Uuid,
    project_path: Option<&str>,
) -> Result<Option<Uuid>> {
    let v = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM vault_entries WHERE id = $1 AND project_path IS NOT DISTINCT FROM $2",
    )
    .bind(id)
    .bind(project_path)
    .fetch_optional(pool)
    .await?;
    Ok(v)
}

/// Read `(value, source_url, entry_type::TEXT)` scoped to project_path (IDOR), for validation.
pub async fn get_validate_fields_scoped(
    pool: &PgPool,
    id: Uuid,
    project_path: Option<&str>,
) -> Result<Option<(String, String, String)>> {
    let row = sqlx::query_as::<_, (String, String, String)>(
        "SELECT value, source_url, entry_type::TEXT FROM vault_entries \
         WHERE id=$1 AND project_path IS NOT DISTINCT FROM $2",
    )
    .bind(id)
    .bind(project_path)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Apply the subset of vault fields that are `Some` (by id only; caller scope-guards first).
/// `value` must already be obfuscated by the caller.
pub async fn update_fields(
    pool: &PgPool,
    id: Uuid,
    name: Option<&str>,
    enc_value: Option<&str>,
    username: Option<&str>,
    notes: Option<&str>,
    project: Option<&str>,
    tags: Option<&serde_json::Value>,
) -> Result<()> {
    sqlx::query(
        "UPDATE vault_entries SET \
           name = COALESCE($2, name), \
           value = COALESCE($3, value), \
           username = COALESCE($4, username), \
           notes = COALESCE($5, notes), \
           project = COALESCE($6, project), \
           tags = COALESCE($7, tags), \
           updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(name)
    .bind(enc_value)
    .bind(username)
    .bind(notes)
    .bind(project)
    .bind(tags)
    .execute(pool)
    .await?;
    Ok(())
}

/// Set status + bump last_validated_at (by id only; caller scope-guards first).
pub async fn set_status(pool: &PgPool, id: Uuid, status: &str) -> Result<()> {
    sqlx::query("UPDATE vault_entries SET status=$1, last_validated_at=NOW() WHERE id=$2")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Set status scoped to project_path (IDOR). Returns rows affected.
pub async fn set_status_scoped(
    pool: &PgPool,
    id: Uuid,
    status: &str,
    project_path: Option<&str>,
) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE vault_entries SET status=$1, last_validated_at=NOW() WHERE id=$2 \
         AND project_path IS NOT DISTINCT FROM $3",
    )
    .bind(status)
    .bind(id)
    .bind(project_path)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Delete a vault entry scoped to project_path (IDOR). Returns rows affected.
pub async fn delete_scoped(pool: &PgPool, id: Uuid, project_path: Option<&str>) -> Result<u64> {
    super::scoped::delete_scoped(pool, "vault_entries", id, project_path).await
}

/// Resolve a `{{vault:name}}` reference to its encrypted value, restricted to the
/// caller's project (`project_path = $2`, or `= ''` for the global namespace).
pub async fn resolve_value(
    pool: &PgPool,
    name: &str,
    project_path: Option<&str>,
) -> Result<Option<String>> {
    let row = match project_path {
        Some(pp) => sqlx::query_scalar::<_, String>(
            "SELECT value FROM vault_entries WHERE (name=$1 OR id::TEXT=$1) AND project_path = $2",
        )
        .bind(name)
        .bind(pp)
        .fetch_optional(pool)
        .await?,
        None => sqlx::query_scalar::<_, String>(
            "SELECT value FROM vault_entries WHERE (name=$1 OR id::TEXT=$1) AND project_path = ''",
        )
        .bind(name)
        .fetch_optional(pool)
        .await?,
    };
    Ok(row)
}
