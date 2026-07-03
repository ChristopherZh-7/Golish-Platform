//! Database row adapter (`TargetRow`) + its conversion to the shared [`Target`]
//! DTO.
//!
//! The core target DTOs (`Target` / `TargetType` / `Scope` / `TargetStatus` /
//! `TargetStore`) and the `detect_type` / `parse_iso8601` helpers now live in
//! `golish_app_core::domain::targets` (shared cross-service contract, S1-3) and
//! are re-exported here so existing `super::types::*` paths stay valid. Only the
//! `sqlx::FromRow` row adapter — a DB-layer detail private to this crate —
//! remains defined here.

use golish_core::time::ts_from_dt;
use uuid::Uuid;

pub use golish_app_core::domain::targets::{
    detect_type, parse_iso8601, Scope, Target, TargetStatus, TargetStore, TargetType,
};

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
    grp: String,
    owner: String,
    time_window_start: Option<chrono::DateTime<chrono::Utc>>,
    time_window_end: Option<chrono::DateTime<chrono::Utc>>,
    organization_id: Option<Uuid>,
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
    liveness_state: Option<String>,
    liveness_reason: Option<String>,
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
            grp: r.grp,
            owner: r.owner,
            time_window_start: r.time_window_start.map(ts_from_dt),
            time_window_end: r.time_window_end.map(ts_from_dt),
            organization_id: r.organization_id.map(|u| u.to_string()),
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
            liveness_state: r.liveness_state,
            liveness_reason: r.liveness_reason,
            created_at: ts_from_dt(r.created_at),
            updated_at: ts_from_dt(r.updated_at),
        }
    }
}
