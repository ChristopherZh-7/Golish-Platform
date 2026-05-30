//! Single step executor: resolves the tool command, runs it, parses
//! the output, and dispatches the parsed items to the caller's
//! [`crate::storage::PipelineStorage`] implementation.
//!
//! The command-execution and parse/store phases live in [`exec`]; the
//! `ai_tool` step variant lives in [`ai_tool`].

use golish_db::repo::audit::PentestAudit;
use uuid::Uuid;

use crate::types::PipelineStep;

use super::super::orchestrator::PipelineRunner;
use super::super::types::{emit_pipeline_event, PipelineEvent, SingleStepResult, StepResult};
use super::{run_foreach_step, run_sub_pipeline_step};

mod ai_tool;
mod exec;

use self::ai_tool::run_ai_tool_step;
use self::exec::{parse_and_store, run_command_iterations};

#[allow(clippy::too_many_arguments)]
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

    let (stdout, stderr, exit_code, last_cmd_str) = run_command_iterations(
        runner,
        step,
        target,
        project_path,
        input_file.as_deref(),
        step_index,
        total_steps,
    )
    .await;

    let output_file = tmp_dir.join(format!("step-{}-{}.txt", step_index, step.tool_name));
    let _ = std::fs::write(&output_file, &stdout);

    let (store_stats, step_stored) = parse_and_store(
        runner,
        step,
        &stdout,
        target,
        project_path,
        parent_target_id,
    )
    .await;

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
