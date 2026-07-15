//! Shared scan-runner DTOs.

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NucleiTemplateSelection {
    pub template_id: String,
    pub rationales: Vec<NucleiTemplateRationale>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NucleiTemplateRationale {
    pub fingerprint_id: uuid::Uuid,
    pub fingerprint_name: String,
    pub fingerprint_version: Option<String>,
    pub poc_id: uuid::Uuid,
    pub cve_id: String,
    pub poc_name: String,
    pub severity: String,
}
