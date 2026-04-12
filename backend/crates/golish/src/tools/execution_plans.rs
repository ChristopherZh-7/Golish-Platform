use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDto {
    pub id: String,
    pub session_id: Option<String>,
    pub project_path: Option<String>,
    pub title: String,
    pub description: String,
    pub steps: Vec<PlanStepDto>,
    pub status: String,
    pub current_step: i32,
    pub context: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStepDto {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub status: String,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
}

fn model_to_dto(plan: golish_db::models::ExecutionPlan) -> PlanDto {
    let steps: Vec<PlanStepDto> = serde_json::from_value(plan.steps.clone()).unwrap_or_default();
    PlanDto {
        id: plan.id.to_string(),
        session_id: plan.session_id.map(|s| s.to_string()),
        project_path: plan.project_path,
        title: plan.title,
        description: plan.description,
        steps,
        status: format!("{:?}", plan.status).to_lowercase(),
        current_step: plan.current_step,
        context: plan.context,
        created_at: plan.created_at.to_rfc3339(),
        updated_at: plan.updated_at.to_rfc3339(),
    }
}

#[tauri::command]
pub async fn plan_create(
    state: tauri::State<'_, AppState>,
    project_path: String,
    title: String,
    description: String,
    steps: Vec<PlanStepDto>,
    session_id: Option<String>,
) -> Result<PlanDto, String> {
    let pool = &*state.db_pool;
    let sid = session_id
        .and_then(|s| s.parse::<uuid::Uuid>().ok());
    let steps_json = serde_json::to_value(&steps).map_err(|e| e.to_string())?;

    let plan = golish_db::repo::execution_plans::create(
        pool,
        golish_db::models::NewExecutionPlan {
            session_id: sid,
            project_path: Some(project_path),
            title,
            description,
            steps: steps_json,
        },
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(model_to_dto(plan))
}

#[tauri::command]
pub async fn plan_get(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Option<PlanDto>, String> {
    let pool = &*state.db_pool;
    let uid: uuid::Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let plan = golish_db::repo::execution_plans::get(pool, uid)
        .await
        .map_err(|e| e.to_string())?;
    Ok(plan.map(model_to_dto))
}

#[tauri::command]
pub async fn plan_list(
    state: tauri::State<'_, AppState>,
    project_path: String,
    include_completed: Option<bool>,
) -> Result<Vec<PlanDto>, String> {
    let pool = &*state.db_pool;
    let plans = golish_db::repo::execution_plans::list_by_project(
        pool,
        &project_path,
        include_completed.unwrap_or(false),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(plans.into_iter().map(model_to_dto).collect())
}

#[tauri::command]
pub async fn plan_list_active(
    state: tauri::State<'_, AppState>,
    project_path: String,
) -> Result<Vec<PlanDto>, String> {
    let pool = &*state.db_pool;
    let plans = golish_db::repo::execution_plans::list_active(pool, &project_path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(plans.into_iter().map(model_to_dto).collect())
}

#[tauri::command]
pub async fn plan_update_steps(
    state: tauri::State<'_, AppState>,
    id: String,
    steps: Vec<PlanStepDto>,
    current_step: i32,
    status: String,
) -> Result<(), String> {
    let pool = &*state.db_pool;
    let uid: uuid::Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let steps_json = serde_json::to_value(&steps).map_err(|e| e.to_string())?;
    let plan_status = parse_plan_status(&status)?;

    golish_db::repo::execution_plans::update_steps(pool, uid, &steps_json, current_step, plan_status)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plan_update_status(
    state: tauri::State<'_, AppState>,
    id: String,
    status: String,
) -> Result<(), String> {
    let pool = &*state.db_pool;
    let uid: uuid::Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let plan_status = parse_plan_status(&status)?;

    golish_db::repo::execution_plans::update_status(pool, uid, plan_status)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plan_update_context(
    state: tauri::State<'_, AppState>,
    id: String,
    context: serde_json::Value,
) -> Result<(), String> {
    let pool = &*state.db_pool;
    let uid: uuid::Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;

    golish_db::repo::execution_plans::update_context(pool, uid, &context)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plan_delete(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let pool = &*state.db_pool;
    let uid: uuid::Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;

    golish_db::repo::execution_plans::delete(pool, uid)
        .await
        .map_err(|e| e.to_string())
}

fn parse_plan_status(s: &str) -> Result<golish_db::models::PlanStatus, String> {
    match s {
        "planning" => Ok(golish_db::models::PlanStatus::Planning),
        "in_progress" => Ok(golish_db::models::PlanStatus::InProgress),
        "paused" => Ok(golish_db::models::PlanStatus::Paused),
        "completed" => Ok(golish_db::models::PlanStatus::Completed),
        "failed" => Ok(golish_db::models::PlanStatus::Failed),
        "cancelled" => Ok(golish_db::models::PlanStatus::Cancelled),
        _ => Err(format!("Invalid plan status: {}", s)),
    }
}
