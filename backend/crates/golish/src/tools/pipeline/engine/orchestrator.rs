use std::sync::atomic::Ordering;
use uuid::Uuid;
use tauri::Emitter;

use crate::tools::pipeline::{Pipeline, PIPELINE_CANCELLED};
use crate::tools::pipeline::templates::{detect_target_type, evaluate_condition, resolve_step_input, topo_layers};
use super::types::{StepResult, PipelineRunResult, PipelineEvent, PipelineStepInfo, emit_pipeline_event};
use super::steps::run_single_step;

pub async fn execute_pipeline_headless(
    pool: &sqlx::PgPool,
    pipeline: &Pipeline,
    target: &str,
    project_path: Option<&str>,
    config_manager: &golish_pentest::ConfigManager,
    app: Option<&tauri::AppHandle>,
) -> anyhow::Result<PipelineRunResult> {
    execute_pipeline_headless_inner(pool, pipeline, target, project_path, config_manager, app, 0).await
}

pub(super) async fn execute_pipeline_headless_inner(
    pool: &sqlx::PgPool,
    pipeline: &Pipeline,
    target: &str,
    project_path: Option<&str>,
    config_manager: &golish_pentest::ConfigManager,
    app: Option<&tauri::AppHandle>,
    depth: usize,
) -> anyhow::Result<PipelineRunResult> {
    let total_steps = pipeline.steps.len();
    let pipeline_id = pipeline.id.clone();
    let run_id = Uuid::new_v4().to_string();
    let mut step_results = Vec::new();
    let mut total_stored = 0usize;
    let start = std::time::Instant::now();
    let target_type = detect_target_type(target);

    let tmp_dir = std::env::temp_dir().join(format!("golish-pipeline-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir)?;

    let parent_target_id: Option<Uuid> = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM targets WHERE value = $1 AND project_path = $2 LIMIT 1",
    )
    .bind(target)
    .bind(project_path)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let mut step_outputs: std::collections::HashMap<String, std::path::PathBuf> =
        std::collections::HashMap::new();

    let step_index_map: std::collections::HashMap<&str, usize> = pipeline
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();

    let layers = topo_layers(&pipeline.steps, &pipeline.connections);
    tracing::info!(
        "[pipeline] DAG: {} steps in {} layers, connections={}",
        total_steps,
        layers.len(),
        pipeline.connections.len()
    );

    emit_pipeline_event(app, &PipelineEvent {
        pipeline_id: pipeline_id.clone(),
        run_id: run_id.clone(),
        step_id: String::new(),
        step_index: 0,
        total_steps,
        status: "started".to_string(),
        tool_name: String::new(),
        message: None,
        store_stats: None,
        pipeline_name: Some(pipeline.name.clone()),
        target: Some(target.to_string()),
        all_steps: Some(
            pipeline
                .steps
                .iter()
                .map(|s| PipelineStepInfo {
                    id: s.id.clone(),
                    tool_name: s.tool_name.clone(),
                    command_template: s.command_template.clone(),
                })
                .collect(),
        ),
        output: None,
        duration_ms: None,
        exit_code: None,
    });

    let mut had_abort = false;

    for (layer_idx, layer) in layers.iter().enumerate() {
        if had_abort {
            break;
        }

        if PIPELINE_CANCELLED.load(Ordering::SeqCst) {
            tracing::info!("[pipeline] Cancelled by user before layer {}", layer_idx + 1);
            emit_pipeline_event(app, &PipelineEvent {
                pipeline_id: pipeline_id.clone(),
                run_id: run_id.clone(),
                step_id: String::new(),
                step_index: 0,
                total_steps,
                status: "cancelled".to_string(),
                tool_name: String::new(),
                message: Some("Pipeline cancelled by user".to_string()),
                store_stats: None,
                pipeline_name: None,
                target: Some(target.to_string()),
                all_steps: None,
                output: None,
                duration_ms: None,
                exit_code: None,
            });
            break;
        }

        let mut runnable: Vec<(&crate::tools::pipeline::PipelineStep, usize, Option<std::path::PathBuf>)> = Vec::new();

        for &step in layer {
            let idx = step_index_map.get(step.id.as_str()).copied().unwrap_or(0);

            if let Some(ref req) = step.requires {
                if req != target_type {
                    tracing::info!(
                        "[pipeline] Skipping '{}': requires={}, target_type={}",
                        step.tool_name, req, target_type
                    );
                    emit_pipeline_event(app, &PipelineEvent {
                        pipeline_id: pipeline_id.clone(),
                        run_id: run_id.clone(),
                        step_id: step.id.clone(),
                        step_index: idx,
                        total_steps,
                        status: "skipped".to_string(),
                        tool_name: step.tool_name.clone(),
                        message: Some(format!("Skipped: requires {} target", req)),
                        store_stats: None,
                        pipeline_name: None,
                        target: None,
                        all_steps: None,
                        output: None,
                        duration_ms: None,
                        exit_code: None,
                    });
                    step_results.push(StepResult {
                        step_id: step.id.clone(),
                        tool_name: step.tool_name.clone(),
                        command: String::new(),
                        exit_code: None,
                        stdout_lines: 0,
                        stderr_preview: format!("Skipped: requires {} target", req),
                        store_stats: None,
                        duration_ms: 0,
                    });
                    continue;
                }
            }

            let incoming_conds: Vec<(&str, &str)> = pipeline.connections.iter()
                .filter(|c| c.to_step == step.id && c.condition.is_some())
                .map(|c| (c.from_step.as_str(), c.condition.as_deref().unwrap()))
                .collect();

            let mut cond_failed = false;
            for (from_id, cond_expr) in &incoming_conds {
                let upstream = step_results.iter().find(|r| r.step_id == *from_id);
                let upstream_output = step_outputs.get(*from_id);
                if let (Some(res), Some(out)) = (upstream, upstream_output) {
                    if !evaluate_condition(cond_expr, res, out) {
                        tracing::info!(
                            "[pipeline] Skipping '{}': condition '{}' on edge from '{}' not met",
                            step.tool_name, cond_expr, from_id
                        );
                        cond_failed = true;
                        break;
                    }
                } else {
                    tracing::warn!(
                        "[pipeline] Skipping '{}': upstream '{}' has no result for condition eval",
                        step.tool_name, from_id
                    );
                    cond_failed = true;
                    break;
                }
            }
            if cond_failed {
                emit_pipeline_event(app, &PipelineEvent {
                    pipeline_id: pipeline_id.clone(),
                    run_id: run_id.clone(),
                    step_id: step.id.clone(),
                    step_index: idx,
                    total_steps,
                    status: "skipped".to_string(),
                    tool_name: step.tool_name.clone(),
                    message: Some("Skipped: condition not met".to_string()),
                    store_stats: None,
                    pipeline_name: None, target: None, all_steps: None,
                    output: None, duration_ms: None, exit_code: None,
                });
                step_results.push(StepResult {
                    step_id: step.id.clone(),
                    tool_name: step.tool_name.clone(),
                    command: String::new(),
                    exit_code: None,
                    stdout_lines: 0,
                    stderr_preview: "Skipped: condition not met".to_string(),
                    store_stats: None,
                    duration_ms: 0,
                });
                continue;
            }

            let input_file = resolve_step_input(
                step,
                &step_outputs,
                &pipeline.connections,
                &tmp_dir,
                target,
            );
            runnable.push((step, idx, input_file));
        }

        if runnable.is_empty() {
            continue;
        }

        tracing::info!(
            "[pipeline] Layer {}/{}: running {} steps concurrently: [{}]",
            layer_idx + 1,
            layers.len(),
            runnable.len(),
            runnable.iter().map(|(s, _, _)| s.tool_name.as_str()).collect::<Vec<_>>().join(", ")
        );

        let layer_futures = runnable.iter().map(|(step, idx, input_file)| {
            run_single_step(
                pool,
                step,
                *idx,
                total_steps,
                target,
                project_path,
                parent_target_id,
                input_file.clone(),
                &tmp_dir,
                config_manager,
                &pipeline_id,
                &run_id,
                app,
                &step_outputs,
                depth,
            )
        });

        let layer_results = futures::future::join_all(layer_futures).await;

        for result in layer_results {
            step_outputs.insert(
                result.step_result.step_id.clone(),
                result.output_path,
            );
            total_stored += result.stored_count;

            let failed = result.step_result.exit_code.is_some()
                && result.step_result.exit_code != Some(0);

            let on_failure = pipeline
                .steps
                .iter()
                .find(|s| s.id == result.step_result.step_id)
                .map(|s| s.on_failure.as_str())
                .unwrap_or("abort");

            step_results.push(result.step_result);

            if failed && on_failure == "abort" {
                had_abort = true;
            }
        }
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);

    let total_duration_ms = start.elapsed().as_millis() as u64;
    let completed_steps = step_results
        .iter()
        .filter(|s| s.exit_code == Some(0))
        .count();
    let failed_steps = step_results
        .iter()
        .filter(|s| s.exit_code.is_some() && s.exit_code != Some(0))
        .count();

    let resolved_target_id = if parent_target_id.is_some() {
        parent_target_id
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM targets WHERE value = $1 AND project_path = $2 LIMIT 1",
        )
        .bind(target)
        .bind(project_path)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
    };

    let step_summaries: Vec<serde_json::Value> = step_results
        .iter()
        .map(|s| {
            serde_json::json!({
                "tool": s.tool_name,
                "stored": s.store_stats.as_ref().map(|st| st.stored_count).unwrap_or(0),
                "new": s.store_stats.as_ref().map(|st| st.new_count).unwrap_or(0),
                "parsed": s.store_stats.as_ref().map(|st| st.parsed_count).unwrap_or(0),
                "exit": s.exit_code,
                "ms": s.duration_ms,
            })
        })
        .collect();

    let _ = golish_db::repo::audit::log_operation(
        pool,
        "pipeline_executed",
        "recon",
        &format!(
            "Pipeline '{}' on {}: {}/{} steps completed, {} items stored",
            pipeline.name, target, completed_steps, total_steps, total_stored
        ),
        project_path,
        "pipeline",
        resolved_target_id,
        None,
        Some(&pipeline.name),
        if failed_steps == 0 {
            "completed"
        } else {
            "partial"
        },
        &serde_json::json!({
            "pipeline_id": pipeline_id,
            "run_id": run_id,
            "target": target,
            "total_steps": total_steps,
            "completed_steps": completed_steps,
            "failed_steps": failed_steps,
            "total_stored": total_stored,
            "total_new": step_results.iter().filter_map(|s| s.store_stats.as_ref()).map(|st| st.new_count).sum::<usize>(),
            "duration_ms": total_duration_ms,
            "steps": step_summaries,
        }),
    )
    .await;

    if total_stored > 0 {
        if let Some(app) = app {
            let _ = app.emit(
                "targets-changed",
                serde_json::json!({
                    "source": "pipeline",
                    "target": target,
                    "stored": total_stored,
                }),
            );
        }
    }

    Ok(PipelineRunResult {
        pipeline_name: pipeline.name.clone(),
        target: target.to_string(),
        steps: step_results,
        total_stored,
        total_duration_ms,
    })
}
