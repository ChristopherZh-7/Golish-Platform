//! Knowledge graph Tauri commands for the frontend KG viewer.
//!
//! The agent already has 5 `graph_*` LLM tools wired into the agentic
//! loop, but the desktop UI has had no way to query the graph directly.
//! These commands expose the three most useful read paths:
//!
//!   * `kg_list_entities`  — top N entities for a project / type
//!   * `kg_search_entities` — name substring search
//!   * `kg_get_neighbors`   — outgoing edges + destination entities
//!
//! Writes stay LLM-side via the `graph_*` tools so the frontend doesn't
//! need to know about the strong-typed entity / relation vocabulary.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::error::GolishError;
use crate::state::AppState;
use golish_graphiti::{GraphClient, GraphEntity, GraphRelation};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgEntity {
    pub id: String,
    pub entity_type: String,
    pub name: String,
    pub properties: serde_json::Value,
    pub project_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<GraphEntity> for KgEntity {
    fn from(e: GraphEntity) -> Self {
        Self {
            id: e.id.to_string(),
            entity_type: e.entity_type,
            name: e.name,
            properties: e.properties,
            project_id: e.project_id,
            created_at: e.created_at.to_rfc3339(),
            updated_at: e.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgRelation {
    pub id: String,
    pub from_entity_id: String,
    pub to_entity_id: String,
    pub relation_type: String,
    pub properties: serde_json::Value,
    pub created_at: String,
}

impl From<GraphRelation> for KgRelation {
    fn from(r: GraphRelation) -> Self {
        Self {
            id: r.id.to_string(),
            from_entity_id: r.from_entity_id.to_string(),
            to_entity_id: r.to_entity_id.to_string(),
            relation_type: r.relation_type,
            properties: r.properties,
            created_at: r.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgNeighbor {
    pub relation: KgRelation,
    pub entity: KgEntity,
}

fn make_client(state: &State<'_, AppState>) -> GraphClient {
    GraphClient::new((*state.db_pool).clone())
}

#[tauri::command]
pub async fn kg_list_entities(
    project_id: Option<String>,
    entity_type: Option<String>,
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<KgEntity>, GolishError> {
    let lim = limit.unwrap_or(50).clamp(1, 500);
    let client = make_client(&state);
    match client
        .list_entities(project_id.as_deref(), entity_type.as_deref(), lim)
        .await
    {
        Ok(rows) => Ok(rows.into_iter().map(Into::into).collect()),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "kg_list_entities: DB query failed, returning empty",
            );
            Ok(vec![])
        }
    }
}

#[tauri::command]
pub async fn kg_search_entities(
    query: String,
    entity_type: Option<String>,
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<KgEntity>, GolishError> {
    let lim = limit.unwrap_or(20).clamp(1, 200);
    let client = make_client(&state);
    match client
        .search_entities(&query, entity_type.as_deref(), lim)
        .await
    {
        Ok(rows) => Ok(rows.into_iter().map(Into::into).collect()),
        Err(e) => {
            tracing::warn!(error = %e, query = %query, "kg_search_entities failed");
            Ok(vec![])
        }
    }
}

#[tauri::command]
pub async fn kg_get_neighbors(
    entity_id: String,
    relation_type: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<KgNeighbor>, GolishError> {
    let id = match Uuid::parse_str(&entity_id) {
        Ok(u) => u,
        Err(e) => {
            return Err(GolishError::Internal(format!(
                "invalid entity_id '{}': {}",
                entity_id, e
            )))
        }
    };
    let client = make_client(&state);
    match client.get_neighbors(id, relation_type.as_deref()).await {
        Ok(rows) => Ok(rows
            .into_iter()
            .map(|(rel, ent)| KgNeighbor {
                relation: rel.into(),
                entity: ent.into(),
            })
            .collect()),
        Err(e) => {
            tracing::warn!(error = %e, entity_id = %entity_id, "kg_get_neighbors failed");
            Ok(vec![])
        }
    }
}

// Suppress unused import warning if Arc isn't needed in all builds.
#[allow(dead_code)]
fn _unused_arc(_p: Arc<sqlx::PgPool>) {}
