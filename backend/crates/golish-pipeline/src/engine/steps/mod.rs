//! Step executors split by kind.

use super::templates::builtin_templates;
use crate::types::{Pipeline, PipelineStep};

mod foreach;
mod single;
mod sub_pipeline;

pub(super) use foreach::run_foreach_step;
pub(super) use single::run_single_step;
pub(super) use sub_pipeline::run_sub_pipeline_step;

/// Resolve a sub-pipeline by template ID or inline definition.
pub(super) fn resolve_sub_pipeline(step: &PipelineStep) -> Option<Pipeline> {
    if let Some(ref inline) = step.inline_pipeline {
        return Some(*inline.clone());
    }
    if let Some(ref template_id) = step.sub_pipeline {
        let all = builtin_templates();
        if let Some(p) = all
            .into_iter()
            .find(|p| p.id == *template_id || p.name == *template_id)
        {
            return Some(p);
        }
    }
    None
}
