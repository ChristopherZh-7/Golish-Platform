use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnFeed {
    pub id: String,
    pub name: String,
    pub feed_type: String,
    pub url: String,
    pub enabled: bool,
    pub last_fetched: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnEntry {
    pub cve_id: String,
    pub title: String,
    pub description: String,
    pub severity: String,
    pub cvss_score: Option<f64>,
    pub published: String,
    pub source: String,
    pub references: Vec<String>,
    pub affected_products: Vec<String>,
}

pub(super) fn ts_from_dt(dt: chrono::DateTime<chrono::Utc>) -> u64 {
    dt.timestamp() as u64
}

#[derive(sqlx::FromRow)]
pub(super) struct FeedRow {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) feed_type: String,
    pub(super) url: String,
    pub(super) enabled: bool,
    pub(super) last_fetched: Option<chrono::DateTime<chrono::Utc>>,
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
pub(super) struct EntryRow {
    pub(super) cve_id: String,
    pub(super) title: String,
    pub(super) description: String,
    pub(super) sev: String,
    pub(super) cvss_score: Option<f64>,
    pub(super) published: String,
    pub(super) source: String,
    pub(super) refs: serde_json::Value,
    pub(super) affected_products: serde_json::Value,
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

pub(super) fn default_feeds() -> Vec<VulnFeed> {
    vec![
        VulnFeed {
            id: "cisa-kev".to_string(),
            name: "CISA Known Exploited Vulnerabilities".to_string(),
            feed_type: "cisa_kev".to_string(),
            url: "https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json".to_string(),
            enabled: true,
            last_fetched: None,
        },
        VulnFeed {
            id: "nvd-recent".to_string(),
            name: "NVD Recent CVEs".to_string(),
            feed_type: "nvd_recent".to_string(),
            url: String::new(),
            enabled: true,
            last_fetched: None,
        },
        VulnFeed {
            id: "cnvd".to_string(),
            name: "CNVD 国家信息安全漏洞共享平台".to_string(),
            feed_type: "rss".to_string(),
            url: "https://www.cnvd.org.cn/rssXml".to_string(),
            enabled: false,
            last_fetched: None,
        },
        VulnFeed {
            id: "seebug-paper".to_string(),
            name: "Seebug Paper 安全技术精粹".to_string(),
            feed_type: "rss".to_string(),
            url: "https://paper.seebug.org/rss/".to_string(),
            enabled: true,
            last_fetched: None,
        },
    ]
}

pub(super) async fn ensure_default_feeds(pool: &sqlx::PgPool) -> Result<(), String> {
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
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) fn nvd_recent_url(days_back: i64) -> String {
    let end = Utc::now();
    let start = end - Duration::days(days_back);
    format!(
        "https://services.nvd.nist.gov/rest/json/cves/2.0?resultsPerPage=200&pubStartDate={}&pubEndDate={}",
        start.format("%Y-%m-%dT00:00:00.000"),
        end.format("%Y-%m-%dT23:59:59.999"),
    )
}

pub(super) async fn upsert_entries(pool: &sqlx::PgPool, entries: &[VulnEntry]) -> Result<(), String> {
    for e in entries {
        let refs_json = serde_json::to_value(&e.references).unwrap_or_else(|_| serde_json::json!([]));
        let products_json = serde_json::to_value(&e.affected_products).unwrap_or_else(|_| serde_json::json!([]));

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
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
