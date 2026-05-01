//! GitHub PoC repository search.

use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

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

/// Search GitHub for PoC repositories matching a CVE ID.
pub async fn search_github_poc(
    client: &reqwest::Client,
    headers: &HeaderMap,
    cve_id: &str,
) -> crate::VulnIntelResult<Vec<GithubPocResult>> {
    use crate::error::VulnIntelError;

    let query = url::form_urlencoded::byte_serialize(cve_id.as_bytes()).collect::<String>();
    let url = format!(
        "https://api.github.com/search/repositories?q={}&sort=stars&order=desc&per_page=20",
        query
    );

    let resp = client
        .get(&url)
        .headers(headers.clone())
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(VulnIntelError::Github(format!(
            "GitHub API error {}: {}",
            status, body
        )));
    }

    let data: GhSearchResponse = resp.json().await?;

    Ok(data
        .items
        .into_iter()
        .map(|item| GithubPocResult {
            full_name: item.full_name,
            html_url: item.html_url,
            description: item.description,
            language: item.language,
            stars: item.stargazers_count,
            updated_at: item.updated_at,
            topics: item.topics,
        })
        .collect())
}
