use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

static PIPELINE_CANCELLED: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub async fn pipeline_cancel() -> Result<(), String> {
    PIPELINE_CANCELLED.store(true, Ordering::SeqCst);
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStep {
    pub id: String,
    #[serde(default = "default_step_type")]
    pub step_type: String,
    pub tool_name: String,
    #[serde(default)]
    pub tool_id: String,
    #[serde(default)]
    pub command_template: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub params: serde_json::Value,
    /// Which step's output to use as input (by step id). None = previous step.
    #[serde(default)]
    pub input_from: Option<String>,
    #[serde(default = "default_exec_mode")]
    pub exec_mode: String,
    /// Target type required for this step to run: "domain", "ip", "url", or null (always run)
    #[serde(default)]
    pub requires: Option<String>,
    /// Iterate this step over a target attribute. "ports" = run once per HTTP port.
    #[serde(default)]
    pub iterate_over: Option<String>,
    /// Override the tool's default db_action for this step.
    #[serde(default)]
    pub db_action: Option<String>,
    /// What to do when this step fails: "abort" (default), "skip", "continue"
    #[serde(default = "default_on_failure")]
    pub on_failure: String,
    /// Timeout for this step in seconds. None = no timeout.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Template ID to execute as a nested sub-pipeline (step_type = "sub_pipeline").
    #[serde(default)]
    pub sub_pipeline: Option<String>,
    /// Inline pipeline definition for nesting (alternative to sub_pipeline ID reference).
    #[serde(default)]
    pub inline_pipeline: Option<Box<Pipeline>>,
    /// Step ID whose output lines become iteration targets (step_type = "foreach").
    #[serde(default)]
    pub foreach_source: Option<String>,
    /// Max concurrent iterations for foreach steps. Defaults to 5.
    #[serde(default)]
    pub max_parallel: Option<usize>,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
}

fn default_step_type() -> String { "shell_command".to_string() }
fn default_exec_mode() -> String { "pipe".to_string() }
fn default_on_failure() -> String { "abort".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConnection {
    pub from_step: String,
    pub to_step: String,
    /// Condition expression evaluated against the upstream step's result.
    /// None = always pass. Examples: "exit_ok", "output_contains:80", "output_not_empty".
    #[serde(default)]
    pub condition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pipeline {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub is_template: bool,
    #[serde(default)]
    pub workflow_id: Option<String>,
    pub steps: Vec<PipelineStep>,
    pub connections: Vec<PipelineConnection>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

pub(crate) fn now_ts() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

mod templates;
mod commands;
mod engine;

pub use templates::*;
pub use commands::*;
pub use engine::*;
