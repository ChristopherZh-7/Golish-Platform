//! Shared scan-runner helpers: progress emission, audit logging, command
//! lookup, and the global Nuclei cancellation flag.

use std::sync::atomic::AtomicBool;

use golish_core::EventEmitterHandle;
use golish_db::repo::audit::PentestAudit;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::types::ScanProgress;

pub static NUCLEI_CANCELLED: AtomicBool = AtomicBool::new(false);

pub fn emit_progress(
    emitter: Option<&EventEmitterHandle>,
    tool: &str,
    phase: &str,
    current: u32,
    total: u32,
    msg: &str,
) {
    if let Some(e) = emitter {
        e.emit(
            "scan-progress",
            &ScanProgress {
                tool: tool.to_string(),
                phase: phase.to_string(),
                current,
                total,
                message: msg.to_string(),
            },
        );
    }
}

pub async fn log_scan_op(
    pool: &PgPool,
    action: &str,
    details: &str,
    project_path: Option<&str>,
    target_id: Option<Uuid>,
    tool_name: &str,
    status: &str,
    detail: &serde_json::Value,
) {
    let _ = golish_db::repo::audit::log_operation(
        pool,
        action,
        "scan",
        details,
        project_path,
        tool_name,
        target_id,
        None,
        Some(tool_name),
        status,
        detail,
    )
    .await;
}

/// Insert a `<tool>_scan_started` audit row and return the new row id so the
/// caller can pair it with the matching `*_completed` / `*_failed` log.
pub async fn audit_scan_started(
    pool: &PgPool,
    action: &str,
    target_id: Uuid,
    tool_name: &str,
    target_url: &str,
    detail_extra: Value,
) -> Option<i64> {
    let mut detail = match detail_extra {
        Value::Object(map) => Value::Object(map),
        Value::Null => serde_json::json!({}),
        other => serde_json::json!({ "value": other }),
    };
    if let Some(obj) = detail.as_object_mut() {
        obj.entry("target_url".to_string())
            .or_insert_with(|| Value::String(target_url.to_string()));
    }
    PentestAudit::started(
        pool,
        action,
        "scan",
        &format!("{} started against {}", tool_name, target_url),
        Some(target_id),
        Some(tool_name),
        detail,
    )
    .await
    .ok()
}

/// Insert a `<tool>_scan_completed` audit row linked to the parent `*_started`
/// row. No-op when `parent_id` is `None`.
pub async fn audit_scan_completed(
    pool: &PgPool,
    parent_id: Option<i64>,
    action: &str,
    target_id: Uuid,
    tool_name: &str,
    details: &str,
    detail_extra: Value,
) {
    let Some(pid) = parent_id else { return };
    let _ = PentestAudit::completed(
        pool,
        pid,
        action,
        "scan",
        details,
        Some(target_id),
        Some(tool_name),
        detail_extra,
    )
    .await;
}

/// Insert a `<tool>_scan_failed` audit row linked to the parent `*_started`
/// row. No-op when `parent_id` is `None`.
pub async fn audit_scan_failed(
    pool: &PgPool,
    parent_id: Option<i64>,
    action: &str,
    target_id: Uuid,
    tool_name: &str,
    error: &str,
    detail_extra: Value,
) {
    let Some(pid) = parent_id else { return };
    let _ = PentestAudit::failed(
        pool,
        pid,
        action,
        "scan",
        error,
        Some(target_id),
        Some(tool_name),
        detail_extra,
    )
    .await;
}

pub async fn which_tool(name: &str) -> Option<String> {
    let output = tokio::process::Command::new("which")
        .arg(name)
        .output()
        .await
        .ok()?;
    if output.status.success() {
        Some(
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .to_string(),
        )
    } else {
        None
    }
}
