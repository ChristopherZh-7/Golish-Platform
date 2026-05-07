//! Single step executor: resolves the tool command, runs it, parses
//! the output, and dispatches the parsed items to the caller's
//! [`crate::storage::PipelineStorage`] implementation.

use golish_db::repo::audit::PentestAudit;
use uuid::Uuid;

use crate::parser::{self, PatternConfig, StoreStats};
use crate::types::PipelineStep;

use super::super::orchestrator::PipelineRunner;
use super::super::templates::resolve_port_targets;
use super::super::tool_resolve::{load_tool_output_config, resolve_tool_command};
use super::super::types::{emit_pipeline_event, PipelineEvent, SingleStepResult, StepResult};
use super::{run_foreach_step, run_sub_pipeline_step};

pub(in super::super) async fn run_single_step<'a>(
    runner: &'a PipelineRunner<'a>,
    step: &'a PipelineStep,
    step_index: usize,
    total_steps: usize,
    target: &'a str,
    project_path: Option<&'a str>,
    parent_target_id: Option<Uuid>,
    input_file: Option<std::path::PathBuf>,
    tmp_dir: &'a std::path::Path,
    pipeline_id: &'a str,
    run_id: &'a str,
    step_outputs: &'a std::collections::HashMap<String, std::path::PathBuf>,
    depth: usize,
) -> SingleStepResult {
    let step_start = std::time::Instant::now();

    emit_pipeline_event(
        runner.emitter,
        &PipelineEvent {
            pipeline_id: pipeline_id.to_string(),
            run_id: run_id.to_string(),
            step_id: step.id.clone(),
            step_index,
            total_steps,
            status: "running".to_string(),
            tool_name: step.tool_name.clone(),
            message: None,
            store_stats: None,
            pipeline_name: None,
            target: None,
            all_steps: None,
            output: None,
            duration_ms: None,
            exit_code: None,
        },
    );

    if step.step_type == "sub_pipeline" {
        return run_sub_pipeline_step(
            runner,
            step,
            step_index,
            total_steps,
            target,
            project_path,
            parent_target_id,
            tmp_dir,
            pipeline_id,
            run_id,
            depth,
        )
        .await;
    }

    if step.step_type == "foreach" {
        return run_foreach_step(
            runner,
            step,
            step_index,
            total_steps,
            target,
            project_path,
            parent_target_id,
            tmp_dir,
            pipeline_id,
            run_id,
            step_outputs,
            depth,
        )
        .await;
    }

    if step.step_type == "ai_tool" {
        return run_ai_tool_step(
            runner,
            step,
            step_index,
            total_steps,
            target,
            project_path,
            parent_target_id,
            tmp_dir,
            pipeline_id,
            run_id,
            step_start,
        )
        .await;
    }

    let step_parent_id = if let Some(pid) = runner.parent_audit_id {
        PentestAudit::started(
            runner.pool,
            "pipeline_step_started",
            "pentest_pipeline",
            &format!("Step '{}' (tool={}) started", step.id, step.tool_name),
            parent_target_id,
            Some(&step.tool_name),
            serde_json::json!({
                "pipeline_id": pipeline_id,
                "run_id": run_id,
                "step_id": step.id,
                "step_index": step_index,
                "tool_name": step.tool_name,
                "command_template": step.command_template,
                "step_type": step.step_type,
                "target": target,
                "parent_audit_id": pid,
            }),
        )
        .await
        .ok()
    } else {
        None
    };

    let preflight = golish_pentest::preflight_tool(
        &step.command_template,
        runner.config_manager,
        golish_pentest::PreflightMode::AllowPathFallback,
    )
    .await;
    if !preflight.ready {
        let duration_ms = step_start.elapsed().as_millis() as u64;
        let err_msg = preflight
            .error_message
            .clone()
            .unwrap_or_else(|| format!("Preflight failed for '{}'", step.command_template));

        if let Some(pid) = step_parent_id {
            let _ = PentestAudit::failed(
                runner.pool,
                pid,
                "pipeline_step_failed",
                "pentest_pipeline",
                &err_msg,
                parent_target_id,
                Some(&step.tool_name),
                serde_json::json!({
                    "pipeline_id": pipeline_id,
                    "run_id": run_id,
                    "step_id": step.id,
                    "tool_name": step.tool_name,
                    "exit_code": 127,
                    "duration_ms": duration_ms,
                    "stage": "preflight",
                }),
            )
            .await;
        }

        emit_pipeline_event(
            runner.emitter,
            &PipelineEvent {
                pipeline_id: pipeline_id.to_string(),
                run_id: run_id.to_string(),
                step_id: step.id.clone(),
                step_index,
                total_steps,
                status: "error".to_string(),
                tool_name: step.tool_name.clone(),
                message: Some(err_msg.clone()),
                store_stats: None,
                pipeline_name: None,
                target: None,
                all_steps: None,
                output: None,
                duration_ms: Some(duration_ms),
                exit_code: Some(127),
            },
        );

        return SingleStepResult {
            step_result: StepResult {
                step_id: step.id.clone(),
                tool_name: step.tool_name.clone(),
                command: step.command_template.clone(),
                exit_code: Some(127),
                stdout_lines: 0,
                stderr_preview: err_msg,
                store_stats: None,
                duration_ms,
            },
            output_path: tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name)),
            stored_count: 0,
        };
    }

    let iter_targets: Vec<String> = if step.iterate_over.as_deref() == Some("ports") {
        resolve_port_targets(runner.pool, target, project_path).await
    } else {
        vec![target.to_string()]
    };

    let resolved_cmd = resolve_tool_command(&step.command_template, runner.config_manager).await;
    let args_str = step.args.join(" ");
    let mut combined_stdout = String::new();
    let mut combined_stderr = String::new();
    let mut last_exit_code: Option<i32> = Some(0);
    let mut last_cmd_str = String::new();

    for iter_target in &iter_targets {
        let mut cmd_str = if args_str.is_empty() {
            resolved_cmd.clone()
        } else {
            format!("{} {}", resolved_cmd, args_str)
        };
        cmd_str = cmd_str.replace("{target}", iter_target);

        if let Some(ref input) = input_file {
            cmd_str = cmd_str.replace("{prev_output}", &input.to_string_lossy());
        }
        last_cmd_str = cmd_str.clone();

        tracing::info!(
            "[pipeline] Step {}/{}: {} → {}{}",
            step_index + 1,
            total_steps,
            step.tool_name,
            cmd_str,
            if iter_targets.len() > 1 {
                format!(
                    " (port iter {}/{})",
                    iter_targets
                        .iter()
                        .position(|t| t == iter_target)
                        .unwrap_or(0)
                        + 1,
                    iter_targets.len()
                )
            } else {
                String::new()
            }
        );

        let make_cmd = || {
            let mut cmd = golish_shell_exec::build_tokio_shell_command(&cmd_str);
            cmd.stdin(if let Some(ref pf) = input_file {
                if step.exec_mode == "pipe" {
                    match std::fs::File::open(pf) {
                        Ok(f) => std::process::Stdio::from(f),
                        Err(_) => std::process::Stdio::null(),
                    }
                } else {
                    std::process::Stdio::null()
                }
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
            cmd
        };

        let proc_result = if let Some(timeout_s) = step.timeout_secs {
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout_s),
                make_cmd().output(),
            )
            .await
        } else {
            Ok(make_cmd().output().await)
        };

        match proc_result {
            Ok(Ok(output)) => {
                combined_stdout.push_str(&String::from_utf8_lossy(&output.stdout));
                combined_stderr.push_str(&String::from_utf8_lossy(&output.stderr));
                if output.status.code() != Some(0) {
                    last_exit_code = output.status.code();
                }
            }
            Ok(Err(e)) => {
                combined_stderr.push_str(&format!("Process error: {e}\n"));
                last_exit_code = Some(-1);
            }
            Err(_) => {
                combined_stderr.push_str(&format!(
                    "Step timed out after {}s\n",
                    step.timeout_secs.unwrap_or(0)
                ));
                last_exit_code = Some(-2);
            }
        }
    }

    let stdout = combined_stdout;
    let stderr = combined_stderr;
    let exit_code = last_exit_code;

    let output_file = tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name));
    let _ = std::fs::write(&output_file, &stdout);

    let mut step_stored = 0usize;
    let store_stats = if let Some(mut output_config) = load_tool_output_config(&step.tool_name) {
        if let Some(ref override_action) = step.db_action {
            output_config.db_action = Some(override_action.clone());
        }
        tracing::info!(
            tool = %step.tool_name,
            format = %output_config.format,
            db_action = ?output_config.db_action,
            stdout_len = stdout.len(),
            "[pipeline-store] Found output config"
        );
        let parse_input = if let Some(ref jq_expr) = output_config.transform {
            parser::transform_with_jq(&stdout, jq_expr).await
        } else {
            stdout.clone()
        };

        let items = match output_config.format.as_str() {
            "text" => {
                let patterns: Vec<PatternConfig> = output_config
                    .patterns
                    .iter()
                    .map(|p| PatternConfig {
                        data_type: p.data_type.clone(),
                        regex: p.regex.clone(),
                        fields: p.fields.clone(),
                    })
                    .collect();
                parser::parse_text_standalone(&parse_input, &patterns)
            }
            "json_lines" | "json" => parser::parse_json_standalone(
                &parse_input,
                &output_config.fields,
                output_config.format == "json_lines",
            ),
            _ => vec![],
        };

        let parsed_count = items.len();
        let mut stored_count = 0usize;
        let mut new_count = 0usize;
        let mut skipped_count = 0usize;
        let mut errors = Vec::new();
        let tool_name = &step.tool_name;

        if let Some(ref db_action) = output_config.db_action {
            for item in &items {
                let mut item = item.clone();
                if !item.fields.contains_key("host")
                    && !item.fields.contains_key("ip")
                    && !item.fields.contains_key("url")
                {
                    item.fields.insert("host".to_string(), target.to_string());
                }
                item.fields
                    .entry("_tool".to_string())
                    .or_insert_with(|| tool_name.clone());
                if db_action == "target_add" {
                    match runner
                        .storage
                        .store_target_from_item(runner.pool, &item, project_path, parent_target_id)
                        .await
                    {
                        Ok(is_new) => {
                            stored_count += 1;
                            if is_new {
                                new_count += 1;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(tool = %step.tool_name, error = %e, "[pipeline-store] Store error");
                            skipped_count += 1;
                            if errors.len() < 5 {
                                errors.push(e.to_string());
                            }
                        }
                    }
                    continue;
                }
                let result = match db_action.as_str() {
                    "target_update_recon" => {
                        runner
                            .storage
                            .store_recon_from_item(runner.pool, &item, project_path)
                            .await
                    }
                    "directory_entry_add" => {
                        runner
                            .storage
                            .store_dirent_from_item(runner.pool, &item, tool_name, project_path)
                            .await
                    }
                    "finding_add" => {
                        runner
                            .storage
                            .store_finding_from_item(runner.pool, &item, tool_name, project_path)
                            .await
                    }
                    _ => {
                        skipped_count += 1;
                        continue;
                    }
                };
                match result {
                    Ok(is_new) => {
                        stored_count += 1;
                        if is_new {
                            new_count += 1;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(tool = %step.tool_name, error = %e, "[pipeline-store] Store error");
                        skipped_count += 1;
                        if errors.len() < 5 {
                            errors.push(e.to_string());
                        }
                    }
                }
            }
        }

        tracing::info!(
            tool = %step.tool_name,
            stored = stored_count,
            new = new_count,
            skipped = skipped_count,
            "[pipeline-store] Store complete"
        );
        step_stored = stored_count;
        Some(StoreStats {
            parsed_count,
            stored_count,
            new_count,
            skipped_count,
            errors,
        })
    } else {
        tracing::debug!(tool = %step.tool_name, "[pipeline-store] No output config found");
        None
    };

    if step.step_type == "web_crawl" && exit_code == Some(0) && !stdout.is_empty() {
        let urls: Vec<String> = stdout
            .lines()
            .filter(|l| l.starts_with("http://") || l.starts_with("https://"))
            .map(|l| l.trim().to_string())
            .collect();
        if !urls.is_empty() {
            tracing::info!(
                count = urls.len(),
                "[pipeline] Merging katana URLs into sitemap"
            );
            runner
                .storage
                .merge_urls_into_sitemap(runner.pool, &urls, project_path)
                .await;
            emit_pipeline_event(
                runner.emitter,
                &PipelineEvent {
                    pipeline_id: pipeline_id.to_string(),
                    run_id: run_id.to_string(),
                    step_id: "sitemap_merge".to_string(),
                    step_index,
                    total_steps,
                    status: "info".to_string(),
                    tool_name: "katana".to_string(),
                    message: Some(format!("Merged {} URLs into sitemap", urls.len())),
                    store_stats: None,
                    pipeline_name: None,
                    target: None,
                    all_steps: None,
                    output: None,
                    duration_ms: None,
                    exit_code: None,
                },
            );
        }
    }

    let duration_ms = step_start.elapsed().as_millis() as u64;

    let truncated_output = if stdout.len() > 4096 {
        let mut s = stdout[..4096].to_string();
        s.push_str("\n… (truncated)");
        Some(s)
    } else if stdout.is_empty() {
        None
    } else {
        Some(stdout.clone())
    };
    emit_pipeline_event(
        runner.emitter,
        &PipelineEvent {
            pipeline_id: pipeline_id.to_string(),
            run_id: run_id.to_string(),
            step_id: step.id.clone(),
            step_index,
            total_steps,
            status: if exit_code == Some(0) {
                "completed".to_string()
            } else {
                "error".to_string()
            },
            tool_name: step.tool_name.clone(),
            message: Some(format!(
                "exit={}, lines={}, stored={}",
                exit_code.unwrap_or(-1),
                stdout.lines().count(),
                store_stats.as_ref().map(|s| s.stored_count).unwrap_or(0),
            )),
            store_stats: store_stats.clone(),
            pipeline_name: None,
            target: None,
            all_steps: None,
            output: truncated_output,
            duration_ms: Some(duration_ms),
            exit_code,
        },
    );

    if let Some(pid) = step_parent_id {
        let succeeded = exit_code == Some(0);
        let stored_count = store_stats.as_ref().map(|s| s.stored_count).unwrap_or(0);
        let detail_extra = serde_json::json!({
            "pipeline_id": pipeline_id,
            "run_id": run_id,
            "step_id": step.id,
            "tool_name": step.tool_name,
            "exit_code": exit_code,
            "stdout_lines": stdout.lines().count(),
            "stored_count": stored_count,
            "duration_ms": duration_ms,
        });
        if succeeded {
            let _ = PentestAudit::completed(
                runner.pool,
                pid,
                "pipeline_step_completed",
                "pentest_pipeline",
                &format!(
                    "Step '{}' completed (exit=0, stored={}, {}ms)",
                    step.id, stored_count, duration_ms
                ),
                parent_target_id,
                Some(&step.tool_name),
                detail_extra,
            )
            .await;
        } else {
            let _ = PentestAudit::failed(
                runner.pool,
                pid,
                "pipeline_step_failed",
                "pentest_pipeline",
                &format!(
                    "Step '{}' failed (exit={:?}, {}ms)",
                    step.id, exit_code, duration_ms
                ),
                parent_target_id,
                Some(&step.tool_name),
                detail_extra,
            )
            .await;
        }
    }

    SingleStepResult {
        step_result: StepResult {
            step_id: step.id.clone(),
            tool_name: step.tool_name.clone(),
            command: last_cmd_str,
            exit_code,
            stdout_lines: stdout.lines().count(),
            stderr_preview: stderr.chars().take(500).collect(),
            store_stats,
            duration_ms,
        },
        output_path: tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name)),
        stored_count: step_stored,
    }
}

// ────────────────────────────────────────────────────────────────────────
// AI-tool step executor — runs an in-process `golish_core::Tool` whose
// implementation lives in the main `golish` crate (e.g. `js_collect`,
// `js_extract_apis`, `auth_probe`). The catalog is wired into the runner
// by the caller (`pipeline_execute` in the main crate) via `with_ai_tools`.
// ────────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn run_ai_tool_step<'a>(
    runner: &'a PipelineRunner<'a>,
    step: &'a PipelineStep,
    step_index: usize,
    total_steps: usize,
    target: &'a str,
    project_path: Option<&'a str>,
    parent_target_id: Option<Uuid>,
    tmp_dir: &'a std::path::Path,
    pipeline_id: &'a str,
    run_id: &'a str,
    step_start: std::time::Instant,
) -> SingleStepResult {
    let tool_name = step.tool_name.as_str();
    let output_file = tmp_dir.join(format!("step-{}-{}.txt", step_index, tool_name));

    let step_parent_id = if let Some(pid) = runner.parent_audit_id {
        PentestAudit::started(
            runner.pool,
            "pipeline_step_started",
            "pentest_pipeline",
            &format!("AI step '{}' (tool={}) started", step.id, tool_name),
            parent_target_id,
            Some(tool_name),
            serde_json::json!({
                "pipeline_id": pipeline_id,
                "run_id": run_id,
                "step_id": step.id,
                "step_index": step_index,
                "tool_name": tool_name,
                "step_type": "ai_tool",
                "target": target,
                "parent_audit_id": pid,
            }),
        )
        .await
        .ok()
    } else {
        None
    };

    let registry = match runner.ai_tools {
        Some(r) => r,
        None => {
            let msg = format!(
                "Step '{}' has step_type='ai_tool' but no AI tool registry was wired \
                 into the pipeline runner. Wire `runner.ai_tools` (or call \
                 `execute_pipeline_headless_with_ai_tools`) when launching pipelines \
                 that use AI tools.",
                step.id
            );
            return ai_tool_failure(
                runner,
                step,
                step_index,
                total_steps,
                parent_target_id,
                pipeline_id,
                run_id,
                step_parent_id,
                &output_file,
                msg,
                step_start.elapsed().as_millis() as u64,
                127,
            )
            .await;
        }
    };
    let tool = match registry.iter().find(|t| t.name() == tool_name) {
        Some(t) => t,
        None => {
            let available: Vec<&str> = registry.iter().map(|t| t.name()).collect();
            let msg = format!(
                "AI tool '{}' not found. Available tools: [{}]",
                tool_name,
                available.join(", ")
            );
            return ai_tool_failure(
                runner,
                step,
                step_index,
                total_steps,
                parent_target_id,
                pipeline_id,
                run_id,
                step_parent_id,
                &output_file,
                msg,
                step_start.elapsed().as_millis() as u64,
                127,
            )
            .await;
        }
    };

    // Merge `target` into args. Each AI tool has slightly different
    // expectations (`target_url` for js_collect / js_extract_apis,
    // plain string for some others), so we set both common keys plus
    // `project_path`. User-provided `step.params` win — we only fill
    // missing keys.
    let mut args = match &step.params {
        serde_json::Value::Object(_) => step.params.clone(),
        _ => serde_json::json!({}),
    };
    if let Some(obj) = args.as_object_mut() {
        obj.entry("target_url".to_string())
            .or_insert(serde_json::json!(target));
        obj.entry("target".to_string())
            .or_insert(serde_json::json!(target));
        if let Some(pp) = project_path {
            obj.entry("project_path".to_string())
                .or_insert(serde_json::json!(pp));
        }
    }

    let workspace: std::path::PathBuf = project_path
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| tmp_dir.to_path_buf());

    tracing::info!(
        "[pipeline] AI step {}/{}: {} → execute({} keys, workspace={})",
        step_index + 1,
        total_steps,
        tool_name,
        args.as_object().map(|o| o.len()).unwrap_or(0),
        workspace.display(),
    );

    let exec_result = tool.execute(args.clone(), &workspace).await;
    let duration_ms = step_start.elapsed().as_millis() as u64;

    let (stdout, exit_code, stored_count, store_stats) = match exec_result {
        Ok(value) => {
            let exit_code = match value.get("status").and_then(|v| v.as_str()) {
                Some("ok") | Some("completed") | Some("partial") | Some("truncated")
                | Some("empty") | Some("no_captures") => 0,
                Some("error") => 1,
                _ => {
                    if value.get("error").is_some() {
                        1
                    } else {
                        0
                    }
                }
            };
            let stored = ai_tool_stored_count(&value);
            let pretty = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
            let stats = if stored > 0 {
                Some(StoreStats {
                    parsed_count: stored,
                    stored_count: stored,
                    new_count: stored,
                    skipped_count: 0,
                    errors: Vec::new(),
                })
            } else {
                None
            };
            (pretty, Some(exit_code), stored, stats)
        }
        Err(e) => (format!("AI tool error: {}", e), Some(1), 0, None),
    };

    let _ = std::fs::write(&output_file, &stdout);

    let truncated_output = if stdout.len() > 4096 {
        let mut s = stdout[..4096].to_string();
        s.push_str("\n… (truncated)");
        Some(s)
    } else if stdout.is_empty() {
        None
    } else {
        Some(stdout.clone())
    };

    emit_pipeline_event(
        runner.emitter,
        &PipelineEvent {
            pipeline_id: pipeline_id.to_string(),
            run_id: run_id.to_string(),
            step_id: step.id.clone(),
            step_index,
            total_steps,
            status: if exit_code == Some(0) {
                "completed".to_string()
            } else {
                "error".to_string()
            },
            tool_name: tool_name.to_string(),
            message: Some(format!(
                "exit={}, lines={}, stored={}",
                exit_code.unwrap_or(-1),
                stdout.lines().count(),
                stored_count,
            )),
            store_stats: store_stats.clone(),
            pipeline_name: None,
            target: None,
            all_steps: None,
            output: truncated_output,
            duration_ms: Some(duration_ms),
            exit_code,
        },
    );

    if let Some(pid) = step_parent_id {
        let succeeded = exit_code == Some(0);
        let detail_extra = serde_json::json!({
            "pipeline_id": pipeline_id,
            "run_id": run_id,
            "step_id": step.id,
            "tool_name": tool_name,
            "exit_code": exit_code,
            "stored_count": stored_count,
            "duration_ms": duration_ms,
            "step_type": "ai_tool",
        });
        if succeeded {
            let _ = PentestAudit::completed(
                runner.pool,
                pid,
                "pipeline_step_completed",
                "pentest_pipeline",
                &format!(
                    "AI step '{}' completed (stored={}, {}ms)",
                    step.id, stored_count, duration_ms
                ),
                parent_target_id,
                Some(tool_name),
                detail_extra,
            )
            .await;
        } else {
            let _ = PentestAudit::failed(
                runner.pool,
                pid,
                "pipeline_step_failed",
                "pentest_pipeline",
                &format!(
                    "AI step '{}' failed (exit={:?}, {}ms)",
                    step.id, exit_code, duration_ms
                ),
                parent_target_id,
                Some(tool_name),
                detail_extra,
            )
            .await;
        }
    }

    SingleStepResult {
        step_result: StepResult {
            step_id: step.id.clone(),
            tool_name: tool_name.to_string(),
            command: format!("ai_tool:{}", tool_name),
            exit_code,
            stdout_lines: stdout.lines().count(),
            stderr_preview: if exit_code == Some(0) {
                String::new()
            } else {
                stdout.chars().take(500).collect()
            },
            store_stats,
            duration_ms,
        },
        output_path: output_file,
        stored_count,
    }
}

#[allow(clippy::too_many_arguments)]
async fn ai_tool_failure(
    runner: &PipelineRunner<'_>,
    step: &PipelineStep,
    step_index: usize,
    total_steps: usize,
    parent_target_id: Option<Uuid>,
    pipeline_id: &str,
    run_id: &str,
    step_parent_id: Option<i64>,
    output_file: &std::path::Path,
    err_msg: String,
    duration_ms: u64,
    exit_code: i32,
) -> SingleStepResult {
    if let Some(pid) = step_parent_id {
        let _ = PentestAudit::failed(
            runner.pool,
            pid,
            "pipeline_step_failed",
            "pentest_pipeline",
            &err_msg,
            parent_target_id,
            Some(&step.tool_name),
            serde_json::json!({
                "pipeline_id": pipeline_id,
                "run_id": run_id,
                "step_id": step.id,
                "tool_name": step.tool_name,
                "exit_code": exit_code,
                "duration_ms": duration_ms,
                "stage": "ai_tool_lookup",
            }),
        )
        .await;
    }
    emit_pipeline_event(
        runner.emitter,
        &PipelineEvent {
            pipeline_id: pipeline_id.to_string(),
            run_id: run_id.to_string(),
            step_id: step.id.clone(),
            step_index,
            total_steps,
            status: "error".to_string(),
            tool_name: step.tool_name.clone(),
            message: Some(err_msg.clone()),
            store_stats: None,
            pipeline_name: None,
            target: None,
            all_steps: None,
            output: None,
            duration_ms: Some(duration_ms),
            exit_code: Some(exit_code),
        },
    );
    SingleStepResult {
        step_result: StepResult {
            step_id: step.id.clone(),
            tool_name: step.tool_name.clone(),
            command: format!("ai_tool:{}", step.tool_name),
            exit_code: Some(exit_code),
            stdout_lines: 0,
            stderr_preview: err_msg,
            store_stats: None,
            duration_ms,
        },
        output_path: output_file.to_path_buf(),
        stored_count: 0,
    }
}

/// Heuristically extract a "stored" count from a tool's JSON result.
/// Different AI tools use different field names; we look at common ones
/// in priority order so the Pipeline UI's `+N stored` chip works for all
/// of them without each tool having to opt in.
fn ai_tool_stored_count(value: &serde_json::Value) -> usize {
    for key in ["stored", "saved", "endpoints_total", "persisted_rows"] {
        if let Some(n) = value.get(key).and_then(|v| v.as_u64()) {
            return n as usize;
        }
    }
    for key in ["files", "endpoints", "items", "results"] {
        if let Some(arr) = value.get(key).and_then(|v| v.as_array()) {
            return arr.len();
        }
    }
    0
}
