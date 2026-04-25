//! Target / Scope / TargetType / TargetStatus DTOs and the database row
//! adapter.

use serde::{Deserialize, Serialize};
use uuid::Uuid;



#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub source: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
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
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TargetType {
    Domain,
    Ip,
    Cidr,
    Url,
    Wildcard,
}

impl TargetType {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Domain => "domain",
            Self::Ip => "ip",
            Self::Cidr => "cidr",
            Self::Url => "url",
            Self::Wildcard => "wildcard",
        }
    }
    pub(super) fn from_str(s: &str) -> Self {
        match s {
            "ip" => Self::Ip,
            "cidr" => Self::Cidr,
            "url" => Self::Url,
            "wildcard" => Self::Wildcard,
            _ => Self::Domain,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    #[serde(rename = "in")]
    InScope,
    #[serde(rename = "out")]
    OutOfScope,
}

impl Scope {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::InScope => "in",
            Self::OutOfScope => "out",
        }
    }
    pub(super) fn from_str(s: &str) -> Self {
        match s {
            "out" => Self::OutOfScope,
            _ => Self::InScope,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
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

pub(super) fn detect_type(value: &str) -> TargetType {
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

pub(super) fn ts_from_chrono(dt: chrono::DateTime<chrono::Utc>) -> u64 {
    dt.timestamp() as u64
}

#[derive(sqlx::FromRow)]
pub(super) struct TargetRow {
    id: Uuid,
    name: String,
    target_type: String,
    value: String,
    tags: serde_json::Value,
    notes: String,
    scope: String,
    status: String,
    source: String,
    parent_id: Option<Uuid>,
    ports: serde_json::Value,
    real_ip: String,
    cdn_waf: String,
    http_title: String,
    http_status: Option<i32>,
    webserver: String,
    os_info: String,
    content_type: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<TargetRow> for Target {
    fn from(r: TargetRow) -> Self {
        Target {
            id: r.id.to_string(),
            name: r.name,
            target_type: TargetType::from_str(&r.target_type),
            value: r.value,
            tags: serde_json::from_value(r.tags).unwrap_or_default(),
            notes: r.notes,
            scope: Scope::from_str(&r.scope),
            status: TargetStatus::from_str(&r.status),
            source: r.source,
            parent_id: r.parent_id.map(|u| u.to_string()),
            ports: serde_json::from_value(r.ports).unwrap_or_default(),
            real_ip: r.real_ip,
            cdn_waf: r.cdn_waf,
            http_title: r.http_title,
            http_status: r.http_status,
            webserver: r.webserver,
            os_info: r.os_info,
            content_type: r.content_type,
            created_at: ts_from_chrono(r.created_at),
            updated_at: ts_from_chrono(r.updated_at),
        }
    }
}

