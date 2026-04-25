//! Shared scan-runner helpers: progress emission, audit logging, command
//! lookup, and the global Nuclei cancellation flag.

use std::sync::atomic::AtomicBool;

use sqlx::PgPool;
use tauri::Emitter;
use uuid::Uuid;

use super::types::ScanProgress;

pub(super) static NUCLEI_CANCELLED: AtomicBool = AtomicBool::new(false);

pub(super) fn emit_progress(app: &tauri::AppHandle, tool: &str, phase: &str, current: u32, total: u32, msg: &str) {
    let _ = app.emit("scan-progress", ScanProgress {
        tool: tool.to_string(),
        phase: phase.to_string(),
        current,
        total,
        message: msg.to_string(),
    });
}

pub(super) async fn log_scan_op(
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
        pool, action, "scan", details, project_path,
        tool_name, target_id, None, Some(tool_name), status, detail,
    ).await;
}

pub(super) async fn which_tool(name: &str) -> Option<String> {
    let output = tokio::process::Command::new("which")
        .arg(name)
        .output()
        .await
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}
