//! feroxbuster (directory busting) over ZAP-discovered paths.


use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;

use super::helpers::{emit_progress, log_scan_op, which_tool};
use super::types::ScanResult;

// ============================================================================
// feroxbuster scan (from ZAP-discovered paths)
// ============================================================================

#[derive(Debug, Deserialize)]
struct FeroxResult {
    url: Option<String>,
    status: Option<u32>,
    #[serde(rename = "content_length")]
    content_length: Option<i32>,
    line_count: Option<i32>,
    word_count: Option<i32>,
    #[serde(rename = "type")]
    result_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeroxScanOptions {
    pub depth: Option<u32>,
    pub threads: Option<u32>,
    pub wordlist: Option<String>,
    pub extensions: Option<Vec<String>>,
    pub status_codes: Option<Vec<u32>>,
    pub timeout: Option<u32>,
}

#[tauri::command]
pub async fn scan_feroxbuster(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    target_url: String,
    target_id: String,
    project_path: Option<String>,
    base_paths: Vec<String>,
    options: Option<FeroxScanOptions>,
) -> Result<ScanResult, String> {
    let start = std::time::Instant::now();
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;

    let ferox_path = which_tool("feroxbuster").await
        .ok_or_else(|| "feroxbuster not found. Install via: brew install feroxbuster or cargo install feroxbuster".to_string())?;

    let opts = options.unwrap_or(FeroxScanOptions {
        depth: Some(3),
        threads: Some(50),
        wordlist: None,
        extensions: None,
        status_codes: None,
        timeout: Some(10),
    });

    let urls_to_scan: Vec<String> = if base_paths.is_empty() {
        vec![target_url.clone()]
    } else {
        base_paths.iter().map(|p| {
            if p.starts_with("http://") || p.starts_with("https://") {
                p.clone()
            } else {
                let base = target_url.trim_end_matches('/');
                let path = p.trim_start_matches('/');
                format!("{}/{}", base, path)
            }
        }).collect()
    };

    let total_urls = urls_to_scan.len() as u32;
    let mut all_items_found = 0u32;
    let mut all_items_stored = 0u32;
    let mut all_errors = Vec::new();

    for (idx, scan_url) in urls_to_scan.iter().enumerate() {
        emit_progress(&app, "feroxbuster", "scanning", idx as u32, total_urls,
            &format!("Scanning {} ({}/{})", scan_url, idx + 1, total_urls));

        let mut args = vec![
            "--url".to_string(), scan_url.clone(),
            "--json".to_string(),
            "--no-state".to_string(),
            "--silent".to_string(),
            "--auto-tune".to_string(),
        ];

        if let Some(d) = opts.depth {
            args.extend_from_slice(&["--depth".to_string(), d.to_string()]);
        }
        if let Some(t) = opts.threads {
            args.extend_from_slice(&["--threads".to_string(), t.to_string()]);
        }
        if let Some(ref w) = opts.wordlist {
            args.extend_from_slice(&["--wordlist".to_string(), w.clone()]);
        }
        if let Some(ref exts) = opts.extensions {
            if !exts.is_empty() {
                args.extend_from_slice(&["--extensions".to_string(), exts.join(",")]);
            }
        }
        if let Some(ref codes) = opts.status_codes {
            if !codes.is_empty() {
                args.extend_from_slice(&["--status-codes".to_string(),
                    codes.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(",")]);
            }
        }
        if let Some(t) = opts.timeout {
            args.extend_from_slice(&["--timeout".to_string(), t.to_string()]);
        }

        let output = match tokio::process::Command::new(&ferox_path)
            .args(&args)
            .output()
            .await
        {
            Ok(o) => o,
            Err(e) => {
                all_errors.push(format!("feroxbuster failed for {}: {}", scan_url, e));
                continue;
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);

        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() { continue; }

            let result: FeroxResult = match serde_json::from_str(trimmed) {
                Ok(r) => r,
                Err(_) => continue,
            };

            if result.result_type.as_deref() != Some("response") { continue; }

            let url = match &result.url {
                Some(u) => u.clone(),
                None => continue,
            };
            let status = result.status.unwrap_or(0) as i32;

            all_items_found += 1;

            let store_result = crate::tools::targets::db_directory_entry_add(
                pool,
                Some(tid),
                &url,
                Some(status),
                result.content_length,
                result.line_count,
                result.word_count,
                "feroxbuster",
                project_path.as_deref(),
            ).await;

            match store_result {
                Ok(_) => all_items_stored += 1,
                Err(e) => all_errors.push(format!("Store failed for {}: {}", url, e)),
            }

            if is_sensitive_path(&url) {
                let finding_id = Uuid::new_v5(
                    &Uuid::NAMESPACE_URL,
                    format!("ferox:sensitive:{}:{}", url, target_id).as_bytes(),
                );
                let _ = sqlx::query(
                    r#"INSERT INTO findings (id, target, target_id, title, severity, description, tool, source, project_path)
                       VALUES ($1, $2, $3, $4, $5, $6, 'feroxbuster', 'feroxbuster', $7)
                       ON CONFLICT (id) DO NOTHING"#,
                )
                .bind(finding_id)
                .bind(&url)
                .bind(tid)
                .bind(format!("Sensitive file/directory: {}", extract_path(&url)))
                .bind(classify_sensitive_severity(&url))
                .bind(format!("Directory enumeration discovered a potentially sensitive resource at {} (HTTP {})", url, status))
                .bind(project_path.as_deref())
                .execute(pool)
                .await;

                emit_progress(&app, "feroxbuster", "sensitive", all_items_found, 0,
                    &format!("Sensitive: {} ({})", extract_path(&url), status));
            }
        }
    }

    emit_progress(&app, "feroxbuster", "done", all_items_stored, all_items_found,
        &format!("Found {} paths, {} stored", all_items_found, all_items_stored));

    let duration_ms = start.elapsed().as_millis() as u64;
    let result = ScanResult {
        tool: "feroxbuster".to_string(),
        success: all_errors.is_empty(),
        items_found: all_items_found,
        items_stored: all_items_stored,
        errors: all_errors,
        duration_ms,
    };

    log_scan_op(
        pool, "feroxbuster_scan",
        &format!("feroxbuster on {}: {} paths found, {} URLs scanned", target_url, all_items_found, total_urls),
        project_path.as_deref(), Some(tid), "feroxbuster",
        if result.success { "completed" } else { "partial" },
        &serde_json::json!({ "urls_scanned": total_urls, "items_found": all_items_found, "items_stored": all_items_stored, "duration_ms": duration_ms }),
    ).await;

    Ok(result)
}

fn is_sensitive_path(url: &str) -> bool {
    let path = extract_path(url).to_lowercase();
    let sensitive_patterns = [
        ".env", ".git", ".svn", ".htaccess", ".htpasswd",
        "wp-config", "config.php", "config.yml", "config.json",
        "backup", ".bak", ".sql", ".dump",
        "admin", "phpmyadmin", "adminer",
        ".DS_Store", "Thumbs.db",
        "web.config", "server-status", "server-info",
        ".aws", "credentials", "id_rsa", ".ssh",
        "phpinfo", "info.php",
        "debug", ".debug", "trace",
        "swagger", "api-docs", "graphql",
    ];
    sensitive_patterns.iter().any(|p| path.contains(p))
}

fn classify_sensitive_severity(url: &str) -> &'static str {
    let path = extract_path(url).to_lowercase();
    if path.contains(".env") || path.contains("credentials") || path.contains("id_rsa")
        || path.contains(".ssh") || path.contains("wp-config")
    {
        "high"
    } else if path.contains(".git") || path.contains("backup") || path.contains(".sql")
        || path.contains("phpinfo") || path.contains("config")
    {
        "medium"
    } else {
        "low"
    }
}

fn extract_path(url: &str) -> &str {
    url.find("://")
        .and_then(|i| url[i + 3..].find('/'))
        .map(|i| {
            let start = url.find("://").unwrap() + 3 + i;
            &url[start..]
        })
        .unwrap_or("/")
}

// ============================================================================
// Get ZAP-discovered paths for feroxbuster
// ============================================================================

#[tauri::command]
pub async fn get_zap_discovered_paths(
    state: tauri::State<'_, AppState>,
    target_host: String,
) -> Result<Vec<String>, String> {
    let pool = state.db_pool_ready().await?;

    let rows = sqlx::query_scalar::<_, String>(
        r#"SELECT DISTINCT url FROM (
            SELECT unnest(urls) as url FROM sitemap_store WHERE name = 'zap-sitemap'
        ) sub
        WHERE url LIKE $1
        ORDER BY url"#,
    )
    .bind(format!("%{}%", target_host))
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let paths: Vec<String> = rows.iter()
        .filter_map(|url| {
            url.find("://")
                .and_then(|i| url[i + 3..].find('/'))
                .map(|i| {
                    let start = url.find("://").unwrap() + 3 + i;
                    url[start..].to_string()
                })
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    Ok(paths)
}
