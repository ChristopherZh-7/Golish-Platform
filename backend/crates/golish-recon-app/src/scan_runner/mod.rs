//! Tauri command wrappers for scan-runner operations.
//!
//! The pure business logic (WhatWeb and feroxbuster runners) now lives
//! in the `golish-scan-runner` crate. This module provides thin
//! `#[tauri::command]` wrappers that adapt `AppState` to the library's API.

use async_trait::async_trait;
use golish_scan_runner as runner;
use uuid::Uuid;

use golish_app_core::DbState;
use golish_app_core::GolishError;
use golish_app_core::TauriEventEmitter;

pub use runner::{FeroxScanOptions, ScanProgress, ScanResult, WhatWebOptions};

/// Main-crate adapter: maps the scan-runner's storage callbacks to
/// `crate::targets::db_directory_entry_add`.
struct MainScanStorage;

#[async_trait]
impl runner::ScanStorage for MainScanStorage {
    async fn store_directory_entry(
        &self,
        pool: &sqlx::PgPool,
        guard: &golish_db::repo::scoped::TargetWriteGuard,
        url: &str,
        status_code: Option<i32>,
        content_length: Option<i32>,
        lines: Option<i32>,
        words: Option<i32>,
        tool: &str,
    ) -> runner::ScanRunnerResult<()> {
        crate::targets::db_directory_entry_add_guarded(
            pool,
            guard,
            url,
            status_code,
            content_length,
            lines,
            words,
            tool,
        )
        .await
        .map(|_| ())
        .map_err(|e| runner::ScanRunnerError::Storage(e.to_string()))
    }
}

#[tauri::command]
pub async fn scan_whatweb(
    app: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    target_url: String,
    target_id: String,
    project_path: Option<String>,
    options: Option<WhatWebOptions>,
) -> Result<ScanResult, GolishError> {
    let pool = state.pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| GolishError::Validation(e.to_string()))?;
    let authorization =
        runner::authorize_scan_target(pool, tid, project_path.as_deref(), &target_url).await?;
    let emitter = TauriEventEmitter::handle(app);
    Ok(runner::run_whatweb(pool, Some(&emitter), &authorization, options).await?)
}

#[tauri::command]
pub async fn scan_feroxbuster(
    app: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    target_url: String,
    target_id: String,
    project_path: Option<String>,
    base_paths: Vec<String>,
    options: Option<FeroxScanOptions>,
) -> Result<ScanResult, GolishError> {
    let pool = state.pool_ready().await?;
    let tid = Uuid::parse_str(&target_id).map_err(|e| GolishError::Validation(e.to_string()))?;
    let authorization =
        runner::authorize_scan_target(pool, tid, project_path.as_deref(), &target_url).await?;
    let emitter = TauriEventEmitter::handle(app);
    Ok(runner::run_feroxbuster(
        pool,
        &MainScanStorage,
        Some(&emitter),
        &authorization,
        &base_paths,
        options,
    )
    .await?)
}
