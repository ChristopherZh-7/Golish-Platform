use super::common::{error_result, extract_string_param, ToolResult};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn in_scope_rows_own_target(rows: &[Value], target_id: uuid::Uuid) -> bool {
    rows.iter().any(|row| {
        row.get("target_id")
            .and_then(Value::as_str)
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            == Some(target_id)
    })
}

async fn mutation_target_is_owned(
    repo: &dyn crate::db_traits::DbRepoProvider,
    target_id: uuid::Uuid,
    harness_org_id: Option<uuid::Uuid>,
) -> std::result::Result<bool, String> {
    repo.in_scope_targets(harness_org_id)
        .await
        .map(|rows| in_scope_rows_own_target(&rows, target_id))
        .map_err(|error| format!("could not verify target ownership: {error}"))
}

pub async fn execute_security_analysis_tool(
    tool_name: &str,
    args: &serde_json::Value,
    db_tracker: Option<&crate::db_tracking::DbTracker>,
    project_path: Option<&str>,
    session_id: Option<&str>,
    // Engagement-org isolation (设计 2026-06-15): the scoping-confirmed engagement
    // root org. `list_in_scope_targets` confines its listing to this org's subtree
    // so a sibling engagement's targets left in the same workspace never surface.
    harness_org_id: Option<uuid::Uuid>,
    harness_stage: Option<crate::harness::StageKind>,
    harness_operation_id: Option<uuid::Uuid>,
) -> Option<ToolResult> {
    let is_sec_tool = matches!(
        tool_name,
        "log_operation"
            | "discover_apis"
            | "save_js_analysis"
            | "fingerprint_target"
            | "log_scan_result"
            | "query_target_data"
            | "list_in_scope_targets"
            | "list_attack_surface_seeds"
            | "list_enumeration_web_roots"
            | "stage_worklist_next"
            | "stage_worklist_status"
            | "check_stage_asset_coverage"
            | "list_recent_evidence"
    );
    if !is_sec_tool {
        return None;
    }

    if harness_stage.is_some()
        && matches!(
            tool_name,
            "log_operation"
                | "discover_apis"
                | "save_js_analysis"
                | "fingerprint_target"
                | "log_scan_result"
        )
    {
        return Some(error_result(format!(
            "legacy mutation tool '{tool_name}' is disabled during harness stages; use the stage-specific guarded producer"
        )));
    }

    let repo = match db_tracker.and_then(|t| t.repo()) {
        Some(r) => r,
        None => {
            return Some(error_result(
                "Database not available for security analysis tools",
            ))
        }
    };
    match tool_name {
        "log_operation" => {
            let op_type =
                extract_string_param(args, &["op_type"]).unwrap_or_else(|| "general".to_string());
            let summary = match extract_string_param(args, &["summary"]) {
                Some(s) if !s.is_empty() => s,
                _ => return Some(error_result("log_operation requires a 'summary' parameter")),
            };
            let tool = extract_string_param(args, &["tool_name"]);
            let target_id = extract_string_param(args, &["target_id"])
                .and_then(|s| uuid::Uuid::parse_str(&s).ok());
            if let Some(target_id) = target_id {
                match mutation_target_is_owned(repo, target_id, harness_org_id).await {
                    Ok(true) => {}
                    Ok(false) => return Some(error_result(
                        "log_operation target is not in the current workspace/organization scope",
                    )),
                    Err(error) => return Some(error_result(error)),
                }
            }
            let status =
                extract_string_param(args, &["status"]).unwrap_or_else(|| "completed".to_string());
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
            )
            .await
            {
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
                None => {
                    return Some(error_result(
                        "discover_apis requires a valid 'target_id' UUID",
                    ))
                }
            };
            match mutation_target_is_owned(repo, target_id, harness_org_id).await {
                Ok(true) => {}
                Ok(false) => {
                    return Some(error_result(
                        "discover_apis target is not in the current workspace/organization scope",
                    ))
                }
                Err(error) => return Some(error_result(error)),
            }
            let source =
                extract_string_param(args, &["source"]).unwrap_or_else(|| "ai".to_string());
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
                let risk_level = ep
                    .get("risk_level")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                match repo
                    .api_endpoints_insert(
                        target_id,
                        project_path,
                        &url,
                        &method,
                        &path,
                        &params,
                        &json!({}),
                        auth_type,
                        &source,
                        risk_level,
                    )
                    .await
                {
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
                None => {
                    return Some(error_result(
                        "save_js_analysis requires a valid 'target_id' UUID",
                    ))
                }
            };
            match mutation_target_is_owned(repo, target_id, harness_org_id).await {
                Ok(true) => {}
                Ok(false) => return Some(error_result(
                    "save_js_analysis target is not in the current workspace/organization scope",
                )),
                Err(error) => return Some(error_result(error)),
            }
            let url = match extract_string_param(args, &["url"]) {
                Some(u) if !u.is_empty() => u,
                _ => return Some(error_result("save_js_analysis requires a 'url' parameter")),
            };
            let filename = extract_string_param(args, &["filename"]).unwrap_or_default();
            let frameworks = args.get("frameworks").cloned().unwrap_or_else(|| json!([]));
            let libraries = args.get("libraries").cloned().unwrap_or_else(|| json!([]));
            let endpoints_found = args
                .get("endpoints_found")
                .cloned()
                .unwrap_or_else(|| json!([]));
            let secrets_found = args
                .get("secrets_found")
                .cloned()
                .unwrap_or_else(|| json!([]));
            let comments = args.get("comments").cloned().unwrap_or_else(|| json!([]));
            let source_maps = args
                .get("source_maps")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
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
            match repo
                .js_analysis_insert(
                    target_id,
                    project_path.unwrap_or(""),
                    &url,
                    &filename,
                    &analysis,
                )
                .await
            {
                Ok(result) => {
                    if let Some(ref fp) = file_path_param {
                        let id = result
                            .get("id")
                            .and_then(|v| v.as_str())
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
                }
                Err(e) => Some(error_result(format!("Failed to save JS analysis: {}", e))),
            }
        }

        "fingerprint_target" => {
            let target_id = match extract_string_param(args, &["target_id"])
                .and_then(|s| uuid::Uuid::parse_str(&s).ok())
            {
                Some(id) => id,
                None => {
                    return Some(error_result(
                        "fingerprint_target requires a valid 'target_id' UUID",
                    ))
                }
            };
            match mutation_target_is_owned(repo, target_id, harness_org_id).await {
                Ok(true) => {}
                Ok(false) => return Some(error_result(
                    "fingerprint_target target is not in the current workspace/organization scope",
                )),
                Err(error) => return Some(error_result(error)),
            }
            let _source =
                extract_string_param(args, &["source"]).unwrap_or_else(|| "ai".to_string());
            let fps = match args.get("fingerprints").and_then(|v| v.as_array()) {
                Some(arr) => arr.clone(),
                None => {
                    return Some(error_result(
                        "fingerprint_target requires a 'fingerprints' array",
                    ))
                }
            };

            let mut saved = 0u32;
            for fp in &fps {
                let category = fp
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("technology");
                let name = match fp.get("name").and_then(|v| v.as_str()) {
                    Some(n) if !n.is_empty() => n,
                    _ => continue,
                };
                let version = fp.get("version").and_then(|v| v.as_str());
                let confidence =
                    fp.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
                let evidence = fp.get("evidence").cloned().unwrap_or_else(|| json!([]));
                let _cpe = fp.get("cpe").and_then(|v| v.as_str());

                if repo
                    .fingerprints_upsert(
                        target_id,
                        project_path.unwrap_or(""),
                        category,
                        name,
                        version,
                        confidence as f64,
                        Some(&evidence),
                    )
                    .await
                    .is_ok()
                {
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
                None => {
                    return Some(error_result(
                        "log_scan_result requires a valid 'target_id' UUID",
                    ))
                }
            };
            match mutation_target_is_owned(repo, target_id, harness_org_id).await {
                Ok(true) => {}
                Ok(false) => {
                    return Some(error_result(
                        "log_scan_result target is not in the current workspace/organization scope",
                    ))
                }
                Err(error) => return Some(error_result(error)),
            }
            let test_type = match extract_string_param(args, &["test_type"]) {
                Some(t) if !t.is_empty() => t,
                _ => {
                    return Some(error_result(
                        "log_scan_result requires a 'test_type' parameter",
                    ))
                }
            };
            let result_str =
                extract_string_param(args, &["result"]).unwrap_or_else(|| "pending".to_string());
            let payload = extract_string_param(args, &["payload"]).unwrap_or_default();
            let url = extract_string_param(args, &["url"]).unwrap_or_default();
            let parameter = extract_string_param(args, &["parameter"]).unwrap_or_default();
            let evidence = extract_string_param(args, &["evidence"]).unwrap_or_default();
            let severity =
                extract_string_param(args, &["severity"]).unwrap_or_else(|| "info".to_string());
            let tool_used = extract_string_param(args, &["tool_used"]).unwrap_or_default();
            let tester =
                extract_string_param(args, &["tester"]).unwrap_or_else(|| "ai".to_string());
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
            match repo
                .passive_scans_insert(
                    target_id,
                    project_path.unwrap_or(""),
                    &test_type,
                    &tool_used,
                    &findings,
                    Some(&evidence),
                    &severity,
                )
                .await
            {
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
                None => {
                    return Some(error_result(
                        "query_target_data requires a valid 'target_id' UUID",
                    ))
                }
            };

            if let Some(org_id) = harness_org_id {
                let in_scope_rows = match repo.in_scope_targets(Some(org_id)).await {
                    Ok(rows) => rows,
                    Err(err) => {
                        return Some(error_result(format!(
                            "Failed to verify query_target_data target ownership: {err}"
                        )))
                    }
                };
                if !in_scope_rows_own_target(&in_scope_rows, target_id) {
                    return Some(error_result(
                        "query_target_data target_id is not in scope for the active organization",
                    ));
                }
            }

            let sections: Vec<String> = args
                .get("sections")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_else(|| vec!["all".to_string()]);
            let data = match repo.query_target_data(target_id, &sections).await {
                Ok(d) => d,
                Err(e) => return Some(error_result(format!("Failed to query target data: {}", e))),
            };

            Some((data, true))
        }

        "list_in_scope_targets" => {
            let wave_filter = current_wave_asset_filter(
                repo,
                harness_operation_id,
                harness_org_id,
                harness_stage,
                "list_in_scope_targets",
            )
            .await;
            let wave_filter = match wave_filter {
                Ok(filter) => filter,
                Err(error) => return Some(error_result(error)),
            };
            let rows = match repo.in_scope_targets(harness_org_id).await {
                Ok(r) => r,
                Err(e) => {
                    return Some(error_result(format!(
                        "Failed to list in-scope targets: {}",
                        e
                    )))
                }
            };
            let rows = filter_rows_to_current_wave(rows, wave_filter.as_ref());
            let count = rows.len();
            let data = json!({
                "in_scope_targets": rows,
                "count": count,
                "current_wave_filtered": wave_filter.is_some()
            });
            Some((data, true))
        }

        "list_attack_surface_seeds" => {
            // L1b (design 2026-06-24): ranked, rich attack-surface seeds for EAS.
            // Optional `limit`/`cap` truncates the ranked set (D3 per-org cap).
            let cap = args
                .get("limit")
                .or_else(|| args.get("cap"))
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            let wave_filter = current_wave_asset_filter(
                repo,
                harness_operation_id,
                harness_org_id,
                harness_stage,
                "list_attack_surface_seeds",
            )
            .await;
            let wave_filter = match wave_filter {
                Ok(filter) => filter,
                Err(error) => return Some(error_result(error)),
            };
            let repo_cap = if wave_filter.is_some() { None } else { cap };
            let mut rows = match repo.attack_surface_seeds(harness_org_id, repo_cap).await {
                Ok(r) => r,
                Err(e) => {
                    return Some(error_result(format!(
                        "Failed to list attack surface seeds: {}",
                        e
                    )))
                }
            };
            rows = filter_rows_to_current_wave(rows, wave_filter.as_ref());
            if let Some(cap) = cap {
                rows.truncate(cap);
            }
            let count = rows.len();
            let data = json!({
                "attack_surface_seeds": rows,
                "count": count,
                "current_wave_filtered": wave_filter.is_some()
            });
            Some((data, true))
        }

        "list_enumeration_web_roots" => {
            let requested_org_id = extract_string_param(args, &["organization_id"])
                .and_then(|s| uuid::Uuid::parse_str(&s).ok());
            let org_id = match resolve_coverage_org_id(requested_org_id, harness_org_id) {
                Ok(org_id) => org_id,
                Err(error) => return Some(error_result(error)),
            };
            let Some(org_id) = org_id else {
                return Some(error_result(
                    "list_enumeration_web_roots requires an organization_id when no active harness organization is bound",
                ));
            };
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n.clamp(1, 50) as usize)
                .unwrap_or(25);
            let include_coverage = args
                .get("include_coverage")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let stage = crate::harness::StageKind::Enumeration.as_str();
            let coverage_context = stage_coverage_read_context(
                repo,
                harness_operation_id,
                org_id,
                stage,
                "enumeration_web_roots",
            )
            .await;
            if let Err(error) =
                validate_stage_coverage_read_context(stage, session_id, &coverage_context)
            {
                return Some(error_result(error));
            }
            let snapshot = match repo
                .stage_asset_coverage_for_operation(
                    harness_operation_id,
                    org_id,
                    stage,
                    session_id,
                    coverage_context.stage_started_at,
                    coverage_context.current_wave_target_ids,
                    coverage_context.current_wave_asset_values,
                )
                .await
            {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    return Some(error_result(format!(
                        "Failed to list enumeration web roots: {err}"
                    )))
                }
            };
            Some((
                enumeration_web_roots_worklist(snapshot, limit, include_coverage),
                true,
            ))
        }

        "stage_worklist_next" | "stage_worklist_status" => {
            let stage = extract_string_param(args, &["stage"])
                .or_else(|| harness_stage.map(|stage| stage.as_str().to_string()));
            let Some(stage) = stage.filter(|stage| !stage.trim().is_empty()) else {
                return Some(error_result(
                    "stage worklist requires a 'stage' parameter when no harness stage is active",
                ));
            };
            let requested_org_id = extract_string_param(args, &["organization_id"])
                .and_then(|s| uuid::Uuid::parse_str(&s).ok());
            let org_id = match resolve_coverage_org_id(requested_org_id, harness_org_id) {
                Ok(org_id) => org_id,
                Err(error) => return Some(error_result(error)),
            };
            let Some(org_id) = org_id else {
                return Some(error_result(
                    "stage worklist requires an organization_id when no active harness organization is bound",
                ));
            };
            let coverage_context = stage_coverage_read_context(
                repo,
                harness_operation_id,
                org_id,
                &stage,
                "stage_worklist",
            )
            .await;
            if let Err(error) =
                validate_stage_coverage_read_context(&stage, session_id, &coverage_context)
            {
                return Some(error_result(error));
            }
            let snapshot = match repo
                .stage_asset_coverage_for_operation(
                    harness_operation_id,
                    org_id,
                    &stage,
                    session_id,
                    coverage_context.stage_started_at,
                    coverage_context.current_wave_target_ids,
                    coverage_context.current_wave_asset_values,
                )
                .await
            {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    return Some(error_result(format!(
                        "Failed to read stage worklist: {err}"
                    )))
                }
            };
            let (snapshot, terminal_exception_preview) =
                match preview_terminal_exceptions(snapshot, args.get("terminal_exceptions")) {
                    Ok(preview) => preview,
                    Err(error) => return Some(error_result(error)),
                };
            if tool_name == "stage_worklist_status" {
                return Some((
                    attach_terminal_exception_preview(
                        stage_worklist_status(snapshot),
                        terminal_exception_preview,
                    ),
                    true,
                ));
            }
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n.clamp(1, 200) as usize)
                .unwrap_or(25);
            let preferred_states = args
                .get("prefer")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .filter(|states| !states.is_empty())
                .unwrap_or_else(|| {
                    vec![
                        "pending".to_string(),
                        "error".to_string(),
                        "partial".to_string(),
                    ]
                });
            Some((
                attach_terminal_exception_preview(
                    stage_worklist_next(snapshot, limit, &preferred_states),
                    terminal_exception_preview,
                ),
                true,
            ))
        }

        "check_stage_asset_coverage" => {
            let stage = extract_string_param(args, &["stage"])
                .or_else(|| harness_stage.map(|stage| stage.as_str().to_string()));
            let Some(stage) = stage.filter(|stage| !stage.trim().is_empty()) else {
                return Some(error_result(
                    "check_stage_asset_coverage requires a 'stage' parameter when no harness stage is active",
                ));
            };
            let requested_org_id = extract_string_param(args, &["organization_id"])
                .and_then(|s| uuid::Uuid::parse_str(&s).ok());
            let org_id = match resolve_coverage_org_id(requested_org_id, harness_org_id) {
                Ok(org_id) => org_id,
                Err(error) => return Some(error_result(error)),
            };
            let Some(org_id) = org_id else {
                return Some(error_result(
                    "check_stage_asset_coverage requires an organization_id when no active harness organization is bound",
                ));
            };
            let max_gaps = args
                .get("max_gaps")
                .and_then(|v| v.as_u64())
                .map(|n| n.clamp(1, 200) as usize)
                .unwrap_or(25);
            let include_assets_requested = args
                .get("include_assets")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if include_assets_requested {
                tracing::debug!(
                    target: "harness::coverage_preflight",
                    "check_stage_asset_coverage ignores include_assets=true; agent preflight stays summary-only"
                );
            }
            let coverage_context = stage_coverage_read_context(
                repo,
                harness_operation_id,
                org_id,
                &stage,
                "coverage_preflight",
            )
            .await;
            if let Err(error) =
                validate_stage_coverage_read_context(&stage, session_id, &coverage_context)
            {
                return Some(error_result(error));
            }
            let snapshot = match repo
                .stage_asset_coverage_for_operation(
                    harness_operation_id,
                    org_id,
                    &stage,
                    session_id,
                    coverage_context.stage_started_at,
                    coverage_context.current_wave_target_ids,
                    coverage_context.current_wave_asset_values,
                )
                .await
            {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    return Some(error_result(format!(
                        "Failed to check stage asset coverage: {err}"
                    )))
                }
            };
            let (snapshot, terminal_exception_preview) =
                match preview_terminal_exceptions(snapshot, args.get("terminal_exceptions")) {
                    Ok(preview) => preview,
                    Err(error) => return Some(error_result(error)),
                };
            Some((
                attach_terminal_exception_preview(
                    compact_stage_asset_coverage(snapshot, max_gaps, include_assets_requested),
                    terminal_exception_preview,
                ),
                true,
            ))
        }

        "list_recent_evidence" => {
            let Some(session_id) = session_id.filter(|s| !s.trim().is_empty()) else {
                return Some(error_result(
                    "list_recent_evidence requires an active session; no evidence context is available",
                ));
            };
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n.clamp(1, 200) as i64)
                .unwrap_or(25);
            let rows = match repo.recent_evidence_detailed(session_id, limit).await {
                Ok(rows) => rows,
                Err(e) => {
                    return Some(error_result(format!("Failed to list recent evidence: {e}")))
                }
            };
            let count = rows.len();
            Some((
                json!({
                    "recent_evidence": rows,
                    "count": count,
                    "contract": "These are this run's REAL evidence-ledger ids (newest first). Put the evidence_id values whose tool/asset/technique backs each claim into that claim's evidence_ids and the top-level evidence_refs. Never invent ids, copy placeholders (1,2,3), or use submit_stage_deliverable to discover missing ids.",
                }),
                true,
            ))
        }

        _ => None,
    }
}

#[derive(Debug, Default)]
struct StageCoverageReadContext {
    stage_started_at: Option<DateTime<Utc>>,
    current_wave_target_ids: Option<Vec<uuid::Uuid>>,
    current_wave_asset_values: Option<Vec<String>>,
    wave_error: Option<String>,
}

fn resolve_coverage_org_id(
    requested_org_id: Option<uuid::Uuid>,
    harness_org_id: Option<uuid::Uuid>,
) -> Result<Option<uuid::Uuid>, String> {
    if let (Some(requested), Some(bound)) = (requested_org_id, harness_org_id) {
        if requested != bound {
            return Err(format!(
                "stage coverage organization_id {requested} does not match the active harness organization {bound}"
            ));
        }
    }
    Ok(harness_org_id.or(requested_org_id))
}

fn validate_stage_coverage_read_context(
    stage: &str,
    session_id: Option<&str>,
    context: &StageCoverageReadContext,
) -> Result<(), String> {
    if let Some(error) = context.wave_error.as_ref() {
        return Err(error.clone());
    }
    if stage != crate::harness::StageKind::Enumeration.as_str() {
        return Ok(());
    }
    if session_id.is_none_or(|run_id| run_id.trim().is_empty()) {
        return Err(
            "Enumeration worklist requires the active run/session; latest or unscoped outcome fallback is forbidden"
                .to_string(),
        );
    }
    if !crate::harness::org_gate::stage_accepts_outcome_projection(
        crate::harness::StageKind::Enumeration,
        context.stage_started_at.is_some(),
    ) {
        return Err(
            "Enumeration worklist requires an active Enumeration operation with a current stage_started_at freshness cutoff"
                .to_string(),
        );
    }
    Ok(())
}

async fn stage_coverage_read_context(
    repo: &dyn crate::db_traits::DbRepoProvider,
    harness_operation_id: Option<uuid::Uuid>,
    org_id: uuid::Uuid,
    stage: &str,
    log_context: &'static str,
) -> StageCoverageReadContext {
    let Some(operation_id) = harness_operation_id else {
        return StageCoverageReadContext::default();
    };

    if crate::harness::StageKind::try_parse(stage).is_some() {
        match repo
            .stage_asset_wave_current_running(operation_id, org_id, stage)
            .await
        {
            Ok(Some(wave)) => {
                if let Err(error) = wave.validate_membership() {
                    return StageCoverageReadContext {
                        stage_started_at: Some(wave.started_at),
                        wave_error: Some(format!(
                            "invalid current asset wave for {stage}: {error}"
                        )),
                        ..StageCoverageReadContext::default()
                    };
                }
                return StageCoverageReadContext {
                    stage_started_at: Some(wave.started_at),
                    current_wave_target_ids: Some(wave.target_ids),
                    current_wave_asset_values: Some(wave.asset_values),
                    wave_error: None,
                };
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    target: "harness::stage_worklist",
                    context = log_context,
                    error = %err,
                    "failed to read current asset wave; stage coverage fails closed"
                );
                return StageCoverageReadContext {
                    wave_error: Some(format!(
                        "failed to read current asset wave for {stage}: {err}"
                    )),
                    ..StageCoverageReadContext::default()
                };
            }
        }
    }

    let stage_started_at = match repo.operation_state_get(operation_id).await {
        Ok(Some(state)) if state.current_stage == stage => Some(state.stage_started_at),
        Ok(_) => None,
        Err(err) => {
            let freshness_behavior = if stage == "enumeration" {
                "Enumeration remains fail-closed without a freshness cutoff"
            } else {
                "stage coverage continues without an operation-state freshness cutoff"
            };
            tracing::warn!(
                target: "harness::stage_worklist",
                context = log_context,
                error = %err,
                freshness_behavior,
                "failed to read operation_state; no operation-state freshness cutoff is available"
            );
            None
        }
    };

    StageCoverageReadContext {
        stage_started_at,
        current_wave_target_ids: None,
        current_wave_asset_values: None,
        wave_error: None,
    }
}

async fn current_wave_asset_filter(
    repo: &dyn crate::db_traits::DbRepoProvider,
    harness_operation_id: Option<uuid::Uuid>,
    harness_org_id: Option<uuid::Uuid>,
    harness_stage: Option<crate::harness::StageKind>,
    log_context: &'static str,
) -> Result<Option<BTreeSet<uuid::Uuid>>, String> {
    let Some(org_id) = harness_org_id else {
        return Ok(None);
    };
    let Some(stage) = harness_stage else {
        return Ok(None);
    };
    let context = stage_coverage_read_context(
        repo,
        harness_operation_id,
        org_id,
        stage.as_str(),
        log_context,
    )
    .await;
    validate_stage_coverage_read_context(stage.as_str(), Some("wave-filter"), &context)?;
    Ok(context
        .current_wave_target_ids
        .map(|target_ids| target_ids.into_iter().collect()))
}

fn filter_rows_to_current_wave(
    rows: Vec<Value>,
    current_wave_target_ids: Option<&BTreeSet<uuid::Uuid>>,
) -> Vec<Value> {
    let Some(current_wave_target_ids) = current_wave_target_ids else {
        return rows;
    };
    rows.into_iter()
        .filter(|row| {
            row.get("id")
                .or_else(|| row.get("target_id"))
                .and_then(Value::as_str)
                .and_then(|value| uuid::Uuid::parse_str(value).ok())
                .is_some_and(|target_id| current_wave_target_ids.contains(&target_id))
        })
        .collect()
}

fn port_state_is_open_json(entry: &Value) -> bool {
    entry
        .get("state")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|state| !state.is_empty())
        .map(|state| state.eq_ignore_ascii_case("open"))
        .unwrap_or(true)
}

fn port_number_from_json(entry: &Value) -> Option<u16> {
    let value = entry.get("port")?;
    let raw = value
        .as_u64()
        .map(|n| n.to_string())
        .or_else(|| value.as_str().map(|s| s.trim().to_string()))?;
    let port = raw.parse::<u16>().ok()?;
    (port > 0).then_some(port)
}

fn port_service_hint(entry: &Value) -> String {
    [
        "service",
        "name",
        "proto",
        "protocol",
        "scheme",
        "webserver",
    ]
    .iter()
    .filter_map(|key| entry.get(*key).and_then(Value::as_str))
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" ")
    .to_ascii_lowercase()
}

fn web_origin_from_port_url(raw: &str) -> Option<(String, String, Option<u16>)> {
    let parsed = url::Url::parse(raw.trim()).ok()?;
    let scheme = parsed.scheme();
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?;
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let port = parsed.port_or_known_default();
    let port_suffix = match (scheme, port) {
        ("http", Some(80)) | ("https", Some(443)) | (_, None) => String::new(),
        (_, Some(port)) => format!(":{port}"),
    };
    Some((
        format!("{scheme}://{host}{port_suffix}/"),
        scheme.to_string(),
        port,
    ))
}

fn web_origin_score(scheme: &str, port: Option<u16>) -> i32 {
    let mut score = 0;
    if scheme == "https" {
        score += 200;
    }
    if !matches!((scheme, port), ("http", Some(80)) | ("https", Some(443))) {
        score += 50;
    }
    if port == Some(443) {
        score += 20;
    }
    if matches!(port, Some(8443 | 9443 | 10443)) {
        score += 15;
    }
    score
}

/// Derive a full `scheme://host:port/` web root URL from an in-scope asset's EAS
/// metadata (design 2026-07-03-enumeration-throughput-optimization PR-A). The
/// final scheme/port suffix rule mirrors `golish_app_core::domain::targets::web_root_url`;
/// this local adapter stays in `agent-kit` because the crate does not depend on
/// `app-core` (avoid a new cross-crate edge). Both sides are pinned by tests.
fn web_root_url_from_meta(
    host: &str,
    http_status: Option<i64>,
    ports: &Value,
    webserver: &str,
) -> (String, String, Option<u16>) {
    // Exact Enumeration rows already carry normalized Web Origin identity.
    // Re-normalize through the shared domain helper so default ports remain
    // explicit and scheme/port cannot collapse in downstream work items.
    if let Some(origin) = golish_pentest_domain::canonical_web_origin(host) {
        return (origin.root_url, origin.scheme, Some(origin.port));
    }

    // Prefer confirmed open HTTP(S) origins carried by EAS/httpx/whatweb rows.
    // `ports[].url` is the least ambiguous source because it preserves scheme and
    // non-default port. Filtered/closed rows must not become enumeration roots.
    if let Some(best) = ports
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| port_state_is_open_json(entry))
        .filter_map(|entry| entry.get("url").and_then(Value::as_str))
        .filter_map(web_origin_from_port_url)
        .max_by_key(|(_, scheme, port)| web_origin_score(scheme, *port))
    {
        return best;
    }

    // Pick the most web-like confirmed-open port from the ports array (prefer an
    // explicit https hint, else the first web-ish port); fall back to scheme
    // inference from http_status/webserver.
    let mut chosen_port: Option<u16> = None;
    let mut chosen_https = false;
    if let Some(arr) = ports.as_array() {
        for p in arr {
            if !port_state_is_open_json(p) {
                continue;
            }
            let port = port_number_from_json(p);
            let service = port_service_hint(p);
            let is_https = service.contains("https")
                || service.contains("ssl")
                || matches!(port, Some(443 | 8443 | 9443));
            let is_web = is_https
                || service.contains("http")
                || matches!(port, Some(80 | 8080 | 8000 | 8888 | 3000 | 5000));
            if is_web && (chosen_port.is_none() || (is_https && !chosen_https)) {
                chosen_port = port;
                chosen_https = is_https;
            }
        }
    }

    let scheme = if chosen_https
        || webserver.to_ascii_lowercase().contains("https")
        || matches!(chosen_port, Some(443 | 8443 | 9443))
    {
        "https"
    } else {
        "http"
    };
    let _ = http_status;
    let port_suffix = match (scheme, chosen_port) {
        ("http", Some(80)) | ("https", Some(443)) | (_, None) => String::new(),
        (_, Some(port)) => format!(":{port}"),
    };
    (
        format!("{scheme}://{host}{port_suffix}/"),
        scheme.to_string(),
        chosen_port,
    )
}

fn exact_enumeration_root_from_asset(asset: &Value) -> Option<(String, String, Option<u16>)> {
    let value = asset.get("value").and_then(Value::as_str).unwrap_or("");
    let exact = asset
        .get("exact_web_origin")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !exact {
        return None;
    }
    let http_status = asset.get("http_status").and_then(Value::as_i64);
    let ports = asset.get("ports").cloned().unwrap_or(Value::Null);
    let webserver = asset.get("webserver").and_then(Value::as_str).unwrap_or("");
    Some(web_root_url_from_meta(
        value,
        http_status,
        &ports,
        webserver,
    ))
}

fn normalize_enumeration_snapshot(mut snapshot: Value) -> Value {
    if snapshot.get("stage").and_then(Value::as_str) != Some("enumeration") {
        return snapshot;
    }
    let raw_assets = snapshot
        .get_mut("assets")
        .and_then(Value::as_array_mut)
        .map(std::mem::take)
        .unwrap_or_default();
    let mut assets = Vec::new();
    for mut asset in raw_assets {
        if asset.get("exact_web_origin").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let Some(origin) = asset
            .get("value")
            .and_then(Value::as_str)
            .and_then(golish_pentest_domain::canonical_web_origin)
        else {
            continue;
        };
        asset["value"] = json!(origin.key);
        asset["target_type"] = json!("url");
        assets.push(asset);
    }

    let mut total_assets = 0usize;
    let mut seed_assets = 0usize;
    let mut new_assets = 0usize;
    let mut done_assets = 0usize;
    let mut pending_assets = 0usize;
    let mut blocked_assets = 0usize;
    for asset in &assets {
        let coverage = asset
            .get("coverage")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if coverage
            .iter()
            .any(|cell| cell.get("state").and_then(Value::as_str) == Some("next_wave_pending"))
        {
            continue;
        }
        total_assets += 1;
        if asset.get("discovered_phase").and_then(Value::as_str) == Some("new_in_stage") {
            new_assets += 1;
        } else {
            seed_assets += 1;
        }
        if coverage.iter().any(|cell| {
            matches!(
                cell.get("state").and_then(Value::as_str),
                Some("blocked" | "error")
            )
        }) {
            blocked_assets += 1;
        } else if coverage.iter().any(|cell| {
            matches!(
                cell.get("state").and_then(Value::as_str),
                Some("pending" | "partial")
            )
        }) {
            pending_assets += 1;
        } else {
            done_assets += 1;
        }
    }
    snapshot["summary"] = json!({
        "total_assets": total_assets,
        "seed_assets": seed_assets,
        "new_assets": new_assets,
        "done_assets": done_assets,
        "pending_assets": pending_assets,
        "blocked_assets": blocked_assets,
    });
    snapshot["assets"] = Value::Array(assets);
    snapshot
}

/// Web-root priority for a coverage-snapshot asset (design 2026-07-03-enumeration-
/// throughput-optimization PR-C, mirrors `domain::targets::rank_enumeration_web_
/// roots`): EAS-proven-alive (has `http_status`) first, then those with open
/// ports, so when the worklist is truncated to `limit` the high-value live roots
/// survive and a cut-short pass spends its budget on them first. Higher = sooner.
fn enum_worklist_asset_priority(asset: &Value) -> i32 {
    let mut score = 0;
    let pending_or_error_cells = asset
        .get("coverage")
        .and_then(Value::as_array)
        .map(|coverage| {
            coverage
                .iter()
                .filter(|cell| {
                    matches!(
                        cell.get("state")
                            .and_then(Value::as_str)
                            .unwrap_or("pending"),
                        "pending" | "error" | "partial"
                    )
                })
                .count()
        })
        .unwrap_or(0);
    score += (pending_or_error_cells as i32) * 1_000;
    if asset.get("http_status").and_then(Value::as_i64).is_some() {
        score += 100;
    }
    if asset
        .get("ports")
        .and_then(Value::as_array)
        .map(|a| !a.is_empty())
        .unwrap_or(false)
    {
        score += 10;
    }
    score
}

fn enumeration_web_roots_worklist(snapshot: Value, limit: usize, include_coverage: bool) -> Value {
    let snapshot = normalize_enumeration_snapshot(snapshot);
    let mut assets = snapshot
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assets.retain(|asset| exact_enumeration_root_from_asset(asset).is_some());
    let total = assets.len();
    // PR-C: alive/high-value roots first (stable) so the `limit` truncation keeps
    // the roots worth enumerating; ties keep the snapshot's created_at order.
    assets.sort_by_key(|asset| std::cmp::Reverse(enum_worklist_asset_priority(asset)));
    let mut roots = Vec::new();
    for asset in assets.into_iter().take(limit) {
        let coverage = asset
            .get("coverage")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut pending_techniques = Vec::new();
        let mut terminal_techniques = Vec::new();
        let mut suggested_capabilities = Vec::new();
        let mut suggested_tools = Vec::new();
        for cell in &coverage {
            let technique = cell.get("technique").and_then(Value::as_str).unwrap_or("");
            let state = cell
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("pending");
            if matches!(state, "pending" | "error" | "partial") {
                pending_techniques.push(technique.to_string());
                extend_unique_capabilities(&mut suggested_capabilities, cell);
                if let Some(tools) = cell.get("suggested_tools").and_then(Value::as_array) {
                    suggested_tools
                        .extend(tools.iter().filter_map(Value::as_str).map(str::to_string));
                }
            } else {
                terminal_techniques.push(technique.to_string());
            }
        }
        suggested_tools.sort();
        suggested_tools.dedup();

        // PR-A: build the full scheme://host:port/ root_url from EAS metadata so
        // the enumerator can feed URLs straight to the content tools instead of
        // querying each target just to reconstruct the scheme/port.
        let host = asset
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let http_status = asset.get("http_status").and_then(Value::as_i64);
        let ports = asset.get("ports").cloned().unwrap_or(Value::Null);
        let webserver = asset.get("webserver").and_then(Value::as_str).unwrap_or("");
        let exact_web_origin = asset
            .get("exact_web_origin")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let origin =
            exact_web_origin.then(|| web_root_url_from_meta(&host, http_status, &ports, webserver));
        let needs_probe = !exact_web_origin;

        let mut root = json!({
            "target_id": asset.get("target_id").cloned().unwrap_or(Value::Null),
            "root_url": origin.as_ref().map(|(root_url, _, _)| root_url),
            "scheme": origin.as_ref().map(|(_, scheme, _)| scheme),
            "port": origin.as_ref().and_then(|(_, _, port)| *port),
            "needs_probe": needs_probe,
            "exact_web_origin": exact_web_origin,
            "asset": asset.get("value").cloned().unwrap_or(Value::Null),
            "target_type": asset.get("target_type").cloned().unwrap_or(Value::Null),
            "organization_id": asset.get("organization_id").cloned().unwrap_or(Value::Null),
            "discovered_phase": asset.get("discovered_phase").cloned().unwrap_or(Value::Null),
            "pending_techniques": pending_techniques,
            "terminal_techniques": terminal_techniques,
            "suggested_capabilities": suggested_capabilities,
            "suggested_tools": suggested_tools,
            "next_steps": enumeration_web_root_next_steps(&coverage),
        });
        if include_coverage {
            root["coverage"] = Value::Array(coverage);
        }
        roots.push(root);
    }
    let count = roots.len();
    let truncated = total > count;

    json!({
        "stage": "enumeration",
        "organization_id": snapshot.get("organization_id").cloned().unwrap_or(Value::Null),
        "session_id": snapshot.get("session_id").cloned().unwrap_or(Value::Null),
        "web_roots": roots,
        "count": count,
        "total": total,
        "truncated": truncated,
        "recommended_batch_size": 25,
        "max_page_size": 50,
        "worklist_semantics": "This list is derived from check_stage_asset_coverage and narrowed to EAS-confirmed exact web origins. Pending/error/partial roots are unfinished. Run enum_preflight_web_origins once on each page before producers. Fresh exact-origin producer evidence owns found/empty. Blocked requires current-target evidence from preflight on all four axes, route recovery on DIR, or browser recovery on JS/JSAPI/PARAM. Business rows remain discovery context.",
        "execution_order": [
            "enum_preflight_web_origins once for the page; exclude only trusted blocked roots from producers",
            "enum_crawl_same_origin_urls once as bounded same-origin browser seed discovery",
            "browser_collect_js_api for GOLISH-ENUM-JS plus runtime API/parameter outcomes",
            "js_extract_apis",
            "route_probe_paths once after JS/API landing, using DB seeds and the full local/built-in wordlist with batch_concurrency=4; resume ordinary partials but do not retry a persisted recovery-exhausted blocked DIR cell",
            "parameter extraction from observed requests, query strings, forms, and targeted param_hints",
            "check_stage_asset_coverage before submit_stage_deliverable"
        ],
        "tool_boundary": "Call enum_preflight_web_origins first with target_id + exact target_url, then call browser_collect_js_api/js_extract_apis/route_probe_paths only for reachable/pending roots; use enum_crawl_same_origin_urls for bounded crawler supplements. Model-authored terminal_exceptions and coverage are forbidden. A direct producer may publish blocked only after its bounded backend recovery breaker exhausts; do not retry a persisted recovery_exhausted blocked cell. Directory discovery must use route_probe_paths. Do not use ffuf/gobuster/feroxbuster, raw katana/pentest_run, or manage_targets in enumeration.",
    })
}

fn enumeration_web_root_next_steps(coverage: &[Value]) -> Vec<&'static str> {
    let mut steps = Vec::new();
    if coverage.iter().any(|cell| {
        matches!(
            cell.get("state").and_then(Value::as_str),
            Some("pending" | "error" | "partial")
        )
    }) {
        steps.push(
            "run enum_preflight_web_origins once for this exact root before content producers; trusted transport failure may block all axes, while bounded route/browser recovery may block only their owned axes",
        );
    }
    for cell in coverage {
        let state = cell
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("pending");
        if !matches!(state, "pending" | "error" | "partial") {
            continue;
        }
        match cell.get("technique").and_then(Value::as_str).unwrap_or("") {
            "GOLISH-ENUM-JS" => steps.push(
                "close GOLISH-ENUM-JS with browser_collect_js_api and require a fresh exact-origin outcome",
            ),
            "GOLISH-ENUM-JSAPI" => {
                steps.push("close GOLISH-ENUM-JSAPI with browser_collect_js_api first, then js_extract_apis on saved JS; require a fresh exact-origin outcome")
            }
            "GOLISH-ENUM-DIR" => steps.push(
                "close GOLISH-ENUM-DIR with route_probe_paths after JS/API landing using target_id + base_url; resume ordinary partials, but a persisted recovery_exhausted blocked result is terminal",
            ),
            "GOLISH-ENUM-PARAM" => {
                steps.push(
                    "close GOLISH-ENUM-PARAM from observed requests/query strings/forms and targeted js_extract_apis param_hints; require a fresh exact-origin outcome",
                )
            }
            _ => steps.push(
                "run enum_preflight_web_origins for the pending root, then close reachable cells with their direct producer",
            ),
        }
    }
    steps.sort();
    steps.dedup();
    steps
}

fn extend_unique_capabilities(out: &mut Vec<Value>, cell: &Value) {
    let Some(capabilities) = cell.get("suggested_capabilities").and_then(Value::as_array) else {
        return;
    };
    for capability in capabilities {
        let id = capability.get("id").and_then(Value::as_str).unwrap_or("");
        if id.is_empty() {
            continue;
        }
        let already_present = out
            .iter()
            .any(|existing| existing.get("id").and_then(Value::as_str) == Some(id));
        if !already_present {
            out.push(capability.clone());
        }
    }
}

const MAX_ENUMERATION_WORKLIST_ROOTS: usize = 50;
const EAS_WEB_FINGERPRINT: &str = "GOLISH-EAS-WEB-FINGERPRINT";

fn terminal_exception_preview_value(stage: &str) -> Value {
    let contract = if stage == "enumeration" {
        "Model-authored Enumeration terminal exceptions are disabled. Omit terminal_exceptions or pass []; current-run producer/preflight/recovery evidence owns every terminal cell and submit coverage must be []."
    } else {
        "Preview only: Target Intel / EAS may close an exact authoritative pending cell as checked_empty (with exact-technique evidence), blocked, or not_applicable (with a concrete note). An EAS WEB-FINGERPRINT parent-cell exception cannot close details.missing_origins; every exact origin still needs guarded producer completion. This does not persist or authorize anything. Copy coverage_to_submit unchanged into submit_stage_deliverable.coverage."
    };
    json!({
        "preview_only": true,
        "persisted": false,
        "provided_cells": 0,
        "accepted_cells": 0,
        "rejected_cells": 0,
        "blocked_cells": 0,
        "not_applicable_cells": 0,
        "coverage_to_submit": [],
        "rejected_terminal_exceptions": [],
        "contract": contract
    })
}

fn attach_terminal_exception_preview(mut output: Value, preview: Value) -> Value {
    output["terminal_exceptions_preview"] = preview;
    output
}

/// The ordinary EAS matrix is parent-asset keyed, but its Web fingerprint gate
/// has a stricter exact `scheme://host:port` denominator. A parent-cell terminal
/// exception cannot replace completion of any origin still listed here.
fn eas_web_cell_has_missing_exact_origins(stage: &str, cell: &Value) -> bool {
    stage == "external_attack_surface"
        && cell.get("technique").and_then(Value::as_str) == Some(EAS_WEB_FINGERPRINT)
        && cell
            .get("details")
            .and_then(|details| details.get("missing_origins"))
            .and_then(Value::as_array)
            .is_some_and(|origins| !origins.is_empty())
}

fn refresh_preview_asset_summary(snapshot: &mut Value) {
    let Some(assets) = snapshot.get("assets").and_then(Value::as_array) else {
        return;
    };
    let mut done_assets = 0usize;
    let mut pending_assets = 0usize;
    let mut blocked_assets = 0usize;
    for asset in assets {
        let coverage = asset
            .get("coverage")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if coverage
            .iter()
            .any(|cell| cell.get("state").and_then(Value::as_str) == Some("next_wave_pending"))
        {
            continue;
        }
        if coverage.iter().any(|cell| {
            matches!(
                cell.get("state").and_then(Value::as_str),
                Some("blocked" | "error")
            )
        }) {
            blocked_assets += 1;
        } else if coverage.iter().any(|cell| {
            matches!(
                cell.get("state").and_then(Value::as_str),
                Some("pending" | "partial")
            )
        }) {
            pending_assets += 1;
        } else {
            done_assets += 1;
        }
    }
    if let Some(summary) = snapshot.get_mut("summary").and_then(Value::as_object_mut) {
        summary.insert("done_assets".to_string(), json!(done_assets));
        summary.insert("pending_assets".to_string(), json!(pending_assets));
        summary.insert("blocked_assets".to_string(), json!(blocked_assets));
    }
}

/// Preview honest terminal coverage against the current authoritative asset ×
/// technique matrix. This never writes DB truth: it only lets Target Intel / EAS
/// use the same exact terminal cells for preflight and final submission. Found is
/// intentionally impossible here; only producers/DB truth may create it.
/// Enumeration stays stricter and rejects all model-authored terminal cells.
fn preview_terminal_exceptions(
    snapshot: Value,
    raw_exceptions: Option<&Value>,
) -> Result<(Value, Value), String> {
    let mut snapshot = normalize_enumeration_snapshot(snapshot);
    let stage = snapshot
        .get("stage")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let Some(raw_exceptions) = raw_exceptions.filter(|value| !value.is_null()) else {
        return Ok((snapshot, terminal_exception_preview_value(&stage)));
    };
    let entries = raw_exceptions
        .as_array()
        .ok_or_else(|| "terminal_exceptions must be an array".to_string())?;
    if entries.is_empty() {
        return Ok((snapshot, terminal_exception_preview_value(&stage)));
    }
    if stage == "enumeration" {
        return Err("terminal_exceptions is disabled for Enumeration; blocked is backend-authored by enum_preflight_web_origins or bounded route/browser recovery, and submit coverage=[]".to_string());
    }
    if !matches!(stage.as_str(), "target_intel" | "external_attack_surface") {
        return Err(format!(
            "terminal_exceptions preview is supported only for target_intel and external_attack_surface, not '{stage}'"
        ));
    }

    let assets = snapshot
        .get_mut("assets")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "coverage snapshot has no authoritative assets array".to_string())?;
    let mut seen = BTreeSet::new();
    let mut coverage_to_submit = Vec::with_capacity(entries.len());
    let mut rejected_terminal_exceptions = Vec::new();
    let mut blocked_cells = 0usize;
    let mut not_applicable_cells = 0usize;

    for (index, entry) in entries.iter().enumerate() {
        let object = entry
            .as_object()
            .ok_or_else(|| format!("terminal_exceptions[{index}] must be an object"))?;
        let asset = object
            .get("asset")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("terminal_exceptions[{index}].asset must be non-empty"))?;
        let technique = object
            .get("technique")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("terminal_exceptions[{index}].technique must be non-empty"))?;
        let status = object
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("terminal_exceptions[{index}].status is required"))?;
        if !matches!(status, "checked_empty" | "blocked" | "not_applicable") {
            return Err(format!(
                "terminal_exceptions[{index}].status '{status}' is invalid; preview accepts only checked_empty, blocked, or not_applicable (found is DB-owned)"
            ));
        }
        if !seen.insert((asset.to_string(), technique.to_string())) {
            return Err(format!(
                "terminal_exceptions contains duplicate cell ({asset}, {technique})"
            ));
        }

        let evidence_refs = match object.get("evidence_refs").filter(|value| !value.is_null()) {
            None => Vec::new(),
            Some(value) => value
                .as_array()
                .ok_or_else(|| {
                    format!("terminal_exceptions[{index}].evidence_refs must be an array")
                })?
                .iter()
                .map(|value| {
                    value.as_i64().filter(|id| *id > 0).ok_or_else(|| {
                        format!(
                            "terminal_exceptions[{index}].evidence_refs must contain positive integer ledger ids"
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        if status == "checked_empty" && evidence_refs.is_empty() {
            return Err(format!(
                "terminal_exceptions[{index}] checked_empty requires exact-technique evidence_refs; checked empty is not the same as unattempted"
            ));
        }
        let note = object
            .get("note")
            .filter(|value| !value.is_null())
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if matches!(status, "blocked" | "not_applicable") && note.is_none() {
            return Err(format!(
                "terminal_exceptions[{index}] status '{status}' requires a concrete non-empty note"
            ));
        }

        let asset_row = assets
            .iter_mut()
            .find(|row| row.get("value").and_then(Value::as_str) == Some(asset))
            .ok_or_else(|| {
                format!(
                    "terminal_exceptions[{index}] asset '{asset}' is not in the authoritative current worklist"
                )
            })?;
        let cell = asset_row
            .get_mut("coverage")
            .and_then(Value::as_array_mut)
            .and_then(|cells| {
                cells.iter_mut().find(|cell| {
                    cell.get("technique").and_then(Value::as_str) == Some(technique)
                })
            })
            .ok_or_else(|| {
                format!(
                    "terminal_exceptions[{index}] technique '{technique}' is not a coverage cell for '{asset}'"
                )
            })?;
        let current = cell
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("pending");
        if !matches!(current, "pending" | "error" | "partial") {
            return Err(format!(
                "terminal_exceptions[{index}] cell ({asset}, {technique}) is already terminal as '{current}'; omit it and keep DB truth"
            ));
        }
        if eas_web_cell_has_missing_exact_origins(&stage, cell) {
            rejected_terminal_exceptions.push(json!({
                "index": index,
                "asset": asset,
                "technique": technique,
                "status": status,
                "missing_origins": cell["details"]["missing_origins"].clone(),
                "reason": "EAS WEB-FINGERPRINT parent-cell terminal exception rejected: parent asset coverage cannot close missing exact Web origins; run eas_fingerprint_web_stack for every details.missing_origins entry"
            }));
            continue;
        }

        cell["state"] = json!(status);
        cell["source"] = json!("submit_stage_deliverable_preview");
        cell["evidence_refs"] = json!(evidence_refs);
        cell["planned_terminal_exception"] = json!(true);
        if let Some(note) = note {
            cell["note"] = json!(note);
        }
        blocked_cells += usize::from(status == "blocked");
        not_applicable_cells += usize::from(status == "not_applicable");
        coverage_to_submit.push(entry.clone());
    }

    refresh_preview_asset_summary(&mut snapshot);
    let preview = json!({
        "preview_only": true,
        "persisted": false,
        "provided_cells": entries.len(),
        "accepted_cells": coverage_to_submit.len(),
        "rejected_cells": rejected_terminal_exceptions.len(),
        "blocked_cells": blocked_cells,
        "not_applicable_cells": not_applicable_cells,
        "coverage_to_submit": coverage_to_submit,
        "rejected_terminal_exceptions": rejected_terminal_exceptions,
        "contract": "Accepted terminal cells were validated only against the current authoritative worklist. Copy coverage_to_submit unchanged into submit_stage_deliverable.coverage. EAS WEB-FINGERPRINT parent-cell exceptions are rejected while details.missing_origins is non-empty; every exact origin still needs guarded producer completion. This preview did not persist outcomes or authorize new assets."
    });
    Ok((snapshot, preview))
}

fn stage_worklist_status(snapshot: Value) -> Value {
    let is_enumeration = snapshot.get("stage").and_then(Value::as_str) == Some("enumeration");
    let mut out = compact_stage_asset_coverage(snapshot, 10, false);
    out["tool"] = json!("stage_worklist_status");
    out["worklist_contract"] = if is_enumeration {
        json!("Status is current-run fresh exact-origin evidence truth. pending/error/partial cells are unfinished; found/checked_empty come from their direct producers. Blocked requires current-target evidence from enum_preflight_web_origins on all axes, route_probe_paths recovery on DIR, or browser_collect_js_api recovery on JS/JSAPI/PARAM. Raw non-Web hosts are excluded from the denominator instead of becoming not_applicable cells. Submit coverage=[].")
    } else {
        json!("Status is DB/gate truth. pending/error/partial cells are unfinished work; checked_empty/found/blocked/not_applicable are terminal when backed by evidence or a concrete note.")
    };
    out["next_tool"] = if out
        .get("ready_to_submit")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        json!("submit_stage_deliverable")
    } else {
        json!("stage_worklist_next")
    };
    out
}

fn stage_worklist_next(snapshot: Value, limit: usize, preferred_states: &[String]) -> Value {
    let snapshot = normalize_enumeration_snapshot(snapshot);
    let stage = snapshot.get("stage").and_then(Value::as_str).unwrap_or("");
    let is_enumeration = stage == "enumeration";
    let is_eas = stage == "external_attack_surface";
    let assets = snapshot
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut items = Vec::new();
    let mut total_cells = 0usize;
    let mut pending_cells = 0usize;
    let mut error_cells = 0usize;
    let mut partial_cells = 0usize;
    let mut terminal_cells = 0usize;
    let mut matching_cells = 0usize;
    let mut matching_roots = BTreeSet::new();
    let mut selected_roots = BTreeSet::new();

    for asset in &assets {
        let asset_value = asset.get("value").and_then(Value::as_str).unwrap_or("");
        let target_id = asset.get("target_id").and_then(Value::as_str).unwrap_or("");
        let target_type = asset
            .get("target_type")
            .and_then(Value::as_str)
            .unwrap_or("");
        let enumeration_root = is_enumeration
            .then(|| exact_enumeration_root_from_asset(asset))
            .flatten();
        for cell in asset
            .get("coverage")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            total_cells += 1;
            let state = cell
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("pending");
            match state {
                "pending" => pending_cells += 1,
                "error" => error_cells += 1,
                "partial" => partial_cells += 1,
                _ => terminal_cells += 1,
            }
            if cell
                .get("planned_terminal_exception")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                continue;
            }
            if !preferred_states.iter().any(|s| s == state) {
                continue;
            }
            matching_cells += 1;
            if is_enumeration {
                matching_roots.insert(asset_value.to_string());
            }
            if items.len() >= limit {
                continue;
            }
            if is_enumeration
                && !selected_roots.contains(asset_value)
                && selected_roots.len() >= MAX_ENUMERATION_WORKLIST_ROOTS
            {
                continue;
            }
            if is_enumeration {
                selected_roots.insert(asset_value.to_string());
            }
            let technique = cell.get("technique").and_then(Value::as_str).unwrap_or("");
            let details = cell.get("details").cloned();
            let recommended_args = cell.get("recommended_args").cloned().or_else(|| {
                details
                    .as_ref()
                    .and_then(|details| details.get("recommended_args"))
                    .cloned()
            });
            let mut item = json!({
                "work_item_id": format!("{target_id}:{asset_value}:{technique}"),
                "target_id": target_id,
                "asset": asset_value,
                "target_type": target_type,
                "technique": cell.get("technique").cloned().unwrap_or(Value::Null),
                "label": cell.get("label").cloned().unwrap_or(Value::Null),
                "state": state,
                "source": cell.get("source").cloned().unwrap_or(Value::Null),
                "evidence_refs": cell.get("evidence_refs").cloned().unwrap_or_else(|| json!([])),
                "note": cell.get("note").cloned().unwrap_or(Value::Null),
                "suggested_capabilities": cell.get("suggested_capabilities").cloned().unwrap_or_else(|| json!([])),
                "suggested_tools": cell.get("suggested_tools").cloned().unwrap_or_else(|| json!([])),
            });
            if let Some(details) = details {
                item["details"] = details;
            }
            if let Some(recommended_args) = recommended_args {
                item["recommended_args"] = recommended_args;
            }
            if is_enumeration {
                item["worklist_source"] = json!("EAS-confirmed live web root");
                item["enumeration_focus"] = json!(enumeration_gap_focus(technique));
                if let Some((root_url, scheme, port)) = &enumeration_root {
                    item["root_url"] = json!(root_url);
                    item["base_url"] = json!(root_url);
                    item["scheme"] = json!(scheme);
                    item["port"] = json!(port);
                    item["origin_resolution"] = json!("exact");
                } else {
                    item["origin_resolution"] = json!("missing_exact_web_origin");
                }
            } else if is_eas {
                item["eas_focus"] = json!(eas_gap_focus(technique));
            }
            items.push(item);
        }
    }

    let missing_vuln_triage_denominator = stage == "vuln_triage" && total_cells == 0;
    let ready_to_submit = !missing_vuln_triage_denominator
        && pending_cells == 0
        && error_cells == 0
        && partial_cells == 0;
    let omitted_item_count = matching_cells.saturating_sub(items.len());
    let omitted_root_count = matching_roots.len().saturating_sub(selected_roots.len());
    json!({
        "tool": "stage_worklist_next",
        "stage": snapshot.get("stage").cloned().unwrap_or(Value::Null),
        "organization_id": snapshot.get("organization_id").cloned().unwrap_or(Value::Null),
        "session_id": snapshot.get("session_id").cloned().unwrap_or(Value::Null),
        "limit": limit,
        "prefer": preferred_states,
        "ready_to_submit": ready_to_submit,
        "coverage_denominator_missing": missing_vuln_triage_denominator,
        "summary": snapshot.get("summary").cloned().unwrap_or_else(|| json!({})),
        "cell_summary": {
            "total_cells": total_cells,
            "pending_cells": pending_cells,
            "error_cells": error_cells,
            "partial_cells": partial_cells,
            "terminal_cells": terminal_cells,
            "matching_cells": matching_cells,
        },
        "items": items,
        "omitted_item_count": omitted_item_count,
        "root_limit": is_enumeration.then_some(MAX_ENUMERATION_WORKLIST_ROOTS),
        "root_count": selected_roots.len(),
        "matching_root_count": matching_roots.len(),
        "omitted_root_count": omitted_root_count,
        "worklist_contract": if is_eas {
            "Items are derived from DB/gate truth. Close each suggested_capabilities item for the named asset x technique cell; suggested_tools are implementation hints. For WEB fingerprint cells, copy recommended_args.target_urls unchanged into eas_fingerprint_web_stack.target_urls and never rebuild a scheme from a port number. EAS tool split: httpx only for domain/URL/web-origin liveness, naabu/masscan for concrete IP/CIDR port discovery and alive-by-port, nmap -sV for every confirmed open port including newly discovered ports, and whatweb once per confirmed HTTP(S) web origin. Refresh this worklist after tools land DB evidence; do not mark work complete by natural-language assertion."
        } else if is_enumeration {
            "Items are derived from current-run fresh exact-origin technique_outcomes. One response contains at most 200 cells and at most 50 distinct exact-origin roots; deduplicate items by asset, call enum_preflight_web_origins once for those roots, then send only reachable/pending roots to content producers. Resume ordinary partial/error results, but do not retry a producer cell after a persisted recovery_exhausted blocked outcome. Non-empty terminal_exceptions are rejected. Business rows and deliverable prose cannot create terminal outcomes."
        } else {
            "Items are derived from DB/gate truth. Close the suggested_capabilities for each asset x technique cell; suggested_tools are implementation hints. Then call stage_worklist_next/status again; do not mark work complete by natural-language assertion."
        },
        "next_action": if missing_vuln_triage_denominator {
            "Do not submit: vuln_triage returned an empty asset x technique denominator. Refresh the stage coverage snapshot/worklist before submitting; the gate requires formulaic scan cells for each in-scope asset."
        } else if ready_to_submit && is_enumeration {
            "No pending/error/partial Enumeration work items remain. Submit summary claims, findings: [], and coverage: []; current-run producer/preflight/recovery evidence and trusted context own every terminal cell."
        } else if ready_to_submit {
            "No pending/error work items remain. Build a slim StageDeliverable with real evidence refs and submit."
        } else if is_eas {
            "Close the returned EAS cells by technique: domain/URL/web-origin LIVENESS with httpx; concrete IP/CIDR LIVENESS by running PORT discovery first; PORT with naabu/masscan; SERVICE with nmap -sV for every confirmed open port after ports are known; run whatweb once per confirmed HTTP(S) web origin."
        } else if is_enumeration {
            "Close all four exact-origin axes on this returned page: JS with browser_collect_js_api; JSAPI with browser_collect_js_api then js_extract_apis; PARAM from observed requests/forms/query strings and targeted param_hints; DIR with route_probe_paths batch_concurrency=4 after JS/API landing. Refresh the worklist and resume ordinary partials; do not retry a persisted recovery_exhausted blocked cell or infer completion from business rows."
        } else {
            "Close the returned items in order, refresh this worklist, and submit only after ready_to_submit=true."
        }
    })
}

fn compact_stage_asset_coverage(snapshot: Value, max_gaps: usize, include_assets: bool) -> Value {
    let snapshot = normalize_enumeration_snapshot(snapshot);
    let stage = snapshot.get("stage").and_then(Value::as_str).unwrap_or("");
    let is_enumeration = stage == "enumeration";
    let is_eas = stage == "external_attack_surface";
    let assets = snapshot
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut total_cells = 0usize;
    let mut pending_cells = 0usize;
    let mut error_cells = 0usize;
    let mut partial_cells = 0usize;
    let mut blocked_cells = 0usize;
    let mut done_cells = 0usize;
    let mut next_wave_cells = 0usize;
    let mut gaps = Vec::new();

    for asset in &assets {
        let value = asset.get("value").and_then(Value::as_str).unwrap_or("");
        let target_type = asset
            .get("target_type")
            .and_then(Value::as_str)
            .unwrap_or("");
        let target_id = asset.get("target_id").and_then(Value::as_str).unwrap_or("");
        let enumeration_root = is_enumeration
            .then(|| exact_enumeration_root_from_asset(asset))
            .flatten();
        for cell in asset
            .get("coverage")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            total_cells += 1;
            let state = cell
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("pending");
            match state {
                "pending" => pending_cells += 1,
                "error" => error_cells += 1,
                "partial" => partial_cells += 1,
                "next_wave_pending" => next_wave_cells += 1,
                "blocked" => {
                    blocked_cells += 1;
                    done_cells += 1;
                }
                _ => done_cells += 1,
            }
            if matches!(state, "pending" | "error" | "partial") && gaps.len() < max_gaps {
                let technique = cell.get("technique").and_then(Value::as_str).unwrap_or("");
                let mut gap = json!({
                    "target_id": target_id,
                    "asset": value,
                    "target_type": target_type,
                    "technique": cell.get("technique").cloned().unwrap_or(Value::Null),
                    "label": cell.get("label").cloned().unwrap_or(Value::Null),
                    "state": state,
                    "source": cell.get("source").cloned().unwrap_or(Value::Null),
                    "evidence_refs": cell.get("evidence_refs").cloned().unwrap_or_else(|| json!([])),
                    "note": cell.get("note").cloned().unwrap_or(Value::Null),
                    "details": cell.get("details").cloned().unwrap_or(Value::Null),
                    "suggested_capabilities": cell.get("suggested_capabilities").cloned().unwrap_or_else(|| json!([])),
                    "suggested_tools": cell.get("suggested_tools").cloned().unwrap_or_else(|| json!([])),
                });
                if is_enumeration {
                    gap["worklist_source"] = json!("EAS-confirmed live web root");
                    gap["enumeration_focus"] = json!(enumeration_gap_focus(technique));
                    if let Some((root_url, scheme, port)) = &enumeration_root {
                        gap["root_url"] = json!(root_url);
                        gap["base_url"] = json!(root_url);
                        gap["scheme"] = json!(scheme);
                        gap["port"] = json!(port);
                        gap["origin_resolution"] = json!("exact");
                    } else {
                        gap["origin_resolution"] = json!("missing_exact_web_origin");
                    }
                } else if is_eas {
                    gap["eas_focus"] = json!(eas_gap_focus(technique));
                }
                gaps.push(gap);
            }
        }
    }

    let omitted_gap_count = pending_cells + error_cells + partial_cells;
    let omitted_gap_count = omitted_gap_count.saturating_sub(gaps.len());
    let missing_vuln_triage_denominator = stage == "vuln_triage" && total_cells == 0;
    let ready_to_submit = !missing_vuln_triage_denominator
        && pending_cells == 0
        && error_cells == 0
        && partial_cells == 0;
    let next_action = if missing_vuln_triage_denominator {
        "Do not submit: vuln_triage returned an empty asset x technique denominator. Refresh the stage coverage snapshot/worklist before submitting; the gate requires formulaic scan cells for each in-scope asset."
    } else if ready_to_submit {
        if next_wave_cells > 0 {
            if is_enumeration {
                "Current wave has no pending/error/partial cells. Submit summary claims with findings: [] and coverage: []; current-run evidence owns terminal cells. next_wave_pending assets belong to the next stage_run wave."
            } else {
                "Current wave has no pending/error cells. Submit this wave's StageDeliverable with real evidence ids; next_wave_pending assets are newly discovered and should be handled by the next stage_run wave after this wave passes."
            }
        } else if is_enumeration {
            "Enumeration has no pending/error/partial cells on the exact-origin worklist. Submit a slim StageDeliverable: summary claims, findings: [], coverage: []. Producer/preflight/recovery evidence and trusted context own every terminal state."
        } else {
            "Coverage has no pending/error/blocked cells in this preflight. Build the final StageDeliverable with real evidence ids and submit_stage_deliverable."
        }
    } else if is_enumeration {
        "Do not submit yet. Treat gap_examples as the current exact-origin page for this org: close JS with browser_collect_js_api, JSAPI with browser_collect_js_api/js_extract_apis, DIR with route_probe_paths after JS/API landing using target_id + base_url plus batch_concurrency=4, and PARAM from observed browser requests, query strings, forms, and targeted js_extract_apis param_hints. Refresh after outcomes land, resume ordinary partials, and do not retry a persisted recovery_exhausted blocked cell or infer completion from business rows or hand-write found/empty. Do not re-port-scan or default to external directory tools."
    } else if is_eas {
        "Do not submit yet. Close EAS gap_examples by the tool boundary: domain/URL/web-origin LIVENESS uses httpx; concrete IP/CIDR LIVENESS uses naabu/masscan PORT discovery first; SERVICE uses eas_fingerprint_services/nmap -sV for every confirmed open host:port set (use details.missing_open_ports when present). WhatWeb is HTTP(S)-only technology fingerprinting, not a generic service-fingerprint fallback."
    } else {
        "Do not submit yet. Close pending/error/partial cells with the suggested tools. For pending Enumeration roots, run enum_preflight_web_origins first; never hand-author blocked/not_applicable coverage, and honor persisted producer recovery-exhausted blocked outcomes as terminal."
    };
    let mut out = json!({
        "ready_to_submit": ready_to_submit,
        "coverage_denominator_missing": missing_vuln_triage_denominator,
        "stage": snapshot.get("stage").cloned().unwrap_or(Value::Null),
        "organization_id": snapshot.get("organization_id").cloned().unwrap_or(Value::Null),
        "session_id": snapshot.get("session_id").cloned().unwrap_or(Value::Null),
        "summary": snapshot.get("summary").cloned().unwrap_or_else(|| json!({})),
        "cell_summary": {
            "total_cells": total_cells,
            "done_cells": done_cells,
            "pending_cells": pending_cells,
            "error_cells": error_cells,
            "partial_cells": partial_cells,
            "blocked_cells": blocked_cells
            ,"next_wave_cells": next_wave_cells
        },
        "gap_examples": gaps,
        "omitted_gap_count": omitted_gap_count,
        "next_action": next_action
    });
    if is_enumeration {
        let excluded = snapshot
            .get("eas_transport_excluded_origins")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        out["eas_transport_excluded_count"] = json!(excluded.len());
        out["eas_transport_excluded_origins"] =
            Value::Array(excluded.into_iter().take(50).collect());
        out["worklist_semantics"] = json!("Enumeration assets are narrowed to EAS-confirmed live web roots and keyed by exact Web Origin. Only current-run fresh exact-origin technique_outcomes close JS/JSAPI/DIR/PARAM as found or checked_empty; directory_entries/api_endpoints/js_analysis_results are discovery context and cannot close a cell.");
        out["deliverable_contract"] = json!("Submit findings: [] and coverage: []. Fresh exact-origin producer outcomes own found/empty. enum_preflight_web_origins evidence owns all-axis transport blocked; route_probe_paths recovery evidence owns DIR blocked; browser_collect_js_api recovery evidence owns JS/JSAPI/PARAM blocked. Non-web/rootless hosts never enter the denominator, and business rows or prose cannot close cells.");
    } else if is_eas {
        out["worklist_semantics"] = json!("EAS cells are split by asset and technique: domain/URL assets need HTTP liveness; concrete IP/CIDR assets need port discovery first, and fresh open-port/no-open-port evidence closes their LIVENESS. Port discovery should be batch-first with naabu/masscan; service fingerprinting is eas_fingerprint_services/nmap -sV on every confirmed open port, including details.missing_open_ports when present; WEB-FINGERPRINT is WhatWeb per confirmed HTTP(S) origin.");
        out["deliverable_contract"] = json!("Submit only slim terminal coverage the DB cannot derive. DB-derived found domain/URL LIVENESS comes from httpx; concrete IP/CIDR LIVENESS and PORT come from naabu/masscan output-store writes; SERVICE-FINGERPRINT found comes from nmap/port-level service landing for every confirmed open port; WEB-FINGERPRINT comes from WhatWeb web-origin fingerprints. WhatWeb does not replace IP:port SERVICE-FINGERPRINT.");
    }
    if include_assets {
        let asset_count = snapshot
            .get("assets")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or_default();
        out["assets_omitted"] = json!(true);
        out["assets_omitted_count"] = json!(asset_count);
        out["asset_detail_hint"] = json!(
            "Full asset matrices are intentionally omitted from agent preflight output. Use gap_examples and cell_summary; the UI coverage panel can fetch full assets separately."
        );
    }
    out
}

fn enumeration_gap_focus(technique: &str) -> &'static str {
    match technique {
        "GOLISH-ENUM-JS" => {
            "Collect JavaScript assets with browser_collect_js_api and require a fresh exact-origin JS outcome."
        }
        "GOLISH-ENUM-JSAPI" => {
            "Collect browser-observed JS/API first, then run js_extract_apis on saved JS."
        }
        "GOLISH-ENUM-DIR" => {
            "Run route_probe_paths after JS/API landing with target_id + base_url; let it read DB seeds and complete its recursive local/built-in wordlist queue."
        }
        "GOLISH-ENUM-PARAM" => {
            "Derive parameters from observed requests, query strings, forms, and targeted js_extract_apis param_hints."
        }
        _ => "Close this enumeration cell with a real run or an honest terminal note.",
    }
}

fn eas_gap_focus(technique: &str) -> &'static str {
    match technique {
        "GOLISH-EAS-LIVENESS" => {
            "Use httpx only for domain/URL/web-origin liveness; a concrete IP becomes live through port discovery evidence, so run PORT first for IP/CIDR gaps."
        }
        "GOLISH-EAS-PORT" => {
            "Use naabu or masscan first for fast concrete IP/CIDR port discovery; nmap is fallback/verification."
        }
        "GOLISH-EAS-SERVICE-FINGERPRINT" => {
            "Inspect confirmed open ports first, then run nmap -sV grouped by shared port set until every open port has a fingerprint attempt. Use WhatWeb once per confirmed HTTP(S) web origin, never for DNS/MySQL/SSH/non-HTTP service gaps."
        }
        "GOLISH-EAS-WEB-FINGERPRINT" => {
            "For each WEB gap, copy recommended_args.target_urls unchanged into eas_fingerprint_web_stack.target_urls. Preserve every target_id and exact target_url; never rebuild or infer a scheme from a port number."
        }
        _ => "Close this EAS cell with the matching probe and real evidence, or an honest terminal note.",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        compact_stage_asset_coverage, enumeration_web_roots_worklist, filter_rows_to_current_wave,
        in_scope_rows_own_target, preview_terminal_exceptions, resolve_coverage_org_id,
        stage_worklist_next, stage_worklist_status, validate_stage_coverage_read_context,
        web_root_url_from_meta, StageCoverageReadContext,
    };

    #[test]
    fn coverage_org_resolution_rejects_cross_org_override() {
        let bound = uuid::Uuid::new_v4();
        let foreign = uuid::Uuid::new_v4();

        assert_eq!(
            resolve_coverage_org_id(None, Some(bound)).unwrap(),
            Some(bound)
        );
        assert_eq!(
            resolve_coverage_org_id(Some(bound), Some(bound)).unwrap(),
            Some(bound)
        );
        assert!(resolve_coverage_org_id(Some(foreign), Some(bound)).is_err());
        assert_eq!(
            resolve_coverage_org_id(Some(foreign), None).unwrap(),
            Some(foreign)
        );
    }
    use chrono::Utc;
    use serde_json::{json, Value};
    use std::collections::BTreeSet;

    fn terminal_exception_snapshot() -> Value {
        json!({
            "stage": "enumeration",
            "organization_id": "org-current",
            "session_id": "run-current",
            "summary": {"total_assets": 2},
            "assets": [
                {
                    "target_id": "target-a",
                    "value": "https://a.example.com:443",
                    "target_type": "url",
                    "exact_web_origin": true,
                    "coverage": [
                        {"technique": "GOLISH-ENUM-JS", "label": "JS", "state": "pending"},
                        {"technique": "GOLISH-ENUM-DIR", "label": "Directory", "state": "pending"}
                    ]
                },
                {
                    "target_id": "target-b",
                    "value": "https://b.example.com:443",
                    "target_type": "url",
                    "exact_web_origin": true,
                    "coverage": [
                        {"technique": "GOLISH-ENUM-DIR", "label": "Directory", "state": "pending"}
                    ]
                }
            ]
        })
    }

    #[test]
    fn terminal_exception_preview_rejects_nonempty_arrays_without_projection() {
        let original = terminal_exception_snapshot();
        let exceptions = json!([
            {
                "asset": "https://a.example.com:443",
                "technique": "GOLISH-ENUM-JS",
                "status": "blocked",
                "note": "TLS connection times out from the current execution environment"
            },
            {
                "asset": "https://a.example.com:443",
                "technique": "GOLISH-ENUM-DIR",
                "status": "not_applicable",
                "note": "This origin exposes no routable HTTP path in the authorized environment"
            }
        ]);

        let error = preview_terminal_exceptions(original.clone(), Some(&exceptions)).unwrap_err();
        assert!(error.contains("disabled"));
        assert_eq!(original["assets"][0]["coverage"][0]["state"], "pending");
    }

    #[test]
    fn terminal_exception_preview_treats_strict_provider_null_as_omitted() {
        let original = terminal_exception_snapshot();
        let missing = preview_terminal_exceptions(original.clone(), None).unwrap();
        let strict_null = preview_terminal_exceptions(original, Some(&Value::Null)).unwrap();

        assert_eq!(strict_null, missing);
        assert_eq!(strict_null.1["provided_cells"], 0);
        assert_eq!(strict_null.1["accepted_cells"], 0);
        assert_eq!(strict_null.1["coverage_to_submit"], json!([]));

        let non_enumeration = json!({"stage": "target_intel", "assets": []});
        assert!(preview_terminal_exceptions(non_enumeration, Some(&Value::Null)).is_ok());
    }

    #[test]
    fn target_intel_terminal_preview_closes_only_exact_authoritative_cells() {
        let snapshot = json!({
            "stage": "target_intel",
            "organization_id": "org-current",
            "session_id": "run-current",
            "summary": {
                "total_assets": 1,
                "seed_assets": 1,
                "new_assets": 0,
                "done_assets": 0,
                "pending_assets": 1,
                "blocked_assets": 0
            },
            "assets": [{
                "target_id": "target-moresec",
                "value": "moresec.cn",
                "target_type": "domain",
                "coverage": [
                    {"technique": "GOLISH-INTEL-ASN", "state": "pending"},
                    {"technique": "GOLISH-INTEL-CT", "state": "pending"},
                    {"technique": "GOLISH-INTEL-OSINT", "state": "pending"}
                ]
            }]
        });
        let exceptions = json!([
            {
                "asset": "moresec.cn",
                "technique": "GOLISH-INTEL-ASN",
                "status": "blocked",
                "note": "No configured provider declares ASN capability",
                "reason_kind": "provider_missing"
            },
            {
                "asset": "moresec.cn",
                "technique": "GOLISH-INTEL-CT",
                "status": "not_applicable",
                "note": "The selected provider does not expose certificate transparency data",
                "reason_kind": "not_applicable"
            },
            {
                "asset": "moresec.cn",
                "technique": "GOLISH-INTEL-OSINT",
                "status": "checked_empty",
                "evidence_refs": [4]
            }
        ]);

        let (projected, preview) =
            preview_terminal_exceptions(snapshot, Some(&exceptions)).unwrap();
        let compact = compact_stage_asset_coverage(projected.clone(), 25, false);

        assert_eq!(preview["provided_cells"], 3);
        assert_eq!(preview["accepted_cells"], 3);
        assert_eq!(preview["blocked_cells"], 1);
        assert_eq!(preview["not_applicable_cells"], 1);
        assert_eq!(preview["coverage_to_submit"], exceptions);
        assert_eq!(projected["summary"]["blocked_assets"], 1);
        assert_eq!(compact["ready_to_submit"], true);
        assert_eq!(compact["cell_summary"]["pending_cells"], 0);
    }

    #[test]
    fn target_intel_checked_empty_preview_requires_evidence_and_known_cell() {
        let snapshot = json!({
            "stage": "target_intel",
            "assets": [{
                "value": "moresec.cn",
                "coverage": [{"technique": "GOLISH-INTEL-OSINT", "state": "pending"}]
            }]
        });
        let no_evidence = json!([{
            "asset": "moresec.cn",
            "technique": "GOLISH-INTEL-OSINT",
            "status": "checked_empty"
        }]);
        assert!(
            preview_terminal_exceptions(snapshot.clone(), Some(&no_evidence))
                .unwrap_err()
                .contains("requires exact-technique evidence_refs")
        );

        let foreign_asset = json!([{
            "asset": "www.moresec.cn",
            "technique": "GOLISH-INTEL-OSINT",
            "status": "blocked",
            "note": "No provider"
        }]);
        assert!(preview_terminal_exceptions(snapshot, Some(&foreign_asset))
            .unwrap_err()
            .contains("not in the authoritative current worklist"));
    }

    #[test]
    fn eas_web_terminal_preview_cannot_close_remaining_exact_origins() {
        for status in ["blocked", "not_applicable", "checked_empty"] {
            let snapshot = json!({
                "stage": "external_attack_surface",
                "organization_id": "org-current",
                "session_id": "run-current",
                "summary": {"total_assets": 1, "pending_assets": 1},
                "assets": [{
                    "target_id": "target-ip",
                    "value": "113.240.117.106",
                    "target_type": "ip",
                    "coverage": [{
                        "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                        "state": "partial",
                        "details": {
                            "required_origins": ["https://113.240.117.106:443"],
                            "completed_origins": [],
                            "missing_origins": ["https://113.240.117.106:443"]
                        }
                    }]
                }]
            });
            let exception = if status == "checked_empty" {
                json!([{
                    "asset": "113.240.117.106",
                    "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                    "status": status,
                    "evidence_refs": [41]
                }])
            } else {
                json!([{
                    "asset": "113.240.117.106",
                    "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                    "status": status,
                    "note": "The parent asset cell cannot replace exact-origin completion"
                }])
            };

            let (projected, preview) =
                preview_terminal_exceptions(snapshot, Some(&exception)).unwrap();
            let compact = compact_stage_asset_coverage(projected.clone(), 25, false);
            let worklist = stage_worklist_next(
                projected,
                25,
                &[
                    "pending".to_string(),
                    "error".to_string(),
                    "partial".to_string(),
                ],
            );

            assert_eq!(preview["accepted_cells"], 0, "status={status}");
            assert_eq!(preview["rejected_cells"], 1, "status={status}");
            assert_eq!(preview["coverage_to_submit"], json!([]), "status={status}");
            assert!(preview["rejected_terminal_exceptions"][0]["reason"]
                .as_str()
                .unwrap()
                .contains("missing exact Web origins"));
            assert_eq!(compact["ready_to_submit"], false, "status={status}");
            assert_eq!(
                compact["cell_summary"]["partial_cells"], 1,
                "status={status}"
            );
            assert_eq!(
                compact["gap_examples"][0]["details"]["missing_origins"],
                json!(["https://113.240.117.106:443"]),
                "status={status}"
            );
            assert_eq!(worklist["ready_to_submit"], false, "status={status}");
            assert_eq!(
                worklist["items"][0]["technique"], "GOLISH-EAS-WEB-FINGERPRINT",
                "status={status}"
            );
        }
    }

    #[test]
    fn eas_non_web_terminal_preview_still_closes_an_honest_exception() {
        let snapshot = json!({
            "stage": "external_attack_surface",
            "organization_id": "org-current",
            "session_id": "run-current",
            "summary": {"total_assets": 1, "pending_assets": 1},
            "assets": [{
                "target_id": "target-ip",
                "value": "113.240.117.106",
                "target_type": "ip",
                "coverage": [{
                    "technique": "GOLISH-EAS-SERVICE-FINGERPRINT",
                    "state": "pending",
                    "details": {"missing_open_ports": [9443]}
                }]
            }]
        });
        let exception = json!([{
            "asset": "113.240.117.106",
            "technique": "GOLISH-EAS-SERVICE-FINGERPRINT",
            "status": "blocked",
            "note": "The authorized scanner runtime cannot load the required service probe"
        }]);

        let (projected, preview) = preview_terminal_exceptions(snapshot, Some(&exception)).unwrap();
        let compact = compact_stage_asset_coverage(projected, 25, false);

        assert_eq!(preview["accepted_cells"], 1);
        assert_eq!(preview["rejected_cells"], 0);
        assert_eq!(preview["rejected_terminal_exceptions"], json!([]));
        assert_eq!(preview["coverage_to_submit"], exception);
        assert_eq!(compact["ready_to_submit"], true);
        assert_eq!(compact["cell_summary"]["blocked_cells"], 1);
    }

    #[test]
    fn enumeration_worklist_caps_one_page_at_fifty_distinct_roots_and_two_hundred_cells() {
        let techniques = [
            "GOLISH-ENUM-JS",
            "GOLISH-ENUM-DIR",
            "GOLISH-ENUM-PARAM",
            "GOLISH-ENUM-JSAPI",
        ];
        let assets = (0..60)
            .map(|index| {
                json!({
                    "target_id": format!("target-{index}"),
                    "value": format!("https://host-{index}.example.com:443"),
                    "target_type": "url",
                    "exact_web_origin": true,
                    "coverage": techniques
                        .iter()
                        .map(|technique| json!({"technique": technique, "state": "pending"}))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();

        let worklist = stage_worklist_next(
            json!({
                "stage": "enumeration",
                "organization_id": "org-current",
                "session_id": "run-current",
                "assets": assets
            }),
            200,
            &["pending".to_string()],
        );

        assert_eq!(worklist["items"].as_array().unwrap().len(), 200);
        assert_eq!(worklist["cell_summary"]["matching_cells"], 240);
        assert_eq!(worklist["omitted_item_count"], 40);
        assert_eq!(worklist["root_limit"], 50);
        assert_eq!(worklist["root_count"], 50);
        assert_eq!(worklist["matching_root_count"], 60);
        assert_eq!(worklist["omitted_root_count"], 10);
        let returned_roots = worklist["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["asset"].as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(returned_roots.len(), 50);
        assert!(worklist["worklist_contract"]
            .as_str()
            .unwrap()
            .contains("at most 50 distinct exact-origin roots"));
    }

    #[test]
    fn enumeration_worklist_includes_pending_exact_ip_origin_cells() {
        let origin = "https://203.0.113.10:443";
        let worklist = stage_worklist_next(
            json!({
                "stage": "enumeration",
                "organization_id": "org-current",
                "session_id": "run-current",
                "assets": [{
                    "target_id": "target-ip",
                    "value": origin,
                    "target_type": "url",
                    "exact_web_origin": true,
                    "coverage": [
                        {"technique": "GOLISH-ENUM-JS", "state": "pending"},
                        {"technique": "GOLISH-ENUM-DIR", "state": "pending"},
                        {"technique": "GOLISH-ENUM-PARAM", "state": "pending"},
                        {"technique": "GOLISH-ENUM-JSAPI", "state": "pending"}
                    ]
                }]
            }),
            200,
            &["pending".to_string()],
        );

        assert_eq!(worklist["ready_to_submit"], false);
        let items = worklist["items"].as_array().unwrap();
        assert_eq!(items.len(), 4);
        assert!(items.iter().all(|item| item["asset"] == origin));
        assert!(items
            .iter()
            .all(|item| item["root_url"] == "https://203.0.113.10:443/"));
    }

    #[test]
    fn enumeration_worklists_require_active_current_run_context() {
        let missing_cutoff = StageCoverageReadContext::default();
        assert!(validate_stage_coverage_read_context(
            "enumeration",
            Some("run-current"),
            &missing_cutoff,
        )
        .is_err());

        let active = StageCoverageReadContext {
            stage_started_at: Some(Utc::now()),
            ..StageCoverageReadContext::default()
        };
        assert!(validate_stage_coverage_read_context("enumeration", None, &active).is_err());
        assert!(validate_stage_coverage_read_context("enumeration", Some("   "), &active).is_err());
        assert!(
            validate_stage_coverage_read_context("enumeration", Some("run-current"), &active,)
                .is_ok()
        );

        // This P0 only tightens Enumeration. Other stages retain their current
        // read-context behaviour; in particular, do not silently alter EAS wave
        // semantics while fixing the Enumeration worklist.
        assert!(validate_stage_coverage_read_context(
            "external_attack_surface",
            None,
            &missing_cutoff,
        )
        .is_ok());

        let invalid_wave = StageCoverageReadContext {
            wave_error: Some("running asset wave has no items".to_string()),
            ..StageCoverageReadContext::default()
        };
        assert!(validate_stage_coverage_read_context(
            "external_attack_surface",
            None,
            &invalid_wave,
        )
        .is_err());
    }

    #[test]
    fn current_wave_filter_limits_listing_rows_to_target_ids() {
        let seed_id = uuid::Uuid::from_u128(1);
        let delta_id = uuid::Uuid::from_u128(2);
        let same_value_other_id = uuid::Uuid::from_u128(3);
        let rows = vec![
            json!({"id": seed_id, "value": "seed.example.com"}),
            json!({"id": delta_id, "value": "same.example.com"}),
            json!({"id": same_value_other_id, "value": "same.example.com"}),
        ];
        let current = BTreeSet::from([delta_id]);

        let filtered = filter_rows_to_current_wave(rows, Some(&current));

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0]["id"], delta_id.to_string());
    }

    #[test]
    fn coverage_preflight_blocks_submit_when_cells_are_pending() {
        let compact = compact_stage_asset_coverage(
            json!({
                "stage": "external_attack_surface",
                "organization_id": "org-1",
                "session_id": "sess",
                "summary": {"total_assets": 1, "done_assets": 0, "pending_assets": 1, "blocked_assets": 0},
                "assets": [{
                    "target_id": "target-1",
                    "value": "example.com",
                    "target_type": "domain",
                    "coverage": [
                        {"technique": "GOLISH-EAS-LIVENESS", "label": "Liveness", "state": "found", "evidence_refs": [], "suggested_tools": []},
                        {"technique": "GOLISH-EAS-PORT", "label": "Port", "state": "pending", "evidence_refs": [], "suggested_tools": ["naabu", "nmap"]}
                    ]
                }]
            }),
            10,
            false,
        );

        assert_eq!(compact["ready_to_submit"], false);
        assert_eq!(compact["cell_summary"]["pending_cells"], 1);
        assert_eq!(compact["gap_examples"][0]["asset"], "example.com");
        assert!(compact["gap_examples"][0]["eas_focus"]
            .as_str()
            .unwrap()
            .contains("naabu"));
        assert!(compact["next_action"]
            .as_str()
            .unwrap()
            .contains("WhatWeb is HTTP(S)-only"));
        assert!(compact["worklist_semantics"]
            .as_str()
            .unwrap()
            .contains("nmap -sV on every confirmed open port"));
        assert!(compact.get("assets").is_none());
    }

    #[test]
    fn coverage_preflight_preserves_gap_details() {
        let compact = compact_stage_asset_coverage(
            json!({
                "stage": "external_attack_surface",
                "organization_id": "org-1",
                "session_id": "sess",
                "summary": {"total_assets": 1},
                "assets": [{
                    "target_id": "target-1",
                    "value": "222.186.129.58",
                    "target_type": "ip",
                    "coverage": [
                        {
                            "technique": "GOLISH-EAS-SERVICE-FINGERPRINT",
                            "label": "Service Fingerprint",
                            "state": "pending",
                            "evidence_refs": [],
                            "note": "confirmed open port(s) still need service fingerprinting: 82",
                            "details": {
                                "missing_open_ports": [82],
                                "recommended_tool": "eas_fingerprint_services",
                                "recommended_args": {"targets": ["222.186.129.58"], "ports": "82"}
                            },
                            "suggested_tools": ["eas_fingerprint_services"]
                        }
                    ]
                }]
            }),
            10,
            false,
        );

        assert_eq!(
            compact["gap_examples"][0]["details"]["missing_open_ports"],
            json!([82])
        );
        assert!(compact["next_action"]
            .as_str()
            .unwrap()
            .contains("details.missing_open_ports"));
    }

    #[test]
    fn coverage_preflight_omits_full_assets_even_when_requested() {
        let compact = compact_stage_asset_coverage(
            json!({
                "summary": {},
                "assets": [{
                    "target_id": "target-1",
                    "value": "example.com",
                    "target_type": "domain",
                    "coverage": [
                        {"technique": "GOLISH-EAS-LIVENESS", "label": "Liveness", "state": "found", "evidence_refs": [], "suggested_tools": []}
                    ]
                }]
            }),
            10,
            true,
        );

        assert_eq!(compact["ready_to_submit"], true);
        assert!(compact.get("assets").is_none());
        assert_eq!(compact["assets_omitted"], true);
        assert_eq!(compact["assets_omitted_count"], 1);
    }

    #[test]
    fn vuln_triage_preflight_does_not_pass_empty_denominator() {
        let compact = compact_stage_asset_coverage(
            json!({
                "stage": "vuln_triage",
                "organization_id": "org-1",
                "session_id": "sess",
                "summary": {"total_assets": 0},
                "assets": []
            }),
            10,
            false,
        );

        assert_eq!(compact["ready_to_submit"], false);
        assert_eq!(compact["coverage_denominator_missing"], true);
        assert!(compact["next_action"]
            .as_str()
            .unwrap()
            .contains("empty asset x technique denominator"));

        let worklist = stage_worklist_next(
            json!({
                "stage": "vuln_triage",
                "organization_id": "org-1",
                "session_id": "sess",
                "summary": {"total_assets": 0},
                "assets": []
            }),
            10,
            &["pending".to_string(), "error".to_string()],
        );

        assert_eq!(worklist["ready_to_submit"], false);
        assert_eq!(worklist["coverage_denominator_missing"], true);
        assert_eq!(worklist["next_action"], compact["next_action"]);
    }

    #[test]
    fn coverage_preflight_treats_blocked_as_terminal() {
        let compact = compact_stage_asset_coverage(
            json!({
                "summary": {},
                "assets": [{
                    "target_id": "target-1",
                    "value": "example.com",
                    "target_type": "domain",
                    "coverage": [
                        {"technique": "GOLISH-EAS-PORT", "label": "Port", "state": "blocked", "evidence_refs": [1], "suggested_tools": []}
                    ]
                }]
            }),
            10,
            false,
        );

        assert_eq!(compact["ready_to_submit"], true);
        assert_eq!(compact["cell_summary"]["blocked_cells"], 1);
        assert_eq!(compact["gap_examples"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn coverage_preflight_does_not_block_on_next_wave_pending_cells() {
        let compact = compact_stage_asset_coverage(
            json!({
                "summary": {"total_assets": 1, "new_assets": 1},
                "assets": [
                    {
                        "target_id": "target-1",
                        "value": "seed.example.com",
                        "target_type": "domain",
                        "discovered_phase": "seed",
                        "coverage": [
                            {"technique": "GOLISH-EAS-LIVENESS", "label": "Liveness", "state": "found", "evidence_refs": [1], "suggested_tools": []}
                        ]
                    },
                    {
                        "target_id": "target-2",
                        "value": "new.example.com",
                        "target_type": "domain",
                        "discovered_phase": "new_in_stage",
                        "coverage": [
                            {"technique": "GOLISH-EAS-LIVENESS", "label": "Liveness", "state": "next_wave_pending", "evidence_refs": [], "suggested_tools": []}
                        ]
                    }
                ]
            }),
            10,
            false,
        );

        assert_eq!(compact["ready_to_submit"], true);
        assert_eq!(compact["cell_summary"]["next_wave_cells"], 1);
        assert_eq!(compact["gap_examples"].as_array().unwrap().len(), 0);
        assert!(compact["next_action"]
            .as_str()
            .unwrap()
            .contains("next_wave_pending"));
    }

    #[test]
    fn enumeration_preflight_surfaces_worklist_contract() {
        let compact = compact_stage_asset_coverage(
            json!({
                "stage": "enumeration",
                "summary": {"total_assets": 1, "done_assets": 0, "pending_assets": 1, "blocked_assets": 0},
                "assets": [{
                    "target_id": "target-1",
                    "value": "https://app.example.com",
                    "target_type": "url",
                    "exact_web_origin": true,
                    "coverage": [
                        {"technique": "GOLISH-ENUM-JSAPI", "label": "JS/API", "state": "pending", "evidence_refs": [], "suggested_tools": ["browser_collect_js_api", "js_extract_apis"]}
                    ]
                }]
            }),
            10,
            false,
        );

        assert_eq!(compact["ready_to_submit"], false);
        assert_eq!(
            compact["gap_examples"][0]["worklist_source"],
            "EAS-confirmed live web root"
        );
        assert!(compact["gap_examples"][0]["enumeration_focus"]
            .as_str()
            .unwrap()
            .contains("browser-observed JS/API"));
        assert!(compact["worklist_semantics"]
            .as_str()
            .unwrap()
            .contains("EAS-confirmed live web roots"));
        assert!(compact["worklist_semantics"]
            .as_str()
            .unwrap()
            .contains("exact Web Origin"));
        assert!(compact["deliverable_contract"]
            .as_str()
            .unwrap()
            .contains("Submit findings: [] and coverage: []"));
        assert!(compact["deliverable_contract"]
            .as_str()
            .unwrap()
            .contains("Fresh exact-origin producer outcomes"));
        assert!(compact["deliverable_contract"]
            .as_str()
            .unwrap()
            .contains("enum_preflight_web_origins evidence owns all-axis transport blocked"));
        assert!(compact["deliverable_contract"]
            .as_str()
            .unwrap()
            .contains("route_probe_paths recovery evidence owns DIR blocked"));
        assert!(compact["deliverable_contract"]
            .as_str()
            .unwrap()
            .contains("browser_collect_js_api recovery evidence owns JS/JSAPI/PARAM blocked"));
        assert!(!compact["deliverable_contract"]
            .as_str()
            .unwrap()
            .contains("DB-derived found cells come from directory_entries/api_endpoints"));
        assert!(compact["next_action"]
            .as_str()
            .unwrap()
            .contains("Do not re-port-scan"));
        assert!(compact["next_action"]
            .as_str()
            .unwrap()
            .contains("close JS with browser_collect_js_api"));
    }

    #[test]
    fn enumeration_preflight_keeps_partial_cell_unfinished() {
        let compact = compact_stage_asset_coverage(
            json!({
                "stage": "enumeration",
                "summary": {"total_assets": 1, "done_assets": 0, "pending_assets": 1, "blocked_assets": 0},
                "assets": [{
                    "target_id": "target-1",
                    "value": "https://app.example.com:443",
                    "target_type": "url",
                    "exact_web_origin": true,
                    "coverage": [
                        {"technique": "GOLISH-ENUM-DIR", "label": "Directory", "state": "partial", "evidence_refs": [], "suggested_tools": ["route_probe_paths"]}
                    ]
                }]
            }),
            10,
            false,
        );

        assert_eq!(compact["ready_to_submit"], false);
        assert_eq!(compact["cell_summary"]["partial_cells"], 1);
        assert_eq!(compact["gap_examples"][0]["state"], "partial");
    }

    #[test]
    fn enumeration_worklist_orders_alive_roots_first_within_limit() {
        // PR-C: with a tight limit, EAS-proven-alive roots must survive over
        // unproven ones (order by http_status/ports before truncation).
        let worklist = enumeration_web_roots_worklist(
            json!({
                "stage": "enumeration",
                "organization_id": "org-1",
                "assets": [
                    {"target_id": "t-dead", "value": "https://dead.example.com:443", "target_type": "url", "exact_web_origin": true, "coverage": []},
                    {"target_id": "t-alive", "value": "https://alive.example.com:443", "target_type": "url", "exact_web_origin": true, "http_status": 200, "coverage": []}
                ]
            }),
            1,
            false,
        );
        assert_eq!(worklist["count"], 1);
        assert_eq!(
            worklist["web_roots"][0]["asset"],
            "https://alive.example.com:443"
        );
        assert_eq!(worklist["truncated"], true);
    }

    #[test]
    fn web_root_url_from_meta_derives_scheme_and_port() {
        // PR-A: full URL derivation from EAS metadata (kills per-target probe).
        // https port → https, non-default port kept in suffix.
        let (url, scheme, port) = web_root_url_from_meta(
            "app.example.com",
            Some(200),
            &json!([{"port": 8443, "service": "https"}]),
            "",
        );
        assert_eq!(url, "https://app.example.com:8443/");
        assert_eq!(scheme, "https");
        assert_eq!(port, Some(8443));

        // Plain http on default 80 → no port suffix.
        let (url, scheme, _) = web_root_url_from_meta(
            "plain.example.com",
            Some(200),
            &json!([{"port": 80, "service": "http"}]),
            "",
        );
        assert_eq!(url, "http://plain.example.com/");
        assert_eq!(scheme, "http");

        // Value already carrying a scheme is normalised as-is.
        let (url, scheme, _) =
            web_root_url_from_meta("https://api.example.com", None, &Value::Null, "");
        assert_eq!(url, "https://api.example.com:443/");
        assert_eq!(scheme, "https");

        // No metadata → http fallback, no port.
        let (url, _, port) = web_root_url_from_meta("bare.example.com", None, &json!([]), "");
        assert_eq!(url, "http://bare.example.com/");
        assert_eq!(port, None);
    }

    #[test]
    fn web_root_url_from_meta_prefers_confirmed_open_url_over_filtered_default_port() {
        let (url, scheme, port) = web_root_url_from_meta(
            "43.248.78.209",
            Some(200),
            &json!([
                {
                    "port": 443,
                    "service": "https",
                    "state": "filtered",
                    "url": "https://43.248.78.209/"
                },
                {
                    "port": 8080,
                    "service": "http",
                    "state": "open",
                    "url": "http://43.248.78.209:8080"
                }
            ]),
            "",
        );

        assert_eq!(url, "http://43.248.78.209:8080/");
        assert_eq!(scheme, "http");
        assert_eq!(port, Some(8080));
    }

    #[test]
    fn enumeration_web_roots_worklist_returns_live_root_contract() {
        let worklist = enumeration_web_roots_worklist(
            json!({
                "stage": "enumeration",
                "organization_id": "org-1",
                "session_id": "sess",
                "assets": [
                    {
                        "target_id": "target-1",
                        "value": "https://app.example.com:443",
                        "target_type": "url",
                        "exact_web_origin": true,
                        "organization_id": "org-1",
                        "coverage": [
                            {"technique": "GOLISH-ENUM-JS", "label": "JS", "state": "pending", "suggested_tools": ["browser_collect_js_api"]},
                            {"technique": "GOLISH-ENUM-JSAPI", "label": "JS/API", "state": "pending", "suggested_tools": ["browser_collect_js_api", "js_extract_apis"]},
                            {"technique": "GOLISH-ENUM-DIR", "label": "Directory", "state": "found", "suggested_tools": []}
                        ]
                    }
                ]
            }),
            10,
            true,
        );

        assert_eq!(worklist["count"], 1);
        assert_eq!(
            worklist["web_roots"][0]["root_url"],
            "https://app.example.com:443/"
        );
        assert_eq!(worklist["web_roots"][0]["scheme"], "https");
        assert!(worklist["web_roots"][0]["pending_techniques"]
            .as_array()
            .unwrap()
            .iter()
            .any(|technique| technique == "GOLISH-ENUM-JS"));
        assert!(worklist["web_roots"][0]["pending_techniques"]
            .as_array()
            .unwrap()
            .iter()
            .any(|technique| technique == "GOLISH-ENUM-JSAPI"));
        assert!(worklist["web_roots"][0]["suggested_tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "browser_collect_js_api"));
        assert!(worklist["execution_order"]
            .as_array()
            .unwrap()
            .iter()
            .any(|step| step
                .as_str()
                .is_some_and(|text| text.contains("enum_crawl_same_origin_urls"))));
        assert!(worklist["web_roots"][0]["next_steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|step| step
                .as_str()
                .is_some_and(|text| text.contains("GOLISH-ENUM-JS"))));
        assert!(worklist["tool_boundary"]
            .as_str()
            .unwrap()
            .contains("raw katana/pentest_run"));
    }

    #[test]
    fn enumeration_web_roots_never_return_rootless_non_denominator_assets() {
        let worklist = enumeration_web_roots_worklist(
            json!({
                "stage": "enumeration",
                "organization_id": "org-1",
                "session_id": "run-current",
                "assets": [
                    {
                        "target_id": "target-bare",
                        "value": "alive-but-bare.example.com",
                        "target_type": "domain",
                        "exact_web_origin": false,
                        "coverage": [
                            {"technique": "GOLISH-ENUM-JS", "state": "pending"}
                        ]
                    },
                    {
                        "target_id": "target-origin",
                        "value": "HTTPS://App.Example.com/login",
                        "target_type": "url",
                        "exact_web_origin": true,
                        "coverage": [
                            {"technique": "GOLISH-ENUM-JS", "state": "pending"}
                        ]
                    }
                ]
            }),
            10,
            true,
        );

        assert_eq!(worklist["total"], 1);
        assert_eq!(worklist["count"], 1);
        assert_eq!(
            worklist["web_roots"][0]["root_url"],
            "https://app.example.com:443/"
        );
        assert_eq!(worklist["web_roots"][0]["needs_probe"], false);
    }

    #[test]
    fn enumeration_status_next_roots_and_gate_share_one_canonical_origin_axis() {
        let snapshot = json!({
            "stage": "enumeration",
            "organization_id": "org-1",
            "session_id": "run-current",
            "summary": {"total_assets": 3, "done_assets": 1, "pending_assets": 2},
            "assets": [
                {
                    "target_id": "target-http",
                    "value": "http://App.Example.com/path",
                    "target_type": "url",
                    "exact_web_origin": true,
                    "coverage": [{"technique": "GOLISH-ENUM-JS", "state": "pending"}]
                },
                {
                    "target_id": "target-https",
                    "value": "HTTPS://App.Example.com/login",
                    "target_type": "url",
                    "exact_web_origin": true,
                    "coverage": [{"technique": "GOLISH-ENUM-DIR", "state": "pending"}]
                },
                {
                    "target_id": "target-rootless",
                    "value": "222.186.129.58",
                    "target_type": "ip",
                    "exact_web_origin": false,
                    "coverage": [
                        {"technique": "GOLISH-ENUM-JS", "state": "not_applicable"},
                        {"technique": "GOLISH-ENUM-DIR", "state": "not_applicable"},
                        {"technique": "GOLISH-ENUM-PARAM", "state": "not_applicable"},
                        {"technique": "GOLISH-ENUM-JSAPI", "state": "not_applicable"}
                    ]
                }
            ]
        });

        let status = stage_worklist_status(snapshot.clone());
        let next = stage_worklist_next(snapshot.clone(), 10, &["pending".to_string()]);
        let roots = enumeration_web_roots_worklist(snapshot.clone(), 10, true);
        let (gate_assets, gate_typed_assets) =
            crate::harness::org_gate::enumeration_axis_from_coverage_snapshot(&snapshot);

        let expected = BTreeSet::from([
            "http://app.example.com:80".to_string(),
            "https://app.example.com:443".to_string(),
        ]);
        let next_assets = next["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["asset"].as_str().map(str::to_string))
            .collect::<BTreeSet<_>>();
        let root_assets = roots["web_roots"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|root| root["asset"].as_str().map(str::to_string))
            .collect::<BTreeSet<_>>();

        assert_eq!(status["summary"]["total_assets"], 2);
        assert_eq!(status["summary"]["done_assets"], 0);
        assert_eq!(status["cell_summary"]["total_cells"], 2);
        assert_eq!(next["cell_summary"]["total_cells"], 2);
        assert_eq!(roots["total"], 2);
        assert_eq!(next_assets, expected);
        assert_eq!(root_assets, expected);
        assert_eq!(gate_assets.into_iter().collect::<BTreeSet<_>>(), expected);
        assert!(gate_typed_assets
            .iter()
            .all(|(_, target_type)| target_type == "url"));
    }

    #[test]
    fn enumeration_preflight_gap_examples_include_base_url() {
        let compact = compact_stage_asset_coverage(
            json!({
                "stage": "enumeration",
                "summary": {"total_assets": 1, "done_assets": 0, "pending_assets": 1, "blocked_assets": 0},
                "assets": [{
                    "target_id": "target-1",
                    "value": "http://43.248.78.209:8080",
                    "target_type": "url",
                    "exact_web_origin": true,
                    "ports": [
                        {"port": 443, "service": "https", "state": "filtered", "url": "https://43.248.78.209/"},
                        {"port": 8080, "service": "http", "state": "open", "url": "http://43.248.78.209:8080"}
                    ],
                    "coverage": [
                        {"technique": "GOLISH-ENUM-DIR", "label": "Directory", "state": "error", "evidence_refs": [], "suggested_tools": ["route_probe_paths"]}
                    ]
                }]
            }),
            10,
            false,
        );

        assert_eq!(
            compact["gap_examples"][0]["base_url"],
            "http://43.248.78.209:8080/"
        );
        assert_eq!(
            compact["gap_examples"][0]["root_url"],
            "http://43.248.78.209:8080/"
        );
    }

    #[test]
    fn stage_worklist_next_returns_only_preferred_gap_cells() {
        let worklist = stage_worklist_next(
            json!({
                "stage": "enumeration",
                "organization_id": "org-1",
                "session_id": "sess",
                "summary": {"total_assets": 1},
                "assets": [{
                    "target_id": "target-1",
                    "value": "https://app.example.com",
                    "target_type": "url",
                    "exact_web_origin": true,
                    "coverage": [
                        {"technique": "GOLISH-ENUM-JSAPI", "label": "API", "state": "pending", "suggested_tools": ["browser_collect_js_api"]},
                        {"technique": "GOLISH-ENUM-DIR", "label": "Directory", "state": "error", "suggested_tools": ["route_probe_paths"]},
                        {"technique": "GOLISH-ENUM-PARAM", "label": "Parameters", "state": "found", "suggested_tools": []}
                    ]
                }]
            }),
            1,
            &["pending".to_string(), "error".to_string()],
        );

        assert_eq!(worklist["ready_to_submit"], false);
        assert_eq!(worklist["cell_summary"]["matching_cells"], 2);
        assert_eq!(worklist["items"].as_array().unwrap().len(), 1);
        assert_eq!(worklist["omitted_item_count"], 1);
        assert_eq!(worklist["items"][0]["technique"], "GOLISH-ENUM-JSAPI");
        assert!(worklist["items"][0]["enumeration_focus"]
            .as_str()
            .unwrap()
            .contains("browser-observed JS/API"));
    }

    #[test]
    fn enumeration_partial_origins_stay_unfinished_with_distinct_work_items() {
        let worklist = stage_worklist_next(
            json!({
                "stage": "enumeration",
                "organization_id": "org-1",
                "session_id": "sess",
                "summary": {"total_assets": 2},
                "assets": [
                    {
                        "target_id": "target-1",
                        "value": "http://app.example.com:80",
                        "target_type": "url",
                        "exact_web_origin": true,
                        "coverage": [
                            {"technique": "GOLISH-ENUM-DIR", "label": "Directory", "state": "partial", "suggested_tools": ["route_probe_paths"]}
                        ]
                    },
                    {
                        "target_id": "target-1",
                        "value": "https://app.example.com:443",
                        "target_type": "url",
                        "exact_web_origin": true,
                        "coverage": [
                            {"technique": "GOLISH-ENUM-DIR", "label": "Directory", "state": "partial", "suggested_tools": ["route_probe_paths"]}
                        ]
                    }
                ]
            }),
            10,
            &[
                "pending".to_string(),
                "error".to_string(),
                "partial".to_string(),
            ],
        );

        assert_eq!(worklist["ready_to_submit"], false);
        assert_eq!(worklist["cell_summary"]["partial_cells"], 2);
        let ids = worklist["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["work_item_id"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), 2);
        assert!(ids
            .iter()
            .any(|id| id.contains("http://app.example.com:80")));
        assert!(ids
            .iter()
            .any(|id| id.contains("https://app.example.com:443")));
    }

    #[test]
    fn stage_worklist_next_includes_base_url_for_enumeration_items() {
        let worklist = stage_worklist_next(
            json!({
                "stage": "enumeration",
                "organization_id": "org-1",
                "session_id": "sess",
                "summary": {"total_assets": 1},
                "assets": [{
                    "target_id": "target-1",
                    "value": "http://43.248.78.209:8080",
                    "target_type": "url",
                    "exact_web_origin": true,
                    "ports": [
                        {"port": 443, "service": "https", "state": "filtered", "url": "https://43.248.78.209/"},
                        {"port": 8080, "service": "http", "state": "open", "url": "http://43.248.78.209:8080"}
                    ],
                    "coverage": [
                        {"technique": "GOLISH-ENUM-DIR", "label": "Directory", "state": "error", "suggested_tools": ["route_probe_paths"]}
                    ]
                }]
            }),
            10,
            &["error".to_string()],
        );

        assert_eq!(
            worklist["items"][0]["base_url"],
            "http://43.248.78.209:8080/"
        );
        assert_eq!(
            worklist["items"][0]["root_url"],
            "http://43.248.78.209:8080/"
        );
    }

    #[test]
    fn unresolved_enumeration_asset_is_excluded_from_the_content_worklist() {
        let worklist = stage_worklist_next(
            json!({
                "stage": "enumeration",
                "organization_id": "org-1",
                "session_id": "sess",
                "summary": {"total_assets": 1},
                "assets": [{
                    "target_id": "target-1",
                    "value": "unresolved.example.com",
                    "target_type": "domain",
                    "exact_web_origin": false,
                    "coverage": [
                        {"technique": "GOLISH-ENUM-DIR", "label": "Directory", "state": "pending", "suggested_tools": ["route_probe_paths"]}
                    ]
                }]
            }),
            10,
            &["pending".to_string()],
        );

        assert_eq!(worklist["cell_summary"]["total_cells"], 0);
        assert!(worklist["items"].as_array().unwrap().is_empty());
        assert_eq!(worklist["ready_to_submit"], true);
    }

    #[test]
    fn stage_worklist_next_surfaces_eas_tool_boundary() {
        let worklist = stage_worklist_next(
            json!({
                "stage": "external_attack_surface",
                "organization_id": "org-1",
                "session_id": "sess",
                "summary": {"total_assets": 1},
                "assets": [{
                    "target_id": "target-1",
                    "value": "118.31.21.136",
                    "target_type": "ip",
                    "coverage": [
                        {"technique": "GOLISH-EAS-SERVICE-FINGERPRINT", "label": "Service", "state": "pending", "suggested_tools": ["nmap"]}
                    ]
                }]
            }),
            10,
            &["pending".to_string()],
        );

        assert_eq!(worklist["items"][0]["suggested_tools"][0], "nmap");
        assert!(worklist["items"][0]["eas_focus"]
            .as_str()
            .unwrap()
            .contains("WhatWeb once per confirmed HTTP(S) web origin"));
        assert!(worklist["worklist_contract"]
            .as_str()
            .unwrap()
            .contains("naabu/masscan"));
        assert!(worklist["worklist_contract"]
            .as_str()
            .unwrap()
            .contains("httpx only for domain/URL/web-origin liveness"));
        assert!(worklist["next_action"]
            .as_str()
            .unwrap()
            .contains("concrete IP/CIDR LIVENESS by running PORT discovery first"));
        assert!(worklist["next_action"]
            .as_str()
            .unwrap()
            .contains("SERVICE with nmap -sV"));
    }

    #[test]
    fn stage_worklist_next_preserves_eas_web_exact_origin_arguments() {
        let recommended_args = json!({
            "target_urls": [{
                "target_id": "target-1",
                "target_url": "https://113.240.117.106:443"
            }]
        });
        let details = json!({
            "required_origins": ["https://113.240.117.106:443"],
            "completed_origins": [],
            "missing_origins": ["https://113.240.117.106:443"],
            "recommended_tool": "eas_fingerprint_web_stack",
            "recommended_args": recommended_args.clone()
        });
        let worklist = stage_worklist_next(
            json!({
                "stage": "external_attack_surface",
                "organization_id": "org-1",
                "session_id": "sess",
                "summary": {"total_assets": 1},
                "assets": [{
                    "target_id": "target-1",
                    "value": "113.240.117.106",
                    "target_type": "ip",
                    "coverage": [{
                        "technique": "GOLISH-EAS-WEB-FINGERPRINT",
                        "label": "Web Fingerprint",
                        "state": "pending",
                        "details": details.clone(),
                        "suggested_tools": ["eas_fingerprint_web_stack"]
                    }]
                }]
            }),
            10,
            &["pending".to_string()],
        );

        assert_eq!(worklist["items"][0]["details"], details);
        assert_eq!(worklist["items"][0]["recommended_args"], recommended_args);
        assert!(worklist["items"][0]["eas_focus"]
            .as_str()
            .unwrap()
            .contains("copy recommended_args.target_urls unchanged"));
        assert!(worklist["worklist_contract"]
            .as_str()
            .unwrap()
            .contains("copy recommended_args.target_urls unchanged"));
        assert!(worklist["worklist_contract"]
            .as_str()
            .unwrap()
            .contains("never rebuild a scheme from a port number"));
    }

    #[test]
    fn stage_worklist_status_points_to_submit_when_ready() {
        let status = stage_worklist_status(json!({
            "stage": "external_attack_surface",
            "organization_id": "org-1",
            "session_id": "sess",
            "summary": {"total_assets": 1},
            "assets": [{
                "target_id": "target-1",
                "value": "example.com",
                "target_type": "domain",
                "coverage": [
                    {"technique": "GOLISH-EAS-LIVENESS", "label": "Liveness", "state": "found", "evidence_refs": [1], "suggested_tools": []}
                ]
            }]
        }));

        assert_eq!(status["ready_to_submit"], true);
        assert_eq!(status["next_tool"], "submit_stage_deliverable");
        assert!(status["worklist_contract"]
            .as_str()
            .unwrap()
            .contains("DB/gate truth"));
    }

    #[test]
    fn enumeration_ready_copy_keeps_outcome_only_deliverable_contract() {
        let snapshot = json!({
            "stage": "enumeration",
            "organization_id": "org-1",
            "session_id": "sess",
            "summary": {"total_assets": 1},
            "assets": [{
                "target_id": "target-1",
                "value": "https://app.example.com:443",
                "target_type": "url",
                "exact_web_origin": true,
                "coverage": [
                    {"technique": "GOLISH-ENUM-JS", "label": "JS", "state": "found"},
                    {"technique": "GOLISH-ENUM-JSAPI", "label": "API", "state": "found"},
                    {"technique": "GOLISH-ENUM-DIR", "label": "Directory", "state": "checked_empty"},
                    {"technique": "GOLISH-ENUM-PARAM", "label": "Parameters", "state": "checked_empty"}
                ]
            }]
        });

        let status = stage_worklist_status(snapshot.clone());
        let next = stage_worklist_next(
            snapshot,
            10,
            &[
                "pending".to_string(),
                "error".to_string(),
                "partial".to_string(),
            ],
        );

        assert_eq!(status["ready_to_submit"], true);
        assert!(status["worklist_contract"]
            .as_str()
            .unwrap()
            .contains("current-run fresh exact-origin evidence truth"));
        assert_eq!(next["ready_to_submit"], true);
        assert!(next["next_action"]
            .as_str()
            .unwrap()
            .contains("No pending/error/partial"));
        assert!(next["next_action"]
            .as_str()
            .unwrap()
            .contains("coverage: []"));
    }

    #[test]
    fn enumeration_freshness_failure_log_does_not_promise_presence_only_fallback() {
        let source = include_str!("security.rs");
        let stale_copy = ["falls back to", " presence-only freshness"].concat();
        assert!(!source.contains(&stale_copy));
        assert!(source.contains("Enumeration remains fail-closed without a freshness cutoff"));
    }

    #[test]
    fn enumeration_methodology_uses_outcome_only_completion_contract() {
        let methodology =
            include_str!("../../../../../resources/harness/stages/enumeration/methodology.md");

        assert!(!methodology.contains("max_requests=1000"));
        assert!(!methodology.contains("tested_units"));
        assert!(!methodology.contains("total_units"));
        assert!(!methodology.contains("each claim must cite real evidence ids"));
        assert!(methodology.contains("fresh exact-origin `technique_outcomes`"));
        assert!(methodology.contains("`coverage: []`"));
        assert!(methodology
            .contains("Do not hand-write `found`, `empty`, `blocked`, or `not_applicable`"));
        assert!(methodology.contains("enum_preflight_web_origins"));
        assert!(methodology.contains("any non-empty array is"));
        assert!(methodology.contains("rejected and cannot turn pending work"));
        assert!(methodology.contains("at most 200 cells"));
        assert!(methodology.contains("at most 50 distinct"));
        assert!(!methodology.contains("terminal_exceptions_preview.coverage_to_submit"));
    }

    #[test]
    fn query_target_data_requires_active_org_target_ownership_guard() {
        let source = include_str!("security.rs");
        let rejection = [
            "query_target_data target_id is not in scope",
            " for the active organization",
        ]
        .concat();
        let guard = ["in_scope_rows", "_own_target"].concat();
        assert!(source.contains(&rejection));
        assert!(source.contains(&guard));
    }

    #[test]
    fn in_scope_target_ownership_rejects_foreign_target_id() {
        let owned = uuid::Uuid::new_v4();
        let foreign = uuid::Uuid::new_v4();
        let rows = vec![json!({"target_id": owned.to_string()})];

        assert!(in_scope_rows_own_target(&rows, owned));
        assert!(!in_scope_rows_own_target(&rows, foreign));
        assert!(!in_scope_rows_own_target(
            &[json!({"target_id": "not-a-uuid"})],
            owned
        ));
    }
}
