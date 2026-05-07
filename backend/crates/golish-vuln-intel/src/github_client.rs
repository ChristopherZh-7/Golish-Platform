//! Shared GitHub API client builder and header factory.
//!
//! Extracted from the application layer so every GitHub-calling function in
//! this crate receives a pre-built `(reqwest::Client, Option<String>)` pair
//! instead of reaching into Tauri state.

use reqwest::header::HeaderMap;

/// Build a `reqwest::Client` with optional proxy support for GitHub API calls.
pub fn build_github_client(proxy_url: Option<&str>) -> crate::VulnIntelResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20));
    if let Some(proxy_url) = proxy_url {
        if !proxy_url.is_empty() {
            tracing::info!(proxy = %proxy_url, "[github-client] Using proxy");
            if let Ok(proxy) = reqwest::Proxy::all(proxy_url) {
                builder = builder.proxy(proxy);
            }
        }
    }
    Ok(builder.build()?)
}

/// Build standard GitHub API headers, optionally including a Bearer token.
pub fn github_headers(token: &Option<String>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("User-Agent", "golish-platform".parse().unwrap());
    headers.insert("Accept", "application/vnd.github+json".parse().unwrap());
    if let Some(t) = token {
        if let Ok(val) = format!("Bearer {}", t).parse() {
            headers.insert("Authorization", val);
        }
    }
    headers
}
