use chrono::{Duration, Utc};

use crate::state::AppState;
use super::types::{VulnFeed, VulnEntry, FeedRow, EntryRow, ensure_default_feeds, nvd_recent_url, upsert_entries};
use super::fetch::{merge_and_enrich, enrich_missing_cvss, fetch_cisa_kev, fetch_nvd, fetch_rss};

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

    let settings = state.settings_manager.get().await;
    let mut client_builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("Mozilla/5.0 (compatible; Golish/1.0)");
    if let Some(ref proxy_url) = settings.network.proxy_url {
        if !proxy_url.is_empty() {
            tracing::info!(proxy = %proxy_url, "[intel-fetch] Using proxy");
            if let Ok(proxy) = reqwest::Proxy::all(proxy_url.as_str()) {
                client_builder = client_builder.proxy(proxy);
            }
        }
    }
    let client = client_builder.build().map_err(|e| e.to_string())?;

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

        match result {
            Ok(entries) => {
                tracing::info!(feed = %feed.name, count = entries.len(), "[intel-fetch] Feed fetched");
                all_entries.extend(entries);
                sqlx::query("UPDATE vuln_feeds SET last_fetched=NOW() WHERE id=$1")
                    .bind(&feed.id)
                    .execute(pool)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            Err(e) => {
                tracing::warn!(feed = %feed.name, error = %e, "[intel-fetch] Feed fetch failed");
            }
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
