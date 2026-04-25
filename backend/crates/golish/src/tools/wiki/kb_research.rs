//! KB research-log persistence.
//!
//! A separate, lightweight conversation history for vulnerability research:
//! each CVE gets at most one log row keyed by `cve_id`, recording the
//! turns of an agent's exploration and a coarse status.  Lives apart from
//! the regular conversation system because the research surface needs to
//! be query-able by CVE rather than by session.

use crate::state::AppState;

#[tauri::command]
pub async fn kb_research_load(
    state: tauri::State<'_, AppState>,
    cve_id: String,
) -> Result<Option<serde_json::Value>, String> {
    let pool = state.db_pool_ready().await?;
    let log = golish_db::repo::kb_research::get_log(pool, &cve_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(log.map(|l| {
        serde_json::json!({
            "cve_id": l.cve_id,
            "session_id": l.session_id,
            "turns": l.turns,
            "status": l.status,
            "updated_at": l.updated_at.to_rfc3339(),
        })
    }))
}

#[tauri::command]
pub async fn kb_research_save_turn(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    session_id: String,
    turn: serde_json::Value,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;

    let existing = golish_db::repo::kb_research::get_log(pool, &cve_id)
        .await
        .map_err(|e| e.to_string())?;

    if existing.is_some() {
        golish_db::repo::kb_research::append_turn(pool, &cve_id, &turn)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        let turns = serde_json::json!([turn]);
        golish_db::repo::kb_research::upsert_log(pool, &cve_id, &session_id, &turns, "in_progress")
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn kb_research_set_status(
    state: tauri::State<'_, AppState>,
    cve_id: String,
    status: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    golish_db::repo::kb_research::set_status(pool, &cve_id, &status)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn kb_research_clear(
    state: tauri::State<'_, AppState>,
    cve_id: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    golish_db::repo::kb_research::delete_log(pool, &cve_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
