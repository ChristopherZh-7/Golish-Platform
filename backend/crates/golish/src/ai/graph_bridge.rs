//! Bridge between the `golish-graphiti` crate and the `GraphKnowledgeBase`
//! trait defined in `golish-ai`, so golish-ai never depends on golish-graphiti
//! directly.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use golish_agent_kit::tool_executors::graph_trait::{
    GraphEntityView, GraphKnowledgeBase, GraphRelationView,
};
use golish_graphiti::GraphClient;

pub struct GraphClientBackend {
    client: GraphClient,
}

impl GraphClientBackend {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self {
            client: GraphClient::new((*pool).clone()),
        }
    }
}

#[async_trait]
impl GraphKnowledgeBase for GraphClientBackend {
    async fn upsert_entity(
        &self,
        entity_type: &str,
        name: &str,
        properties: Value,
        session_id: Option<Uuid>,
    ) -> anyhow::Result<GraphEntityView> {
        let project_id_str = session_id.map(|id| id.to_string());
        let entity = self
            .client
            .upsert_entity(entity_type, name, properties, project_id_str.as_deref())
            .await?;
        Ok(GraphEntityView {
            id: entity.id,
            entity_type: entity.entity_type,
            name: entity.name,
            properties: entity.properties,
            session_id: entity.session_id,
            project_id: entity.project_id.and_then(|s| Uuid::parse_str(&s).ok()),
            created_at: entity.created_at,
            updated_at: entity.updated_at,
        })
    }

    async fn upsert_relation(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        relation_type: &str,
        properties: Value,
    ) -> anyhow::Result<GraphRelationView> {
        let rel = self
            .client
            .upsert_relation(from_id, to_id, relation_type, properties)
            .await?;
        Ok(GraphRelationView {
            id: rel.id,
            from_entity_id: rel.from_entity_id,
            to_entity_id: rel.to_entity_id,
            relation_type: rel.relation_type,
            properties: rel.properties,
            created_at: rel.created_at,
        })
    }

    async fn search_entities(
        &self,
        query: &str,
        entity_type: Option<&str>,
        limit: i64,
    ) -> anyhow::Result<Vec<GraphEntityView>> {
        let entities = self
            .client
            .search_entities(query, entity_type, limit)
            .await?;
        Ok(entities
            .into_iter()
            .map(|e| GraphEntityView {
                id: e.id,
                entity_type: e.entity_type,
                name: e.name,
                properties: e.properties,
                session_id: e.session_id,
                project_id: e.project_id.and_then(|s| Uuid::parse_str(&s).ok()),
                created_at: e.created_at,
                updated_at: e.updated_at,
            })
            .collect())
    }

    async fn get_neighbors(
        &self,
        entity_id: Uuid,
        relation_type: Option<&str>,
    ) -> anyhow::Result<Vec<(GraphRelationView, GraphEntityView)>> {
        let rows = self.client.get_neighbors(entity_id, relation_type).await?;
        Ok(rows
            .into_iter()
            .map(|(rel, ent)| {
                (
                    GraphRelationView {
                        id: rel.id,
                        from_entity_id: rel.from_entity_id,
                        to_entity_id: rel.to_entity_id,
                        relation_type: rel.relation_type,
                        properties: rel.properties,
                        created_at: rel.created_at,
                    },
                    GraphEntityView {
                        id: ent.id,
                        entity_type: ent.entity_type,
                        name: ent.name,
                        properties: ent.properties,
                        session_id: ent.session_id,
                        project_id: ent.project_id.and_then(|s| Uuid::parse_str(&s).ok()),
                        created_at: ent.created_at,
                        updated_at: ent.updated_at,
                    },
                )
            })
            .collect())
    }

    async fn find_attack_paths(
        &self,
        from_id: Uuid,
        max_depth: i32,
    ) -> anyhow::Result<Vec<Vec<GraphEntityView>>> {
        let paths = self.client.find_attack_paths(from_id, max_depth).await?;
        Ok(paths
            .into_iter()
            .map(|path| {
                path.into_iter()
                    .map(|e| GraphEntityView {
                        id: e.id,
                        entity_type: e.entity_type,
                        name: e.name,
                        properties: e.properties,
                        session_id: e.session_id,
                        project_id: e.project_id.and_then(|s| Uuid::parse_str(&s).ok()),
                        created_at: e.created_at,
                        updated_at: e.updated_at,
                    })
                    .collect()
            })
            .collect())
    }
}
