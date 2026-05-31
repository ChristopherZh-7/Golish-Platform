//! Main-crate entry for pipeline commands.
//!
//! The pure business logic (DAG orchestration, step executors, template
//! loading, output parsing) now lives in the `golish-pipeline` crate.
//! This module only provides:
//!
//! - Tauri command wrappers that adapt `AppState` to the new crate's API.
//! - A `PipelineStorage` adapter (`storage::MainStorage`) that delegates
//!   to the main crate's existing `tools::targets::*` helpers.
//!
//! Re-export the `Pipeline` / `PipelineStep` types so existing call sites
//! (`pentest_bridge`, AI tool integrations) keep working without churn.

mod commands;
mod storage;

pub use commands::*;

pub use golish_pipeline::{
    get_builtin_recon_basic, now_ts, Pipeline, PipelineConnection, PipelineRunResult, PipelineStep,
    PipelineStepInfo, StepResult, PIPELINE_CANCELLED,
};

pub use commands::pipeline_save_template_inner;
pub use storage::MainStorage;
