//! Vuln-intel data types, DB row mappings, and storage helpers.

use uuid::Uuid;

pub use golish_vuln_intel_domain::{default_feeds, nvd_recent_url, VulnEntry, VulnFeed};

pub fn ts_from_dt(dt: chrono::DateTime<chrono::Utc>) -> u64 {
    dt.timestamp() as u64
}

#[derive(sqlx::FromRow)]
pub struct FeedRow {
    pub id: String,
    pub name: String,
    pub feed_type: String,
    pub url: String,
    pub enabled: bool,
    pub last_fetched: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<FeedRow> for VulnFeed {
    fn from(r: FeedRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            feed_type: r.feed_type,
            url: r.url,
            enabled: r.enabled,
            last_fetched: r.last_fetched.map(ts_from_dt),
        }
    }
}

#[derive(sqlx::FromRow)]
pub struct EntryRow {
    pub cve_id: String,
    pub title: String,
    pub description: String,
    pub sev: String,
    pub cvss_score: Option<f64>,
    pub published: String,
    pub source: String,
    pub refs: serde_json::Value,
    pub affected_products: serde_json::Value,
}

impl From<EntryRow> for VulnEntry {
    fn from(r: EntryRow) -> Self {
        Self {
            cve_id: r.cve_id,
            title: r.title,
            description: r.description,
            severity: r.sev,
            cvss_score: r.cvss_score,
            published: r.published,
            source: r.source,
            references: serde_json::from_value(r.refs).unwrap_or_default(),
            affected_products: serde_json::from_value(r.affected_products).unwrap_or_default(),
        }
    }
}

pub(crate) async fn ensure_default_feeds(pool: &sqlx::PgPool) -> crate::VulnIntelResult<()> {
    for feed in default_feeds() {
        sqlx::query(
            "INSERT INTO vuln_feeds (id, name, feed_type, url, enabled) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (id) DO NOTHING",
        )
        .bind(&feed.id)
        .bind(&feed.name)
        .bind(&feed.feed_type)
        .bind(&feed.url)
        .bind(feed.enabled)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub(crate) async fn upsert_entries(
    pool: &sqlx::PgPool,
    entries: &[VulnEntry],
) -> crate::VulnIntelResult<()> {
    for e in entries {
        let refs_json =
            serde_json::to_value(&e.references).unwrap_or_else(|_| serde_json::json!([]));
        let products_json =
            serde_json::to_value(&e.affected_products).unwrap_or_else(|_| serde_json::json!([]));

        sqlx::query(
            r#"INSERT INTO vuln_entries (id, cve_id, title, description, sev, cvss_score, published, source, refs, affected_products)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (cve_id) DO UPDATE SET
                 title = CASE WHEN LENGTH($3) > LENGTH(vuln_entries.title) THEN $3 ELSE vuln_entries.title END,
                 description = CASE WHEN LENGTH($4) > LENGTH(vuln_entries.description) THEN $4 ELSE vuln_entries.description END,
                 sev = CASE WHEN vuln_entries.cvss_score IS NULL AND $6 IS NOT NULL THEN $5 ELSE vuln_entries.sev END,
                 cvss_score = COALESCE($6, vuln_entries.cvss_score),
                 source = CASE WHEN vuln_entries.source NOT LIKE '%' || $8 || '%' THEN vuln_entries.source || ' + ' || $8 ELSE vuln_entries.source END,
                 refs = vuln_entries.refs || $9,
                 affected_products = vuln_entries.affected_products || $10,
                 fetched_at = NOW()"#,
        )
        .bind(Uuid::new_v4())
        .bind(&e.cve_id)
        .bind(&e.title)
        .bind(&e.description)
        .bind(&e.severity)
        .bind(e.cvss_score)
        .bind(&e.published)
        .bind(&e.source)
        .bind(&refs_json)
        .bind(&products_json)
        .execute(pool)
        .await?;
    }
    Ok(())
}
