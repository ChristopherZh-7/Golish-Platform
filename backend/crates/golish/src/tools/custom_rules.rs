use crate::error::GolishError;
use serde::{Deserialize, Serialize};

use crate::state::DbState;

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
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<Vec<CustomPassiveRule>, GolishError> {
    let pool = state.pool_ready().await?;
    let rows: Vec<(String, String, String, String, String, bool)> =
        golish_db::repo::custom_rules::list_by_project(pool, project_path.as_deref()).await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, name, pattern, scope, severity, enabled)| CustomPassiveRule {
                id,
                name,
                pattern,
                scope,
                severity,
                enabled,
            },
        )
        .collect())
}

#[tauri::command]
pub async fn custom_rules_upsert(
    state: tauri::State<'_, DbState>,
    rule: CustomPassiveRule,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
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
?;
    Ok(())
}

#[tauri::command]
pub async fn custom_rules_save_all(
    state: tauri::State<'_, DbState>,
    rules: Vec<CustomPassiveRule>,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;

    golish_db::repo::custom_rules::clear_by_project(pool, project_path.as_deref()).await?;

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
?;
    }

    Ok(())
}

#[tauri::command]
pub async fn custom_rules_delete(
    state: tauri::State<'_, DbState>,
    id: String,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    sqlx::query("DELETE FROM custom_passive_rules WHERE id = $1")
        .bind(&id)
        .execute(pool)
        .await?;
    Ok(())
}
