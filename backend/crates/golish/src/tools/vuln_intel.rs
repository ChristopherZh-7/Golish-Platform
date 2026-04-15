use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;

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

fn ts_from_dt(dt: chrono::DateTime<chrono::Utc>) -> u64 {
    dt.timestamp() as u64
}

#[derive(sqlx::FromRow)]
struct FeedRow {
    id: String,
    name: String,
    feed_type: String,
    url: String,
    enabled: bool,
    last_fetched: Option<chrono::DateTime<chrono::Utc>>,
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
struct EntryRow {
    cve_id: String,
    title: String,
    description: String,
    sev: String,
    cvss_score: Option<f64>,
    published: String,
    source: String,
    refs: serde_json::Value,
    affected_products: serde_json::Value,
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

fn default_feeds() -> Vec<VulnFeed> {
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
            enabled: true,
            last_fetched: None,
        },
    ]
}

async fn ensure_default_feeds(pool: &sqlx::PgPool) -> Result<(), String> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM vuln_feeds")
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;

    if count == 0 {
        for feed in default_feeds() {
            sqlx::query(
                "INSERT INTO vuln_feeds (id, name, feed_type, url, enabled) VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
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
    }
    Ok(())
}

fn nvd_recent_url(days_back: i64) -> String {
    let end = Utc::now();
    let start = end - Duration::days(days_back);
    format!(
        "https://services.nvd.nist.gov/rest/json/cves/2.0?resultsPerPage=200&pubStartDate={}&pubEndDate={}",
        start.format("%Y-%m-%dT00:00:00.000"),
        end.format("%Y-%m-%dT23:59:59.999"),
    )
}

async fn upsert_entries(pool: &sqlx::PgPool, entries: &[VulnEntry]) -> Result<(), String> {
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

#[tauri::command]
pub async fn intel_list_feeds(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<VulnFeed>, String> {
    let pool = state.db_pool_ready().await?;
    ensure_default_feeds(pool).await?;
    let rows: Vec<FeedRow> = sqlx::query_as("SELECT id, name, feed_type, url, enabled, last_fetched FROM vuln_feeds")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(VulnFeed::from).collect())
}

#[tauri::command]
pub async fn intel_add_feed(
    state: tauri::State<'_, AppState>,
    name: String,
    feed_type: String,
    url: String,
) -> Result<String, String> {
    let pool = state.db_pool_ready().await?;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO vuln_feeds (id, name, feed_type, url, enabled) VALUES ($1, $2, $3, $4, true)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&feed_type)
    .bind(&url)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn intel_toggle_feed(
    state: tauri::State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    sqlx::query("UPDATE vuln_feeds SET enabled=$1 WHERE id=$2")
        .bind(enabled)
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn intel_delete_feed(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    sqlx::query("DELETE FROM vuln_feeds WHERE id=$1")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn intel_fetch(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<VulnEntry>, String> {
    let pool = state.db_pool_ready().await?;
    ensure_default_feeds(pool).await?;

    let feeds: Vec<FeedRow> = sqlx::query_as(
        "SELECT id, name, feed_type, url, enabled, last_fetched FROM vuln_feeds WHERE enabled = true",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let mut all_entries: Vec<VulnEntry> = Vec::new();

    for feed in &feeds {
        let result = match feed.feed_type.as_str() {
            "cisa_kev" => fetch_cisa_kev(&client, &feed.url).await,
            "nvd" => {
                let url = if feed.url.is_empty() { nvd_recent_url(120) } else { feed.url.clone() };
                fetch_nvd(&client, &url).await
            }
            "nvd_recent" => fetch_nvd(&client, &nvd_recent_url(120)).await,
            "rss" => fetch_rss(&client, &feed.url, &feed.name).await,
            _ => continue,
        };

        if let Ok(entries) = result {
            all_entries.extend(entries);
            sqlx::query("UPDATE vuln_feeds SET last_fetched=NOW() WHERE id=$1")
                .bind(&feed.id)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    all_entries = merge_and_enrich(all_entries);
    enrich_missing_cvss(&client, &mut all_entries).await;
    all_entries.sort_by(|a, b| b.published.cmp(&a.published));

    upsert_entries(pool, &all_entries).await?;

    Ok(all_entries)
}

#[tauri::command]
pub async fn intel_get_cached(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<VulnEntry>, String> {
    let pool = state.db_pool_ready().await?;
    let rows: Vec<EntryRow> = sqlx::query_as(
        "SELECT cve_id, title, description, sev, cvss_score, published, source, refs, affected_products \
         FROM vuln_entries ORDER BY published DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(VulnEntry::from).collect())
}

#[tauri::command]
pub async fn intel_fetch_page(
    state: tauri::State<'_, AppState>,
    page: u32,
) -> Result<Vec<VulnEntry>, String> {
    let pool = state.db_pool_ready().await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let days_back = 120 + (page as i64 * 120);
    let days_start = days_back;
    let days_end = days_back - 120;
    let end = Utc::now() - Duration::days(days_end);
    let start = Utc::now() - Duration::days(days_start);
    let url = format!(
        "https://services.nvd.nist.gov/rest/json/cves/2.0?resultsPerPage=200&pubStartDate={}&pubEndDate={}",
        start.format("%Y-%m-%dT00:00:00.000"),
        end.format("%Y-%m-%dT23:59:59.999"),
    );

    let new_entries = fetch_nvd(&client, &url).await?;
    upsert_entries(pool, &new_entries).await?;

    let rows: Vec<EntryRow> = sqlx::query_as(
        "SELECT cve_id, title, description, sev, cvss_score, published, source, refs, affected_products \
         FROM vuln_entries ORDER BY published DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(VulnEntry::from).collect())
}

#[tauri::command]
pub async fn intel_search_remote(
    state: tauri::State<'_, AppState>,
    query: String,
) -> Result<Vec<VulnEntry>, String> {
    let pool = state.db_pool_ready().await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let cve_pattern = regex::Regex::new(r"(?i)^CVE-\d{4}-\d{4,}$").unwrap();
    let is_cve = cve_pattern.is_match(query.trim());

    let url = if is_cve {
        format!(
            "https://services.nvd.nist.gov/rest/json/cves/2.0?cveId={}",
            query.trim().to_uppercase()
        )
    } else {
        format!(
            "https://services.nvd.nist.gov/rest/json/cves/2.0?keywordSearch={}&resultsPerPage=50",
            url::form_urlencoded::byte_serialize(query.trim().as_bytes()).collect::<String>()
        )
    };

    let mut entries = fetch_nvd(&client, &url).await?;
    entries.sort_by(|a, b| b.published.cmp(&a.published));

    upsert_entries(pool, &entries).await?;

    Ok(entries)
}

#[tauri::command]
pub async fn intel_search_remote_page(
    state: tauri::State<'_, AppState>,
    query: String,
    start_index: u32,
) -> Result<Vec<VulnEntry>, String> {
    let pool = state.db_pool_ready().await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!(
        "https://services.nvd.nist.gov/rest/json/cves/2.0?keywordSearch={}&resultsPerPage=50&startIndex={}",
        url::form_urlencoded::byte_serialize(query.trim().as_bytes()).collect::<String>(),
        start_index
    );

    let mut entries = fetch_nvd(&client, &url).await?;
    entries.sort_by(|a, b| b.published.cmp(&a.published));

    upsert_entries(pool, &entries).await?;

    Ok(entries)
}

#[tauri::command]
pub async fn intel_search(
    state: tauri::State<'_, AppState>,
    query: String,
) -> Result<Vec<VulnEntry>, String> {
    let pool = state.db_pool_ready().await?;
    let pattern = format!("%{}%", query.to_lowercase());
    let rows: Vec<EntryRow> = sqlx::query_as(
        "SELECT cve_id, title, description, sev, cvss_score, published, source, refs, affected_products \
         FROM vuln_entries \
         WHERE LOWER(cve_id) LIKE $1 OR LOWER(title) LIKE $1 OR LOWER(description) LIKE $1 \
         ORDER BY published DESC",
    )
    .bind(&pattern)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(VulnEntry::from).collect())
}

#[tauri::command]
pub async fn intel_match_targets(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
) -> Result<Vec<VulnEntry>, String> {
    let pool = state.db_pool_ready().await?;
    let _ = project_path;

    let target_rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT name, tags FROM targets",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut keywords = Vec::new();
    for (name, tags) in &target_rows {
        let lower = name.to_lowercase();
        if lower.len() >= 3 {
            keywords.push(lower);
        }
        if let Some(arr) = tags.as_array() {
            for tag in arr {
                if let Some(s) = tag.as_str() {
                    let lower = s.to_lowercase();
                    if lower.len() >= 3 {
                        keywords.push(lower);
                    }
                }
            }
        }
    }
    keywords.sort();
    keywords.dedup();

    if keywords.is_empty() {
        return Ok(vec![]);
    }

    let rows: Vec<EntryRow> = sqlx::query_as(
        "SELECT cve_id, title, description, sev, cvss_score, published, source, refs, affected_products \
         FROM vuln_entries ORDER BY published DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let entries: Vec<VulnEntry> = rows.into_iter().map(VulnEntry::from).collect();
    let matched: Vec<VulnEntry> = entries
        .into_iter()
        .filter(|entry| {
            let text = format!(
                "{} {} {}",
                entry.title,
                entry.description,
                entry.affected_products.join(" ")
            )
            .to_lowercase();
            keywords.iter().any(|kw| text.contains(kw))
        })
        .collect();

    Ok(matched)
}

fn merge_and_enrich(entries: Vec<VulnEntry>) -> Vec<VulnEntry> {
    let mut map = std::collections::HashMap::<String, VulnEntry>::new();
    for entry in entries {
        let key = entry.cve_id.clone();
        if let Some(existing) = map.get(&key) {
            if existing.cvss_score.is_none() && entry.cvss_score.is_some() {
                map.insert(key, entry);
            } else if existing.cvss_score.is_some() && entry.cvss_score.is_none() {
                // keep existing (has score)
            } else {
                let mut merged = existing.clone();
                if merged.description.len() < entry.description.len() {
                    merged.description = entry.description.clone();
                    merged.title = entry.title.clone();
                }
                for r in &entry.references {
                    if !merged.references.contains(r) {
                        merged.references.push(r.clone());
                    }
                }
                for p in &entry.affected_products {
                    if !merged.affected_products.contains(p) {
                        merged.affected_products.push(p.clone());
                    }
                }
                merged.source = format!("{} + {}", merged.source, entry.source);
                map.insert(key, merged);
            }
        } else {
            map.insert(key, entry);
        }
    }
    map.into_values().collect()
}

async fn enrich_missing_cvss(client: &reqwest::Client, entries: &mut [VulnEntry]) {
    let missing: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.cvss_score.is_none())
        .map(|(i, _)| i)
        .take(20)
        .collect();

    for (batch_idx, idx) in missing.iter().enumerate() {
        if batch_idx > 0 && batch_idx % 5 == 0 {
            tokio::time::sleep(std::time::Duration::from_secs(6)).await;
        }
        let cve_id = &entries[*idx].cve_id;
        let url = format!(
            "https://services.nvd.nist.gov/rest/json/cves/2.0?cveId={}",
            cve_id
        );
        if let Ok(resp) = client.get(&url).header("Accept", "application/json").send().await {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(vulns) = body.get("vulnerabilities").and_then(|v| v.as_array()) {
                    if let Some(item) = vulns.first() {
                        if let Some(cve) = item.get("cve") {
                            if let Some(metrics) = cve.get("metrics") {
                                for key in ["cvssMetricV31", "cvssMetricV30", "cvssMetricV2"] {
                                    if let Some(arr) = metrics.get(key).and_then(|m| m.as_array()) {
                                        if let Some(first) = arr.first() {
                                            if let Some(data) = first.get("cvssData") {
                                                if let Some(score) = data
                                                    .get("baseScore")
                                                    .and_then(|s| s.as_f64())
                                                {
                                                    entries[*idx].cvss_score = Some(score);
                                                    entries[*idx].severity = if score >= 9.0 {
                                                        "critical"
                                                    } else if score >= 7.0 {
                                                        "high"
                                                    } else if score >= 4.0 {
                                                        "medium"
                                                    } else {
                                                        "low"
                                                    }
                                                    .to_string();
                                                }
                                            }
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn fetch_cisa_kev(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<VulnEntry>, String> {
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    let mut entries = Vec::new();
    if let Some(vulns) = body.get("vulnerabilities").and_then(|v| v.as_array()) {
        for v in vulns.iter() {
            let cve_id = v.get("cveID").and_then(|s| s.as_str()).unwrap_or("").to_string();
            let title = v.get("vulnerabilityName").and_then(|s| s.as_str()).unwrap_or("").to_string();
            let description = v.get("shortDescription").and_then(|s| s.as_str()).unwrap_or("").to_string();
            let published = v.get("dateAdded").and_then(|s| s.as_str()).unwrap_or("").to_string();
            let vendor = v.get("vendorProject").and_then(|s| s.as_str()).unwrap_or("").to_string();
            let product = v.get("product").and_then(|s| s.as_str()).unwrap_or("").to_string();

            entries.push(VulnEntry {
                cve_id,
                title,
                description,
                severity: "high".to_string(),
                cvss_score: None,
                published,
                source: "CISA KEV".to_string(),
                references: vec![],
                affected_products: if !vendor.is_empty() {
                    vec![format!("{vendor} {product}")]
                } else {
                    vec![]
                },
            });
        }
    }
    Ok(entries)
}

async fn fetch_nvd(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<VulnEntry>, String> {
    let resp = client
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    let mut entries = Vec::new();
    if let Some(vulns) = body.get("vulnerabilities").and_then(|v| v.as_array()) {
        for item in vulns.iter() {
            let cve = match item.get("cve") {
                Some(c) => c,
                None => continue,
            };
            let cve_id = cve.get("id").and_then(|s| s.as_str()).unwrap_or("").to_string();

            let descriptions = cve.get("descriptions").and_then(|d| d.as_array()).cloned().unwrap_or_default();
            let description = descriptions
                .iter()
                .find(|d| d.get("lang").and_then(|l| l.as_str()) == Some("en"))
                .and_then(|d| d.get("value").and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_string();

            let title = if description.len() > 100 {
                format!("{}...", &description[..100])
            } else {
                description.clone()
            };

            let published = cve.get("published").and_then(|s| s.as_str()).unwrap_or("").to_string();

            let mut severity = "info".to_string();
            let mut cvss_score: Option<f64> = None;
            if let Some(metrics) = cve.get("metrics") {
                for key in ["cvssMetricV31", "cvssMetricV30", "cvssMetricV2"] {
                    if let Some(arr) = metrics.get(key).and_then(|m| m.as_array()) {
                        if let Some(first) = arr.first() {
                            if let Some(data) = first.get("cvssData") {
                                if let Some(score) = data.get("baseScore").and_then(|s| s.as_f64()) {
                                    cvss_score = Some(score);
                                    severity = if score >= 9.0 {
                                        "critical"
                                    } else if score >= 7.0 {
                                        "high"
                                    } else if score >= 4.0 {
                                        "medium"
                                    } else {
                                        "low"
                                    }
                                    .to_string();
                                }
                            }
                        }
                        break;
                    }
                }
            }

            let mut refs = Vec::new();
            if let Some(ref_arr) = cve.get("references").and_then(|r| r.as_array()) {
                for r in ref_arr.iter().take(3) {
                    if let Some(url) = r.get("url").and_then(|u| u.as_str()) {
                        refs.push(url.to_string());
                    }
                }
            }

            entries.push(VulnEntry {
                cve_id,
                title,
                description,
                severity,
                cvss_score,
                published,
                source: "NVD".to_string(),
                references: refs,
                affected_products: vec![],
            });
        }
    }
    Ok(entries)
}

async fn fetch_rss(
    client: &reqwest::Client,
    url: &str,
    source_name: &str,
) -> Result<Vec<VulnEntry>, String> {
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    let xml_text = resp.text().await.map_err(|e| e.to_string())?;

    let mut reader = quick_xml::Reader::from_str(&xml_text);
    reader.config_mut().trim_text(true);

    let mut entries = Vec::new();
    let mut in_item = false;
    let mut current_tag = String::new();
    let mut title = String::new();
    let mut link = String::new();
    let mut description = String::new();
    let mut pub_date = String::new();

    let cve_re = regex::Regex::new(r"(?i)(CVE-\d{4}-\d{4,})|(CNVD-\d{4}-\d+)|(CNNVD-\d{6}-\d+)")
        .unwrap();

    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if tag == "item" || tag == "entry" {
                    in_item = true;
                    title.clear();
                    link.clear();
                    description.clear();
                    pub_date.clear();
                }
                if in_item {
                    current_tag = tag;
                }
            }
            Ok(quick_xml::events::Event::Text(ref e)) => {
                if in_item {
                    let text = e.unescape().unwrap_or_default().to_string();
                    match current_tag.as_str() {
                        "title" => title.push_str(&text),
                        "link" => link.push_str(&text),
                        "description" | "summary" | "content" => description.push_str(&text),
                        "pubDate" | "published" | "updated" | "dc:date" => pub_date.push_str(&text),
                        _ => {}
                    }
                }
            }
            Ok(quick_xml::events::Event::End(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if (tag == "item" || tag == "entry") && in_item {
                    in_item = false;

                    let combined = format!("{} {}", title, description);
                    let cve_id = cve_re
                        .find(&combined)
                        .map(|m| m.as_str().to_uppercase())
                        .unwrap_or_else(|| {
                            format!("RSS-{:x}", {
                                use std::hash::{Hash, Hasher};
                                let mut h = std::collections::hash_map::DefaultHasher::new();
                                title.hash(&mut h);
                                link.hash(&mut h);
                                h.finish()
                            })
                        });

                    let severity = guess_severity(&combined);

                    entries.push(VulnEntry {
                        cve_id,
                        title: title.clone(),
                        description: strip_html_tags(&description),
                        severity,
                        cvss_score: None,
                        published: pub_date.clone(),
                        source: source_name.to_string(),
                        references: if link.is_empty() { vec![] } else { vec![link.clone()] },
                        affected_products: vec![],
                    });
                }
                current_tag.clear();
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    Ok(entries)
}

fn strip_html_tags(input: &str) -> String {
    let re = regex::Regex::new(r"<[^>]*>").unwrap();
    let text = re.replace_all(input, "").to_string();
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn guess_severity(text: &str) -> String {
    let lower = text.to_lowercase();
    if lower.contains("critical") || lower.contains("严重") || lower.contains("超危") {
        "critical".to_string()
    } else if lower.contains("high") || lower.contains("高危") || lower.contains("高风险") {
        "high".to_string()
    } else if lower.contains("medium") || lower.contains("中危") || lower.contains("中风险") {
        "medium".to_string()
    } else if lower.contains("low") || lower.contains("低危") || lower.contains("低风险") {
        "low".to_string()
    } else {
        "info".to_string()
    }
}

// ─── GitHub PoC Search ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubPocResult {
    pub full_name: String,
    pub html_url: String,
    pub description: Option<String>,
    pub language: Option<String>,
    pub stars: u32,
    pub updated_at: String,
    pub topics: Vec<String>,
}

#[derive(Deserialize)]
struct GhSearchResponse {
    items: Vec<GhRepoItem>,
}

#[derive(Deserialize)]
struct GhRepoItem {
    full_name: String,
    html_url: String,
    description: Option<String>,
    language: Option<String>,
    stargazers_count: u32,
    updated_at: String,
    #[serde(default)]
    topics: Vec<String>,
}

#[tauri::command]
pub async fn intel_search_github_poc(
    state: tauri::State<'_, AppState>,
    cve_id: String,
) -> Result<Vec<GithubPocResult>, String> {
    let (client, token) = build_github_client_from_state(&state).await?;
    let headers = github_headers(&token);

    let query = url::form_urlencoded::byte_serialize(cve_id.as_bytes()).collect::<String>();
    let url = format!(
        "https://api.github.com/search/repositories?q={}&sort=stars&order=desc&per_page=20",
        query
    );

    let resp = client
        .get(&url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub API error {}: {}", status, body));
    }

    let data: GhSearchResponse = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;

    Ok(data.items.into_iter().map(|item| GithubPocResult {
        full_name: item.full_name,
        html_url: item.html_url,
        description: item.description,
        language: item.language,
        stars: item.stargazers_count,
        updated_at: item.updated_at,
        topics: item.topics,
    }).collect())
}

// ─── Nuclei Template Search ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NucleiTemplateResult {
    pub name: String,
    pub path: String,
    pub html_url: String,
    pub content: Option<String>,
    pub severity: Option<String>,
}

#[derive(Deserialize)]
struct GhCodeSearchResponse {
    items: Vec<GhCodeItem>,
}

#[derive(Deserialize)]
struct GhCodeItem {
    name: String,
    path: String,
    html_url: String,
    repository: GhCodeRepo,
}

#[derive(Deserialize)]
struct GhCodeRepo {
    full_name: String,
}

async fn build_github_client_from_state(
    state: &tauri::State<'_, AppState>,
) -> Result<(reqwest::Client, Option<String>), String> {
    let settings = state.settings_manager.get().await;
    let github_token = settings
        .api_keys
        .github
        .clone()
        .or_else(|| settings.network.github_token.clone());

    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20));
    if let Some(ref proxy_url) = settings.network.proxy_url {
        if !proxy_url.is_empty() {
            tracing::info!(proxy = %proxy_url, "[github-client] Using proxy");
            if let Ok(proxy) = reqwest::Proxy::all(proxy_url.as_str()) {
                builder = builder.proxy(proxy);
            }
        }
    }
    Ok((builder.build().map_err(|e| e.to_string())?, github_token))
}

fn github_headers(token: &Option<String>) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("User-Agent", "golish-platform".parse().unwrap());
    headers.insert("Accept", "application/vnd.github+json".parse().unwrap());
    if let Some(t) = token {
        if let Ok(val) = format!("Bearer {}", t).parse() {
            headers.insert("Authorization", val);
        }
    }
    headers
}

fn extract_nuclei_severity(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("severity:") {
            return Some(trimmed.trim_start_matches("severity:").trim().to_string());
        }
    }
    None
}

#[tauri::command]
pub async fn intel_search_nuclei_templates(
    state: tauri::State<'_, AppState>,
    cve_id: String,
) -> Result<Vec<NucleiTemplateResult>, String> {
    tracing::info!(cve_id = %cve_id, "[nuclei-search] Starting single CVE search");
    let (client, token) = build_github_client_from_state(&state).await?;
    let has_token = token.is_some();
    tracing::info!(has_token = has_token, "[nuclei-search] Client built");
    let headers = github_headers(&token);

    let query = url::form_urlencoded::byte_serialize(
        format!("{} repo:projectdiscovery/nuclei-templates extension:yaml", cve_id).as_bytes(),
    )
    .collect::<String>();
    let url = format!(
        "https://api.github.com/search/code?q={}&per_page=20",
        query
    );

    let resp = client
        .get(&url)
        .headers(headers.clone())
        .send()
        .await
        .map_err(|e| format!("GitHub code search failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub API error {}: {}", status, body));
    }

    let data: GhCodeSearchResponse =
        resp.json().await.map_err(|e| format!("Parse error: {}", e))?;

    tracing::info!(items = data.items.len(), "[nuclei-search] GitHub returned results");

    let mut results: Vec<NucleiTemplateResult> = Vec::new();

    for item in data.items {
        if item.repository.full_name != "projectdiscovery/nuclei-templates" {
            continue;
        }

        let raw_url = format!(
            "https://raw.githubusercontent.com/projectdiscovery/nuclei-templates/main/{}",
            item.path
        );
        let content = client
            .get(&raw_url)
            .headers(headers.clone())
            .send()
            .await
            .ok()
            .and_then(|r| {
                if r.status().is_success() {
                    Some(r)
                } else {
                    None
                }
            });

        let content_text = match content {
            Some(r) => r.text().await.ok(),
            None => None,
        };

        let severity = content_text.as_deref().and_then(extract_nuclei_severity);

        results.push(NucleiTemplateResult {
            name: item.name,
            path: item.path,
            html_url: item.html_url,
            content: content_text,
            severity,
        });
    }

    Ok(results)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchNucleiResult {
    pub cve_id: String,
    pub templates: Vec<NucleiTemplateResult>,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn intel_batch_search_nuclei_templates(
    state: tauri::State<'_, AppState>,
    cve_ids: Vec<String>,
) -> Result<Vec<BatchNucleiResult>, String> {
    tracing::info!(count = cve_ids.len(), "[nuclei-batch] Starting batch search");
    let (client, token) = build_github_client_from_state(&state).await?;
    let headers = github_headers(&token);
    let mut results = Vec::new();

    for (i, cve_id) in cve_ids.iter().enumerate() {
        if i > 0 && i % 8 == 0 {
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        }

        let query = url::form_urlencoded::byte_serialize(
            format!(
                "{} repo:projectdiscovery/nuclei-templates extension:yaml",
                cve_id
            )
            .as_bytes(),
        )
        .collect::<String>();
        let url = format!(
            "https://api.github.com/search/code?q={}&per_page=5",
            query
        );

        tracing::info!(cve_id = %cve_id, "[nuclei-batch] Sending request");
        let resp = client.get(&url).headers(headers.clone()).send().await;

        match resp {
            Ok(r) => {
                let status = r.status();
                tracing::info!(cve_id = %cve_id, status = %status, "[nuclei-batch] Got HTTP response");
                if status.is_success() {
                    match r.json::<GhCodeSearchResponse>().await {
                        Ok(d) => {
                            tracing::info!(cve_id = %cve_id, items = d.items.len(), "[nuclei-batch] Parsed results");
                            let mut templates = Vec::new();
                            for item in d.items {
                                if item.repository.full_name != "projectdiscovery/nuclei-templates"
                                {
                                    continue;
                                }
                                let raw_url = format!(
                                    "https://raw.githubusercontent.com/projectdiscovery/nuclei-templates/main/{}",
                                    item.path
                                );
                                let content_text = match client
                                    .get(&raw_url)
                                    .headers(headers.clone())
                                    .send()
                                    .await
                                {
                                    Ok(cr) if cr.status().is_success() => cr.text().await.ok(),
                                    _ => None,
                                };

                                let severity =
                                    content_text.as_deref().and_then(extract_nuclei_severity);

                                templates.push(NucleiTemplateResult {
                                    name: item.name,
                                    path: item.path,
                                    html_url: item.html_url,
                                    content: content_text,
                                    severity,
                                });
                            }
                            results.push(BatchNucleiResult {
                                cve_id: cve_id.clone(),
                                templates,
                                error: None,
                            });
                        }
                        Err(e) => {
                            tracing::warn!(cve_id = %cve_id, error = %e, "[nuclei-batch] JSON parse failed");
                            results.push(BatchNucleiResult {
                                cve_id: cve_id.clone(),
                                templates: vec![],
                                error: Some(format!("Parse error: {}", e)),
                            });
                        }
                    }
                } else {
                    if status.as_u16() == 403 {
                        tracing::warn!(cve_id = %cve_id, "[nuclei-batch] Rate limit hit, pausing 60s");
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    }
                    let body = r.text().await.unwrap_or_default();
                    tracing::warn!(cve_id = %cve_id, status = %status, body_len = body.len(), "[nuclei-batch] HTTP error");
                    results.push(BatchNucleiResult {
                        cve_id: cve_id.clone(),
                        templates: vec![],
                        error: Some(format!("HTTP {}: {}", status, &body[..body.len().min(200)])),
                    });
                }
            }
            Err(e) => {
                tracing::error!(cve_id = %cve_id, error = %e, "[nuclei-batch] Request failed");
                results.push(BatchNucleiResult {
                    cve_id: cve_id.clone(),
                    templates: vec![],
                    error: Some(e.to_string()),
                });
            }
        }
    }

    Ok(results)
}
