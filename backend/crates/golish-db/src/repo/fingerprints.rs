use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::Fingerprint;

pub async fn upsert(
    pool: &PgPool,
    target_id: Uuid,
    project_path: Option<&str>,
    category: &str,
    name: &str,
    version: Option<&str>,
    confidence: f32,
    evidence: &serde_json::Value,
    cpe: Option<&str>,
    source: &str,
) -> Result<Fingerprint> {
    let row = sqlx::query_as::<_, Fingerprint>(
        r#"INSERT INTO fingerprints
               (target_id, project_path, category, name, version, confidence, evidence, cpe, source)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           ON CONFLICT (target_id, category, name) DO UPDATE SET
               version = COALESCE($5, fingerprints.version),
               confidence = GREATEST($6, fingerprints.confidence),
               evidence = fingerprints.evidence || $7,
               cpe = COALESCE($8, fingerprints.cpe),
               detected_at = NOW()
           RETURNING *"#,
    )
    .bind(target_id)
    .bind(project_path)
    .bind(category)
    .bind(name)
    .bind(version)
    .bind(confidence)
    .bind(evidence)
    .bind(cpe)
    .bind(source)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_by_target(pool: &PgPool, target_id: Uuid) -> Result<Vec<Fingerprint>> {
    let rows = sqlx::query_as::<_, Fingerprint>(
        "SELECT * FROM fingerprints WHERE target_id = $1 ORDER BY confidence DESC, detected_at DESC",
    )
    .bind(target_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_category(
    pool: &PgPool,
    target_id: Uuid,
    category: &str,
) -> Result<Vec<Fingerprint>> {
    let rows = sqlx::query_as::<_, Fingerprint>(
        "SELECT * FROM fingerprints WHERE target_id = $1 AND category = $2 ORDER BY confidence DESC",
    )
    .bind(target_id)
    .bind(category)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM fingerprints WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
