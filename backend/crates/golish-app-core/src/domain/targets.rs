//! Shared target / recon domain DTOs (servitization S1-3).
//!
//! These are the remote-ready contract types for the recon `targets` surface:
//! every field is serializable, no `PgPool`/closures, no crate-private deps.
//! They live here (not in `golish-recon-app`) so consuming services — pentest,
//! agent, … — can hold them without an upward sibling-crate dependency. The
//! `sqlx::FromRow` row adapters (`TargetRow` / `DirEntryRow`) and their `From`
//! conversions stay private inside `golish-recon-app` (DB-layer detail).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct Target {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub target_type: TargetType,
    pub value: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
    pub scope: Scope,
    pub status: TargetStatus,
    #[serde(default)]
    pub grp: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub time_window_start: Option<u64>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub time_window_end: Option<u64>,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    // The frontend view-model replaces this with a structured `PortInfo[]`; the
    // backend stores it as opaque JSON. Skip it from the generated type so the
    // binding stays free of the serde_json `JsonValue` import (the FE layers its
    // own `ports` + `technologies` on top via intersection).
    #[serde(default)]
    #[ts(skip)]
    pub ports: Vec<serde_json::Value>,
    #[serde(default)]
    pub real_ip: String,
    #[serde(default)]
    pub cdn_waf: String,
    #[serde(default)]
    pub http_title: String,
    #[serde(default)]
    pub http_status: Option<i32>,
    #[serde(default)]
    pub webserver: String,
    #[serde(default)]
    pub os_info: String,
    #[serde(default)]
    pub content_type: String,
    #[ts(type = "number")]
    pub created_at: u64,
    #[ts(type = "number")]
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum TargetType {
    Domain,
    Ip,
    Cidr,
    Url,
    Wildcard,
}

impl TargetType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Domain => "domain",
            Self::Ip => "ip",
            Self::Cidr => "cidr",
            Self::Url => "url",
            Self::Wildcard => "wildcard",
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "ip" => Self::Ip,
            "cidr" => Self::Cidr,
            "url" => Self::Url,
            "wildcard" => Self::Wildcard,
            _ => Self::Domain,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum Scope {
    #[serde(rename = "in")]
    InScope,
    #[serde(rename = "out")]
    OutOfScope,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InScope => "in",
            Self::OutOfScope => "out",
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "out" => Self::OutOfScope,
            _ => Self::InScope,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum TargetStatus {
    New,
    Recon,
    ReconDone,
    Scanning,
    Tested,
}

impl TargetStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Recon => "recon",
            Self::ReconDone => "recon_done",
            Self::Scanning => "scanning",
            Self::Tested => "tested",
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "recon" => Self::Recon,
            "recon_done" => Self::ReconDone,
            "scanning" => Self::Scanning,
            "tested" => Self::Tested,
            _ => Self::New,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TargetStore {
    pub targets: Vec<Target>,
}

/// Infer a [`TargetType`] from a raw target value (URL > CIDR > wildcard > IP >
/// domain).
pub fn detect_type(value: &str) -> TargetType {
    let v = value.trim();
    if v.starts_with("http://") || v.starts_with("https://") {
        return TargetType::Url;
    }
    if v.contains('/') {
        return TargetType::Cidr;
    }
    if v.starts_with("*.") {
        return TargetType::Wildcard;
    }
    if v.parse::<std::net::IpAddr>().is_ok() {
        return TargetType::Ip;
    }
    TargetType::Domain
}

/// Parse an ISO 8601 datetime string (with timezone) into UTC.
/// Returns None for empty/whitespace/malformed inputs so callers can pass
/// through to SQL as NULL.
pub fn parse_iso8601(s: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    let trimmed = s?.trim();
    if trimmed.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(trimmed)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// Fields for an extended recon update. Only non-empty values overwrite
/// existing data when applied.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReconUpdate {
    #[serde(default)]
    pub real_ip: String,
    #[serde(default)]
    pub cdn_waf: String,
    #[serde(default)]
    pub http_title: String,
    #[serde(default)]
    pub http_status: Option<i32>,
    #[serde(default)]
    pub webserver: String,
    #[serde(default)]
    pub os_info: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub ports: serde_json::Value,
}

impl ReconUpdate {
    pub fn new() -> Self {
        Self {
            ports: serde_json::json!([]),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub id: String,
    pub target_id: Option<String>,
    pub url: String,
    pub status_code: Option<i32>,
    pub content_length: Option<i32>,
    pub lines: Option<i32>,
    pub words: Option<i32>,
    pub content_type: String,
    pub tool: String,
    pub created_at: u64,
}
