use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::VulnScanHistory;

pub async fn add_scan(
    pool: &PgPool,
    cve_id: &str,
    target: &str,
    result: &str,
    details: Option<&str>,
) -> Result<VulnScanHistory> {
    let row = sqlx::query_as::<_, VulnScanHistory>(
        r#"INSERT INTO vuln_scan_history (cve_id, target, result, details)
           VALUES ($1, $2, $3, $4)
           RETURNING *"#,
    )
    .bind(cve_id)
    .bind(target)
    .bind(result)
    .bind(details)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn get_scans_for_cve(pool: &PgPool, cve_id: &str) -> Result<Vec<VulnScanHistory>> {
    let rows = sqlx::query_as::<_, VulnScanHistory>(
        "SELECT * FROM vuln_scan_history WHERE cve_id = $1 ORDER BY scanned_at DESC",
    )
    .bind(cve_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn delete_scan(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM vuln_scan_history WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
