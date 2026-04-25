//! Foreach step executor: iterates the same step over a dynamic input set
//! (URL list, port list, etc.).

use uuid::Uuid;

use crate::tools::pipeline::PipelineStep;

use super::resolve_sub_pipeline;
use super::super::orchestrator::execute_pipeline_headless_inner;
use super::super::tool_resolve::resolve_tool_command;
use super::super::types::{
    emit_pipeline_event, PipelineEvent, PipelineRunResult, SingleStepResult, StepResult,
};

pub(in super::super::super) async fn run_foreach_step<'a>(
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
