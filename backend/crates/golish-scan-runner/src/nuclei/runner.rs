//! Nuclei targeted-scan runner.
//!
//! Spawns the `nuclei` CLI against a single target URL, parses streaming
//! JSON-Lines output, and persists hits into the database.

use std::sync::atomic::Ordering;

use golish_core::EventEmitterHandle;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::helpers::{emit_progress, log_scan_op, which_tool, NUCLEI_CANCELLED};
use crate::types::ScanResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NucleiScanOptions {
    pub rate_limit: Option<u32>,
    pub bulk_size: Option<u32>,
    pub concurrency: Option<u32>,
    pub tags: Option<Vec<String>>,
    pub exclude_tags: Option<Vec<String>>,
    pub template_path: Option<String>,
    pub proxy: Option<String>,
    pub timeout: Option<u32>,
    pub extra_args: Option<Vec<String>>,
}

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

pub async fn run_nuclei_targeted(
    pool: &sqlx::PgPool,
    emitter: Option<&EventEmitterHandle>,
    target_url: &str,
    target_id: Uuid,
    project_path: Option<&str>,
    template_ids: &[String],
    severity_filter: Option<&[String]>,
    options: Option<NucleiScanOptions>,
) -> crate::ScanRunnerResult<ScanResult> {
    let start = std::time::Instant::now();

    let nuclei_path = which_tool("nuclei").await.ok_or_else(|| {
        crate::ScanRunnerError::Nuclei("Nuclei not found. Install via: brew install nuclei or go install github.com/projectdiscovery/nuclei/v3/cmd/nuclei@latest".into())
    })?;

    let opts = options.unwrap_or(NucleiScanOptions {
        rate_limit: None,
        bulk_size: None,
        concurrency: None,
        tags: None,
        exclude_tags: None,
        template_path: None,
        proxy: None,
        timeout: None,
        extra_args: None,
    });

    let total = template_ids.len() as u32;
    emit_progress(
        emitter,
        "nuclei",
        "preparing",
        0,
        total,
        &format!("Preparing targeted scan with {} templates", total),
    );

    let mut args = vec![
        "-target".to_string(),
        target_url.to_string(),
        "-jsonl".to_string(),
        "-silent".to_string(),
        "-no-color".to_string(),
        "-stats".to_string(),
    ];

    if !template_ids.is_empty() {
        args.push("-template-id".to_string());
        args.push(template_ids.join(","));
    }

    if let Some(sevs) = severity_filter {
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
    emit_progress(
        emitter,
        "nuclei",
        "scanning",
        0,
        total,
        &format!("Scanning {} with {} templates", target_url, total),
    );

    let mut child = tokio::process::Command::new(&nuclei_path)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| crate::ScanRunnerError::Nuclei(format!("Nuclei execution failed: {}", e)))?;

    let child_stdout = child.stdout.take();
    let child_stderr = child.stderr.take();

    let stdout_handle = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        if let Some(mut r) = child_stdout {
            let _ = r.read_to_end(&mut buf).await;
        }
        buf
    });
    let _stderr_handle = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        if let Some(mut r) = child_stderr {
            let _ = r.read_to_end(&mut buf).await;
        }
        buf
    });

    let wait_result = tokio::select! {
        res = child.wait() => res.map_err(|e| crate::ScanRunnerError::Nuclei(format!("Nuclei wait failed: {}", e))),
        _ = async {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                if NUCLEI_CANCELLED.load(Ordering::SeqCst) { break; }
            }
        } => {
            let _ = child.kill().await;
            return Err(crate::ScanRunnerError::Nuclei("Nuclei scan cancelled".into()));
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
        if trimmed.is_empty() {
            continue;
        }

        let result: NucleiJsonResult = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(_) => continue,
        };

        items_found += 1;
        let info = result.info.as_ref();
        let title = info.and_then(|i| i.name.as_deref()).unwrap_or("Unknown");
        let severity = info.and_then(|i| i.severity.as_deref()).unwrap_or("info");
        let description = info.and_then(|i| i.description.as_deref()).unwrap_or("");
        let matched_url = result.matched_at.as_deref().unwrap_or(target_url);
        let tmpl_id = result.template_id.as_deref().unwrap_or("");

        let cve_id = extract_cve_from_template(tmpl_id)
            .or_else(|| extract_cve_from_tags(info.and_then(|i| i.tags.as_ref())));

        let evidence = serde_json::json!({
            "template_id": tmpl_id,
            "matcher_name": result.matcher_name,
            "extracted_results": result.extracted_results,
            "curl_command": result.curl_command,
            "scan_type": result.scan_type,
            "references": info.and_then(|i| i.reference.clone()),
        });

        let finding_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("nuclei:{}:{}:{}", tmpl_id, matched_url, target_id).as_bytes(),
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
        .bind(target_id)
        .bind(title)
        .bind(severity)
        .bind(description)
        .bind(evidence.to_string())
        .bind(project_path)
        .execute(pool)
        .await;

        match insert_result {
            Ok(_) => items_stored += 1,
            Err(e) => errors.push(format!("Failed to store finding: {}", e)),
        }

        let scan_log_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("nuclei-log:{}:{}:{}", tmpl_id, matched_url, target_id).as_bytes(),
        );
        let _ = sqlx::query(
            r#"INSERT INTO passive_scan_logs
                   (id, target_id, test_type, url, result, evidence, severity, tool_used, tester, notes, project_path)
               VALUES ($1, $2, $3, $4, $5, $6, $7, 'nuclei', 'nuclei-scanner', $8, $9)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(scan_log_id)
        .bind(target_id)
        .bind(format!("nuclei:{}", tmpl_id))
        .bind(matched_url)
        .bind("vulnerable")
        .bind(description)
        .bind(severity)
        .bind(format!(
            "Template: {}, CVE: {}",
            tmpl_id,
            cve_id.as_deref().unwrap_or("N/A")
        ))
        .bind(project_path)
        .execute(pool)
        .await;

        emit_progress(
            emitter,
            "nuclei",
            "found",
            items_found,
            total,
            &format!("[{}] {} at {}", severity.to_uppercase(), title, matched_url),
        );
    }

    emit_progress(
        emitter,
        "nuclei",
        "done",
        items_stored,
        items_found,
        &format!(
            "Scan complete: {} findings from {} templates",
            items_found, total
        ),
    );

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
        pool,
        "nuclei_targeted_scan",
        &format!(
            "Nuclei targeted scan on {}: {} templates, {} findings",
            target_url, total, items_found
        ),
        project_path,
        Some(target_id),
        "nuclei",
        if result.success {
            "completed"
        } else {
            "partial"
        },
        &serde_json::json!({ "templates": total, "items_found": items_found, "items_stored": items_stored, "duration_ms": duration_ms }),
    )
    .await;

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
    tags?.iter()
        .find(|t| t.to_uppercase().starts_with("CVE-"))
        .map(|t| t.to_uppercase())
}
