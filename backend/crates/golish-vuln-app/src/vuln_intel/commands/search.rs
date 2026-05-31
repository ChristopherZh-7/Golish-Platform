//! Tauri commands: local + remote vuln-intel search.

use golish_vuln_intel::{
    self as intel, EntryRow, PgVulnIntelStore, VulnEntry, VulnIntelStore as _,
};

use golish_app_core::DbState;
use golish_app_core::GolishError;

#[tauri::command]
pub async fn intel_search(
    state: tauri::State<'_, DbState>,
    query: String,
) -> Result<Vec<VulnEntry>, GolishError> {
    let pool = state.pool_ready().await?;
    let pattern = format!("%{}%", query.to_lowercase());
    let rows: Vec<EntryRow> = sqlx::query_as(
        "SELECT cve_id, title, description, sev, cvss_score, published, source, refs, affected_products \
         FROM vuln_entries \
         WHERE LOWER(cve_id) LIKE $1 OR LOWER(title) LIKE $1 OR LOWER(description) LIKE $1 \
         ORDER BY published DESC",
    )
    .bind(&pattern)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(VulnEntry::from).collect())
}

#[tauri::command]
pub async fn intel_search_remote(
    state: tauri::State<'_, DbState>,
    query: String,
) -> Result<Vec<VulnEntry>, GolishError> {
    let pool = state.pool_ready().await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

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

    let mut entries = intel::fetch_nvd(&client, &url).await?;
    entries.sort_by(|a, b| b.published.cmp(&a.published));
    PgVulnIntelStore::new(pool).upsert_entries(&entries).await?;
    Ok(entries)
}

#[tauri::command]
pub async fn intel_search_remote_page(
    state: tauri::State<'_, DbState>,
    query: String,
    start_index: u32,
) -> Result<Vec<VulnEntry>, GolishError> {
    let pool = state.pool_ready().await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let url = format!(
        "https://services.nvd.nist.gov/rest/json/cves/2.0?keywordSearch={}&resultsPerPage=50&startIndex={}",
        url::form_urlencoded::byte_serialize(query.trim().as_bytes()).collect::<String>(),
        start_index
    );

    let mut entries = intel::fetch_nvd(&client, &url).await?;
    entries.sort_by(|a, b| b.published.cmp(&a.published));
    PgVulnIntelStore::new(pool).upsert_entries(&entries).await?;
    Ok(entries)
}
