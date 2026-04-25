//! Sub-pipeline step executor: recurses into `execute_pipeline_headless_inner`
//! when a step references another pipeline.

use uuid::Uuid;

use crate::tools::pipeline::PipelineStep;

use super::resolve_sub_pipeline;
use super::super::orchestrator::execute_pipeline_headless_inner;
use super::super::types::{
    emit_pipeline_event, PipelineEvent, SingleStepResult, StepResult, MAX_NESTING_DEPTH,
};

pub(in super::super::super) async fn run_sub_pipeline_step<'a>(
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
