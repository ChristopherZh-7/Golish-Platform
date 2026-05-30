//! Methodology data models (templates, phases, check items, project state).
//! Extracted verbatim from `methodology.rs`; re-exported by `methodology/mod.rs`
//! so `tools::methodology::MethodologyTemplate` etc. stay reachable.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodologyTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub phases: Vec<Phase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Phase {
    pub id: String,
    pub name: String,
    pub description: String,
    pub items: Vec<CheckItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckItem {
    pub id: String,
    pub title: String,
    pub description: String,
    pub checked: bool,
    pub notes: String,
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMethodology {
    pub id: String,
    pub template_id: String,
    pub template_name: String,
    pub project_name: String,
    pub phases: Vec<Phase>,
    pub created_at: String,
    pub updated_at: String,
}
