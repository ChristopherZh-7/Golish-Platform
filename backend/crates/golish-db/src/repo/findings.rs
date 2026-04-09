use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{Finding, FindingStatus, Severity};

pub async fn create(pool: &PgPool, title: &str, sev: Severity, project_path: Option<&str>, source: &str) -> Result<Finding> {
    let row = sqlx::query_as::<_, Finding>(
        r#"INSERT INTO findings (title, sev, project_path, source)
           VALUES ($1, $2, $3, $4)
           RETURNING *"#,
    )
    .bind(title)
    .bind(sev)
    .bind(project_path)
    .bind(source)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<Finding>> {
    let rows = sqlx::query_as::<_, Finding>(
        "SELECT * FROM findings WHERE project_path IS NOT DISTINCT FROM $1 ORDER BY created_at DESC",
    )
    .bind(project_path)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Finding>> {
    let row = sqlx::query_as::<_, Finding>("SELECT * FROM findings WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn update_full(pool: &PgPool, f: &Finding) -> Result<()> {
    sqlx::query(
        r#"UPDATE findings SET title=$1, sev=$2, cvss=$3, url=$4, target=$5,
           description=$6, steps=$7, remediation=$8, tags=$9, tool=$10,
           template=$11, refs=$12, evidence=$13, status=$14, updated_at=NOW()
           WHERE id=$15"#,
    )
    .bind(&f.title).bind(f.sev).bind(f.cvss).bind(&f.url).bind(&f.target)
    .bind(&f.description).bind(&f.steps).bind(&f.remediation)
    .bind(&f.tags).bind(&f.tool).bind(&f.template)
    .bind(&f.refs).bind(&f.evidence).bind(f.status).bind(f.id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_status(pool: &PgPool, id: Uuid, status: FindingStatus) -> Result<()> {
    sqlx::query("UPDATE findings SET status = $1, updated_at = NOW() WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM findings WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
