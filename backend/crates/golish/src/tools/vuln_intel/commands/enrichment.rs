//! Tauri commands: GitHub PoC + Nuclei template lookups (per-CVE enrichment).

use golish_vuln_intel::{
    self as intel, BatchNucleiResult, GithubPocResult, NucleiDiscoverResult, NucleiTemplateResult,
};

use super::shared::github_client_from_settings;
use crate::error::GolishError;
use crate::event_emitter::TauriEventEmitter;
use crate::settings::SettingsManager;
use crate::state::DbState;

#[tauri::command]
pub async fn intel_search_github_poc(
    _state: tauri::State<'_, DbState>,
    settings_mgr: tauri::State<'_, std::sync::Arc<SettingsManager>>,
    cve_id: String,
) -> Result<Vec<GithubPocResult>, GolishError> {
    let (client, token) = github_client_from_settings(&settings_mgr).await?;
    let headers = intel::github_headers(&token);
    Ok(intel::search_github_poc(&client, &headers, &cve_id).await?)
}

#[tauri::command]
pub async fn intel_search_nuclei_templates(
    _state: tauri::State<'_, DbState>,
    settings_mgr: tauri::State<'_, std::sync::Arc<SettingsManager>>,
    cve_id: String,
) -> Result<Vec<NucleiTemplateResult>, GolishError> {
    let (client, token) = github_client_from_settings(&settings_mgr).await?;
    let headers = intel::github_headers(&token);
    Ok(intel::search_nuclei_templates(&client, &headers, &cve_id).await?)
}

#[tauri::command]
pub async fn intel_batch_search_nuclei_templates(
    _state: tauri::State<'_, DbState>,
    settings_mgr: tauri::State<'_, std::sync::Arc<SettingsManager>>,
    cve_ids: Vec<String>,
) -> Result<Vec<BatchNucleiResult>, GolishError> {
    let (client, token) = github_client_from_settings(&settings_mgr).await?;
    let headers = intel::github_headers(&token);
    Ok(intel::batch_search_nuclei_templates(&client, &headers, &cve_ids).await?)
}

#[tauri::command]
pub async fn intel_discover_all_nuclei(
    app: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    settings_mgr: tauri::State<'_, std::sync::Arc<SettingsManager>>,
) -> Result<NucleiDiscoverResult, GolishError> {
    let pool = state.pool_ready().await?;
    let (client, token) = github_client_from_settings(&settings_mgr).await?;
    let headers = intel::github_headers(&token);
    let emitter = TauriEventEmitter::handle(app);
    Ok(intel::discover_all_nuclei(pool, &client, &headers, Some(&emitter)).await?)
}
