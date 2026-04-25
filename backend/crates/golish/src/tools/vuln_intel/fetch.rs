
use super::types::VulnEntry;

pub(super) fn merge_and_enrich(entries: Vec<VulnEntry>) -> Vec<VulnEntry> {
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

pub(super) async fn enrich_missing_cvss(client: &reqwest::Client, entries: &mut [VulnEntry]) {
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

pub(super) async fn fetch_cisa_kev(
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

pub(super) async fn fetch_nvd(
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

pub(super) async fn fetch_rss(
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

pub(super) fn strip_html_tags(input: &str) -> String {
    let re = regex::Regex::new(r"<[^>]*>").unwrap();
    let text = re.replace_all(input, "").to_string();
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

pub(super) fn guess_severity(text: &str) -> String {
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
