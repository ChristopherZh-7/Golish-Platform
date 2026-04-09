use anyhow::Result;
use sqlx::PgPool;

use crate::models::{VulnEntry, VulnFeed};

pub async fn upsert_feed(pool: &PgPool, feed: &VulnFeed) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO vuln_feeds (id, name, feed_type, url, enabled, last_fetched)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (id) DO UPDATE SET
               name = $2, feed_type = $3, url = $4, enabled = $5, last_fetched = $6"#,
    )
    .bind(&feed.id)
    .bind(&feed.name)
    .bind(&feed.feed_type)
    .bind(&feed.url)
    .bind(feed.enabled)
    .bind(feed.last_fetched)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_feeds(pool: &PgPool) -> Result<Vec<VulnFeed>> {
    let rows = sqlx::query_as::<_, VulnFeed>("SELECT * FROM vuln_feeds ORDER BY name")
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn delete_feed(pool: &PgPool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM vuln_feeds WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn insert_entries(pool: &PgPool, entries: &[VulnEntry]) -> Result<u64> {
    let mut count = 0u64;
    for e in entries {
        let result = sqlx::query(
            r#"INSERT INTO vuln_entries (cve_id, title, description, sev, cvss_score, published, source, refs, affected_products)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(&e.cve_id)
        .bind(&e.title)
        .bind(&e.description)
        .bind(&e.sev)
        .bind(e.cvss_score)
        .bind(&e.published)
        .bind(&e.source)
        .bind(&e.refs)
        .bind(&e.affected_products)
        .execute(pool)
        .await?;
        count += result.rows_affected();
    }
    Ok(count)
}

pub async fn search_entries(pool: &PgPool, query: &str, limit: i64) -> Result<Vec<VulnEntry>> {
    let pattern = format!("%{}%", query);
    let rows = sqlx::query_as::<_, VulnEntry>(
        r#"SELECT * FROM vuln_entries
           WHERE cve_id ILIKE $1 OR title ILIKE $1 OR description ILIKE $1
           ORDER BY fetched_at DESC LIMIT $2"#,
    )
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_recent(pool: &PgPool, limit: i64) -> Result<Vec<VulnEntry>> {
    let rows = sqlx::query_as::<_, VulnEntry>(
        "SELECT * FROM vuln_entries ORDER BY fetched_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
