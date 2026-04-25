use serde::{Serialize, Deserialize};

use crate::state::AppState;

// ─── Nuclei Template Search ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NucleiTemplateResult {
    pub name: String,
    pub path: String,
    pub html_url: String,
    pub content: Option<String>,
    pub severity: Option<String>,
}

#[derive(Deserialize)]
struct GhCodeSearchResponse {
    items: Vec<GhCodeItem>,
}

#[derive(Deserialize)]
struct GhCodeItem {
    name: String,
    path: String,
    html_url: String,
    repository: GhCodeRepo,
}

#[derive(Deserialize)]
struct GhCodeRepo {
    full_name: String,
}

pub(super) async fn build_github_client_from_state(
    state: &tauri::State<'_, AppState>,
) -> Result<(reqwest::Client, Option<String>), String> {
    let settings = state.settings_manager.get().await;
    let github_token = settings
        .api_keys
        .github
        .clone()
        .or_else(|| settings.network.github_token.clone());

    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20));
    if let Some(ref proxy_url) = settings.network.proxy_url {
        if !proxy_url.is_empty() {
            tracing::info!(proxy = %proxy_url, "[github-client] Using proxy");
            if let Ok(proxy) = reqwest::Proxy::all(proxy_url.as_str()) {
                builder = builder.proxy(proxy);
            }
        }
    }
    Ok((builder.build().map_err(|e| e.to_string())?, github_token))
}

pub(super) fn github_headers(token: &Option<String>) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("User-Agent", "golish-platform".parse().unwrap());
    headers.insert("Accept", "application/vnd.github+json".parse().unwrap());
    if let Some(t) = token {
        if let Ok(val) = format!("Bearer {}", t).parse() {
            headers.insert("Authorization", val);
        }
    }
    headers
}

pub(super) fn extract_nuclei_severity(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("severity:") {
            return Some(trimmed.trim_start_matches("severity:").trim().to_string());
        }
    }
    None
}

#[tauri::command]
pub async fn intel_search_nuclei_templates(
    state: tauri::State<'_, AppState>,
    cve_id: String,
) -> Result<Vec<NucleiTemplateResult>, String> {
    tracing::info!(cve_id = %cve_id, "[nuclei-search] Starting single CVE search");
    let (client, token) = build_github_client_from_state(&state).await?;
    let has_token = token.is_some();
    tracing::info!(has_token = has_token, "[nuclei-search] Client built");
    let headers = github_headers(&token);

    let query = url::form_urlencoded::byte_serialize(
        format!("{} repo:projectdiscovery/nuclei-templates extension:yaml", cve_id).as_bytes(),
    )
    .collect::<String>();
    let url = format!(
        "https://api.github.com/search/code?q={}&per_page=20",
        query
    );

    let resp = client
        .get(&url)
        .headers(headers.clone())
        .send()
        .await
        .map_err(|e| format!("GitHub code search failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub API error {}: {}", status, body));
    }

    let data: GhCodeSearchResponse =
        resp.json().await.map_err(|e| format!("Parse error: {}", e))?;

    tracing::info!(items = data.items.len(), "[nuclei-search] GitHub returned results");

    let mut results: Vec<NucleiTemplateResult> = Vec::new();

    for item in data.items {
        if item.repository.full_name != "projectdiscovery/nuclei-templates" {
            continue;
        }

        let raw_url = format!(
            "https://raw.githubusercontent.com/projectdiscovery/nuclei-templates/main/{}",
            item.path
        );
        let content = client
            .get(&raw_url)
            .headers(headers.clone())
            .send()
            .await
            .ok()
            .and_then(|r| {
                if r.status().is_success() {
                    Some(r)
                } else {
                    None
                }
            });

        let content_text = match content {
            Some(r) => r.text().await.ok(),
            None => None,
        };

        let severity = content_text.as_deref().and_then(extract_nuclei_severity);

        results.push(NucleiTemplateResult {
            name: item.name,
            path: item.path,
            html_url: item.html_url,
            content: content_text,
            severity,
        });
    }

    Ok(results)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchNucleiResult {
    pub cve_id: String,
    pub templates: Vec<NucleiTemplateResult>,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn intel_batch_search_nuclei_templates(
    state: tauri::State<'_, AppState>,
    cve_ids: Vec<String>,
) -> Result<Vec<BatchNucleiResult>, String> {
    tracing::info!(count = cve_ids.len(), "[nuclei-batch] Starting batch search");
    let (client, token) = build_github_client_from_state(&state).await?;
    let headers = github_headers(&token);
    let mut results = Vec::new();

    for (i, cve_id) in cve_ids.iter().enumerate() {
        if i > 0 && i % 8 == 0 {
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        }

        let query = url::form_urlencoded::byte_serialize(
            format!(
                "{} repo:projectdiscovery/nuclei-templates extension:yaml",
                cve_id
            )
            .as_bytes(),
        )
        .collect::<String>();
        let url = format!(
            "https://api.github.com/search/code?q={}&per_page=5",
            query
        );

        tracing::info!(cve_id = %cve_id, "[nuclei-batch] Sending request");
        let resp = client.get(&url).headers(headers.clone()).send().await;

        match resp {
            Ok(r) => {
                let status = r.status();
                tracing::info!(cve_id = %cve_id, status = %status, "[nuclei-batch] Got HTTP response");
                if status.is_success() {
                    match r.json::<GhCodeSearchResponse>().await {
                        Ok(d) => {
                            tracing::info!(cve_id = %cve_id, items = d.items.len(), "[nuclei-batch] Parsed results");
                            let mut templates = Vec::new();
                            for item in d.items {
                                if item.repository.full_name != "projectdiscovery/nuclei-templates"
                                {
                                    continue;
                                }
                                let raw_url = format!(
                                    "https://raw.githubusercontent.com/projectdiscovery/nuclei-templates/main/{}",
                                    item.path
                                );
                                let content_text = match client
                                    .get(&raw_url)
                                    .headers(headers.clone())
                                    .send()
                                    .await
                                {
                                    Ok(cr) if cr.status().is_success() => cr.text().await.ok(),
                                    _ => None,
                                };

                                let severity =
                                    content_text.as_deref().and_then(extract_nuclei_severity);

                                templates.push(NucleiTemplateResult {
                                    name: item.name,
                                    path: item.path,
                                    html_url: item.html_url,
                                    content: content_text,
                                    severity,
                                });
                            }
                            results.push(BatchNucleiResult {
                                cve_id: cve_id.clone(),
                                templates,
                                error: None,
                            });
                        }
                        Err(e) => {
                            tracing::warn!(cve_id = %cve_id, error = %e, "[nuclei-batch] JSON parse failed");
                            results.push(BatchNucleiResult {
                                cve_id: cve_id.clone(),
                                templates: vec![],
                                error: Some(format!("Parse error: {}", e)),
                            });
                        }
                    }
                } else {
                    if status.as_u16() == 403 {
                        tracing::warn!(cve_id = %cve_id, "[nuclei-batch] Rate limit hit, pausing 60s");
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    }
                    let body = r.text().await.unwrap_or_default();
                    tracing::warn!(cve_id = %cve_id, status = %status, body_len = body.len(), "[nuclei-batch] HTTP error");
                    results.push(BatchNucleiResult {
                        cve_id: cve_id.clone(),
                        templates: vec![],
                        error: Some(format!("HTTP {}: {}", status, &body[..body.len().min(200)])),
                    });
                }
            }
            Err(e) => {
                tracing::error!(cve_id = %cve_id, error = %e, "[nuclei-batch] Request failed");
                results.push(BatchNucleiResult {
                    cve_id: cve_id.clone(),
                    templates: vec![],
                    error: Some(e.to_string()),
                });
            }
        }
    }

    Ok(results)
}
