//! Vulnerability ↔ wiki / PoC / scan-history CRUD.
//!
//! Each CVE can be associated with three kinds of artefacts, all stored in
//! Postgres:
//! - **wiki paths** — references to pages under `<wiki>/`.
//! - **PoC templates** — uploaded exploit code, optionally tagged with
//!   severity / verification state / source attribution.
//! - **scan history** — past scan results against named targets.
//!
//! [`VulnLinkFull`] is the aggregated view returned to the UI for a single
//! CVE; [`vuln_link_get_all`] returns the same shape per-CVE in one shot
//! for the dashboard, fetched via three parallel `SELECT *` queries that
//! we then merge in-memory.
//!
//! The `vuln_poc_*` commands are a thin overlay on the same table that
//! drives the PoC-first workflow (lists CVEs by their PoC state instead
//! of by their wiki state).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnLinkFull {
    pub wiki_paths: Vec<String>,
    pub poc_templates: Vec<VulnPocEntry>,
    pub scan_history: Vec<VulnScanEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnPocEntry {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub poc_type: String,
    pub language: String,
    pub content: String,
    pub source: String,
    pub source_url: String,
    pub severity: String,
    pub verified: bool,
    pub description: String,
    pub tags: Vec<String>,
    pub created: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnScanEntry {
    pub id: String,
    pub target: String,
    pub date: i64,
    pub result: String,
    pub details: Option<String>,
}

/// DB row → API shape for a PoC.
///
/// Kept here because every command in this module that returns a PoC has
/// to massage `VulnKbPoc` into [`VulnPocEntry`] (string IDs, ms timestamps).
fn poc_to_entry(p: golish_db::models::VulnKbPoc) -> VulnPocEntry {
    VulnPocEntry {
        id: p.id.to_string(),
        name: p.name,
        poc_type: p.poc_type,
        language: p.language,
        content: p.content,
        source: p.source,
        source_url: p.source_url,
        severity: p.severity,
        verified: p.verified,
        description: p.description,
        tags: p.tags,
        created: p.created_at.timestamp_millis(),
    }
}

#[tauri::command]
pub async fn vuln_link_get_all(
    state: tauri::State<'_, AppState>,
) -> Result<HashMap<String, VulnLinkFull>, String> {
    let pool = state.db_pool_ready().await?;
    let mut result: HashMap<String, VulnLinkFull> = HashMap::new();

    // Load all wiki links
    let all_wiki: Vec<golish_db::models::VulnKbLink> =
        sqlx::query_as("SELECT * FROM vuln_kb_links ORDER BY created_at DESC")
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
    for l in all_wiki {
        result
            .entry(l.cve_id.clone())
            .or_insert_with(|| VulnLinkFull {
                wiki_paths: vec![],
                poc_templates: vec![],
                scan_history: vec![],
            })
            .wiki_paths
            .push(l.wiki_path);
    }

    // Load all PoCs
    let all_pocs: Vec<golish_db::models::VulnKbPoc> =
        sqlx::query_as("SELECT * FROM vuln_kb_pocs ORDER BY created_at DESC")
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
    for p in all_pocs {
        result
            .entry(p.cve_id.clone())
            .or_insert_with(|| VulnLinkFull {
                wiki_paths: vec![],
                poc_templates: vec![],
                scan_history: vec![],
            })
            .poc_templates
            .push(poc_to_entry(p));
    }

    // Load all scans
    let all_scans: Vec<golish_db::models::VulnScanHistory> =
        sqlx::query_as("SELECT * FROM vuln_scan_history ORDER BY scanned_at DESC")
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
    for s in all_scans {
        result
            .entry(s.cve_id.clone())
            .or_insert_with(|| VulnLinkFull {
                wiki_paths: vec![],
                poc_templates: vec![],
                scan_history: vec![],
            })
            .scan_history
            .push(VulnScanEntry {
                id: s.id.to_string(),
                target: s.target,
                date: s.scanned_at.timestamp_millis(),
                result: s.result,
                details: s.details,
            });
    }

    Ok(result)
}

#[tauri::command]
pub async fn vuln_link_get(
    state: tauri::State<'_, AppState>,
    cve_id: String,
) -> Result<VulnLinkFull, String> {
    let pool = state.db_pool_ready().await?;

    let links = golish_db::repo::wiki_kb::get_links_for_cve(pool, &cve_id)
        .await
        .map_err(|e| e.to_string())?;
    let wiki_paths: Vec<String> = links.into_iter().map(|l| l.wiki_path).collect();

    let pocs = golish_db::repo::wiki_kb::get_pocs_for_cve(pool, &cve_id)
        .await
        .map_err(|e| e.to_string())?;
    let poc_templates: Vec<VulnPocEntry> = pocs.into_iter().map(poc_to_entry).collect();

    let scans = golish_db::repo::vuln_scan::get_scans_for_cve(pool, &cve_id)
        .await
        .map_err(|e| e.to_string())?;
    let scan_history: Vec<VulnScanEntry> = scans
        .into_iter()
        .map(|s| VulnScanEntry {
            id: s.id.to_string(),
            target: s.target,
            date: s.scanned_at.timestamp_millis(),
            result: s.result,
            details: s.details,
        })
        .collect();

    Ok(VulnLinkFull {
        wiki_paths,
        poc_templates,
        scan_history,
    })
}

#[tauri::command]
pub async fn vuln_link_add_wiki(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    wiki_path: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    golish_db::repo::wiki_kb::link_cve_to_wiki(pool, &cve_id, &wiki_path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn vuln_link_remove_wiki(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    wiki_path: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    sqlx::query("DELETE FROM vuln_kb_links WHERE cve_id = $1 AND wiki_path = $2")
        .bind(&cve_id)
        .bind(&wiki_path)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn vuln_link_add_poc(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    name: String,
    poc_type: String,
    language: String,
    content: String,
) -> Result<VulnPocEntry, String> {
    let pool = state.db_pool_ready().await?;
    let poc = golish_db::repo::wiki_kb::upsert_poc(pool, &cve_id, &name, &poc_type, &language, &content)
        .await
        .map_err(|e| e.to_string())?;
    Ok(poc_to_entry(poc))
}

#[tauri::command]
pub async fn vuln_link_update_poc(
    state: tauri::State<'_, AppState>,
    poc_id: String,
    name: String,
    content: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    let id: uuid::Uuid = poc_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    sqlx::query("UPDATE vuln_kb_pocs SET name = $2, content = $3, updated_at = NOW() WHERE id = $1")
        .bind(id)
        .bind(&name)
        .bind(&content)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn vuln_link_remove_poc(
    state: tauri::State<'_, AppState>,
    poc_id: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    let id: uuid::Uuid = poc_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    golish_db::repo::wiki_kb::delete_poc(pool, id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn vuln_link_add_scan(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    target: String,
    result: String,
    details: Option<String>,
) -> Result<VulnScanEntry, String> {
    let pool = state.db_pool_ready().await?;
    let scan = golish_db::repo::vuln_scan::add_scan(
        pool, &cve_id, &target, &result, details.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(VulnScanEntry {
        id: scan.id.to_string(),
        target: scan.target,
        date: scan.scanned_at.timestamp_millis(),
        result: scan.result,
        details: scan.details,
    })
}

#[tauri::command]
pub async fn vuln_link_remove_scan(
    state: tauri::State<'_, AppState>,
    scan_id: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    let id: uuid::Uuid = scan_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    golish_db::repo::vuln_scan::delete_scan(pool, id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ============================================================================
// PoC-first workflow
// ============================================================================

#[tauri::command]
pub async fn vuln_link_add_poc_full(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    name: String,
    poc_type: String,
    language: String,
    content: String,
    source: String,
    source_url: String,
    severity: String,
    description: String,
    tags: Vec<String>,
) -> Result<VulnPocEntry, String> {
    let pool = state.db_pool_ready().await?;
    let poc = golish_db::repo::wiki_kb::upsert_poc_full(
        pool, &cve_id, &name, &poc_type, &language, &content,
        &source, &source_url, &severity, &description, &tags,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(poc_to_entry(poc))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CvePocSummaryResponse {
    pub cve_id: String,
    pub poc_count: i64,
    pub max_severity: String,
    pub any_verified: bool,
    pub has_research: bool,
    pub has_wiki: bool,
}

#[tauri::command]
pub async fn vuln_poc_list_cves(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<CvePocSummaryResponse>, String> {
    let pool = state.db_pool_ready().await?;
    let rows = golish_db::repo::wiki_kb::list_cves_with_pocs(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| CvePocSummaryResponse {
            cve_id: r.cve_id,
            poc_count: r.poc_count,
            max_severity: r.max_severity.unwrap_or_else(|| "unknown".to_string()),
            any_verified: r.any_verified.unwrap_or(false),
            has_research: r.has_research.unwrap_or(false),
            has_wiki: r.has_wiki.unwrap_or(false),
        })
        .collect())
}

#[tauri::command]
pub async fn vuln_poc_list_unresearched(
    state: tauri::State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<CvePocSummaryResponse>, String> {
    let pool = state.db_pool_ready().await?;
    let rows = golish_db::repo::wiki_kb::list_unresearched_cves(pool, limit.unwrap_or(20))
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| CvePocSummaryResponse {
            cve_id: r.cve_id,
            poc_count: r.poc_count,
            max_severity: r.max_severity.unwrap_or_else(|| "unknown".to_string()),
            any_verified: r.any_verified.unwrap_or(false),
            has_research: false,
            has_wiki: r.has_wiki.unwrap_or(false),
        })
        .collect())
}

#[tauri::command]
pub async fn vuln_poc_stats(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    golish_db::repo::wiki_kb::poc_stats(pool)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn vuln_poc_set_verified(
    state: tauri::State<'_, AppState>,
    poc_id: String,
    verified: bool,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    let id: uuid::Uuid = poc_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    sqlx::query("UPDATE vuln_kb_pocs SET verified = $2, updated_at = NOW() WHERE id = $1")
        .bind(id)
        .bind(verified)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
