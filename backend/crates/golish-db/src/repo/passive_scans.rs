use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::PassiveScanLog;

pub async fn insert(
    pool: &PgPool,
    target_id: Uuid,
    project_path: Option<&str>,
    test_type: &str,
    payload: &str,
    url: &str,
    parameter: &str,
    result: &str,
    evidence: &str,
    severity: &str,
    tool_used: &str,
    tester: &str,
    notes: &str,
    detail: &serde_json::Value,
) -> Result<PassiveScanLog> {
    let row = sqlx::query_as::<_, PassiveScanLog>(
        r#"INSERT INTO passive_scan_logs
               (target_id, project_path, test_type, payload, url, parameter,
                result, evidence, severity, tool_used, tester, notes, detail)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
           RETURNING *"#,
    )
    .bind(target_id)
    .bind(project_path)
    .bind(test_type)
    .bind(payload)
    .bind(url)
    .bind(parameter)
    .bind(result)
    .bind(evidence)
    .bind(severity)
    .bind(tool_used)
    .bind(tester)
    .bind(notes)
    .bind(detail)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_by_target(pool: &PgPool, target_id: Uuid, limit: i64) -> Result<Vec<PassiveScanLog>> {
    let rows = sqlx::query_as::<_, PassiveScanLog>(
        "SELECT * FROM passive_scan_logs WHERE target_id = $1 ORDER BY tested_at DESC LIMIT $2",
    )
    .bind(target_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_type(
    pool: &PgPool,
    target_id: Uuid,
    test_type: &str,
) -> Result<Vec<PassiveScanLog>> {
    let rows = sqlx::query_as::<_, PassiveScanLog>(
        "SELECT * FROM passive_scan_logs WHERE target_id = $1 AND test_type = $2 ORDER BY tested_at DESC",
    )
    .bind(target_id)
    .bind(test_type)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_url(pool: &PgPool, url: &str, limit: i64) -> Result<Vec<PassiveScanLog>> {
    let rows = sqlx::query_as::<_, PassiveScanLog>(
        "SELECT * FROM passive_scan_logs WHERE url LIKE $1 || '%' ORDER BY tested_at DESC LIMIT $2",
    )
    .bind(url)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_vulnerable(pool: &PgPool, target_id: Uuid) -> Result<Vec<PassiveScanLog>> {
    let rows = sqlx::query_as::<_, PassiveScanLog>(
        "SELECT * FROM passive_scan_logs WHERE target_id = $1 AND result IN ('vulnerable', 'potential') ORDER BY severity DESC, tested_at DESC",
    )
    .bind(target_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn stats_by_target(pool: &PgPool, target_id: Uuid) -> Result<serde_json::Value> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT result, COUNT(*) FROM passive_scan_logs WHERE target_id = $1 GROUP BY result",
    )
    .bind(target_id)
    .fetch_all(pool)
    .await?;
    let mut map = serde_json::Map::new();
    for (result, count) in rows {
        map.insert(result, serde_json::Value::from(count));
    }
    Ok(serde_json::Value::Object(map))
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM passive_scan_logs WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
