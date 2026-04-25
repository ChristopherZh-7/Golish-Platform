//! Recon-related types: `ReconUpdate` (extended scan results) and the
//! `DirectoryEntry` / `DirEntryRow` pair for directory-discovery storage.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::types::ts_from_chrono;



/// Fields for an extended recon update.
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

// ============================================================================
// Directory entry storage (for ffuf / feroxbuster output)
// ============================================================================

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

#[derive(sqlx::FromRow)]
pub(super) struct DirEntryRow {
    id: Uuid,
    target_id: Option<Uuid>,
    url: String,
    status_code: Option<i32>,
    content_length: Option<i32>,
    lines: Option<i32>,
    words: Option<i32>,
    content_type: String,
    tool: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<DirEntryRow> for DirectoryEntry {
    fn from(r: DirEntryRow) -> Self {
        DirectoryEntry {
            id: r.id.to_string(),
            target_id: r.target_id.map(|u| u.to_string()),
            url: r.url,
            status_code: r.status_code,
            content_length: r.content_length,
            lines: r.lines,
            words: r.words,
            content_type: r.content_type,
            tool: r.tool,
            created_at: ts_from_chrono(r.created_at),
        }
    }
}
