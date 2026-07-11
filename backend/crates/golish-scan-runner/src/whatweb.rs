//! WhatWeb fingerprinting scanner.

use std::collections::HashMap;

use golish_core::EventEmitterHandle;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::authorization::{
    after_successful_validation, url_has_authorized_origin, AuthorizedScanTarget,
};
use crate::helpers::{
    audit_scan_completed, audit_scan_failed, audit_scan_started, emit_progress,
    scanner_process_succeeded, which_tool,
};
use crate::types::ScanResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatWebOptions {
    pub aggression: Option<u32>,
    pub plugins: Option<Vec<String>>,
    pub user_agent: Option<String>,
    pub proxy: Option<String>,
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
    authorization: &AuthorizedScanTarget,
) -> (u32, Vec<String>) {
    let mut errors = Vec::new();
    let mut writes = Vec::new();

    let results: Vec<WhatWebResult> = match serde_json::from_str(json_output) {
        Ok(v) => v,
        Err(e) => {
            errors.push(format!("WhatWeb JSON parse failed: {}", e));
            return (0, errors);
        }
    };

    for result in &results {
        let Some(result_target) = result.target.as_deref() else {
            errors.push("WhatWeb result omitted its target URL".to_string());
            continue;
        };
        if !url_has_authorized_origin(authorization, result_target) {
            errors.push(format!(
                "WhatWeb result escaped the authorized exact origin: {result_target}"
            ));
            continue;
        }
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

            writes.push(golish_db::repo::fingerprints::FingerprintWrite {
                category,
                name: plugin_name.clone(),
                version,
                confidence,
                evidence,
                cpe: None,
                source: "whatweb".to_string(),
            });
        }
    }

    match golish_db::repo::fingerprints::upsert_batch_guarded(pool, &authorization.guard, &writes)
        .await
    {
        Ok(rows) => (rows.len() as u32, errors),
        Err(error) => {
            errors.push(format!("Failed guarded WhatWeb landing: {error}"));
            (0, errors)
        }
    }
}

fn infer_whatweb_category(plugin_name: &str) -> String {
    let lower = plugin_name.to_lowercase();
    if lower.contains("php")
        || lower.contains("asp")
        || lower.contains("python")
        || lower.contains("ruby")
        || lower.contains("java")
        || lower.contains("node")
    {
        "language".to_string()
    } else if lower.contains("apache")
        || lower.contains("nginx")
        || lower.contains("iis")
        || lower.contains("lighttpd")
        || lower.contains("tomcat")
        || lower.contains("caddy")
    {
        "web_server".to_string()
    } else if lower.contains("wordpress")
        || lower.contains("drupal")
        || lower.contains("joomla")
        || lower.contains("shopify")
        || lower.contains("magento")
    {
        "cms".to_string()
    } else if lower.contains("jquery")
        || lower.contains("react")
        || lower.contains("angular")
        || lower.contains("vue")
        || lower.contains("bootstrap")
    {
        "frontend_framework".to_string()
    } else if lower.contains("spring")
        || lower.contains("django")
        || lower.contains("laravel")
        || lower.contains("express")
        || lower.contains("flask")
        || lower.contains("rails")
    {
        "backend_framework".to_string()
    } else if lower.contains("mysql")
        || lower.contains("postgres")
        || lower.contains("mongo")
        || lower.contains("redis")
        || lower.contains("sqlite")
    {
        "database".to_string()
    } else if lower.contains("cdn")
        || lower.contains("cloudflare")
        || lower.contains("akamai")
        || lower.contains("fastly")
    {
        "cdn".to_string()
    } else if lower.contains("waf") || lower.contains("firewall") || lower.contains("mod_security")
    {
        "security".to_string()
    } else if lower.contains("os")
        || lower.contains("linux")
        || lower.contains("windows")
        || lower.contains("ubuntu")
        || lower.contains("centos")
        || lower.contains("debian")
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

/// Run WhatWeb fingerprinting scan.
pub async fn run_whatweb(
    pool: &PgPool,
    emitter: Option<&EventEmitterHandle>,
    authorization: &AuthorizedScanTarget,
    options: Option<WhatWebOptions>,
) -> crate::ScanRunnerResult<ScanResult> {
    let start = std::time::Instant::now();
    let target_url = authorization.requested_url.as_str();
    let project_path = authorization.guard.project_path.as_str();

    let opts = options.unwrap_or(WhatWebOptions {
        aggression: None,
        plugins: None,
        user_agent: None,
        proxy: None,
        extra_args: None,
    });
    validate_whatweb_options(&opts)?;

    let whatweb_path = match which_tool("whatweb").await {
        Some(p) => p,
        None => {
            return Err(crate::ScanRunnerError::WhatWeb(
                "WhatWeb not found. Install via: brew install whatweb or gem install whatweb"
                    .into(),
            ));
        }
    };

    let args = build_whatweb_args(target_url, &opts);

    golish_db::repo::scoped::validate_target_write_guard(pool, &authorization.guard).await?;
    let parent_audit_id = audit_scan_started(
        pool,
        &authorization.guard,
        "whatweb_scan_started",
        "whatweb",
        target_url,
        serde_json::json!({
            "project_path": project_path,
            "exact_origin": authorization.exact_origin,
        }),
    )
    .await?;

    emit_progress(
        emitter,
        "whatweb",
        "running",
        0,
        1,
        &format!("Scanning {}", target_url),
    );

    let output = match after_successful_validation(
        async {
            golish_db::repo::scoped::validate_target_write_guard(pool, &authorization.guard)
                .await
                .map_err(crate::ScanRunnerError::from)
        },
        || async {
            tokio::process::Command::new(&whatweb_path)
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
            let msg = format!("WhatWeb execution failed: {}", e);
            let _ = audit_scan_failed(
                pool,
                &authorization.guard,
                parent_audit_id,
                "whatweb_scan_failed",
                "whatweb",
                &msg,
                serde_json::json!({
                    "target_url": target_url,
                    "duration_ms": start.elapsed().as_millis() as u64,
                }),
            )
            .await;
            return Err(crate::ScanRunnerError::WhatWeb(msg));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !scanner_process_succeeded(output.status, &stderr) || stdout.trim().is_empty() {
        let msg = format!(
            "WhatWeb did not complete successfully (status={}): {}",
            output.status,
            stderr.trim()
        );
        let _ = audit_scan_failed(
            pool,
            &authorization.guard,
            parent_audit_id,
            "whatweb_scan_failed",
            "whatweb",
            &msg,
            serde_json::json!({
                "target_url": target_url,
                "duration_ms": start.elapsed().as_millis() as u64,
            }),
        )
        .await;
        return Err(crate::ScanRunnerError::WhatWeb(msg));
    }

    emit_progress(emitter, "whatweb", "parsing", 1, 2, "Parsing results...");

    let (stored, errors) = parse_whatweb_and_store(pool, &stdout, authorization).await;

    emit_progress(
        emitter,
        "whatweb",
        "done",
        1,
        1,
        &format!("Found {} technologies", stored),
    );

    let duration_ms = start.elapsed().as_millis() as u64;
    let result = ScanResult {
        tool: "whatweb".to_string(),
        success: errors.is_empty(),
        items_found: stored + errors.len() as u32,
        items_stored: stored,
        errors,
        duration_ms,
    };

    audit_scan_completed(
        pool,
        &authorization.guard,
        parent_audit_id,
        "whatweb_scan_completed",
        "whatweb",
        &format!("WhatWeb scan on {}: {} techs found", target_url, stored),
        serde_json::json!({
            "target_url": target_url,
            "template_count": 0,
            "items_found": result.items_found,
            "items_stored": result.items_stored,
            "errors": result.errors.len(),
            "duration_ms": duration_ms,
        }),
    )
    .await?;

    Ok(result)
}

fn build_whatweb_args(target_url: &str, options: &WhatWebOptions) -> Vec<String> {
    let mut args = vec![
        "--color=never".to_string(),
        "--log-json=-".to_string(),
        "--quiet".to_string(),
        // WhatWeb defaults to following redirects across sites.  Active scan
        // authorization is one exact Web Origin, so redirects must be disabled
        // in the fixed recipe rather than filtered only after the request.
        "--follow-redirect=never".to_string(),
        "--max-redirects=0".to_string(),
    ];

    if let Some(aggression) = options.aggression {
        args.push(format!("--aggression={}", aggression.clamp(1, 4)));
    }
    if let Some(plugins) = &options.plugins {
        if !plugins.is_empty() {
            args.push(format!("--plugins={}", plugins.join(",")));
        }
    }
    if let Some(user_agent) = &options.user_agent {
        args.push(format!("--user-agent={user_agent}"));
    }
    args.push(target_url.to_string());
    args
}

fn validate_whatweb_options(options: &WhatWebOptions) -> crate::ScanRunnerResult<()> {
    if options
        .proxy
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return Err(crate::ScanRunnerError::WhatWeb(
            "caller-supplied proxy is disabled for authorized scans; use platform settings"
                .to_string(),
        ));
    }
    if options
        .extra_args
        .as_ref()
        .is_some_and(|args| !args.is_empty())
    {
        return Err(crate::ScanRunnerError::WhatWeb(
            "caller-supplied WhatWeb extra_args are disabled because they can override target/input/output/proxy controls"
                .to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn caller_cannot_override_whatweb_target_or_proxy_surface() {
        let options = WhatWebOptions {
            aggression: Some(1),
            plugins: None,
            user_agent: None,
            proxy: Some("http://127.0.0.1:8080".to_string()),
            extra_args: None,
        };
        assert!(validate_whatweb_options(&options).is_err());

        let options = WhatWebOptions {
            proxy: None,
            extra_args: Some(vec!["--input-file=/tmp/foreign".to_string()]),
            ..options
        };
        assert!(validate_whatweb_options(&options).is_err());
    }

    #[tokio::test]
    async fn fake_launch_receives_one_non_redirecting_whatweb_recipe() {
        let options = WhatWebOptions {
            aggression: Some(1),
            plugins: Some(vec!["HTTPServer".to_string()]),
            user_agent: None,
            proxy: None,
            extra_args: None,
        };
        validate_whatweb_options(&options).unwrap();
        let args = build_whatweb_args("https://app.example/", &options);
        let launches = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let captured = Arc::clone(&launches);

        after_successful_validation(async { Ok::<(), &'static str>(()) }, move || async move {
            captured.lock().unwrap().push(args);
            Ok::<(), &'static str>(())
        })
        .await
        .unwrap();

        let launched = launches.lock().unwrap();
        assert_eq!(launched.len(), 1);
        assert_eq!(
            launched[0]
                .iter()
                .filter(|arg| arg.as_str() == "--follow-redirect=never")
                .count(),
            1
        );
        assert_eq!(
            launched[0]
                .iter()
                .filter(|arg| arg.as_str() == "--max-redirects=0")
                .count(),
            1
        );
        assert_eq!(launched[0].last().unwrap(), "https://app.example/");
    }
}
