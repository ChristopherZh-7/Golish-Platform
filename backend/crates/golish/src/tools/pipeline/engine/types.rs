use serde::{Serialize, Deserialize};
use tauri::Emitter;
use crate::tools::output_parser::StoreStats;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_id: String,
    pub tool_name: String,
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout_lines: usize,
    pub stderr_preview: String,
    pub store_stats: Option<StoreStats>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRunResult {
    pub pipeline_name: String,
    pub target: String,
    pub steps: Vec<StepResult>,
    pub total_stored: usize,
    pub total_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PipelineEvent {
    pub(super) pipeline_id: String,
    pub(super) run_id: String,
    pub(super) step_id: String,
    pub(super) step_index: usize,
    pub(super) total_steps: usize,
    pub(super) status: String,
    pub(super) tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) store_stats: Option<StoreStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) pipeline_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) all_steps: Option<Vec<PipelineStepInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PipelineStepInfo {
    pub(super) id: String,
    pub(super) tool_name: String,
    pub(super) command_template: String,
}

pub(super) struct SingleStepResult {
    pub(super) step_result: StepResult,
    pub(super) output_path: std::path::PathBuf,
    pub(super) stored_count: usize,
}

pub(super) const MAX_NESTING_DEPTH: usize = 5;

pub(super) fn emit_pipeline_event(app: Option<&tauri::AppHandle>, event: &PipelineEvent) {
    if let Some(app) = app {
        tracing::info!(
            "[pipeline-event] Emitting: status={}, step={}, pipeline={}",
            event.status, event.tool_name, event.pipeline_id
        );
        let _ = app.emit("pipeline-event", event);
    } else {
        tracing::warn!("[pipeline-event] No AppHandle available, skipping event emission");
    }
}
