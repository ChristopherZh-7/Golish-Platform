use uuid::Uuid;

use crate::tools::pipeline::{Pipeline, PipelineStep};
use crate::tools::pipeline::templates::{builtin_templates, resolve_port_targets};
use crate::tools::output_parser::{PatternConfig, StoreStats};
use super::types::{StepResult, PipelineRunResult, PipelineEvent, SingleStepResult, MAX_NESTING_DEPTH, emit_pipeline_event};
use super::tool_resolve::{resolve_tool_command, load_tool_output_config};
use super::item_store::{store_target_from_item, store_recon_from_item, store_dirent_from_item, store_finding_from_item, merge_urls_into_sitemap};
use super::orchestrator::execute_pipeline_headless_inner;

/// Resolve a sub-pipeline by template ID or inline definition.
pub(super) fn resolve_sub_pipeline(step: &PipelineStep) -> Option<Pipeline> {
    if let Some(ref inline) = step.inline_pipeline {
        return Some(*inline.clone());
    }
    if let Some(ref template_id) = step.sub_pipeline {
        let all = builtin_templates();
        if let Some(p) = all.into_iter().find(|p| p.id == *template_id || p.name == *template_id) {
            return Some(p);
        }
    }
    None
}

/// Execute a sub-pipeline step by recursively calling `execute_pipeline_headless_inner`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_sub_pipeline_step<'a>(
    pool: &'a sqlx::PgPool,
    step: &'a PipelineStep,
    step_index: usize,
    total_steps: usize,
    target: &'a str,
    project_path: Option<&'a str>,
    _parent_target_id: Option<Uuid>,
    tmp_dir: &'a std::path::Path,
    config_manager: &'a golish_pentest::ConfigManager,
    pipeline_id: &'a str,
    run_id: &'a str,
    app: Option<&'a tauri::AppHandle>,
    depth: usize,
) -> SingleStepResult {
    let step_start = std::time::Instant::now();

    if depth >= MAX_NESTING_DEPTH {
        let msg = format!("Max nesting depth ({}) exceeded", MAX_NESTING_DEPTH);
        tracing::warn!("[pipeline] {}", msg);
        emit_pipeline_event(app, &PipelineEvent {
            pipeline_id: pipeline_id.to_string(), run_id: run_id.to_string(),
            step_id: step.id.clone(), step_index, total_steps,
            status: "error".to_string(), tool_name: step.tool_name.clone(),
            message: Some(msg.clone()), store_stats: None,
            pipeline_name: None, target: None, all_steps: None,
            output: None, duration_ms: Some(0), exit_code: Some(-3),
        });
        return SingleStepResult {
            step_result: StepResult {
                step_id: step.id.clone(), tool_name: step.tool_name.clone(),
                command: String::new(), exit_code: Some(-3),
                stdout_lines: 0, stderr_preview: msg, store_stats: None, duration_ms: 0,
            },
            output_path: tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name)),
            stored_count: 0,
        };
    }

    let sub = match resolve_sub_pipeline(step) {
        Some(p) => p,
        None => {
            let msg = format!("Sub-pipeline '{}' not found", step.sub_pipeline.as_deref().unwrap_or("?"));
            emit_pipeline_event(app, &PipelineEvent {
                pipeline_id: pipeline_id.to_string(), run_id: run_id.to_string(),
                step_id: step.id.clone(), step_index, total_steps,
                status: "error".to_string(), tool_name: step.tool_name.clone(),
                message: Some(msg.clone()), store_stats: None,
                pipeline_name: None, target: None, all_steps: None,
                output: None, duration_ms: Some(0), exit_code: Some(-4),
            });
            return SingleStepResult {
                step_result: StepResult {
                    step_id: step.id.clone(), tool_name: step.tool_name.clone(),
                    command: String::new(), exit_code: Some(-4),
                    stdout_lines: 0, stderr_preview: msg, store_stats: None, duration_ms: 0,
                },
                output_path: tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name)),
                stored_count: 0,
            };
        }
    };

    tracing::info!(
        "[pipeline] Sub-pipeline '{}' at depth {} for target '{}'",
        sub.name, depth + 1, target
    );

    let sub_result = execute_pipeline_headless_inner(
        pool, &sub, target, project_path, config_manager, app, depth + 1,
    ).await;

    let duration_ms = step_start.elapsed().as_millis() as u64;
    let output_file = tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name));

    match sub_result {
        Ok(result) => {
            let summary: String = result.steps.iter()
                .map(|s| format!("{}: exit={}", s.tool_name, s.exit_code.unwrap_or(-1)))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = std::fs::write(&output_file, &summary);

            let all_ok = result.steps.iter().all(|s| s.exit_code == Some(0) || s.exit_code.is_none());
            emit_pipeline_event(app, &PipelineEvent {
                pipeline_id: pipeline_id.to_string(), run_id: run_id.to_string(),
                step_id: step.id.clone(), step_index, total_steps,
                status: if all_ok { "completed" } else { "error" }.to_string(),
                tool_name: step.tool_name.clone(),
                message: Some(format!("Sub-pipeline '{}': {}", sub.name, summary)),
                store_stats: None, pipeline_name: None, target: None, all_steps: None,
                output: Some(summary.clone()), duration_ms: Some(duration_ms),
                exit_code: if all_ok { Some(0) } else { Some(1) },
            });

            SingleStepResult {
                step_result: StepResult {
                    step_id: step.id.clone(), tool_name: step.tool_name.clone(),
                    command: format!("[sub_pipeline:{}]", sub.name),
                    exit_code: if all_ok { Some(0) } else { Some(1) },
                    stdout_lines: summary.lines().count(),
                    stderr_preview: String::new(),
                    store_stats: None,
                    duration_ms,
                },
                output_path: output_file,
                stored_count: result.total_stored,
            }
        }
        Err(e) => {
            let msg = format!("Sub-pipeline error: {}", e);
            let _ = std::fs::write(&output_file, &msg);
            emit_pipeline_event(app, &PipelineEvent {
                pipeline_id: pipeline_id.to_string(), run_id: run_id.to_string(),
                step_id: step.id.clone(), step_index, total_steps,
                status: "error".to_string(), tool_name: step.tool_name.clone(),
                message: Some(msg.clone()), store_stats: None,
                pipeline_name: None, target: None, all_steps: None,
                output: None, duration_ms: Some(duration_ms), exit_code: Some(-5),
            });
            SingleStepResult {
                step_result: StepResult {
                    step_id: step.id.clone(), tool_name: step.tool_name.clone(),
                    command: format!("[sub_pipeline:{}]", step.sub_pipeline.as_deref().unwrap_or("?")),
                    exit_code: Some(-5), stdout_lines: 0,
                    stderr_preview: msg, store_stats: None, duration_ms,
                },
                output_path: output_file,
                stored_count: 0,
            }
        }
    }
}

/// Execute a foreach step: iterate over output lines from a source step, running either
/// a sub-pipeline or a command for each line as the target.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_foreach_step<'a>(
    pool: &'a sqlx::PgPool,
    step: &'a PipelineStep,
    step_index: usize,
    total_steps: usize,
    _target: &'a str,
    project_path: Option<&'a str>,
    _parent_target_id: Option<Uuid>,
    tmp_dir: &'a std::path::Path,
    config_manager: &'a golish_pentest::ConfigManager,
    pipeline_id: &'a str,
    run_id: &'a str,
    app: Option<&'a tauri::AppHandle>,
    step_outputs: &'a std::collections::HashMap<String, std::path::PathBuf>,
    depth: usize,
) -> SingleStepResult {
    let step_start = std::time::Instant::now();
    let output_file = tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name));
    let max_par = step.max_parallel.unwrap_or(5);

    let source_id = match step.foreach_source.as_deref() {
        Some(id) => id,
        None => {
            let msg = "foreach step requires foreach_source".to_string();
            let _ = std::fs::write(&output_file, &msg);
            return SingleStepResult {
                step_result: StepResult {
                    step_id: step.id.clone(), tool_name: step.tool_name.clone(),
                    command: String::new(), exit_code: Some(-6),
                    stdout_lines: 0, stderr_preview: msg, store_stats: None, duration_ms: 0,
                },
                output_path: output_file, stored_count: 0,
            };
        }
    };

    let source_path = match step_outputs.get(source_id) {
        Some(p) => p.clone(),
        None => {
            let msg = format!("foreach source step '{}' has no output", source_id);
            let _ = std::fs::write(&output_file, &msg);
            return SingleStepResult {
                step_result: StepResult {
                    step_id: step.id.clone(), tool_name: step.tool_name.clone(),
                    command: String::new(), exit_code: Some(-6),
                    stdout_lines: 0, stderr_preview: msg, store_stats: None, duration_ms: 0,
                },
                output_path: output_file, stored_count: 0,
            };
        }
    };

    let lines: Vec<String> = std::fs::read_to_string(&source_path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    if lines.is_empty() {
        let _ = std::fs::write(&output_file, "No iteration targets");
        let duration_ms = step_start.elapsed().as_millis() as u64;
        emit_pipeline_event(app, &PipelineEvent {
            pipeline_id: pipeline_id.to_string(), run_id: run_id.to_string(),
            step_id: step.id.clone(), step_index, total_steps,
            status: "completed".to_string(), tool_name: step.tool_name.clone(),
            message: Some("foreach: 0 iterations".to_string()), store_stats: None,
            pipeline_name: None, target: None, all_steps: None,
            output: None, duration_ms: Some(duration_ms), exit_code: Some(0),
        });
        return SingleStepResult {
            step_result: StepResult {
                step_id: step.id.clone(), tool_name: step.tool_name.clone(),
                command: format!("[foreach:{}]", source_id), exit_code: Some(0),
                stdout_lines: 0, stderr_preview: String::new(), store_stats: None, duration_ms,
            },
            output_path: output_file, stored_count: 0,
        };
    }

    tracing::info!(
        "[pipeline] foreach '{}': {} targets from '{}', max_parallel={}",
        step.tool_name, lines.len(), source_id, max_par
    );

    let mut total_stored = 0usize;
    let mut total_lines = 0usize;
    let mut any_failed = false;
    let mut combined_output = String::new();

    // Process in chunks of max_parallel
    for chunk in lines.chunks(max_par) {
        let chunk_futures = chunk.iter().enumerate().map(|(ci, iter_target)| {
            let sub_tmp = tmp_dir.join(format!("foreach-{}-{}", step.id, ci));
            let _ = std::fs::create_dir_all(&sub_tmp);

            async move {
                if let Some(sub_pipeline) = resolve_sub_pipeline(step) {
                    execute_pipeline_headless_inner(
                        pool, &sub_pipeline, iter_target, project_path,
                        config_manager, app, depth + 1,
                    ).await
                } else if !step.command_template.is_empty() {
                    let cmd = resolve_tool_command(&step.command_template, config_manager).await;
                    let args_str = step.args.join(" ");
                    let mut cmd_str = if args_str.is_empty() { cmd } else { format!("{} {}", cmd, args_str) };
                    cmd_str = cmd_str.replace("{target}", iter_target);

                    let output = tokio::process::Command::new("sh")
                        .arg("-c").arg(&cmd_str)
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .output().await;

                    match output {
                        Ok(out) => {
                            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                            Ok(PipelineRunResult {
                                pipeline_name: step.tool_name.clone(),
                                target: iter_target.clone(),
                                steps: vec![StepResult {
                                    step_id: step.id.clone(),
                                    tool_name: step.tool_name.clone(),
                                    command: cmd_str,
                                    exit_code: out.status.code(),
                                    stdout_lines: stdout.lines().count(),
                                    stderr_preview: String::from_utf8_lossy(&out.stderr).chars().take(200).collect(),
                                    store_stats: None,
                                    duration_ms: 0,
                                }],
                                total_stored: 0,
                                total_duration_ms: 0,
                            })
                        }
                        Err(e) => Err(anyhow::anyhow!("foreach cmd error: {}", e)),
                    }
                } else {
                    Err(anyhow::anyhow!("foreach step has neither sub_pipeline nor command_template"))
                }
            }
        });

        let chunk_results = futures::future::join_all(chunk_futures).await;
        for res in chunk_results {
            match res {
                Ok(r) => {
                    total_stored += r.total_stored;
                    for s in &r.steps {
                        total_lines += s.stdout_lines;
                        if s.exit_code.is_some() && s.exit_code != Some(0) {
                            any_failed = true;
                        }
                    }
                    combined_output.push_str(&format!("{}: {} stored\n", r.target, r.total_stored));
                }
                Err(e) => {
                    any_failed = true;
                    combined_output.push_str(&format!("ERROR: {}\n", e));
                }
            }
        }
    }

    let _ = std::fs::write(&output_file, &combined_output);
    let duration_ms = step_start.elapsed().as_millis() as u64;

    emit_pipeline_event(app, &PipelineEvent {
        pipeline_id: pipeline_id.to_string(), run_id: run_id.to_string(),
        step_id: step.id.clone(), step_index, total_steps,
        status: if any_failed { "error" } else { "completed" }.to_string(),
        tool_name: step.tool_name.clone(),
        message: Some(format!("foreach: {} iterations, {} stored", lines.len(), total_stored)),
        store_stats: None, pipeline_name: None, target: None, all_steps: None,
        output: if combined_output.len() > 4096 { Some(combined_output[..4096].to_string()) } else { Some(combined_output) },
        duration_ms: Some(duration_ms),
        exit_code: if any_failed { Some(1) } else { Some(0) },
    });

    SingleStepResult {
        step_result: StepResult {
            step_id: step.id.clone(), tool_name: step.tool_name.clone(),
            command: format!("[foreach:{}:{}]", source_id, lines.len()),
            exit_code: if any_failed { Some(1) } else { Some(0) },
            stdout_lines: total_lines,
            stderr_preview: String::new(), store_stats: None, duration_ms,
        },
        output_path: output_file,
        stored_count: total_stored,
    }
}

/// Execute a single pipeline step: resolve command, run process(es), parse output, store to DB.
/// This is the shared core used by both the headless and Tauri executors.

pub(super) async fn run_single_step<'a>(
    pool: &'a sqlx::PgPool,
    step: &'a PipelineStep,
    step_index: usize,
    total_steps: usize,
    target: &'a str,
    project_path: Option<&'a str>,
    parent_target_id: Option<Uuid>,
    input_file: Option<std::path::PathBuf>,
    tmp_dir: &'a std::path::Path,
    config_manager: &'a golish_pentest::ConfigManager,
    pipeline_id: &'a str,
    run_id: &'a str,
    app: Option<&'a tauri::AppHandle>,
    step_outputs: &'a std::collections::HashMap<String, std::path::PathBuf>,
    depth: usize,
) -> SingleStepResult {
    let step_start = std::time::Instant::now();

    emit_pipeline_event(app, &PipelineEvent {
        pipeline_id: pipeline_id.to_string(),
        run_id: run_id.to_string(),
        step_id: step.id.clone(),
        step_index,
        total_steps,
        status: "running".to_string(),
        tool_name: step.tool_name.clone(),
        message: None,
        store_stats: None,
        pipeline_name: None, target: None, all_steps: None,
        output: None, duration_ms: None, exit_code: None,
    });

    // ── Sub-pipeline execution ──
    if step.step_type == "sub_pipeline" {
        return run_sub_pipeline_step(
            pool, step, step_index, total_steps, target, project_path,
            parent_target_id, tmp_dir, config_manager, pipeline_id, run_id, app, depth,
        ).await;
    }

    // ── Foreach iteration ──
    if step.step_type == "foreach" {
        return run_foreach_step(
            pool, step, step_index, total_steps, target, project_path,
            parent_target_id, tmp_dir, config_manager, pipeline_id, run_id, app,
            step_outputs, depth,
        ).await;
    }

    let iter_targets: Vec<String> = if step.iterate_over.as_deref() == Some("ports") {
        resolve_port_targets(pool, target, project_path).await
    } else {
        vec![target.to_string()]
    };

    let resolved_cmd = resolve_tool_command(&step.command_template, config_manager).await;
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
            step_index + 1, total_steps, step.tool_name, cmd_str,
            if iter_targets.len() > 1 { format!(" (port iter {}/{})", iter_targets.iter().position(|t| t == iter_target).unwrap_or(0) + 1, iter_targets.len()) } else { String::new() }
        );

        let proc_result = if let Some(timeout_s) = step.timeout_secs {
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout_s),
                tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd_str)
                    .stdin(if let Some(ref pf) = input_file {
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
                    .stderr(std::process::Stdio::piped())
                    .output(),
            )
            .await
        } else {
            Ok(
                tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd_str)
                    .stdin(if let Some(ref pf) = input_file {
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
                    .stderr(std::process::Stdio::piped())
                    .output()
                    .await,
            )
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
                combined_stderr.push_str(&format!("Step timed out after {}s\n", step.timeout_secs.unwrap_or(0)));
                last_exit_code = Some(-2);
            }
        }
    }

    let stdout = combined_stdout;
    let stderr = combined_stderr;
    let exit_code = last_exit_code;

    let output_file = tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name));
    let _ = std::fs::write(&output_file, &stdout);

    // Parse output and store to DB
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
            crate::tools::output_parser::transform_with_jq(&stdout, jq_expr).await
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
                crate::tools::output_parser::parse_text_standalone(&parse_input, &patterns)
            }
            "json_lines" | "json" => {
                crate::tools::output_parser::parse_json_standalone(
                    &parse_input,
                    &output_config.fields,
                    output_config.format == "json_lines",
                )
            }
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
                    match store_target_from_item(pool, &item, project_path, parent_target_id).await
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
                                errors.push(e);
                            }
                        }
                    }
                    continue;
                }
                let result = match db_action.as_str() {
                    "target_update_recon" => {
                        store_recon_from_item(pool, &item, project_path).await
                    }
                    "directory_entry_add" => {
                        store_dirent_from_item(pool, &item, tool_name, project_path).await
                    }
                    "finding_add" => {
                        store_finding_from_item(pool, &item, tool_name, project_path).await
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
                            errors.push(e);
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

    // Post-step actions: merge crawler URLs into sitemap
    if step.step_type == "web_crawl" && exit_code == Some(0) && !stdout.is_empty() {
        let urls: Vec<String> = stdout
            .lines()
            .filter(|l| l.starts_with("http://") || l.starts_with("https://"))
            .map(|l| l.trim().to_string())
            .collect();
        if !urls.is_empty() {
            tracing::info!(count = urls.len(), "[pipeline] Merging katana URLs into sitemap");
            merge_urls_into_sitemap(pool, &urls, project_path).await;
            emit_pipeline_event(app, &PipelineEvent {
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
            });
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
    emit_pipeline_event(app, &PipelineEvent {
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
            store_stats
                .as_ref()
                .map(|s| s.stored_count)
                .unwrap_or(0),
        )),
        store_stats: store_stats.clone(),
        pipeline_name: None,
        target: None,
        all_steps: None,
        output: truncated_output,
        duration_ms: Some(duration_ms),
        exit_code,
    });

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
