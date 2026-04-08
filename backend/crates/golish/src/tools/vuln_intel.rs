use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;

fn intel_base() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    #[cfg(target_os = "macos")]
    let base = home
        .join("Library")
        .join("Application Support")
        .join("golish-platform")
        .join("vuln_intel");
    #[cfg(target_os = "windows")]
    let base = home
        .join("AppData")
        .join("Local")
        .join("golish-platform")
        .join("vuln_intel");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = home.join(".golish-platform").join("vuln_intel");
    base
}

fn feeds_path() -> PathBuf {
    intel_base().join("feeds.json")
}

fn cache_path() -> PathBuf {
    intel_base().join("cache.json")
}

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct FeedStore {
    feeds: Vec<VulnFeed>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CacheStore {
    entries: Vec<VulnEntry>,
    last_updated: Option<u64>,
}

async fn load_feeds() -> FeedStore {
    let mut store: FeedStore = if let Ok(data) = fs::read_to_string(feeds_path()).await {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        let mut s = FeedStore::default();
        s.feeds = default_feeds();
        return s;
    };

    let defaults = default_feeds();
    for d in defaults {
        if !store.feeds.iter().any(|f| f.id == d.id) {
            store.feeds.push(d);
        }
    }
    store
}

async fn save_feeds(store: &FeedStore) -> Result<(), String> {
    fs::create_dir_all(intel_base()).await.map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(feeds_path(), json).await.map_err(|e| e.to_string())
}

async fn load_cache() -> CacheStore {
    if let Ok(data) = fs::read_to_string(cache_path()).await {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        CacheStore::default()
    }
}

async fn save_cache(store: &CacheStore) -> Result<(), String> {
    fs::create_dir_all(intel_base()).await.map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(cache_path(), json).await.map_err(|e| e.to_string())
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

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

#[tauri::command]
pub async fn intel_list_feeds() -> Result<Vec<VulnFeed>, String> {
    Ok(load_feeds().await.feeds)
}

#[tauri::command]
pub async fn intel_add_feed(
    name: String,
    feed_type: String,
    url: String,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let feed = VulnFeed {
        id: id.clone(),
        name,
        feed_type,
        url,
        enabled: true,
        last_fetched: None,
    };
    let mut store = load_feeds().await;
    store.feeds.push(feed);
    save_feeds(&store).await?;
    Ok(id)
}

#[tauri::command]
pub async fn intel_toggle_feed(id: String, enabled: bool) -> Result<(), String> {
    let mut store = load_feeds().await;
    if let Some(f) = store.feeds.iter_mut().find(|f| f.id == id) {
        f.enabled = enabled;
    }
    save_feeds(&store).await
}

#[tauri::command]
pub async fn intel_delete_feed(id: String) -> Result<(), String> {
    let mut store = load_feeds().await;
    store.feeds.retain(|f| f.id != id);
    save_feeds(&store).await
}

#[tauri::command]
pub async fn intel_fetch() -> Result<Vec<VulnEntry>, String> {
    let mut feeds_store = load_feeds().await;
    let mut all_entries: Vec<VulnEntry> = Vec::new();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    for feed in feeds_store.feeds.iter_mut() {
        if !feed.enabled {
            continue;
        }

        match feed.feed_type.as_str() {
            "cisa_kev" => {
                if let Ok(entries) = fetch_cisa_kev(&client, &feed.url).await {
                    all_entries.extend(entries);
                    feed.last_fetched = Some(now_ts());
                }
            }
            "nvd" => {
                let url = if feed.url.is_empty() {
                    nvd_recent_url(120)
                } else {
                    feed.url.clone()
                };
                if let Ok(entries) = fetch_nvd(&client, &url).await {
                    all_entries.extend(entries);
                    feed.last_fetched = Some(now_ts());
                }
            }
            "nvd_recent" => {
                let url = nvd_recent_url(120);
                if let Ok(entries) = fetch_nvd(&client, &url).await {
                    all_entries.extend(entries);
                    feed.last_fetched = Some(now_ts());
                }
            }
            "rss" => {
                if let Ok(entries) = fetch_rss(&client, &feed.url, &feed.name).await {
                    all_entries.extend(entries);
                    feed.last_fetched = Some(now_ts());
                }
            }
            _ => {}
        }
    }

    save_feeds(&feeds_store).await?;

    all_entries = merge_and_enrich(all_entries);

    enrich_missing_cvss(&client, &mut all_entries).await;

    all_entries.sort_by(|a, b| b.published.cmp(&a.published));

    let cache = CacheStore {
        entries: all_entries.clone(),
        last_updated: Some(now_ts()),
    };
    save_cache(&cache).await?;

    Ok(all_entries)
}

#[tauri::command]
pub async fn intel_get_cached() -> Result<Vec<VulnEntry>, String> {
    Ok(load_cache().await.entries)
}

#[tauri::command]
pub async fn intel_fetch_page(page: u32) -> Result<Vec<VulnEntry>, String> {
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

    let mut new_entries = fetch_nvd(&client, &url).await?;

    let mut cache = load_cache().await;
    cache.entries.append(&mut new_entries);
    cache.entries = merge_and_enrich(cache.entries);
    cache.entries.sort_by(|a, b| b.published.cmp(&a.published));
    cache.last_updated = Some(now_ts());
    save_cache(&cache).await?;

    Ok(cache.entries)
}

#[tauri::command]
pub async fn intel_search_remote(query: String) -> Result<Vec<VulnEntry>, String> {
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

    let mut cache = load_cache().await;
    for entry in &entries {
        if !cache.entries.iter().any(|e| e.cve_id == entry.cve_id) {
            cache.entries.push(entry.clone());
        }
    }
    cache.entries.sort_by(|a, b| b.published.cmp(&a.published));
    save_cache(&cache).await?;

    Ok(entries)
}

#[tauri::command]
pub async fn intel_search_remote_page(
    query: String,
    start_index: u32,
) -> Result<Vec<VulnEntry>, String> {
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

    let mut cache = load_cache().await;
    for entry in &entries {
        if !cache.entries.iter().any(|e| e.cve_id == entry.cve_id) {
            cache.entries.push(entry.clone());
        }
    }
    save_cache(&cache).await?;

    Ok(entries)
}

#[tauri::command]
pub async fn intel_search(query: String) -> Result<Vec<VulnEntry>, String> {
    let cache = load_cache().await;
    let q = query.to_lowercase();
    let mut results: Vec<VulnEntry> = cache
        .entries
        .into_iter()
        .filter(|e| {
            e.cve_id.to_lowercase().contains(&q)
                || e.title.to_lowercase().contains(&q)
                || e.description.to_lowercase().contains(&q)
                || e.affected_products.iter().any(|p| p.to_lowercase().contains(&q))
        })
        .collect();
    results.sort_by(|a, b| b.published.cmp(&a.published));
    Ok(results)
}

#[tauri::command]
pub async fn intel_match_targets(project_path: Option<String>) -> Result<Vec<VulnEntry>, String> {
    let cache = load_cache().await;
    if cache.entries.is_empty() {
        return Ok(vec![]);
    }

    let targets_path = if let Some(pp) = &project_path {
        PathBuf::from(pp).join(".golish").join("targets").join("targets.json")
    } else {
        return Ok(vec![]);
    };

    let target_data = fs::read_to_string(&targets_path)
        .await
        .unwrap_or_default();

    let keywords: Vec<String> = extract_target_keywords(&target_data);
    if keywords.is_empty() {
        return Ok(vec![]);
    }

    let matched: Vec<VulnEntry> = cache
        .entries
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

fn extract_target_keywords(target_json: &str) -> Vec<String> {
    let mut keywords = Vec::new();
    let parsed: serde_json::Value = serde_json::from_str(target_json).unwrap_or_default();

    if let Some(targets) = parsed.get("targets").and_then(|t| t.as_array()) {
        for t in targets {
            if let Some(name) = t.get("name").and_then(|n| n.as_str()) {
                let lower = name.to_lowercase();
                if lower.len() >= 3 {
                    keywords.push(lower);
                }
            }
            if let Some(tags) = t.get("tags").and_then(|t| t.as_array()) {
                for tag in tags {
                    if let Some(s) = tag.as_str() {
                        let lower = s.to_lowercase();
                        if lower.len() >= 3 {
                            keywords.push(lower);
                        }
                    }
                }
            }
        }
    }
    keywords.sort();
    keywords.dedup();
    keywords
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
                                    if let Some(arr) = metrics.get(key).and_then(|m| m.as_array())
                                    {
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
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    let mut entries = Vec::new();
    if let Some(vulns) = body.get("vulnerabilities").and_then(|v| v.as_array()) {
        for v in vulns.iter() {
            let cve_id = v
                .get("cveID")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let title = v
                .get("vulnerabilityName")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let description = v
                .get("shortDescription")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let published = v
                .get("dateAdded")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let vendor = v
                .get("vendorProject")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let product = v
                .get("product")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();

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
            let cve_id = cve
                .get("id")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();

            let descriptions = cve
                .get("descriptions")
                .and_then(|d| d.as_array())
                .cloned()
                .unwrap_or_default();
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

            let published = cve
                .get("published")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();

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
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?;

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
                        "pubDate" | "published" | "updated" | "dc:date" => {
                            pub_date.push_str(&text)
                        }
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
                        references: if link.is_empty() {
                            vec![]
                        } else {
                            vec![link.clone()]
                        },
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
