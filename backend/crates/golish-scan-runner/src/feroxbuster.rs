//! feroxbuster (directory busting) over caller-supplied seed paths.

use std::path::{Path, PathBuf};

use golish_core::EventEmitterHandle;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::authorization::{
    after_successful_validation, url_has_authorized_origin, AuthorizedScanTarget,
};
use crate::helpers::{
    audit_scan_completed, audit_scan_started, emit_progress, scanner_process_succeeded, which_tool,
};
use crate::storage::ScanStorage;
use crate::types::ScanResult;

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

pub async fn run_feroxbuster(
    pool: &sqlx::PgPool,
    storage: &dyn ScanStorage,
    emitter: Option<&EventEmitterHandle>,
    authorization: &AuthorizedScanTarget,
    base_paths: &[String],
    options: Option<FeroxScanOptions>,
) -> crate::ScanRunnerResult<ScanResult> {
    let start = std::time::Instant::now();
    let target_url = authorization.requested_url.as_str();
    let target_id = authorization.guard.target_id;
    let project_path = authorization.guard.project_path.as_str();

    let mut opts = options.unwrap_or(FeroxScanOptions {
        depth: Some(3),
        threads: Some(50),
        wordlist: None,
        extensions: None,
        status_codes: None,
        timeout: Some(10),
    });
    opts.wordlist = resolve_workspace_wordlist(project_path, opts.wordlist.as_deref())?;
    let urls_to_scan = build_authorized_scan_urls(authorization, base_paths)?;

    let ferox_path = match which_tool("feroxbuster").await {
        Some(p) => p,
        None => {
            let msg = "feroxbuster not found. Install via: brew install feroxbuster or cargo install feroxbuster";
            return Err(crate::ScanRunnerError::Feroxbuster(msg.into()));
        }
    };

    golish_db::repo::scoped::validate_target_write_guard(pool, &authorization.guard).await?;
    let parent_audit_id = audit_scan_started(
        pool,
        &authorization.guard,
        "feroxbuster_scan_started",
        "feroxbuster",
        target_url,
        serde_json::json!({
            "base_paths_count": base_paths.len(),
            "base_paths_sample": base_paths.iter().take(20).cloned().collect::<Vec<_>>(),
            "project_path": project_path,
            "exact_origin": authorization.exact_origin,
        }),
    )
    .await?;

    let total_urls = urls_to_scan.len() as u32;
    let mut all_items_found = 0u32;
    let mut all_items_stored = 0u32;
    let mut all_errors = Vec::new();

    for (idx, scan_url) in urls_to_scan.iter().enumerate() {
        emit_progress(
            emitter,
            "feroxbuster",
            "scanning",
            idx as u32,
            total_urls,
            &format!("Scanning {} ({}/{})", scan_url, idx + 1, total_urls),
        );

        let mut args = vec![
            "--url".to_string(),
            scan_url.clone(),
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
                args.extend_from_slice(&[
                    "--status-codes".to_string(),
                    codes
                        .iter()
                        .map(|c| c.to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                ]);
            }
        }
        if let Some(t) = opts.timeout {
            args.extend_from_slice(&["--timeout".to_string(), t.to_string()]);
        }

        let output = match after_successful_validation(
            async {
                golish_db::repo::scoped::validate_target_write_guard(pool, &authorization.guard)
                    .await
                    .map_err(crate::ScanRunnerError::from)
            },
            || async {
                tokio::process::Command::new(&ferox_path)
                    .args(&args)
                    .output()
                    .await
                    .map_err(crate::ScanRunnerError::from)
            },
        )
        .await
        {
            Ok(o) => o,
            Err(e) => {
                all_errors.push(format!("feroxbuster failed for {}: {}", scan_url, e));
                break;
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !scanner_process_succeeded(output.status, &stderr) {
            all_errors.push(format!(
                "feroxbuster failed for {scan_url} (status={}): {}",
                output.status,
                stderr.trim()
            ));
            continue;
        }

        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let result: FeroxResult = match serde_json::from_str(trimmed) {
                Ok(r) => r,
                Err(error) => {
                    all_errors.push(format!("feroxbuster JSONL parse failed: {error}"));
                    continue;
                }
            };

            if result.result_type.as_deref() != Some("response") {
                continue;
            }

            let url = match &result.url {
                Some(u) => u.clone(),
                None => continue,
            };
            if !url_has_authorized_origin(authorization, &url) {
                all_errors.push(format!(
                    "feroxbuster result escaped the authorized exact origin: {url}"
                ));
                continue;
            }
            let status = result.status.unwrap_or(0) as i32;

            all_items_found += 1;

            let store_result = storage
                .store_directory_entry(
                    pool,
                    &authorization.guard,
                    &url,
                    Some(status),
                    result.content_length,
                    result.line_count,
                    result.word_count,
                    "feroxbuster",
                )
                .await;

            match store_result {
                Ok(_) => all_items_stored += 1,
                Err(e) => all_errors.push(format!("Store failed for {}: {}", url, e)),
            }

            if is_sensitive_path(&url) {
                let finding_id = Uuid::new_v5(
                    &Uuid::NAMESPACE_URL,
                    format!("ferox:sensitive:{}:{}", url, target_id).as_bytes(),
                );
                if let Err(error) =
                    store_sensitive_finding_guarded(pool, authorization, finding_id, &url, status)
                        .await
                {
                    all_errors.push(format!(
                        "Failed guarded feroxbuster finding landing for {url}: {error}"
                    ));
                }

                emit_progress(
                    emitter,
                    "feroxbuster",
                    "sensitive",
                    all_items_found,
                    0,
                    &format!("Sensitive: {} ({})", extract_path(&url), status),
                );
            }
        }
    }

    emit_progress(
        emitter,
        "feroxbuster",
        "done",
        all_items_stored,
        all_items_found,
        &format!(
            "Found {} paths, {} stored",
            all_items_found, all_items_stored
        ),
    );

    let duration_ms = start.elapsed().as_millis() as u64;
    let result = ScanResult {
        tool: "feroxbuster".to_string(),
        success: all_errors.is_empty(),
        items_found: all_items_found,
        items_stored: all_items_stored,
        errors: all_errors,
        duration_ms,
    };

    audit_scan_completed(
        pool,
        &authorization.guard,
        parent_audit_id,
        "feroxbuster_scan_completed",
        "feroxbuster",
        &format!(
            "feroxbuster on {}: {} paths found, {} URLs scanned",
            target_url, all_items_found, total_urls
        ),
        serde_json::json!({
            "target_url": target_url,
            "template_count": total_urls,
            "items_found": all_items_found,
            "items_stored": all_items_stored,
            "errors": result.errors.len(),
            "duration_ms": duration_ms,
            "outcome": if result.success { "completed" } else { "partial" },
        }),
    )
    .await?;

    Ok(result)
}

async fn store_sensitive_finding_guarded(
    pool: &sqlx::PgPool,
    authorization: &AuthorizedScanTarget,
    finding_id: Uuid,
    url: &str,
    status: i32,
) -> crate::ScanRunnerResult<()> {
    let operation_id =
        golish_core::current_agent_tool_context().and_then(|context| context.operation_id);
    let requested_context = if operation_id.is_some() {
        golish_pentest_domain::FindingWriteContext::HarnessLegacy
    } else {
        golish_pentest_domain::FindingWriteContext::LegacyNonHarness
    };
    let write_context =
        golish_db::repo::findings::authorize_legacy_write(pool, requested_context, operation_id)
            .await?;
    let mut tx = pool.begin().await?;
    golish_db::repo::scoped::lock_target_write_guard(&mut tx, &authorization.guard).await?;
    golish_db::repo::findings::insert_legacy_with_executor(
        &mut *tx,
        write_context,
        &golish_db::repo::findings::LegacyFindingWrite {
            id: finding_id,
            title: format!("Sensitive file/directory: {}", extract_path(url)),
            severity: classify_sensitive_severity(url).to_ascii_lowercase(),
            cvss: None,
            url: url.to_string(),
            target: url.to_string(),
            target_id: Some(authorization.guard.target_id),
            description: format!(
                "Directory enumeration discovered a potentially sensitive resource at {} (HTTP {})",
                url, status
            ),
            steps: String::new(),
            remediation: String::new(),
            evidence: serde_json::json!([]),
            tool: "feroxbuster".to_string(),
            template: String::new(),
            refs: serde_json::json!([]),
            source: "feroxbuster".to_string(),
            project_path: Some(authorization.guard.project_path.clone()),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

fn build_authorized_scan_urls(
    authorization: &AuthorizedScanTarget,
    base_paths: &[String],
) -> crate::ScanRunnerResult<Vec<String>> {
    if base_paths.len() > 100 {
        return Err(crate::ScanRunnerError::Feroxbuster(
            "feroxbuster accepts at most 100 base paths".to_string(),
        ));
    }
    if base_paths.is_empty() {
        return Ok(vec![authorization.requested_url.clone()]);
    }
    let base = url::Url::parse(&authorization.requested_url).map_err(|error| {
        crate::ScanRunnerError::Feroxbuster(format!("invalid authorized target URL: {error}"))
    })?;
    let mut urls = Vec::with_capacity(base_paths.len());
    for raw in base_paths {
        let raw = raw.trim();
        if raw.is_empty() || raw.contains('\0') {
            return Err(crate::ScanRunnerError::Feroxbuster(
                "feroxbuster base paths must be non-empty URL/path strings".to_string(),
            ));
        }
        let joined = match url::Url::parse(raw) {
            Ok(url) => url,
            Err(url::ParseError::RelativeUrlWithoutBase) => base.join(raw).map_err(|error| {
                crate::ScanRunnerError::Feroxbuster(format!(
                    "invalid feroxbuster base path {raw:?}: {error}"
                ))
            })?,
            Err(error) => {
                return Err(crate::ScanRunnerError::Feroxbuster(format!(
                    "invalid feroxbuster base URL {raw:?}: {error}"
                )))
            }
        };
        let joined = joined.to_string();
        if !url_has_authorized_origin(authorization, &joined) {
            return Err(crate::ScanRunnerError::Feroxbuster(format!(
                "feroxbuster base path escaped the authorized exact origin: {raw}"
            )));
        }
        urls.push(joined);
    }
    urls.sort();
    urls.dedup();
    Ok(urls)
}

fn resolve_workspace_wordlist(
    project_path: &str,
    requested: Option<&str>,
) -> crate::ScanRunnerResult<Option<String>> {
    let Some(requested) = requested else {
        return Ok(None);
    };
    let requested = requested.trim();
    if requested.is_empty() {
        return Err(crate::ScanRunnerError::Feroxbuster(
            "feroxbuster wordlist must not be empty".to_string(),
        ));
    }
    let project = Path::new(project_path).canonicalize().map_err(|error| {
        crate::ScanRunnerError::Feroxbuster(format!(
            "cannot resolve target workspace for wordlist authorization: {error}"
        ))
    })?;
    let raw = PathBuf::from(requested);
    let candidate = if raw.is_absolute() {
        raw
    } else {
        project.join(raw)
    };
    let candidate = candidate.canonicalize().map_err(|error| {
        crate::ScanRunnerError::Feroxbuster(format!("cannot resolve feroxbuster wordlist: {error}"))
    })?;
    let allowed = candidate == project.join("1.txt")
        || candidate.starts_with(project.join(".golish").join("wordlists"));
    if !allowed || !candidate.is_file() {
        return Err(crate::ScanRunnerError::Feroxbuster(
            "feroxbuster wordlist must be workspace/1.txt or a regular file under workspace/.golish/wordlists"
                .to_string(),
        ));
    }
    Ok(Some(candidate.to_string_lossy().to_string()))
}

fn is_sensitive_path(url: &str) -> bool {
    let path = extract_path(url).to_lowercase();
    let sensitive_patterns = [
        ".env",
        ".git",
        ".svn",
        ".htaccess",
        ".htpasswd",
        "wp-config",
        "config.php",
        "config.yml",
        "config.json",
        "backup",
        ".bak",
        ".sql",
        ".dump",
        "admin",
        "phpmyadmin",
        "adminer",
        ".DS_Store",
        "Thumbs.db",
        "web.config",
        "server-status",
        "server-info",
        ".aws",
        "credentials",
        "id_rsa",
        ".ssh",
        "phpinfo",
        "info.php",
        "debug",
        ".debug",
        "trace",
        "swagger",
        "api-docs",
        "graphql",
    ];
    sensitive_patterns.iter().any(|p| path.contains(p))
}

fn classify_sensitive_severity(url: &str) -> &'static str {
    let path = extract_path(url).to_lowercase();
    if path.contains(".env")
        || path.contains("credentials")
        || path.contains("id_rsa")
        || path.contains(".ssh")
        || path.contains("wp-config")
    {
        "high"
    } else if path.contains(".git")
        || path.contains("backup")
        || path.contains(".sql")
        || path.contains("phpinfo")
        || path.contains("config")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::authorize_scan_target_from_guard;

    fn authorization(project_path: &str) -> AuthorizedScanTarget {
        authorize_scan_target_from_guard(
            golish_db::repo::scoped::TargetWriteGuard {
                target_id: Uuid::new_v4(),
                organization_id: Some(Uuid::new_v4()),
                project_path: project_path.to_string(),
                scope: "in".to_string(),
                name: "https://app.example/".to_string(),
                value: "https://app.example/".to_string(),
                ports: serde_json::json!([]),
            },
            Some(project_path),
            "https://app.example/root/",
        )
        .unwrap()
    }

    #[test]
    fn base_paths_cannot_add_a_foreign_command_target() {
        let authorization = authorization("/workspace/a");
        assert!(build_authorized_scan_urls(
            &authorization,
            &["https://foreign.example/admin".to_string()]
        )
        .is_err());
        assert!(build_authorized_scan_urls(
            &authorization,
            &["//foreign.example/admin".to_string()]
        )
        .is_err());
        assert_eq!(
            build_authorized_scan_urls(&authorization, &["admin".to_string()]).unwrap(),
            vec!["https://app.example/root/admin"]
        );
    }

    #[test]
    fn wordlist_cannot_traverse_or_escape_the_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let allowed_dir = workspace.join(".golish/wordlists");
        std::fs::create_dir_all(&allowed_dir).unwrap();
        let allowed = allowed_dir.join("paths.txt");
        std::fs::write(&allowed, "admin\n").unwrap();
        let outside = temp.path().join("outside.txt");
        std::fs::write(&outside, "secret\n").unwrap();

        assert!(resolve_workspace_wordlist(
            workspace.to_string_lossy().as_ref(),
            Some(".golish/wordlists/paths.txt")
        )
        .is_ok());
        assert!(resolve_workspace_wordlist(
            workspace.to_string_lossy().as_ref(),
            Some("../outside.txt")
        )
        .is_err());
        assert!(resolve_workspace_wordlist(
            workspace.to_string_lossy().as_ref(),
            Some(outside.to_string_lossy().as_ref())
        )
        .is_err());
    }
}
