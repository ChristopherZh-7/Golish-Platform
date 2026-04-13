use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomPassiveRule {
    pub id: String,
    pub name: String,
    pub pattern: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_scope() -> String {
    "all".to_string()
}
fn default_severity() -> String {
    "medium".to_string()
}
fn default_true() -> bool {
    true
}

#[tauri::command]
pub async fn custom_rules_list(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
) -> Result<Vec<CustomPassiveRule>, String> {
    let pool = state.db_pool_ready().await?;
    let rows: Vec<(String, String, String, String, String, bool)> = sqlx::query_as(
        "SELECT id, name, pattern, scope, severity, enabled \
         FROM custom_passive_rules WHERE project_path IS NOT DISTINCT FROM $1 \
         ORDER BY created_at ASC",
    )
    .bind(project_path.as_deref())
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|(id, name, pattern, scope, severity, enabled)| CustomPassiveRule {
            id,
            name,
            pattern,
            scope,
            severity,
            enabled,
        })
        .collect())
}

#[tauri::command]
pub async fn custom_rules_upsert(
    state: tauri::State<'_, AppState>,
    rule: CustomPassiveRule,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    sqlx::query(
        r#"INSERT INTO custom_passive_rules (id, name, pattern, scope, severity, enabled, project_path)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (id) DO UPDATE SET
             name = EXCLUDED.name,
             pattern = EXCLUDED.pattern,
             scope = EXCLUDED.scope,
             severity = EXCLUDED.severity,
             enabled = EXCLUDED.enabled,
             updated_at = NOW()"#,
    )
    .bind(&rule.id)
    .bind(&rule.name)
    .bind(&rule.pattern)
    .bind(&rule.scope)
    .bind(&rule.severity)
    .bind(rule.enabled)
    .bind(project_path.as_deref())
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn custom_rules_save_all(
    state: tauri::State<'_, AppState>,
    rules: Vec<CustomPassiveRule>,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;

    sqlx::query("DELETE FROM custom_passive_rules WHERE project_path IS NOT DISTINCT FROM $1")
        .bind(project_path.as_deref())
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    for rule in &rules {
        sqlx::query(
            r#"INSERT INTO custom_passive_rules (id, name, pattern, scope, severity, enabled, project_path)
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        )
        .bind(&rule.id)
        .bind(&rule.name)
        .bind(&rule.pattern)
        .bind(&rule.scope)
        .bind(&rule.severity)
        .bind(rule.enabled)
        .bind(project_path.as_deref())
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[tauri::command]
pub async fn custom_rules_delete(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    sqlx::query("DELETE FROM custom_passive_rules WHERE id = $1")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
