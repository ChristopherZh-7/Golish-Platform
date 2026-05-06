//! Tauri command wrappers around `golish_pipeline::*`.
//!
//! These retain the original argument/return shapes (so the frontend and
//! other integrations don't need to change) while internally:
//! 1. Borrow the pool + config manager off `AppState`.
//! 2. Build a `TauriEventEmitter` and a `MainStorage`.
//! 3. Delegate to the new crate's `execute_pipeline_headless` /
//!    template helpers.

use std::sync::atomic::Ordering;

use golish_pipeline::{
    builtin_templates, now_ts, templates_dir, Pipeline, PipelineRunResult, PIPELINE_CANCELLED,
};
use serde::Serialize;
use uuid::Uuid;

use crate::error::GolishError;
use crate::event_emitter::TauriEventEmitter;
use crate::state::DbState;
use crate::tools::pentest_bridge::{
    ai_tool_catalog_entry, create_pentest_bridge_tools,
};

use super::storage::MainStorage;

#[tauri::command]
pub async fn pipeline_cancel() -> Result<(), GolishError> {
    PIPELINE_CANCELLED.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub async fn pipeline_execute(
    app: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    pentest_cfg: tauri::State<'_, std::sync::Arc<golish_pentest::ConfigManager>>,
    pipeline: Pipeline,
    target: String,
    project_path: Option<String>,
) -> Result<PipelineRunResult, GolishError> {
    PIPELINE_CANCELLED.store(false, Ordering::SeqCst);

    let pool = state.pool_ready().await?;
    let emitter = TauriEventEmitter::handle(app.clone());
    let storage = MainStorage;

    // Wire the in-process AI tool registry so steps with
    // `step_type = "ai_tool"` can call `js_collect`, `js_extract_apis`,
    // `auth_probe`, etc. inline. Built lazily here (per-execution) because
    // each `Tool` impl borrows the pool / config, both of which already
    // live in `AppState`.
    let pool_arc = std::sync::Arc::new(pool.clone());
    let ai_tools = create_pentest_bridge_tools(
        pool_arc,
        std::sync::Arc::clone(&pentest_cfg),
        Some(app),
    );

    let result = golish_pipeline::execute_pipeline_headless_with_ai_tools(
        pool,
        &pipeline,
        &target,
        project_path.as_deref(),
        &pentest_cfg,
        &storage,
        Some(&emitter),
        None,
        Some(&ai_tools),
    )
    .await?;

    PIPELINE_CANCELLED.store(false, Ordering::SeqCst);
    Ok(result)
}

/// Public DTO for the `pentest_list_ai_tools` Tauri command. Mirrors the
/// `Tool` trait's `name() / description() / parameters()` plus the static
/// catalog metadata the UI needs to group and icon entries.
#[derive(Debug, Clone, Serialize)]
pub struct AiToolMeta {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    /// `recon`, `scan`, `data`, `control`, or `other` (when the tool isn't
    /// in `pentest_bridge::ai_tool_catalog`).
    pub category: String,
    pub icon: String,
}

/// List all AI tools available to the Pipeline editor. Driven by the same
/// factory `pipeline_execute` uses so the picker and the runtime never
/// drift out of sync.
#[tauri::command]
pub async fn pipeline_list_ai_tools(
    app: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    pentest_cfg: tauri::State<'_, std::sync::Arc<golish_pentest::ConfigManager>>,
) -> Result<Vec<AiToolMeta>, GolishError> {
    let pool = state.pool_ready().await?;
    let pool_arc = std::sync::Arc::new(pool.clone());
    let tools = create_pentest_bridge_tools(
        pool_arc,
        std::sync::Arc::clone(&pentest_cfg),
        Some(app),
    );

    let mut metas = Vec::with_capacity(tools.len());
    for tool in &tools {
        let name = tool.name();
        let entry = ai_tool_catalog_entry(name);
        metas.push(AiToolMeta {
            name: name.to_string(),
            description: tool.description().to_string(),
            parameters: tool.parameters(),
            category: entry
                .as_ref()
                .map(|e| e.category.to_string())
                .unwrap_or_else(|| "other".to_string()),
            icon: entry
                .as_ref()
                .map(|e| e.icon.to_string())
                .unwrap_or_else(|| "🤖".to_string()),
        });
    }
    Ok(metas)
}

#[tauri::command]
pub async fn pipeline_list(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<Vec<Pipeline>, GolishError> {
    let pool = state.pool_ready().await?;
    let rows: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT data FROM pipelines WHERE project_path = $1 ORDER BY updated_at DESC",
    )
    .bind(project_path.as_deref())
    .fetch_all(pool)
    .await
    ?;

    let items: Vec<Pipeline> = rows
        .into_iter()
        .filter_map(|j| serde_json::from_value(j).ok())
        .collect();

    let saved_workflow_ids: std::collections::HashSet<&str> = items
        .iter()
        .filter_map(|p| p.workflow_id.as_deref())
        .collect();

    let mut result: Vec<Pipeline> = builtin_templates()
        .into_iter()
        .filter(|t| {
            t.workflow_id
                .as_deref()
                .map_or(true, |wid| !saved_workflow_ids.contains(wid))
        })
        .collect();
    result.extend(items);
    Ok(result)
}

#[tauri::command]
pub async fn pipeline_save(
    state: tauri::State<'_, DbState>,
    pipeline: Pipeline,
    project_path: Option<String>,
) -> Result<String, GolishError> {
    let pool = state.pool_ready().await?;
    let id = if pipeline.id.is_empty() || pipeline.id.parse::<Uuid>().is_err() {
        Uuid::new_v4().to_string()
    } else {
        pipeline.id.clone()
    };
    let ts = now_ts();
    let entry = Pipeline {
        id: id.clone(),
        updated_at: ts,
        created_at: if pipeline.created_at == 0 {
            ts
        } else {
            pipeline.created_at
        },
        ..pipeline
    };
    let json = serde_json::to_value(&entry)?;
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
    ?;
    Ok(id)
}

#[tauri::command]
pub async fn pipeline_delete(
    state: tauri::State<'_, DbState>,
    id: String,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let _ = project_path;
    let Ok(uid) = id.parse::<Uuid>() else {
        return Ok(());
    };
    sqlx::query("DELETE FROM pipelines WHERE id=$1")
        .bind(uid)
        .execute(pool)
        .await
        ?;
    Ok(())
}

#[tauri::command]
pub async fn pipeline_load(
    state: tauri::State<'_, DbState>,
    id: String,
    project_path: Option<String>,
) -> Result<Pipeline, GolishError> {
    let pool = state.pool_ready().await?;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| GolishError::Validation(e.to_string()))?;
    let data: serde_json::Value = sqlx::query_scalar("SELECT data FROM pipelines WHERE id=$1")
        .bind(uid)
        .fetch_one(pool)
        .await?;
    Ok(serde_json::from_value(data)?)
}

#[tauri::command]
pub async fn pipeline_list_templates() -> Result<Vec<Pipeline>, GolishError> {
    let mut all = builtin_templates();
    for p in &mut all {
        p.is_template = true;
    }
    Ok(all)
}

/// Save a pipeline as a JSON template file (non-async, for use from AI tools).
pub fn pipeline_save_template_inner(pipeline: &Pipeline) -> Result<String, GolishError> {
    let dir = templates_dir().ok_or("Cannot determine app data directory")?;
    std::fs::create_dir_all(&dir)?;
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
        created_at: if pipeline.created_at == 0 {
            ts
        } else {
            pipeline.created_at
        },
        ..pipeline.clone()
    };
    let filename = format!("{}.json", entry.name.to_lowercase().replace(' ', "_"));
    let path = dir.join(&filename);
    let json = serde_json::to_string_pretty(&entry)?;
    std::fs::write(&path, json)?;
    tracing::info!(
        "[pipeline] Saved template '{}' to {}",
        entry.name,
        path.display()
    );
    Ok(id)
}

#[tauri::command]
pub async fn pipeline_save_template(pipeline: Pipeline) -> Result<String, GolishError> {
    pipeline_save_template_inner(&pipeline)
}

#[tauri::command]
pub async fn pipeline_delete_template(id: String) -> Result<(), GolishError> {
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
                            std::fs::remove_file(&path)?;
                            tracing::info!(
                                "[pipeline] Deleted template '{}' at {}",
                                id,
                                path.display()
                            );
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
