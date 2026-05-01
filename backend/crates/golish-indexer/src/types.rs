//! Shared DTOs for the indexer command layer.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodebaseInfo {
    pub path: String,
    pub file_count: usize,
    pub status: String,
    pub error: Option<String>,
    pub memory_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexResult {
    pub files_indexed: usize,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSearchResult {
    pub file_path: String,
    pub line_number: usize,
    pub line_content: String,
    pub matches: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentDirectory {
    pub path: String,
    pub name: String,
    pub branch: Option<String>,
    pub file_count: u32,
    pub insertions: i32,
    pub deletions: i32,
    pub last_accessed: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub path: String,
    pub file_count: u32,
    pub insertions: i32,
    pub deletions: i32,
    pub last_activity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub path: String,
    pub name: String,
    pub branches: Vec<BranchInfo>,
    pub warnings: u32,
    pub last_activity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeCreated {
    pub path: String,
    pub branch: String,
    pub init_script_run: bool,
    pub init_script_output: Option<String>,
}
