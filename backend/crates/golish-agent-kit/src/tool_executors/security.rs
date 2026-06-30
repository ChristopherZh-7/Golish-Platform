use super::common::{error_result, extract_string_param, ToolResult};
use serde_json::{json, Value};

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
            | "check_stage_asset_coverage"
    );
    if !is_sec_tool {
        return None;
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
            let rows = match repo.in_scope_targets(harness_org_id).await {
                Ok(r) => r,
                Err(e) => {
                    return Some(error_result(format!(
                        "Failed to list in-scope targets: {}",
                        e
                    )))
                }
            };
            let count = rows.len();
            let data = json!({ "in_scope_targets": rows, "count": count });
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
            let rows = match repo.attack_surface_seeds(harness_org_id, cap).await {
                Ok(r) => r,
                Err(e) => {
                    return Some(error_result(format!(
                        "Failed to list attack surface seeds: {}",
                        e
                    )))
                }
            };
            let count = rows.len();
            let data = json!({ "attack_surface_seeds": rows, "count": count });
            Some((data, true))
        }

        "list_enumeration_web_roots" => {
            let org_id = extract_string_param(args, &["organization_id"])
                .and_then(|s| uuid::Uuid::parse_str(&s).ok())
                .or(harness_org_id);
            let Some(org_id) = org_id else {
                return Some(error_result(
                    "list_enumeration_web_roots requires an organization_id when no active harness organization is bound",
                ));
            };
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n.clamp(1, 500) as usize)
                .unwrap_or(100);
            let include_coverage = args
                .get("include_coverage")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let stage = crate::harness::StageKind::Enumeration.as_str();
            let stage_started_at = match harness_operation_id {
                Some(operation_id) => match repo.operation_state_get(operation_id).await {
                    Ok(Some(state)) if state.current_stage == stage => Some(state.stage_started_at),
                    Ok(_) => None,
                    Err(err) => {
                        tracing::warn!(
                            target: "harness::enumeration_worklist",
                            error = %err,
                            "failed to read operation_state; enumeration web-root worklist falls back to presence-only freshness"
                        );
                        None
                    }
                },
                None => None,
            };
            let snapshot = match repo
                .stage_asset_coverage(org_id, stage, session_id, stage_started_at)
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

        "check_stage_asset_coverage" => {
            let stage = extract_string_param(args, &["stage"])
                .or_else(|| harness_stage.map(|stage| stage.as_str().to_string()));
            let Some(stage) = stage.filter(|stage| !stage.trim().is_empty()) else {
                return Some(error_result(
                    "check_stage_asset_coverage requires a 'stage' parameter when no harness stage is active",
                ));
            };
            let org_id = extract_string_param(args, &["organization_id"])
                .and_then(|s| uuid::Uuid::parse_str(&s).ok())
                .or(harness_org_id);
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
            let stage_started_at = match harness_operation_id {
                Some(operation_id) => match repo.operation_state_get(operation_id).await {
                    Ok(Some(state)) if state.current_stage == stage => Some(state.stage_started_at),
                    Ok(_) => None,
                    Err(err) => {
                        tracing::warn!(
                            target: "harness::coverage_preflight",
                            error = %err,
                            "failed to read operation_state; coverage preflight falls back to presence-only freshness"
                        );
                        None
                    }
                },
                None => None,
            };
            let snapshot = match repo
                .stage_asset_coverage(org_id, &stage, session_id, stage_started_at)
                .await
            {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    return Some(error_result(format!(
                        "Failed to check stage asset coverage: {err}"
                    )))
                }
            };
            Some((
                compact_stage_asset_coverage(snapshot, max_gaps, include_assets_requested),
                true,
            ))
        }

        _ => None,
    }
}

fn enumeration_web_roots_worklist(snapshot: Value, limit: usize, include_coverage: bool) -> Value {
    let assets = snapshot
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let total = assets.len();
    let mut roots = Vec::new();
    for asset in assets.into_iter().take(limit) {
        let coverage = asset
            .get("coverage")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut pending_techniques = Vec::new();
        let mut terminal_techniques = Vec::new();
        let mut suggested_tools = Vec::new();
        for cell in &coverage {
            let technique = cell.get("technique").and_then(Value::as_str).unwrap_or("");
            let state = cell
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("pending");
            if matches!(state, "pending" | "error") {
                pending_techniques.push(technique.to_string());
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

        let mut root = json!({
            "target_id": asset.get("target_id").cloned().unwrap_or(Value::Null),
            "root_url": asset.get("value").cloned().unwrap_or(Value::Null),
            "asset": asset.get("value").cloned().unwrap_or(Value::Null),
            "target_type": asset.get("target_type").cloned().unwrap_or(Value::Null),
            "organization_id": asset.get("organization_id").cloned().unwrap_or(Value::Null),
            "discovered_phase": asset.get("discovered_phase").cloned().unwrap_or(Value::Null),
            "pending_techniques": pending_techniques,
            "terminal_techniques": terminal_techniques,
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
        "worklist_semantics": "This list is derived from check_stage_asset_coverage and is narrowed to EAS-confirmed live web roots when DB truth exists. Enumerate only these roots for this org.",
        "execution_order": [
            "browser_collect_js_api",
            "js_extract_apis",
            "route_probe_paths with observed prefixes and the small local wordlist",
            "parameter extraction from observed requests, query strings, forms, and targeted param_hints",
            "check_stage_asset_coverage before submit_stage_deliverable"
        ],
        "tool_boundary": "Call browser_collect_js_api/js_collect/js_extract_apis/route_probe_paths directly. Directory discovery must use route_probe_paths plus observed JS/API prefixes and a small local wordlist; do not use external directory tools such as ffuf/gobuster/feroxbuster in enumeration. Bounded crawler CLIs such as katana must be called through pentest_run(tool_name=...) and used only as URL sources. Do not call manage_targets in enumeration.",
    })
}

fn enumeration_web_root_next_steps(coverage: &[Value]) -> Vec<&'static str> {
    let mut steps = Vec::new();
    for cell in coverage {
        let state = cell
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("pending");
        if !matches!(state, "pending" | "error") {
            continue;
        }
        match cell.get("technique").and_then(Value::as_str).unwrap_or("") {
            "GOLISH-ENUM-JSAPI" => {
                steps.push("run browser_collect_js_api first, then js_extract_apis on saved JS")
            }
            "GOLISH-ENUM-DIR" => steps.push(
                "run route_probe_paths over observed JS/API prefixes with the small local wordlist",
            ),
            "GOLISH-ENUM-PARAM" => {
                steps.push(
                    "derive parameters from observed requests/query strings/forms and targeted js_extract_apis param_hints",
                )
            }
            _ => steps
                .push("close this pending/error coverage cell with a real run or terminal note"),
        }
    }
    steps.sort();
    steps.dedup();
    steps
}

fn compact_stage_asset_coverage(snapshot: Value, max_gaps: usize, include_assets: bool) -> Value {
    let stage = snapshot.get("stage").and_then(Value::as_str).unwrap_or("");
    let is_enumeration = stage == "enumeration";
    let assets = snapshot
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut total_cells = 0usize;
    let mut pending_cells = 0usize;
    let mut error_cells = 0usize;
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
                "next_wave_pending" => next_wave_cells += 1,
                "blocked" => {
                    blocked_cells += 1;
                    done_cells += 1;
                }
                _ => done_cells += 1,
            }
            if matches!(state, "pending" | "error") && gaps.len() < max_gaps {
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
                    "suggested_tools": cell.get("suggested_tools").cloned().unwrap_or_else(|| json!([])),
                });
                if is_enumeration {
                    gap["worklist_source"] = json!("EAS-confirmed live web root");
                    gap["enumeration_focus"] = json!(enumeration_gap_focus(technique));
                }
                gaps.push(gap);
            }
        }
    }

    let omitted_gap_count = pending_cells + error_cells;
    let omitted_gap_count = omitted_gap_count.saturating_sub(gaps.len());
    let ready_to_submit = pending_cells == 0 && error_cells == 0;
    let next_action = if ready_to_submit {
        if next_wave_cells > 0 {
            "Current wave has no pending/error cells. Submit this wave's StageDeliverable with real evidence ids; next_wave_pending assets are newly discovered and should be handled by the next stage_run wave after this wave passes."
        } else if is_enumeration {
            "Enumeration preflight has no pending/error cells on the EAS-confirmed web-root worklist. Submit a slim StageDeliverable: summary claims plus only DB-nonderivable checked_empty/blocked/not_applicable coverage; do not hand-write found cells."
        } else {
            "Coverage has no pending/error/blocked cells in this preflight. Build the final StageDeliverable with real evidence ids and submit_stage_deliverable."
        }
    } else if is_enumeration {
        "Do not submit yet. Treat gap_examples as the exact EAS-confirmed live web-root worklist for this org: close JSAPI with browser_collect_js_api/js_extract_apis, DIR with route_probe_paths over observed JS/API prefixes plus the small local wordlist, and PARAM from observed browser requests, query strings, forms, and targeted js_extract_apis param_hints. Do not re-port-scan, default to external directory tools, or hand-write found cells."
    } else {
        "Do not submit yet. Close the pending/error cells first: run the suggested tools, wait for background jobs to land evidence, or mark truly blocked/not_applicable cells with concrete notes."
    };
    let mut out = json!({
        "ready_to_submit": ready_to_submit,
        "stage": snapshot.get("stage").cloned().unwrap_or(Value::Null),
        "organization_id": snapshot.get("organization_id").cloned().unwrap_or(Value::Null),
        "session_id": snapshot.get("session_id").cloned().unwrap_or(Value::Null),
        "summary": snapshot.get("summary").cloned().unwrap_or_else(|| json!({})),
        "cell_summary": {
            "total_cells": total_cells,
            "done_cells": done_cells,
            "pending_cells": pending_cells,
            "error_cells": error_cells,
            "blocked_cells": blocked_cells
            ,"next_wave_cells": next_wave_cells
        },
        "gap_examples": gaps,
        "omitted_gap_count": omitted_gap_count,
        "next_action": next_action
    });
    if is_enumeration {
        out["worklist_semantics"] = json!("Enumeration assets are narrowed to EAS-confirmed live web roots when that DB truth exists; no EAS live truth keeps the denominator fail-safe instead of passing empty.");
        out["deliverable_contract"] = json!("Submit no findings. DB-derived found cells come from directory_entries/api_endpoints; coverage should only contain checked_empty, blocked, or not_applicable terminal cells the DB cannot derive.");
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
        "GOLISH-ENUM-JSAPI" => {
            "Collect browser-observed JS/API first, then run js_extract_apis on saved JS."
        }
        "GOLISH-ENUM-DIR" => {
            "Probe observed JS/API route prefixes with the small local wordlist; avoid external directory tools by default."
        }
        "GOLISH-ENUM-PARAM" => {
            "Derive parameters from observed requests, query strings, forms, and targeted js_extract_apis param_hints."
        }
        _ => "Close this enumeration cell with a real run or an honest terminal note.",
    }
}

#[cfg(test)]
mod tests {
    use super::{compact_stage_asset_coverage, enumeration_web_roots_worklist};
    use serde_json::json;

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
        assert!(compact.get("assets").is_none());
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
        assert!(compact["deliverable_contract"]
            .as_str()
            .unwrap()
            .contains("Submit no findings"));
        assert!(compact["next_action"]
            .as_str()
            .unwrap()
            .contains("Do not re-port-scan"));
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
                        "value": "https://app.example.com",
                        "target_type": "url",
                        "organization_id": "org-1",
                        "coverage": [
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
            "https://app.example.com"
        );
        assert_eq!(
            worklist["web_roots"][0]["pending_techniques"][0],
            "GOLISH-ENUM-JSAPI"
        );
        assert!(worklist["web_roots"][0]["suggested_tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "browser_collect_js_api"));
        assert!(worklist["tool_boundary"]
            .as_str()
            .unwrap()
            .contains("pentest_run(tool_name=...)"));
    }
}
