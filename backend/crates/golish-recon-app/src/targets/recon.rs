//! Directory-entry row adapter (`DirEntryRow`) + its conversion to the shared
//! [`DirectoryEntry`] DTO.
//!
//! The `ReconUpdate` extended-scan payload and the `DirectoryEntry` DTO now live
//! in `golish_app_core::domain::targets` (shared cross-service contract, S1-3)
//! and are re-exported here so existing `super::recon::*` paths stay valid. Only
//! the `sqlx::FromRow` row adapter — a DB-layer detail private to this crate —
//! remains defined here.

use uuid::Uuid;

use golish_core::time::ts_from_dt;

pub use golish_app_core::domain::targets::{DirectoryEntry, ReconUpdate};

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
            created_at: ts_from_dt(r.created_at),
        }
    }
}
