use super::*;
use crate::state::AppState;

#[tauri::command]
pub async fn pipeline_list(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
) -> Result<Vec<Pipeline>, String> {
    let pool = state.db_pool_ready().await?;
    let rows: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT data FROM pipelines WHERE project_path = $1 ORDER BY updated_at DESC",
    )
    .bind(project_path.as_deref())
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let items: Vec<Pipeline> = rows
        .into_iter()
        .filter_map(|j| serde_json::from_value(j).ok())
        .collect();

    // Only include built-in defaults that haven't been saved/customized by the user yet
    let saved_workflow_ids: std::collections::HashSet<&str> = items
        .iter()
        .filter_map(|p| p.workflow_id.as_deref())
        .collect();

    let mut result: Vec<Pipeline> = builtin_templates()
        .into_iter()
        .filter(|t| {
            t.workflow_id.as_deref().map_or(true, |wid| !saved_workflow_ids.contains(wid))
        })
        .collect();
    result.extend(items);
    Ok(result)
}

#[tauri::command]
pub async fn pipeline_save(
    state: tauri::State<'_, AppState>,
    pipeline: Pipeline,
    project_path: Option<String>,
) -> Result<String, String> {
    let pool = state.db_pool_ready().await?;
    // Generate a new UUID for empty ids or non-UUID ids (built-in defaults)
    let id = if pipeline.id.is_empty() || pipeline.id.parse::<Uuid>().is_err() {
        Uuid::new_v4().to_string()
    } else {
        pipeline.id.clone()
    };
    let ts = now_ts();
    let entry = Pipeline {
        id: id.clone(),
        updated_at: ts,
        created_at: if pipeline.created_at == 0 { ts } else { pipeline.created_at },
        ..pipeline
    };
    let json = serde_json::to_value(&entry).map_err(|e| e.to_string())?;
    let uid: Uuid = id.parse().unwrap();
    sqlx::query(
        r#"INSERT INTO pipelines (id, data, project_path)
           VALUES ($1, $2, $3)
           ON CONFLICT (id) DO UPDATE SET data = $2, updated_at = NOW()"#,
    )
    .bind(uid)
    .bind(&json)
    .bind(project_path.as_deref())
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn pipeline_delete(
    state: tauri::State<'_, AppState>,
    id: String,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    let _ = project_path;
    let Ok(uid) = id.parse::<Uuid>() else {
        // Non-UUID ids are built-in defaults (not stored in DB), nothing to delete
        return Ok(());
    };
    sqlx::query("DELETE FROM pipelines WHERE id=$1")
        .bind(uid)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn pipeline_load(
    state: tauri::State<'_, AppState>,
    id: String,
    project_path: Option<String>,
) -> Result<Pipeline, String> {
    let pool = state.db_pool_ready().await?;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let data: serde_json::Value = sqlx::query_scalar("SELECT data FROM pipelines WHERE id=$1")
        .bind(uid)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::from_value(data).map_err(|e| e.to_string())
}

// ============================================================================
// Template management: save/list/delete user flow templates (JSON files)
// ============================================================================

#[tauri::command]
pub async fn pipeline_list_templates() -> Result<Vec<Pipeline>, String> {
    let mut all = builtin_templates();
    for p in &mut all {
        p.is_template = true;
    }
    Ok(all)
}

/// Save a pipeline as a JSON template file (non-async, for use from AI tools).
pub fn pipeline_save_template_inner(pipeline: &Pipeline) -> Result<String, String> {
    let dir = templates_dir().ok_or("Cannot determine app data directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let id = if pipeline.id.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        pipeline.id.clone()
    };
    let ts = now_ts();
    let entry = Pipeline {
        id: id.clone(),
        is_template: true,
        updated_at: ts,
        created_at: if pipeline.created_at == 0 { ts } else { pipeline.created_at },
        ..pipeline.clone()
    };
    let filename = format!("{}.json", entry.name.to_lowercase().replace(' ', "_"));
    let path = dir.join(&filename);
    let json = serde_json::to_string_pretty(&entry).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    tracing::info!("[pipeline] Saved template '{}' to {}", entry.name, path.display());
    Ok(id)
}

#[tauri::command]
pub async fn pipeline_save_template(pipeline: Pipeline) -> Result<String, String> {
    let dir = templates_dir().ok_or("Cannot determine app data directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let id = if pipeline.id.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        pipeline.id.clone()
    };
    let ts = now_ts();
    let entry = Pipeline {
        id: id.clone(),
        is_template: true,
        updated_at: ts,
        created_at: if pipeline.created_at == 0 { ts } else { pipeline.created_at },
        ..pipeline
    };

    let filename = format!("{}.json", entry.name.to_lowercase().replace(' ', "_"));
    let path = dir.join(&filename);
    let json = serde_json::to_string_pretty(&entry).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    tracing::info!("[pipeline] Saved template '{}' to {}", entry.name, path.display());
    Ok(id)
}

#[tauri::command]
pub async fn pipeline_delete_template(id: String) -> Result<(), String> {
    let dir = templates_dir().ok_or("Cannot determine app data directory")?;
    if !dir.exists() {
        return Ok(());
    }
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                if let Ok(data) = std::fs::read_to_string(&path) {
                    if let Ok(p) = serde_json::from_str::<Pipeline>(&data) {
                        if p.id == id {
                            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                            tracing::info!("[pipeline] Deleted template '{}' at {}", id, path.display());
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
