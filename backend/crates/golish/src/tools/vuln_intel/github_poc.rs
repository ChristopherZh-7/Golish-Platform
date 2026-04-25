use serde::{Serialize, Deserialize};

use crate::state::AppState;
use super::nuclei_search::{build_github_client_from_state, github_headers};

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
