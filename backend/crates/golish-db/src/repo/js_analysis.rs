use crate::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::JsAnalysisResult;
use crate::repo::scoped::{lock_target_write_guard, TargetWriteGuard};

fn is_browser_collect_placeholder(raw: &serde_json::Value) -> bool {
    raw.get("collected_by").and_then(|v| v.as_str()) == Some("browser_collect_js_api")
        && raw
            .get("analysis_pending")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
}

fn is_static_js_extract(raw: &serde_json::Value) -> bool {
    raw.get("extracted_by").and_then(|v| v.as_str()) == Some("js_extract_apis")
}

const GUARDED_SELECT_EXISTING_SQL: &str = r#"SELECT *
FROM js_analysis_results
WHERE target_id = $1
  AND filename = $2
  AND project_path IS NOT DISTINCT FROM $3
ORDER BY analyzed_at DESC
LIMIT 1"#;

const GUARDED_UPDATE_EXISTING_SQL: &str = r#"UPDATE js_analysis_results
SET project_path = $2,
    url = $3,
    size_bytes = COALESCE($4, size_bytes),
    hash_sha256 = COALESCE($5, hash_sha256),
    frameworks = $6,
    libraries = $7,
    endpoints_found = $8,
    secrets_found = $9,
    comments = $10,
    source_maps = $11,
    risk_summary = $12,
    raw_analysis = $13,
    analyzed_at = NOW()
WHERE id = $1
  AND target_id = $14
  AND project_path IS NOT DISTINCT FROM $15
RETURNING *"#;

const GUARDED_UPDATE_FILE_PATH_SQL: &str = r#"UPDATE js_analysis_results
SET file_path = $2
WHERE id = $1
  AND target_id = $3
  AND project_path IS NOT DISTINCT FROM $4
RETURNING id"#;

fn build_list_by_current_target_owner_sql() -> &'static str {
    r#"SELECT *
       FROM (
         SELECT DISTINCT ON (js.filename) js.*
         FROM js_analysis_results js
         JOIN targets t ON t.id = js.target_id
         WHERE js.target_id = $1
           AND t.scope::text = 'in'
           AND js.project_path IS NOT DISTINCT FROM t.project_path
         ORDER BY js.filename, js.analyzed_at DESC
       ) latest
       ORDER BY analyzed_at DESC"#
}

pub async fn insert(
    pool: &PgPool,
    target_id: Uuid,
    project_path: Option<&str>,
    url: &str,
    filename: &str,
    size_bytes: Option<i64>,
    hash_sha256: Option<&str>,
    frameworks: &serde_json::Value,
    libraries: &serde_json::Value,
    endpoints_found: &serde_json::Value,
    secrets_found: &serde_json::Value,
    comments: &serde_json::Value,
    source_maps: bool,
    risk_summary: &str,
    raw_analysis: &serde_json::Value,
) -> Result<JsAnalysisResult> {
    if let Some(existing) = sqlx::query_as::<_, JsAnalysisResult>(
        "SELECT * FROM js_analysis_results WHERE target_id = $1 AND filename = $2 ORDER BY analyzed_at DESC LIMIT 1",
    )
    .bind(target_id)
    .bind(filename)
    .fetch_optional(pool)
    .await?
    {
        if is_browser_collect_placeholder(raw_analysis) && is_static_js_extract(&existing.raw_analysis)
        {
            return Ok(existing);
        }

        let row = sqlx::query_as::<_, JsAnalysisResult>(
            r#"UPDATE js_analysis_results
               SET project_path = COALESCE($2, project_path),
                   url = $3,
                   size_bytes = COALESCE($4, size_bytes),
                   hash_sha256 = COALESCE($5, hash_sha256),
                   frameworks = $6,
                   libraries = $7,
                   endpoints_found = $8,
                   secrets_found = $9,
                   comments = $10,
                   source_maps = $11,
                   risk_summary = $12,
                   raw_analysis = $13,
                   analyzed_at = NOW()
               WHERE id = $1
               RETURNING *"#,
        )
        .bind(existing.id)
        .bind(project_path)
        .bind(url)
        .bind(size_bytes)
        .bind(hash_sha256)
        .bind(frameworks)
        .bind(libraries)
        .bind(endpoints_found)
        .bind(secrets_found)
        .bind(comments)
        .bind(source_maps)
        .bind(risk_summary)
        .bind(raw_analysis)
        .fetch_one(pool)
        .await?;
        return Ok(row);
    }

    let row = sqlx::query_as::<_, JsAnalysisResult>(
        r#"INSERT INTO js_analysis_results
               (target_id, project_path, url, filename, size_bytes, hash_sha256,
                frameworks, libraries, endpoints_found, secrets_found, comments,
                source_maps, risk_summary, raw_analysis)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
           RETURNING *"#,
    )
    .bind(target_id)
    .bind(project_path)
    .bind(url)
    .bind(filename)
    .bind(size_bytes)
    .bind(hash_sha256)
    .bind(frameworks)
    .bind(libraries)
    .bind(endpoints_found)
    .bind(secrets_found)
    .bind(comments)
    .bind(source_maps)
    .bind(risk_summary)
    .bind(raw_analysis)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Guarded JS-analysis upsert for active Enumeration producers.
///
/// The target row lock serializes guarded writers for one target, so the
/// existing-row lookup and update/insert decision stay inside the same short
/// transaction even though the legacy table has no `(target_id, filename)`
/// unique constraint.
#[allow(clippy::too_many_arguments)]
pub async fn insert_guarded(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    url: &str,
    filename: &str,
    size_bytes: Option<i64>,
    hash_sha256: Option<&str>,
    frameworks: &serde_json::Value,
    libraries: &serde_json::Value,
    endpoints_found: &serde_json::Value,
    secrets_found: &serde_json::Value,
    comments: &serde_json::Value,
    source_maps: bool,
    risk_summary: &str,
    raw_analysis: &serde_json::Value,
) -> Result<JsAnalysisResult> {
    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, guard).await?;

    if let Some(existing) = sqlx::query_as::<_, JsAnalysisResult>(GUARDED_SELECT_EXISTING_SQL)
        .bind(guard.target_id)
        .bind(filename)
        .bind(&guard.project_path)
        .fetch_optional(&mut *tx)
        .await?
    {
        if is_browser_collect_placeholder(raw_analysis)
            && is_static_js_extract(&existing.raw_analysis)
        {
            tx.commit().await?;
            return Ok(existing);
        }

        let row = sqlx::query_as::<_, JsAnalysisResult>(GUARDED_UPDATE_EXISTING_SQL)
            .bind(existing.id)
            .bind(&guard.project_path)
            .bind(url)
            .bind(size_bytes)
            .bind(hash_sha256)
            .bind(frameworks)
            .bind(libraries)
            .bind(endpoints_found)
            .bind(secrets_found)
            .bind(comments)
            .bind(source_maps)
            .bind(risk_summary)
            .bind(raw_analysis)
            .bind(guard.target_id)
            .bind(&guard.project_path)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(row);
    }

    let row = sqlx::query_as::<_, JsAnalysisResult>(
        r#"INSERT INTO js_analysis_results
               (target_id, project_path, url, filename, size_bytes, hash_sha256,
                frameworks, libraries, endpoints_found, secrets_found, comments,
                source_maps, risk_summary, raw_analysis)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
           RETURNING *"#,
    )
    .bind(guard.target_id)
    .bind(&guard.project_path)
    .bind(url)
    .bind(filename)
    .bind(size_bytes)
    .bind(hash_sha256)
    .bind(frameworks)
    .bind(libraries)
    .bind(endpoints_found)
    .bind(secrets_found)
    .bind(comments)
    .bind(source_maps)
    .bind(risk_summary)
    .bind(raw_analysis)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn update_file_path(pool: &PgPool, id: Uuid, file_path: &str) -> Result<()> {
    sqlx::query("UPDATE js_analysis_results SET file_path = $2 WHERE id = $1")
        .bind(id)
        .bind(file_path)
        .execute(pool)
        .await?;
    Ok(())
}

/// Guarded capture-path update. A row id from another target/project cannot be
/// redirected even if a caller accidentally carries it across iterations.
pub async fn update_file_path_guarded(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    id: Uuid,
    file_path: &str,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, guard).await?;
    let updated = sqlx::query_scalar::<_, Uuid>(GUARDED_UPDATE_FILE_PATH_SQL)
        .bind(id)
        .bind(file_path)
        .bind(guard.target_id)
        .bind(&guard.project_path)
        .fetch_optional(&mut *tx)
        .await?;
    if updated.is_none() {
        return Err(crate::DbError::NotFound(format!(
            "guarded JS capture-path update rejected for {id}"
        )));
    }
    tx.commit().await?;
    Ok(())
}

/// Update file_path for an existing JS analysis result matched by target_id + url.
pub async fn update_file_path_by_url(
    pool: &PgPool,
    target_id: Uuid,
    url: &str,
    file_path: &str,
) -> Result<u64> {
    let result = sqlx::query(
        "UPDATE js_analysis_results SET file_path = $3 WHERE target_id = $1 AND url = $2 AND file_path IS NULL",
    )
    .bind(target_id)
    .bind(url)
    .bind(file_path)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub async fn list_by_target(pool: &PgPool, target_id: Uuid) -> Result<Vec<JsAnalysisResult>> {
    let rows = sqlx::query_as::<_, JsAnalysisResult>(
        r#"SELECT *
           FROM (
             SELECT DISTINCT ON (filename) *
             FROM js_analysis_results
             WHERE target_id = $1
             ORDER BY filename, analyzed_at DESC
           ) latest
           ORDER BY analyzed_at DESC"#,
    )
    .bind(target_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// List only JS rows whose stored project still matches the target's current
/// in-scope owner binding.
pub async fn list_by_current_target_owner(
    pool: &PgPool,
    target_id: Uuid,
) -> Result<Vec<JsAnalysisResult>> {
    let rows = sqlx::query_as::<_, JsAnalysisResult>(build_list_by_current_target_owner_sql())
        .bind(target_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<JsAnalysisResult>> {
    super::scoped::get_by_id(pool, "js_analysis_results", id).await
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    super::scoped::delete_by_id(pool, "js_analysis_results", id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_js_writes_bind_target_and_project() {
        assert!(GUARDED_SELECT_EXISTING_SQL.contains("target_id = $1"));
        assert!(GUARDED_SELECT_EXISTING_SQL.contains("project_path IS NOT DISTINCT FROM $3"));
        assert!(GUARDED_UPDATE_EXISTING_SQL.contains("target_id = $14"));
        assert!(GUARDED_UPDATE_EXISTING_SQL.contains("project_path IS NOT DISTINCT FROM $15"));
        assert!(GUARDED_UPDATE_FILE_PATH_SQL.contains("target_id = $3"));
        assert!(GUARDED_UPDATE_FILE_PATH_SQL.contains("project_path IS NOT DISTINCT FROM $4"));
    }

    #[test]
    fn current_owner_reads_join_target_scope_and_project() {
        let sql = build_list_by_current_target_owner_sql();
        assert!(sql.contains("JOIN targets t ON t.id = js.target_id"));
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("js.project_path IS NOT DISTINCT FROM t.project_path"));
    }
}
