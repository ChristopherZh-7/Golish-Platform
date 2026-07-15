//! Shared scan-runner helpers: progress emission, audit logging, and command
//! lookup.

use std::process::ExitStatus;

use golish_core::EventEmitterHandle;
use golish_db::repo::audit::PentestAudit;
use golish_db::repo::scoped::TargetWriteGuard;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::types::ScanProgress;

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
    guard: &TargetWriteGuard,
    action: &str,
    tool_name: &str,
    target_url: &str,
    detail_extra: Value,
) -> crate::ScanRunnerResult<i64> {
    let mut detail = match detail_extra {
        Value::Object(map) => Value::Object(map),
        Value::Null => serde_json::json!({}),
        other => serde_json::json!({ "value": other }),
    };
    if let Some(obj) = detail.as_object_mut() {
        obj.entry("target_url".to_string())
            .or_insert_with(|| Value::String(target_url.to_string()));
    }
    PentestAudit::started_guarded(
        pool,
        guard,
        action,
        "scan",
        &format!("{} started against {}", tool_name, target_url),
        Some(tool_name),
        detail,
    )
    .await
    .map_err(crate::ScanRunnerError::from)
}

/// Insert a `<tool>_scan_completed` audit row linked to the parent `*_started`
/// row. Failure is propagated so a scan cannot report clean completion without
/// its guarded timeline closeout.
pub async fn audit_scan_completed(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    parent_id: i64,
    action: &str,
    tool_name: &str,
    details: &str,
    detail_extra: Value,
) -> crate::ScanRunnerResult<()> {
    PentestAudit::completed_guarded(
        pool,
        guard,
        parent_id,
        action,
        "scan",
        details,
        Some(tool_name),
        detail_extra,
    )
    .await?;
    Ok(())
}

/// Insert a `<tool>_scan_failed` audit row linked to the parent `*_started`
/// row. Callers already returning the primary process error may best-effort
/// this secondary write, but the write itself remains guarded.
pub async fn audit_scan_failed(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    parent_id: i64,
    action: &str,
    tool_name: &str,
    error: &str,
    detail_extra: Value,
) -> crate::ScanRunnerResult<()> {
    PentestAudit::failed_guarded(
        pool,
        guard,
        parent_id,
        action,
        "scan",
        error,
        Some(tool_name),
        detail_extra,
    )
    .await?;
    Ok(())
}

pub async fn which_tool(name: &str) -> Option<String> {
    golish_shell_exec::which_executable_async(name)
        .await
        .map(|p| p.to_string_lossy().to_string())
}

/// Conservative process-success check for scanners that have historically
/// returned exit code 0 while reporting a runtime failure on stderr (notably
/// some WhatWeb/Ruby combinations).  Partial stdout never upgrades a fatal
/// runtime diagnostic into success.
pub fn scanner_process_succeeded(status: ExitStatus, stderr: &str) -> bool {
    status.success() && !stderr_indicates_runtime_failure(stderr)
}

fn stderr_indicates_runtime_failure(stderr: &str) -> bool {
    let normalized = golish_core::utils::strip_ansi(stderr).to_ascii_lowercase();
    normalized.lines().any(|line| {
        let line = line.trim();
        line.starts_with("error")
            || line.starts_with("failed")
            || line.starts_with("fatal")
            || line.starts_with("panic")
            || line.contains("[err]")
            || line.contains("[ftl]")
            || line.contains("exception")
            || line.contains("can't modify frozen")
            || line.contains("not installed")
            || line.contains("missing dependencies")
            || line.contains("request timeout")
            || line.contains("operation timed out")
            || line.contains("context deadline exceeded")
            || line.contains("failed to resolve")
            || line.contains("no such host")
            || line.contains("network is unreachable")
    })
}

#[cfg(test)]
mod tests {
    use super::stderr_indicates_runtime_failure;

    #[test]
    fn runtime_failure_marker_is_not_hidden_by_other_output() {
        assert!(stderr_indicates_runtime_failure(
            "progress\nERROR Opening https://example.test: can't modify frozen Hash"
        ));
        assert!(stderr_indicates_runtime_failure(
            "[WRN] request timeout for https://example.test"
        ));
        assert!(!stderr_indicates_runtime_failure(
            "warning: retry recovered\n0 errors, 5 results"
        ));
    }
}
