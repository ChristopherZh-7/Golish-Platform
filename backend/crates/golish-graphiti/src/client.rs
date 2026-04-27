//! `GraphClient` — high-level operations on `graph_entities` / `graph_relations`.

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::GraphError;
use crate::types::{GraphEntity, GraphRelation};

/// Async handle for reading and writing the security knowledge graph.
///
/// Cheap to clone (the underlying `PgPool` is reference-counted), so it can be
/// passed by value into agent state.
#[derive(Debug, Clone)]
pub struct GraphClient {
    pool: PgPool,
}

impl GraphClient {
    /// Construct a new client over an existing connection pool. The pool must
    /// point at the database where the `20260427000001_graph_knowledge_base`
    /// migration has been applied (typically the embedded PG instance owned
    /// by `golish-db`).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert an entity, or merge into an existing one with the same
    /// `(entity_type, name, project_id)` triple.
    ///
    /// On conflict, `properties` is shallow-merged using JSONB `||` so newly
    /// supplied fields overwrite older values while preserving keys that the
    /// caller did not touch this turn. `updated_at` is bumped to `NOW()`.
    pub async fn upsert_entity(
        &self,
        entity_type: &str,
        name: &str,
        properties: Value,
        project_id: Option<&str>,
    ) -> Result<GraphEntity, GraphError> {
        let row = sqlx::query_as::<_, GraphEntity>(
            r#"INSERT INTO graph_entities (entity_type, name, properties, project_id)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (entity_type, name, COALESCE(project_id, '')) DO UPDATE SET
                   properties = graph_entities.properties || EXCLUDED.properties,
                   updated_at = NOW()
               RETURNING *"#,
        )
        .bind(entity_type)
        .bind(name)
        .bind(properties)
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Insert a directed edge `from_id -> to_id` of `relation_type`, or merge
    /// `properties` into the existing edge if one already exists.
    pub async fn upsert_relation(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        relation_type: &str,
        properties: Value,
    ) -> Result<GraphRelation, GraphError> {
        let row = sqlx::query_as::<_, GraphRelation>(
            r#"INSERT INTO graph_relations (from_entity_id, to_entity_id, relation_type, properties)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (from_entity_id, to_entity_id, relation_type) DO UPDATE SET
                   properties = graph_relations.properties || EXCLUDED.properties
               RETURNING *"#,
        )
        .bind(from_id)
        .bind(to_id)
        .bind(relation_type)
        .bind(properties)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Search entities whose `name` contains `query` (case-insensitive),
    /// optionally restricted to a single `entity_type`. Most-recently-updated
    /// first.
    pub async fn search_entities(
        &self,
        query: &str,
        entity_type: Option<&str>,
        limit: i64,
    ) -> Result<Vec<GraphEntity>, GraphError> {
        let pattern = format!("%{}%", query);
        let rows = sqlx::query_as::<_, GraphEntity>(
            r#"SELECT * FROM graph_entities
               WHERE name ILIKE $1
                 AND ($2::text IS NULL OR entity_type = $2)
               ORDER BY updated_at DESC
               LIMIT $3"#,
        )
        .bind(pattern)
        .bind(entity_type)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Return every direct neighbor of `entity_id` (outgoing edges), optionally
    /// filtered to a single `relation_type`. Each tuple pairs the edge with
    /// the destination entity.
    pub async fn get_neighbors(
        &self,
        entity_id: Uuid,
        relation_type: Option<&str>,
    ) -> Result<Vec<(GraphRelation, GraphEntity)>, GraphError> {
        let rows = sqlx::query_as::<_, NeighborRow>(
            r#"SELECT
                   r.id              AS r_id,
                   r.from_entity_id  AS r_from_entity_id,
                   r.to_entity_id    AS r_to_entity_id,
                   r.relation_type   AS r_relation_type,
                   r.properties      AS r_properties,
                   r.created_at      AS r_created_at,
                   e.id              AS e_id,
                   e.entity_type     AS e_entity_type,
                   e.name            AS e_name,
                   e.properties      AS e_properties,
                   e.session_id      AS e_session_id,
                   e.project_id      AS e_project_id,
                   e.created_at      AS e_created_at,
                   e.updated_at      AS e_updated_at
               FROM graph_relations r
               JOIN graph_entities  e ON e.id = r.to_entity_id
               WHERE r.from_entity_id = $1
                 AND ($2::text IS NULL OR r.relation_type = $2)
               ORDER BY r.created_at DESC"#,
        )
        .bind(entity_id)
        .bind(relation_type)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(NeighborRow::split).collect())
    }

    /// Enumerate distinct attack paths starting at `from_entity_id`, walking
    /// at most `max_depth` outgoing edges. Each result is the ordered list of
    /// entities along one root-to-leaf path; cycles are pruned by the
    /// recursive CTE's path-tracking column.
    pub async fn find_attack_paths(
        &self,
        from_entity_id: Uuid,
        max_depth: i32,
    ) -> Result<Vec<Vec<GraphEntity>>, GraphError> {
        if max_depth <= 0 {
            return Err(GraphError::InvalidArgument(
                "max_depth must be > 0".to_string(),
            ));
        }

        let path_rows: Vec<(Vec<Uuid>,)> = sqlx::query_as(
            r#"WITH RECURSIVE walk(node_id, depth, path) AS (
                   SELECT id, 0, ARRAY[id]
                   FROM graph_entities
                   WHERE id = $1
                 UNION ALL
                   SELECT r.to_entity_id, w.depth + 1, w.path || r.to_entity_id
                   FROM walk w
                   JOIN graph_relations r ON r.from_entity_id = w.node_id
                   WHERE w.depth < $2
                     AND NOT (r.to_entity_id = ANY(w.path))
               )
               SELECT path
               FROM walk
               WHERE depth > 0
               ORDER BY array_length(path, 1) ASC, path"#,
        )
        .bind(from_entity_id)
        .bind(max_depth)
        .fetch_all(&self.pool)
        .await?;

        let mut paths: Vec<Vec<GraphEntity>> = Vec::with_capacity(path_rows.len());
        for (ids,) in path_rows {
            let entities = sqlx::query_as::<_, GraphEntity>(
                r#"SELECT e.*
                   FROM unnest($1::uuid[]) WITH ORDINALITY AS u(id, ord)
                   JOIN graph_entities e ON e.id = u.id
                   ORDER BY u.ord"#,
            )
            .bind(&ids)
            .fetch_all(&self.pool)
            .await?;
            paths.push(entities);
        }

        Ok(paths)
    }

    /// List entities, optionally filtered by `project_id` and/or `entity_type`.
    /// Most-recently-updated first.
    pub async fn list_entities(
        &self,
        project_id: Option<&str>,
        entity_type: Option<&str>,
        limit: i64,
    ) -> Result<Vec<GraphEntity>, GraphError> {
        let rows = sqlx::query_as::<_, GraphEntity>(
            r#"SELECT * FROM graph_entities
               WHERE ($1::text IS NULL OR project_id = $1)
                 AND ($2::text IS NULL OR entity_type = $2)
               ORDER BY updated_at DESC
               LIMIT $3"#,
        )
        .bind(project_id)
        .bind(entity_type)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Delete a single entity by id. Relations touching it are removed by the
    /// `ON DELETE CASCADE` on `graph_relations`.
    pub async fn delete_entity(&self, entity_id: Uuid) -> Result<(), GraphError> {
        let result = sqlx::query("DELETE FROM graph_entities WHERE id = $1")
            .bind(entity_id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(GraphError::NotFound(entity_id.to_string()));
        }
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct NeighborRow {
    r_id: Uuid,
    r_from_entity_id: Uuid,
    r_to_entity_id: Uuid,
    r_relation_type: String,
    r_properties: Value,
    r_created_at: chrono::DateTime<chrono::Utc>,
    e_id: Uuid,
    e_entity_type: String,
    e_name: String,
    e_properties: Value,
    e_session_id: Option<Uuid>,
    e_project_id: Option<String>,
    e_created_at: chrono::DateTime<chrono::Utc>,
    e_updated_at: chrono::DateTime<chrono::Utc>,
}

impl NeighborRow {
    fn split(self) -> (GraphRelation, GraphEntity) {
        (
            GraphRelation {
                id: self.r_id,
                from_entity_id: self.r_from_entity_id,
                to_entity_id: self.r_to_entity_id,
                relation_type: self.r_relation_type,
                properties: self.r_properties,
                created_at: self.r_created_at,
            },
            GraphEntity {
                id: self.e_id,
                entity_type: self.e_entity_type,
                name: self.e_name,
                properties: self.e_properties,
                session_id: self.e_session_id,
                project_id: self.e_project_id,
                created_at: self.e_created_at,
                updated_at: self.e_updated_at,
            },
        )
    }
}
