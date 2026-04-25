use std::sync::atomic::Ordering;
use crate::state::AppState;
use crate::tools::pipeline::PIPELINE_CANCELLED;

mod types;
mod tool_resolve;
mod item_store;
mod steps;
mod orchestrator;

pub use types::{StepResult, PipelineRunResult};
pub use orchestrator::execute_pipeline_headless;

/// Tauri command wrapper around [`execute_pipeline_headless`] with cancel support.
#[tauri::command]
pub async fn pipeline_execute(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    pipeline: super::Pipeline,
    target: String,
    project_path: Option<String>,
) -> Result<PipelineRunResult, String> {
    PIPELINE_CANCELLED.store(false, Ordering::SeqCst);

    let pool = state.db_pool_ready().await?;
    let result = execute_pipeline_headless(
        pool,
        &pipeline,
        &target,
        project_path.as_deref(),
        &state.pentest_config_manager,
        Some(&app),
    )
    .await
    .map_err(|e| e.to_string());

    PIPELINE_CANCELLED.store(false, Ordering::SeqCst);
    result
}

#[cfg(test)]
mod tests;
