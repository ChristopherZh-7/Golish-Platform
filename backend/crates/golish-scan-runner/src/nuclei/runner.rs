//! Nuclei targeted-scan runner.
//!
//! Spawns the `nuclei` CLI against a single target URL, parses streaming
//! JSON-Lines output, and persists hits into the database.

use std::sync::atomic::Ordering;

use golish_core::EventEmitterHandle;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::authorization::{
    after_successful_validation, url_has_authorized_origin, AuthorizedScanTarget,
};
use crate::helpers::{
    audit_scan_completed, audit_scan_failed, audit_scan_started, emit_progress,
    scanner_process_succeeded, which_tool, NUCLEI_CANCELLED,
};
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
struct NucleiJsonResult {
    #[serde(rename = "template-id")]
    template_id: Option<String>,
    info: Option<NucleiInfo>,
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

#[derive(Debug)]
struct ValidatedNucleiJsonResult {
    template_id: String,
    matched_url: String,
    title: String,
    severity: String,
    description: String,
    matcher_name: Option<String>,
    extracted_results: Option<Vec<String>>,
    curl_command: Option<String>,
    scan_type: Option<String>,
    tags: Option<Vec<String>>,
    references: Option<Vec<String>>,
}

type NucleiPipeTask = JoinHandle<std::io::Result<Vec<u8>>>;

async fn read_nuclei_pipe<R>(pipe_name: &'static str, reader: Option<R>) -> std::io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut reader = reader.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            format!("Nuclei {pipe_name} pipe was not captured"),
        )
    })?;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await?;
    Ok(bytes)
}

async fn finish_nuclei_pipe(
    pipe_name: &'static str,
    task: NucleiPipeTask,
) -> crate::ScanRunnerResult<Vec<u8>> {
    task.await
        .map_err(|error| {
            crate::ScanRunnerError::Nuclei(format!(
                "Nuclei {pipe_name} output pump task failed: {error}"
            ))
        })?
        .map_err(|error| {
            crate::ScanRunnerError::Nuclei(format!(
                "Nuclei {pipe_name} output read failed: {error}"
            ))
        })
}

async fn collect_nuclei_output(
    stdout_task: NucleiPipeTask,
    stderr_task: NucleiPipeTask,
) -> crate::ScanRunnerResult<(Vec<u8>, Vec<u8>)> {
    let (stdout, stderr) = tokio::join!(
        finish_nuclei_pipe("stdout", stdout_task),
        finish_nuclei_pipe("stderr", stderr_task)
    );
    Ok((stdout?, stderr?))
}

fn required_nuclei_string(value: Option<String>, field: &str) -> Result<String, String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("missing or empty {field}"))
}

fn parse_nuclei_json_line(
    line: &str,
    authorization: &AuthorizedScanTarget,
    requested_template_ids: &[String],
) -> Result<ValidatedNucleiJsonResult, String> {
    let NucleiJsonResult {
        template_id,
        info,
        matched_at,
        matcher_name,
        extracted_results,
        curl_command,
        scan_type,
    } = serde_json::from_str(line).map_err(|error| format!("malformed JSON: {error}"))?;

    let template_id = required_nuclei_string(template_id, "template-id")?;
    if !safe_filter_token(&template_id) {
        return Err("template-id contains unsafe characters".to_string());
    }
    if !requested_template_ids
        .iter()
        .any(|requested| requested == &template_id)
    {
        return Err(format!(
            "template-id was not part of the targeted request: {template_id}"
        ));
    }

    let matched_url = required_nuclei_string(matched_at, "matched-at")?;
    if !url_has_authorized_origin(authorization, &matched_url) {
        return Err(format!(
            "matched-at is not an absolute URL on the authorized exact origin: {matched_url}"
        ));
    }

    let info = info.ok_or_else(|| "missing info object".to_string())?;
    let title = required_nuclei_string(info.name, "info.name")?;
    let severity = required_nuclei_string(info.severity, "info.severity")?;
    if !is_nuclei_severity(&severity) {
        return Err(format!("invalid info.severity: {severity}"));
    }

    Ok(ValidatedNucleiJsonResult {
        template_id,
        matched_url,
        title,
        severity: severity.to_ascii_lowercase(),
        description: info.description.unwrap_or_default(),
        matcher_name,
        extracted_results,
        curl_command,
        scan_type,
        tags: info.tags,
        references: info.reference,
    })
}

struct NucleiStoredHit<'a> {
    finding_id: Uuid,
    scan_log_id: Uuid,
    matched_url: &'a str,
    title: &'a str,
    severity: &'a str,
    description: &'a str,
    template_id: &'a str,
    cve_id: Option<&'a str>,
    evidence: &'a serde_json::Value,
}

async fn store_nuclei_hit_guarded(
    pool: &sqlx::PgPool,
    authorization: &AuthorizedScanTarget,
    hit: &NucleiStoredHit<'_>,
) -> crate::ScanRunnerResult<()> {
    let mut tx = pool.begin().await?;
    golish_db::repo::scoped::lock_target_write_guard(&mut tx, &authorization.guard).await?;

    sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO findings (id, target, target_id, title, severity, description, evidence, tool, source, project_path)
           VALUES ($1, $2, $3, $4, $5, $6, $7, 'nuclei', 'nuclei', $8)
           ON CONFLICT (id) DO UPDATE SET
               description = EXCLUDED.description,
               evidence = EXCLUDED.evidence,
               target_id = EXCLUDED.target_id
           WHERE findings.project_path IS NOT DISTINCT FROM EXCLUDED.project_path
           RETURNING id"#,
    )
    .bind(hit.finding_id)
    .bind(hit.matched_url)
    .bind(authorization.guard.target_id)
    .bind(hit.title)
    .bind(hit.severity)
    .bind(hit.description)
    .bind(hit.evidence.to_string())
    .bind(&authorization.guard.project_path)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO passive_scan_logs
               (id, target_id, test_type, url, result, evidence, severity, tool_used, tester, notes, project_path)
           VALUES ($1, $2, $3, $4, 'vulnerable', $5, $6, 'nuclei', 'nuclei-scanner', $7, $8)
           ON CONFLICT (id) DO UPDATE SET
               result = EXCLUDED.result,
               evidence = EXCLUDED.evidence,
               severity = EXCLUDED.severity,
               notes = EXCLUDED.notes
           WHERE passive_scan_logs.project_path IS NOT DISTINCT FROM EXCLUDED.project_path
           RETURNING id"#,
    )
    .bind(hit.scan_log_id)
    .bind(authorization.guard.target_id)
    .bind(format!("nuclei:{}", hit.template_id))
    .bind(hit.matched_url)
    .bind(hit.description)
    .bind(hit.severity)
    .bind(format!(
        "Template: {}, CVE: {}",
        hit.template_id,
        hit.cve_id.unwrap_or("N/A")
    ))
    .bind(&authorization.guard.project_path)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn run_nuclei_targeted(
    pool: &sqlx::PgPool,
    emitter: Option<&EventEmitterHandle>,
    authorization: &AuthorizedScanTarget,
    template_ids: &[String],
    severity_filter: Option<&[String]>,
    options: Option<NucleiScanOptions>,
) -> crate::ScanRunnerResult<ScanResult> {
    let start = std::time::Instant::now();
    let target_url = authorization.requested_url.as_str();
    let target_id = authorization.guard.target_id;
    let project_path = authorization.guard.project_path.as_str();

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
    validate_nuclei_launch_options(template_ids, severity_filter, &opts)?;

    let nuclei_path = match which_tool("nuclei").await {
        Some(p) => p,
        None => {
            let msg = "Nuclei not found. Install via: brew install nuclei or go install github.com/projectdiscovery/nuclei/v3/cmd/nuclei@latest";
            return Err(crate::ScanRunnerError::Nuclei(msg.into()));
        }
    };

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
        "-dr".to_string(),
        "-ni".to_string(),
        "-dut".to_string(),
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
    if let Some(t) = opts.timeout {
        args.extend_from_slice(&["-timeout".to_string(), t.to_string()]);
    }

    golish_db::repo::scoped::validate_target_write_guard(pool, &authorization.guard).await?;
    let parent_audit_id = audit_scan_started(
        pool,
        &authorization.guard,
        "nuclei_scan_started",
        "nuclei",
        target_url,
        serde_json::json!({
            "template_count": template_ids.len(),
            "templates_sample": template_ids.iter().take(20).cloned().collect::<Vec<_>>(),
            "severity_filter": severity_filter,
            "project_path": project_path,
            "exact_origin": authorization.exact_origin,
        }),
    )
    .await?;

    NUCLEI_CANCELLED.store(false, Ordering::SeqCst);
    emit_progress(
        emitter,
        "nuclei",
        "scanning",
        0,
        total,
        &format!("Scanning {} with {} templates", target_url, total),
    );

    let mut child = match after_successful_validation(
        async {
            golish_db::repo::scoped::validate_target_write_guard(pool, &authorization.guard)
                .await
                .map_err(crate::ScanRunnerError::from)
        },
        || async {
            tokio::process::Command::new(&nuclei_path)
                .args(&args)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(crate::ScanRunnerError::from)
        },
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Nuclei execution failed: {}", e);
            let _ = audit_scan_failed(
                pool,
                &authorization.guard,
                parent_audit_id,
                "nuclei_scan_failed",
                "nuclei",
                &msg,
                serde_json::json!({
                    "target_url": target_url,
                    "template_count": template_ids.len(),
                    "duration_ms": start.elapsed().as_millis() as u64,
                }),
            )
            .await;
            return Err(crate::ScanRunnerError::Nuclei(msg));
        }
    };

    let child_stdout = child.stdout.take();
    let child_stderr = child.stderr.take();

    let stdout_handle = tokio::spawn(read_nuclei_pipe("stdout", child_stdout));
    let stderr_handle = tokio::spawn(read_nuclei_pipe("stderr", child_stderr));

    let wait_result = tokio::select! {
        res = child.wait() => res.map_err(|e| crate::ScanRunnerError::Nuclei(format!("Nuclei wait failed: {}", e))),
        _ = async {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                if NUCLEI_CANCELLED.load(Ordering::SeqCst) { break; }
            }
        } => {
            let _ = child.kill().await;
            let _ = audit_scan_failed(
                pool,
                &authorization.guard,
                parent_audit_id,
                "nuclei_scan_failed",
                "nuclei",
                "Nuclei scan cancelled",
                serde_json::json!({
                    "target_url": target_url,
                    "template_count": template_ids.len(),
                    "duration_ms": start.elapsed().as_millis() as u64,
                    "cancelled": true,
                }),
            )
            .await;
            return Err(crate::ScanRunnerError::Nuclei("Nuclei scan cancelled".into()));
        }
    };
    let exit_status = match wait_result {
        Ok(s) => s,
        Err(e) => {
            let _ = audit_scan_failed(
                pool,
                &authorization.guard,
                parent_audit_id,
                "nuclei_scan_failed",
                "nuclei",
                &e.to_string(),
                serde_json::json!({
                    "target_url": target_url,
                    "template_count": template_ids.len(),
                    "duration_ms": start.elapsed().as_millis() as u64,
                }),
            )
            .await;
            return Err(e);
        }
    };
    let (stdout_bytes, stderr_bytes) =
        match collect_nuclei_output(stdout_handle, stderr_handle).await {
            Ok(output) => output,
            Err(error) => {
                let _ = audit_scan_failed(
                    pool,
                    &authorization.guard,
                    parent_audit_id,
                    "nuclei_scan_failed",
                    "nuclei",
                    &error.to_string(),
                    serde_json::json!({
                        "target_url": target_url,
                        "template_count": template_ids.len(),
                        "duration_ms": start.elapsed().as_millis() as u64,
                        "failure_stage": "output_pump",
                    }),
                )
                .await;
                return Err(error);
            }
        };
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let stderr = String::from_utf8_lossy(&stderr_bytes);

    if !scanner_process_succeeded(exit_status, &stderr) {
        let msg = format!(
            "Nuclei did not complete successfully (status={exit_status}): {}",
            stderr.trim()
        );
        let _ = audit_scan_failed(
            pool,
            &authorization.guard,
            parent_audit_id,
            "nuclei_scan_failed",
            "nuclei",
            &msg,
            serde_json::json!({
                "target_url": target_url,
                "template_count": template_ids.len(),
                "duration_ms": start.elapsed().as_millis() as u64,
            }),
        )
        .await;
        return Err(crate::ScanRunnerError::Nuclei(msg));
    }

    let mut items_found = 0u32;
    let mut items_stored = 0u32;
    let mut errors = Vec::new();

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let result = match parse_nuclei_json_line(trimmed, authorization, template_ids) {
            Ok(result) => result,
            Err(error) => {
                errors.push(format!("Nuclei JSONL record rejected: {error}"));
                continue;
            }
        };

        items_found += 1;
        let cve_id = extract_cve_from_template(&result.template_id)
            .or_else(|| extract_cve_from_tags(result.tags.as_deref()));

        let evidence = serde_json::json!({
            "template_id": &result.template_id,
            "matcher_name": &result.matcher_name,
            "extracted_results": &result.extracted_results,
            "curl_command": &result.curl_command,
            "scan_type": &result.scan_type,
            "references": &result.references,
        });

        let finding_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!(
                "nuclei:{}:{}:{}",
                result.template_id, result.matched_url, target_id
            )
            .as_bytes(),
        );

        let scan_log_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!(
                "nuclei-log:{}:{}:{}",
                result.template_id, result.matched_url, target_id
            )
            .as_bytes(),
        );

        match store_nuclei_hit_guarded(
            pool,
            authorization,
            &NucleiStoredHit {
                finding_id,
                scan_log_id,
                matched_url: &result.matched_url,
                title: &result.title,
                severity: &result.severity,
                description: &result.description,
                template_id: &result.template_id,
                cve_id: cve_id.as_deref(),
                evidence: &evidence,
            },
        )
        .await
        {
            Ok(()) => items_stored += 1,
            Err(error) => errors.push(format!("Failed guarded Nuclei landing: {error}")),
        }

        emit_progress(
            emitter,
            "nuclei",
            "found",
            items_found,
            total,
            &format!(
                "[{}] {} at {}",
                result.severity.to_uppercase(),
                result.title,
                result.matched_url
            ),
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

    audit_scan_completed(
        pool,
        &authorization.guard,
        parent_audit_id,
        "nuclei_scan_completed",
        "nuclei",
        &format!(
            "Nuclei targeted scan on {}: {} templates, {} findings",
            target_url, total, items_found
        ),
        serde_json::json!({
            "target_url": target_url,
            "template_count": total,
            "items_found": items_found,
            "items_stored": items_stored,
            "errors": result.errors.len(),
            "duration_ms": duration_ms,
            "outcome": if result.success { "completed" } else { "partial" },
        }),
    )
    .await?;

    Ok(result)
}

fn validate_nuclei_launch_options(
    template_ids: &[String],
    severity_filter: Option<&[String]>,
    options: &NucleiScanOptions,
) -> crate::ScanRunnerResult<()> {
    if template_ids.is_empty() || template_ids.len() > 512 {
        return Err(crate::ScanRunnerError::Nuclei(
            "targeted Nuclei scan requires 1..=512 explicit template ids".to_string(),
        ));
    }
    if template_ids.iter().any(|value| !safe_filter_token(value)) {
        return Err(crate::ScanRunnerError::Nuclei(
            "template ids may contain only letters, digits, '.', '_' and '-'".to_string(),
        ));
    }
    if severity_filter
        .into_iter()
        .flatten()
        .any(|value| !is_nuclei_severity(value))
    {
        return Err(crate::ScanRunnerError::Nuclei(
            "invalid Nuclei severity filter".to_string(),
        ));
    }
    if options.tags.as_ref().is_some_and(|tags| !tags.is_empty()) {
        return Err(crate::ScanRunnerError::Nuclei(
            "caller-supplied positive tag selection is disabled for explicit targeted scans"
                .to_string(),
        ));
    }
    if options
        .exclude_tags
        .iter()
        .flatten()
        .any(|value| !safe_filter_token(value))
    {
        return Err(crate::ScanRunnerError::Nuclei(
            "Nuclei tag filters may not contain paths, separators, or whitespace".to_string(),
        ));
    }
    if options.template_path.is_some() {
        return Err(crate::ScanRunnerError::Nuclei(
            "caller-supplied template_path is disabled; targeted scans use platform-managed template ids"
                .to_string(),
        ));
    }
    if options.proxy.is_some() {
        return Err(crate::ScanRunnerError::Nuclei(
            "caller-supplied proxy is disabled for authorized scans; use platform settings"
                .to_string(),
        ));
    }
    if options
        .extra_args
        .as_ref()
        .is_some_and(|args| !args.is_empty())
    {
        return Err(crate::ScanRunnerError::Nuclei(
            "caller-supplied Nuclei extra_args are disabled because they can override target/input/output/template/proxy controls"
                .to_string(),
        ));
    }
    Ok(())
}

fn safe_filter_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_nuclei_severity(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "info" | "low" | "medium" | "high" | "critical" | "unknown"
    )
}

fn extract_cve_from_template(template_id: &str) -> Option<String> {
    let upper = template_id.to_uppercase();
    if upper.starts_with("CVE-") {
        Some(upper)
    } else {
        None
    }
}

fn extract_cve_from_tags(tags: Option<&[String]>) -> Option<String> {
    tags?
        .iter()
        .find(|t| t.to_uppercase().starts_with("CVE-"))
        .map(|t| t.to_uppercase())
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use golish_db::repo::scoped::TargetWriteGuard;
    use tokio::io::ReadBuf;

    use super::*;

    fn options() -> NucleiScanOptions {
        NucleiScanOptions {
            rate_limit: None,
            bulk_size: None,
            concurrency: None,
            tags: None,
            exclude_tags: None,
            template_path: None,
            proxy: None,
            timeout: None,
            extra_args: None,
        }
    }

    fn authorization() -> AuthorizedScanTarget {
        crate::authorization::authorize_scan_target_from_guard(
            TargetWriteGuard {
                target_id: Uuid::new_v4(),
                organization_id: Some(Uuid::new_v4()),
                project_path: "/workspace/a".to_string(),
                scope: "in".to_string(),
                name: "https://app.example/".to_string(),
                value: "app.example".to_string(),
                ports: serde_json::json!([]),
            },
            Some("/workspace/a"),
            "https://app.example/",
        )
        .expect("test target should authorize")
    }

    struct FailingReader;

    impl AsyncRead for FailingReader {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Ready(Err(io::Error::other("synthetic read failure")))
        }
    }

    #[test]
    fn caller_cannot_override_nuclei_target_input_output_template_or_proxy() {
        let templates = vec!["CVE-2025-0001".to_string()];

        let mut value = options();
        value.extra_args = Some(vec!["-target".to_string(), "https://foreign/".to_string()]);
        assert!(validate_nuclei_launch_options(&templates, None, &value).is_err());

        let mut value = options();
        value.template_path = Some("../../foreign-template.yaml".to_string());
        assert!(validate_nuclei_launch_options(&templates, None, &value).is_err());

        let mut value = options();
        value.proxy = Some("http://127.0.0.1:8080".to_string());
        assert!(validate_nuclei_launch_options(&templates, None, &value).is_err());

        assert!(validate_nuclei_launch_options(
            &["/tmp/template-id-file".to_string()],
            None,
            &options()
        )
        .is_err());
        assert!(validate_nuclei_launch_options(&["CVE-*".to_string()], None, &options()).is_err());

        let mut value = options();
        value.tags = Some(vec!["cve".to_string()]);
        assert!(validate_nuclei_launch_options(&templates, None, &value).is_err());
    }

    #[test]
    fn valid_nuclei_jsonl_record_satisfies_finding_contract() {
        let authorization = authorization();
        let templates = vec!["CVE-2025-0001".to_string()];
        let result = parse_nuclei_json_line(
            r#"{"template-id":"CVE-2025-0001","info":{"name":"Example finding","severity":"HIGH","description":"proof"},"matched-at":"https://app.example/admin"}"#,
            &authorization,
            &templates,
        )
        .expect("complete in-origin JSONL record should be accepted");

        assert_eq!(result.template_id, "CVE-2025-0001");
        assert_eq!(result.matched_url, "https://app.example/admin");
        assert_eq!(result.title, "Example finding");
        assert_eq!(result.severity, "high");
    }

    #[test]
    fn semantic_or_malformed_jsonl_never_counts_as_clean_finding() {
        let authorization = authorization();
        let templates = vec!["CVE-2025-0001".to_string()];
        let fake_jsonl = [
            r#"{}"#,
            r#"{"template-id":null,"info":null,"matched-at":null}"#,
            r#"{"template-id":"../unsafe","info":{"name":"Finding","severity":"high"},"matched-at":"https://app.example/admin"}"#,
            r#"{"template-id":"CVE-2025-9999","info":{"name":"Finding","severity":"high"},"matched-at":"https://app.example/admin"}"#,
            r#"{"template-id":"CVE-2025-0001","info":{"name":"Finding","severity":"high"}}"#,
            r#"{"template-id":"CVE-2025-0001","matched-at":"https://app.example/admin"}"#,
            r#"{"template-id":"CVE-2025-0001","info":{"severity":"high"},"matched-at":"https://app.example/admin"}"#,
            r#"{"template-id":"CVE-2025-0001","info":{"name":"Finding"},"matched-at":"https://app.example/admin"}"#,
            r#"{"template-id":"CVE-2025-0001","info":{"name":"Finding","severity":"banana"},"matched-at":"https://app.example/admin"}"#,
            r#"{"template-id":"CVE-2025-0001","info":{"name":"Finding","severity":"high"},"matched-at":"https://foreign.example/admin"}"#,
            "not-json",
        ]
        .join("\n");

        let mut findings = 0;
        let mut errors = Vec::new();
        for line in fake_jsonl.lines() {
            match parse_nuclei_json_line(line, &authorization, &templates) {
                Ok(_) => findings += 1,
                Err(error) => errors.push(error),
            }
        }

        assert_eq!(findings, 0);
        assert_eq!(errors.len(), 11);
        assert!(
            !errors.is_empty(),
            "invalid output must prevent clean-empty"
        );
    }

    #[tokio::test]
    async fn nuclei_pipe_read_error_propagates_as_scan_failure() {
        let stdout = tokio::spawn(read_nuclei_pipe("stdout", Some(FailingReader)));
        let stderr = tokio::spawn(read_nuclei_pipe("stderr", Some(tokio::io::empty())));

        let error = collect_nuclei_output(stdout, stderr)
            .await
            .expect_err("pipe read failures must propagate");
        assert!(error.to_string().contains("stdout output read failed"));
    }

    #[tokio::test]
    async fn nuclei_pipe_join_error_propagates_as_scan_failure() {
        let stdout: NucleiPipeTask = tokio::spawn(std::future::pending());
        stdout.abort();
        let stderr = tokio::spawn(read_nuclei_pipe("stderr", Some(tokio::io::empty())));

        let error = collect_nuclei_output(stdout, stderr)
            .await
            .expect_err("pipe task join failures must propagate");
        assert!(error.to_string().contains("stdout output pump task failed"));
    }
}
