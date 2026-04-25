//! Nuclei targeted scan + fingerprint → PoC matching engine.

use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;

use super::helpers::{emit_progress, log_scan_op, which_tool, NUCLEI_CANCELLED};
use super::types::{PocMatch, ScanResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NucleiScanOptions {
    /// Rate limit (requests per second)
    pub rate_limit: Option<u32>,
    /// Bulk size (number of hosts to process per template)
    pub bulk_size: Option<u32>,
    /// Concurrency (number of templates to run in parallel)
    pub concurrency: Option<u32>,
    /// Tags to filter templates (e.g. ["cve", "rce"])
    pub tags: Option<Vec<String>>,
    /// Exclude tags
    pub exclude_tags: Option<Vec<String>>,
    /// Custom template directory path
    pub template_path: Option<String>,
    /// HTTP proxy
    pub proxy: Option<String>,
    /// Timeout per request (seconds)
    pub timeout: Option<u32>,
    /// Additional raw CLI arguments
    pub extra_args: Option<Vec<String>>,
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

    let start = std::time::Instant::now();
    let mut fingerprints = golish_db::repo::fingerprints::list_by_target(pool, tid)
        .await
        .map_err(|e| e.to_string())?;

    if fingerprints.is_empty() {
        let backfilled = backfill_fingerprints_from_target(pool, tid).await;
        if backfilled > 0 {
            tracing::info!("[PoC-Match] Backfilled {} fingerprints from targets table for {}", backfilled, target_id);
            fingerprints = golish_db::repo::fingerprints::list_by_target(pool, tid)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    if fingerprints.is_empty() {
        tracing::info!("[PoC-Match] 0 fingerprints for target {} after backfill attempt ({}ms)", target_id, start.elapsed().as_millis());
        return Ok(vec![]);
    }

    tracing::info!("[PoC-Match] {} fingerprints for target {} ({}ms): {:?}",
        fingerprints.len(), target_id, start.elapsed().as_millis(),
        fingerprints.iter().map(|f| format!("{}:{}", f.category, f.name)).collect::<Vec<_>>());

    let mut all_terms: Vec<(String, String, String)> = Vec::new();
    let mut tag_terms: Vec<String> = Vec::new();
    for fp in &fingerprints {
        let name_lower = fp.name.to_lowercase();
        let version = fp.version.clone().unwrap_or_default();
        tag_terms.push(name_lower.clone());
        for term in build_search_terms(&fp.name, fp.version.as_deref()) {
            all_terms.push((term, name_lower.clone(), version.clone()));
        }
    }

    let combined_pattern = all_terms.iter()
        .map(|(t, _, _)| regex::escape(t))
        .collect::<Vec<_>>()
        .join("|");

    let q_start = std::time::Instant::now();

    let rows_text = sqlx::query_as::<_, PocRow>(
        r#"SELECT DISTINCT id, cve_id, name, poc_type, severity, source, content
           FROM vuln_kb_pocs
           WHERE LOWER(name) ~* $1
              OR LOWER(cve_id) ~* $1
              OR LOWER(description) ~* $1
           LIMIT 200"#,
    )
    .bind(&combined_pattern)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let rows_tags = sqlx::query_as::<_, PocRow>(
        r#"SELECT DISTINCT id, cve_id, name, poc_type, severity, source, content
           FROM vuln_kb_pocs
           WHERE tags && $1
           LIMIT 200"#,
    )
    .bind(&tag_terms)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut rows = rows_text;
    rows.extend(rows_tags);
    tracing::info!("[PoC-Match] Queries returned {} rows ({}ms)",
        rows.len(), q_start.elapsed().as_millis());

    let mut seen_ids = std::collections::HashSet::new();
    let mut matches = Vec::new();

    for row in rows {
        let row_id_str = row.id.to_string();
        if !seen_ids.insert(row_id_str.clone()) { continue; }

        let row_name_lower = row.name.to_lowercase();
        let row_cve_lower = row.cve_id.to_lowercase();

        let matched = all_terms.iter().find(|(term, _, _)| {
            row_name_lower.contains(term) || row_cve_lower.contains(term)
        });

        let (fp_name, fp_ver) = match matched {
            Some((_, n, v)) => (n.clone(), v.clone()),
            None => {
                let fallback = all_terms.first().map(|(_, n, v)| (n.clone(), v.clone()))
                    .unwrap_or_default();
                fallback
            }
        };

        let template_id = extract_nuclei_template_id(&row.content);

        matches.push(PocMatch {
            poc_id: row_id_str,
            cve_id: row.cve_id,
            poc_name: row.name,
            poc_type: row.poc_type,
            severity: row.severity.unwrap_or_default(),
            source: row.source.unwrap_or_default(),
            matched_fingerprint: fp_name,
            matched_version: fp_ver,
            template_id,
        });
    }

    matches.sort_by(|a, b| {
        severity_rank(&b.severity).cmp(&severity_rank(&a.severity))
    });

    tracing::info!("[PoC-Match] Total {} matches in {}ms", matches.len(), start.elapsed().as_millis());
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

async fn backfill_fingerprints_from_target(pool: &sqlx::PgPool, target_id: Uuid) -> u32 {
    let row: Option<(String, String, String, sqlx::types::Json<serde_json::Value>, String)> = sqlx::query_as(
        "SELECT webserver, cdn_waf, os_info, ports, COALESCE(project_path, '') FROM targets WHERE id = $1"
    )
    .bind(target_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let Some((ws, cdn, os, ports, project_path)) = row else { return 0 };
    let pp = if project_path.is_empty() { None } else { Some(project_path.as_str()) };
    let mut count = 0u32;

    fn parse_sv(s: &str) -> (String, Option<String>) {
        let s = s.trim();
        if let Some(idx) = s.find('/') {
            let name = s[..idx].trim().to_string();
            let ver = s[idx + 1..].trim().to_string();
            if ver.is_empty() { (name, None) } else { (name, Some(ver)) }
        } else {
            (s.to_string(), None)
        }
    }

    if !ws.is_empty() {
        let (name, version) = parse_sv(&ws);
        let ev = serde_json::json!({ "source": "backfill", "raw": ws });
        if golish_db::repo::fingerprints::upsert(pool, target_id, pp, "webserver", &name, version.as_deref(), 0.8, &ev, None, "httpx").await.is_ok() {
            count += 1;
        }
    }
    if !cdn.is_empty() {
        let ev = serde_json::json!({ "source": "backfill", "raw": cdn });
        if golish_db::repo::fingerprints::upsert(pool, target_id, pp, "cdn", &cdn, None, 0.9, &ev, None, "httpx").await.is_ok() {
            count += 1;
        }
    }
    if !os.is_empty() {
        let (name, version) = parse_sv(&os);
        let ev = serde_json::json!({ "source": "backfill", "raw": os });
        if golish_db::repo::fingerprints::upsert(pool, target_id, pp, "os", &name, version.as_deref(), 0.6, &ev, None, "httpx").await.is_ok() {
            count += 1;
        }
    }

    if let Some(arr) = ports.0.as_array() {
        for port_entry in arr {
            if let Some(techs) = port_entry.get("technologies").and_then(|t| t.as_array()) {
                for tech_val in techs {
                    if let Some(tech) = tech_val.as_str() {
                        if !tech.is_empty() {
                            let (name, version) = parse_sv(tech);
                            let ev = serde_json::json!({ "source": "backfill", "port": port_entry.get("port") });
                            if golish_db::repo::fingerprints::upsert(pool, target_id, pp, "technology", &name, version.as_deref(), 0.7, &ev, None, "httpx").await.is_ok() {
                                count += 1;
                            }
                        }
                    }
                }
            }
            if let Some(ws_val) = port_entry.get("webserver").and_then(|w| w.as_str()) {
                if !ws_val.is_empty() {
                    let (name, version) = parse_sv(ws_val);
                    let ev = serde_json::json!({ "source": "backfill", "port": port_entry.get("port") });
                    if golish_db::repo::fingerprints::upsert(pool, target_id, pp, "webserver", &name, version.as_deref(), 0.8, &ev, None, "httpx").await.is_ok() {
                        count += 1;
                    }
                }
            }
        }
    }

    count
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
#[allow(dead_code)]
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
    options: Option<NucleiScanOptions>,
) -> Result<ScanResult, String> {
    let start = std::time::Instant::now();
    let pool = state.db_pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;

    let nuclei_path = which_tool("nuclei").await
        .ok_or_else(|| "Nuclei not found. Install via: brew install nuclei or go install github.com/projectdiscovery/nuclei/v3/cmd/nuclei@latest".to_string())?;

    let opts = options.unwrap_or(NucleiScanOptions {
        rate_limit: None, bulk_size: None, concurrency: None, tags: None,
        exclude_tags: None, template_path: None, proxy: None, timeout: None, extra_args: None,
    });

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

    if let Some(rl) = opts.rate_limit {
        args.extend_from_slice(&["-rate-limit".to_string(), rl.to_string()]);
    }
    if let Some(bs) = opts.bulk_size {
        args.extend_from_slice(&["-bulk-size".to_string(), bs.to_string()]);
    }
    if let Some(c) = opts.concurrency {
        args.extend_from_slice(&["-concurrency".to_string(), c.to_string()]);
    }
    if let Some(ref tags) = opts.tags {
        if !tags.is_empty() {
            args.extend_from_slice(&["-tags".to_string(), tags.join(",")]);
        }
    }
    if let Some(ref et) = opts.exclude_tags {
        if !et.is_empty() {
            args.extend_from_slice(&["-etags".to_string(), et.join(",")]);
        }
    }
    if let Some(ref tp) = opts.template_path {
        args.extend_from_slice(&["-t".to_string(), tp.clone()]);
    }
    if let Some(ref proxy) = opts.proxy {
        args.extend_from_slice(&["-proxy".to_string(), proxy.clone()]);
    }
    if let Some(t) = opts.timeout {
        args.extend_from_slice(&["-timeout".to_string(), t.to_string()]);
    }
    if let Some(ref extra) = opts.extra_args {
        args.extend(extra.iter().cloned());
    }

    NUCLEI_CANCELLED.store(false, Ordering::SeqCst);
    emit_progress(&app, "nuclei", "scanning", 0, total, &format!("Scanning {} with {} templates", target_url, total));

    let mut child = tokio::process::Command::new(&nuclei_path)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Nuclei execution failed: {}", e))?;

    let child_stdout = child.stdout.take();
    let child_stderr = child.stderr.take();

    let stdout_handle = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        if let Some(mut r) = child_stdout { let _ = r.read_to_end(&mut buf).await; }
        buf
    });
    let _stderr_handle = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        if let Some(mut r) = child_stderr { let _ = r.read_to_end(&mut buf).await; }
        buf
    });

    let wait_result = tokio::select! {
        res = child.wait() => res.map_err(|e| format!("Nuclei wait failed: {}", e)),
        _ = async {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                if NUCLEI_CANCELLED.load(Ordering::SeqCst) { break; }
            }
        } => {
            let _ = child.kill().await;
            return Err("Nuclei scan cancelled".to_string());
        }
    };
    let _exit_status = wait_result?;
    let stdout_bytes = stdout_handle.await.unwrap_or_default();
    let stdout = String::from_utf8_lossy(&stdout_bytes);

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
            r#"INSERT INTO findings (id, target, target_id, title, severity, description, evidence, tool, source, project_path)
               VALUES ($1, $2, $3, $4, $5, $6, $7, 'nuclei', 'nuclei', $8)
               ON CONFLICT (id) DO UPDATE SET
                   description = EXCLUDED.description,
                   evidence = EXCLUDED.evidence,
                   target_id = COALESCE(EXCLUDED.target_id, findings.target_id)"#,
        )
        .bind(finding_id)
        .bind(matched_url)
        .bind(tid)
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

    let duration_ms = start.elapsed().as_millis() as u64;
    let result = ScanResult {
        tool: "nuclei".to_string(),
        success: errors.is_empty(),
        items_found,
        items_stored,
        errors,
        duration_ms,
    };

    log_scan_op(
        pool, "nuclei_targeted_scan",
        &format!("Nuclei targeted scan on {}: {} templates, {} findings", target_url, total, items_found),
        project_path.as_deref(), Some(tid), "nuclei",
        if result.success { "completed" } else { "partial" },
        &serde_json::json!({ "templates": total, "items_found": items_found, "items_stored": items_stored, "duration_ms": duration_ms }),
    ).await;

    Ok(result)
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

