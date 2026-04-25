//! Pipeline engine step types — execution helpers split by step kind.
//!
//! - [`single`]: the bulk of the runtime — resolve a tool command, run it,
//!   parse output, store findings.
//! - [`foreach`]: iterates the same step over a dynamic input set (URLs,
//!   ports, ...).
//! - [`sub_pipeline`]: recursive descent into another pipeline referenced by
//!   ID or inline definition.
//! - [`resolve_sub_pipeline`] (in this file): looks up a sub-pipeline by
//!   template ID or returns the inline definition.

use crate::tools::pipeline::templates::builtin_templates;
use crate::tools::pipeline::{Pipeline, PipelineStep};

mod foreach;
mod single;
mod sub_pipeline;

pub(super) use foreach::run_foreach_step;
pub(super) use single::run_single_step;
pub(super) use sub_pipeline::run_sub_pipeline_step;



/// Resolve a sub-pipeline by template ID or inline definition.
pub(in super::super) fn resolve_sub_pipeline(step: &PipelineStep) -> Option<Pipeline> {
    if let Some(ref inline) = step.inline_pipeline {
        return Some(*inline.clone());
    }
    if let Some(ref template_id) = step.sub_pipeline {
        let all = builtin_templates();
        if let Some(p) = all.into_iter().find(|p| p.id == *template_id || p.name == *template_id) {
            return Some(p);
        }
    }
    None
}
