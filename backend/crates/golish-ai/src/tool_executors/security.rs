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

            match golish_db::repo::audit::log_operation(
                pool,
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
                        "log_id": entry.id.to_string(),
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

                match golish_db::repo::api_endpoints::insert(
                    pool, target_id, project_path, url, method, path,
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

            match golish_db::repo::js_analysis::insert(
                pool, target_id, project_path, &url, &filename,
                None, None,
                &frameworks, &libraries, &endpoints_found, &secrets_found,
                &comments, source_maps, &risk_summary, &json!({}),
            ).await {
                Ok(result) => {
                    if let Some(ref fp) = file_path_param {
                        let _ = golish_db::repo::js_analysis::update_file_path(pool, result.id, fp).await;
                    }
                    Some((
                        json!({
                            "success": true,
                            "analysis_id": result.id.to_string(),
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

                if golish_db::repo::fingerprints::upsert(
                    pool, target_id, project_path, category, name,
                    version, confidence, &evidence, cpe, &source,
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

            match golish_db::repo::passive_scans::insert(
                pool, target_id, project_path,
                &test_type, &payload, &url, &parameter,
                &result_str, &evidence, &severity,
                &tool_used, &tester, &notes, &json!({}),
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
                            "scan_id": entry.id.to_string(),
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

            let mut data = json!({});

            if include_all || sections.contains(&"assets".to_string()) {
                if let Ok(assets) = golish_db::repo::target_assets::list_by_target(pool, target_id).await {
                    data["assets"] = json!(assets);
                    data["assets_count"] = json!(assets.len());
                }
            }
            if include_all || sections.contains(&"endpoints".to_string()) {
                if let Ok(endpoints) = golish_db::repo::api_endpoints::list_by_target(pool, target_id).await {
                    data["endpoints"] = json!(endpoints);
                    data["endpoints_count"] = json!(endpoints.len());
                }
            }
            if include_all || sections.contains(&"fingerprints".to_string()) {
                if let Ok(fps) = golish_db::repo::fingerprints::list_by_target(pool, target_id).await {
                    data["fingerprints"] = json!(fps);
                }
            }
            if include_all || sections.contains(&"js_analysis".to_string()) {
                if let Ok(results) = golish_db::repo::js_analysis::list_by_target(pool, target_id).await {
                    data["js_analysis"] = json!(results);
                }
            }
            if include_all || sections.contains(&"scan_logs".to_string()) {
                if let Ok(logs) = golish_db::repo::passive_scans::list_by_target(pool, target_id, 100).await {
                    data["scan_logs"] = json!(logs);
                    if let Ok(stats) = golish_db::repo::passive_scans::stats_by_target(pool, target_id).await {
                        data["scan_stats"] = stats;
                    }
                }
            }

            Some((data, true))
        }

        _ => None,
    }
}
