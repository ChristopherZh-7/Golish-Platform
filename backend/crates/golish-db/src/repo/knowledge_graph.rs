use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

const EMPTY_BUILD_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphScopeRecord {
    pub scope_key: String,
    pub visibility: String,
    pub project_scope_id: Option<Uuid>,
    pub organization_id_at_time: Option<Uuid>,
}

impl GraphScopeRecord {
    pub fn organization(project_scope_id: Uuid, organization_id_at_time: Uuid) -> Self {
        Self {
            scope_key: format!("org:{project_scope_id}:{organization_id_at_time}"),
            visibility: "organization_long_term".to_string(),
            project_scope_id: Some(project_scope_id),
            organization_id_at_time: Some(organization_id_at_time),
        }
    }

    pub fn global_sanitized() -> Self {
        Self {
            scope_key: "global_sanitized".to_string(),
            visibility: "global_sanitized".to_string(),
            project_scope_id: None,
            organization_id_at_time: None,
        }
    }

    pub fn validate(&self) -> Result<(), KnowledgeGraphError> {
        let valid = match (
            self.visibility.as_str(),
            self.project_scope_id,
            self.organization_id_at_time,
        ) {
            ("organization_long_term", Some(project), Some(organization)) => {
                self.scope_key == format!("org:{project}:{organization}")
            }
            ("global_sanitized", None, None) => self.scope_key == "global_sanitized",
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(KnowledgeGraphError::InvalidScope)
        }
    }
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct GraphGenerationRow {
    pub generation_id: Uuid,
    pub scope_key: String,
    pub visibility: String,
    pub project_scope_id: Option<Uuid>,
    pub organization_id_at_time: Option<Uuid>,
    pub projection_schema_version: i32,
    pub status: String,
    pub build_hash: Option<String>,
    pub entity_count: Option<i64>,
    pub relation_count: Option<i64>,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub activated_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct EntityIdentityWrite {
    pub entity_id: Uuid,
    pub generation_id: Uuid,
    pub scope: GraphScopeRecord,
    pub canonical_ref: String,
    pub identity_hash: String,
    pub entity_type: String,
    pub display_name: String,
    pub properties: Value,
}

#[derive(Clone, Debug, sqlx::FromRow, PartialEq)]
pub struct GraphEntityRow {
    pub entity_id: Uuid,
    pub generation_id: Uuid,
    pub scope_key: String,
    pub visibility: String,
    pub project_scope_id: Option<Uuid>,
    pub organization_id_at_time: Option<Uuid>,
    pub canonical_ref: String,
    pub identity_hash: String,
    pub entity_type: String,
    pub display_name: String,
    pub properties: Value,
}

#[derive(Clone, Debug)]
pub struct AssertionLineageWrite {
    pub assertion_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub evidence_refs: Vec<i64>,
    pub status: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub fresh_until: Option<DateTime<Utc>>,
    pub classification: String,
    pub projection_schema_version: i32,
}

#[derive(Clone, Debug, sqlx::FromRow, PartialEq)]
pub struct EntityAssertionRow {
    pub entity_id: Uuid,
    pub generation_id: Uuid,
    pub assertion_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub evidence_refs: Vec<i64>,
    pub status: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub fresh_until: Option<DateTime<Utc>>,
    pub classification: String,
    pub projection_schema_version: i32,
}

#[derive(Clone, Debug)]
pub struct RelationIdentityWrite {
    pub relation_id: Uuid,
    pub generation_id: Uuid,
    pub scope_key: String,
    pub from_entity_id: Uuid,
    pub to_entity_id: Uuid,
    pub relation_type: String,
    pub identity_hash: String,
    pub properties: Value,
}

#[derive(Clone, Debug, sqlx::FromRow, PartialEq)]
pub struct GraphRelationRow {
    pub relation_id: Uuid,
    pub generation_id: Uuid,
    pub scope_key: String,
    pub from_entity_id: Uuid,
    pub to_entity_id: Uuid,
    pub relation_type: String,
    pub identity_hash: String,
    pub properties: Value,
}

#[derive(Clone, Debug, sqlx::FromRow, PartialEq)]
pub struct RelationAssertionRow {
    pub relation_id: Uuid,
    pub generation_id: Uuid,
    pub assertion_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub evidence_refs: Vec<i64>,
    pub status: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub fresh_until: Option<DateTime<Utc>>,
    pub classification: String,
    pub projection_schema_version: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineageWriteDisposition {
    Inserted,
    ExactReplay,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerationAttestationRow {
    pub build_hash: String,
    pub entity_count: i64,
    pub relation_count: i64,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct ActiveEntityLineageRow {
    pub generation_id: Uuid,
    pub entity_id: Uuid,
    pub scope_key: String,
    pub visibility: String,
    pub project_scope_id: Option<Uuid>,
    pub organization_id_at_time: Option<Uuid>,
    pub canonical_ref: String,
    pub entity_type: String,
    pub display_name: String,
    pub properties: Value,
    pub assertion_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub evidence_refs: Vec<i64>,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub fresh_until: Option<DateTime<Utc>>,
    pub classification: String,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct ActiveRelationLineageRow {
    pub generation_id: Uuid,
    pub relation_id: Uuid,
    pub scope_key: String,
    pub project_scope_id: Option<Uuid>,
    pub organization_id_at_time: Option<Uuid>,
    pub from_entity_id: Uuid,
    pub from_canonical_ref: String,
    pub from_entity_type: String,
    pub from_display_name: String,
    pub to_entity_id: Uuid,
    pub to_canonical_ref: String,
    pub to_entity_type: String,
    pub to_display_name: String,
    pub relation_type: String,
    pub properties: Value,
    pub assertion_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub evidence_refs: Vec<i64>,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub fresh_until: Option<DateTime<Utc>>,
    pub classification: String,
}

pub async fn ensure_active_generation(
    pool: &PgPool,
    scope: &GraphScopeRecord,
    projection_schema_version: i32,
) -> Result<GraphGenerationRow, KnowledgeGraphError> {
    scope.validate()?;
    if projection_schema_version <= 0 {
        return Err(KnowledgeGraphError::InvalidSchemaVersion);
    }
    let mut tx = pool.begin().await?;
    lock_scope_generation(&mut tx, &scope.scope_key, projection_schema_version).await?;
    if let Some(active) =
        active_generation_with_connection(&mut *tx, scope, projection_schema_version, true).await?
    {
        tx.commit().await?;
        return Ok(active);
    }
    let row = sqlx::query_as::<_, GraphGenerationRow>(
        r#"INSERT INTO knowledge_graph_generations (
               generation_id, scope_key, visibility, project_scope_id,
               organization_id_at_time, projection_schema_version, status,
               build_hash, entity_count, relation_count, activated_at, completed_at
           ) VALUES ($1,$2,$3,$4,$5,$6,'active',$7,0,0,NOW(),NOW())
           RETURNING generation_id, scope_key, visibility, project_scope_id,
                     organization_id_at_time, projection_schema_version, status,
                     build_hash, entity_count, relation_count, failure_reason,
                     created_at, activated_at, completed_at"#,
    )
    .bind(Uuid::new_v4())
    .bind(&scope.scope_key)
    .bind(&scope.visibility)
    .bind(scope.project_scope_id)
    .bind(scope.organization_id_at_time)
    .bind(projection_schema_version)
    .bind(EMPTY_BUILD_HASH)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn begin_rebuild_generation(
    pool: &PgPool,
    scope: &GraphScopeRecord,
    projection_schema_version: i32,
) -> Result<GraphGenerationRow, KnowledgeGraphError> {
    scope.validate()?;
    if projection_schema_version <= 0 {
        return Err(KnowledgeGraphError::InvalidSchemaVersion);
    }
    Ok(sqlx::query_as::<_, GraphGenerationRow>(
        r#"INSERT INTO knowledge_graph_generations (
               generation_id, scope_key, visibility, project_scope_id,
               organization_id_at_time, projection_schema_version, status
           ) VALUES ($1,$2,$3,$4,$5,$6,'building')
           RETURNING generation_id, scope_key, visibility, project_scope_id,
                     organization_id_at_time, projection_schema_version, status,
                     build_hash, entity_count, relation_count, failure_reason,
                     created_at, activated_at, completed_at"#,
    )
    .bind(Uuid::new_v4())
    .bind(&scope.scope_key)
    .bind(&scope.visibility)
    .bind(scope.project_scope_id)
    .bind(scope.organization_id_at_time)
    .bind(projection_schema_version)
    .fetch_one(pool)
    .await?)
}

pub async fn activate_rebuild_generation(
    pool: &PgPool,
    generation_id: Uuid,
    build_hash: &str,
    expected_entity_count: i64,
    expected_relation_count: i64,
) -> Result<GraphGenerationRow, KnowledgeGraphError> {
    if build_hash.len() != 64
        || !build_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        || expected_entity_count < 0
        || expected_relation_count < 0
    {
        return Err(KnowledgeGraphError::InvalidBuildAttestation);
    }
    let mut tx = pool.begin().await?;
    let generation = generation_for_update(&mut tx, generation_id).await?;
    if generation.status != "building" {
        return Err(KnowledgeGraphError::GenerationNotBuilding);
    }
    lock_scope_generation(
        &mut tx,
        &generation.scope_key,
        generation.projection_schema_version,
    )
    .await?;
    let (actual_entities, actual_relations) = generation_counts(&mut tx, generation_id).await?;
    if actual_entities != expected_entity_count || actual_relations != expected_relation_count {
        return Err(KnowledgeGraphError::GenerationCountMismatch {
            expected_entities: expected_entity_count,
            actual_entities,
            expected_relations: expected_relation_count,
            actual_relations,
        });
    }
    let actual_hash: String =
        sqlx::query_scalar("SELECT compute_knowledge_graph_generation_hash($1)")
            .bind(generation_id)
            .fetch_one(&mut *tx)
            .await?;
    if !actual_hash.eq_ignore_ascii_case(build_hash) {
        return Err(KnowledgeGraphError::GenerationHashMismatch);
    }
    sqlx::query(
        r#"UPDATE knowledge_graph_generations
           SET status = 'retired'
           WHERE scope_key = $1
             AND projection_schema_version = $2
             AND status = 'active'"#,
    )
    .bind(&generation.scope_key)
    .bind(generation.projection_schema_version)
    .execute(&mut *tx)
    .await?;
    let activated = sqlx::query_as::<_, GraphGenerationRow>(
        r#"UPDATE knowledge_graph_generations
           SET status = 'active', build_hash = $2, entity_count = $3,
               relation_count = $4, activated_at = NOW(), completed_at = NOW()
           WHERE generation_id = $1 AND status = 'building'
           RETURNING generation_id, scope_key, visibility, project_scope_id,
                     organization_id_at_time, projection_schema_version, status,
                     build_hash, entity_count, relation_count, failure_reason,
                     created_at, activated_at, completed_at"#,
    )
    .bind(generation_id)
    .bind(actual_hash)
    .bind(actual_entities)
    .bind(actual_relations)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(activated)
}

pub async fn fail_rebuild_generation(
    pool: &PgPool,
    generation_id: Uuid,
    reason: &str,
) -> Result<(), KnowledgeGraphError> {
    if reason.trim().is_empty() {
        return Err(KnowledgeGraphError::EmptyFailureReason);
    }
    let updated = sqlx::query(
        r#"UPDATE knowledge_graph_generations
           SET status = 'failed', failure_reason = $2, completed_at = NOW()
           WHERE generation_id = $1 AND status = 'building'"#,
    )
    .bind(generation_id)
    .bind(reason.trim())
    .execute(pool)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(KnowledgeGraphError::GenerationNotBuilding);
    }
    Ok(())
}

pub async fn discard_failed_generation(
    pool: &PgPool,
    generation_id: Uuid,
) -> Result<(), KnowledgeGraphError> {
    let deleted = sqlx::query(
        "DELETE FROM knowledge_graph_generations WHERE generation_id = $1 AND status IN ('building','failed')",
    )
    .bind(generation_id)
    .execute(pool)
    .await?;
    if deleted.rows_affected() != 1 {
        return Err(KnowledgeGraphError::GenerationNotDiscardable);
    }
    Ok(())
}

pub async fn active_generation(
    pool: &PgPool,
    scope: &GraphScopeRecord,
    projection_schema_version: i32,
) -> Result<Option<GraphGenerationRow>, KnowledgeGraphError> {
    scope.validate()?;
    active_generation_with_connection(pool, scope, projection_schema_version, false).await
}

pub async fn generation(
    pool: &PgPool,
    generation_id: Uuid,
) -> Result<GraphGenerationRow, KnowledgeGraphError> {
    Ok(sqlx::query_as::<_, GraphGenerationRow>(
        r#"SELECT generation_id, scope_key, visibility, project_scope_id,
                  organization_id_at_time, projection_schema_version, status,
                  build_hash, entity_count, relation_count, failure_reason,
                  created_at, activated_at, completed_at
           FROM knowledge_graph_generations WHERE generation_id = $1"#,
    )
    .bind(generation_id)
    .fetch_one(pool)
    .await?)
}

pub async fn lock_writable_generation(
    connection: &mut PgConnection,
    generation_id: Uuid,
) -> Result<GraphGenerationRow, KnowledgeGraphError> {
    let generation = sqlx::query_as::<_, GraphGenerationRow>(
        r#"SELECT generation_id, scope_key, visibility, project_scope_id,
                  organization_id_at_time, projection_schema_version, status,
                  build_hash, entity_count, relation_count, failure_reason,
                  created_at, activated_at, completed_at
           FROM knowledge_graph_generations WHERE generation_id = $1 FOR UPDATE"#,
    )
    .bind(generation_id)
    .fetch_one(&mut *connection)
    .await?;
    if !matches!(generation.status.as_str(), "active" | "building") {
        return Err(KnowledgeGraphError::GenerationNotWritable);
    }
    Ok(generation)
}

pub async fn generation_attestation(
    pool: &PgPool,
    generation_id: Uuid,
) -> Result<GenerationAttestationRow, KnowledgeGraphError> {
    let row = sqlx::query_as::<_, (String, i64, i64)>(
        r#"SELECT compute_knowledge_graph_generation_hash($1),
                  (SELECT COUNT(*) FROM knowledge_graph_entities
                   WHERE generation_id = $1),
                  (SELECT COUNT(*) FROM knowledge_graph_relations
                   WHERE generation_id = $1)
           FROM knowledge_graph_generations
           WHERE generation_id = $1"#,
    )
    .bind(generation_id)
    .fetch_one(pool)
    .await?;
    Ok(GenerationAttestationRow {
        build_hash: row.0,
        entity_count: row.1,
        relation_count: row.2,
    })
}

/// Local single-user command authorization boundary for organization-scoped
/// graph reads/rebuilds. The caller must separately resolve the opaque active
/// local operator principal. This check proves that the stable project scope is
/// active and the exact organization is bound either by the live canonical
/// workspace path or by a sealed immutable operation-scope snapshot.
pub async fn organization_scope_is_registered_and_bound(
    pool: &PgPool,
    project_scope_id: Uuid,
    organization_id_at_time: Uuid,
) -> Result<bool, KnowledgeGraphError> {
    Ok(sqlx::query_scalar(
        r#"SELECT EXISTS (
               SELECT 1
               FROM project_scopes project
               WHERE project.project_scope_id = $1
                 AND project.retired_at IS NULL
                 AND (
                     EXISTS (
                         SELECT 1
                         FROM organizations organization
                         WHERE organization.id = $2
                           AND organization.project_path = project.canonical_project_path
                     )
                     OR EXISTS (
                         SELECT 1
                         FROM operation_org_scope_snapshots snapshot
                         JOIN operation_org_scope_units unit
                           ON unit.snapshot_id = snapshot.id
                         WHERE snapshot.project_scope_id = project.project_scope_id
                           AND snapshot.sealed_at IS NOT NULL
                           AND unit.organization_id = $2
                     )
                 )
           )"#,
    )
    .bind(project_scope_id)
    .bind(organization_id_at_time)
    .fetch_one(pool)
    .await?)
}

pub async fn upsert_entity_identity(
    connection: &mut PgConnection,
    input: &EntityIdentityWrite,
) -> Result<GraphEntityRow, KnowledgeGraphError> {
    input.scope.validate()?;
    let inserted = sqlx::query_as::<_, GraphEntityRow>(
        r#"INSERT INTO knowledge_graph_entities (
               entity_id, generation_id, scope_key, visibility, project_scope_id,
               organization_id_at_time, canonical_ref, identity_hash, entity_type,
               display_name, properties
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
           ON CONFLICT (generation_id, canonical_ref) DO UPDATE SET
               display_name = LEAST(knowledge_graph_entities.display_name,
                                    EXCLUDED.display_name),
               properties = CASE
                   WHEN knowledge_graph_entities.properties::text <= EXCLUDED.properties::text
                   THEN knowledge_graph_entities.properties
                   ELSE EXCLUDED.properties
               END,
               updated_at = NOW()
           WHERE knowledge_graph_entities.entity_id = EXCLUDED.entity_id
             AND knowledge_graph_entities.scope_key = EXCLUDED.scope_key
             AND knowledge_graph_entities.visibility = EXCLUDED.visibility
             AND knowledge_graph_entities.project_scope_id IS NOT DISTINCT FROM EXCLUDED.project_scope_id
             AND knowledge_graph_entities.organization_id_at_time IS NOT DISTINCT FROM EXCLUDED.organization_id_at_time
             AND knowledge_graph_entities.identity_hash = EXCLUDED.identity_hash
             AND knowledge_graph_entities.entity_type = EXCLUDED.entity_type
           RETURNING entity_id, generation_id, scope_key, visibility,
                     project_scope_id, organization_id_at_time, canonical_ref,
                     identity_hash, entity_type, display_name, properties"#,
    )
    .bind(input.entity_id)
    .bind(input.generation_id)
    .bind(&input.scope.scope_key)
    .bind(&input.scope.visibility)
    .bind(input.scope.project_scope_id)
    .bind(input.scope.organization_id_at_time)
    .bind(&input.canonical_ref)
    .bind(&input.identity_hash)
    .bind(&input.entity_type)
    .bind(&input.display_name)
    .bind(&input.properties)
    .fetch_optional(&mut *connection)
    .await?;
    if let Some(row) = inserted {
        return Ok(row);
    }
    let existing = entity_by_ref(connection, input.generation_id, &input.canonical_ref).await?;
    if existing.entity_id != input.entity_id
        || existing.scope_key != input.scope.scope_key
        || existing.visibility != input.scope.visibility
        || existing.project_scope_id != input.scope.project_scope_id
        || existing.organization_id_at_time != input.scope.organization_id_at_time
        || existing.identity_hash != input.identity_hash
        || existing.entity_type != input.entity_type
    {
        return Err(KnowledgeGraphError::IdentityReplayConflict);
    }
    Ok(existing)
}

pub async fn attach_entity_assertion(
    connection: &mut PgConnection,
    entity_id: Uuid,
    generation_id: Uuid,
    lineage: &AssertionLineageWrite,
) -> Result<(EntityAssertionRow, LineageWriteDisposition), KnowledgeGraphError> {
    let inserted = sqlx::query_as::<_, EntityAssertionRow>(
        r#"INSERT INTO knowledge_graph_entity_assertions (
               entity_id, generation_id, assertion_id, source_stream_key,
               source_version, evidence_refs, status, valid_from, valid_to,
               fresh_until, classification, projection_schema_version
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
           ON CONFLICT (entity_id, assertion_id, projection_schema_version) DO NOTHING
           RETURNING entity_id, generation_id, assertion_id, source_stream_key,
                     source_version, evidence_refs, status, valid_from, valid_to,
                     fresh_until, classification, projection_schema_version"#,
    )
    .bind(entity_id)
    .bind(generation_id)
    .bind(lineage.assertion_id)
    .bind(&lineage.source_stream_key)
    .bind(lineage.source_version)
    .bind(&lineage.evidence_refs)
    .bind(&lineage.status)
    .bind(lineage.valid_from)
    .bind(lineage.valid_to)
    .bind(lineage.fresh_until)
    .bind(&lineage.classification)
    .bind(lineage.projection_schema_version)
    .fetch_optional(&mut *connection)
    .await?;
    if let Some(row) = inserted {
        return Ok((row, LineageWriteDisposition::Inserted));
    }
    let existing = sqlx::query_as::<_, EntityAssertionRow>(
        r#"SELECT entity_id, generation_id, assertion_id, source_stream_key,
                  source_version, evidence_refs, status, valid_from, valid_to,
                  fresh_until, classification, projection_schema_version
           FROM knowledge_graph_entity_assertions
           WHERE entity_id = $1 AND assertion_id = $2
             AND projection_schema_version = $3"#,
    )
    .bind(entity_id)
    .bind(lineage.assertion_id)
    .bind(lineage.projection_schema_version)
    .fetch_one(&mut *connection)
    .await?;
    if existing.generation_id != generation_id
        || existing.source_stream_key != lineage.source_stream_key
        || existing.source_version != lineage.source_version
        || existing.evidence_refs != lineage.evidence_refs
        || existing.status != lineage.status
        || existing.valid_from.timestamp_micros() != lineage.valid_from.timestamp_micros()
        || existing.valid_to.map(|value| value.timestamp_micros())
            != lineage.valid_to.map(|value| value.timestamp_micros())
        || existing.fresh_until.map(|value| value.timestamp_micros())
            != lineage.fresh_until.map(|value| value.timestamp_micros())
        || existing.classification != lineage.classification
    {
        return Err(KnowledgeGraphError::LineageReplayConflict);
    }
    Ok((existing, LineageWriteDisposition::ExactReplay))
}

pub async fn upsert_relation_identity(
    connection: &mut PgConnection,
    input: &RelationIdentityWrite,
) -> Result<GraphRelationRow, KnowledgeGraphError> {
    let inserted = sqlx::query_as::<_, GraphRelationRow>(
        r#"INSERT INTO knowledge_graph_relations (
               relation_id, generation_id, scope_key, from_entity_id,
               to_entity_id, relation_type, identity_hash, properties
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
           ON CONFLICT (generation_id, from_entity_id, to_entity_id, relation_type)
           DO UPDATE SET
               properties = CASE
                   WHEN knowledge_graph_relations.properties::text <= EXCLUDED.properties::text
                   THEN knowledge_graph_relations.properties
                   ELSE EXCLUDED.properties
               END,
               updated_at = NOW()
           WHERE knowledge_graph_relations.relation_id = EXCLUDED.relation_id
             AND knowledge_graph_relations.scope_key = EXCLUDED.scope_key
             AND knowledge_graph_relations.identity_hash = EXCLUDED.identity_hash
           RETURNING relation_id, generation_id, scope_key, from_entity_id,
                     to_entity_id, relation_type, identity_hash, properties"#,
    )
    .bind(input.relation_id)
    .bind(input.generation_id)
    .bind(&input.scope_key)
    .bind(input.from_entity_id)
    .bind(input.to_entity_id)
    .bind(&input.relation_type)
    .bind(&input.identity_hash)
    .bind(&input.properties)
    .fetch_optional(&mut *connection)
    .await?;
    if let Some(row) = inserted {
        return Ok(row);
    }
    let existing = relation_by_identity(connection, input).await?;
    if existing.relation_id != input.relation_id
        || existing.scope_key != input.scope_key
        || existing.identity_hash != input.identity_hash
    {
        return Err(KnowledgeGraphError::IdentityReplayConflict);
    }
    Ok(existing)
}

pub async fn attach_relation_assertion(
    connection: &mut PgConnection,
    relation_id: Uuid,
    generation_id: Uuid,
    lineage: &AssertionLineageWrite,
) -> Result<(RelationAssertionRow, LineageWriteDisposition), KnowledgeGraphError> {
    let inserted = sqlx::query_as::<_, RelationAssertionRow>(
        r#"INSERT INTO knowledge_graph_relation_assertions (
               relation_id, generation_id, assertion_id, source_stream_key,
               source_version, evidence_refs, status, valid_from, valid_to,
               fresh_until, classification, projection_schema_version
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
           ON CONFLICT (relation_id, assertion_id, projection_schema_version) DO NOTHING
           RETURNING relation_id, generation_id, assertion_id, source_stream_key,
                     source_version, evidence_refs, status, valid_from, valid_to,
                     fresh_until, classification, projection_schema_version"#,
    )
    .bind(relation_id)
    .bind(generation_id)
    .bind(lineage.assertion_id)
    .bind(&lineage.source_stream_key)
    .bind(lineage.source_version)
    .bind(&lineage.evidence_refs)
    .bind(&lineage.status)
    .bind(lineage.valid_from)
    .bind(lineage.valid_to)
    .bind(lineage.fresh_until)
    .bind(&lineage.classification)
    .bind(lineage.projection_schema_version)
    .fetch_optional(&mut *connection)
    .await?;
    if let Some(row) = inserted {
        return Ok((row, LineageWriteDisposition::Inserted));
    }
    let existing = sqlx::query_as::<_, RelationAssertionRow>(
        r#"SELECT relation_id, generation_id, assertion_id, source_stream_key,
                  source_version, evidence_refs, status, valid_from, valid_to,
                  fresh_until, classification, projection_schema_version
           FROM knowledge_graph_relation_assertions
           WHERE relation_id = $1 AND assertion_id = $2
             AND projection_schema_version = $3"#,
    )
    .bind(relation_id)
    .bind(lineage.assertion_id)
    .bind(lineage.projection_schema_version)
    .fetch_one(&mut *connection)
    .await?;
    if existing.generation_id != generation_id
        || existing.source_stream_key != lineage.source_stream_key
        || existing.source_version != lineage.source_version
        || existing.evidence_refs != lineage.evidence_refs
        || existing.status != lineage.status
        || existing.valid_from.timestamp_micros() != lineage.valid_from.timestamp_micros()
        || existing.valid_to.map(|value| value.timestamp_micros())
            != lineage.valid_to.map(|value| value.timestamp_micros())
        || existing.fresh_until.map(|value| value.timestamp_micros())
            != lineage.fresh_until.map(|value| value.timestamp_micros())
        || existing.classification != lineage.classification
    {
        return Err(KnowledgeGraphError::LineageReplayConflict);
    }
    Ok((existing, LineageWriteDisposition::ExactReplay))
}

pub async fn max_source_version(
    connection: &mut PgConnection,
    generation_id: Uuid,
    source_stream_key: &str,
    projection_schema_version: i32,
) -> Result<Option<i64>, KnowledgeGraphError> {
    Ok(sqlx::query_scalar(
        r#"SELECT MAX(source_version) FROM (
               SELECT source_version
               FROM knowledge_graph_entity_assertions
               WHERE generation_id = $1 AND source_stream_key = $2
                 AND projection_schema_version = $3
               UNION ALL
               SELECT source_version
               FROM knowledge_graph_relation_assertions
               WHERE generation_id = $1 AND source_stream_key = $2
                 AND projection_schema_version = $3
           ) versions"#,
    )
    .bind(generation_id)
    .bind(source_stream_key)
    .bind(projection_schema_version)
    .fetch_one(&mut *connection)
    .await?)
}

pub async fn close_assertion_lineage(
    pool: &PgPool,
    assertion_id: Uuid,
    expected_valid_to: DateTime<Utc>,
) -> Result<(u64, u64), KnowledgeGraphError> {
    let mut tx = pool.begin().await?;
    let assertion = sqlx::query_as::<_, (String, Option<DateTime<Utc>>)>(
        "SELECT status, valid_to FROM knowledge_assertions WHERE assertion_id = $1 FOR SHARE",
    )
    .bind(assertion_id)
    .fetch_one(&mut *tx)
    .await?;
    if assertion.0 == "active"
        || assertion
            .1
            .is_none_or(|value| value.timestamp_micros() != expected_valid_to.timestamp_micros())
    {
        return Err(KnowledgeGraphError::CanonicalAssertionStillActive);
    }
    let entities = sqlx::query(
        r#"UPDATE knowledge_graph_entity_assertions lineage
           SET status = assertion.status,
               valid_from = assertion.valid_from,
               valid_to = assertion.valid_to,
               fresh_until = assertion.fresh_until,
               updated_at = NOW()
           FROM knowledge_assertions assertion
           WHERE lineage.assertion_id = $1
             AND assertion.assertion_id = lineage.assertion_id"#,
    )
    .bind(assertion_id)
    .execute(&mut *tx)
    .await?;
    let relations = sqlx::query(
        r#"UPDATE knowledge_graph_relation_assertions lineage
           SET status = assertion.status,
               valid_from = assertion.valid_from,
               valid_to = assertion.valid_to,
               fresh_until = assertion.fresh_until,
               updated_at = NOW()
           FROM knowledge_assertions assertion
           WHERE lineage.assertion_id = $1
             AND assertion.assertion_id = lineage.assertion_id"#,
    )
    .bind(assertion_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((entities.rows_affected(), relations.rows_affected()))
}

pub async fn query_active_entities(
    pool: &PgPool,
    scope: &GraphScopeRecord,
    projection_schema_version: i32,
    query: &str,
    classification_ceiling: &str,
    valid_at: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<ActiveEntityLineageRow>, KnowledgeGraphError> {
    scope.validate()?;
    if limit <= 0 || query.trim().is_empty() {
        return Err(KnowledgeGraphError::InvalidQuery);
    }
    let ceiling_rank = classification_rank(classification_ceiling)?;
    Ok(sqlx::query_as::<_, ActiveEntityLineageRow>(
        r#"SELECT generation.generation_id, entity.entity_id, entity.scope_key,
                  entity.visibility, entity.project_scope_id,
                  entity.organization_id_at_time, entity.canonical_ref,
                  entity.entity_type, entity.display_name, entity.properties,
                  lineage.assertion_id, lineage.source_stream_key,
                  lineage.source_version, lineage.evidence_refs,
                  lineage.valid_from, lineage.valid_to, lineage.fresh_until,
                  lineage.classification
           FROM knowledge_graph_generations generation
           JOIN knowledge_graph_entities entity
             ON entity.generation_id = generation.generation_id
            AND entity.scope_key = generation.scope_key
           JOIN knowledge_graph_entity_assertions lineage
             ON lineage.entity_id = entity.entity_id
            AND lineage.generation_id = generation.generation_id
           JOIN knowledge_assertions assertion
             ON assertion.assertion_id = lineage.assertion_id
           WHERE generation.status = 'active'
             AND generation.projection_schema_version = $1
             AND generation.scope_key = $2
             AND generation.project_scope_id IS NOT DISTINCT FROM $3
             AND generation.organization_id_at_time IS NOT DISTINCT FROM $4
             AND (entity.canonical_ref ILIKE $5 OR entity.display_name ILIKE $5)
             AND lineage.status = 'active'
             AND assertion.status = 'active'
             AND lineage.valid_from <= $6
             AND (lineage.valid_to IS NULL OR lineage.valid_to > $6)
             AND (lineage.fresh_until IS NULL OR lineage.fresh_until > $6)
             AND assertion.valid_from <= $6
             AND (assertion.valid_to IS NULL OR assertion.valid_to > $6)
             AND (assertion.fresh_until IS NULL OR assertion.fresh_until > $6)
             AND CASE lineage.classification
                   WHEN 'public' THEN 0 WHEN 'internal' THEN 1
                   WHEN 'customer_confidential' THEN 2 WHEN 'restricted' THEN 3
                 END <= $7
           ORDER BY entity.canonical_ref, lineage.assertion_id
           LIMIT $8"#,
    )
    .bind(projection_schema_version)
    .bind(&scope.scope_key)
    .bind(scope.project_scope_id)
    .bind(scope.organization_id_at_time)
    .bind(format!("%{}%", query.trim()))
    .bind(valid_at)
    .bind(ceiling_rank)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

pub async fn query_active_relations(
    pool: &PgPool,
    scope: &GraphScopeRecord,
    projection_schema_version: i32,
    query: &str,
    classification_ceiling: &str,
    valid_at: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<ActiveRelationLineageRow>, KnowledgeGraphError> {
    scope.validate()?;
    if limit <= 0 || query.trim().is_empty() {
        return Err(KnowledgeGraphError::InvalidQuery);
    }
    let ceiling_rank = classification_rank(classification_ceiling)?;
    Ok(sqlx::query_as::<_, ActiveRelationLineageRow>(
        r#"WITH current_entities AS (
               SELECT DISTINCT lineage.generation_id, lineage.entity_id
               FROM knowledge_graph_entity_assertions lineage
               JOIN knowledge_assertions assertion
                 ON assertion.assertion_id = lineage.assertion_id
               WHERE lineage.status = 'active' AND assertion.status = 'active'
                 AND lineage.valid_from <= $6
                 AND (lineage.valid_to IS NULL OR lineage.valid_to > $6)
                 AND (lineage.fresh_until IS NULL OR lineage.fresh_until > $6)
                 AND assertion.valid_from <= $6
                 AND (assertion.valid_to IS NULL OR assertion.valid_to > $6)
                 AND (assertion.fresh_until IS NULL OR assertion.fresh_until > $6)
                 AND CASE lineage.classification
                       WHEN 'public' THEN 0 WHEN 'internal' THEN 1
                       WHEN 'customer_confidential' THEN 2 WHEN 'restricted' THEN 3
                     END <= $7
           )
           SELECT generation.generation_id, relation.relation_id,
                  relation.scope_key, generation.project_scope_id,
                  generation.organization_id_at_time,
                  source.entity_id AS from_entity_id,
                  source.canonical_ref AS from_canonical_ref,
                  source.entity_type AS from_entity_type,
                  source.display_name AS from_display_name,
                  target.entity_id AS to_entity_id,
                  target.canonical_ref AS to_canonical_ref,
                  target.entity_type AS to_entity_type,
                  target.display_name AS to_display_name,
                  relation.relation_type, relation.properties,
                  lineage.assertion_id, lineage.source_stream_key,
                  lineage.source_version, lineage.evidence_refs,
                  lineage.valid_from, lineage.valid_to, lineage.fresh_until,
                  lineage.classification
           FROM knowledge_graph_generations generation
           JOIN knowledge_graph_relations relation
             ON relation.generation_id = generation.generation_id
            AND relation.scope_key = generation.scope_key
           JOIN knowledge_graph_entities source
             ON source.entity_id = relation.from_entity_id
            AND source.generation_id = generation.generation_id
            AND source.scope_key = generation.scope_key
           JOIN knowledge_graph_entities target
             ON target.entity_id = relation.to_entity_id
            AND target.generation_id = generation.generation_id
            AND target.scope_key = generation.scope_key
           JOIN current_entities source_current
             ON source_current.generation_id = generation.generation_id
            AND source_current.entity_id = source.entity_id
           JOIN current_entities target_current
             ON target_current.generation_id = generation.generation_id
            AND target_current.entity_id = target.entity_id
           JOIN knowledge_graph_relation_assertions lineage
             ON lineage.relation_id = relation.relation_id
            AND lineage.generation_id = generation.generation_id
           JOIN knowledge_assertions assertion
             ON assertion.assertion_id = lineage.assertion_id
           WHERE generation.status = 'active'
             AND generation.projection_schema_version = $1
             AND generation.scope_key = $2
             AND generation.project_scope_id IS NOT DISTINCT FROM $3
             AND generation.organization_id_at_time IS NOT DISTINCT FROM $4
             AND (relation.relation_type ILIKE $5
                  OR source.canonical_ref ILIKE $5
                  OR source.display_name ILIKE $5
                  OR target.canonical_ref ILIKE $5
                  OR target.display_name ILIKE $5)
             AND lineage.status = 'active' AND assertion.status = 'active'
             AND lineage.valid_from <= $6
             AND (lineage.valid_to IS NULL OR lineage.valid_to > $6)
             AND (lineage.fresh_until IS NULL OR lineage.fresh_until > $6)
             AND assertion.valid_from <= $6
             AND (assertion.valid_to IS NULL OR assertion.valid_to > $6)
             AND (assertion.fresh_until IS NULL OR assertion.fresh_until > $6)
             AND CASE lineage.classification
                   WHEN 'public' THEN 0 WHEN 'internal' THEN 1
                   WHEN 'customer_confidential' THEN 2 WHEN 'restricted' THEN 3
                 END <= $7
           ORDER BY source.canonical_ref, relation.relation_type,
                    target.canonical_ref, lineage.assertion_id
           LIMIT $8"#,
    )
    .bind(projection_schema_version)
    .bind(&scope.scope_key)
    .bind(scope.project_scope_id)
    .bind(scope.organization_id_at_time)
    .bind(format!("%{}%", query.trim()))
    .bind(valid_at)
    .bind(ceiling_rank)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

pub async fn active_entity_lineage_count(
    pool: &PgPool,
    entity_id: Uuid,
    valid_at: DateTime<Utc>,
) -> Result<i64, KnowledgeGraphError> {
    Ok(sqlx::query_scalar(
        r#"SELECT COUNT(*)
           FROM knowledge_graph_entity_assertions lineage
           JOIN knowledge_assertions assertion
             ON assertion.assertion_id = lineage.assertion_id
           WHERE lineage.entity_id = $1
             AND lineage.status = 'active' AND assertion.status = 'active'
             AND lineage.valid_from <= $2
             AND (lineage.valid_to IS NULL OR lineage.valid_to > $2)
             AND (lineage.fresh_until IS NULL OR lineage.fresh_until > $2)
             AND assertion.valid_from <= $2
             AND (assertion.valid_to IS NULL OR assertion.valid_to > $2)
             AND (assertion.fresh_until IS NULL OR assertion.fresh_until > $2)"#,
    )
    .bind(entity_id)
    .bind(valid_at)
    .fetch_one(pool)
    .await?)
}

pub async fn active_relation_lineage_count(
    pool: &PgPool,
    relation_id: Uuid,
    valid_at: DateTime<Utc>,
) -> Result<i64, KnowledgeGraphError> {
    Ok(sqlx::query_scalar(
        r#"SELECT COUNT(*)
           FROM knowledge_graph_relation_assertions lineage
           JOIN knowledge_assertions assertion
             ON assertion.assertion_id = lineage.assertion_id
           WHERE lineage.relation_id = $1
             AND lineage.status = 'active' AND assertion.status = 'active'
             AND lineage.valid_from <= $2
             AND (lineage.valid_to IS NULL OR lineage.valid_to > $2)
             AND (lineage.fresh_until IS NULL OR lineage.fresh_until > $2)
             AND assertion.valid_from <= $2
             AND (assertion.valid_to IS NULL OR assertion.valid_to > $2)
             AND (assertion.fresh_until IS NULL OR assertion.fresh_until > $2)"#,
    )
    .bind(relation_id)
    .bind(valid_at)
    .fetch_one(pool)
    .await?)
}

async fn lock_scope_generation(
    tx: &mut Transaction<'_, Postgres>,
    scope_key: &str,
    projection_schema_version: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, $2::bigint))")
        .bind(scope_key)
        .bind(i64::from(projection_schema_version))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn active_generation_with_connection<'e, E>(
    executor: E,
    scope: &GraphScopeRecord,
    projection_schema_version: i32,
    for_update: bool,
) -> Result<Option<GraphGenerationRow>, KnowledgeGraphError>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = if for_update {
        active_generation_select_sql(true)
    } else {
        active_generation_select_sql(false)
    };
    Ok(sqlx::query_as::<_, GraphGenerationRow>(sql)
        .bind(&scope.scope_key)
        .bind(scope.project_scope_id)
        .bind(scope.organization_id_at_time)
        .bind(projection_schema_version)
        .fetch_optional(executor)
        .await?)
}

fn active_generation_select_sql(for_update: bool) -> &'static str {
    if for_update {
        r#"SELECT generation_id, scope_key, visibility, project_scope_id,
                  organization_id_at_time, projection_schema_version, status,
                  build_hash, entity_count, relation_count, failure_reason,
                  created_at, activated_at, completed_at
           FROM knowledge_graph_generations
           WHERE scope_key = $1
             AND project_scope_id IS NOT DISTINCT FROM $2
             AND organization_id_at_time IS NOT DISTINCT FROM $3
             AND projection_schema_version = $4 AND status = 'active'
           FOR UPDATE"#
    } else {
        r#"SELECT generation_id, scope_key, visibility, project_scope_id,
                  organization_id_at_time, projection_schema_version, status,
                  build_hash, entity_count, relation_count, failure_reason,
                  created_at, activated_at, completed_at
           FROM knowledge_graph_generations
           WHERE scope_key = $1
             AND project_scope_id IS NOT DISTINCT FROM $2
             AND organization_id_at_time IS NOT DISTINCT FROM $3
             AND projection_schema_version = $4 AND status = 'active'"#
    }
}

async fn generation_for_update(
    tx: &mut Transaction<'_, Postgres>,
    generation_id: Uuid,
) -> Result<GraphGenerationRow, KnowledgeGraphError> {
    Ok(sqlx::query_as::<_, GraphGenerationRow>(
        r#"SELECT generation_id, scope_key, visibility, project_scope_id,
                  organization_id_at_time, projection_schema_version, status,
                  build_hash, entity_count, relation_count, failure_reason,
                  created_at, activated_at, completed_at
           FROM knowledge_graph_generations WHERE generation_id = $1 FOR UPDATE"#,
    )
    .bind(generation_id)
    .fetch_one(&mut **tx)
    .await?)
}

async fn generation_counts(
    tx: &mut Transaction<'_, Postgres>,
    generation_id: Uuid,
) -> Result<(i64, i64), sqlx::Error> {
    sqlx::query_as(
        r#"SELECT
             (SELECT COUNT(*) FROM knowledge_graph_entities WHERE generation_id = $1),
             (SELECT COUNT(*) FROM knowledge_graph_relations WHERE generation_id = $1)"#,
    )
    .bind(generation_id)
    .fetch_one(&mut **tx)
    .await
}

async fn entity_by_ref(
    connection: &mut PgConnection,
    generation_id: Uuid,
    canonical_ref: &str,
) -> Result<GraphEntityRow, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT entity_id, generation_id, scope_key, visibility,
                  project_scope_id, organization_id_at_time, canonical_ref,
                  identity_hash, entity_type, display_name, properties
           FROM knowledge_graph_entities
           WHERE generation_id = $1 AND canonical_ref = $2"#,
    )
    .bind(generation_id)
    .bind(canonical_ref)
    .fetch_one(&mut *connection)
    .await
}

async fn relation_by_identity(
    connection: &mut PgConnection,
    input: &RelationIdentityWrite,
) -> Result<GraphRelationRow, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT relation_id, generation_id, scope_key, from_entity_id,
                  to_entity_id, relation_type, identity_hash, properties
           FROM knowledge_graph_relations
           WHERE generation_id = $1 AND from_entity_id = $2
             AND to_entity_id = $3 AND relation_type = $4"#,
    )
    .bind(input.generation_id)
    .bind(input.from_entity_id)
    .bind(input.to_entity_id)
    .bind(&input.relation_type)
    .fetch_one(&mut *connection)
    .await
}

fn classification_rank(value: &str) -> Result<i32, KnowledgeGraphError> {
    match value {
        "public" => Ok(0),
        "internal" => Ok(1),
        "customer_confidential" => Ok(2),
        "restricted" => Ok(3),
        _ => Err(KnowledgeGraphError::InvalidClassification),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KnowledgeGraphError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid temporal graph scope")]
    InvalidScope,
    #[error("projection schema version must be positive")]
    InvalidSchemaVersion,
    #[error("invalid rebuild attestation")]
    InvalidBuildAttestation,
    #[error("generation is not building")]
    GenerationNotBuilding,
    #[error("generation is not writable")]
    GenerationNotWritable,
    #[error("generation is not discardable")]
    GenerationNotDiscardable,
    #[error("generation count mismatch")]
    GenerationCountMismatch {
        expected_entities: i64,
        actual_entities: i64,
        expected_relations: i64,
        actual_relations: i64,
    },
    #[error("generation content hash does not match rebuild attestation")]
    GenerationHashMismatch,
    #[error("rebuild failure reason cannot be empty")]
    EmptyFailureReason,
    #[error("graph identity replay conflicts with stored identity")]
    IdentityReplayConflict,
    #[error("graph lineage replay conflicts with stored lineage")]
    LineageReplayConflict,
    #[error("canonical assertion is still active or has a different validity boundary")]
    CanonicalAssertionStillActive,
    #[error("invalid temporal graph query")]
    InvalidQuery,
    #[error("invalid classification ceiling")]
    InvalidClassification,
}

impl KnowledgeGraphError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Sqlx(_) => "knowledge_graph_database_error",
            Self::InvalidScope => "knowledge_graph_scope_invalid",
            Self::InvalidSchemaVersion => "knowledge_graph_schema_version_invalid",
            Self::InvalidBuildAttestation => "knowledge_graph_build_attestation_invalid",
            Self::GenerationNotBuilding => "knowledge_graph_generation_not_building",
            Self::GenerationNotWritable => "knowledge_graph_generation_not_writable",
            Self::GenerationNotDiscardable => "knowledge_graph_generation_not_discardable",
            Self::GenerationCountMismatch { .. } => "knowledge_graph_generation_count_mismatch",
            Self::GenerationHashMismatch => "knowledge_graph_generation_hash_mismatch",
            Self::EmptyFailureReason => "knowledge_graph_failure_reason_empty",
            Self::IdentityReplayConflict => "knowledge_graph_identity_replay_conflict",
            Self::LineageReplayConflict => "knowledge_graph_lineage_replay_conflict",
            Self::CanonicalAssertionStillActive => "knowledge_graph_assertion_still_active",
            Self::InvalidQuery => "knowledge_graph_query_invalid",
            Self::InvalidClassification => "knowledge_graph_classification_invalid",
        }
    }
}
