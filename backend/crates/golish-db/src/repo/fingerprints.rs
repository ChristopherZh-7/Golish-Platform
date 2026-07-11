use crate::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::Fingerprint;
use crate::repo::scoped::{lock_target_write_guard, TargetWriteGuard};

const GUARDED_UPSERT_SQL: &str = r#"INSERT INTO fingerprints
       (target_id, project_path, category, name, version, confidence, evidence, cpe, source)
   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
   ON CONFLICT (target_id, category, name) DO UPDATE SET
       version = COALESCE(EXCLUDED.version, fingerprints.version),
       confidence = GREATEST(EXCLUDED.confidence, fingerprints.confidence),
       evidence = fingerprints.evidence || EXCLUDED.evidence,
       cpe = COALESCE(EXCLUDED.cpe, fingerprints.cpe),
       detected_at = NOW()
   WHERE fingerprints.project_path IS NOT DISTINCT FROM EXCLUDED.project_path
   RETURNING *"#;

#[derive(Debug, Clone)]
pub struct FingerprintWrite {
    pub category: String,
    pub name: String,
    pub version: Option<String>,
    pub confidence: f32,
    pub evidence: serde_json::Value,
    pub cpe: Option<String>,
    pub source: String,
}

fn build_list_by_current_target_owner_sql() -> &'static str {
    r#"SELECT f.*
       FROM fingerprints f
       JOIN targets t ON t.id = f.target_id
       WHERE f.target_id = $1
         AND t.scope::text = 'in'
         AND f.project_path IS NOT DISTINCT FROM t.project_path
       ORDER BY f.confidence DESC, f.detected_at DESC"#
}

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
    let pp = project_path.unwrap_or("");
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
    .bind(pp)
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

/// Atomically publish a set of fingerprints only while the target's immutable
/// owner/scope/origin witness remains unchanged. A same-key row from an old
/// project makes the whole batch fail closed instead of being updated.
pub async fn upsert_batch_guarded(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    writes: &[FingerprintWrite],
) -> Result<Vec<Fingerprint>> {
    if writes.is_empty() {
        return Ok(Vec::new());
    }
    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, guard).await?;
    let mut rows = Vec::with_capacity(writes.len());
    for write in writes {
        let row = sqlx::query_as::<_, Fingerprint>(GUARDED_UPSERT_SQL)
            .bind(guard.target_id)
            .bind(&guard.project_path)
            .bind(&write.category)
            .bind(&write.name)
            .bind(write.version.as_deref())
            .bind(write.confidence)
            .bind(&write.evidence)
            .bind(write.cpe.as_deref())
            .bind(&write.source)
            .fetch_one(&mut *tx)
            .await?;
        rows.push(row);
    }
    tx.commit().await?;
    Ok(rows)
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

/// List only fingerprints whose stored project still matches the target's
/// current in-scope owner binding.
pub async fn list_by_current_target_owner(
    pool: &PgPool,
    target_id: Uuid,
) -> Result<Vec<Fingerprint>> {
    let rows = sqlx::query_as::<_, Fingerprint>(build_list_by_current_target_owner_sql())
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
    super::scoped::delete_by_id(pool, "fingerprints", id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_owner_reads_join_target_scope_and_project() {
        let sql = build_list_by_current_target_owner_sql();
        assert!(sql.contains("JOIN targets t ON t.id = f.target_id"));
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("f.project_path IS NOT DISTINCT FROM t.project_path"));
    }

    #[test]
    fn guarded_batch_locks_target_and_rejects_cross_project_conflicts() {
        assert!(GUARDED_UPSERT_SQL.contains("ON CONFLICT (target_id, category, name)"));
        assert!(GUARDED_UPSERT_SQL.contains(
            "WHERE fingerprints.project_path IS NOT DISTINCT FROM EXCLUDED.project_path"
        ));
    }
}
