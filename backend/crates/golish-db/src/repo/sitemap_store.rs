//! `sitemap_store` project-scoped repo helpers (AGENTS.md I2).
//!
//! Three command-layer call sites (`pipeline::storage`, `sensitive_scan`,
//! `pentest_bridge::js_collect`) read and delete the `'zap-sitemap'` blob with
//! identical scoped SQL; this centralises that read/delete. The companion
//! insert stays in the command layer (it is not a scope guard).

use anyhow::Result;
use sqlx::PgPool;

fn build_read_zap_sitemap_sql() -> String {
    "SELECT data FROM sitemap_store WHERE name = 'zap-sitemap' AND project_path = $1".to_string()
}

fn build_delete_zap_sitemap_sql() -> String {
    "DELETE FROM sitemap_store WHERE name = 'zap-sitemap' AND project_path = $1".to_string()
}

/// Read the `'zap-sitemap'` data blob for a project. `None` == no stored
/// sitemap. Callers decide how to swallow errors (`.ok().flatten()` /
/// `.unwrap_or(None)`), so this surfaces the `Result` rather than defaulting.
pub async fn read_zap_sitemap(
    pool: &PgPool,
    project_path: Option<&str>,
) -> Result<Option<serde_json::Value>> {
    let data = sqlx::query_scalar::<_, serde_json::Value>(&build_read_zap_sitemap_sql())
        .bind(project_path)
        .fetch_optional(pool)
        .await?;
    Ok(data)
}

/// Delete the `'zap-sitemap'` blob for a project. Returns rows affected.
pub async fn delete_zap_sitemap(pool: &PgPool, project_path: Option<&str>) -> Result<u64> {
    let res = sqlx::query(&build_delete_zap_sitemap_sql())
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sitemap_store_sql_matches_command_layer() {
        assert_eq!(
            build_read_zap_sitemap_sql(),
            "SELECT data FROM sitemap_store WHERE name = 'zap-sitemap' AND project_path = $1"
        );
        assert_eq!(
            build_delete_zap_sitemap_sql(),
            "DELETE FROM sitemap_store WHERE name = 'zap-sitemap' AND project_path = $1"
        );
    }
}
