//! Bulk Nuclei template import via Git Trees/Contents API.
//!
//! Uses [`golish_core::EventEmitterHandle`] instead of `tauri::AppHandle`
//! so this crate compiles and tests without Tauri.

use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

use golish_core::EventEmitterHandle;

use crate::nuclei_search::extract_nuclei_severity;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NucleiDiscoverProgress {
    pub phase: String,
    pub current: usize,
    pub total: usize,
    pub cve_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NucleiDiscoverResult {
    pub total_files: usize,
    pub total_cves: usize,
    pub imported: usize,
    pub skipped: usize,
    pub errors: usize,
}

#[derive(Deserialize)]
struct GhTreeResponse {
    tree: Vec<GhTreeEntry>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Deserialize)]
struct GhTreeEntry {
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct GhContentsEntry {
    name: String,
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
    #[serde(default)]
    download_url: Option<String>,
}

fn extract_cve_from_path(path: &str) -> Option<String> {
    let re = regex::Regex::new(r"(CVE-\d{4}-\d{4,})").ok()?;
    re.captures(path).map(|c| c[1].to_uppercase())
}

fn extract_template_identifier(path: &str) -> String {
    if let Some(cve) = extract_cve_from_path(path) {
        return cve;
    }
    let file_name = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".yaml");
    format!("NUCLEI-{}", file_name.to_uppercase())
}

const NUCLEI_DIRS: &[&str] = &[
    "http/cves",
    "network/cves",
    "dns/cves",
    "ssl/cves",
    "http/technologies",
    "http/misconfiguration",
    "http/default-logins",
    "http/exposures",
    "http/vulnerabilities",
    "http/miscellaneous",
];

fn extract_nuclei_tags(content: &str) -> Vec<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("tags:") {
            let raw = trimmed.trim_start_matches("tags:").trim();
            return raw
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
        }
    }
    Vec::new()
}

fn extract_nuclei_description(content: &str) -> String {
    let mut in_info = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "info:" {
            in_info = true;
            continue;
        }
        if in_info {
            if let Some(stripped) = trimmed.strip_prefix("description:") {
                let desc = stripped.trim().trim_matches('"').trim_matches('\'');
                if !desc.is_empty() {
                    return desc.to_string();
                }
            }
            if let Some(stripped) = trimmed.strip_prefix("name:") {
                let name = stripped.trim().trim_matches('"').trim_matches('\'');
                if !name.is_empty() {
                    return name.to_string();
                }
            }
            if trimmed.is_empty()
                || (!trimmed.contains(':')
                    && !trimmed.starts_with('-')
                    && !trimmed.starts_with(' '))
            {
                in_info = false;
            }
        }
    }
    String::new()
}

fn emit_progress(emitter: Option<&EventEmitterHandle>, progress: &NucleiDiscoverProgress) {
    if let Some(e) = emitter {
        e.emit("nuclei-discover-progress", progress);
    }
}

/// Discover Nuclei templates from the nuclei-templates repo.
///
/// First tries Git Trees API; falls back to Contents API per directory
/// if tree is too large. Imports CVE templates AND technology/misconfiguration
/// templates.
pub async fn discover_all_nuclei(
    pool: &sqlx::PgPool,
    client: &reqwest::Client,
    headers: &HeaderMap,
    emitter: Option<&EventEmitterHandle>,
) -> crate::VulnIntelResult<NucleiDiscoverResult> {
    emit_progress(
        emitter,
        &NucleiDiscoverProgress {
            phase: "listing".to_string(),
            current: 0,
            total: 0,
            cve_id: None,
        },
    );

    let yaml_paths = match fetch_tree_api(client, headers).await {
        Ok(paths) => {
            tracing::info!(count = paths.len(), "[nuclei-discover] Tree API succeeded");
            paths
        }
        Err(e) => {
            tracing::warn!(error = %e, "[nuclei-discover] Tree API failed, falling back to Contents API");
            emit_progress(
                emitter,
                &NucleiDiscoverProgress {
                    phase: "listing_fallback".to_string(),
                    current: 0,
                    total: NUCLEI_DIRS.len(),
                    cve_id: None,
                },
            );
            fetch_contents_api(client, headers, emitter).await?
        }
    };

    let total_files = yaml_paths.len();
    tracing::info!(total = total_files, "[nuclei-discover] Total YAML files to import");

    emit_progress(
        emitter,
        &NucleiDiscoverProgress {
            phase: "downloading".to_string(),
            current: 0,
            total: total_files,
            cve_id: None,
        },
    );

    let existing_ids: std::collections::HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT cve_id FROM vuln_kb_pocs WHERE source = 'nuclei_template'",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .collect();
    tracing::info!(existing = existing_ids.len(), "[nuclei-discover] Existing Nuclei templates in DB");

    let new_paths: Vec<&String> = yaml_paths
        .iter()
        .filter(|p| {
            let id = extract_template_identifier(p);
            !existing_ids.contains(&id)
        })
        .collect();

    let total_new = new_paths.len();
    tracing::info!(total_new = total_new, skipping = total_files - total_new, "[nuclei-discover] New templates to download");

    emit_progress(
        emitter,
        &NucleiDiscoverProgress {
            phase: "downloading".to_string(),
            current: 0,
            total: total_new,
            cve_id: Some(format!(
                "{} new, {} existing",
                total_new,
                existing_ids.len()
            )),
        },
    );

    let mut imported = 0usize;
    let mut skipped = total_files - total_new;
    let mut errors = 0usize;
    let mut seen_ids = std::collections::HashSet::new();

    for (i, path) in new_paths.iter().enumerate() {
        let identifier = extract_template_identifier(path);
        seen_ids.insert(identifier.clone());

        if i % 20 == 0 {
            emit_progress(
                emitter,
                &NucleiDiscoverProgress {
                    phase: "downloading".to_string(),
                    current: i,
                    total: total_new,
                    cve_id: Some(identifier.clone()),
                },
            );
        }

        if i > 0 && i % 50 == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        let raw_url = format!(
            "https://raw.githubusercontent.com/projectdiscovery/nuclei-templates/main/{}",
            path
        );

        let content = match client
            .get(&raw_url)
            .headers(headers.clone())
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => match r.text().await {
                Ok(t) => t,
                Err(_) => {
                    errors += 1;
                    continue;
                }
            },
            _ => {
                errors += 1;
                continue;
            }
        };

        let severity =
            extract_nuclei_severity(&content).unwrap_or_else(|| "unknown".to_string());
        let tags = extract_nuclei_tags(&content);
        let description = extract_nuclei_description(&content);
        let file_name = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".yaml");
        let template_name = format!("[Nuclei] {}", file_name);
        let source_url = format!(
            "https://github.com/projectdiscovery/nuclei-templates/blob/main/{}",
            path
        );

        match golish_db::repo::wiki_kb::upsert_poc_full(
            pool,
            &identifier,
            &template_name,
            "nuclei",
            "yaml",
            &content,
            "nuclei_template",
            &source_url,
            &severity,
            &description,
            &tags,
        )
        .await
        {
            Ok(_) => imported += 1,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("duplicate") || msg.contains("unique") || msg.contains("no rows")
                {
                    skipped += 1;
                } else {
                    tracing::warn!(id = %identifier, error = %msg, "[nuclei-discover] DB insert failed");
                    errors += 1;
                }
            }
        }
    }

    let result = NucleiDiscoverResult {
        total_files,
        total_cves: seen_ids.len(),
        imported,
        skipped,
        errors,
    };

    emit_progress(
        emitter,
        &NucleiDiscoverProgress {
            phase: "done".to_string(),
            current: total_new,
            total: total_new,
            cve_id: None,
        },
    );

    tracing::info!(
        imported = imported,
        skipped = skipped,
        errors = errors,
        total_ids = seen_ids.len(),
        "[nuclei-discover] Complete"
    );

    Ok(result)
}

async fn fetch_tree_api(
    client: &reqwest::Client,
    headers: &HeaderMap,
) -> crate::VulnIntelResult<Vec<String>> {
    use crate::error::VulnIntelError;

    let tree_url = "https://api.github.com/repos/projectdiscovery/nuclei-templates/git/trees/main?recursive=1";
    let resp = client
        .get(tree_url)
        .headers(headers.clone())
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(VulnIntelError::Github(format!(
            "GitHub API error {}: {}",
            status,
            &body[..body.len().min(300)]
        )));
    }

    let body_bytes = resp.bytes().await?;
    tracing::info!(size = body_bytes.len(), "[nuclei-discover] Tree response size");

    let tree_data: GhTreeResponse = serde_json::from_slice(&body_bytes)
        .map_err(|e| VulnIntelError::Nuclei(format!("Parse tree: {}", e)))?;

    if tree_data.truncated {
        tracing::warn!("[nuclei-discover] Tree was truncated");
    }

    let paths: Vec<String> = tree_data
        .tree
        .iter()
        .filter(|e| {
            e.entry_type == "blob"
                && e.path.ends_with(".yaml")
                && NUCLEI_DIRS.iter().any(|d| e.path.starts_with(d))
        })
        .map(|e| e.path.clone())
        .collect();

    Ok(paths)
}

async fn fetch_contents_api(
    client: &reqwest::Client,
    headers: &HeaderMap,
    emitter: Option<&EventEmitterHandle>,
) -> crate::VulnIntelResult<Vec<String>> {
    let mut all_paths = Vec::new();

    for (dir_idx, dir) in NUCLEI_DIRS.iter().enumerate() {
        tracing::info!(dir = %dir, "[nuclei-discover] Listing directory via Contents API");
        emit_progress(
            emitter,
            &NucleiDiscoverProgress {
                phase: "listing_fallback".to_string(),
                current: dir_idx,
                total: NUCLEI_DIRS.len(),
                cve_id: Some(dir.to_string()),
            },
        );

        let mut dirs_to_scan = vec![dir.to_string()];
        while let Some(current_dir) = dirs_to_scan.pop() {
            let url = format!(
                "https://api.github.com/repos/projectdiscovery/nuclei-templates/contents/{}?ref=main",
                current_dir
            );
            let resp = match client.get(&url).headers(headers.clone()).send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(dir = %current_dir, error = %e, "[nuclei-discover] Failed to list");
                    continue;
                }
            };
            if !resp.status().is_success() {
                tracing::warn!(dir = %current_dir, status = %resp.status(), "[nuclei-discover] Non-200");
                continue;
            }
            let entries: Vec<GhContentsEntry> = match resp.json().await {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(dir = %current_dir, error = %e, "[nuclei-discover] Parse failed");
                    continue;
                }
            };
            for entry in entries {
                if entry.entry_type == "dir" {
                    dirs_to_scan.push(entry.path);
                } else if entry.entry_type == "file" && entry.name.ends_with(".yaml") {
                    all_paths.push(entry.path);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    tracing::info!(count = all_paths.len(), "[nuclei-discover] Contents API collected files");
    Ok(all_paths)
}
