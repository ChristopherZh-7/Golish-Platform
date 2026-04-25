//! Single step executor: the bulk of the pipeline runtime — resolves the
//! tool command, runs it, parses output, stores findings.

use uuid::Uuid;

use crate::tools::output_parser::{PatternConfig, StoreStats};
use crate::tools::pipeline::templates::resolve_port_targets;
use crate::tools::pipeline::PipelineStep;

use super::{run_foreach_step, run_sub_pipeline_step};
use super::super::item_store::{
    merge_urls_into_sitemap, store_dirent_from_item, store_finding_from_item,
    store_recon_from_item, store_target_from_item,
};
use super::super::tool_resolve::{load_tool_output_config, resolve_tool_command};
use super::super::types::{
    emit_pipeline_event, PipelineEvent, SingleStepResult, StepResult,
};

pub(in super::super::super) async fn run_single_step<'a>(
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
