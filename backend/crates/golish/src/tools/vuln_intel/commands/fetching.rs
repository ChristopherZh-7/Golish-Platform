//! Tauri commands: NVD/CISA/RSS feed ingestion + cached entries.

use chrono::{Duration, Utc};

use golish_vuln_intel::{
    self as intel, EntryRow, FeedRow, PgVulnIntelStore, VulnEntry, VulnIntelStore as _,
};

use crate::error::GolishError;
use crate::settings::SettingsManager;
use crate::state::DbState;

#[tauri::command]
pub async fn intel_fetch(
    state: tauri::State<'_, DbState>,
    settings_mgr: tauri::State<'_, std::sync::Arc<SettingsManager>>,
) -> Result<Vec<VulnEntry>, GolishError> {
    let pool = state.pool_ready().await?;
    let store = PgVulnIntelStore::new(pool);
    store.ensure_default_feeds().await?;

    let feeds: Vec<FeedRow> = sqlx::query_as(
        "SELECT id, name, feed_type, url, enabled, last_fetched FROM vuln_feeds WHERE enabled = true",
    )
    .fetch_all(pool)
    .await?;

    let settings = settings_mgr.get().await;
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
    let client = client_builder.build()?;

    let mut all_entries: Vec<VulnEntry> = Vec::new();

    for feed in &feeds {
        let result = match feed.feed_type.as_str() {
            "cisa_kev" => intel::fetch_cisa_kev(&client, &feed.url).await,
            "nvd" => {
                let url = if feed.url.is_empty() {
                    intel::nvd_recent_url(120)
                } else {
                    feed.url.clone()
                };
                intel::fetch_nvd(&client, &url).await
            }
            "nvd_recent" => intel::fetch_nvd(&client, &intel::nvd_recent_url(120)).await,
            "rss" => intel::fetch_rss(&client, &feed.url, &feed.name).await,
            _ => continue,
        };

        match result {
            Ok(entries) => {
                tracing::info!(feed = %feed.name, count = entries.len(), "[intel-fetch] Feed fetched");
                all_entries.extend(entries);
                sqlx::query("UPDATE vuln_feeds SET last_fetched=NOW() WHERE id=$1")
                    .bind(&feed.id)
                    .execute(pool)
                    .await?;
            }
            Err(e) => {
                tracing::warn!(feed = %feed.name, error = %e, "[intel-fetch] Feed fetch failed");
            }
        }
    }

    all_entries = intel::merge_and_enrich(all_entries);
    intel::enrich_missing_cvss(&client, &mut all_entries).await;
    all_entries.sort_by(|a, b| b.published.cmp(&a.published));

    store.upsert_entries(&all_entries).await?;

    Ok(all_entries)
}

#[tauri::command]
pub async fn intel_get_cached(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<VulnEntry>, GolishError> {
    let pool = state.pool_ready().await?;
    let rows: Vec<EntryRow> = sqlx::query_as(
        "SELECT cve_id, title, description, sev, cvss_score, published, source, refs, affected_products \
         FROM vuln_entries ORDER BY published DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(VulnEntry::from).collect())
}

#[tauri::command]
pub async fn intel_fetch_page(
    state: tauri::State<'_, DbState>,
    page: u32,
) -> Result<Vec<VulnEntry>, GolishError> {
    let pool = state.pool_ready().await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

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

    let new_entries = intel::fetch_nvd(&client, &url).await?;
    PgVulnIntelStore::new(pool)
        .upsert_entries(&new_entries)
        .await?;

    let rows: Vec<EntryRow> = sqlx::query_as(
        "SELECT cve_id, title, description, sev, cvss_score, published, source, refs, affected_products \
         FROM vuln_entries ORDER BY published DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(VulnEntry::from).collect())
}
