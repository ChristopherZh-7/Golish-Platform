//! Trait abstraction for the graph knowledge base, decoupling golish-ai from
//! the concrete `golish-graphiti` crate.

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

#[async_trait]
pub trait GraphKnowledgeBase: Send + Sync {
    async fn upsert_entity(
        &self,
        entity_type: &str,
        name: &str,
        properties: Value,
        session_id: Option<Uuid>,
    ) -> anyhow::Result<GraphEntityView>;

    async fn upsert_relation(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        relation_type: &str,
        properties: Value,
    ) -> anyhow::Result<GraphRelationView>;

    async fn search_entities(
        &self,
        query: &str,
        entity_type: Option<&str>,
        limit: i64,
    ) -> anyhow::Result<Vec<GraphEntityView>>;

    async fn get_neighbors(
        &self,
        entity_id: Uuid,
        relation_type: Option<&str>,
    ) -> anyhow::Result<Vec<(GraphRelationView, GraphEntityView)>>;

    async fn find_attack_paths(
        &self,
        from_id: Uuid,
        max_depth: i32,
    ) -> anyhow::Result<Vec<Vec<GraphEntityView>>>;

    /// Most-recently-updated entities, optionally filtered by
    /// `project_id` (e.g. the workspace path) and/or `entity_type`.
    /// Used by the sub-agent briefing builder and the frontend KG
    /// viewer. Default impl wraps `search_entities("", ...)` so legacy
    /// backends keep working without explicit overrides.
    async fn list_entities(
        &self,
        project_id: Option<&str>,
        entity_type: Option<&str>,
        limit: i64,
    ) -> anyhow::Result<Vec<GraphEntityView>> {
        let _ = project_id;
        // Default: fall back to a permissive name-search.
        self.search_entities("", entity_type, limit).await
    }
}

/// Lightweight view of a graph entity (no dependency on golish-graphiti types).
#[derive(Debug, Clone)]
pub struct GraphEntityView {
    pub id: Uuid,
    pub entity_type: String,
    pub name: String,
    pub properties: Value,
    pub session_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Lightweight view of a graph relation.
#[derive(Debug, Clone)]
pub struct GraphRelationView {
    pub id: Uuid,
    pub from_entity_id: Uuid,
    pub to_entity_id: Uuid,
    pub relation_type: String,
    pub properties: Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
