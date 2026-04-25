//! WhatWeb fingerprinting scanner.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::state::AppState;

use super::helpers::{emit_progress, log_scan_op, which_tool};
use super::types::ScanResult;

// ============================================================================
// WhatWeb scanner
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatWebOptions {
    /// Aggression level: 1 (stealthy) to 4 (heavy)
    pub aggression: Option<u32>,
    /// Specific plugins to enable (comma-separated in WhatWeb)
    pub plugins: Option<Vec<String>>,
    /// Custom user-agent string
    pub user_agent: Option<String>,
    /// HTTP proxy (e.g. http://127.0.0.1:8080)
    pub proxy: Option<String>,
    /// Additional raw CLI arguments
    pub extra_args: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct WhatWebResult {
    target: Option<String>,
    #[serde(default)]
    plugins: HashMap<String, serde_json::Value>,
}

async fn parse_whatweb_and_store(
    pool: &PgPool,
    json_output: &str,
    target_id: Uuid,
    project_path: Option<&str>,
) -> (u32, Vec<String>) {
    let mut stored = 0u32;
    let mut errors = Vec::new();

    let results: Vec<WhatWebResult> = match serde_json::from_str(json_output) {
        Ok(v) => v,
        Err(e) => {
            errors.push(format!("WhatWeb JSON parse failed: {}", e));
            return (0, errors);
        }
    };

    for result in &results {
        for (plugin_name, value) in &result.plugins {
            let name_lower = plugin_name.to_lowercase();
            if name_lower == "httpserver" || name_lower == "http-server" {
                continue;
            }

            let category = infer_whatweb_category(plugin_name);
            let (version, confidence) = extract_whatweb_version_confidence(value);

            let evidence = serde_json::json!({
                "source": "whatweb",
                "raw": value,
                "target": result.target,
            });

            if let Err(e) = golish_db::repo::fingerprints::upsert(
                pool,
                target_id,
                project_path,
                &category,
                plugin_name,
                version.as_deref(),
                confidence,
                &evidence,
                None,
                "whatweb",
            )
            .await
            {
                errors.push(format!("Failed to store {}: {}", plugin_name, e));
            } else {
                stored += 1;
            }
        }
    }

    (stored, errors)
}

fn infer_whatweb_category(plugin_name: &str) -> String {
    let lower = plugin_name.to_lowercase();
    if lower.contains("php") || lower.contains("asp") || lower.contains("python")
        || lower.contains("ruby") || lower.contains("java") || lower.contains("node")
    {
        "language".to_string()
    } else if lower.contains("apache") || lower.contains("nginx") || lower.contains("iis")
        || lower.contains("lighttpd") || lower.contains("tomcat") || lower.contains("caddy")
    {
        "web_server".to_string()
    } else if lower.contains("wordpress") || lower.contains("drupal") || lower.contains("joomla")
        || lower.contains("shopify") || lower.contains("magento")
    {
        "cms".to_string()
    } else if lower.contains("jquery") || lower.contains("react") || lower.contains("angular")
        || lower.contains("vue") || lower.contains("bootstrap")
    {
        "frontend_framework".to_string()
    } else if lower.contains("spring") || lower.contains("django") || lower.contains("laravel")
        || lower.contains("express") || lower.contains("flask") || lower.contains("rails")
    {
        "backend_framework".to_string()
    } else if lower.contains("mysql") || lower.contains("postgres") || lower.contains("mongo")
        || lower.contains("redis") || lower.contains("sqlite")
    {
        "database".to_string()
    } else if lower.contains("cdn") || lower.contains("cloudflare") || lower.contains("akamai")
        || lower.contains("fastly")
    {
        "cdn".to_string()
    } else if lower.contains("waf") || lower.contains("firewall") || lower.contains("mod_security") {
        "security".to_string()
    } else if lower.contains("os") || lower.contains("linux") || lower.contains("windows")
        || lower.contains("ubuntu") || lower.contains("centos") || lower.contains("debian")
    {
        "os".to_string()
    } else {
        "technology".to_string()
    }
}

fn extract_whatweb_version_confidence(value: &serde_json::Value) -> (Option<String>, f32) {
    let mut version: Option<String> = None;
    let mut confidence = 0.5f32;

    if let Some(obj) = value.as_object() {
        if let Some(ver) = obj.get("version") {
            if let Some(v) = ver.as_array() {
                if let Some(first) = v.first().and_then(|v| v.as_str()) {
                    version = Some(first.to_string());
                    confidence = 0.9;
                }
            } else if let Some(v) = ver.as_str() {
                version = Some(v.to_string());
                confidence = 0.9;
            }
        }
        if let Some(c) = obj.get("certainty") {
            if let Some(n) = c.as_f64() {
                confidence = (n as f32 / 100.0).clamp(0.0, 1.0);
            }
        }
    }

    (version, confidence)
}

#[tauri::command]
pub async fn scan_whatweb(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    target_url: String,
    target_id: String,
    project_path: Option<String>,
    options: Option<WhatWebOptions>,
) -> Result<ScanResult, String> {
    let start = std::time::Instant::now();
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;

    let whatweb_path = which_tool("whatweb").await
        .ok_or_else(|| "WhatWeb not found. Install via: brew install whatweb or gem install whatweb".to_string())?;

    emit_progress(&app, "whatweb", "running", 0, 1, &format!("Scanning {}", target_url));

    let opts = options.unwrap_or(WhatWebOptions {
        aggression: None, plugins: None, user_agent: None, proxy: None, extra_args: None,
    });

    let mut args = vec![
        "--color=never".to_string(),
        "--log-json=-".to_string(),
        "--quiet".to_string(),
    ];

    if let Some(agg) = opts.aggression {
        args.push(format!("--aggression={}", agg.clamp(1, 4)));
    }
    if let Some(ref plugins) = opts.plugins {
        if !plugins.is_empty() {
            args.push(format!("--plugins={}", plugins.join(",")));
        }
    }
    if let Some(ref ua) = opts.user_agent {
        args.push(format!("--user-agent={}", ua));
    }
    if let Some(ref proxy) = opts.proxy {
        args.push(format!("--proxy={}", proxy));
    }
    if let Some(ref extra) = opts.extra_args {
        args.extend(extra.iter().cloned());
    }
    args.push(target_url.clone());

    let output = tokio::process::Command::new(&whatweb_path)
        .args(&args)
        .output()
        .await
        .map_err(|e| format!("WhatWeb execution failed: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if stdout.trim().is_empty() {
        return Err(format!("WhatWeb returned no output. stderr: {}", stderr.trim()));
    }

    emit_progress(&app, "whatweb", "parsing", 1, 2, "Parsing results...");

    let (stored, errors) = parse_whatweb_and_store(
        pool, &stdout, tid, project_path.as_deref(),
    ).await;

    emit_progress(&app, "whatweb", "done", 1, 1, &format!("Found {} technologies", stored));

    let duration_ms = start.elapsed().as_millis() as u64;
    let result = ScanResult {
        tool: "whatweb".to_string(),
        success: errors.is_empty(),
        items_found: stored + errors.len() as u32,
        items_stored: stored,
        errors,
        duration_ms,
    };

    log_scan_op(
        pool, "whatweb_scan", &format!("WhatWeb scan on {}: {} techs found", target_url, stored),
        project_path.as_deref(), Some(tid), "whatweb",
        if result.success { "completed" } else { "partial" },
        &serde_json::json!({ "items_found": result.items_found, "items_stored": result.items_stored, "duration_ms": duration_ms }),
    ).await;

    Ok(result)
}

