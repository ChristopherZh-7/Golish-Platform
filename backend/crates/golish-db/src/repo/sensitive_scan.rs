//! `sensitive_scan_results` / `sensitive_scan_history` project-scoped repo
//! helpers (AGENTS.md I2).
//!
//! Sinks the scoped result-list and clear operations from
//! `golish::tools::sensitive_scan`. Inserts and id-keyed updates stay in the
//! command layer. The sitemap-directory read used by the scanner lives in
//! `repo::sitemap_store`.

use crate::Result;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

const SENSITIVE_RESULT_COLS: &str = "id, base_url, probe_path, full_url, status_code, content_length, content_type, is_confirmed, ai_verdict, created_at";

fn build_list_results_sql(confirmed_only: bool) -> String {
    if confirmed_only {
        format!("SELECT {SENSITIVE_RESULT_COLS} FROM sensitive_scan_results WHERE project_path = $1 AND is_confirmed = TRUE ORDER BY created_at DESC")
    } else {
        format!("SELECT {SENSITIVE_RESULT_COLS} FROM sensitive_scan_results WHERE project_path = $1 ORDER BY created_at DESC")
    }
}

fn build_list_results_unordered_sql() -> String {
    format!("SELECT {SENSITIVE_RESULT_COLS} FROM sensitive_scan_results WHERE project_path = $1")
}

fn build_clear_results_sql() -> String {
    "DELETE FROM sensitive_scan_results WHERE project_path = $1".to_string()
}

fn build_clear_history_sql() -> String {
    "DELETE FROM sensitive_scan_history WHERE project_path = $1".to_string()
}

fn build_set_verdict_by_id_scoped_sql() -> String {
    "UPDATE sensitive_scan_results SET ai_verdict = $1 WHERE id = $2 AND project_path IS NOT DISTINCT FROM $3".to_string()
}

/// List sensitive-scan results for a project, newest first; `confirmed_only`
/// adds `AND is_confirmed = TRUE`. Generic over the caller's row type.
pub async fn list_results_by_project<T>(
    pool: &PgPool,
    project_path: Option<&str>,
    confirmed_only: bool,
) -> Result<Vec<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let rows = sqlx::query_as::<_, T>(&build_list_results_sql(confirmed_only))
        .bind(project_path)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// List sensitive-scan results for a project in physical (unordered) form,
/// matching the AI verdict-application probe exactly (no `ORDER BY`, so the
/// caller's first-match semantics are preserved). Generic over the row type.
pub async fn list_results_unordered<T>(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let rows = sqlx::query_as::<_, T>(&build_list_results_unordered_sql())
        .bind(project_path)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Delete every sensitive-scan result for a project. Returns rows affected.
pub async fn clear_results(pool: &PgPool, project_path: Option<&str>) -> Result<u64> {
    let res = sqlx::query(&build_clear_results_sql())
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// Delete every sensitive-scan history row for a project. Returns rows affected.
pub async fn clear_history(pool: &PgPool, project_path: Option<&str>) -> Result<u64> {
    let res = sqlx::query(&build_clear_history_sql())
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// Set the AI verdict on a result by id, guarded by project scope (IDOR
/// defense-in-depth; the caller already loaded the row from a project-scoped
/// list, so legitimate flows are unaffected). Returns rows affected.
pub async fn set_verdict_by_id_scoped(
    pool: &PgPool,
    id: Uuid,
    verdict: &str,
    project_path: Option<&str>,
) -> Result<u64> {
    let res = sqlx::query(&build_set_verdict_by_id_scoped_sql())
        .bind(verdict)
        .bind(id)
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_scan_sql_matches_command_layer() {
        let cols = "id, base_url, probe_path, full_url, status_code, content_length, content_type, is_confirmed, ai_verdict, created_at";
        assert_eq!(
            build_list_results_sql(true),
            format!("SELECT {cols} FROM sensitive_scan_results WHERE project_path = $1 AND is_confirmed = TRUE ORDER BY created_at DESC")
        );
        assert_eq!(
            build_list_results_sql(false),
            format!("SELECT {cols} FROM sensitive_scan_results WHERE project_path = $1 ORDER BY created_at DESC")
        );
        assert_eq!(
            build_list_results_unordered_sql(),
            format!("SELECT {cols} FROM sensitive_scan_results WHERE project_path = $1")
        );
        assert_eq!(
            build_clear_results_sql(),
            "DELETE FROM sensitive_scan_results WHERE project_path = $1"
        );
        assert_eq!(
            build_clear_history_sql(),
            "DELETE FROM sensitive_scan_history WHERE project_path = $1"
        );
    }

    #[test]
    fn set_verdict_scoped_sql_has_project_guard() {
        assert_eq!(
            build_set_verdict_by_id_scoped_sql(),
            "UPDATE sensitive_scan_results SET ai_verdict = $1 WHERE id = $2 AND project_path IS NOT DISTINCT FROM $3"
        );
    }
}
