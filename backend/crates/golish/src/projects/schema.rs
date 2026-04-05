//! Project configuration schema.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for a single project/codebase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Display name for the project
    pub name: String,

    /// Root path to the main project directory
    pub root_path: PathBuf,
}
