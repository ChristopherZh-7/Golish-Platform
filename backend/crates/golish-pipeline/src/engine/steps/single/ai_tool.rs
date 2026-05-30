//! AI-tool step executor — runs an in-process `golish_core::Tool` whose
//! implementation lives in the main `golish` crate (e.g. `js_collect`,
//! `js_extract_apis`, `auth_probe`). The catalog is wired into the runner
//! by the caller (`pipeline_execute` in the main crate) via `with_ai_tools`.

use golish_db::repo::audit::PentestAudit;
use uuid::Uuid;

use crate::parser::StoreStats;
use crate::types::PipelineStep;

use super::super::super::orchestrator::PipelineRunner;
use super::super::super::types::{
    emit_pipeline_event, PipelineEvent, SingleStepResult, StepResult,
};

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_ai_tool_step<'a>(
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
