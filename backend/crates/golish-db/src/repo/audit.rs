use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::AuditEntry;

pub async fn log(
    pool: &PgPool,
    action: &str,
    category: &str,
    details: &str,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
    project_path: Option<&str>,
    source: &str,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO audit_log (action, category, details, entity_type, entity_id, project_path, source)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(action)
    .bind(category)
    .bind(details)
    .bind(entity_type)
    .bind(entity_id)
    .bind(project_path)
    .bind(source)
    .execute(pool)
    .await?;
    Ok(())
}

/// Extended log with pentest operation fields
pub async fn log_operation(
    pool: &PgPool,
    action: &str,
    category: &str,
    details: &str,
    project_path: Option<&str>,
    source: &str,
    target_id: Option<Uuid>,
    session_id: Option<&str>,
    tool_name: Option<&str>,
    status: &str,
    detail: &serde_json::Value,
) -> Result<AuditEntry> {
    let row = sqlx::query_as::<_, AuditEntry>(
        r#"INSERT INTO audit_log
               (action, category, details, project_path, source,
                target_id, session_id, tool_name, status, detail)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING *"#,
    )
    .bind(action)
    .bind(category)
    .bind(details)
    .bind(project_path)
    .bind(source)
    .bind(target_id)
    .bind(session_id)
    .bind(tool_name)
    .bind(status)
    .bind(detail)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list(pool: &PgPool, project_path: Option<&str>, limit: i64) -> Result<Vec<AuditEntry>> {
    let rows = sqlx::query_as::<_, AuditEntry>(
        r#"SELECT * FROM audit_log
           WHERE project_path IS NOT DISTINCT FROM $1
           ORDER BY created_at DESC LIMIT $2"#,
    )
    .bind(project_path)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_category(
    pool: &PgPool,
    category: &str,
    project_path: Option<&str>,
    limit: i64,
) -> Result<Vec<AuditEntry>> {
    let rows = sqlx::query_as::<_, AuditEntry>(
        r#"SELECT * FROM audit_log
           WHERE category = $1 AND project_path IS NOT DISTINCT FROM $2
           ORDER BY created_at DESC LIMIT $3"#,
    )
    .bind(category)
    .bind(project_path)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_target(pool: &PgPool, target_id: Uuid, limit: i64) -> Result<Vec<AuditEntry>> {
    let rows = sqlx::query_as::<_, AuditEntry>(
        r#"SELECT * FROM audit_log
           WHERE target_id = $1
           ORDER BY created_at DESC LIMIT $2"#,
    )
    .bind(target_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_session(pool: &PgPool, session_id: &str, limit: i64) -> Result<Vec<AuditEntry>> {
    let rows = sqlx::query_as::<_, AuditEntry>(
        r#"SELECT * FROM audit_log
           WHERE session_id = $1
           ORDER BY created_at DESC LIMIT $2"#,
    )
    .bind(session_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn search(
    pool: &PgPool,
    project_path: Option<&str>,
    query: &str,
    limit: i64,
) -> Result<Vec<AuditEntry>> {
    let pattern = format!("%{}%", query.to_lowercase());
    let rows = sqlx::query_as::<_, AuditEntry>(
        r#"SELECT * FROM audit_log
           WHERE project_path IS NOT DISTINCT FROM $1
             AND (LOWER(action) LIKE $2 OR LOWER(details) LIKE $2
                  OR LOWER(category) LIKE $2 OR LOWER(COALESCE(tool_name, '')) LIKE $2)
           ORDER BY created_at DESC LIMIT $3"#,
    )
    .bind(project_path)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn count(pool: &PgPool, project_path: Option<&str>) -> Result<i64> {
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM audit_log WHERE project_path IS NOT DISTINCT FROM $1")
            .bind(project_path)
            .fetch_one(pool)
            .await?;
    Ok(count)
}

pub async fn clear(pool: &PgPool, project_path: Option<&str>) -> Result<u64> {
    let result = sqlx::query("DELETE FROM audit_log WHERE project_path IS NOT DISTINCT FROM $1")
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
