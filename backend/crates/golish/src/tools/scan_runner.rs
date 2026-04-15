use crate::state::AppState;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use tauri::Emitter;
use uuid::Uuid;

// ============================================================================
// Shared types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgress {
    pub tool: String,
    pub phase: String,
    pub current: u32,
    pub total: u32,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub tool: String,
    pub success: bool,
    pub items_found: u32,
    pub items_stored: u32,
    pub errors: Vec<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PocMatch {
    pub poc_id: String,
    pub cve_id: String,
    pub poc_name: String,
    pub poc_type: String,
    pub severity: String,
    pub source: String,
    pub matched_fingerprint: String,
    pub matched_version: String,
    pub template_id: Option<String>,
}

fn emit_progress(app: &tauri::AppHandle, tool: &str, phase: &str, current: u32, total: u32, msg: &str) {
    let _ = app.emit("scan-progress", ScanProgress {
        tool: tool.to_string(),
        phase: phase.to_string(),
        current,
        total,
        message: msg.to_string(),
    });
}

async fn which_tool(name: &str) -> Option<String> {
    let output = tokio::process::Command::new("which")
        .arg(name)
        .output()
        .await
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

// ============================================================================
// WhatWeb scanner
// ============================================================================

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
) -> Result<ScanResult, String> {
    let start = std::time::Instant::now();
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;

    let whatweb_path = which_tool("whatweb").await
        .ok_or_else(|| "WhatWeb not found. Install via: brew install whatweb or gem install whatweb".to_string())?;

    emit_progress(&app, "whatweb", "running", 0, 1, &format!("Scanning {}", target_url));

    let output = tokio::process::Command::new(&whatweb_path)
        .args(["--color=never", "--log-json=-", "--quiet", &target_url])
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

    Ok(ScanResult {
        tool: "whatweb".to_string(),
        success: errors.is_empty(),
        items_found: stored + errors.len() as u32,
        items_stored: stored,
        errors,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

// ============================================================================
// Fingerprint → PoC matching engine
// ============================================================================

#[tauri::command]
pub async fn match_pocs_for_target(
    state: tauri::State<'_, AppState>,
    target_id: String,
) -> Result<Vec<PocMatch>, String> {
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;

    let fingerprints = golish_db::repo::fingerprints::list_by_target(pool, tid)
        .await
        .map_err(|e| e.to_string())?;

    if fingerprints.is_empty() {
        return Ok(vec![]);
    }

    let mut matches = Vec::new();

    for fp in &fingerprints {
        let name_lower = fp.name.to_lowercase();
        let search_terms = build_search_terms(&fp.name, fp.version.as_deref());

        for term in &search_terms {
            let rows = sqlx::query_as::<_, PocRow>(
                r#"SELECT id, cve_id, name, poc_type, severity, source, content
                   FROM vuln_kb_pocs
                   WHERE LOWER(tags::text) LIKE $1
                      OR LOWER(name) LIKE $1
                      OR LOWER(cve_id) LIKE $1
                      OR LOWER(description) LIKE $1
                   LIMIT 50"#,
            )
            .bind(format!("%{}%", term))
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;

            for row in rows {
                if matches.iter().any(|m: &PocMatch| m.poc_id == row.id.to_string()) {
                    continue;
                }

                let template_id = extract_nuclei_template_id(&row.content);

                matches.push(PocMatch {
                    poc_id: row.id.to_string(),
                    cve_id: row.cve_id,
                    poc_name: row.name,
                    poc_type: row.poc_type,
                    severity: row.severity.unwrap_or_default(),
                    source: row.source.unwrap_or_default(),
                    matched_fingerprint: name_lower.clone(),
                    matched_version: fp.version.clone().unwrap_or_default(),
                    template_id,
                });
            }
        }
    }

    matches.sort_by(|a, b| {
        severity_rank(&b.severity).cmp(&severity_rank(&a.severity))
    });

    Ok(matches)
}

#[derive(sqlx::FromRow)]
struct PocRow {
    id: Uuid,
    cve_id: String,
    name: String,
    poc_type: String,
    severity: Option<String>,
    source: Option<String>,
    content: String,
}

fn build_search_terms(name: &str, version: Option<&str>) -> Vec<String> {
    let lower = name.to_lowercase();
    let mut terms = vec![lower.clone()];

    let mapped = match lower.as_str() {
        "apache" => Some("apache"),
        "nginx" => Some("nginx"),
        "iis" | "microsoft-iis" => Some("iis"),
        "tomcat" | "apache-tomcat" => Some("tomcat"),
        "wordpress" => Some("wordpress"),
        "drupal" => Some("drupal"),
        "joomla" => Some("joomla"),
        "php" => Some("php"),
        "jquery" => Some("jquery"),
        "spring" | "spring-boot" | "spring-framework" => Some("spring"),
        "struts" | "apache-struts" => Some("struts"),
        "log4j" => Some("log4j"),
        "openssl" => Some("openssl"),
        "jenkins" => Some("jenkins"),
        "gitlab" => Some("gitlab"),
        "grafana" => Some("grafana"),
        "elasticsearch" => Some("elasticsearch"),
        "redis" => Some("redis"),
        "mongodb" | "mongo" => Some("mongodb"),
        _ => None,
    };
    if let Some(m) = mapped {
        if m != lower {
            terms.push(m.to_string());
        }
    }

    if let Some(ver) = version {
        terms.push(format!("{} {}", lower, ver));
    }

    terms
}

fn extract_nuclei_template_id(content: &str) -> Option<String> {
    for line in content.lines().take(20) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("id:") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn severity_rank(s: &str) -> u8 {
    match s.to_lowercase().as_str() {
        "critical" => 4,
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

// ============================================================================
// Nuclei targeted scan
// ============================================================================

#[derive(Debug, Deserialize)]
struct NucleiJsonResult {
    #[serde(rename = "template-id")]
    template_id: Option<String>,
    info: Option<NucleiInfo>,
    host: Option<String>,
    #[serde(rename = "matched-at")]
    matched_at: Option<String>,
    #[serde(rename = "matcher-name")]
    matcher_name: Option<String>,
    #[serde(rename = "extracted-results")]
    extracted_results: Option<Vec<String>>,
    #[serde(rename = "curl-command")]
    curl_command: Option<String>,
    #[serde(rename = "type")]
    scan_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NucleiInfo {
    name: Option<String>,
    severity: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
    reference: Option<Vec<String>>,
}

#[tauri::command]
pub async fn scan_nuclei_targeted(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    target_url: String,
    target_id: String,
    project_path: Option<String>,
    template_ids: Vec<String>,
    severity_filter: Option<Vec<String>>,
) -> Result<ScanResult, String> {
    let start = std::time::Instant::now();
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;

    let nuclei_path = which_tool("nuclei").await
        .ok_or_else(|| "Nuclei not found. Install via: brew install nuclei or go install github.com/projectdiscovery/nuclei/v3/cmd/nuclei@latest".to_string())?;

    let total = template_ids.len() as u32;
    emit_progress(&app, "nuclei", "preparing", 0, total, &format!("Preparing targeted scan with {} templates", total));

    let mut args = vec![
        "-target".to_string(), target_url.clone(),
        "-jsonl".to_string(),
        "-silent".to_string(),
        "-no-color".to_string(),
        "-stats".to_string(),
    ];

    if !template_ids.is_empty() {
        args.push("-template-id".to_string());
        args.push(template_ids.join(","));
    }

    if let Some(ref sevs) = severity_filter {
        if !sevs.is_empty() {
            args.push("-severity".to_string());
            args.push(sevs.join(","));
        }
    }

    emit_progress(&app, "nuclei", "scanning", 0, total, &format!("Scanning {} with {} templates", target_url, total));

    let output = tokio::process::Command::new(&nuclei_path)
        .args(&args)
        .output()
        .await
        .map_err(|e| format!("Nuclei execution failed: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut items_found = 0u32;
    let mut items_stored = 0u32;
    let mut errors = Vec::new();

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }

        let result: NucleiJsonResult = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(_) => continue,
        };

        items_found += 1;
        let info = result.info.as_ref();
        let title = info.and_then(|i| i.name.as_deref()).unwrap_or("Unknown");
        let severity = info.and_then(|i| i.severity.as_deref()).unwrap_or("info");
        let description = info.and_then(|i| i.description.as_deref()).unwrap_or("");
        let matched_url = result.matched_at.as_deref().unwrap_or(&target_url);
        let template_id = result.template_id.as_deref().unwrap_or("");

        let cve_id = extract_cve_from_template(template_id)
            .or_else(|| extract_cve_from_tags(info.and_then(|i| i.tags.as_ref())));

        let evidence = serde_json::json!({
            "template_id": template_id,
            "matcher_name": result.matcher_name,
            "extracted_results": result.extracted_results,
            "curl_command": result.curl_command,
            "scan_type": result.scan_type,
            "references": info.and_then(|i| i.reference.clone()),
        });

        let finding_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("nuclei:{}:{}:{}", template_id, matched_url, target_id).as_bytes(),
        );

        let insert_result = sqlx::query(
            r#"INSERT INTO findings (id, target, title, severity, description, evidence, tool, source, project_path)
               VALUES ($1, $2, $3, $4, $5, $6, 'nuclei', 'nuclei', $7)
               ON CONFLICT (id) DO UPDATE SET
                   description = EXCLUDED.description,
                   evidence = EXCLUDED.evidence"#,
        )
        .bind(finding_id)
        .bind(matched_url)
        .bind(title)
        .bind(severity)
        .bind(description)
        .bind(evidence.to_string())
        .bind(project_path.as_deref())
        .execute(pool)
        .await;

        match insert_result {
            Ok(_) => items_stored += 1,
            Err(e) => errors.push(format!("Failed to store finding: {}", e)),
        }

        let scan_log_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("nuclei-log:{}:{}:{}", template_id, matched_url, target_id).as_bytes(),
        );
        let _ = sqlx::query(
            r#"INSERT INTO passive_scan_logs
                   (id, target_id, test_type, url, result, evidence, severity, tool_used, tester, notes, project_path)
               VALUES ($1, $2, $3, $4, $5, $6, $7, 'nuclei', 'nuclei-scanner', $8, $9)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(scan_log_id)
        .bind(tid)
        .bind(format!("nuclei:{}", template_id))
        .bind(matched_url)
        .bind("vulnerable")
        .bind(description)
        .bind(severity)
        .bind(format!("Template: {}, CVE: {}", template_id, cve_id.as_deref().unwrap_or("N/A")))
        .bind(project_path.as_deref())
        .execute(pool)
        .await;

        emit_progress(&app, "nuclei", "found", items_found, total, &format!("[{}] {} at {}", severity.to_uppercase(), title, matched_url));
    }

    emit_progress(&app, "nuclei", "done", items_stored, items_found, &format!("Scan complete: {} findings from {} templates", items_found, total));

    Ok(ScanResult {
        tool: "nuclei".to_string(),
        success: errors.is_empty(),
        items_found,
        items_stored,
        errors,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

fn extract_cve_from_template(template_id: &str) -> Option<String> {
    let upper = template_id.to_uppercase();
    if upper.starts_with("CVE-") {
        Some(upper)
    } else {
        None
    }
}

fn extract_cve_from_tags(tags: Option<&Vec<String>>) -> Option<String> {
    tags?.iter().find(|t| t.to_uppercase().starts_with("CVE-")).map(|t| t.to_uppercase())
}

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
                    r#"INSERT INTO findings (id, target, title, severity, description, tool, source, project_path)
                       VALUES ($1, $2, $3, $4, $5, 'feroxbuster', 'feroxbuster', $6)
                       ON CONFLICT (id) DO NOTHING"#,
                )
                .bind(finding_id)
                .bind(&url)
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

    Ok(ScanResult {
        tool: "feroxbuster".to_string(),
        success: all_errors.is_empty(),
        items_found: all_items_found,
        items_stored: all_items_stored,
        errors: all_errors,
        duration_ms: start.elapsed().as_millis() as u64,
    })
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
            SELECT unnest(urls) as url FROM topology_scans WHERE name = 'zap-sitemap'
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
