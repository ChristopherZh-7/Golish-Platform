//! Shared scan-runner DTOs: progress, result, and PoC match.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgress {
    pub tool: String,
    pub phase: String,
    pub current: u32,
    pub total: u32,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub tool: String,
    pub success: bool,
    pub items_found: u32,
    pub items_stored: u32,
    pub errors: Vec<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PocMatch {
    pub poc_id: String,
    pub cve_id: String,
    pub poc_name: String,
    pub poc_type: String,
    pub severity: String,
    pub source: String,
    pub matched_fingerprint: String,
    pub matched_version: String,
    pub template_id: Option<String>,
}
