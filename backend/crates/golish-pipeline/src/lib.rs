#![allow(clippy::too_many_arguments)]

//! Pipeline orchestration engine for Golish.
//!
//! This crate owns the full runtime for chaining pentest tools into a DAG,
//! resolving commands, executing steps in parallel, parsing their output,
//! and routing findings to storage via trait callbacks.
//!
//! It intentionally has **no** Tauri dependency — the frontend shell wires
//! it up with `golish_core::EventEmitterHandle` (see `TauriEventEmitter` in
//! the main `golish` crate).
//!
//! ## Layout
//! - [`types`] — public pipeline DTOs (`Pipeline`, `PipelineStep`, `PipelineConnection`).
//! - [`parser`] — pure parsing utilities (regex/JSON) used by step executors.
//! - [`storage`] — [`PipelineStorage`] trait callers must implement.
//! - [`engine`] — orchestrator, step executors, tool resolution, template loading.
//!
//! ## Public API
//! The main entry point is [`engine::execute_pipeline_headless`] which takes a
//! fully-built context (DB pool, pentest config manager, storage impl, optional
//! emitter) and runs the pipeline to completion. Template loading utilities
//! live in [`engine::templates`].

pub mod engine;
pub mod error;
pub mod parser;
pub mod storage;
pub mod types;

pub use engine::templates::{
    builtin_templates, detect_target_type, get_builtin_recon_basic, pipeline_from_json,
    recon_basic_template, templates_dir, PIPELINE_CANCELLED,
};
pub use engine::{
    execute_pipeline_headless, execute_pipeline_headless_with_ai_tools,
    execute_pipeline_headless_with_parent, PipelineRunner,
};
pub use engine::{PipelineRunResult, PipelineStepInfo, StepResult};
pub use error::{PipelineError, PipelineResult};
pub use parser::{
    extract_hostname, parse_json, parse_json_lines, parse_json_standalone, parse_text,
    parse_text_standalone, transform_with_jq, OutputParserConfig, ParsedItem, PatternConfig,
    StoreStats,
};
pub use storage::{NoopStorage, PipelineStorage};
pub use types::{now_ts, Pipeline, PipelineConnection, PipelineStep};
