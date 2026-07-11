use crate::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::TargetAsset;

fn build_list_by_current_target_owner_sql() -> &'static str {
    r#"SELECT ta.*
       FROM target_assets ta
       JOIN targets t ON t.id = ta.target_id
       WHERE ta.target_id = $1
         AND t.scope::text = 'in'
         AND ta.project_path IS NOT DISTINCT FROM t.project_path
       ORDER BY ta.discovered_at DESC"#
}

fn build_count_by_current_target_owner_sql() -> &'static str {
    r#"SELECT COUNT(*)
       FROM target_assets ta
       JOIN targets t ON t.id = ta.target_id
       WHERE ta.target_id = $1
         AND t.scope::text = 'in'
         AND ta.project_path IS NOT DISTINCT FROM t.project_path"#
}

pub async fn upsert(
    pool: &PgPool,
    target_id: Uuid,
    project_path: Option<&str>,
    asset_type: &str,
    value: &str,
    port: Option<i32>,
    protocol: Option<&str>,
    service: Option<&str>,
    version: Option<&str>,
    metadata: &serde_json::Value,
) -> Result<TargetAsset> {
    let row = sqlx::query_as::<_, TargetAsset>(
        r#"INSERT INTO target_assets
               (target_id, project_path, asset_type, value, port, protocol, service, version, metadata)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           ON CONFLICT (target_id, asset_type, value) DO UPDATE SET
               port = COALESCE($5, target_assets.port),
               protocol = COALESCE($6, target_assets.protocol),
               service = COALESCE($7, target_assets.service),
               version = COALESCE($8, target_assets.version),
               metadata = target_assets.metadata || $9,
               updated_at = NOW()
           RETURNING *"#,
    )
    .bind(target_id)
    .bind(project_path)
    .bind(asset_type)
    .bind(value)
    .bind(port)
    .bind(protocol)
    .bind(service)
    .bind(version)
    .bind(metadata)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_by_target(pool: &PgPool, target_id: Uuid) -> Result<Vec<TargetAsset>> {
    let rows = sqlx::query_as::<_, TargetAsset>(
        "SELECT * FROM target_assets WHERE target_id = $1 ORDER BY discovered_at DESC",
    )
    .bind(target_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// List only rows that still belong to the target's current in-scope project.
/// A target moved to another workspace cannot carry its old child rows with it.
pub async fn list_by_current_target_owner(
    pool: &PgPool,
    target_id: Uuid,
) -> Result<Vec<TargetAsset>> {
    let rows = sqlx::query_as::<_, TargetAsset>(build_list_by_current_target_owner_sql())
        .bind(target_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn list_by_type(
    pool: &PgPool,
    target_id: Uuid,
    asset_type: &str,
) -> Result<Vec<TargetAsset>> {
    let rows = sqlx::query_as::<_, TargetAsset>(
        "SELECT * FROM target_assets WHERE target_id = $1 AND asset_type = $2 ORDER BY discovered_at DESC",
    )
    .bind(target_id)
    .bind(asset_type)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn count_by_target(pool: &PgPool, target_id: Uuid) -> Result<i64> {
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM target_assets WHERE target_id = $1")
            .bind(target_id)
            .fetch_one(pool)
            .await?;
    Ok(count)
}

pub async fn count_by_current_target_owner(pool: &PgPool, target_id: Uuid) -> Result<i64> {
    let (count,): (i64,) = sqlx::query_as(build_count_by_current_target_owner_sql())
        .bind(target_id)
        .fetch_one(pool)
        .await?;
    Ok(count)
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    super::scoped::delete_by_id(pool, "target_assets", id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_owner_reads_join_target_scope_and_project() {
        for sql in [
            build_list_by_current_target_owner_sql(),
            build_count_by_current_target_owner_sql(),
        ] {
            assert!(sql.contains("JOIN targets t ON t.id = ta.target_id"));
            assert!(sql.contains("t.scope::text = 'in'"));
            assert!(sql.contains("ta.project_path IS NOT DISTINCT FROM t.project_path"));
        }
    }
}
