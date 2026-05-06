//! Pipeline runtime: orchestrator, step executors, template loader.

mod orchestrator;
mod steps;
mod tool_resolve;
pub(crate) mod types;

pub mod templates;

pub use orchestrator::{
    execute_pipeline_headless, execute_pipeline_headless_with_ai_tools,
    execute_pipeline_headless_with_parent, PipelineRunner,
};
pub use templates::PIPELINE_CANCELLED;
pub use types::{PipelineRunResult, PipelineStepInfo, StepResult};

#[cfg(test)]
mod tests;
