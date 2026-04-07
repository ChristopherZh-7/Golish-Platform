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
    if let Ok(data) = fs::read_to_string(feeds_path()).await {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        let mut store = FeedStore::default();
        store.feeds = default_feeds();
        store
    }
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
            feed_type: "nvd".to_string(),
            url: "https://services.nvd.nist.gov/rest/json/cves/2.0?resultsPerPage=50".to_string(),
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
                if let Ok(entries) = fetch_nvd(&client, &feed.url).await {
                    all_entries.extend(entries);
                    feed.last_fetched = Some(now_ts());
                }
            }
            _ => {}
        }
    }

    save_feeds(&feeds_store).await?;

    all_entries.sort_by(|a, b| b.published.cmp(&a.published));
    all_entries.truncate(200);

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
pub async fn intel_search(query: String) -> Result<Vec<VulnEntry>, String> {
    let cache = load_cache().await;
    let q = query.to_lowercase();
    let results: Vec<VulnEntry> = cache
        .entries
        .into_iter()
        .filter(|e| {
            e.cve_id.to_lowercase().contains(&q)
                || e.title.to_lowercase().contains(&q)
                || e.description.to_lowercase().contains(&q)
                || e.affected_products.iter().any(|p| p.to_lowercase().contains(&q))
        })
        .collect();
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
        for v in vulns.iter().take(100) {
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
        for item in vulns.iter().take(50) {
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
