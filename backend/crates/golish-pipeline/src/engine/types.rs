//! Internal pipeline engine types (step results, event payloads, helpers).
//!
//! `StepResult`, `PipelineRunResult`, `PipelineStepInfo` are re-exported
//! at crate root and appear in the public API. `PipelineEvent` and
//! `emit_pipeline_event` are crate-private; the frontend receives them
//! serialized through the `"pipeline-event"` channel.

use golish_core::EventEmitterHandle;
use serde::{Deserialize, Serialize};

use crate::parser::StoreStats;

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
pub(crate) struct PipelineEvent {
    pub(crate) pipeline_id: String,
    pub(crate) run_id: String,
    pub(crate) step_id: String,
    pub(crate) step_index: usize,
    pub(crate) total_steps: usize,
    pub(crate) status: String,
    pub(crate) tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) store_stats: Option<StoreStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pipeline_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) all_steps: Option<Vec<PipelineStepInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStepInfo {
    pub id: String,
    pub tool_name: String,
    pub command_template: String,
}

pub(crate) struct SingleStepResult {
    pub(crate) step_result: StepResult,
    pub(crate) output_path: std::path::PathBuf,
    pub(crate) stored_count: usize,
}

pub(crate) const MAX_NESTING_DEPTH: usize = 5;

pub(crate) fn emit_pipeline_event(emitter: Option<&EventEmitterHandle>, event: &PipelineEvent) {
    if let Some(emitter) = emitter {
        tracing::info!(
            "[pipeline-event] Emitting: status={}, step={}, pipeline={}",
            event.status, event.tool_name, event.pipeline_id
        );
        emitter.emit("pipeline-event", event);
    } else {
        tracing::debug!("[pipeline-event] No emitter attached, skipping event");
    }
}
