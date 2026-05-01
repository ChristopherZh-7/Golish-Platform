use serde_json::json;
use super::common::{error_result, extract_string_param, ToolResult};

pub async fn execute_security_analysis_tool(
    tool_name: &str,
    args: &serde_json::Value,
    db_tracker: Option<&crate::db_tracking::DbTracker>,
    project_path: Option<&str>,
    session_id: Option<&str>,
) -> Option<ToolResult> {
    let is_sec_tool = matches!(
        tool_name,
        "log_operation" | "discover_apis" | "save_js_analysis"
        | "fingerprint_target" | "log_scan_result" | "query_target_data"
    );
    if !is_sec_tool {
        return None;
    }

    let repo = match db_tracker.and_then(|t| t.repo()) {
        Some(r) => r,
        None => return Some(error_result("Database not available for security analysis tools")),
    };
    let pool = match db_tracker {
        Some(t) => t.pool(),
        None => return Some(error_result("Database not available for security analysis tools")),
    };

    match tool_name {
        "log_operation" => {
            let op_type = extract_string_param(args, &["op_type"])
                .unwrap_or_else(|| "general".to_string());
            let summary = match extract_string_param(args, &["summary"]) {
                Some(s) if !s.is_empty() => s,
                _ => return Some(error_result("log_operation requires a 'summary' parameter")),
            };
            let tool = extract_string_param(args, &["tool_name"]);
            let target_id = extract_string_param(args, &["target_id"])
                .and_then(|s| uuid::Uuid::parse_str(&s).ok());
            let status = extract_string_param(args, &["status"])
                .unwrap_or_else(|| "completed".to_string());
            let detail = args.get("detail").cloned().unwrap_or_else(|| json!({}));

            match crate::db_shim::audit::log_operation(
                repo,
                &summary,
                &op_type,
                &summary,
                project_path,
                "ai",
                target_id,
                session_id,
                tool.as_deref(),
                &status,
                &detail,
            ).await {
                Ok(entry) => Some((
                    json!({
                        "success": true,
                        "log_id": entry.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                        "message": format!("Operation logged: {}", summary),
                    }),
                    true,
                )),
                Err(e) => Some(error_result(format!("Failed to log operation: {}", e))),
            }
        }

        "discover_apis" => {
            let target_id = match extract_string_param(args, &["target_id"])
                .and_then(|s| uuid::Uuid::parse_str(&s).ok())
            {
                Some(id) => id,
                None => return Some(error_result("discover_apis requires a valid 'target_id' UUID")),
            };
            let source = extract_string_param(args, &["source"])
                .unwrap_or_else(|| "ai".to_string());
            let endpoints = match args.get("endpoints").and_then(|v| v.as_array()) {
                Some(arr) => arr.clone(),
                None => return Some(error_result("discover_apis requires an 'endpoints' array")),
            };

            let mut saved = 0u32;
            let mut errors = Vec::new();
            for ep in &endpoints {
                let url = ep.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let method = ep.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
                let path = ep.get("path").and_then(|v| v.as_str()).unwrap_or("/");
                let params = ep.get("params").cloned().unwrap_or_else(|| json!([]));
                let auth_type = ep.get("auth_type").and_then(|v| v.as_str());
                let risk_level = ep.get("risk_level").and_then(|v| v.as_str()).unwrap_or("unknown");

                match repo.api_endpoints_insert(
                    target_id, project_path, &url, &method, &path,
                    &params, &json!({}), auth_type, &source, risk_level,
                ).await {
                    Ok(_) => saved += 1,
                    Err(e) => errors.push(format!("{}: {}", url, e)),
                }
            }

            Some((
                json!({
                    "success": errors.is_empty(),
                    "saved": saved,
                    "total": endpoints.len(),
                    "errors": errors,
                }),
                errors.is_empty(),
            ))
        }

        "save_js_analysis" => {
            let target_id = match extract_string_param(args, &["target_id"])
                .and_then(|s| uuid::Uuid::parse_str(&s).ok())
            {
                Some(id) => id,
                None => return Some(error_result("save_js_analysis requires a valid 'target_id' UUID")),
            };
            let url = match extract_string_param(args, &["url"]) {
                Some(u) if !u.is_empty() => u,
                _ => return Some(error_result("save_js_analysis requires a 'url' parameter")),
            };
            let filename = extract_string_param(args, &["filename"]).unwrap_or_default();
            let frameworks = args.get("frameworks").cloned().unwrap_or_else(|| json!([]));
            let libraries = args.get("libraries").cloned().unwrap_or_else(|| json!([]));
            let endpoints_found = args.get("endpoints_found").cloned().unwrap_or_else(|| json!([]));
            let secrets_found = args.get("secrets_found").cloned().unwrap_or_else(|| json!([]));
            let comments = args.get("comments").cloned().unwrap_or_else(|| json!([]));
            let source_maps = args.get("source_maps").and_then(|v| v.as_bool()).unwrap_or(false);
            let risk_summary = extract_string_param(args, &["risk_summary"]).unwrap_or_default();

            let file_path_param = extract_string_param(args, &["file_path"]);

            let analysis = json!({
                "frameworks": frameworks,
                "libraries": libraries,
                "endpoints_found": endpoints_found,
                "secrets_found": secrets_found,
                "comments": comments,
                "source_maps": source_maps,
                "risk_summary": risk_summary,
            });
            match repo.js_analysis_insert(
                target_id, project_path.unwrap_or(""), &url, &filename, &analysis,
            ).await {
                Ok(result) => {
                    if let Some(ref fp) = file_path_param {
                        let id = result.get("id").and_then(|v| v.as_str())
                            .and_then(|s| uuid::Uuid::parse_str(s).ok());
                        if let Some(id) = id {
                            let _ = repo.js_analysis_update_file_path(id, fp).await;
                        }
                    }
                    Some((
                        json!({
                            "success": true,
                            "analysis_id": result.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                            "file_path": file_path_param,
                            "frameworks_count": frameworks.as_array().map(|a| a.len()).unwrap_or(0),
                            "endpoints_count": endpoints_found.as_array().map(|a| a.len()).unwrap_or(0),
                            "secrets_count": secrets_found.as_array().map(|a| a.len()).unwrap_or(0),
                        }),
                        true,
                    ))
                },
                Err(e) => Some(error_result(format!("Failed to save JS analysis: {}", e))),
            }
        }

        "fingerprint_target" => {
            let target_id = match extract_string_param(args, &["target_id"])
                .and_then(|s| uuid::Uuid::parse_str(&s).ok())
            {
                Some(id) => id,
                None => return Some(error_result("fingerprint_target requires a valid 'target_id' UUID")),
            };
            let source = extract_string_param(args, &["source"])
                .unwrap_or_else(|| "ai".to_string());
            let fps = match args.get("fingerprints").and_then(|v| v.as_array()) {
                Some(arr) => arr.clone(),
                None => return Some(error_result("fingerprint_target requires a 'fingerprints' array")),
            };

            let mut saved = 0u32;
            for fp in &fps {
                let category = fp.get("category").and_then(|v| v.as_str()).unwrap_or("technology");
                let name = match fp.get("name").and_then(|v| v.as_str()) {
                    Some(n) if !n.is_empty() => n,
                    _ => continue,
                };
                let version = fp.get("version").and_then(|v| v.as_str());
                let confidence = fp.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
                let evidence = fp.get("evidence").cloned().unwrap_or_else(|| json!([]));
                let cpe = fp.get("cpe").and_then(|v| v.as_str());

                if repo.fingerprints_upsert(
                    target_id, project_path.unwrap_or(""), category, name,
                    version, confidence as f64, Some(&evidence),
                ).await.is_ok() {
                    saved += 1;
                }
            }

            Some((
                json!({
                    "success": true,
                    "saved": saved,
                    "total": fps.len(),
                }),
                true,
            ))
        }

        "log_scan_result" => {
            let target_id = match extract_string_param(args, &["target_id"])
                .and_then(|s| uuid::Uuid::parse_str(&s).ok())
            {
                Some(id) => id,
                None => return Some(error_result("log_scan_result requires a valid 'target_id' UUID")),
            };
            let test_type = match extract_string_param(args, &["test_type"]) {
                Some(t) if !t.is_empty() => t,
                _ => return Some(error_result("log_scan_result requires a 'test_type' parameter")),
            };
            let result_str = extract_string_param(args, &["result"])
                .unwrap_or_else(|| "pending".to_string());
            let payload = extract_string_param(args, &["payload"]).unwrap_or_default();
            let url = extract_string_param(args, &["url"]).unwrap_or_default();
            let parameter = extract_string_param(args, &["parameter"]).unwrap_or_default();
            let evidence = extract_string_param(args, &["evidence"]).unwrap_or_default();
            let severity = extract_string_param(args, &["severity"]).unwrap_or_else(|| "info".to_string());
            let tool_used = extract_string_param(args, &["tool_used"]).unwrap_or_default();
            let tester = extract_string_param(args, &["tester"]).unwrap_or_else(|| "ai".to_string());
            let notes = extract_string_param(args, &["notes"]).unwrap_or_default();

            let findings = json!({
                "test_type": test_type,
                "payload": payload,
                "url": url,
                "parameter": parameter,
                "result": result_str,
                "evidence": evidence,
                "tool_used": tool_used,
                "tester": tester,
                "notes": notes,
            });
            match repo.passive_scans_insert(
                target_id, project_path.unwrap_or(""),
                &test_type, &tool_used, &findings, Some(&evidence), &severity,
            ).await {
                Ok(entry) => {
                    let msg = if result_str == "vulnerable" || result_str == "potential" {
                        format!("⚠ {} test on {} — {}", test_type, url, result_str)
                    } else {
                        format!("{} test on {} — {}", test_type, url, result_str)
                    };
                    Some((
                        json!({
                            "success": true,
                            "scan_id": entry.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                            "message": msg,
                        }),
                        true,
                    ))
                }
                Err(e) => Some(error_result(format!("Failed to log scan result: {}", e))),
            }
        }

        "query_target_data" => {
            let target_id = match extract_string_param(args, &["target_id"])
                .and_then(|s| uuid::Uuid::parse_str(&s).ok())
            {
                Some(id) => id,
                None => return Some(error_result("query_target_data requires a valid 'target_id' UUID")),
            };

            let sections: Vec<String> = args
                .get("sections")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_else(|| vec!["all".to_string()]);
            let include_all = sections.contains(&"all".to_string());

            let data = match repo.query_target_data(target_id, &sections).await {
                Ok(d) => d,
                Err(e) => return Some(error_result(format!("Failed to query target data: {}", e))),
            };

            Some((data, true))
        }

        _ => None,
    }
}
