//! Vulnerability intelligence domain types — pure data structures.

use serde::{Deserialize, Serialize};

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

pub fn default_feeds() -> Vec<VulnFeed> {
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
            id: "nvd".to_string(),
            name: "NVD Vulnerabilities".to_string(),
            feed_type: "nvd".to_string(),
            url: String::new(),
            enabled: true,
            last_fetched: None,
        },
        VulnFeed {
            id: "nvd-recent".to_string(),
            name: "NVD Recent (120 days)".to_string(),
            feed_type: "nvd_recent".to_string(),
            url: String::new(),
            enabled: true,
            last_fetched: None,
        },
        VulnFeed {
            id: "seebug-paper".to_string(),
            name: "Seebug Paper — security research highlights".to_string(),
            feed_type: "rss".to_string(),
            url: "https://paper.seebug.org/rss/".to_string(),
            enabled: true,
            last_fetched: None,
        },
    ]
}

pub fn nvd_recent_url(days_back: i64) -> String {
    let end = chrono::Utc::now();
    let start = end - chrono::Duration::days(days_back);
    format!(
        "https://services.nvd.nist.gov/rest/json/cves/2.0?resultsPerPage=200&pubStartDate={}&pubEndDate={}",
        start.format("%Y-%m-%dT00:00:00.000"),
        end.format("%Y-%m-%dT23:59:59.999"),
    )
}
