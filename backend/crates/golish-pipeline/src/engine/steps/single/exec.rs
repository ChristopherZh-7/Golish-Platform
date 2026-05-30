//! Phase helpers for [`super::run_single_step`]: command execution (with
//! optional per-port iteration) and the parse → store pipeline.

use uuid::Uuid;

use crate::parser::{self, PatternConfig, StoreStats};
use crate::types::PipelineStep;

use super::super::super::orchestrator::PipelineRunner;
use super::super::super::templates::resolve_port_targets;
use super::super::super::tool_resolve::{load_tool_output_config, resolve_tool_command};

/// Resolve the tool command and run it once per iteration target
/// (single target, or one per open port when `iterate_over == "ports"`),
/// accumulating stdout/stderr. Returns
/// `(stdout, stderr, last_exit_code, last_cmd_str)`.
pub(super) async fn run_command_iterations(
    runner: &PipelineRunner<'_>,
    step: &PipelineStep,
    target: &str,
    project_path: Option<&str>,
    input_file: Option<&std::path::Path>,
    step_index: usize,
    total_steps: usize,
) -> (String, String, Option<i32>, String) {
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

        if let Some(input) = input_file {
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
            cmd.stdin(if let Some(pf) = input_file {
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

    (
        combined_stdout,
        combined_stderr,
        last_exit_code,
        last_cmd_str,
    )
}

/// Load the tool's output config, parse stdout into items, and persist
/// each via the runner's storage backend. Returns `(store_stats, stored)`.
pub(super) async fn parse_and_store(
    runner: &PipelineRunner<'_>,
    step: &PipelineStep,
    stdout: &str,
    target: &str,
    project_path: Option<&str>,
    parent_target_id: Option<Uuid>,
) -> (Option<StoreStats>, usize) {
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
            parser::transform_with_jq(stdout, jq_expr).await
        } else {
            stdout.to_string()
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
    (store_stats, step_stored)
}
