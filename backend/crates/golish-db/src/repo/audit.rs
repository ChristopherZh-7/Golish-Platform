use anyhow::Result;
use sqlx::PgPool;

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

pub async fn clear(pool: &PgPool, project_path: Option<&str>) -> Result<u64> {
    let result = sqlx::query("DELETE FROM audit_log WHERE project_path IS NOT DISTINCT FROM $1")
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
