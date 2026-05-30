//! DAG orchestrator: drives the full pipeline execution through
//! topological layers, emits lifecycle events, and aggregates per-step
//! results into a [`PipelineRunResult`].

use std::sync::Arc;

use golish_core::{EventEmitterHandle, Tool};

use super::types::PipelineRunResult;
use crate::storage::PipelineStorage;
use crate::types::Pipeline;

mod run;
pub(crate) use run::execute_pipeline_inner;

/// Bundle of references shared by every step executor.
///
/// `'a` borrows from the top-level `execute_pipeline_headless` frame;
/// the internal recursion (sub-pipelines, foreach) reuses the same
/// runner by re-borrowing.
///
/// `parent_audit_id` (when present) is the id of the outer `*_started` row
/// that wraps the whole pipeline run, so per-step audit rows can be linked
/// as children for end-to-end traceability.
///
/// `ai_tools` (optional) is the catalog of in-process [`Tool`] implementations
/// (e.g. `js_collect`, `js_extract_apis`, `auth_probe`) the engine can
/// execute when a step has `step_type = "ai_tool"`. When `None`, AI-tool
/// steps fail with a clear error so users notice the wiring is missing
/// rather than silently being skipped.
#[derive(Clone, Copy)]
pub struct PipelineRunner<'a> {
    pub pool: &'a sqlx::PgPool,
    pub config_manager: &'a golish_pentest::ConfigManager,
    pub storage: &'a dyn PipelineStorage,
    pub emitter: Option<&'a EventEmitterHandle>,
    pub parent_audit_id: Option<i64>,
    pub ai_tools: Option<&'a [Arc<dyn Tool>]>,
}

impl<'a> PipelineRunner<'a> {
    pub fn new(
        pool: &'a sqlx::PgPool,
        config_manager: &'a golish_pentest::ConfigManager,
        storage: &'a dyn PipelineStorage,
    ) -> Self {
        Self {
            pool,
            config_manager,
            storage,
            emitter: None,
            parent_audit_id: None,
            ai_tools: None,
        }
    }

    pub fn with_emitter(mut self, emitter: &'a EventEmitterHandle) -> Self {
        self.emitter = Some(emitter);
        self
    }

    pub fn with_optional_emitter(mut self, emitter: Option<&'a EventEmitterHandle>) -> Self {
        self.emitter = emitter;
        self
    }

    pub fn with_parent_audit_id(mut self, parent_audit_id: Option<i64>) -> Self {
        self.parent_audit_id = parent_audit_id;
        self
    }

    pub fn with_ai_tools(mut self, ai_tools: &'a [Arc<dyn Tool>]) -> Self {
        self.ai_tools = Some(ai_tools);
        self
    }

    pub fn with_optional_ai_tools(mut self, ai_tools: Option<&'a [Arc<dyn Tool>]>) -> Self {
        self.ai_tools = ai_tools;
        self
    }
}

/// Run a pipeline from start to finish.
///
/// Returns when every layer has completed or an abort-on-failure step
/// fails. Cancel mid-flight by setting [`PIPELINE_CANCELLED`] to `true`.
pub async fn execute_pipeline_headless(
    pool: &sqlx::PgPool,
    pipeline: &Pipeline,
    target: &str,
    project_path: Option<&str>,
    config_manager: &golish_pentest::ConfigManager,
    storage: &dyn PipelineStorage,
    emitter: Option<&EventEmitterHandle>,
) -> anyhow::Result<PipelineRunResult> {
    execute_pipeline_headless_with_parent(
        pool,
        pipeline,
        target,
        project_path,
        config_manager,
        storage,
        emitter,
        None,
    )
    .await
}

/// Same as [`execute_pipeline_headless`] but allows the caller to pass an
/// outer audit_log row id so per-step `pipeline_step_*` rows are emitted
/// with `parent_id` set, giving full lineage from `pipeline_started` →
/// each `pipeline_step_started` → `pipeline_step_completed/failed`.
#[allow(clippy::too_many_arguments)]
pub async fn execute_pipeline_headless_with_parent(
    pool: &sqlx::PgPool,
    pipeline: &Pipeline,
    target: &str,
    project_path: Option<&str>,
    config_manager: &golish_pentest::ConfigManager,
    storage: &dyn PipelineStorage,
    emitter: Option<&EventEmitterHandle>,
    parent_audit_id: Option<i64>,
) -> anyhow::Result<PipelineRunResult> {
    execute_pipeline_headless_with_ai_tools(
        pool,
        pipeline,
        target,
        project_path,
        config_manager,
        storage,
        emitter,
        parent_audit_id,
        None,
    )
    .await
}

/// Full-fledged entry point that also accepts an in-process AI tool catalog
/// used to execute steps whose `step_type = "ai_tool"` (e.g. `js_collect`,
/// `js_extract_apis`, `auth_probe`).
///
/// Existing callers that do not need AI tools should keep using
/// [`execute_pipeline_headless`] / [`execute_pipeline_headless_with_parent`]
/// — those simply pass `None` for `ai_tools` and so behave exactly as before.
#[allow(clippy::too_many_arguments)]
pub async fn execute_pipeline_headless_with_ai_tools(
    pool: &sqlx::PgPool,
    pipeline: &Pipeline,
    target: &str,
    project_path: Option<&str>,
    config_manager: &golish_pentest::ConfigManager,
    storage: &dyn PipelineStorage,
    emitter: Option<&EventEmitterHandle>,
    parent_audit_id: Option<i64>,
    ai_tools: Option<&[Arc<dyn Tool>]>,
) -> anyhow::Result<PipelineRunResult> {
    let runner = PipelineRunner {
        pool,
        config_manager,
        storage,
        emitter,
        parent_audit_id,
        ai_tools,
    };
    execute_pipeline_inner(&runner, pipeline, target, project_path, 0).await
}
