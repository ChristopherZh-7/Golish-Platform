mod crud;
pub use crud::*;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

pub(super) fn evidence_dir(project_path: Option<&str>) -> PathBuf {
    if let Some(pp) = project_path {
        if !pp.is_empty() {
            return PathBuf::from(pp).join(".golish").join("evidence");
        }
    }
    golish_core::paths::app_data_base()
        .expect("cannot resolve home directory")
        .join("evidence")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub title: String,
    pub severity: Severity,
    #[serde(default)]
    pub cvss: Option<f64>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub steps: String,
    #[serde(default)]
    pub remediation: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub template: String,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    pub status: FindingStatus,
    #[serde(default = "default_finding_source")]
    pub source: String,
    pub created_at: u64,
    pub updated_at: u64,
}

pub(super) fn default_finding_source() -> String {
    "manual".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub caption: String,
    pub added_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl Severity {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Info => "info",
        }
    }
    fn from_str(s: &str) -> Self {
        match s {
            "critical" => Self::Critical,
            "high" => Self::High,
            "medium" => Self::Medium,
            "low" => Self::Low,
            _ => Self::Info,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FindingStatus {
    Open,
    Confirmed,
    FalsePositive,
    Resolved,
}

impl FindingStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Confirmed => "confirmed",
            Self::FalsePositive => "falsepositif",
            Self::Resolved => "resolved",
        }
    }
    fn from_str(s: &str) -> Self {
        match s {
            "confirmed" => Self::Confirmed,
            "falsepositif" | "falsepositive" => Self::FalsePositive,
            "resolved" => Self::Resolved,
            _ => Self::Open,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FindingsStore {
    #[serde(default)]
    pub findings: Vec<Finding>,
}

pub(super) fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) fn ts_from_dt(dt: chrono::DateTime<chrono::Utc>) -> u64 {
    dt.timestamp() as u64
}

#[derive(sqlx::FromRow)]
pub(super) struct FindingRow {
    id: Uuid,
    title: String,
    sev: String,
    cvss: Option<f64>,
    url: String,
    target: String,
    target_id: Option<Uuid>,
    description: String,
    steps: String,
    remediation: String,
    tags: serde_json::Value,
    tool: String,
    template: String,
    refs: serde_json::Value,
    evidence: serde_json::Value,
    status: String,
    source: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<FindingRow> for Finding {
    fn from(r: FindingRow) -> Self {
        Self {
            id: r.id.to_string(),
            title: r.title,
            severity: Severity::from_str(&r.sev),
            cvss: r.cvss,
            url: r.url,
            target: r.target,
            target_id: r.target_id.map(|u| u.to_string()),
            description: r.description,
            steps: r.steps,
            remediation: r.remediation,
            tags: serde_json::from_value(r.tags).unwrap_or_default(),
            tool: r.tool,
            template: r.template,
            references: serde_json::from_value(r.refs).unwrap_or_default(),
            evidence: serde_json::from_value(r.evidence).unwrap_or_default(),
            status: FindingStatus::from_str(&r.status),
            source: r.source,
            created_at: ts_from_dt(r.created_at),
            updated_at: ts_from_dt(r.updated_at),
        }
    }
}

#[allow(dead_code)]
const SELECT_COLS: &str = "id, title, sev::TEXT, cvss, url, target, target_id, description, steps, remediation, tags, tool, template, refs, evidence, status::TEXT, source, created_at, updated_at";
