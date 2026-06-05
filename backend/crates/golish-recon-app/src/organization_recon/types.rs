use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::asset_intel::AssetIntelHydrateConfig;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum OrganizationReconRunStatus {
    Queued,
    Running,
    Completed,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum OrganizationReconStageName {
    EnterpriseIntel,
    PassiveInternet,
    ActiveCollection,
    Processing,
    Persistence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum ReconTaskStatus {
    Queued,
    Running,
    Completed,
    CheckedEmpty,
    Failed,
    Skipped,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct OrganizationReconTaskSnapshot {
    pub task_id: String,
    pub stage: OrganizationReconStageName,
    pub source_id: String,
    pub status: ReconTaskStatus,
    pub record_count: usize,
    pub artifacts: Vec<ReconArtifactRef>,
    pub errors: Vec<ReconTaskError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct OrganizationReconStageSnapshot {
    pub stage: OrganizationReconStageName,
    pub status: ReconTaskStatus,
    pub task_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum OrganizationReconTraceKind {
    RunStarted,
    RunCompleted,
    StepStarted,
    StepLog,
    StepAnnotation,
    StepCompleted,
    ArtifactCreated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct OrganizationReconTraceEvent {
    pub id: String,
    pub kind: OrganizationReconTraceKind,
    #[ts(type = "number")]
    pub timestamp: u64,
    pub stage: Option<OrganizationReconStageName>,
    pub task_id: Option<String>,
    pub status: Option<ReconTaskStatus>,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct OrganizationReconRunSnapshot {
    pub run_id: String,
    pub organization_id: String,
    pub project_path: String,
    pub status: OrganizationReconRunStatus,
    pub stages: Vec<OrganizationReconStageSnapshot>,
    pub tasks: Vec<OrganizationReconTaskSnapshot>,
    pub errors: Vec<ReconTaskError>,
    pub trace_events: Vec<OrganizationReconTraceEvent>,
    #[ts(type = "number")]
    pub created_at: u64,
    #[ts(type = "number")]
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct OrganizationReconStartArgs {
    pub organization_id: String,
    #[serde(default)]
    pub provider_ids: Vec<String>,
    #[serde(default)]
    pub config: AssetIntelHydrateConfig,
    #[serde(default)]
    pub allow_external: bool,
    #[serde(default)]
    pub allow_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct OrganizationReconEvent {
    pub run: OrganizationReconRunSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct ReconArtifactRef {
    pub path: String,
    pub kind: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct OrganizationReconExportResult {
    pub output_path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct ReconTaskError {
    pub code: String,
    pub message: String,
}

impl ReconTaskError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReconTaskManifest {
    pub run_id: String,
    pub task_id: String,
    pub stage: String,
    pub source_id: String,
    pub status: ReconTaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub encoding: String,
    pub artifacts: Vec<ReconArtifactRef>,
    pub record_count: usize,
    pub checked_empty: bool,
    pub errors: Vec<ReconTaskError>,
}

impl ReconTaskManifest {
    pub fn new(
        run_id: impl Into<String>,
        task_id: impl Into<String>,
        stage: impl Into<String>,
        source_id: impl Into<String>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            task_id: task_id.into(),
            stage: stage.into(),
            source_id: source_id.into(),
            status: ReconTaskStatus::Queued,
            exit_code: None,
            encoding: "utf-8".into(),
            artifacts: Vec::new(),
            record_count: 0,
            checked_empty: false,
            errors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ReconRecordKind {
    Organization,
    Domain,
    Ip,
    Port,
    Service,
    Url,
    Site,
    App,
    MiniProgram,
    Wechat,
    Certificate,
    Contact,
    Leak,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReconEvidenceRef {
    pub source_id: String,
    pub run_id: String,
    pub task_id: String,
    pub raw_artifact_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedReconRecord {
    pub record_id: String,
    pub kind: ReconRecordKind,
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub attributes: Value,
    #[serde(default)]
    pub evidence: Vec<ReconEvidenceRef>,
}
