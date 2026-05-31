//! Pipeline execution and template management commands.
//!
//! Expected commands exposed here (documentation only):
//! - `pipeline_list`, `pipeline_save`, `pipeline_cancel`,
//!   `pipeline_delete`, `pipeline_load`, `pipeline_execute`
//! - `pipeline_list_templates`, `pipeline_save_template`,
//!   `pipeline_delete_template`
//! - `pipeline_list_ai_tools` (in-process AI tool catalog for the editor)

pub use golish_pentest_app::pipeline::*;
